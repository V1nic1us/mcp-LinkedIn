use std::sync::Arc;

use rust_mcp_sdk::schema::Tool;
use serde::Deserialize;
use serde_json::json;

use crate::{
    config::AppConfig,
    linkedin::{client::LinkedInClient, models::ConnectionStatus},
};

use super::tool;

pub(super) const NAME: &str = "linkedin_connection_status";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StatusArgs {}

pub(super) fn definition() -> Tool {
    tool(
        NAME,
        "Read posting readiness and approved scopes from local LinkedIn credential metadata.",
        json!({}),
        &[],
        true,
    )
}

pub(super) async fn linkedin_connection_status(
    config: Arc<AppConfig>,
) -> Result<ConnectionStatus, &'static str> {
    tokio::task::spawn_blocking(move || LinkedInClient::local_connection_status(&config))
        .await
        .map_err(|_| "could not read local LinkedIn connection status")
}
