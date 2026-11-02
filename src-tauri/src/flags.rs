//! Copies read/unread changes made in Mailbub to the server (the \Seen flag), so other
//! mail apps agree. Changes wait in the DB (`seen_dirty`) until this succeeds; sync retries.

use std::collections::BTreeMap;
use std::sync::Mutex;

use futures_util::TryStreamExt;
use rusqlite::Connection;

use crate::auth;
use crate::connect::open_imap;
use crate::db::{self, SeenChange};
use crate::error::{AppError, ErrorCode, Result};
use crate::model::Account;

fn imap_err(context: &str, e: impl std::fmt::Display) -> AppError {
    AppError::new(ErrorCode::ImapConnect, format!("{context}: {e}"))
}

/// One UID STORE per folder and direction.
fn batches(pending: &[SeenChange]) -> BTreeMap<(&str, bool), Vec<&SeenChange>> {
    let mut map: BTreeMap<(&str, bool), Vec<&SeenChange>> = BTreeMap::new();
    for p in pending {
        map.entry((p.folder.as_str(), p.seen)).or_default().push(p);
    }
    map
}

/// Pushes every pending change of one account. Returns how many messages were updated.
pub async fn push_seen(db: &Mutex<Connection>, account: &Account) -> Result<usize> {
    let lock = || {
        db.lock()
            .map_err(|_| AppError::new(ErrorCode::Storage, "Database lock poisoned"))
    };
    let pending = db::pending_seen(&*lock()?, &account.id)?;
    if pending.is_empty() {
        return Ok(0);
    }
    let creds = auth::credentials(account).await?;
    let mut session = open_imap(&account.imap, creds.login()).await?;
    let mut done = 0;
    let mut selected: Option<&str> = None;
    for ((folder, seen), items) in batches(&pending) {
        if selected != Some(folder) {
            // SELECT (not EXAMINE): flags can only change in a read-write mailbox.
            session
                .select(folder)
                .await
                .map_err(|e| imap_err(folder, e))?;
            selected = Some(folder);
        }
        let set = items
            .iter()
            .map(|p| p.uid.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let op = if seen {
            "+FLAGS.SILENT"
        } else {
            "-FLAGS.SILENT"
        };
        session
            .uid_store(&set, format!("{op} (\\Seen)"))
            .await
            .map_err(|e| imap_err("STORE", e))?
            .try_collect::<Vec<_>>()
            .await
            .map_err(|e| imap_err("STORE", e))?;
        let conn = lock()?;
        for p in &items {
            db::seen_pushed(&conn, p.id, p.seen)?;
        }
        done += items.len();
    }
    let _ = session.logout().await;
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batches_by_folder_and_direction() {
        let c = |id, folder: &str, uid, seen| SeenChange {
            id,
            folder: folder.into(),
            uid,
            seen,
        };
        let pending = [
            c(1, "INBOX", 5, true),
            c(2, "INBOX", 6, false),
            c(3, "Archive", 9, true),
            c(4, "INBOX", 7, true),
        ];
        let map = batches(&pending);
        assert_eq!(map.len(), 3);
        assert_eq!(
            map[&("INBOX", true)]
                .iter()
                .map(|p| p.uid)
                .collect::<Vec<_>>(),
            [5, 7]
        );
        assert_eq!(map[&("INBOX", false)].len(), 1);
        let order: Vec<_> = map.keys().map(|(f, _)| *f).collect();
        assert_eq!(
            order,
            ["Archive", "INBOX", "INBOX"],
            "same folder stays together"
        );
    }
}
