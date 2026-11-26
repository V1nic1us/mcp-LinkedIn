mod stdio;

use std::{env, error::Error, io, process::ExitCode, sync::Arc};

#[cfg(target_os = "linux")]
use std::process::Stdio;

use async_trait::async_trait;
#[cfg(target_os = "linux")]
use linkedin_mcp::auth_session;
#[cfg(target_os = "linux")]
use linkedin_mcp::linkedin::{oauth::OAuthClient, token_store::TokenStore};
use linkedin_mcp::{config::AppConfig, tools};
use rust_mcp_sdk::{
    mcp_server::{server_runtime, McpServerOptions, ServerHandler},
    schema::{
        schema_utils::CallToolError, CallToolRequestParams, CallToolResult, Implementation,
        InitializeRequestParams, InitializeResult, ListToolsResult, PaginatedRequestParams,
        ProtocolVersion, RpcError, ServerCapabilities, ServerCapabilitiesTools,
    },
    McpServer, ToMcpServerHandler,
};

use crate::stdio::{StdioFailure, StdioTransport};

struct LinkedInServer {
    config: Arc<AppConfig>,
}

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
            tools: tools::list(),
            meta: None,
            next_cursor: None,
        })
    }

    async fn handle_call_tool_request(
        &self,
        params: CallToolRequestParams,
        _runtime: Arc<dyn McpServer>,
    ) -> Result<CallToolResult, CallToolError> {
        tools::call(self.config.clone(), params).await
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<_> = env::args_os().skip(1).collect();
    let result = match args.as_slice() {
        [] => run_server().await,
        #[cfg(target_os = "linux")]
        [auth, command] if auth == "auth" && command == "login" => login().await,
        #[cfg(target_os = "linux")]
        [auth, command] if auth == "auth" && command == "logout" => logout(),
        #[cfg(not(target_os = "linux"))]
        [auth, command] if auth == "auth" && (command == "login" || command == "logout") => {
            Err(linkedin_mcp::linkedin::token_store::TokenStoreError::Unavailable.into())
        }
        _ => Err("usage: linkedin-mcp [auth login|auth logout]".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("linkedin-mcp: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run_server() -> Result<(), Box<dyn Error>> {
    let config = AppConfig::from_env()?;

    tracing_subscriber::fmt()
        .with_max_level(tracing_subscriber::filter::LevelFilter::WARN)
        .with_writer(io::stderr)
        // SDK event fields can contain request IDs or other client data.
        .fmt_fields(tracing_subscriber::fmt::format::debug_fn(|_, _, _| Ok(())))
        .with_file(true)
        .with_line_number(true)
        .with_ansi(false)
        .init();

    serve_stdio(config).await?;
    Ok(())
}

#[cfg(target_os = "linux")]
async fn login() -> Result<(), Box<dyn Error>> {
    let started_at = auth_session::monotonic_now();
    let config = AppConfig::from_env()?;
    let oauth = OAuthClient::new(&config)?;
    let store = TokenStore::new(&config)?;
    let session = auth_session::AuthSession::open(&config)?;
    // Check keyring access before opening the browser.
    drop(store.load()?);
    let pending = oauth.authorize_url().await?;

    // The consent URL goes directly to the OS launcher. Suppress its output
    // because a launcher can include the URL in its own error messages.
    const BROWSER_ERROR: &str =
        "could not open the default browser; run auth login from a Linux desktop session with xdg-open";
    let mut browser = tokio::process::Command::new("xdg-open")
        .arg(pending.url().as_str())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| BROWSER_ERROR)?;

    eprintln!("linkedin-mcp: complete LinkedIn consent in your browser");
    let callback = pending.wait_for_callback();
    tokio::pin!(callback);
    let callback = tokio::select! {
        result = &mut callback => result?,
        result = browser.wait() => {
            if !result.map_err(|_| BROWSER_ERROR)?.success() {
                return Err(BROWSER_ERROR.into());
            }
            callback.await?
        }
    };
    let tokens = oauth.exchange_code(callback).await?;
    session.save(&store, &tokens, started_at)?;
    eprintln!("linkedin-mcp: LinkedIn credential saved in the OS keyring");
    Ok(())
}

#[cfg(target_os = "linux")]
fn logout() -> Result<(), Box<dyn Error>> {
    let config = AppConfig::from_env()?;
    let store = TokenStore::new(&config)?;
    auth_session::AuthSession::open(&config)?.logout(&store)?;
    eprintln!("linkedin-mcp: local LinkedIn credential cleared");
    Ok(())
}

async fn serve_stdio(config: AppConfig) -> Result<(), &'static str> {
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
        handler: LinkedInServer {
            config: Arc::new(config),
        }
        .to_mcp_server_handler(),
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
