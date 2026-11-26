use std::{
    env::{self, VarError},
    error::Error,
    fmt,
};

pub struct AppConfig {
    pub linkedin_client_id: Option<String>,
    pub linkedin_redirect_uri: Option<String>,
    pub linkedin_allowed_scopes: Vec<String>,
    pub linkedin_api_version: Option<String>,
}

impl AppConfig {
    pub fn from_env() -> Result<AppConfig, ConfigError> {
        Ok(Self {
            linkedin_client_id: read_optional("LINKEDIN_CLIENT_ID")?,
            linkedin_redirect_uri: read_optional("LINKEDIN_REDIRECT_URI")?,
            linkedin_allowed_scopes: read_optional("LINKEDIN_ALLOWED_SCOPES")?
                .as_deref()
                .unwrap_or_default()
                .split_ascii_whitespace()
                .map(str::to_owned)
                .collect(),
            linkedin_api_version: read_optional("LINKEDIN_API_VERSION")?,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConfigError {
    Empty { variable: &'static str },
    NonUnicode { variable: &'static str },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty { variable } => write!(formatter, "{variable} is set but empty"),
            Self::NonUnicode { variable } => write!(formatter, "{variable} must be valid UTF-8"),
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
