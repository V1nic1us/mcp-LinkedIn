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
    signature::{RsaPublicKeyComponents, RSA_PKCS1_2048_8192_SHA256},
};
use serde::Deserialize;
use serde_json::Value;
use subtle::ConstantTimeEq;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::{sleep_until, timeout, timeout_at, Instant},
};
use url::{form_urlencoded, Host, Position, Url};
use zeroize::Zeroizing;

use crate::config::{parse_loopback_redirect, AppConfig, ConfigError, PUBLISH_SCOPES};

use super::{valid_token, ScopeGrant, TokenRecord, TokenSet};

const AUTHORIZE_ENDPOINT: &str = "https://www.linkedin.com/oauth/native-pkce/authorization";
const TOKEN_ENDPOINT: &str = "https://www.linkedin.com/oauth/v2/accessToken";
const JWKS_ENDPOINT: &str = "https://www.linkedin.com/oauth/openid/jwks";
const CONSENT_TIMEOUT: Duration = Duration::from_secs(300);
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(2);
const TOKEN_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_CALLBACK_BYTES: usize = 8192;
const MAX_TOKEN_BODY_BYTES: usize = 32768;
const MAX_JWKS_BODY_BYTES: usize = 65536;

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
        if !config.allows_publish_scopes() {
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
        let listener = TcpListener::bind(SocketAddr::new(ip, 0))
            .await
            .map_err(|_| OAuthError::BindCallback)?;
        let port = listener
            .local_addr()
            .map_err(|_| OAuthError::BindCallback)?
            .port();
        let mut redirect_uri = self.redirect_uri.clone();
        redirect_uri
            .set_port(Some(port))
            .map_err(|_| OAuthError::Config(ConfigError::InvalidRedirect))?;

        let state = random_url_safe()?;
        let verifier = random_url_safe()?;
        let nonce = random_url_safe()?;
        let challenge = URL_SAFE_NO_PAD.encode(digest(&SHA256, verifier.as_bytes()).as_ref());
        let mut url =
            Url::parse(AUTHORIZE_ENDPOINT).expect("fixed LinkedIn authorization endpoint");
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", redirect_uri.as_str())
            .append_pair("scope", &requested_scopes())
            .append_pair("nonce", &nonce)
            .append_pair("state", &state)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256");

        Ok(PendingAuthorization {
            listener,
            url,
            client_id: self.client_id.clone(),
            redirect_uri,
            state,
            verifier,
            nonce,
            deadline: Instant::now() + CONSENT_TIMEOUT,
        })
    }

    pub async fn exchange_code(&self, callback: LoopbackCallback) -> Result<TokenSet, OAuthError> {
        let mut redirect_template = callback.redirect_uri.clone();
        redirect_template
            .set_port(None)
            .map_err(|_| OAuthError::InvalidCallback)?;
        if redirect_template != self.redirect_uri || callback.client_id != self.client_id {
            return Err(OAuthError::InvalidCallback);
        }

        let deadline = callback.deadline;
        if Instant::now() >= deadline {
            return Err(OAuthError::ExchangeExpired);
        }
        let result = tokio::select! {
            biased;
            () = sleep_until(deadline) => return Err(OAuthError::ExchangeExpired),
            result = self.request_token(callback) => result,
        };
        // Bounded JSON parsing runs synchronously and cannot be preempted.
        if Instant::now() >= deadline {
            return Err(OAuthError::ExchangeExpired);
        }
        result
    }

    async fn request_token(&self, callback: LoopbackCallback) -> Result<TokenSet, OAuthError> {
        // Start the local lifetime before the request, so latency cannot extend it.
        let requested_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| OAuthError::InvalidClock)?
            .as_secs();
        let request = self.http.post(TOKEN_ENDPOINT).form(&[
            ("grant_type", "authorization_code"),
            ("code", callback.code.as_str()),
            ("redirect_uri", callback.redirect_uri.as_str()),
            ("client_id", self.client_id.as_str()),
            ("code_verifier", callback.verifier.as_str()),
        ]);
        if Instant::now() >= callback.deadline {
            return Err(OAuthError::ExchangeExpired);
        }
        let mut response = request.send().await.map_err(|_| OAuthError::Transport)?;
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
        let scopes: Vec<_> = token.scope.split_ascii_whitespace().collect();
        if !valid_token(&token.access_token)
            || scopes.len() != PUBLISH_SCOPES.len()
            || !PUBLISH_SCOPES.iter().all(|scope| scopes.contains(scope))
        {
            return Err(OAuthError::InvalidToken);
        }
        let id_token = token
            .id_token
            .as_deref()
            .ok_or(OAuthError::MissingIdentityToken)?;
        let subject = validate_id_token(
            &self.http,
            id_token,
            &self.client_id,
            callback.nonce.as_str(),
        )
        .await?;
        let subject_hash: [u8; 32] = digest(&SHA256, subject.as_bytes())
            .as_ref()
            .try_into()
            .map_err(|_| OAuthError::InvalidIdentityToken)?;
        Ok(TokenSet {
            client_id: callback.client_id,
            record: TokenRecord {
                access_token: token.access_token,
                expires_at_unix,
                grant: ScopeGrant::OpenIdAndShare,
                subject_hash: Some(subject_hash),
            },
        })
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Zeroizing<String>,
    expires_in: u64,
    id_token: Option<Zeroizing<String>>,
    // OAuth permits omission when the grant matches the requested scopes.
    #[serde(default = "requested_scopes")]
    scope: String,
}

async fn validate_id_token(
    http: &reqwest::Client,
    token: &str,
    client_id: &str,
    nonce: &str,
) -> Result<String, OAuthError> {
    let mut sections = token.split('.');
    let (Some(encoded_header), Some(encoded_claims), Some(encoded_signature), None) = (
        sections.next(),
        sections.next(),
        sections.next(),
        sections.next(),
    ) else {
        return Err(OAuthError::InvalidIdentityToken);
    };
    let decode = |part: &str| {
        URL_SAFE_NO_PAD
            .decode(part)
            .map_err(|_| OAuthError::InvalidIdentityToken)
    };
    let header: Value = serde_json::from_slice(&decode(encoded_header)?)
        .map_err(|_| OAuthError::InvalidIdentityToken)?;
    if header.get("alg").and_then(Value::as_str) != Some("RS256") {
        return Err(OAuthError::InvalidIdentityToken);
    }
    let kid = header
        .get("kid")
        .and_then(Value::as_str)
        .filter(|kid| !kid.is_empty())
        .ok_or(OAuthError::InvalidIdentityToken)?;
    let claims: Value = serde_json::from_slice(&decode(encoded_claims)?)
        .map_err(|_| OAuthError::InvalidIdentityToken)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| OAuthError::InvalidClock)?
        .as_secs();
    let valid_audience = claims.get("aud").is_some_and(|audience| match audience {
        Value::String(value) => value == client_id,
        Value::Array(items) => items.len() == 1 && items[0].as_str() == Some(client_id),
        _ => false,
    });
    let valid_authorized_party = claims
        .get("azp")
        .is_none_or(|authorized_party| authorized_party.as_str() == Some(client_id));
    let expiration = claims.get("exp").and_then(Value::as_u64);
    let issued_at = claims.get("iat").and_then(Value::as_u64);
    let subject = claims
        .get("sub")
        .and_then(Value::as_str)
        .filter(|subject| valid_subject(subject))
        .ok_or(OAuthError::InvalidIdentityToken)?;
    let claim_nonce = claims.get("nonce").and_then(Value::as_str);
    if claims.get("iss").and_then(Value::as_str) != Some("https://www.linkedin.com")
        || !valid_audience
        || !valid_authorized_party
        || expiration.is_none_or(|value| value <= now)
        || issued_at.is_none_or(|value| value > now.saturating_add(60))
        || expiration
            .zip(issued_at)
            .is_none_or(|(exp, iat)| exp <= iat)
        || claim_nonce.is_none_or(|value| !bool::from(value.as_bytes().ct_eq(nonce.as_bytes())))
    {
        return Err(OAuthError::InvalidIdentityToken);
    }

    let mut response = http
        .get(JWKS_ENDPOINT)
        .send()
        .await
        .map_err(|_| OAuthError::IdentityKeysUnavailable)?;
    if !response.status().is_success() {
        return Err(OAuthError::IdentityKeysUnavailable);
    }
    let mut body = Zeroizing::new(Vec::new());
    while let Some(chunk) = response.chunk().await.map_err(|_| OAuthError::Transport)? {
        if chunk.len() > MAX_JWKS_BODY_BYTES.saturating_sub(body.len()) {
            return Err(OAuthError::InvalidIdentityToken);
        }
        body.extend_from_slice(&chunk);
    }
    let keys: Value =
        serde_json::from_slice(&body).map_err(|_| OAuthError::InvalidIdentityToken)?;
    let mut matches = keys
        .get("keys")
        .and_then(Value::as_array)
        .ok_or(OAuthError::InvalidIdentityToken)?
        .iter()
        .filter(|key| key.get("kid").and_then(Value::as_str) == Some(kid));
    let key = matches.next().ok_or(OAuthError::InvalidIdentityToken)?;
    if matches.next().is_some() || key.get("kty").and_then(Value::as_str) != Some("RSA") {
        return Err(OAuthError::InvalidIdentityToken);
    }
    let modulus = decode(
        key.get("n")
            .and_then(Value::as_str)
            .ok_or(OAuthError::InvalidIdentityToken)?,
    )?;
    let exponent = decode(
        key.get("e")
            .and_then(Value::as_str)
            .ok_or(OAuthError::InvalidIdentityToken)?,
    )?;
    RsaPublicKeyComponents {
        n: &modulus,
        e: &exponent,
    }
    .verify(
        &RSA_PKCS1_2048_8192_SHA256,
        format!("{encoded_header}.{encoded_claims}").as_bytes(),
        &decode(encoded_signature)?,
    )
    .map_err(|_| OAuthError::InvalidIdentityToken)?;
    Ok(subject.to_owned())
}

fn valid_subject(subject: &str) -> bool {
    !subject.is_empty()
        && subject.len() <= 255
        && subject
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn requested_scopes() -> String {
    PUBLISH_SCOPES.join(" ")
}

pub struct PendingAuthorization {
    listener: TcpListener,
    url: Url,
    client_id: String,
    redirect_uri: Url,
    state: Zeroizing<String>,
    verifier: Zeroizing<String>,
    nonce: Zeroizing<String>,
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
            nonce: self.nonce,
            deadline: self.deadline,
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
    nonce: Zeroizing<String>,
    deadline: Instant,
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
    ExchangeExpired,
    InvalidCallback,
    StateMismatch,
    ConsentDenied,
    AuthorizationRejected,
    InvalidClock,
    Transport,
    TokenRejected(u16),
    InvalidToken,
    MissingIdentityToken,
    InvalidIdentityToken,
    IdentityKeysUnavailable,
}

impl fmt::Display for OAuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Config(error) => return fmt::Display::fmt(error, formatter),
            Self::MissingClientId => "set LINKEDIN_CLIENT_ID before starting LinkedIn OAuth",
            Self::MissingRedirect => "set LINKEDIN_REDIRECT_URI before starting LinkedIn OAuth",
            Self::MissingScope => "set LINKEDIN_ALLOWED_SCOPES to 'openid w_member_social' after enabling Sign In with LinkedIn using OpenID Connect and Share on LinkedIn",
            Self::Randomness => "could not generate secure randomness for LinkedIn OAuth",
            Self::BindCallback => "could not bind the LinkedIn loopback callback; check that the configured loopback address is available",
            Self::ReadCallback => "could not accept the LinkedIn loopback callback",
            Self::CallbackTimeout => "no valid LinkedIn callback arrived; check the browser and loopback callback settings, and confirm that LinkedIn Developer Support enabled native PKCE OAuth for the app",
            Self::ExchangeExpired => "the LinkedIn authorization window expired; start authorization again",
            Self::InvalidCallback => "invalid LinkedIn OAuth callback",
            Self::StateMismatch => "LinkedIn OAuth callback did not match the pending authorization",
            Self::ConsentDenied => "LinkedIn consent was declined",
            Self::AuthorizationRejected => "LinkedIn rejected authorization; check both LinkedIn products and the redirect URI, and confirm that LinkedIn Developer Support enabled native PKCE OAuth for the app",
            Self::InvalidClock => "the system clock is invalid; correct it before using LinkedIn OAuth",
            Self::Transport => "could not complete the HTTPS request to LinkedIn; check connectivity and start authorization again",
            Self::TokenRejected(status) => return write!(
                formatter,
                "LinkedIn rejected the token exchange (HTTP {status}); verify the client ID and redirect URI, confirm that LinkedIn Developer Support enabled native PKCE OAuth, and start authorization again"
            ),
            Self::InvalidToken => "LinkedIn returned an invalid access token, scope, or expiration",
            Self::MissingIdentityToken => "LinkedIn's native PKCE token response omitted the required OpenID Connect ID token; confirm that OIDC ID tokens are enabled for this app and native flow",
            Self::InvalidIdentityToken => "LinkedIn did not return a valid nonce-bound OpenID Connect ID token; reauthorize after confirming that Sign In with LinkedIn using OpenID Connect is enabled for native PKCE",
            Self::IdentityKeysUnavailable => "could not validate LinkedIn's OpenID Connect signature because the official signing keys were unavailable",
        };
        formatter.write_str(message)
    }
}

impl Error for OAuthError {}
