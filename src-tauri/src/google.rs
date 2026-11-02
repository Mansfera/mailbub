//! "Sign in with Google" for Gmail: OAuth 2.0 for installed apps, using the browser flow in
//! `oauth.rs` with http://127.0.0.1:<port> as the redirect. Needs a Google Cloud "Desktop
//! app" OAuth client given as
//! MAILBUB_GOOGLE_CLIENT_ID / MAILBUB_GOOGLE_CLIENT_SECRET (e.g. in `.env.local`).
//! Google documents the desktop client secret as not confidential.

use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use reqwest::Url;
use serde::Deserialize;
use tokio::sync::Mutex;

use crate::error::{AppError, ErrorCode, Result};
use crate::oauth::{
    BrowserStart, Loopback, RefreshResponse, Sessions, Tokens, http, net_err, now, oauth_err,
    send_token_request,
};

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// Full IMAP/SMTP access is only granted under the mail.google.com scope.
const SCOPES: &str = "https://mail.google.com/ openid email";

fn setting(runtime: Option<String>, built_in: Option<&str>) -> Option<String> {
    runtime
        .or_else(|| built_in.map(str::to_owned))
        .filter(|s| !s.trim().is_empty())
}

pub fn credentials() -> Option<(String, String)> {
    let id = setting(
        std::env::var("MAILBUB_GOOGLE_CLIENT_ID").ok(),
        option_env!("MAILBUB_GOOGLE_CLIENT_ID"),
    )?;
    let secret = setting(
        std::env::var("MAILBUB_GOOGLE_CLIENT_SECRET").ok(),
        option_env!("MAILBUB_GOOGLE_CLIENT_SECRET"),
    )?;
    Some((id, secret))
}

pub async fn start(sessions: Arc<Mutex<Sessions>>, email: String) -> Result<BrowserStart> {
    let (client_id, client_secret) = credentials().ok_or_else(|| {
        AppError::new(
            ErrorCode::OauthNotConfigured,
            "Sign in with Google needs an OAuth client (MAILBUB_GOOGLE_CLIENT_ID is not set)",
        )
    })?;
    let flow = Loopback::bind("127.0.0.1").await?;
    let url = Url::parse_with_params(
        AUTH_URL,
        &[
            ("client_id", client_id.as_str()),
            ("redirect_uri", flow.redirect.as_str()),
            ("response_type", "code"),
            ("scope", SCOPES),
            ("code_challenge", flow.challenge().as_str()),
            ("code_challenge_method", "S256"),
            ("state", flow.state.as_str()),
            // A refresh token is only issued with offline access and an explicit consent.
            ("access_type", "offline"),
            ("prompt", "consent"),
            ("login_hint", email.as_str()),
        ],
    )
    .map_err(|e| oauth_err(e.to_string()))?
    .to_string();

    let (verifier, redirect) = (flow.verifier.clone(), flow.redirect.clone());
    let session = flow
        .finish_in_background(sessions, move |code| async move {
            let (tokens, signed_in) =
                exchange(&client_id, &client_secret, &code, &verifier, &redirect).await?;
            if let Some(actual) = signed_in
                && !email.trim().is_empty()
                && !actual.eq_ignore_ascii_case(email.trim())
            {
                return Err(oauth_err(format!(
                    "Signed in as {actual}, but the form says {}",
                    email.trim()
                )));
            }
            Ok(tokens)
        })
        .await;

    Ok(BrowserStart { session, url })
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    id_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// The address in the ID token. It came straight from Google over TLS, so its signature
/// does not need checking here.
fn id_token_email(id_token: &str) -> Option<String> {
    let payload = URL_SAFE_NO_PAD.decode(id_token.split('.').nth(1)?).ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&payload).ok()?;
    claims["email"].as_str().map(str::to_lowercase)
}

async fn exchange(
    client_id: &str,
    client_secret: &str,
    code: &str,
    verifier: &str,
    redirect: &str,
) -> Result<(Tokens, Option<String>)> {
    let body: TokenResponse = send_token_request(http()?.post(TOKEN_URL).form(&[
        ("grant_type", "authorization_code"),
        ("code", code),
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("code_verifier", verifier),
        ("redirect_uri", redirect),
    ]))
    .await?
    .json()
    .await
    .map_err(net_err)?;
    match (body.access_token, body.refresh_token) {
        (Some(access_token), Some(refresh_token)) => Ok((
            Tokens {
                access_token,
                refresh_token,
                expires_at: now() + body.expires_in.unwrap_or(3600),
            },
            body.id_token.as_deref().and_then(id_token_email),
        )),
        _ => Err(oauth_err(
            body.error_description
                .or(body.error)
                .unwrap_or_else(|| "Google did not return a token".into()),
        )),
    }
}

pub async fn refresh(refresh_token: &str) -> Result<Tokens> {
    let (client_id, client_secret) = credentials()
        .ok_or_else(|| AppError::new(ErrorCode::OauthNotConfigured, "Google client id missing"))?;
    let body: RefreshResponse = http()?
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await
        .map_err(net_err)?
        .json()
        .await
        .map_err(net_err)?;
    body.into_tokens(refresh_token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_email_from_id_token() {
        let payload = URL_SAFE_NO_PAD.encode(br#"{"email":"Me@Gmail.com","sub":"1"}"#);
        assert_eq!(
            id_token_email(&format!("h.{payload}.s")).as_deref(),
            Some("me@gmail.com")
        );
        assert_eq!(id_token_email("garbage"), None);
    }
}
