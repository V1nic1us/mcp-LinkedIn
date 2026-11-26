use std::{io, process::ExitCode, sync::Arc};

use async_trait::async_trait;
use linkedin_mcp::config::AppConfig;
use rust_mcp_sdk::{
    error::{McpSdkError, SdkResult},
    mcp_server::{server_runtime, McpServerOptions, ServerHandler},
    schema::{
        Implementation, InitializeRequestParams, InitializeResult, ListToolsResult,
        PaginatedRequestParams, ProtocolVersion, RpcError, ServerCapabilities,
        ServerCapabilitiesTools,
    },
    McpServer, StdioTransport, ToMcpServerHandler, TransportError, TransportOptions,
};

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
        .with_ansi(false)
        .init();

    match serve_stdio().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(McpSdkError::Io(error) | McpSdkError::Transport(TransportError::Io(error)))
            if error.kind() == io::ErrorKind::BrokenPipe =>
        {
            ExitCode::SUCCESS
        }
        Err(_) => {
            eprintln!("linkedin-mcp: MCP stdio session failed");
            ExitCode::FAILURE
        }
    }
}

async fn serve_stdio() -> SdkResult<()> {
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
        transport: StdioTransport::new(TransportOptions::default())?,
        handler: LinkedInServer.to_mcp_server_handler(),
        task_store: None,
        client_task_store: None,
        message_observer: None,
    });

    server.start().await
}
