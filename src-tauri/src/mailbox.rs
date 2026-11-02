//! Mailboxes beyond the synced ones: listing an account's folders, browsing one on demand
//! (Trash, Junk, labels; read live from the server, never cached), and moving messages
//! between folders.

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_imap::types::{Flag, NameAttribute};
use futures_util::{StreamExt, TryStreamExt};
use mail_parser::MessageParser;
use rusqlite::Connection;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::archive::{imap_move, move_messages, searchable, uid_set};
use crate::auth;
use crate::commands::AppState;
use crate::connect::{ImapSession, open_imap};
use crate::db::{self, MessageLoc};
use crate::error::{AppError, ErrorCode, Result};
use crate::model::{Account, Message, Person};
use crate::sync::{self, ARCHIVE_NAMES, Role, SENT_NAMES, Special};

/// Messages per page when browsing.
const PAGE: usize = 40;

const DRAFTS_NAMES: &[&str] = &[
    "Drafts",
    "Draft",
    "INBOX.Drafts",
    "INBOX/Drafts",
    "[Gmail]/Drafts",
    "Koncepty",
    "Чернетки",
];
const JUNK_NAMES: &[&str] = &[
    "Junk",
    "Spam",
    "Junk E-mail",
    "Junk Email",
    "Bulk Mail",
    "INBOX.Junk",
    "INBOX.Spam",
    "INBOX/Junk",
    "INBOX/Spam",
    "[Gmail]/Spam",
    "Nevyžiadaná pošta",
    "Нежелательная почта",
    "Спам",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MailboxRole {
    Inbox,
    Sent,
    Drafts,
    Archive,
    Junk,
    Trash,
    /// Gmail's All Mail: everything, already synced.
    All,
    Other,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mailbox {
    /// IMAP name, as the server wants it back.
    pub name: String,
    /// Human name: decoded, without "INBOX." / "[Gmail]/" prefixes, levels joined by " / ".
    pub label: String,
    pub role: MailboxRole,
    /// Kept in the local cache by sync; its mail already shows in chats.
    pub synced: bool,
}

/// One browsed message, with what the UI needs to group it into chats.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Browsed {
    pub uid: u32,
    pub chat_key: String,
    pub others: Vec<Person>,
    pub message: Message,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowsePage {
    /// Newest first.
    pub messages: Vec<Browsed>,
    /// Pass back as `before` for the next page; None when nothing older is left.
    pub before: Option<u32>,
}

fn imap_err(context: &str, e: impl std::fmt::Display) -> AppError {
    AppError::new(ErrorCode::ImapConnect, format!("{context}: {e}"))
}

fn named(name: &str, names: &[&str]) -> bool {
    names.iter().any(|n| n.eq_ignore_ascii_case(name))
}

/// Special-use flags (RFC 6154) first, well-known names second.
fn role_of(name: &str, attrs: &[NameAttribute<'_>]) -> MailboxRole {
    let flagged = [
        (NameAttribute::All, MailboxRole::All),
        (NameAttribute::Trash, MailboxRole::Trash),
        (NameAttribute::Junk, MailboxRole::Junk),
        (NameAttribute::Sent, MailboxRole::Sent),
        (NameAttribute::Drafts, MailboxRole::Drafts),
        (NameAttribute::Archive, MailboxRole::Archive),
    ];
    if name.eq_ignore_ascii_case("INBOX") {
        return MailboxRole::Inbox;
    }
    if let Some((_, role)) = flagged.iter().find(|(a, _)| attrs.contains(a)) {
        return *role;
    }
    if named(name, crate::trash::TRASH_NAMES) {
        MailboxRole::Trash
    } else if named(name, JUNK_NAMES) {
        MailboxRole::Junk
    } else if named(name, SENT_NAMES) {
        MailboxRole::Sent
    } else if named(name, DRAFTS_NAMES) {
        MailboxRole::Drafts
    } else if named(name, ARCHIVE_NAMES) {
        MailboxRole::Archive
    } else {
        MailboxRole::Other
    }
}

/// IMAP mailbox names are "modified UTF-7" (RFC 3501 5.1.3): `&…-` holds UTF-16 in base64
/// with `,` for `/`, and `&-` is a literal `&`. Undecodable runs are kept as they are.
pub fn decode_mutf7(name: &str) -> String {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD_NO_PAD;
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('-') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let run = &after[..end];
        if run.is_empty() {
            out.push('&');
        } else {
            let decoded = STANDARD_NO_PAD
                .decode(run.replace(',', "/"))
                .ok()
                .filter(|b| b.len() % 2 == 0)
                .and_then(|b| {
                    let units: Vec<u16> = b
                        .chunks(2)
                        .map(|c| u16::from_be_bytes([c[0], c[1]]))
                        .collect();
                    String::from_utf16(&units).ok()
                });
            match decoded {
                Some(s) => out.push_str(&s),
                None => out.push_str(&rest[start..start + 2 + end]),
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

fn label_of(name: &str, delimiter: Option<&str>) -> String {
    let decoded = decode_mutf7(name);
    let mut parts: Vec<&str> = match delimiter.filter(|d| !d.is_empty()) {
        Some(d) => decoded.split(d).collect(),
        None => vec![decoded.as_str()],
    };
    // "INBOX.Work" and "[Gmail]/Spam" read as "Work" and "Spam".
    if parts.len() > 1
        && (parts[0].eq_ignore_ascii_case("INBOX")
            || parts[0].eq_ignore_ascii_case("[Gmail]")
            || parts[0].eq_ignore_ascii_case("[Google Mail]"))
    {
        parts.remove(0);
    }
    parts.join(" / ")
}

async fn list(session: &mut ImapSession) -> Result<Vec<Mailbox>> {
    let names: Vec<_> = session
        .list(Some(""), Some("*"))
        .await
        .map_err(|e| imap_err("LIST", e))?
        .try_collect()
        .await
        .map_err(|e| imap_err("LIST", e))?;
    let selectable: Vec<_> = names
        .iter()
        .filter(|n| !n.attributes().contains(&NameAttribute::NoSelect))
        .collect();
    let for_sync: Vec<(String, Special)> = selectable
        .iter()
        .map(|n| (n.name().to_owned(), sync::special(n.attributes())))
        .collect();
    let synced: Vec<String> = sync::choose_folders(&for_sync)
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    Ok(selectable
        .iter()
        .map(|n| Mailbox {
            name: n.name().to_owned(),
            label: label_of(n.name(), n.delimiter()),
            role: role_of(n.name(), n.attributes()),
            synced: synced.iter().any(|s| s == n.name()),
        })
        .collect())
}

async fn connect(account: &Account) -> Result<ImapSession> {
    let creds = auth::credentials(account).await?;
    open_imap(&account.imap, creds.login()).await
}

fn find<'a>(boxes: &'a [Mailbox], name: &str) -> Result<&'a Mailbox> {
    boxes
        .iter()
        .find(|b| b.name == name)
        .ok_or_else(|| AppError::new(ErrorCode::NotFound, format!("No folder named {name}")))
}

fn account_of(state: &AppState, id: &str) -> Result<Account> {
    let conn = state
        .db
        .lock()
        .map_err(|_| AppError::new(ErrorCode::Storage, "Database lock poisoned"))?;
    db::get_account(&conn, id)
}

/// The account's folders, for the mailbox picker and "Move to".
#[tauri::command]
pub async fn list_mailboxes(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Vec<Mailbox>> {
    let account = account_of(&state, &account_id)?;
    let mut session = connect(&account).await?;
    let boxes = list(&mut session).await;
    let _ = session.logout().await;
    boxes
}

/// A page of a folder's newest mail, read-only (EXAMINE never marks anything as read).
#[tauri::command]
pub async fn browse_mailbox(
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    before: Option<u32>,
) -> Result<BrowsePage> {
    let account = account_of(&state, &account_id)?;
    let mut session = connect(&account).await?;
    let page = browse(&mut session, &account, &folder, before).await;
    let _ = session.logout().await;
    page
}

async fn browse(
    session: &mut ImapSession,
    account: &Account,
    folder: &str,
    before: Option<u32>,
) -> Result<BrowsePage> {
    session
        .examine(folder)
        .await
        .map_err(|e| imap_err(folder, e))?;
    let query = match before {
        Some(b) if b <= 1 => {
            return Ok(BrowsePage {
                messages: vec![],
                before: None,
            });
        }
        Some(b) => format!("UID 1:{}", b - 1),
        None => "ALL".to_owned(),
    };
    let mut uids: Vec<u32> = session
        .uid_search(query)
        .await
        .map_err(|e| imap_err(folder, e))?
        .into_iter()
        .filter(|u| before.is_none_or(|b| *u < b))
        .collect();
    uids.sort_unstable();
    let page: Vec<u32> = uids.split_off(uids.len().saturating_sub(PAGE));
    let next = (!uids.is_empty()).then(|| page[0]);
    if page.is_empty() {
        return Ok(BrowsePage {
            messages: vec![],
            before: None,
        });
    }

    let own = sync::own_addresses(account);
    let set = page
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let mut stream = session
        .uid_fetch(&set, "(UID FLAGS BODY.PEEK[])")
        .await
        .map_err(|e| imap_err(folder, e))?;
    let mut messages = Vec::with_capacity(page.len());
    while let Some(item) = stream.next().await {
        let f = item.map_err(|e| imap_err(folder, e))?;
        let (Some(uid), Some(raw)) = (f.uid, f.body()) else {
            continue;
        };
        let Some(p) = crate::parse::parse(raw, &own, false, 0) else {
            continue;
        };
        let read = f.flags().any(|fl| fl == Flag::Seen);
        messages.push(Browsed {
            uid,
            chat_key: crate::parse::chat_key(&account.id, &p.others),
            others: p.others,
            message: Message {
                id: format!("box|{}|{uid}|{folder}", account.id),
                from: p.from,
                outgoing: p.outgoing,
                own_address: p.own_address,
                sent_at: p.sent_at,
                subject: p.subject,
                body: p.body,
                quoted: p.quoted,
                html: p.html,
                ics: p.ics,
                attachments: p.attachments,
                read,
                unsubscribe: p.unsubscribe,
                reply_to: None,
            },
        });
    }
    drop(stream);
    messages.sort_by(|a, b| b.message.sent_at.cmp(&a.message.sent_at));
    Ok(BrowsePage {
        messages,
        before: next,
    })
}

async fn gmail_labels(session: &mut ImapSession, set: &str, op: &str) -> Result<()> {
    session
        .uid_store(set, format!("{op}X-GM-LABELS (\\Inbox)"))
        .await
        .map_err(|e| imap_err("STORE", e))?
        .try_collect::<Vec<_>>()
        .await
        .map_err(|e| imap_err("STORE", e))?;
    Ok(())
}

/// Gmail keeps every message in All Mail and folders are labels: Trash and Spam really
/// move it out; Inbox puts the \Inbox label back; any other label is added, and the message
/// leaves the inbox, as "Move to" does in Gmail.
async fn move_gmail(
    session: &mut ImapSession,
    db: &Mutex<Connection>,
    locs: &[&MessageLoc],
    all: &str,
    target: &Mailbox,
) -> Result<()> {
    let lock = || {
        db.lock()
            .map_err(|_| AppError::new(ErrorCode::Storage, "Database lock poisoned"))
    };
    let set = uid_set(locs);
    match target.role {
        MailboxRole::Trash | MailboxRole::Junk => {
            return move_messages(session, db, locs, all, &target.name, false).await;
        }
        MailboxRole::Inbox => {
            session.select(all).await.map_err(|e| imap_err(all, e))?;
            gmail_labels(session, &set, "+").await?;
        }
        _ => {
            session.select(all).await.map_err(|e| imap_err(all, e))?;
            session
                .uid_copy(&set, &target.name)
                .await
                .map_err(|e| imap_err("COPY", e))?;
            gmail_labels(session, &set, "-").await?;
        }
    }
    let conn = lock()?;
    for l in locs {
        db::set_in_inbox(&conn, l.id, target.role == MailboxRole::Inbox)?;
    }
    Ok(())
}

/// Moves stored (synced) messages, by local id, to another folder of their account.
#[tauri::command]
pub async fn move_stored_messages(
    app: AppHandle,
    state: State<'_, AppState>,
    message_ids: Vec<String>,
    target: String,
) -> Result<()> {
    let ids: Vec<i64> = message_ids.iter().filter_map(|i| i.parse().ok()).collect();
    let locs = {
        let conn = state
            .db
            .lock()
            .map_err(|_| AppError::new(ErrorCode::Storage, "Database lock poisoned"))?;
        db::message_locs(&conn, &ids)?
    };
    let mut by_account: BTreeMap<String, Vec<MessageLoc>> = BTreeMap::new();
    for (account, loc) in locs {
        // Just sent and not on the server yet: nothing to move.
        if loc.folder != db::LOCAL_FOLDER {
            by_account.entry(account).or_default().push(loc);
        }
    }
    // A sync running meanwhile would write back the state it read before the move.
    let _no_sync = state.sync_lock.lock().await;
    let mut result = Ok(());
    for (account_id, locs) in &by_account {
        let account = account_of(&state, account_id)?;
        let mut session = connect(&account).await?;
        let moved = async {
            let boxes = list(&mut session).await?;
            let target = find(&boxes, &target)?.clone();
            if let Some(all) = boxes
                .iter()
                .find(|b| b.role == MailboxRole::All && b.synced)
            {
                let refs: Vec<&MessageLoc> = locs.iter().filter(|l| l.folder == all.name).collect();
                return move_gmail(&mut session, &state.db, &refs, &all.name, &target).await;
            }
            let mut by_folder: BTreeMap<&str, Vec<&MessageLoc>> = BTreeMap::new();
            for l in locs.iter().filter(|l| l.folder != target.name) {
                by_folder.entry(l.folder.as_str()).or_default().push(l);
            }
            for (from, refs) in by_folder {
                move_messages(
                    &mut session,
                    &state.db,
                    &refs,
                    from,
                    &target.name,
                    target.synced,
                )
                .await?;
            }
            Ok(())
        }
        .await;
        let _ = session.logout().await;
        if moved.is_err() {
            result = moved;
        }
    }
    let _ = app.emit("mail-changed", ());
    result
}

/// Message-IDs of the given UIDs in the selected folder.
async fn message_ids(session: &mut ImapSession, set: &str) -> Result<Vec<String>> {
    let mut stream = session
        .uid_fetch(set, "(UID BODY.PEEK[HEADER])")
        .await
        .map_err(|e| imap_err("FETCH", e))?;
    let mut ids = Vec::new();
    while let Some(item) = stream.next().await {
        let f = item.map_err(|e| imap_err("FETCH", e))?;
        if let Some(id) = f
            .header()
            .and_then(|h| MessageParser::default().parse(h))
            .and_then(|m| m.message_id().map(str::to_owned))
        {
            ids.push(id);
        }
    }
    Ok(ids)
}

/// Moves browsed messages (by UID in their folder). When they land in a synced folder they
/// are fetched into the cache at once, so e.g. mail restored from Trash shows in its chat.
#[tauri::command]
pub async fn move_browsed_messages(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    folder: String,
    uids: Vec<u32>,
    target: String,
) -> Result<()> {
    if uids.is_empty() || folder == target {
        return Ok(());
    }
    let account = account_of(&state, &account_id)?;
    let _no_sync = state.sync_lock.lock().await;
    let mut session = connect(&account).await?;
    let result = async {
        let boxes = list(&mut session).await?;
        let target = find(&boxes, &target)?.clone();
        let set = uids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        session
            .examine(&folder)
            .await
            .map_err(|e| imap_err(&folder, e))?;
        let ids = message_ids(&mut session, &set).await?;
        imap_move(&mut session, &set, &folder, &target.name).await?;

        // Where the moved mail now lives in the cache's terms: Gmail's All Mail holds all
        // but Trash and Spam; elsewhere only the synced folders.
        let all = boxes
            .iter()
            .find(|b| b.role == MailboxRole::All && b.synced);
        let (cached, role) = match all {
            Some(all) if !matches!(target.role, MailboxRole::Trash | MailboxRole::Junk) => {
                (all.name.clone(), Role::All)
            }
            None if target.synced => {
                let role = match target.role {
                    MailboxRole::Inbox => Role::Inbox,
                    MailboxRole::Sent => Role::Sent,
                    _ => Role::Archive,
                };
                (target.name.clone(), role)
            }
            _ => return Ok(()),
        };
        session
            .examine(&cached)
            .await
            .map_err(|e| imap_err(&cached, e))?;
        let mut found = Vec::new();
        for id in ids.iter().filter(|id| searchable(id)) {
            if let Ok(hits) = session
                .uid_search(format!("HEADER Message-ID \"{id}\""))
                .await
            {
                found.extend(hits.into_iter().max());
            }
        }
        if found.is_empty() {
            return Ok(());
        }
        found.sort_unstable();
        let own = sync::own_addresses(&account);
        let fetched = sync::fetch_parsed(&mut session, &cached, role, &own, &found).await?;
        let conn = state
            .db
            .lock()
            .map_err(|_| AppError::new(ErrorCode::Storage, "Database lock poisoned"))?;
        sync::store_parsed(&conn, &account, &cached, &fetched)?;
        Ok(())
    }
    .await;
    let _ = session.logout().await;
    let _ = app.emit("mail-changed", ());
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_modified_utf7() {
        assert_eq!(decode_mutf7("INBOX"), "INBOX");
        assert_eq!(decode_mutf7("&BBoEPgRIBDgEOg-"), "Кошик");
        assert_eq!(decode_mutf7("Ko&AWE-"), "Koš");
        assert_eq!(decode_mutf7("Tom &- Jerry"), "Tom & Jerry");
        assert_eq!(decode_mutf7("broken &abc"), "broken &abc");
    }

    #[test]
    fn labels_drop_server_prefixes() {
        assert_eq!(label_of("INBOX.Work.Clients", Some(".")), "Work / Clients");
        assert_eq!(label_of("[Gmail]/Spam", Some("/")), "Spam");
        assert_eq!(label_of("INBOX", Some(".")), "INBOX");
        assert_eq!(label_of("Projects", None), "Projects");
    }

    #[test]
    fn roles_from_names_when_unflagged() {
        assert_eq!(role_of("INBOX", &[]), MailboxRole::Inbox);
        assert_eq!(role_of("Deleted Items", &[]), MailboxRole::Trash);
        assert_eq!(role_of("Junk E-mail", &[]), MailboxRole::Junk);
        assert_eq!(role_of("INBOX.Drafts", &[]), MailboxRole::Drafts);
        assert_eq!(role_of("Receipts", &[]), MailboxRole::Other);
        assert_eq!(
            role_of("Papierkorb", &[NameAttribute::Trash]),
            MailboxRole::Trash
        );
    }
}
