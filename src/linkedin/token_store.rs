use std::{error::Error, fmt};

#[cfg(target_os = "linux")]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(target_os = "linux")]
use zeroize::Zeroizing;

use crate::config::AppConfig;

use super::TokenSet;
#[cfg(target_os = "linux")]
use super::{valid_token, TokenRecord};

pub struct TokenStore {
    #[cfg(target_os = "linux")]
    client_id: String,
    #[cfg(target_os = "linux")]
    entry: keyring::Entry,
}

impl TokenStore {
    pub fn new(config: &AppConfig) -> Result<Self, TokenStoreError> {
        let client_id = config
            .linkedin_client_id
            .as_deref()
            .filter(|id| !id.trim().is_empty())
            .ok_or(TokenStoreError::MissingClientId)?;

        #[cfg(target_os = "linux")]
        {
            // Select Secret Service explicitly, regardless of any global keyring
            // builder chosen by the application embedding this library.
            let credential = keyring::secret_service::SsCredential::new_with_target(
                None,
                "linkedin-mcp",
                client_id,
            )
            .map_err(|_| TokenStoreError::Unavailable)?;
            Ok(Self {
                client_id: client_id.to_owned(),
                entry: keyring::Entry::new_with_credential(Box::new(credential)),
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = client_id;
            Err(TokenStoreError::Unavailable)
        }
    }

    /// Missing and expired credentials both require authorization.
    pub fn load(&self) -> Result<Option<TokenSet>, TokenStoreError> {
        #[cfg(target_os = "linux")]
        {
            let secret = match self.entry.get_secret() {
                Ok(secret) => Zeroizing::new(secret),
                Err(keyring::Error::NoEntry) => return Ok(None),
                Err(_) => return Err(TokenStoreError::Unavailable),
            };
            let record: TokenRecord =
                serde_json::from_slice(&secret).map_err(|_| TokenStoreError::InvalidRecord)?;
            if !valid_token(&record.access_token) || record.expires_at_unix == 0 {
                return Err(TokenStoreError::InvalidRecord);
            }
            // Leave deletion to logout so a stale reader cannot remove a new token.
            if is_expired(record.expires_at_unix)? {
                return Ok(None);
            }
            Ok(Some(TokenSet {
                client_id: self.client_id.clone(),
                record,
            }))
        }
        #[cfg(not(target_os = "linux"))]
        Err(TokenStoreError::Unavailable)
    }

    pub fn save(&self, tokens: &TokenSet) -> Result<(), TokenStoreError> {
        #[cfg(target_os = "linux")]
        {
            if tokens.client_id != self.client_id {
                return Err(TokenStoreError::ClientMismatch);
            }
            if is_expired(tokens.expires_at_unix())? {
                return Err(TokenStoreError::Expired);
            }
            let secret = Zeroizing::new(
                serde_json::to_vec(&tokens.record).map_err(|_| TokenStoreError::InvalidRecord)?,
            );
            self.entry
                .set_secret(&secret)
                .map_err(|_| TokenStoreError::Unavailable)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = tokens;
            Err(TokenStoreError::Unavailable)
        }
    }

    /// Delete the credential for this app. This is idempotent.
    pub fn delete(&self) -> Result<(), TokenStoreError> {
        #[cfg(target_os = "linux")]
        {
            match self.entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(_) => Err(TokenStoreError::Unavailable),
            }
        }
        #[cfg(not(target_os = "linux"))]
        Err(TokenStoreError::Unavailable)
    }
}

#[cfg(target_os = "linux")]
fn is_expired(expires_at_unix: u64) -> Result<bool, TokenStoreError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| TokenStoreError::InvalidClock)?;
    Ok(now.as_secs() >= expires_at_unix)
}

#[derive(Debug, PartialEq, Eq)]
pub enum TokenStoreError {
    MissingClientId,
    ClientMismatch,
    Unavailable,
    InvalidRecord,
    InvalidClock,
    Expired,
}

impl fmt::Display for TokenStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MissingClientId => "set LINKEDIN_CLIENT_ID to select the LinkedIn credential",
            Self::ClientMismatch => "the LinkedIn credential belongs to a different client ID",
            Self::Unavailable => {
                "could not access the OS keyring; use a Linux desktop session with an available, unlocked Secret Service keyring"
            }
            Self::InvalidRecord => "the stored LinkedIn credential is invalid; log out and authorize again",
            Self::InvalidClock => "the system clock is invalid; correct it before using LinkedIn OAuth",
            Self::Expired => "the LinkedIn credential expired; authorize again",
        })
    }
}

impl Error for TokenStoreError {}
