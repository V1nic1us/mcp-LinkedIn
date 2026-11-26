mod stdio;

use std::{io, process::ExitCode, sync::Arc};

use async_trait::async_trait;
use linkedin_mcp::config::AppConfig;
use rust_mcp_sdk::{
    mcp_server::{server_runtime, McpServerOptions, ServerHandler},
    schema::{
        Implementation, InitializeRequestParams, InitializeResult, ListToolsResult,
        PaginatedRequestParams, ProtocolVersion, RpcError, ServerCapabilities,
        ServerCapabilitiesTools,
    },
    McpServer, ToMcpServerHandler,
};

use crate::stdio::{StdioFailure, StdioTransport};

struct LinkedInServer;

#[async_trait]
impl ServerHandler for LinkedInServer {
    async fn handle_initialize_request(
        &self,
        params: InitializeRequestParams,
        runtime: Arc<dyn McpServer>,
    ) -> Result<InitializeResult, RpcError> {
        let mut server_info = runtime.server_info().clone();
        if let Ok(version) = ProtocolVersion::try_from(params.protocol_version.as_str()) {
            if version <= ProtocolVersion::latest() {
                server_info.protocol_version = version.to_string();
            }
        }

        runtime
            .set_client_details(params)
            .await
            .map_err(|_| RpcError::internal_error())?;

        Ok(server_info)
    }

    async fn handle_list_tools_request(
        &self,
        _params: Option<PaginatedRequestParams>,
        _runtime: Arc<dyn McpServer>,
    ) -> Result<ListToolsResult, RpcError> {
        Ok(ListToolsResult {
            tools: Vec::new(),
            meta: None,
            next_cursor: None,
        })
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    if let Err(error) = AppConfig::from_env() {
        eprintln!("linkedin-mcp: {error}");
        return ExitCode::FAILURE;
    }

    tracing_subscriber::fmt()
        .with_max_level(tracing_subscriber::filter::LevelFilter::WARN)
        .with_writer(io::stderr)
        // SDK event fields can contain request IDs or other client data.
        .fmt_fields(tracing_subscriber::fmt::format::debug_fn(|_, _, _| Ok(())))
        .with_file(true)
        .with_line_number(true)
        .with_ansi(false)
        .init();

    match serve_stdio().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("linkedin-mcp: {message}");
            ExitCode::FAILURE
        }
    }
}

async fn serve_stdio() -> Result<(), &'static str> {
    let (transport, mut failures) =
        StdioTransport::open().map_err(|_| "could not start MCP stdio transport")?;
    let server = server_runtime::create_server(McpServerOptions {
        server_details: InitializeResult {
            server_info: Implementation {
                name: env!("CARGO_PKG_NAME").into(),
                version: env!("CARGO_PKG_VERSION").into(),
                title: None,
                description: None,
                icons: Vec::new(),
                website_url: None,
            },
            protocol_version: ProtocolVersion::latest().to_string(),
            capabilities: ServerCapabilities {
                tools: Some(ServerCapabilitiesTools::default()),
                ..ServerCapabilities::default()
            },
            instructions: None,
            meta: None,
        },
        transport,
        handler: LinkedInServer.to_mcp_server_handler(),
        task_store: None,
        client_task_store: None,
        message_observer: None,
    });

    let result = tokio::select! {
        biased;
        Some(failure) = failures.recv() => return stdio_outcome(failure),
        result = server.start() => result,
    };

    // The worker reports failure before closing input or acknowledging a write.
    // Check again if the server completed during the same select poll.
    match failures.try_recv() {
        Ok(failure) => stdio_outcome(failure),
        Err(_) => result.map_err(|_| "MCP stdio session failed"),
    }
}

fn stdio_outcome(failure: StdioFailure) -> Result<(), &'static str> {
    match failure {
        StdioFailure::Read => Err("could not read MCP stdin"),
        StdioFailure::Write(io::ErrorKind::BrokenPipe) => Ok(()),
        StdioFailure::Write(_) => Err("could not write MCP stdout"),
    }
}
