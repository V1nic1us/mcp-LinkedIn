use std::fmt::Write as _;

use ring::digest::{digest, SHA256};
use rust_mcp_sdk::schema::Tool;
use serde::{Deserialize, Serialize};
use serde_json::json;
use url::Url;

use crate::{
    config::AppConfig,
    linkedin::{
        client::{validate_text, LinkedInClient},
        models::{LinkedInApiError, PublishedPost},
    },
};

use super::{normalize_text, tool};

pub(super) const PREPARE_NAME: &str = "prepare_post";
pub(super) const PUBLISH_NAME: &str = "publish_post";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PreparePostArgs {
    text: String,
    url: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PublishPostArgs {
    text: String,
    confirmation: String,
}

#[derive(Serialize)]
pub(super) struct PreparedPost {
    text: String,
    confirmation: String,
}

pub(super) fn prepare_definition() -> Tool {
    tool(
        PREPARE_NAME,
        "Prepare the exact text of a public LinkedIn post locally. Show the returned text to the user and wait for explicit approval before calling publish_post.",
        json!({
            "text": {"type": "string", "description": "Post text. The final preview, including any URL, must fit within 3000 UTF-16 code units."},
            "url": {"type": ["string", "null"], "description": "Optional absolute HTTP(S) URL to append as plain text."}
        }),
        &["text"],
        true,
    )
}

pub(super) fn publish_definition() -> Tool {
    tool(
        PUBLISH_NAME,
        "Publish a public LinkedIn text post. The MCP host must require human approval for every publish call. The digest binds the exact preview and cannot prove human approval. Each confirmation is single-use within this server process. If the result is uncertain, ask the user to check their LinkedIn activity and stop.",
        json!({
            "text": {"type": "string", "description": "The text returned from prepare_post, copied verbatim, including any appended URL."},
            "confirmation": {"type": "string", "pattern": "^PUBLISH:[0-9a-f]{64}$", "description": "The exact confirmation from prepare_post. Supply it only after the user explicitly approves the preview."}
        }),
        &["text", "confirmation"],
        false,
    )
}

pub(super) fn prepare_post(args: PreparePostArgs) -> Result<PreparedPost, String> {
    let mut text = normalize_text(&args.text);
    validate_text(&text).map_err(|error| error.to_string())?;
    if let Some(url) = args.url.as_deref() {
        let url = post_url(url)?;
        text.push_str("\n\n");
        text.push_str(url.as_str());
        validate_text(&text).map_err(|error| error.to_string())?;
    }

    Ok(PreparedPost {
        confirmation: confirmation_for(&text),
        text,
    })
}

pub(super) async fn publish_post(
    config: &AppConfig,
    args: PublishPostArgs,
) -> Result<PublishedPost, String> {
    validate_text(&args.text).map_err(|error| error.to_string())?;
    if args.text.trim() != args.text
        || args.text.contains('\r')
        || args.confirmation != confirmation_for(&args.text)
    {
        return Err("confirmation must match the exact approved text from prepare_post".into());
    }
    if !config.allows_publish_scopes() {
        return Err(LinkedInApiError::AccessRequired.to_string());
    }

    let client = LinkedInClient::new(config).map_err(|error| error.to_string())?;
    client
        .create_text_post(&args.text)
        .await
        .map_err(|error| error.to_string())
}

fn post_url(input: &str) -> Result<Url, &'static str> {
    const INVALID_URL: &str = "url must be an absolute HTTP(S) URL without credentials";
    let input = input.trim();
    if input
        .chars()
        .any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return Err(INVALID_URL);
    }
    let url = Url::parse(input).map_err(|_| INVALID_URL)?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.has_host()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(INVALID_URL);
    }
    Ok(url)
}

fn confirmation_for(text: &str) -> String {
    let mut confirmation = String::from("PUBLISH:");
    for byte in digest(&SHA256, text.as_bytes()).as_ref() {
        write!(&mut confirmation, "{byte:02x}").expect("writing to a String cannot fail");
    }
    confirmation
}
