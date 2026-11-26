use std::{sync::Arc, time::Duration};

use reqwest::{
    header::{CONTENT_TYPE, RETRY_AFTER},
    Response, StatusCode,
};
use serde::Deserialize;
use serde_json::json;
use zeroize::Zeroizing;

use crate::config::{AppConfig, PUBLISH_SCOPES};

use super::{
    models::{ConnectionStatus, LinkedInApiError, PublishedPost},
    token_store::{is_expired, TokenStore, TokenStoreError},
    TokenSet,
};

const USERINFO_ENDPOINT: &str = "https://api.linkedin.com/v2/userinfo";
const UGC_POSTS_ENDPOINT: &str = "https://api.linkedin.com/v2/ugcPosts";
const MAX_USERINFO_BYTES: usize = 16 * 1024;
const MAX_POST_LENGTH: usize = 3000;

pub struct LinkedInClient {
    http: reqwest::Client,
    store: Option<Arc<TokenStore>>,
    allows_publish_scopes: bool,
}

impl LinkedInClient {
    pub fn new(config: &AppConfig) -> Result<Self, LinkedInApiError> {
        let store = config
            .linkedin_client_id
            .as_deref()
            .filter(|id| !id.trim().is_empty())
            .map(|_| TokenStore::new(config).map(Arc::new))
            .transpose()?;
        let http = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            // Avoid stale pooled connections and transport-level stream replays.
            .http1_only()
            .pool_max_idle_per_host(0)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|_| LinkedInApiError::Transport)?;
        Ok(Self {
            http,
            store,
            allows_publish_scopes: config.allows_publish_scopes(),
        })
    }

    /// Read local credential state from the keyring without contacting LinkedIn.
    pub fn connection_status(&self) -> ConnectionStatus {
        let tokens = self
            .store
            .as_ref()
            .and_then(|store| store.load().ok().flatten());
        let approved_scopes = tokens.map_or_else(Vec::new, |tokens| tokens.scopes().to_vec());
        ConnectionStatus {
            connected: self.allows_publish_scopes
                && PUBLISH_SCOPES
                    .iter()
                    .all(|scope| approved_scopes.contains(scope)),
            approved_scopes,
        }
    }

    /// Publish the exact text after the caller has obtained explicit confirmation.
    pub async fn create_text_post(&self, text: &str) -> Result<PublishedPost, LinkedInApiError> {
        if !self.allows_publish_scopes {
            return Err(LinkedInApiError::AccessRequired);
        }
        let store = self.store.clone().ok_or(LinkedInApiError::AccessRequired)?;
        let tokens = tokio::task::spawn_blocking(move || store.load())
            .await
            .map_err(|_| TokenStoreError::Unavailable)??
            .ok_or(LinkedInApiError::AccessRequired)?;
        if !PUBLISH_SCOPES
            .iter()
            .all(|scope| tokens.scopes().contains(scope))
        {
            return Err(LinkedInApiError::AccessRequired);
        }
        validate_text(text)?;
        let author = self.resolve_author(&tokens).await?;
        let body = json!({
            "author": author.as_str(),
            "lifecycleState": "PUBLISHED",
            "specificContent": {
                "com.linkedin.ugc.ShareContent": {
                    "shareCommentary": { "text": text },
                    "shareMediaCategory": "NONE"
                }
            },
            "visibility": { "com.linkedin.ugc.MemberNetworkVisibility": "PUBLIC" }
        });
        if is_expired(tokens.expires_at_unix())? {
            return Err(LinkedInApiError::AccessRequired);
        }

        let response = self
            .http
            .post(UGC_POSTS_ENDPOINT)
            .bearer_auth(tokens.access_token())
            .header("X-Restli-Protocol-Version", "2.0.0")
            .header(CONTENT_TYPE, "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|_| LinkedInApiError::PublishOutcomeUnknown)?;
        let status = response.status();
        if status != StatusCode::CREATED {
            let error = if status.is_client_error()
                && !matches!(status, StatusCode::REQUEST_TIMEOUT | StatusCode::CONFLICT)
            {
                response_error(&response)
            } else {
                LinkedInApiError::PublishOutcomeUnknown
            };
            return Err(error);
        }

        let mut ids = response.headers().get_all("x-restli-id").iter();
        let urn = ids
            .next()
            .and_then(|value| value.to_str().ok())
            .filter(|value| valid_post_urn(value) && ids.next().is_none())
            .ok_or(LinkedInApiError::PublishOutcomeUnknown)?;
        Ok(PublishedPost {
            urn: urn.to_owned(),
            url: None,
        })
    }

    async fn resolve_author(&self, tokens: &TokenSet) -> Result<String, LinkedInApiError> {
        let mut response = self
            .http
            .get(USERINFO_ENDPOINT)
            .bearer_auth(tokens.access_token())
            .send()
            .await
            .map_err(|_| LinkedInApiError::Transport)?;
        if response.status() != StatusCode::OK {
            return Err(response_error(&response));
        }

        let mut body = Zeroizing::new(Vec::new());
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| LinkedInApiError::Transport)?
        {
            if chunk.len() > MAX_USERINFO_BYTES - body.len() {
                return Err(LinkedInApiError::InvalidIdentityResponse);
            }
            body.extend_from_slice(&chunk);
        }
        let UserInfo { sub } =
            serde_json::from_slice(&body).map_err(|_| LinkedInApiError::InvalidIdentityResponse)?;
        if sub.is_empty()
            || sub.len() > 255
            || !sub
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(LinkedInApiError::InvalidIdentityResponse);
        }
        Ok(format!("urn:li:person:{}", sub.as_str()))
    }
}

#[derive(Deserialize)]
struct UserInfo {
    sub: Zeroizing<String>,
}

fn validate_text(text: &str) -> Result<(), LinkedInApiError> {
    if text.encode_utf16().take(MAX_POST_LENGTH + 1).count() > MAX_POST_LENGTH
        || text.trim().is_empty()
        || text
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\t' | '\n' | '\r'))
    {
        return Err(LinkedInApiError::InvalidText);
    }
    Ok(())
}

fn valid_post_urn(value: &str) -> bool {
    value
        .strip_prefix("urn:li:ugcPost:")
        .or_else(|| value.strip_prefix("urn:li:share:"))
        .is_some_and(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
}

fn response_error(response: &Response) -> LinkedInApiError {
    // LinkedIn's error messages may echo request or account data.
    match response.status() {
        StatusCode::UNAUTHORIZED => LinkedInApiError::Unauthorized,
        StatusCode::FORBIDDEN => LinkedInApiError::AccessRequired,
        StatusCode::TOO_MANY_REQUESTS => LinkedInApiError::RateLimited {
            retry_after_seconds: response
                .headers()
                .get(RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .filter(|value| value.bytes().all(|byte| byte.is_ascii_digit()))
                .and_then(|value| value.parse().ok()),
        },
        status => LinkedInApiError::Rejected {
            status: status.as_u16(),
        },
    }
}
