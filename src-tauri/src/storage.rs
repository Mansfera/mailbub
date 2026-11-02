//! What Mailbub keeps on disk, and clearing it: the mail database in the app data folder,
//! downloaded attachments and profile pictures in the cache folder.

use std::path::Path;

use rusqlite::Connection;

use crate::db;
use crate::error::Result;

/// How far back mail is downloaded again after the cache is cleared.
pub const RESYNC_DAYS: i64 = 30;

/// Sizes in bytes, plus how many messages are cached.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageUsage {
    /// Mail and the search index.
    pub database: u64,
    pub attachments: u64,
    pub avatars: u64,
    pub messages: i64,
}

/// Total size of the files under `path`; symlinks are not followed.
fn dir_size(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

pub fn usage(conn: &Connection, data_dir: &Path, cache_dir: &Path) -> Result<StorageUsage> {
    let database = ["", "-wal", "-shm"]
        .iter()
        .filter_map(|suffix| std::fs::metadata(data_dir.join(format!("{}{suffix}", db::FILE))).ok())
        .map(|m| m.len())
        .sum();
    Ok(StorageUsage {
        database,
        attachments: dir_size(&cache_dir.join("attachments")),
        avatars: dir_size(&cache_dir.join("avatars")),
        messages: db::message_count(conn)?,
    })
}

/// Deletes downloaded attachments, profile pictures and leftover calendar files. Anything
/// else in the cache folder (the web view's own) is left alone.
pub fn clear_files(cache_dir: &Path) {
    for dir in ["attachments", "avatars"] {
        let _ = std::fs::remove_dir_all(cache_dir.join(dir));
    }
    let Ok(entries) = std::fs::read_dir(cache_dir) else {
        return;
    };
    for e in entries.filter_map(|e| e.ok()) {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("event-") && name.ends_with(".ics") {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clears_only_its_own_files() {
        let dir = std::env::temp_dir().join(format!("mailbub-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("attachments/1/a")).unwrap();
        std::fs::write(dir.join("attachments/1/a/file.pdf"), [0u8; 100]).unwrap();
        std::fs::create_dir_all(dir.join("avatars")).unwrap();
        std::fs::write(dir.join("avatars/ab"), [0u8; 20]).unwrap();
        std::fs::write(dir.join("event-1.ics"), "x").unwrap();
        std::fs::create_dir_all(dir.join("WebKit")).unwrap();
        std::fs::write(dir.join("WebKit/keep"), "x").unwrap();

        assert_eq!(dir_size(&dir.join("attachments")), 100);
        clear_files(&dir);
        assert!(!dir.join("attachments").exists());
        assert!(!dir.join("avatars").exists());
        assert!(!dir.join("event-1.ics").exists());
        assert!(dir.join("WebKit/keep").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
