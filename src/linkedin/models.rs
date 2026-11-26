use std::{error::Error, fmt};

use serde::Serialize;

use super::token_store::TokenStoreError;

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct ConnectionStatus {
    pub connected: bool,
    pub approved_scopes: Vec<&'static str>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct PublishedPost {
    pub urn: String,
    pub url: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum LinkedInApiError {
    AccessRequired,
    CredentialStore(TokenStoreError),
    Coordination(&'static str),
    InvalidText,
    ConfirmationAlreadyUsed,
    Unauthorized,
    RateLimited { retry_after_seconds: Option<u64> },
    Transport,
    InvalidIdentityResponse,
    Rejected { status: u16 },
    PublishOutcomeUnknown,
}

impl fmt::Display for LinkedInApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AccessRequired => formatter.write_str(
                "LinkedIn access is required. Enable both LinkedIn products, allow openid and w_member_social, and run auth login.",
            ),
            Self::CredentialStore(error) => fmt::Display::fmt(error, formatter),
            Self::Coordination(error) => formatter.write_str(error),
            Self::InvalidText => formatter.write_str(
                "post text must be nonblank and at most 3000 UTF-16 code units, with control characters limited to tabs and line breaks",
            ),
            Self::ConfirmationAlreadyUsed => formatter.write_str(
                "this post confirmation has already been consumed. Check your LinkedIn activity for the outcome of the earlier attempt.",
            ),
            Self::Unauthorized => {
                formatter.write_str("LinkedIn rejected the credential. Run auth login again.")
            }
            Self::RateLimited { retry_after_seconds } => {
                formatter.write_str("LinkedIn rate limit reached")?;
                if let Some(seconds) = retry_after_seconds {
                    write!(formatter, " (Retry-After: {seconds} seconds)")?;
                }
                Ok(())
            }
            Self::Transport => formatter.write_str("could not complete the HTTPS request to LinkedIn"),
            Self::InvalidIdentityResponse => {
                formatter.write_str("LinkedIn returned an invalid member identity response")
            }
            Self::Rejected { status } => write!(formatter, "LinkedIn rejected the request (HTTP {status})"),
            Self::PublishOutcomeUnknown => formatter.write_str(
                "LinkedIn may have published the post. Check your LinkedIn activity before attempting to publish again.",
            ),
        }
    }
}

impl Error for LinkedInApiError {}

impl From<TokenStoreError> for LinkedInApiError {
    fn from(error: TokenStoreError) -> Self {
        Self::CredentialStore(error)
    }
}
