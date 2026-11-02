//! Opening things outside the app: web/mail links in the default browser or mail app,
//! and attachments (fetched from the server on demand, cached, then opened).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;

use crate::auth;
use crate::connect::open_imap;
use crate::db;
use crate::error::{AppError, ErrorCode, Result};
use crate::parse;

/// The Kotlin side of the core (SystemPlugin.kt).
#[cfg(target_os = "android")]
pub struct AndroidSystem(pub tauri::plugin::PluginHandle<tauri::Wry>);

/// Hands a file or URL to the system, which picks the default app. On Android this waits for
/// the main thread, so never call it from there.
pub fn open_with_system(app: &tauri::AppHandle, target: &std::ffi::OsStr) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        let status = std::process::Command::new("/usr/bin/open")
            .arg(target)
            .status()
            .map_err(|e| AppError::new(ErrorCode::Invalid, format!("Could not open: {e}")))?;
        if status.success() {
            Ok(())
        } else {
            Err(AppError::new(ErrorCode::Invalid, "No app could open this"))
        }
    }
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;
        app.state::<AndroidSystem>()
            .0
            .run_mobile_plugin::<serde_json::Value>(
                "open",
                serde_json::json!({ "target": target.to_string_lossy() }),
            )
            .map(|_| ())
            .map_err(|e| AppError::new(ErrorCode::Invalid, format!("Could not open: {e}")))
    }
    #[cfg(not(any(target_os = "macos", target_os = "android")))]
    {
        let _ = (app, target);
        Err(AppError::new(
            ErrorCode::Invalid,
            "Opening files is not supported on this platform yet",
        ))
    }
}

/// Only links that cannot run anything locally: web, mail, phone.
pub fn is_safe_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    ["https://", "http://", "mailto:", "tel:"]
        .iter()
        .any(|p| lower.starts_with(p))
}

/// A navigation inside a webview that belongs in the browser or mail app instead: a link
/// clicked in an email frame. The app itself (localhost in dev, *.localhost schemes), the
/// map preview frame and about:srcdoc stay in place.
pub fn leaves_app(url: &tauri::Url) -> bool {
    if !is_safe_url(url.as_str()) {
        return false;
    }
    match url.host_str() {
        Some(host) => {
            !(host == "localhost"
                || host.ends_with(".localhost")
                || host == "www.openstreetmap.org")
        }
        // mailto: and tel: have no host.
        None => true,
    }
}

/// Keeps a server-supplied file name from escaping the cache folder.
fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '\0') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_start_matches('.').to_owned();
    if trimmed.is_empty() {
        "attachment".into()
    } else {
        trimmed
    }
}

/// The cache folder of one attachment; it holds the file under its own name.
fn attachment_dir(cache: &Path, message_id: i64, attachment_id: &str) -> PathBuf {
    cache
        .join("attachments")
        .join(message_id.to_string())
        .join(safe_file_name(attachment_id))
}

/// Saves an attachment where `fetch_attachment` looks first.
pub fn cache_attachment(
    cache: &Path,
    message_id: i64,
    attachment_id: &str,
    name: &str,
    bytes: &[u8],
) -> Result<PathBuf> {
    let dir = attachment_dir(cache, message_id, attachment_id);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(safe_file_name(name));
    std::fs::write(&path, bytes)?;
    Ok(path)
}

/// Fetches the message from IMAP (once), saves the attachment and returns its path.
pub async fn fetch_attachment(
    db: &Mutex<Connection>,
    cache: &Path,
    message_id: i64,
    attachment_id: &str,
) -> Result<PathBuf> {
    let lock = || {
        db.lock()
            .map_err(|_| AppError::new(ErrorCode::Storage, "Database lock poisoned"))
    };
    let (account_id, folder, uid) = db::message_location(&*lock()?, message_id)?;
    let dir = attachment_dir(cache, message_id, attachment_id);
    if let Some(existing) = std::fs::read_dir(&dir)
        .ok()
        .and_then(|mut d| d.next())
        .and_then(|e| e.ok())
    {
        return Ok(existing.path());
    }

    let account = db::get_account(&*lock()?, &account_id)?;
    let creds = auth::credentials(&account).await?;
    let mut session = open_imap(&account.imap, creds.login()).await?;
    session
        .examine(&folder)
        .await
        .map_err(|e| AppError::new(ErrorCode::ImapConnect, format!("{folder}: {e}")))?;
    let raw = {
        use futures_util::StreamExt;
        let mut stream = session
            .uid_fetch(uid.to_string(), "BODY.PEEK[]")
            .await
            .map_err(|e| AppError::new(ErrorCode::ImapConnect, e.to_string()))?;
        let mut raw = None;
        while let Some(item) = stream.next().await {
            let f = item.map_err(|e| AppError::new(ErrorCode::ImapConnect, e.to_string()))?;
            if let Some(body) = f.body() {
                raw = Some(body.to_vec());
            }
        }
        raw
    };
    let _ = session.logout().await;
    let raw = raw.ok_or_else(|| {
        AppError::new(
            ErrorCode::NotFound,
            "The message is no longer on the server",
        )
    })?;
    let (name, bytes) = parse::attachment_file(&raw, attachment_id)
        .ok_or_else(|| AppError::new(ErrorCode::NotFound, "Attachment not found"))?;

    cache_attachment(cache, message_id, attachment_id, &name, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_local_and_script_urls() {
        assert!(is_safe_url("https://meet.google.com/abc"));
        assert!(is_safe_url("MAILTO:a@b.c"));
        assert!(!is_safe_url("file:///etc/passwd"));
        assert!(!is_safe_url("javascript:alert(1)"));
        assert!(!is_safe_url("x-apple.systempreferences:foo"));
    }

    #[test]
    fn email_links_leave_the_app() {
        let leaves = |u: &str| leaves_app(&u.parse().unwrap());
        assert!(leaves(
            "https://c.vialoops.com/CL0/https:%2F%2Fwww.theacademysf.com%2F/1/abc"
        ));
        assert!(leaves("mailto:a@b.c"));
        assert!(!leaves("about:srcdoc"));
        assert!(!leaves("tauri://localhost/index.html"));
        assert!(!leaves("http://localhost:1420/"));
        assert!(!leaves("http://ipc.localhost/open_external"));
        assert!(!leaves(
            "https://www.openstreetmap.org/export/embed.html?bbox=1"
        ));
        assert!(!leaves("file:///etc/passwd"));
    }

    #[test]
    fn file_names_stay_inside_the_folder() {
        assert_eq!(safe_file_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(safe_file_name("Zmluva Alex.pdf"), "Zmluva Alex.pdf");
        assert_eq!(safe_file_name(".."), "attachment");
    }
}
