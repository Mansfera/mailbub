//! Pulls mail into SQLite. Gmail: "All Mail" (inbox, archive and sent in one folder).
//! Other servers: INBOX, Sent and Archive. First run takes the newest `INITIAL_MAX` messages
//! per folder (many people keep INBOX empty, so a date window can come back empty), or
//! everything since a date when one is given (re-downloading after clearing the cache). After
//! that only UIDs above the last one seen, restarting a folder if its UIDVALIDITY changes.
//! Older mail is fetched on demand (`backfill_account`), `OLDER_BATCH` UIDs below the
//! lowest one cached at a time.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use async_imap::types::{Flag, NameAttribute};
use futures_util::{StreamExt, TryStreamExt};
use rusqlite::Connection;

use crate::auth;
use crate::connect::{ImapSession, open_imap};
use crate::db::{self, FolderState};
use crate::error::{AppError, ErrorCode, Result};
use crate::model::Account;
use crate::parse;

const INITIAL_MAX: usize = 300;
/// Older messages fetched per folder each time the user asks for more.
const OLDER_BATCH: usize = 200;
const BATCH: usize = 25;

pub(crate) const SENT_NAMES: &[&str] = &[
    "Sent",
    "Sent Items",
    "Sent Messages",
    "Sent Mail",
    "INBOX.Sent",
    "INBOX/Sent",
    "[Gmail]/Sent Mail",
    "Odoslané",
    "Odeslaná pošta",
    "Надіслані",
];

fn imap_err(context: &str, e: impl std::fmt::Display) -> AppError {
    AppError::new(ErrorCode::ImapConnect, format!("{context}: {e}"))
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// A Unix time as an IMAP search date (`2-Oct-2026`, UTC day).
fn imap_date(unix: i64) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = unix.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{day}-{}-{year}", MONTHS[(month - 1) as usize])
}

pub(crate) const ARCHIVE_NAMES: &[&str] = &[
    "Archive",
    "Archives",
    "INBOX.Archive",
    "INBOX/Archive",
    "Archív",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Special {
    All,
    Sent,
    Archive,
    Other,
}

pub(crate) fn special(attrs: &[NameAttribute<'_>]) -> Special {
    if attrs.contains(&NameAttribute::All) {
        Special::All
    } else if attrs.contains(&NameAttribute::Sent) {
        Special::Sent
    } else if attrs.contains(&NameAttribute::Archive) {
        Special::Archive
    } else {
        Special::Other
    }
}

/// What a synced folder means for a chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Inbox,
    Sent,
    Archive,
    /// Gmail "All Mail": inbox membership comes from the \Inbox label.
    All,
}

impl Role {
    fn in_sent(self) -> bool {
        self == Role::Sent
    }
}

pub(crate) fn named_folder(
    folders: &[(String, Special)],
    kind: Special,
    names: &[&str],
) -> Option<String> {
    folders
        .iter()
        .find(|(_, s)| *s == kind)
        .map(|(n, _)| n.clone())
        .or_else(|| {
            names.iter().find_map(|want| {
                folders
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case(want))
                    .map(|(n, _)| n.clone())
            })
        })
}

pub(crate) fn archive_folder(folders: &[(String, Special)]) -> Option<String> {
    named_folder(folders, Special::Archive, ARCHIVE_NAMES)
}

pub(crate) fn sent_folder(folders: &[(String, Special)]) -> Option<String> {
    named_folder(folders, Special::Sent, SENT_NAMES)
}

pub(crate) fn all_mail_folder(folders: &[(String, Special)]) -> Option<String> {
    named_folder(folders, Special::All, &[])
}

/// Which folders to sync and their role. Special-use flags (RFC 6154) win over
/// well-known names. INBOX comes first so moves out of it are seen before arrivals elsewhere.
pub(crate) fn choose_folders(folders: &[(String, Special)]) -> Vec<(String, Role)> {
    let flagged = |kind| {
        folders
            .iter()
            .find(|(_, s)| *s == kind)
            .map(|(n, _)| n.clone())
    };
    let named = |names: &[&str]| {
        names.iter().find_map(|want| {
            folders
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(want))
                .map(|(n, _)| n.clone())
        })
    };
    if let Some(all) = flagged(Special::All) {
        return vec![(all, Role::All)];
    }
    let mut out = vec![("INBOX".to_owned(), Role::Inbox)];
    if let Some(sent) = flagged(Special::Sent).or_else(|| named(SENT_NAMES)) {
        out.push((sent, Role::Sent));
    }
    if let Some(archive) = flagged(Special::Archive).or_else(|| named(ARCHIVE_NAMES)) {
        out.push((archive, Role::Archive));
    }
    out
}

pub(crate) async fn list_folders(session: &mut ImapSession) -> Result<Vec<(String, Special)>> {
    let names: Vec<_> = session
        .list(Some(""), Some("*"))
        .await
        .map_err(|e| imap_err("LIST", e))?
        .try_collect()
        .await
        .map_err(|e| imap_err("LIST", e))?;
    let folders: Vec<(String, Special)> = names
        .iter()
        .filter(|n| !n.attributes().contains(&NameAttribute::NoSelect))
        .map(|n| (n.name().to_owned(), special(n.attributes())))
        .collect();
    Ok(folders)
}

pub(crate) fn own_addresses(account: &Account) -> Vec<String> {
    let mut own = vec![
        account.email.to_lowercase(),
        account.username.to_lowercase(),
    ];
    own.dedup();
    own.extend(account.aliases.iter().cloned());
    own
}

pub(crate) fn has_inbox_label(f: &async_imap::types::Fetch) -> bool {
    f.gmail_labels()
        .is_some_and(|labels| labels.iter().any(|l| is_inbox_label(l)))
}

/// Gmail sends system labels quoted (`"\\Inbox"`) and the parser keeps the escape, so
/// unescape before comparing. Missing this made every Gmail chat look archived.
fn is_inbox_label(label: &str) -> bool {
    label.replace("\\\\", "\\").eq_ignore_ascii_case("\\Inbox")
}

/// Notices what changed on the server for messages we already have: gone (moved to
/// another folder, deleted) or, on Gmail, archived/unarchived via the \Inbox label.
/// Returns how many local messages changed.
async fn reconcile_folder(
    session: &mut ImapSession,
    db: &Mutex<Connection>,
    account: &Account,
    folder: &str,
    role: Role,
) -> Result<usize> {
    let lock = || {
        db.lock()
            .map_err(|_| AppError::new(ErrorCode::Storage, "Database lock poisoned"))
    };
    let Some(known) = db::folder_state(&*lock()?, &account.id, folder)? else {
        return Ok(0);
    };
    let mbox = session
        .examine(folder)
        .await
        .map_err(|e| imap_err(folder, e))?;
    if mbox.uid_validity.unwrap_or(0) != known.uid_validity {
        return Ok(0); // sync_folder starts this folder over
    }
    let local = db::folder_uids(&*lock()?, &account.id, folder)?;
    let mut present: HashMap<u32, bool> = HashMap::new();
    let query = if role == Role::All {
        "(UID X-GM-LABELS)"
    } else {
        "(UID)"
    };
    for chunk in local.chunks(500) {
        let set = chunk
            .iter()
            .map(|(_, uid)| uid.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let mut stream = session
            .uid_fetch(&set, query)
            .await
            .map_err(|e| imap_err(folder, e))?;
        while let Some(item) = stream.next().await {
            let f = item.map_err(|e| imap_err(folder, e))?;
            if let Some(uid) = f.uid {
                present.insert(uid, has_inbox_label(&f));
            }
        }
    }
    let conn = lock()?;
    let gone: Vec<i64> = local
        .iter()
        .filter(|(_, uid)| !present.contains_key(uid))
        .map(|(id, _)| *id)
        .collect();
    db::forget_messages(&conn, &gone)?;
    let mut changed = gone.len();
    if role == Role::All {
        for (id, uid) in &local {
            if let Some(in_inbox) = present.get(uid)
                && db::set_in_inbox(&conn, *id, *in_inbox)?
            {
                changed += 1;
            }
        }
    }
    Ok(changed)
}

/// Messages fetched from one batch of UIDs, parsed and ready to store.
pub(crate) struct Fetched {
    parsed: Vec<(u32, bool, bool, parse::Parsed)>,
    fetched: usize,
    no_body: usize,
    unparsed: usize,
}

/// Downloads and parses the given UIDs (one UID FETCH). On Gmail inbox membership comes
/// from the \Inbox label, elsewhere from the folder's role.
pub(crate) async fn fetch_parsed(
    session: &mut ImapSession,
    folder: &str,
    role: Role,
    own: &[String],
    uids: &[u32],
) -> Result<Fetched> {
    let set = uids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let query = if role == Role::All {
        "(UID FLAGS X-GM-LABELS BODY.PEEK[])"
    } else {
        "(UID FLAGS BODY.PEEK[])"
    };
    let mut out = Fetched {
        parsed: Vec::with_capacity(uids.len()),
        fetched: 0,
        no_body: 0,
        unparsed: 0,
    };
    let mut stream = session
        .uid_fetch(&set, query)
        .await
        .map_err(|e| imap_err(folder, e))?;
    while let Some(item) = stream.next().await {
        let f = item.map_err(|e| imap_err(folder, e))?;
        let Some(uid) = f.uid else { continue };
        out.fetched += 1;
        let seen = f.flags().any(|fl| fl == Flag::Seen);
        let in_inbox = match role {
            Role::Inbox => true,
            Role::All => has_inbox_label(&f),
            Role::Sent | Role::Archive => false,
        };
        let Some(raw) = f.body() else {
            out.no_body += 1;
            continue;
        };
        match parse::parse(raw, own, role.in_sent(), now()) {
            Some(m) => out.parsed.push((uid, seen, in_inbox, m)),
            None => out.unparsed += 1,
        }
    }
    Ok(out)
}

/// Stores fetched messages; returns how many were new.
pub(crate) fn store_parsed(
    conn: &Connection,
    account: &Account,
    folder: &str,
    f: &Fetched,
) -> Result<usize> {
    let mut added = 0;
    for (uid, seen, in_inbox, m) in &f.parsed {
        if db::insert_message(conn, &account.id, folder, *uid, *seen, *in_inbox, m)? {
            added += 1;
        }
    }
    Ok(added)
}

/// `since` (Unix seconds) replaces the newest-`INITIAL_MAX` rule for a folder that has no
/// sync state yet: everything from that day on is fetched.
async fn sync_folder(
    session: &mut ImapSession,
    db: &Mutex<Connection>,
    account: &Account,
    folder: &str,
    role: Role,
    since: Option<i64>,
) -> Result<usize> {
    let lock = || {
        db.lock()
            .map_err(|_| AppError::new(ErrorCode::Storage, "Database lock poisoned"))
    };
    // EXAMINE is read-only, so syncing never marks anything as read on the server.
    let mbox = session
        .examine(folder)
        .await
        .map_err(|e| imap_err(folder, e))?;
    let validity = mbox.uid_validity.unwrap_or(0);
    let next_uid = mbox.uid_next.unwrap_or(1);
    let known = db::folder_state(&*lock()?, &account.id, folder)?;
    let current = known.filter(|k| k.uid_validity == validity);

    let mut uids: Vec<u32> = match current {
        Some(k) => {
            if next_uid <= k.last_uid + 1 {
                return Ok(0);
            }
            session
                .uid_search(format!("UID {}:*", k.last_uid + 1))
                .await
                .map_err(|e| imap_err(folder, e))?
                .into_iter()
                // "n:*" always matches the newest message, even if it is old.
                .filter(|u| *u > k.last_uid)
                .collect()
        }
        None => {
            if known.is_some() {
                db::clear_folder(&*lock()?, &account.id, folder)?;
            }
            let query = since.map_or("ALL".to_owned(), |t| format!("SINCE {}", imap_date(t)));
            session
                .uid_search(query)
                .await
                .map_err(|e| imap_err(folder, e))?
                .into_iter()
                .collect()
        }
    };
    uids.sort_unstable();
    let truncated = since.is_none() && uids.len() > INITIAL_MAX;
    if truncated {
        uids.drain(..uids.len() - INITIAL_MAX);
    }
    // Where older mail starts for `backfill_folder`. Only used for a new folder or numbering;
    // `set_folder_state` keeps the stored value otherwise.
    let min_uid = match current {
        Some(k) => k.min_uid,
        None if truncated => uids[0],
        // Mail before the date may exist; a backfill finds out (and sets 1 if not).
        None if since.is_some() => uids.first().copied().unwrap_or(next_uid).max(1),
        None => 1,
    };

    let own = own_addresses(account);
    let mut added = 0;
    let (mut fetched, mut no_body, mut unparsed) = (0, 0, 0);
    let mut last_uid = current.map_or(0, |k| k.last_uid);
    for chunk in uids.chunks(BATCH) {
        let f = fetch_parsed(session, folder, role, &own, chunk).await?;
        fetched += f.fetched;
        no_body += f.no_body;
        unparsed += f.unparsed;
        let conn = lock()?;
        added += store_parsed(&conn, account, folder, &f)?;
        last_uid = last_uid.max(*chunk.last().unwrap_or(&0));
        db::set_folder_state(
            &conn,
            &account.id,
            folder,
            FolderState {
                uid_validity: validity,
                last_uid,
                min_uid,
            },
        )?;
    }
    eprintln!(
        "[sync] {} / {folder}: {} uids, {fetched} fetched, {no_body} without body, {unparsed} unparsable, {added} new",
        account.label,
        uids.len()
    );
    // Everything up to the newest message is covered now; older mail comes via backfill.
    db::set_folder_state(
        &*lock()?,
        &account.id,
        folder,
        FolderState {
            uid_validity: validity,
            last_uid: last_uid.max(next_uid.saturating_sub(1)),
            min_uid,
        },
    )?;
    Ok(added)
}

/// Picks the next batch below what the cache covers (`min_uid`): the newest `limit` UIDs
/// under it. Returns the batch and the new `min_uid` (1 once nothing older is left).
fn older_batch(mut below: Vec<u32>, min_uid: u32, limit: usize) -> (Vec<u32>, u32) {
    below.retain(|u| *u < min_uid);
    below.sort_unstable();
    if below.len() > limit {
        below.drain(..below.len() - limit);
        let next = below[0];
        (below, next)
    } else {
        (below, 1)
    }
}

/// Fetches the next `OLDER_BATCH` older messages of one folder. Returns (added, more left).
async fn backfill_folder(
    session: &mut ImapSession,
    db: &Mutex<Connection>,
    account: &Account,
    folder: &str,
    role: Role,
) -> Result<(usize, bool)> {
    let lock = || {
        db.lock()
            .map_err(|_| AppError::new(ErrorCode::Storage, "Database lock poisoned"))
    };
    let Some(known) = db::folder_state(&*lock()?, &account.id, folder)? else {
        // Not synced yet; the regular sync takes the newest mail first.
        return Ok((0, true));
    };
    if known.min_uid <= 1 {
        return Ok((0, false));
    }
    let mbox = session
        .examine(folder)
        .await
        .map_err(|e| imap_err(folder, e))?;
    if mbox.uid_validity.unwrap_or(0) != known.uid_validity {
        return Ok((0, true)); // renumbered: sync_folder starts this folder over
    }
    let below: Vec<u32> = session
        .uid_search(format!("UID 1:{}", known.min_uid - 1))
        .await
        .map_err(|e| imap_err(folder, e))?
        .into_iter()
        .collect();
    let (uids, min_uid) = older_batch(below, known.min_uid, OLDER_BATCH);

    let own = own_addresses(account);
    let mut added = 0;
    // Newest first, so an interrupted backfill still leaves a gap-free recent history.
    for chunk in uids.rchunks(BATCH) {
        let f = fetch_parsed(session, folder, role, &own, chunk).await?;
        let conn = lock()?;
        added += store_parsed(&conn, account, folder, &f)?;
        db::set_min_uid(&conn, &account.id, folder, known.uid_validity, chunk[0])?;
    }
    db::set_min_uid(&*lock()?, &account.id, folder, known.uid_validity, min_uid)?;
    eprintln!(
        "[backfill] {} / {folder}: {} older uids, {added} new, min uid {min_uid}",
        account.label,
        uids.len()
    );
    Ok((added, min_uid > 1))
}

/// How a backfill of one account went.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Backfill {
    pub account_id: String,
    pub added: usize,
    /// Older mail is still left on the server.
    pub more: bool,
    pub error: Option<String>,
}

/// Fetches the next batch of older mail in every synced folder of the account. Gmail uses
/// "All Mail" with its labels, exactly like `sync_account`.
pub async fn backfill_account(db: &Mutex<Connection>, account: &Account) -> Result<Backfill> {
    let done = Backfill {
        account_id: account.id.clone(),
        ..Default::default()
    };
    let pending = db
        .lock()
        .map_err(|_| AppError::new(ErrorCode::Storage, "Database lock poisoned"))
        .and_then(|c| db::has_older_mail(&c, &account.id))?;
    if !pending {
        return Ok(done); // no need to connect
    }
    let creds = auth::credentials(account).await?;
    let mut session = open_imap(&account.imap, creds.login()).await?;
    let folders = choose_folders(&list_folders(&mut session).await?);
    let mut out = done;
    for (folder, role) in &folders {
        let (added, more) = backfill_folder(&mut session, db, account, folder, *role).await?;
        out.added += added;
        out.more |= more;
    }
    let _ = session.logout().await;
    Ok(out)
}

/// Syncs one account; returns how many messages were added or changed. `since` limits the
/// first download of each folder (see `sync_folder`).
pub async fn sync_account(
    db: &Mutex<Connection>,
    account: &Account,
    since: Option<i64>,
) -> Result<usize> {
    let creds = auth::credentials(account).await?;
    let mut session = open_imap(&account.imap, creds.login()).await?;
    let folders = choose_folders(&list_folders(&mut session).await?);
    let mut added = 0;
    // Removals first, in every folder: a message moved INBOX → Archive must leave INBOX
    // before its copy in Archive is fetched (and the other way round).
    for (folder, role) in &folders {
        added += reconcile_folder(&mut session, db, account, folder, *role).await?;
    }
    for (folder, role) in &folders {
        added += sync_folder(&mut session, db, account, folder, *role, since).await?;
    }
    let _ = session.logout().await;
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(name: &str, s: Special) -> (String, Special) {
        (name.to_owned(), s)
    }

    #[test]
    fn older_batches_walk_down_to_the_first_message() {
        let uids: Vec<u32> = (1..=10).chain([500]).collect();
        // Takes the newest under min_uid; the server may also return min_uid itself.
        assert_eq!(older_batch(uids.clone(), 9, 3), (vec![6, 7, 8], 6));
        assert_eq!(older_batch(uids.clone(), 6, 5), (vec![1, 2, 3, 4, 5], 1));
        assert_eq!(older_batch(vec![9, 3, 5], 9, 3), (vec![3, 5], 1));
        assert_eq!(older_batch(vec![], 40, 3), (vec![], 1));
    }

    #[test]
    fn imap_dates() {
        assert_eq!(imap_date(0), "1-Jan-1970");
        assert_eq!(imap_date(951_782_400), "29-Feb-2000");
        assert_eq!(imap_date(1_790_000_000), "21-Sep-2026");
        assert_eq!(
            imap_date(1_790_000_000 + 86_399 - 1_790_000_000 % 86_400),
            "21-Sep-2026"
        );
    }

    #[test]
    fn gmail_uses_all_mail_only() {
        let folders = [
            f("INBOX", Special::Other),
            f("[Gmail]/All Mail", Special::All),
            f("[Gmail]/Sent Mail", Special::Sent),
        ];
        assert_eq!(
            choose_folders(&folders),
            [("[Gmail]/All Mail".to_owned(), Role::All)]
        );
    }

    fn labels(raw: &[u8]) -> Vec<String> {
        use async_imap::imap_proto::{AttributeValue, Response};
        let Ok((_, Response::Fetch(_, attrs))) = Response::parse(raw) else {
            panic!("not a FETCH response");
        };
        attrs
            .into_iter()
            .find_map(|a| match a {
                AttributeValue::GmailLabels(l) => {
                    Some(l.into_iter().map(|l| l.into_owned()).collect())
                }
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn inbox_label_quoted_or_bare() {
        // As Gmail sends it: system labels quoted with an escaped backslash.
        let quoted = labels(b"* 1 FETCH (UID 5 X-GM-LABELS (\"\\\\Important\" \"\\\\Inbox\"))\r\n");
        assert!(quoted.iter().any(|l| is_inbox_label(l)));
        let bare = labels(b"* 2 FETCH (UID 6 X-GM-LABELS (\\Inbox Work))\r\n");
        assert!(bare.iter().any(|l| is_inbox_label(l)));
        let archived = labels(b"* 3 FETCH (UID 7 X-GM-LABELS (\"\\\\Sent\" Inbox2))\r\n");
        assert!(!archived.iter().any(|l| is_inbox_label(l)));
    }

    #[test]
    fn other_servers_use_inbox_sent_archive() {
        let folders = [
            f("INBOX", Special::Other),
            f("Sent Items", Special::Other),
            f("Archive", Special::Archive),
            f("Junk", Special::Other),
        ];
        assert_eq!(
            choose_folders(&folders),
            [
                ("INBOX".to_owned(), Role::Inbox),
                ("Sent Items".to_owned(), Role::Sent),
                ("Archive".to_owned(), Role::Archive)
            ]
        );
        assert_eq!(choose_folders(&[f("INBOX", Special::Other)]).len(), 1);
        assert_eq!(archive_folder(&folders).as_deref(), Some("Archive"));
        assert_eq!(
            archive_folder(&[f("Archív", Special::Other)]).as_deref(),
            Some("Archív")
        );
    }
}
