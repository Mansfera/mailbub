use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::future::join_all;
use rusqlite::Connection;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::connect::{self, Login};
use crate::error::{AppError, ErrorCode, Result};
use crate::google;
use crate::model::{
    ACCOUNT_COLORS, Account, AccountDraft, AccountUpdate, AddAccountResult, AuthKind, ChatState,
    ChatSummary, Discovered, ImportCandidate, Message, Provider, SecretInput, SyncError,
    SyncStatus,
};
use crate::oauth::{self, BrowserStart, Sessions};
use crate::secrets::{self, StoredSecret};
use crate::storage::{self, StorageUsage};
use crate::{db, discover, flags, mail_import, send, sync, trash};

pub struct AppState {
    pub db: Mutex<Connection>,
    pub oauth: Arc<tokio::sync::Mutex<Sessions>>,
    pub sync_status: Mutex<SyncStatus>,
    /// Held while a sync runs, so runs never overlap (and while archiving, see `archive_chat`).
    pub sync_lock: tokio::sync::Mutex<()>,
    /// Held while older mail is fetched, so scrolling can't start a second backfill.
    pub backfill_lock: tokio::sync::Mutex<()>,
    pub geocoder: crate::calendar::Geocoder,
}

impl AppState {
    fn db(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.db
            .lock()
            .map_err(|_| AppError::new(ErrorCode::Storage, "Database lock poisoned"))
    }
}

#[tauri::command]
pub fn list_accounts(state: State<'_, AppState>) -> Result<Vec<Account>> {
    db::list_accounts(&*state.db()?)
}

#[tauri::command]
pub async fn discover_servers(email: String) -> Result<Discovered> {
    if !looks_like_email(&email) {
        return Err(AppError::new(
            ErrorCode::Invalid,
            "Enter a full email address",
        ));
    }
    Ok(discover::discover(email.trim()).await)
}

#[tauri::command]
pub fn ms_oauth_available() -> bool {
    oauth::client_id().is_some()
}

/// Opens a loopback listener and returns the sign-in URL for the UI to open in the browser.
#[tauri::command]
pub async fn ms_oauth_start(state: State<'_, AppState>, email: String) -> Result<BrowserStart> {
    oauth::start(state.oauth.clone(), email).await
}

#[tauri::command]
pub fn google_oauth_available() -> bool {
    google::credentials().is_some()
}

/// Opens a loopback listener and returns the consent URL for the UI to open in the browser.
#[tauri::command]
pub async fn google_oauth_start(state: State<'_, AppState>, email: String) -> Result<BrowserStart> {
    google::start(state.oauth.clone(), email).await
}

#[tauri::command]
pub async fn oauth_poll(state: State<'_, AppState>, session: String) -> Result<()> {
    oauth::poll(&state.oauth, &session).await
}

/// Accounts configured in Apple Mail, as setup suggestions (no passwords).
#[tauri::command]
pub async fn list_mail_app_accounts(state: State<'_, AppState>) -> Result<Vec<ImportCandidate>> {
    let connected = db::list_accounts(&*state.db()?)?
        .into_iter()
        .map(|a| a.email)
        .collect();
    mail_import::list(connected).await
}

fn looks_like_email(s: &str) -> bool {
    let s = s.trim();
    matches!(s.split_once('@'), Some((user, domain)) if !user.is_empty() && domain.contains('.') && !s.contains(char::is_whitespace))
}

fn validate(draft: &AccountDraft) -> Result<()> {
    if !looks_like_email(&draft.email) {
        return Err(AppError::new(
            ErrorCode::Invalid,
            "Enter a full email address",
        ));
    }
    for s in [&draft.imap, &draft.smtp] {
        if s.host.trim().is_empty() || s.port == 0 {
            return Err(AppError::new(
                ErrorCode::Invalid,
                "Server host and port are required",
            ));
        }
    }
    Ok(())
}

/// Aliases as stored: lowercase, without the main address or repeats.
fn clean_aliases(email: &str, aliases: &[String]) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for alias in aliases {
        let alias = alias.trim().to_lowercase();
        if !looks_like_email(&alias) {
            return Err(AppError::new(
                ErrorCode::Invalid,
                format!("Not an email address: {alias}"),
            ));
        }
        if alias != email && !out.contains(&alias) {
            out.push(alias);
        }
    }
    Ok(out)
}

/// Verifies the credentials against IMAP (required) and SMTP (warning only), then saves the
/// account row and puts the secret in the keychain.
#[tauri::command]
pub async fn add_account(
    state: State<'_, AppState>,
    draft: AccountDraft,
    secret: SecretInput,
) -> Result<AddAccountResult> {
    validate(&draft)?;
    let email = draft.email.trim().to_lowercase();
    let username = match draft.username.trim() {
        "" => email.clone(),
        u => u.to_owned(),
    };
    if db::email_exists(&*state.db()?, &email)? {
        return Err(AppError::new(
            ErrorCode::Duplicate,
            "This address is already connected",
        ));
    }
    let aliases = clean_aliases(&email, &draft.aliases)?;

    let (stored, auth, smtp_result) = match secret {
        SecretInput::Password { password } => {
            // Google shows app passwords in groups of four; the spaces are not part of it.
            let password = if draft.provider == Provider::Gmail {
                password.replace(' ', "")
            } else {
                password
            };
            let login = || Login::Password {
                user: &username,
                password: &password,
            };
            let (imap, smtp) = tokio::join!(
                connect::check_imap(&draft.imap, login()),
                connect::check_smtp(&draft.smtp, login())
            );
            imap?;
            (
                StoredSecret::Password { password },
                AuthKind::Password,
                smtp,
            )
        }
        SecretInput::Oauth { session } => {
            let tokens = oauth::take_tokens(&state.oauth, &session).await?;
            let login = || Login::XOAuth2 {
                user: &username,
                token: &tokens.access_token,
            };
            let (imap, smtp) = tokio::join!(
                connect::check_imap(&draft.imap, login()),
                connect::check_smtp(&draft.smtp, login())
            );
            if let Err(e) = imap {
                oauth::put_back(&state.oauth, &session, tokens).await;
                return Err(e);
            }
            (
                StoredSecret::Oauth {
                    refresh_token: tokens.refresh_token,
                    access_token: tokens.access_token,
                    expires_at: tokens.expires_at,
                },
                AuthKind::Oauth,
                smtp,
            )
        }
    };

    let account = Account {
        id: uuid::Uuid::new_v4().to_string(),
        provider: draft.provider,
        label: if draft.label.trim().is_empty() {
            email.split('@').nth(1).unwrap_or(&email).to_owned()
        } else {
            draft.label.trim().to_owned()
        },
        default_from: email.clone(),
        email,
        username,
        aliases,
        display_name: draft.display_name.trim().to_owned(),
        color: draft.color,
        imap: draft.imap,
        smtp: draft.smtp,
        auth,
        created_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or_default(),
    };

    secrets::save(&account.id, &stored)?;
    if let Err(e) = db::insert_account(&*state.db()?, &account) {
        let _ = secrets::delete(&account.id);
        return Err(e);
    }

    Ok(AddAccountResult {
        account,
        smtp_warning: smtp_result.err().map(|e| e.message),
    })
}

/// Saves account settings. Changing the login, servers or password re-checks IMAP (required)
/// and SMTP (warning only) before anything is stored. New aliases refile cached mail, since
/// mail to or from them was seen as someone else's.
#[tauri::command]
pub async fn update_account(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    update: AccountUpdate,
) -> Result<AddAccountResult> {
    let current = db::get_account(&*state.db()?, &id)?;
    if !ACCOUNT_COLORS.contains(&update.color.as_str()) {
        return Err(AppError::new(ErrorCode::Invalid, "Unknown account color"));
    }
    for s in [&update.imap, &update.smtp] {
        if s.host.trim().is_empty() || s.port == 0 {
            return Err(AppError::new(
                ErrorCode::Invalid,
                "Server host and port are required",
            ));
        }
    }
    let username = match update.username.trim() {
        "" => current.email.clone(),
        u => u.to_owned(),
    };
    let aliases = clean_aliases(&current.email.to_lowercase(), &update.aliases)?;
    let default_from = update.default_from.trim().to_lowercase();
    let default_from = if aliases.contains(&default_from) {
        default_from
    } else {
        current.email.clone()
    };
    let new_alias = aliases.iter().any(|a| !current.aliases.contains(a));
    let new_password = update.password.filter(|p| !p.is_empty()).map(|p| {
        // Google shows app passwords in groups of four; the spaces are not part of it.
        if current.provider == Provider::Gmail {
            p.replace(' ', "")
        } else {
            p
        }
    });
    let login_changed = username != current.username
        || update.imap != current.imap
        || update.smtp != current.smtp
        || new_password.is_some();

    let mut smtp_warning = None;
    if login_changed {
        if current.auth == AuthKind::Oauth {
            return Err(AppError::new(
                ErrorCode::Invalid,
                "Accounts that sign in with OAuth use fixed servers",
            ));
        }
        let password = match &new_password {
            Some(p) => p.clone(),
            None => match secrets::load(&id)? {
                StoredSecret::Password { password } => password,
                StoredSecret::Oauth { .. } => {
                    return Err(AppError::new(
                        ErrorCode::Keychain,
                        "Stored secret is not a password",
                    ));
                }
            },
        };
        let login = || Login::Password {
            user: &username,
            password: &password,
        };
        let (imap, smtp) = tokio::join!(
            connect::check_imap(&update.imap, login()),
            connect::check_smtp(&update.smtp, login())
        );
        imap?;
        smtp_warning = smtp.err().map(|e| e.message);
    }

    let account = Account {
        label: if update.label.trim().is_empty() {
            current.label.clone()
        } else {
            update.label.trim().to_owned()
        },
        display_name: update.display_name.trim().to_owned(),
        aliases,
        default_from,
        color: update.color,
        username,
        imap: update.imap,
        smtp: update.smtp,
        ..current
    };
    if let Some(password) = new_password {
        secrets::save(&id, &StoredSecret::Password { password })?;
    }
    {
        let conn = state.db()?;
        db::update_account(&conn, &account)?;
        if new_alias && db::claim_addresses(&conn, &id, &sync::own_addresses(&account))? > 0 {
            let _ = app.emit("mail-changed", ());
        }
    }

    Ok(AddAccountResult {
        account,
        smtp_warning,
    })
}

#[tauri::command]
pub fn remove_account(state: State<'_, AppState>, id: String) -> Result<()> {
    db::delete_account(&*state.db()?, &id)?;
    secrets::delete(&id)
}

#[tauri::command]
pub fn list_chats(state: State<'_, AppState>) -> Result<Vec<ChatSummary>> {
    db::list_chats(&*state.db()?)
}

/// Chats matching a full-text search over all cached mail, best match first.
#[tauri::command]
pub fn search_chats(
    state: State<'_, AppState>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<db::SearchHit>> {
    db::search_chats(&*state.db()?, &query, limit.unwrap_or(200).min(1000))
}

/// Fetches the next batch of older mail for the given accounts (one IMAP session each,
/// concurrently). Failures are reported per account; `mail-changed` fires if anything new.
#[tauri::command]
pub async fn load_older_mail(
    app: AppHandle,
    state: State<'_, AppState>,
    account_ids: Vec<String>,
) -> Result<Vec<sync::Backfill>> {
    let _running = state.backfill_lock.lock().await;
    let accounts: Vec<Account> = db::list_accounts(&*state.db()?)?
        .into_iter()
        .filter(|a| account_ids.contains(&a.id))
        .collect();
    let results = join_all(accounts.iter().map(|account| async {
        sync::backfill_account(&state.db, account)
            .await
            .unwrap_or_else(|e| {
                eprintln!("[backfill] {} failed: {}", account.label, e.message);
                sync::Backfill {
                    account_id: account.id.clone(),
                    more: true,
                    error: Some(e.message),
                    ..Default::default()
                }
            })
    }))
    .await;
    if results.iter().any(|r| r.added > 0) {
        let _ = app.emit("mail-changed", ());
    }
    Ok(results)
}

#[tauri::command]
pub fn chat_messages(state: State<'_, AppState>, chat_id: String) -> Result<Vec<Message>> {
    db::chat_messages(&*state.db()?, &chat_id)
}

/// Saves read state locally, then copies it to the server in the background.
#[tauri::command]
pub fn set_chat_read(
    app: AppHandle,
    state: State<'_, AppState>,
    chat_id: String,
    read: bool,
) -> Result<()> {
    db::set_chat_read(&*state.db()?, &chat_id, read)?;
    let account = db::get_account(&*state.db()?, chat_account(&chat_id)?)?;
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        if let Err(e) = flags::push_seen(&state.db, &account).await {
            // Stays pending; the next sync retries.
            eprintln!("[seen] {}: {}", account.label, e.message);
        }
    });
    Ok(())
}

/// The account id part of a chat key (`account|addresses`).
fn chat_account(chat_id: &str) -> Result<&str> {
    chat_id
        .split_once('|')
        .map(|(a, _)| a)
        .ok_or_else(|| AppError::new(ErrorCode::Invalid, "Bad chat id"))
}

/// After a send: file a copy in Sent, then sync so the server's copy replaces the local one.
fn after_send(app: AppHandle, account: Account, raw: Vec<u8>) {
    let _ = app.emit("mail-changed", ());
    tauri::async_runtime::spawn(async move {
        if let Err(e) = send::save_to_sent(&account, &raw).await {
            eprintln!(
                "[send] {}: saving to Sent failed: {}",
                account.label, e.message
            );
        }
        sync_all(app).await;
    });
}

fn cache_dir(app: &AppHandle) -> Result<std::path::PathBuf> {
    app.path()
        .app_cache_dir()
        .map_err(|e| AppError::new(ErrorCode::Storage, e.to_string()))
}

/// Replies in a chat, to everyone in it, threaded under `reply_to` (a message id in the
/// chat) or else its newest message.
#[tauri::command]
pub async fn send_reply(
    app: AppHandle,
    state: State<'_, AppState>,
    chat_id: String,
    body: String,
    attachments: Option<Vec<send::FileUpload>>,
    reply_to: Option<String>,
    from: Option<String>,
) -> Result<()> {
    let files = send::decode_files(attachments.unwrap_or_default())?;
    let reply_to = reply_to
        .map(|id| id.parse::<i64>())
        .transpose()
        .map_err(|_| AppError::new(ErrorCode::Invalid, "Bad message id"))?;
    let (account, ctx) = {
        let conn = state.db()?;
        (
            db::get_account(&conn, chat_account(&chat_id)?)?,
            db::reply_context(&conn, &chat_id, reply_to)?,
        )
    };
    let sender = send::sender_as(&account, from.as_deref())?;
    let draft = send::reply_draft(sender, &ctx, &body, files);
    let (_, raw) = send::send(&state.db, &cache_dir(&app)?, &account, &draft).await?;
    after_send(app, account, raw);
    Ok(())
}

/// Sends a new message; returns the id of the chat it landed in.
#[allow(clippy::too_many_arguments)] // one per IPC argument
#[tauri::command]
pub async fn send_message(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    to: String,
    subject: String,
    body: String,
    attachments: Option<Vec<send::FileUpload>>,
    from: Option<String>,
) -> Result<String> {
    let files = send::decode_files(attachments.unwrap_or_default())?;
    let account = db::get_account(&*state.db()?, &account_id)?;
    let draft = send::Draft {
        from: send::sender_as(&account, from.as_deref())?,
        to: send::parse_recipients(&to)?,
        subject: subject.trim().to_owned(),
        body,
        in_reply_to: None,
        references: vec![],
        attachments: files,
    };
    let (chat_id, raw) = send::send(&state.db, &cache_dir(&app)?, &account, &draft).await?;
    after_send(app, account, raw);
    Ok(chat_id)
}

#[tauri::command]
pub fn set_chat_state(
    state: State<'_, AppState>,
    chat_id: String,
    chat_state: ChatState,
) -> Result<()> {
    db::set_chat_state(&*state.db()?, &chat_id, &chat_state)
}

#[tauri::command]
pub fn sync_status(state: State<'_, AppState>) -> SyncStatus {
    state
        .sync_status
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default()
}

#[tauri::command]
pub fn list_remote_rules(state: State<'_, AppState>) -> Result<Vec<db::RemoteRule>> {
    db::list_remote_rules(&*state.db()?)
}

#[tauri::command]
pub fn add_remote_rules(state: State<'_, AppState>, rules: Vec<db::RemoteRule>) -> Result<()> {
    db::add_remote_rules(&*state.db()?, &rules, crate::oauth::now())
}

#[tauri::command]
pub fn remove_remote_rule(state: State<'_, AppState>, rule: db::RemoteRule) -> Result<()> {
    db::remove_remote_rule(&*state.db()?, &rule)
}

/// Archives (or un-archives) a chat on the server, then tells the UI to reload.
#[tauri::command]
pub async fn archive_chat(
    app: AppHandle,
    state: State<'_, AppState>,
    chat_id: String,
    archived: bool,
) -> Result<()> {
    let account = db::get_account(&*state.db()?, chat_account(&chat_id)?)?;
    // A sync running meanwhile would write back the inbox state it read before the move.
    let _no_sync = state.sync_lock.lock().await;
    let result = crate::archive::set_archived(&state.db, &account, &chat_id, archived).await;
    let _ = app.emit("mail-changed", ());
    result
}

/// Opens a web, mail or phone link in its default app. Anything else is refused.
#[tauri::command]
pub async fn open_external(app: AppHandle, url: String) -> Result<()> {
    if !crate::files::is_safe_url(&url) {
        return Err(AppError::new(
            ErrorCode::Invalid,
            "Only web and mail links can be opened",
        ));
    }
    crate::files::open_with_system(&app, std::ffi::OsStr::new(url.trim()))
}

/// Android: keeps the app awake while a sign-in is open in the browser.
#[tauri::command]
pub async fn set_sign_in_active(app: AppHandle, active: bool) -> Result<()> {
    #[cfg(target_os = "android")]
    app.state::<crate::files::AndroidSystem>()
        .0
        .run_mobile_plugin::<serde_json::Value>(
            "setSignInActive",
            serde_json::json!({ "active": active }),
        )
        .map_err(|e| AppError::new(ErrorCode::Invalid, e.to_string()))?;
    #[cfg(not(target_os = "android"))]
    let _ = (app, active);
    Ok(())
}

/// Android: makes the strips behind the system bars match the app's theme.
#[tauri::command]
pub async fn set_system_bars(app: AppHandle, dark: bool) -> Result<()> {
    #[cfg(target_os = "android")]
    app.state::<crate::files::AndroidSystem>()
        .0
        .run_mobile_plugin::<serde_json::Value>("setBars", serde_json::json!({ "dark": dark }))
        .map_err(|e| AppError::new(ErrorCode::Invalid, e.to_string()))?;
    #[cfg(not(target_os = "android"))]
    let _ = (app, dark);
    Ok(())
}

/// Downloads an attachment from the server (cached after the first time) and opens it.
#[tauri::command]
pub async fn open_attachment(
    app: AppHandle,
    state: State<'_, AppState>,
    message_id: String,
    attachment_id: String,
) -> Result<()> {
    let id: i64 = message_id
        .parse()
        .map_err(|_| AppError::new(ErrorCode::Invalid, "Bad message id"))?;
    let path =
        crate::files::fetch_attachment(&state.db, &cache_dir(&app)?, id, &attachment_id).await?;
    crate::files::open_with_system(&app, path.as_os_str())
}

/// Opens a suggested event in the default calendar app (via a temporary .ics file).
#[tauri::command]
pub async fn add_to_calendar(app: AppHandle, event: crate::calendar::NewEvent) -> Result<()> {
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| AppError::new(ErrorCode::Storage, e.to_string()))?;
    crate::calendar::open_in_calendar(&app, &dir, &event)
}

/// Coordinates for a place name, for the map preview. Sends only the place text.
#[tauri::command]
pub async fn geocode(
    state: State<'_, AppState>,
    query: String,
) -> Result<Option<crate::calendar::Place>> {
    state.geocoder.lookup(&query).await
}

fn storage_dirs(app: &AppHandle) -> Result<(std::path::PathBuf, std::path::PathBuf)> {
    let path = app.path();
    let err = |e: tauri::Error| AppError::new(ErrorCode::Storage, e.to_string());
    Ok((
        path.app_data_dir().map_err(err)?,
        path.app_cache_dir().map_err(err)?,
    ))
}

/// Disk space used by cached mail, attachments and profile pictures.
#[tauri::command]
pub fn storage_usage(app: AppHandle, state: State<'_, AppState>) -> Result<StorageUsage> {
    let (data, cache) = storage_dirs(&app)?;
    storage::usage(&*state.db()?, &data, &cache)
}

/// Clears the local cache and downloads the last `RESYNC_DAYS` days again from every
/// account. Waits for a running sync or backfill first and keeps new ones out until done;
/// per-account failures arrive in `sync-status`. Returns the new usage.
#[tauri::command]
pub async fn reset_cache(app: AppHandle, state: State<'_, AppState>) -> Result<StorageUsage> {
    let (data, cache) = storage_dirs(&app)?;
    let _sync = state.sync_lock.lock().await;
    let _backfill = state.backfill_lock.lock().await;
    db::clear_mail_cache(&*state.db()?)?;
    storage::clear_files(&cache);
    let _ = app.emit("mail-changed", ());
    let since = crate::oauth::now() - storage::RESYNC_DAYS * 86_400;
    run_sync(&app, Some(since)).await;
    storage::usage(&*state.db()?, &data, &cache)
}

/// Hides a chat and schedules moving its messages to Trash after the undo window.
#[tauri::command]
pub fn delete_chat(app: AppHandle, state: State<'_, AppState>, chat_id: String) -> Result<()> {
    db::mark_chat_deleted(&*state.db()?, &chat_id, crate::oauth::now())?;
    tauri::async_runtime::spawn(async move {
        let wait = trash::UNDO_WINDOW_SECS as u64 + 1;
        tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
        commit_deletions(&app).await;
    });
    Ok(())
}

/// Undo of `delete_chat` within the undo window.
#[tauri::command]
pub fn restore_chat(state: State<'_, AppState>, chat_id: String) -> Result<()> {
    db::restore_chat(&*state.db()?, &chat_id)
}

/// Moves expired deletions to Trash on every account; failures are retried on the next sync.
async fn commit_deletions(app: &AppHandle) {
    let state = app.state::<AppState>();
    let accounts = state
        .db()
        .and_then(|c| db::list_accounts(&c))
        .unwrap_or_default();
    join_all(
        accounts
            .iter()
            .map(|a| async { trash::commit(&state.db, a).await }),
    )
    .await;
}

/// Starts a sync of all accounts in the background; progress arrives as events.
#[tauri::command]
pub fn sync_now(app: AppHandle) {
    tauri::async_runtime::spawn(sync_all(app));
}

fn publish_status(app: &AppHandle, update: impl FnOnce(&mut SyncStatus)) {
    let state = app.state::<AppState>();
    let snapshot = match state.sync_status.lock() {
        Ok(mut s) => {
            update(&mut s);
            s.clone()
        }
        Err(_) => return,
    };
    let _ = app.emit("sync-status", snapshot);
}

/// Syncs every account, unless a sync is already running.
pub async fn sync_all(app: AppHandle) {
    let state = app.state::<AppState>();
    let Ok(_running) = state.sync_lock.try_lock() else {
        return;
    };
    run_sync(&app, None).await;
}

/// Syncs every account concurrently; the caller holds `sync_lock`. Emits `mail-changed` as
/// soon as an account brings new mail, and `sync-status` at start and end. `since` limits
/// the first download of a folder (see `sync::sync_account`).
async fn run_sync(app: &AppHandle, since: Option<i64>) {
    let state = app.state::<AppState>();
    publish_status(app, |s| s.running = true);

    let accounts = state
        .db()
        .and_then(|c| db::list_accounts(&c))
        .unwrap_or_default();
    let results = join_all(accounts.iter().map(|account| {
        let app = app.clone();
        async move {
            let state = app.state::<AppState>();
            // Deletions left over from a failed attempt or a previous run go first.
            if let Err(e) = trash::commit(&state.db, account).await {
                return (account.id.clone(), Err(e));
            }
            // Read/unread changes that did not reach the server yet; sync goes on regardless.
            if let Err(e) = flags::push_seen(&state.db, account).await {
                eprintln!("[seen] {}: {}", account.label, e.message);
            }
            let result = sync::sync_account(&state.db, account, since).await;
            if matches!(result, Ok(n) if n > 0) {
                let _ = app.emit("mail-changed", ());
            }
            (account.id.clone(), result)
        }
    }))
    .await;

    let errors = results
        .into_iter()
        .filter_map(|(account_id, r)| {
            if let Err(e) = &r {
                eprintln!("[sync] {account_id} failed: {}", e.message);
            }
            r.err().map(|e| SyncError {
                account_id,
                message: e.message,
            })
        })
        .collect();
    publish_status(app, |s| {
        s.running = false;
        s.last_run = Some(crate::oauth::now());
        s.errors = errors;
    });
}
