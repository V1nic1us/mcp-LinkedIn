use std::{
    error::Error,
    fmt,
    net::{IpAddr, SocketAddr},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ring::{
    digest::{digest, SHA256},
    rand::{SecureRandom, SystemRandom},
};
use serde::Deserialize;
use subtle::ConstantTimeEq;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::{timeout, timeout_at, Instant},
};
use url::{form_urlencoded, Host, Position, Url};
use zeroize::Zeroizing;

use crate::config::{parse_loopback_redirect, AppConfig, ConfigError, SHARE_SCOPE};

use super::{valid_token, TokenRecord, TokenSet};

const AUTHORIZE_ENDPOINT: &str = "https://www.linkedin.com/oauth/native-pkce/authorization";
const TOKEN_ENDPOINT: &str = "https://www.linkedin.com/oauth/v2/accessToken";
const CONSENT_TIMEOUT: Duration = Duration::from_secs(300);
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(2);
const TOKEN_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_CALLBACK_BYTES: usize = 8192;
const MAX_TOKEN_BODY_BYTES: usize = 32768;

pub struct OAuthClient {
    client_id: String,
    redirect_uri: Url,
    http: reqwest::Client,
}

impl OAuthClient {
    pub fn new(config: &AppConfig) -> Result<Self, OAuthError> {
        let client_id = config
            .linkedin_client_id
            .as_deref()
            .filter(|id| !id.trim().is_empty())
            .ok_or(OAuthError::MissingClientId)?;
        let redirect_uri = parse_loopback_redirect(
            config
                .linkedin_redirect_uri
                .as_deref()
                .ok_or(OAuthError::MissingRedirect)?,
        )
        .map_err(OAuthError::Config)?;
        config.validate_scopes().map_err(OAuthError::Config)?;
        if config.linkedin_allowed_scopes.is_empty() {
            return Err(OAuthError::MissingScope);
        }

        let http = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .timeout(TOKEN_TIMEOUT)
            .build()
            .map_err(|_| OAuthError::Transport)?;

        Ok(Self {
            client_id: client_id.to_owned(),
            redirect_uri,
            http,
        })
    }

    /// Bind the loopback listener before exposing the URL to the browser.
    /// The returned guard owns this single-use consent attempt.
    pub async fn authorize_url(&self) -> Result<PendingAuthorization, OAuthError> {
        let ip = match self.redirect_uri.host() {
            Some(Host::Ipv4(ip)) => IpAddr::V4(ip),
            Some(Host::Ipv6(ip)) => IpAddr::V6(ip),
            _ => return Err(OAuthError::Config(ConfigError::InvalidRedirect)),
        };
        let port = self
            .redirect_uri
            .port()
            .ok_or(OAuthError::Config(ConfigError::InvalidRedirect))?;
        let listener = TcpListener::bind(SocketAddr::new(ip, port))
            .await
            .map_err(|_| OAuthError::BindCallback)?;

        let state = random_url_safe()?;
        let verifier = random_url_safe()?;
        let challenge = URL_SAFE_NO_PAD.encode(digest(&SHA256, verifier.as_bytes()).as_ref());
        let mut url =
            Url::parse(AUTHORIZE_ENDPOINT).expect("fixed LinkedIn authorization endpoint");
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", self.redirect_uri.as_str())
            .append_pair("scope", SHARE_SCOPE)
            .append_pair("state", &state)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256");

        Ok(PendingAuthorization {
            listener,
            url,
            client_id: self.client_id.clone(),
            redirect_uri: self.redirect_uri.clone(),
            state,
            verifier,
            deadline: Instant::now() + CONSENT_TIMEOUT,
        })
    }

    pub async fn exchange_code(&self, callback: LoopbackCallback) -> Result<TokenSet, OAuthError> {
        if callback.redirect_uri != self.redirect_uri || callback.client_id != self.client_id {
            return Err(OAuthError::InvalidCallback);
        }

        // Start the local lifetime before the request, so latency cannot extend it.
        let requested_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| OAuthError::InvalidClock)?
            .as_secs();
        let mut response = self
            .http
            .post(TOKEN_ENDPOINT)
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", callback.code.as_str()),
                ("redirect_uri", self.redirect_uri.as_str()),
                ("client_id", self.client_id.as_str()),
                ("code_verifier", callback.verifier.as_str()),
            ])
            .send()
            .await
            .map_err(|_| OAuthError::Transport)?;
        if !response.status().is_success() {
            return Err(OAuthError::TokenRejected(response.status().as_u16()));
        }

        let mut body = Zeroizing::new(Vec::new());
        while let Some(chunk) = response.chunk().await.map_err(|_| OAuthError::Transport)? {
            if chunk.len() > MAX_TOKEN_BODY_BYTES - body.len() {
                return Err(OAuthError::InvalidToken);
            }
            body.extend_from_slice(&chunk);
        }
        let token: TokenResponse =
            serde_json::from_slice(&body).map_err(|_| OAuthError::InvalidToken)?;
        let expires_at_unix = requested_at
            .checked_add(token.expires_in)
            .filter(|_| token.expires_in != 0)
            .ok_or(OAuthError::InvalidToken)?;
        if !valid_token(&token.access_token)
            || token
                .scope
                .as_deref()
                .is_some_and(|scope| scope != SHARE_SCOPE)
        {
            return Err(OAuthError::InvalidToken);
        }
        Ok(TokenSet {
            client_id: callback.client_id,
            record: TokenRecord {
                access_token: token.access_token,
                expires_at_unix,
            },
        })
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Zeroizing<String>,
    expires_in: u64,
    scope: Option<String>,
}

pub struct PendingAuthorization {
    listener: TcpListener,
    url: Url,
    client_id: String,
    redirect_uri: Url,
    state: Zeroizing<String>,
    verifier: Zeroizing<String>,
    deadline: Instant,
}

impl PendingAuthorization {
    /// Send only to the default browser. This URL contains the OAuth state.
    pub fn url(&self) -> &Url {
        &self.url
    }

    pub async fn wait_for_callback(self) -> Result<LoopbackCallback, OAuthError> {
        let code = timeout_at(self.deadline, self.receive_code())
            .await
            .map_err(|_| OAuthError::CallbackTimeout)??;
        Ok(LoopbackCallback {
            code,
            client_id: self.client_id,
            redirect_uri: self.redirect_uri,
            verifier: self.verifier,
        })
    }

    async fn receive_code(&self) -> Result<Zeroizing<String>, OAuthError> {
        loop {
            let (mut stream, peer) = self
                .listener
                .accept()
                .await
                .map_err(|_| OAuthError::ReadCallback)?;
            if !peer.ip().is_loopback() {
                continue;
            }
            let result = match timeout(CONNECTION_TIMEOUT, self.read_request(&mut stream)).await {
                Ok(result) => result,
                Err(_) => continue,
            };
            let (status, message) = match &result {
                Ok(_) => (
                    "200 OK",
                    "Consent received. Return to the terminal to check the result.",
                ),
                Err(OAuthError::ConsentDenied) => ("200 OK", "LinkedIn consent was declined."),
                Err(OAuthError::AuthorizationRejected) => (
                    "400 Bad Request",
                    "LinkedIn authorization failed. Return to the terminal for setup guidance.",
                ),
                Err(OAuthError::StateMismatch) => {
                    ("401 Unauthorized", "Invalid LinkedIn OAuth callback.")
                }
                Err(_) => ("400 Bad Request", "Invalid LinkedIn OAuth callback."),
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{message}",
                message.len(),
            );
            let _ = timeout(CONNECTION_TIMEOUT, stream.write_all(response.as_bytes())).await;
            // Browser probes and unsolicited requests cannot consume the attempt.
            match result {
                Err(OAuthError::InvalidCallback | OAuthError::StateMismatch) => continue,
                result => return result,
            }
        }
    }

    async fn read_request(&self, stream: &mut TcpStream) -> Result<Zeroizing<String>, OAuthError> {
        let mut input = Zeroizing::new([0_u8; MAX_CALLBACK_BYTES]);
        let mut filled = 0;
        loop {
            let count = stream
                .read(&mut input[filled..])
                .await
                .map_err(|_| OAuthError::InvalidCallback)?;
            if count == 0 {
                return Err(OAuthError::InvalidCallback);
            }
            filled += count;

            let mut headers = [httparse::EMPTY_HEADER; 32];
            let mut request = httparse::Request::new(&mut headers);
            match request
                .parse(&input[..filled])
                .map_err(|_| OAuthError::InvalidCallback)?
            {
                httparse::Status::Complete(length) if length == filled => {
                    return self.validate_request(&request);
                }
                httparse::Status::Partial if filled < input.len() => {}
                _ => return Err(OAuthError::InvalidCallback),
            }
        }
    }

    fn validate_request(
        &self,
        request: &httparse::Request<'_, '_>,
    ) -> Result<Zeroizing<String>, OAuthError> {
        let authority = &self.redirect_uri[Position::BeforeHost..Position::AfterPort];
        let mut hosts = request
            .headers
            .iter()
            .filter(|header| header.name.eq_ignore_ascii_case("Host"));
        if request.method != Some("GET")
            || hosts.next().map(|header| header.value) != Some(authority.as_bytes())
            || hosts.next().is_some()
            || request.headers.iter().any(|header| {
                header.name.eq_ignore_ascii_case("Transfer-Encoding")
                    || header.name.eq_ignore_ascii_case("Content-Length")
            })
        {
            return Err(OAuthError::InvalidCallback);
        }

        let (path, query) = request
            .path
            .and_then(|target| target.split_once('?'))
            .ok_or(OAuthError::InvalidCallback)?;
        if path != self.redirect_uri.path() || query.contains('#') {
            return Err(OAuthError::InvalidCallback);
        }

        let (mut state, mut code, mut error) = (None, None, None);
        for (key, value) in form_urlencoded::parse(query.as_bytes()) {
            let field = match key.as_ref() {
                "state" => &mut state,
                "code" => &mut code,
                "error" => &mut error,
                _ => continue,
            };
            if field.replace(Zeroizing::new(value.into_owned())).is_some() {
                return Err(OAuthError::InvalidCallback);
            }
        }
        let state = state.ok_or(OAuthError::StateMismatch)?;
        if !bool::from(state.as_bytes().ct_eq(self.state.as_bytes())) {
            return Err(OAuthError::StateMismatch);
        }
        match (code, error.as_deref().map(String::as_str)) {
            (Some(code), None) if valid_token(&code) => Ok(code),
            (None, Some("access_denied" | "user_cancelled_authorize" | "user_cancelled_login")) => {
                Err(OAuthError::ConsentDenied)
            }
            (None, Some(_)) => Err(OAuthError::AuthorizationRejected),
            _ => Err(OAuthError::InvalidCallback),
        }
    }
}

/// Proof of a validated loopback callback. Its private fields and move-only
/// ownership prevent callers from constructing or replaying an exchange.
pub struct LoopbackCallback {
    code: Zeroizing<String>,
    client_id: String,
    redirect_uri: Url,
    verifier: Zeroizing<String>,
}

fn random_url_safe() -> Result<Zeroizing<String>, OAuthError> {
    let mut random = Zeroizing::new([0_u8; 32]);
    SystemRandom::new()
        .fill(&mut *random)
        .map_err(|_| OAuthError::Randomness)?;
    Ok(Zeroizing::new(URL_SAFE_NO_PAD.encode(&*random)))
}

#[derive(Debug, PartialEq, Eq)]
pub enum OAuthError {
    Config(ConfigError),
    MissingClientId,
    MissingRedirect,
    MissingScope,
    Randomness,
    BindCallback,
    ReadCallback,
    CallbackTimeout,
    InvalidCallback,
    StateMismatch,
    ConsentDenied,
    AuthorizationRejected,
    InvalidClock,
    Transport,
    TokenRejected(u16),
    InvalidToken,
}

impl fmt::Display for OAuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Config(error) => return fmt::Display::fmt(error, formatter),
            Self::MissingClientId => "set LINKEDIN_CLIENT_ID before starting LinkedIn OAuth",
            Self::MissingRedirect => "set LINKEDIN_REDIRECT_URI before starting LinkedIn OAuth",
            Self::MissingScope => "set LINKEDIN_ALLOWED_SCOPES to w_member_social after enabling Share on LinkedIn",
            Self::Randomness => "could not generate secure randomness for LinkedIn OAuth",
            Self::BindCallback => "could not bind the LinkedIn loopback callback; check the configured address and whether the port is in use",
            Self::ReadCallback => "could not accept the LinkedIn loopback callback",
            Self::CallbackTimeout => "no valid LinkedIn callback arrived; check the browser and registered redirect URI, and confirm that LinkedIn Developer Support enabled native PKCE OAuth for the app",
            Self::InvalidCallback => "invalid LinkedIn OAuth callback",
            Self::StateMismatch => "LinkedIn OAuth callback did not match the pending authorization",
            Self::ConsentDenied => "LinkedIn consent was declined",
            Self::AuthorizationRejected => "LinkedIn rejected authorization; check Share on LinkedIn and the redirect URI, and confirm that LinkedIn Developer Support enabled native PKCE OAuth for the app",
            Self::InvalidClock => "the system clock is invalid; correct it before using LinkedIn OAuth",
            Self::Transport => "could not complete the HTTPS request to LinkedIn; check connectivity and start authorization again",
            Self::TokenRejected(status) => return write!(
                formatter,
                "LinkedIn rejected the token exchange (HTTP {status}); verify the client ID and redirect URI, confirm that LinkedIn Developer Support enabled native PKCE OAuth, and start authorization again"
            ),
            Self::InvalidToken => "LinkedIn returned an invalid access token, scope, or expiration",
        };
        formatter.write_str(message)
    }
}

impl Error for OAuthError {}
