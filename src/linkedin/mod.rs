pub mod oauth;
pub mod token_store;

use std::fmt;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::config::SHARE_SCOPE;

pub struct TokenSet {
    client_id: String,
    record: TokenRecord,
}

impl TokenSet {
    /// Use this value only as an authenticated API credential.
    pub fn access_token(&self) -> &str {
        self.record.access_token.as_str()
    }

    pub fn expires_at_unix(&self) -> u64 {
        self.record.expires_at_unix
    }

    pub fn scopes(&self) -> &[&'static str] {
        &[SHARE_SCOPE]
    }
}

impl fmt::Debug for TokenSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TokenSet")
            .field("access_token", &"[REDACTED]")
            .field("expires_at_unix", &self.expires_at_unix())
            .field("scopes", &self.scopes())
            .finish()
    }
}

// Keep the serializable credential record private to the auth/storage modules.
#[derive(Deserialize, Serialize)]
struct TokenRecord {
    access_token: Zeroizing<String>,
    expires_at_unix: u64,
}

fn valid_token(token: &str) -> bool {
    !token.is_empty() && token.bytes().all(|byte| byte.is_ascii_graphic())
}
