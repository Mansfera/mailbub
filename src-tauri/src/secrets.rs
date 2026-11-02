//! Account credentials live only in the OS keychain (Android: encrypted with a Keystore key),
//! one entry per account id.

use serde::{Deserialize, Serialize};

use crate::error::{AppError, ErrorCode, Result};

const SERVICE: &str = "com.mansfera.mailbub";

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum StoredSecret {
    Password {
        password: String,
    },
    Oauth {
        refresh_token: String,
        access_token: String,
        /// Unix seconds.
        expires_at: i64,
    },
}

/// Registers the platform credential store. Call once at startup.
pub fn init() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        keyring_core::set_default_store(apple_native_keyring_store::keychain::Store::new()?);
        Ok(())
    }
    #[cfg(target_os = "android")]
    {
        keyring_core::set_default_store(android_native_keyring_store::Store::new()?);
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "android")))]
    Err(AppError::new(
        ErrorCode::Keychain,
        "No credential store for this platform yet",
    ))
}

fn entry(account_id: &str) -> Result<keyring_core::Entry> {
    Ok(keyring_core::Entry::new(SERVICE, account_id)?)
}

pub fn save(account_id: &str, secret: &StoredSecret) -> Result<()> {
    let json = serde_json::to_string(secret)
        .map_err(|e| AppError::new(ErrorCode::Keychain, e.to_string()))?;
    entry(account_id)?.set_password(&json)?;
    Ok(())
}

pub fn load(account_id: &str) -> Result<StoredSecret> {
    let json = entry(account_id)?.get_password()?;
    serde_json::from_str(&json).map_err(|e| AppError::new(ErrorCode::Keychain, e.to_string()))
}

pub fn delete(account_id: &str) -> Result<()> {
    match entry(account_id)?.delete_credential() {
        Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}
