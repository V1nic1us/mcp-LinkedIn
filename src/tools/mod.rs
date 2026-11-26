mod posts;
mod profile;
mod status;

use std::sync::Arc;

use rust_mcp_sdk::schema::{
    schema_utils::CallToolError, CallToolRequestParams, CallToolResult, ContentBlock, Tool,
    ToolAnnotations, ToolInputSchema,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{Map, Value};

use crate::config::AppConfig;

pub fn list() -> Vec<Tool> {
    vec![
        profile::definition(),
        status::definition(),
        posts::prepare_definition(),
        posts::publish_definition(),
    ]
}

pub async fn call(
    config: Arc<AppConfig>,
    params: CallToolRequestParams,
) -> Result<CallToolResult, CallToolError> {
    let arguments = params.arguments.unwrap_or_default();
    match params.name.as_str() {
        profile::NAME => respond(
            parse_arguments(arguments, "provide a supported field (headline or about), proposed_text, and optional current_text")
                .and_then(profile::draft_profile_update),
        ),
        status::NAME => {
            if let Err(message) = parse_arguments::<status::StatusArgs>(
                arguments,
                "linkedin_connection_status takes no arguments",
            ) {
                return Ok(tool_error(message));
            }
            respond(status::linkedin_connection_status(config).await)
        }
        posts::PREPARE_NAME => respond(
            parse_arguments(arguments, "provide text and an optional URL for the post")
                .map_err(str::to_owned)
                .and_then(posts::prepare_post),
        ),
        posts::PUBLISH_NAME => {
            let args = match parse_arguments(
                arguments,
                "provide the exact preview text and explicit confirmation from prepare_post",
            ) {
                Ok(args) => args,
                Err(message) => return Ok(tool_error(message)),
            };
            respond(posts::publish_post(&config, args).await)
        }
        _ => Err(CallToolError::unknown_tool("unrecognized tool")),
    }
}

fn tool(
    name: &str,
    description: &str,
    properties: Value,
    required: &[&str],
    read_only: bool,
) -> Tool {
    let properties = serde_json::from_value(properties).expect("valid tool property schemas");
    Tool {
        name: name.to_owned(),
        description: Some(description.to_owned()),
        input_schema: ToolInputSchema::new(
            required.iter().map(|name| (*name).to_owned()).collect(),
            Some(properties),
            None,
        ),
        annotations: Some(ToolAnnotations {
            read_only_hint: Some(read_only),
            open_world_hint: Some(!read_only),
            destructive_hint: (!read_only).then_some(true),
            idempotent_hint: (!read_only).then_some(false),
            ..ToolAnnotations::default()
        }),
        execution: None,
        icons: Vec::new(),
        meta: None,
        output_schema: None,
        title: None,
    }
}

fn parse_arguments<T: DeserializeOwned>(
    arguments: Map<String, Value>,
    error: &'static str,
) -> Result<T, &'static str> {
    serde_json::from_value(Value::Object(arguments)).map_err(|_| error)
}

fn respond(
    result: Result<impl Serialize, impl std::fmt::Display>,
) -> Result<CallToolResult, CallToolError> {
    let output = match result {
        Ok(output) => output,
        Err(error) => return Ok(tool_error(error.to_string())),
    };
    let value = serde_json::to_value(output)
        .map_err(|_| CallToolError::from_message("could not serialize tool result"))?;
    let Value::Object(fields) = value else {
        return Err(CallToolError::from_message("expected a tool result object"));
    };
    Ok(CallToolResult {
        content: vec![ContentBlock::text_content(
            Value::Object(fields.clone()).to_string(),
        )],
        is_error: None,
        meta: None,
        structured_content: Some(fields),
    })
}

fn tool_error(message: impl Into<String>) -> CallToolResult {
    CallToolResult {
        content: vec![ContentBlock::text_content(message.into())],
        is_error: Some(true),
        meta: None,
        structured_content: None,
    }
}

fn normalize_text(text: &str) -> String {
    text.trim().replace("\r\n", "\n").replace('\r', "\n")
}
