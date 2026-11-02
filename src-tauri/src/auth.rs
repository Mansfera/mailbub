//! Login material for a saved account: the stored password, or a fresh OAuth access token
//! (refreshed and written back to the keychain when it is about to expire).

use crate::connect::Login;
use crate::error::{AppError, ErrorCode, Result};
use crate::model::{Account, Provider};
use crate::oauth::now;
use crate::secrets::{self, StoredSecret};
use crate::{google, oauth};

pub enum Credentials {
    Password { user: String, password: String },
    OAuth { user: String, token: String },
}

impl Credentials {
    pub fn login(&self) -> Login<'_> {
        match self {
            Credentials::Password { user, password } => Login::Password { user, password },
            Credentials::OAuth { user, token } => Login::XOAuth2 { user, token },
        }
    }
}

pub async fn credentials(account: &Account) -> Result<Credentials> {
    let user = account.username.clone();
    match secrets::load(&account.id)? {
        StoredSecret::Password { password } => Ok(Credentials::Password { user, password }),
        StoredSecret::Oauth {
            refresh_token,
            access_token,
            expires_at,
        } => {
            if expires_at - 120 > now() {
                return Ok(Credentials::OAuth {
                    user,
                    token: access_token,
                });
            }
            let tokens = match account.provider {
                Provider::Gmail => google::refresh(&refresh_token).await?,
                Provider::Microsoft => oauth::refresh(&refresh_token).await?,
                Provider::Imap => {
                    return Err(AppError::new(
                        ErrorCode::Invalid,
                        "OAuth on a plain IMAP account",
                    ));
                }
            };
            secrets::save(
                &account.id,
                &StoredSecret::Oauth {
                    refresh_token: tokens.refresh_token.clone(),
                    access_token: tokens.access_token.clone(),
                    expires_at: tokens.expires_at,
                },
            )?;
            Ok(Credentials::OAuth {
                user,
                token: tokens.access_token,
            })
        }
    }
}
