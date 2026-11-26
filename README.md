# LinkedIn MCP

LinkedIn MCP is a local Linux server that speaks MCP over standard input and
output. The initial server accepts MCP initialization and advertises an empty
tool list. LinkedIn credentials are unnecessary for this stage.

## Build on Linux

Install [Rust and Cargo](https://rustup.rs/) 1.84.0 or newer and your Linux
distribution's native build tools (for example, `build-essential` on Debian or
Ubuntu). From the repository root, build and run the release binary:

```sh
cargo build --locked --release
./target/release/linkedin-mcp
```

The process waits for MCP messages on stdin and exits when the client closes
the input stream. Stdout carries MCP messages and stderr carries diagnostics.

## Optional environment variables

| Variable | Value |
| --- | --- |
| `LINKEDIN_CLIENT_ID` | LinkedIn application client ID. |
| `LINKEDIN_REDIRECT_URI` | OAuth redirect URI registered with the LinkedIn app. |
| `LINKEDIN_ALLOWED_SCOPES` | Space-separated scope allowlist. Defaults to an empty list. |
| `LINKEDIN_API_VERSION` | LinkedIn API version to use when API calls are added. |

The server loads these settings at startup. Omit an unused variable
entirely. A blank value or invalid UTF-8 causes a startup error that names the
variable. OAuth and LinkedIn API calls are planned for later tasks.

## Configure each MCP client

Replace `/absolute/path/to/linkedin-mcp` with the full path to the built binary,
such as `/home/you/src/linkedin-mcp/target/release/linkedin-mcp`.

### Codex

Add the server with the [Codex MCP CLI](https://developers.openai.com/codex/mcp/):

```sh
codex mcp add linkedin -- /absolute/path/to/linkedin-mcp
```

The equivalent entry in `~/.codex/config.toml` is:

```toml
[mcp_servers.linkedin]
command = "/absolute/path/to/linkedin-mcp"
args = []
```

### Claude Code

Register the executable as a [local stdio server](https://code.claude.com/docs/en/mcp):

```sh
claude mcp add --transport stdio linkedin -- /absolute/path/to/linkedin-mcp
```

### Claude Desktop and other stdio clients

For clients that accept a local `mcpServers` configuration, including editions of
[Claude Desktop](https://support.claude.com/en/articles/10949351-getting-started-with-local-mcp-servers-on-claude-desktop)
that accept manual entries in `claude_desktop_config.json`, add this entry to
the client's configuration and restart the client:

```json
{
  "mcpServers": {
    "linkedin": {
      "command": "/absolute/path/to/linkedin-mcp",
      "args": []
    }
  }
}
```

Clients with a different configuration format can use the same executable path
and empty argument list. Compatibility requires local stdio transport and support
for the MCP initialization handshake through protocol version `2025-11-25`.
