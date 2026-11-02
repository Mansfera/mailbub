//! Opening and verifying IMAP/SMTP connections.

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::{AsyncSmtpTransport, Tokio1Executor};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::{ClientConfig, RootCertStore, pki_types::ServerName};

use crate::error::{AppError, ErrorCode, Result};
use crate::model::{Security, Server};

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

static TLS: LazyLock<Arc<ClientConfig>> = LazyLock::new(|| {
    let roots = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    Arc::new(
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
});

/// An HTTP client builder. reqwest's default verifier needs Kotlin glue on Android, so there
/// it trusts the same bundled roots as IMAP.
pub fn http_builder() -> reqwest::ClientBuilder {
    let builder = reqwest::Client::builder();
    #[cfg(target_os = "android")]
    let builder = builder.tls_backend_preconfigured((**TLS).clone());
    builder
}

pub enum Login<'a> {
    Password { user: &'a str, password: &'a str },
    XOAuth2 { user: &'a str, token: &'a str },
}

impl Login<'_> {
    fn user(&self) -> &str {
        match self {
            Login::Password { user, .. } | Login::XOAuth2 { user, .. } => user,
        }
    }
}

struct XOAuth2(String);

impl async_imap::Authenticator for XOAuth2 {
    type Response = String;
    fn process(&mut self, _challenge: &[u8]) -> String {
        std::mem::take(&mut self.0)
    }
}

pub fn xoauth2_payload(user: &str, token: &str) -> String {
    format!("user={user}\x01auth=Bearer {token}\x01\x01")
}

async fn tls_connect(host: &str, port: u16, tls_code: ErrorCode) -> Result<TlsStream<TcpStream>> {
    let connect_code = match tls_code {
        ErrorCode::ImapTls => ErrorCode::ImapConnect,
        _ => ErrorCode::SmtpConnect,
    };
    let tcp = timeout(CONNECT_TIMEOUT, TcpStream::connect((host, port)))
        .await
        .map_err(|_| AppError::new(ErrorCode::Timeout, format!("{host}:{port} did not answer")))?
        .map_err(|e| AppError::new(connect_code, format!("{host}:{port}: {e}")))?;
    let name = ServerName::try_from(host.to_owned())
        .map_err(|_| AppError::new(ErrorCode::Invalid, format!("Invalid host name: {host}")))?;
    timeout(
        CONNECT_TIMEOUT,
        TlsConnector::from(TLS.clone()).connect(name, tcp),
    )
    .await
    .map_err(|_| {
        AppError::new(
            ErrorCode::Timeout,
            format!("TLS handshake with {host} timed out"),
        )
    })?
    .map_err(|e| AppError::new(tls_code, format!("TLS with {host}: {e}")))
}

pub type ImapSession = async_imap::Session<TlsStream<TcpStream>>;

/// Connects over TLS and logs in.
pub async fn open_imap(server: &Server, login: Login<'_>) -> Result<ImapSession> {
    if server.security != Security::Tls {
        return Err(AppError::new(
            ErrorCode::Invalid,
            "IMAP needs a TLS port (usually 993); STARTTLS is not supported",
        ));
    }
    let stream = tls_connect(&server.host, server.port, ErrorCode::ImapTls).await?;
    let mut client = async_imap::Client::new(stream);
    timeout(CONNECT_TIMEOUT, client.read_response())
        .await
        .map_err(|_| AppError::new(ErrorCode::Timeout, "IMAP server sent no greeting"))?
        .map_err(|e| AppError::new(ErrorCode::ImapConnect, e.to_string()))?
        .ok_or_else(|| {
            AppError::new(ErrorCode::ImapConnect, "IMAP server closed the connection")
        })?;

    let user = login.user().to_owned();
    let result = timeout(CONNECT_TIMEOUT, async {
        match login {
            Login::Password { user, password } => client.login(user, password).await,
            Login::XOAuth2 { user, token } => {
                client
                    .authenticate("XOAUTH2", XOAuth2(xoauth2_payload(user, token)))
                    .await
            }
        }
    })
    .await
    .map_err(|_| AppError::new(ErrorCode::Timeout, "IMAP login timed out"))?;
    result.map_err(|(e, _)| imap_login_error(&user, e))
}

/// Logs in to IMAP and selects INBOX, proving the credentials can read mail.
pub async fn check_imap(server: &Server, login: Login<'_>) -> Result<()> {
    let mut session = open_imap(server, login).await?;
    session
        .select("INBOX")
        .await
        .map_err(|e| AppError::new(ErrorCode::ImapConnect, format!("Could not open INBOX: {e}")))?;
    let _ = session.logout().await;
    Ok(())
}

fn imap_login_error(user: &str, e: async_imap::error::Error) -> AppError {
    use async_imap::error::Error;
    match e {
        Error::No(msg) | Error::Bad(msg) => AppError::new(
            ErrorCode::ImapAuth,
            format!("Login for {user} refused: {msg}"),
        ),
        other => AppError::new(ErrorCode::ImapConnect, other.to_string()),
    }
}

/// An SMTP transport for the account's server, logging in with `login` on first use.
pub fn smtp_transport(
    server: &Server,
    login: Login<'_>,
) -> Result<AsyncSmtpTransport<Tokio1Executor>> {
    let builder = match server.security {
        Security::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&server.host),
        Security::Starttls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&server.host),
    }
    .map_err(|e| AppError::new(ErrorCode::SmtpConnect, e.to_string()))?
    .port(server.port)
    .timeout(Some(CONNECT_TIMEOUT));

    Ok(match login {
        Login::Password { user, password } => builder
            .credentials(Credentials::new(user.into(), password.into()))
            .authentication(vec![Mechanism::Plain, Mechanism::Login]),
        Login::XOAuth2 { user, token } => builder
            .credentials(Credentials::new(user.into(), token.into()))
            .authentication(vec![Mechanism::Xoauth2]),
    }
    .build())
}

pub fn smtp_error(e: lettre::transport::smtp::Error) -> AppError {
    if e.is_timeout() {
        AppError::new(ErrorCode::Timeout, e.to_string())
    } else if e.is_permanent() {
        AppError::new(ErrorCode::SmtpAuth, e.to_string())
    } else {
        AppError::new(ErrorCode::SmtpConnect, e.to_string())
    }
}

/// Connects to SMTP and authenticates, without sending anything.
pub async fn check_smtp(server: &Server, login: Login<'_>) -> Result<()> {
    match smtp_transport(server, login)?.test_connection().await {
        Ok(true) => Ok(()),
        Ok(false) => Err(AppError::new(
            ErrorCode::SmtpConnect,
            "SMTP server did not respond",
        )),
        Err(e) => Err(smtp_error(e)),
    }
}

async fn first_line<S: AsyncRead + AsyncWrite + Unpin>(stream: S) -> Option<String> {
    let mut line = String::new();
    timeout(
        Duration::from_secs(5),
        BufReader::new(stream).read_line(&mut line),
    )
    .await
    .ok()?
    .ok()?;
    Some(line)
}

/// True if `host:port` speaks TLS and greets like the expected protocol.
pub async fn probe_tls(host: &str, port: u16, greeting: &str) -> bool {
    let attempt = async {
        let stream = tls_connect(host, port, ErrorCode::ImapTls).await.ok()?;
        first_line(stream).await
    };
    matches!(timeout(Duration::from_secs(6), attempt).await, Ok(Some(l)) if l.starts_with(greeting))
}

/// True if `host:port` is a plain SMTP endpoint (for STARTTLS).
pub async fn probe_plain_smtp(host: &str, port: u16) -> bool {
    let attempt = async {
        let tcp = TcpStream::connect((host, port)).await.ok()?;
        first_line(tcp).await
    };
    matches!(timeout(Duration::from_secs(6), attempt).await, Ok(Some(l)) if l.starts_with("220"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xoauth2_payload_matches_spec() {
        assert_eq!(
            xoauth2_payload("a@b.c", "tok"),
            "user=a@b.c\u{1}auth=Bearer tok\u{1}\u{1}"
        );
    }
}
