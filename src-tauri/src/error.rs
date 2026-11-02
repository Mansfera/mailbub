use serde::Serialize;

/// Machine-readable reason, so the UI can pick a hint without parsing messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Invalid,
    Duplicate,
    NotFound,
    Storage,
    Keychain,
    Timeout,
    ImapConnect,
    ImapTls,
    ImapAuth,
    SmtpConnect,
    SmtpAuth,
    OauthNotConfigured,
    OauthPending,
    MailNotAllowed,
    MailUnavailable,
    OauthDenied,
    OauthExpired,
    Oauth,
}

/// Error returned from every Tauri command; serialises as `{ code, message }`.
#[derive(Debug, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
}

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        Self::new(ErrorCode::Storage, e.to_string())
    }
}

impl From<keyring_core::Error> for AppError {
    fn from(e: keyring_core::Error) -> Self {
        Self::new(ErrorCode::Keychain, e.to_string())
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::new(ErrorCode::Storage, e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
