//! Moves messages the user deleted (and did not undo) to the account's Trash folder.

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_imap::types::NameAttribute;
use futures_util::TryStreamExt;
use rusqlite::Connection;

use crate::auth;
use crate::connect::{ImapSession, open_imap};
use crate::db::{self, PendingDelete};
use crate::error::{AppError, ErrorCode, Result};
use crate::model::Account;

/// How long a deletion can be undone before it reaches the server.
pub const UNDO_WINDOW_SECS: i64 = 10;

pub(crate) const TRASH_NAMES: &[&str] = &[
    "Trash",
    "Deleted Items",
    "Deleted Messages",
    "Deleted",
    "INBOX.Trash",
    "INBOX/Trash",
    "[Gmail]/Trash",
    "[Gmail]/Bin",
    "Kôš",
    "Koš",
    "Кошик",
];

fn imap_err(context: &str, e: impl std::fmt::Display) -> AppError {
    AppError::new(ErrorCode::ImapConnect, format!("{context}: {e}"))
}

async fn trash_folder(session: &mut ImapSession) -> Result<Option<String>> {
    let names: Vec<_> = session
        .list(Some(""), Some("*"))
        .await
        .map_err(|e| imap_err("LIST", e))?
        .try_collect()
        .await
        .map_err(|e| imap_err("LIST", e))?;
    if let Some(n) = names
        .iter()
        .find(|n| n.attributes().contains(&NameAttribute::Trash))
    {
        return Ok(Some(n.name().to_owned()));
    }
    Ok(TRASH_NAMES.iter().find_map(|want| {
        names
            .iter()
            .find(|n| n.name().eq_ignore_ascii_case(want))
            .map(|n| n.name().to_owned())
    }))
}

fn by_folder(pending: &[PendingDelete]) -> BTreeMap<&str, Vec<&PendingDelete>> {
    let mut map: BTreeMap<&str, Vec<&PendingDelete>> = BTreeMap::new();
    for p in pending {
        map.entry(p.folder.as_str()).or_default().push(p);
    }
    map
}

/// Commits deletions older than the undo window. Returns how many messages were moved.
pub async fn commit(db: &Mutex<Connection>, account: &Account) -> Result<usize> {
    let lock = || {
        db.lock()
            .map_err(|_| AppError::new(ErrorCode::Storage, "Database lock poisoned"))
    };
    let before = crate::oauth::now() - UNDO_WINDOW_SECS;
    let pending = db::pending_deletions(&*lock()?, &account.id, before)?;
    if pending.is_empty() {
        return Ok(0);
    }

    let creds = auth::credentials(account).await?;
    let mut session = open_imap(&account.imap, creds.login()).await?;
    let caps = session
        .capabilities()
        .await
        .map_err(|e| imap_err("CAPABILITY", e))?;
    let (can_move, uidplus) = (caps.has_str("MOVE"), caps.has_str("UIDPLUS"));
    let trash = trash_folder(&mut session).await?;

    let mut done = 0;
    for (folder, items) in by_folder(&pending) {
        let set = items
            .iter()
            .map(|p| p.uid.to_string())
            .collect::<Vec<_>>()
            .join(",");
        session
            .select(folder)
            .await
            .map_err(|e| imap_err(folder, e))?;
        match trash.as_deref().filter(|t| *t != folder) {
            Some(trash) if can_move => session
                .uid_mv(&set, trash)
                .await
                .map_err(|e| imap_err("MOVE", e))?,
            other => {
                if let Some(trash) = other {
                    session
                        .uid_copy(&set, trash)
                        .await
                        .map_err(|e| imap_err("COPY", e))?;
                }
                // Without a Trash folder (or already in it) this deletes for good.
                session
                    .uid_store(&set, "+FLAGS.SILENT (\\Deleted)")
                    .await
                    .map_err(|e| imap_err("STORE", e))?
                    .try_collect::<Vec<_>>()
                    .await
                    .map_err(|e| imap_err("STORE", e))?;
                if uidplus {
                    session
                        .uid_expunge(&set)
                        .await
                        .map_err(|e| imap_err("EXPUNGE", e))?
                        .try_collect::<Vec<_>>()
                        .await
                        .map_err(|e| imap_err("EXPUNGE", e))?;
                } else {
                    session
                        .expunge()
                        .await
                        .map_err(|e| imap_err("EXPUNGE", e))?
                        .try_collect::<Vec<_>>()
                        .await
                        .map_err(|e| imap_err("EXPUNGE", e))?;
                }
            }
        }
        let ids: Vec<i64> = items.iter().map(|p| p.id).collect();
        db::forget_messages(&*lock()?, &ids)?;
        done += ids.len();
    }
    let _ = session.logout().await;
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_pending_by_folder() {
        let p = |id, folder: &str, uid| PendingDelete {
            id,
            folder: folder.into(),
            uid,
        };
        let pending = [p(1, "INBOX", 5), p(2, "Sent", 9), p(3, "INBOX", 7)];
        let map = by_folder(&pending);
        assert_eq!(
            map["INBOX"].iter().map(|p| p.uid).collect::<Vec<_>>(),
            [5, 7]
        );
        assert_eq!(map["Sent"].len(), 1);
    }
}
