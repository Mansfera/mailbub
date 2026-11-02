//! Archiving a chat on the server: Gmail drops the \Inbox label; other servers move the
//! messages between INBOX and the Archive folder (created if missing). The next sync
//! confirms it, and picks up archiving done in other mail apps the same way.

use std::sync::Mutex;

use futures_util::TryStreamExt;
use rusqlite::Connection;

use crate::auth;
use crate::connect::{ImapSession, open_imap};
use crate::db::{self, MessageLoc};
use crate::error::{AppError, ErrorCode, Result};
use crate::model::Account;
use crate::sync::{all_mail_folder, archive_folder, list_folders};

fn imap_err(context: &str, e: impl std::fmt::Display) -> AppError {
    AppError::new(ErrorCode::ImapConnect, format!("{context}: {e}"))
}

pub(crate) fn uid_set(locs: &[&MessageLoc]) -> String {
    locs.iter()
        .map(|l| l.uid.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// Message-IDs go into a quoted IMAP string; skip any that could break out of it.
pub(crate) fn searchable(id: &str) -> bool {
    !id.is_empty() && !id.contains(['"', '\\', '\r', '\n'])
}

/// Moves UIDs from one folder to another: MOVE where the server has it, else
/// COPY + \Deleted + EXPUNGE.
pub(crate) async fn imap_move(
    session: &mut ImapSession,
    set: &str,
    from: &str,
    to: &str,
) -> Result<()> {
    let caps = session
        .capabilities()
        .await
        .map_err(|e| imap_err("CAPABILITY", e))?;
    session.select(from).await.map_err(|e| imap_err(from, e))?;
    if caps.has_str("MOVE") {
        session
            .uid_mv(set, to)
            .await
            .map_err(|e| imap_err("MOVE", e))?;
    } else {
        session
            .uid_copy(set, to)
            .await
            .map_err(|e| imap_err("COPY", e))?;
        session
            .uid_store(set, "+FLAGS.SILENT (\\Deleted)")
            .await
            .map_err(|e| imap_err("STORE", e))?
            .try_collect::<Vec<_>>()
            .await
            .map_err(|e| imap_err("STORE", e))?;
        session
            .uid_expunge(set)
            .await
            .map_err(|e| imap_err("EXPUNGE", e))?
            .try_collect::<Vec<_>>()
            .await
            .map_err(|e| imap_err("EXPUNGE", e))?;
    }
    Ok(())
}

/// Moves stored messages and keeps the cache pointing at them. `track`: the destination is
/// synced, so each row learns its new UID; otherwise the rows are dropped from the cache.
pub(crate) async fn move_messages(
    session: &mut ImapSession,
    db: &Mutex<Connection>,
    locs: &[&MessageLoc],
    from: &str,
    to: &str,
    track: bool,
) -> Result<()> {
    if locs.is_empty() {
        return Ok(());
    }
    let lock = || {
        db.lock()
            .map_err(|_| AppError::new(ErrorCode::Storage, "Database lock poisoned"))
    };
    imap_move(session, &uid_set(locs), from, to).await?;
    if !track {
        let ids: Vec<i64> = locs.iter().map(|l| l.id).collect();
        return db::forget_messages(&*lock()?, &ids);
    }

    // Learn the new UIDs so later actions (and sync) keep pointing at the right message.
    session.select(to).await.map_err(|e| imap_err(to, e))?;
    let in_inbox = to.eq_ignore_ascii_case("INBOX");
    for loc in locs {
        let new_uid = match loc.message_id.as_deref().filter(|id| searchable(id)) {
            Some(id) => session
                .uid_search(format!("HEADER Message-ID \"{id}\""))
                .await
                .ok()
                .and_then(|found| found.into_iter().max()),
            None => None,
        };
        let conn = lock()?;
        match new_uid {
            Some(uid) => db::relocate(&conn, loc.id, to, uid, in_inbox)?,
            // Not found: drop it here; sync fetches it again from the new folder.
            None => db::forget_messages(&conn, &[loc.id])?,
        }
    }
    Ok(())
}

async fn ensure_archive(session: &mut ImapSession) -> Result<String> {
    let folders = list_folders(session).await?;
    if let Some(name) = archive_folder(&folders) {
        return Ok(name);
    }
    // Some servers only allow folders under INBOX.
    for name in ["Archive", "INBOX.Archive"] {
        if session.create(name).await.is_ok() {
            return Ok(name.to_owned());
        }
    }
    Err(AppError::new(
        ErrorCode::ImapConnect,
        "Could not create an Archive folder",
    ))
}

/// Archives (or brings back) every message of a chat on the server.
pub async fn set_archived(
    db: &Mutex<Connection>,
    account: &Account,
    chat_key: &str,
    archive: bool,
) -> Result<()> {
    let locs = {
        let conn = db
            .lock()
            .map_err(|_| AppError::new(ErrorCode::Storage, "Database lock poisoned"))?;
        let locs = db::chat_locations(&conn, chat_key)?;
        // Only your own mail and none of it in the inbox: nothing to move on the server.
        if locs.iter().all(|l| l.outgoing && !l.in_inbox) {
            return db::set_local_archived(&conn, chat_key, archive);
        }
        locs
    };
    let creds = auth::credentials(account).await?;
    let mut session = open_imap(&account.imap, creds.login()).await?;
    let folders = list_folders(&mut session).await?;

    let result = if let Some(all) = all_mail_folder(&folders) {
        // Gmail: archive = remove the \Inbox label; unarchive = put it back on incoming mail.
        let targets: Vec<&MessageLoc> = locs
            .iter()
            .filter(|l| l.folder == all && if archive { l.in_inbox } else { !l.outgoing })
            .collect();
        if targets.is_empty() {
            Ok(())
        } else {
            session.select(&all).await.map_err(|e| imap_err(&all, e))?;
            let op = if archive {
                "-X-GM-LABELS"
            } else {
                "+X-GM-LABELS"
            };
            session
                .uid_store(uid_set(&targets), format!("{op} (\\Inbox)"))
                .await
                .map_err(|e| imap_err("STORE", e))?
                .try_collect::<Vec<_>>()
                .await
                .map_err(|e| imap_err("STORE", e))?;
            let conn = db
                .lock()
                .map_err(|_| AppError::new(ErrorCode::Storage, "Database lock poisoned"))?;
            for l in &targets {
                db::set_in_inbox(&conn, l.id, !archive)?;
            }
            Ok(())
        }
    } else if archive {
        let dest = ensure_archive(&mut session).await?;
        let from_inbox: Vec<&MessageLoc> = locs.iter().filter(|l| l.folder == "INBOX").collect();
        move_messages(&mut session, db, &from_inbox, "INBOX", &dest, true).await
    } else {
        match archive_folder(&folders) {
            Some(src) => {
                let back: Vec<&MessageLoc> = locs
                    .iter()
                    .filter(|l| l.folder == src && !l.outgoing)
                    .collect();
                move_messages(&mut session, db, &back, &src, "INBOX", true).await
            }
            None => Ok(()),
        }
    };
    let _ = session.logout().await;
    result
}

#[cfg(test)]
mod tests {
    use super::searchable;

    #[test]
    fn only_plain_message_ids_are_searched() {
        assert!(searchable("CAF1x@mail.gmail.com"));
        assert!(!searchable("a\"b@x"));
        assert!(!searchable("a\r\nb"));
        assert!(!searchable(""));
    }
}
