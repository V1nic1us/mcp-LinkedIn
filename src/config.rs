use std::{
    env::{self, VarError},
    error::Error,
    fmt,
};

use url::{Host, Url};

pub(crate) const SHARE_SCOPE: &str = "w_member_social";
pub(crate) const OPENID_SCOPE: &str = "openid";
pub(crate) const PUBLISH_SCOPES: [&str; 2] = [OPENID_SCOPE, SHARE_SCOPE];

pub struct AppConfig {
    pub linkedin_client_id: Option<String>,
    pub linkedin_redirect_uri: Option<String>,
    pub linkedin_allowed_scopes: Vec<String>,
    pub linkedin_api_version: Option<String>,
}

impl AppConfig {
    pub fn from_env() -> Result<AppConfig, ConfigError> {
        let config = Self {
            linkedin_client_id: read_optional("LINKEDIN_CLIENT_ID")?,
            linkedin_redirect_uri: read_optional("LINKEDIN_REDIRECT_URI")?,
            linkedin_allowed_scopes: read_optional("LINKEDIN_ALLOWED_SCOPES")?
                .as_deref()
                .unwrap_or_default()
                .split_ascii_whitespace()
                .map(str::to_owned)
                .collect(),
            linkedin_api_version: read_optional("LINKEDIN_API_VERSION")?,
        };
        if let Some(uri) = config.linkedin_redirect_uri.as_deref() {
            parse_loopback_redirect(uri)?;
        }
        config.validate_scopes()?;
        Ok(config)
    }

    pub(crate) fn validate_scopes(&self) -> Result<(), ConfigError> {
        if self
            .linkedin_allowed_scopes
            .iter()
            .any(|scope| !PUBLISH_SCOPES.contains(&scope.as_str()))
        {
            return Err(ConfigError::UnsupportedScope);
        }
        Ok(())
    }

    pub(crate) fn allows_publish_scopes(&self) -> bool {
        self.validate_scopes().is_ok()
            && PUBLISH_SCOPES.iter().all(|scope| {
                self.linkedin_allowed_scopes
                    .iter()
                    .any(|allowed| allowed.as_str() == *scope)
            })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConfigError {
    Empty { variable: &'static str },
    NonUnicode { variable: &'static str },
    InvalidRedirect,
    UnsupportedScope,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty { variable } => write!(formatter, "{variable} is set but empty"),
            Self::NonUnicode { variable } => write!(formatter, "{variable} must be valid UTF-8"),
            Self::InvalidRedirect => formatter.write_str(
                "LINKEDIN_REDIRECT_URI must be an HTTP URL on 127.0.0.1 or [::1] with a simple path and no port",
            ),
            Self::UnsupportedScope => formatter.write_str(
                "LINKEDIN_ALLOWED_SCOPES supports only openid and w_member_social",
            ),
        }
    }
}

impl Error for ConfigError {}

fn read_optional(variable: &'static str) -> Result<Option<String>, ConfigError> {
    match env::var(variable) {
        Ok(value) if value.trim().is_empty() => Err(ConfigError::Empty { variable }),
        Ok(value) => Ok(Some(value)),
        Err(VarError::NotPresent) => Ok(None),
        Err(VarError::NotUnicode(_)) => Err(ConfigError::NonUnicode { variable }),
    }
}

pub(crate) fn parse_loopback_redirect(value: &str) -> Result<Url, ConfigError> {
    let url = Url::parse(value).map_err(|_| ConfigError::InvalidRedirect)?;
    let loopback = match url.host() {
        Some(Host::Ipv4(ip)) => ip == std::net::Ipv4Addr::LOCALHOST,
        Some(Host::Ipv6(ip)) => ip == std::net::Ipv6Addr::LOCALHOST,
        _ => false,
    };
    // Reject inputs that the URL parser would rewrite, including alternative IP
    // spellings, dot segments, whitespace and a missing trailing slash.
    if url.scheme() != "http"
        || !loopback
        || url.port().is_some()
        || url.as_str() != value
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.path().bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.' | b'~')
        })
        || url.path().starts_with("//")
    {
        return Err(ConfigError::InvalidRedirect);
    }
    Ok(url)
}
