mod archive;
mod auth;
mod avatars;
mod calendar;
mod commands;
mod connect;
mod db;
mod discover;
mod error;
mod files;
mod flags;
mod frame_menu;
mod google;
mod mail_import;
mod mailbox;
mod model;
mod oauth;
mod parse;
mod quotes;
mod secrets;
mod send;
mod storage;
mod sync;
mod trash;
mod unsubscribe;

use std::sync::Mutex;

use tauri::Manager;
use tauri::http::{Response, StatusCode, header};

use commands::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // One process-wide TLS crypto provider (ring) for rustls users that don't pick their own.
    let _ = rustls::crypto::ring::default_provider().install_default();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // WebKit never runs the parent's click listener inside the script-less email
        // iframe, so a clicked link navigates the frame; send it to the browser instead.
        // The CSP's frame-src must allow web and mail links, or WebKit blocks the frame's
        // navigation before this hook ever sees it.
        .plugin(
            tauri::plugin::Builder::<tauri::Wry>::new("external-links")
                .setup(|app, api| {
                    #[cfg(target_os = "android")]
                    app.manage(files::AndroidSystem(
                        api.register_android_plugin("com.mansfera.mailbub", "SystemPlugin")?,
                    ));
                    #[cfg(not(target_os = "android"))]
                    let _ = (app, api);
                    Ok(())
                })
                .on_navigation(|webview, url| {
                    if !files::leaves_app(url) {
                        return true;
                    }
                    let app = webview.app_handle().clone();
                    let target = url.to_string();
                    std::thread::spawn(move || {
                        let _ = files::open_with_system(&app, std::ffi::OsStr::new(&target));
                    });
                    false
                })
                .build(),
        )
        // <img src="avatar://localhost/<address>">: a profile picture, or 404 for initials.
        .register_asynchronous_uri_scheme_protocol("avatar", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let address =
                percent_encoding::percent_decode_str(request.uri().path().trim_start_matches('/'))
                    .decode_utf8_lossy()
                    .into_owned();
            tauri::async_runtime::spawn(async move {
                let found = match app.path().app_cache_dir() {
                    Ok(dir) => {
                        app.state::<avatars::Avatars>()
                            .get(&dir.join("avatars"), &address)
                            .await
                    }
                    Err(_) => None,
                };
                let res = match found {
                    Some(img) => Response::builder()
                        .header(header::CONTENT_TYPE, img.mime)
                        .header(header::CACHE_CONTROL, "max-age=86400")
                        .body(img.bytes),
                    None => Response::builder()
                        .status(StatusCode::NOT_FOUND)
                        .header(header::CACHE_CONTROL, "max-age=3600")
                        .body(Vec::new()),
                };
                if let Ok(res) = res {
                    responder.respond(res);
                }
            });
        })
        .setup(|app| {
            secrets::init()?;
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let conn = db::open(&dir.join(db::FILE))?;
            app.manage(AppState {
                db: Mutex::new(conn),
                oauth: Default::default(),
                sync_status: Default::default(),
                sync_lock: Default::default(),
                backfill_lock: Default::default(),
                geocoder: Default::default(),
            });
            app.manage(avatars::Avatars::default());
            if let Some(main) = app.get_webview_window("main") {
                frame_menu::install(&main);
            }
            // Background sync: once at start, then every few minutes (IMAP IDLE comes later).
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    commands::sync_all(handle.clone()).await;
                    tokio::time::sleep(std::time::Duration::from_secs(5 * 60)).await;
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_accounts,
            commands::discover_servers,
            commands::add_account,
            commands::update_account,
            commands::remove_account,
            commands::ms_oauth_available,
            commands::ms_oauth_start,
            commands::list_mail_app_accounts,
            commands::google_oauth_available,
            commands::google_oauth_start,
            commands::oauth_poll,
            commands::list_chats,
            commands::chat_messages,
            commands::search_chats,
            commands::load_older_mail,
            commands::set_chat_read,
            commands::set_chat_state,
            commands::sync_status,
            commands::sync_now,
            commands::delete_chat,
            commands::restore_chat,
            commands::list_remote_rules,
            commands::add_remote_rules,
            commands::remove_remote_rule,
            commands::add_to_calendar,
            commands::geocode,
            commands::open_external,
            commands::set_system_bars,
            commands::set_sign_in_active,
            commands::open_attachment,
            commands::storage_usage,
            commands::reset_cache,
            frame_menu::set_mail_frame_hover,
            unsubscribe::unsubscribe_one_click,
            mailbox::list_mailboxes,
            mailbox::browse_mailbox,
            mailbox::move_stored_messages,
            mailbox::move_browsed_messages,
            commands::archive_chat,
            commands::send_reply,
            commands::send_message,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
