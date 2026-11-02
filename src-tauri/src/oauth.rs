//! Shared OAuth plumbing: the session store and the browser sign-in both providers use (a
//! loopback redirect with PKCE, answered by a one-shot listener here), plus Microsoft sign-in.
//! Microsoft needs an Entra ID app registration with the `http://localhost` mobile/desktop
//! redirect URI, whose client id is given as MAILBUB_MS_CLIENT_ID (e.g. in `.env.local`).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

use crate::error::{AppError, ErrorCode, Result};

const AUTHORITY: &str = "https://login.microsoftonline.com/common/oauth2/v2.0";
const SCOPES: &str = "offline_access https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send";
const WAIT: Duration = Duration::from_secs(300);

pub fn client_id() -> Option<String> {
    std::env::var("MAILBUB_MS_CLIENT_ID")
        .ok()
        .or_else(|| option_env!("MAILBUB_MS_CLIENT_ID").map(str::to_owned))
        .filter(|s| !s.trim().is_empty())
}

pub(crate) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

#[derive(Debug, Clone)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
}

/// A sign-in started from the UI; tokens stay here until the account is saved.
pub struct Pending {
    pub(crate) expires_at: i64,
    pub tokens: Option<Tokens>,
    /// Set when the background sign-in fails.
    pub(crate) error: Option<(ErrorCode, String)>,
}

#[derive(Default)]
pub struct Sessions(pub HashMap<String, Pending>);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserStart {
    pub session: String,
    /// Sign-in page to open in the browser.
    pub url: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    error: Option<String>,
    error_description: Option<String>,
}

pub(crate) fn http() -> Result<reqwest::Client> {
    crate::connect::http_builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| AppError::new(ErrorCode::Oauth, e.to_string()))
}

/// How long a sign-in's token request keeps trying to connect. Android cuts a backgrounded app
/// off from the network, and the code arrives while the browser is still in front.
#[cfg(target_os = "android")]
const CONNECT_RETRY: Duration = Duration::from_secs(180);
#[cfg(not(target_os = "android"))]
const CONNECT_RETRY: Duration = Duration::ZERO;

/// Sends the request that spends a sign-in's code. One that never connected didn't use the code,
/// so it is safe to repeat until the app is back in front.
pub(crate) async fn send_token_request(
    request: reqwest::RequestBuilder,
) -> Result<reqwest::Response> {
    let started = std::time::Instant::now();
    loop {
        let attempt = request
            .try_clone()
            .ok_or_else(|| oauth_err("Could not prepare the sign-in request"))?;
        match attempt.send().await {
            Err(e) if e.is_connect() && started.elapsed() < CONNECT_RETRY => {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            result => return result.map_err(net_err),
        }
    }
}

pub(crate) fn net_err(e: reqwest::Error) -> AppError {
    AppError::new(
        ErrorCode::Oauth,
        format!("Could not reach the sign-in service: {e}"),
    )
}

pub(crate) fn oauth_err(message: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::Oauth, message)
}

pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// The local redirect target and PKCE values of one browser sign-in.
pub(crate) struct Loopback {
    listener: TcpListener,
    pub(crate) redirect: String,
    pub(crate) verifier: String,
    pub(crate) state: String,
}

impl Loopback {
    /// `host` is the redirect host the provider has on record; the port is picked here.
    pub(crate) async fn bind(host: &str) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| oauth_err(format!("Could not open a local port: {e}")))?;
        let port = listener
            .local_addr()
            .map_err(|e| oauth_err(e.to_string()))?
            .port();
        Ok(Self {
            listener,
            redirect: format!("http://{host}:{port}"),
            verifier: format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
            state: uuid::Uuid::new_v4().simple().to_string(),
        })
    }

    pub(crate) fn challenge(&self) -> String {
        pkce_challenge(&self.verifier)
    }

    /// Starts a session that waits for the browser in the background and turns the returned
    /// code into tokens with `exchange`. The UI follows it with `poll`.
    pub(crate) async fn finish_in_background<F, Fut>(
        self,
        sessions: Arc<Mutex<Sessions>>,
        exchange: F,
    ) -> String
    where
        F: FnOnce(String) -> Fut + Send + 'static,
        Fut: Future<Output = Result<Tokens>> + Send + 'static,
    {
        let session = uuid::Uuid::new_v4().to_string();
        sessions.lock().await.0.insert(
            session.clone(),
            Pending {
                expires_at: now() + WAIT.as_secs() as i64,
                tokens: None,
                error: None,
            },
        );

        let id = session.clone();
        tauri::async_runtime::spawn(async move {
            let outcome = async {
                let code = tokio::time::timeout(WAIT, wait_for_code(self.listener, &self.state))
                    .await
                    .map_err(|_| AppError::new(ErrorCode::OauthExpired, "Sign-in timed out"))??;
                exchange(code).await
            }
            .await;
            if let Some(p) = sessions.lock().await.0.get_mut(&id) {
                match outcome {
                    Ok(tokens) => p.tokens = Some(tokens),
                    Err(e) => p.error = Some((e.code, e.message)),
                }
            }
        });
        session
    }
}

#[cfg(not(target_os = "android"))]
const PAGE_OK: &str = "<!doctype html><meta charset=utf-8><title>Mailbub</title>\
<body style=\"font:15px -apple-system,system-ui,sans-serif;padding:48px;color:#1b1d21\">\
<h2 style=\"margin:0 0 8px\">Signed in</h2><p>You can close this tab and go back to Mailbub.</p>";
#[cfg(not(target_os = "android"))]
const PAGE_FAIL: &str = "<!doctype html><meta charset=utf-8><title>Mailbub</title>\
<body style=\"font:15px -apple-system,system-ui,sans-serif;padding:48px;color:#1b1d21\">\
<h2 style=\"margin:0 0 8px\">Sign-in did not finish</h2><p>Go back to Mailbub and try again.</p>";

/// On Android the page itself returns to the app, which closes the Custom Tab; the link is for
/// a browser that won't follow the redirect without a tap. The address carries no data.
#[cfg(target_os = "android")]
macro_rules! return_page {
    ($title:literal) => {
        concat!(
            "<!doctype html><meta charset=utf-8><title>Mailbub</title>",
            "<meta name=viewport content=\"width=device-width,initial-scale=1\">",
            "<body style=\"font:16px system-ui,sans-serif;padding:32px;color:#1b1d21\">",
            "<h2 style=\"margin:0 0 16px\">",
            $title,
            "</h2><a href=\"com.mansfera.mailbub://signed-in\">Return to Mailbub</a>",
            "<script>location.replace(\"com.mansfera.mailbub://signed-in\")</script>"
        )
    };
}
#[cfg(target_os = "android")]
const PAGE_OK: &str = return_page!("Signed in");
#[cfg(target_os = "android")]
const PAGE_FAIL: &str = return_page!("Sign-in did not finish");

async fn respond(sock: &mut TcpStream, status: &str, body: &str) {
    let res = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = sock.write_all(res.as_bytes()).await;
    let _ = sock.shutdown().await;
}

/// Parses the query of a request line like `GET /?code=…&state=… HTTP/1.1`.
fn callback_params(request: &str) -> Option<HashMap<String, String>> {
    let target = request.lines().next()?.split_whitespace().nth(1)?;
    let url = Url::parse(&format!("http://127.0.0.1{target}")).ok()?;
    if url.path() != "/" {
        return None;
    }
    let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
    (params.contains_key("code") || params.contains_key("error")).then_some(params)
}

/// Microsoft reports policy blocks as `access_denied` too, with the reason in the description;
/// only its `cancel` subcode, or a bare `access_denied` from Google, is the user saying no.
fn callback_error(params: &HashMap<String, String>) -> AppError {
    let description = params.get("error_description").cloned();
    let cancelled =
        description.is_none() || params.get("error_subcode").map(String::as_str) == Some("cancel");
    match params.get("error").map(String::as_str) {
        Some("access_denied") if cancelled => AppError::new(
            ErrorCode::OauthDenied,
            description.unwrap_or_else(|| "Access was not granted".into()),
        ),
        Some(e) => oauth_err(description.unwrap_or_else(|| e.to_owned())),
        None => oauth_err("The sign-in response did not match this request"),
    }
}

async fn wait_for_code(listener: TcpListener, state: &str) -> Result<String> {
    loop {
        let (mut sock, _) = listener
            .accept()
            .await
            .map_err(|e| oauth_err(e.to_string()))?;
        let mut buf = vec![0u8; 8192];
        let n = sock.read(&mut buf).await.unwrap_or(0);
        // Browsers also ask for /favicon.ico and the like; ignore anything but the callback.
        let Some(params) = callback_params(&String::from_utf8_lossy(&buf[..n])) else {
            respond(&mut sock, "404 Not Found", "").await;
            continue;
        };
        let state_ok = params.get("state").map(String::as_str) == Some(state);
        let code = params.get("code").filter(|_| state_ok).cloned();
        respond(
            &mut sock,
            "200 OK",
            if code.is_some() { PAGE_OK } else { PAGE_FAIL },
        )
        .await;
        return code.ok_or_else(|| callback_error(&params));
    }
}

/// Ok(()) once signed in; `OauthPending` while the browser step is still open.
pub async fn poll(sessions: &Mutex<Sessions>, session: &str) -> Result<()> {
    let guard = sessions.lock().await;
    let p = guard
        .0
        .get(session)
        .ok_or_else(|| AppError::new(ErrorCode::OauthExpired, "Sign-in session not found"))?;
    if p.tokens.is_some() {
        return Ok(());
    }
    if let Some((code, message)) = &p.error {
        return Err(AppError::new(*code, message.clone()));
    }
    if now() > p.expires_at {
        return Err(AppError::new(ErrorCode::OauthExpired, "Sign-in timed out"));
    }
    Err(AppError::new(
        ErrorCode::OauthPending,
        "Waiting for sign-in",
    ))
}

fn authorize_url(
    client_id: &str,
    redirect: &str,
    challenge: &str,
    state: &str,
    email: &str,
) -> Result<String> {
    let mut params = vec![
        ("client_id", client_id),
        ("response_type", "code"),
        ("redirect_uri", redirect),
        ("response_mode", "query"),
        ("scope", SCOPES),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
        ("prompt", "select_account"),
    ];
    // Only a hint: an organisation's login name can differ from the mail address.
    if !email.trim().is_empty() {
        params.push(("login_hint", email.trim()));
    }
    Url::parse_with_params(&format!("{AUTHORITY}/authorize"), &params)
        .map(|url| url.to_string())
        .map_err(|e| oauth_err(e.to_string()))
}

/// Opens a loopback listener and returns Microsoft's sign-in page for the UI to open.
pub async fn start(sessions: Arc<Mutex<Sessions>>, email: String) -> Result<BrowserStart> {
    let client_id = client_id().ok_or_else(|| {
        AppError::new(
            ErrorCode::OauthNotConfigured,
            "Microsoft sign-in needs an app registration (MAILBUB_MS_CLIENT_ID is not set)",
        )
    })?;
    // Entra matches `http://localhost` on any port, but treats 127.0.0.1 as a different URI.
    let flow = Loopback::bind("localhost").await?;
    let url = authorize_url(
        &client_id,
        &flow.redirect,
        &flow.challenge(),
        &flow.state,
        &email,
    )?;
    let (verifier, redirect) = (flow.verifier.clone(), flow.redirect.clone());
    let session = flow
        .finish_in_background(sessions, move |code| async move {
            exchange(&client_id, &code, &verifier, &redirect).await
        })
        .await;
    Ok(BrowserStart { session, url })
}

async fn exchange(client_id: &str, code: &str, verifier: &str, redirect: &str) -> Result<Tokens> {
    let body: TokenResponse =
        send_token_request(http()?.post(format!("{AUTHORITY}/token")).form(&[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect),
            ("scope", SCOPES),
        ]))
        .await?
        .json()
        .await
        .map_err(net_err)?;
    match (body.access_token, body.refresh_token) {
        (Some(access_token), Some(refresh_token)) => Ok(Tokens {
            access_token,
            refresh_token,
            expires_at: now() + body.expires_in.unwrap_or(3600),
        }),
        // e.g. AADSTS65001: the organisation requires admin consent for this app.
        _ => Err(oauth_err(
            body.error_description
                .or(body.error)
                .unwrap_or_else(|| "Microsoft did not return a token".into()),
        )),
    }
}

#[derive(Deserialize)]
pub(crate) struct RefreshResponse {
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub expires_in: Option<i64>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

impl RefreshResponse {
    /// Providers may or may not rotate the refresh token; keep the old one if not.
    pub(crate) fn into_tokens(self, old_refresh: &str) -> Result<Tokens> {
        match self.access_token {
            Some(access_token) => Ok(Tokens {
                access_token,
                refresh_token: self.refresh_token.unwrap_or_else(|| old_refresh.to_owned()),
                expires_at: now() + self.expires_in.unwrap_or(3600),
            }),
            None if self.error.as_deref() == Some("invalid_grant") => Err(AppError::new(
                ErrorCode::OauthExpired,
                "Sign-in expired; remove and add the account again",
            )),
            None => Err(AppError::new(
                ErrorCode::Oauth,
                self.error_description
                    .or(self.error)
                    .unwrap_or_else(|| "Token refresh failed".into()),
            )),
        }
    }
}

pub async fn refresh(refresh_token: &str) -> Result<Tokens> {
    let client_id = client_id()
        .ok_or_else(|| AppError::new(ErrorCode::OauthNotConfigured, "Client id missing"))?;
    let body: RefreshResponse = http()?
        .post(format!("{AUTHORITY}/token"))
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", client_id.as_str()),
            ("refresh_token", refresh_token),
            ("scope", SCOPES),
        ])
        .send()
        .await
        .map_err(net_err)?
        .json()
        .await
        .map_err(net_err)?;
    body.into_tokens(refresh_token)
}

pub async fn take_tokens(sessions: &tokio::sync::Mutex<Sessions>, session: &str) -> Result<Tokens> {
    sessions
        .lock()
        .await
        .0
        .remove(session)
        .and_then(|p| p.tokens)
        .ok_or_else(|| AppError::new(ErrorCode::OauthExpired, "Sign in with Microsoft first"))
}

pub async fn put_back(sessions: &tokio::sync::Mutex<Sessions>, session: &str, tokens: Tokens) {
    sessions.lock().await.0.insert(
        session.to_owned(),
        Pending {
            expires_at: tokens.expires_at,
            tokens: Some(tokens),
            error: None,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn pkce_matches_rfc7636_example() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn reads_callback_and_ignores_other_paths() {
        let p =
            callback_params("GET /?state=abc&code=4%2F0Ab HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        assert_eq!(p["code"], "4/0Ab");
        assert_eq!(p["state"], "abc");
        assert!(callback_params("GET /favicon.ico HTTP/1.1\r\n").is_none());
        assert!(callback_params("GET / HTTP/1.1\r\n").is_none());
    }

    #[test]
    fn tells_a_cancelled_sign_in_from_a_policy_block() {
        let bare = callback_error(&params(&[("error", "access_denied")]));
        assert!(matches!(bare.code, ErrorCode::OauthDenied));

        let cancelled = callback_error(&params(&[
            ("error", "access_denied"),
            ("error_subcode", "cancel"),
            ("error_description", "AADSTS65004: declined"),
        ]));
        assert!(matches!(cancelled.code, ErrorCode::OauthDenied));

        let blocked = callback_error(&params(&[
            ("error", "access_denied"),
            ("error_description", "AADSTS53003: blocked"),
        ]));
        assert!(matches!(blocked.code, ErrorCode::Oauth));
        assert_eq!(blocked.message, "AADSTS53003: blocked");
    }

    #[test]
    fn authorize_url_asks_for_mail_scopes_with_pkce() {
        let url = authorize_url("id", "http://localhost:5000", "ch", "st", " me@uni.sk ").unwrap();
        let url = Url::parse(&url).unwrap();
        let q: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(q["redirect_uri"], "http://localhost:5000");
        assert_eq!(q["scope"], SCOPES);
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["login_hint"], "me@uni.sk");

        let no_hint = authorize_url("id", "http://localhost:5000", "ch", "st", " ").unwrap();
        assert!(!no_hint.contains("login_hint"));
    }

    async fn call_back(redirect: &str, query: &str) -> String {
        let port: u16 = redirect.rsplit(':').next().unwrap().parse().unwrap();
        let mut sock = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        sock.write_all(format!("GET /{query} HTTP/1.1\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut page = String::new();
        sock.read_to_string(&mut page).await.unwrap();
        page
    }

    async fn settled(sessions: &Mutex<Sessions>, session: &str) -> Result<()> {
        loop {
            match poll(sessions, session).await {
                Err(e) if matches!(e.code, ErrorCode::OauthPending) => {
                    tokio::time::sleep(Duration::from_millis(5)).await
                }
                outcome => return outcome,
            }
        }
    }

    #[tokio::test]
    async fn browser_flow_turns_the_callback_into_tokens() {
        let sessions = Arc::new(Mutex::new(Sessions::default()));
        let flow = Loopback::bind("localhost").await.unwrap();
        let (redirect, state) = (flow.redirect.clone(), flow.state.clone());
        let session = flow
            .finish_in_background(sessions.clone(), |code| async move {
                Ok(Tokens {
                    access_token: code,
                    refresh_token: "refresh".into(),
                    expires_at: 0,
                })
            })
            .await;

        assert!(call_back(&redirect, "favicon.ico").await.contains("404"));
        let page = call_back(&redirect, &format!("?code=abc&state={state}")).await;
        assert!(page.contains("Signed in"));

        settled(&sessions, &session).await.unwrap();
        let tokens = take_tokens(&sessions, &session).await.unwrap();
        assert_eq!(tokens.access_token, "abc");
    }

    #[tokio::test]
    async fn browser_flow_rejects_a_code_with_the_wrong_state() {
        let sessions = Arc::new(Mutex::new(Sessions::default()));
        let flow = Loopback::bind("localhost").await.unwrap();
        let redirect = flow.redirect.clone();
        let session = flow
            .finish_in_background(sessions.clone(), |_| async move {
                panic!("must not exchange a code from another request")
            })
            .await;

        let page = call_back(&redirect, "?code=abc&state=forged").await;
        assert!(page.contains("did not finish"));
        assert!(matches!(
            settled(&sessions, &session).await.unwrap_err().code,
            ErrorCode::Oauth
        ));
    }
}
