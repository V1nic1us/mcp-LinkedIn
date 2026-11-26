# LinkedIn MCP

LinkedIn MCP is a local Linux server that speaks MCP over standard input and
output. The server accepts MCP initialization and advertises an empty tool list.
You can authorize a LinkedIn app separately from the stdio session.

## Build on Linux

Install [Rust and Cargo](https://rustup.rs/) 1.84.0 or newer and your Linux
distribution's native build tools, `pkg-config`, and D-Bus development headers
(`build-essential pkg-config libdbus-1-dev` on Debian or Ubuntu). Build and run
the release binary from the repository root.

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
| `LINKEDIN_REDIRECT_URI` | Exact OAuth callback URI for the LinkedIn app, using `http://127.0.0.1` or `http://[::1]` with an explicit port above 1023 and a simple path. |
| `LINKEDIN_ALLOWED_SCOPES` | Space-separated scope allowlist. Set to `w_member_social` for login. This is the only supported scope. |
| `LINKEDIN_API_VERSION` | LinkedIn API version to use when API calls are added. |

The server loads and validates these settings at startup. All four variables
can be omitted for a stdio session. A blank value or invalid UTF-8 causes a
startup error that names the variable.

## Authorize a LinkedIn app on Linux

1. Create or select an app in the [LinkedIn Developer Portal](https://www.linkedin.com/developers/apps).
   Add **Share on LinkedIn** in the app's **Products** tab to obtain
   `w_member_social`. [Share on LinkedIn setup](https://learn.microsoft.com/en-us/linkedin/consumer/integrations/self-serve/share-on-linkedin)
2. Ask LinkedIn Developer Support or your LinkedIn contact to enable **native
   PKCE OAuth** for the app. Wait for confirmation before attempting consent.
   In the app's **Auth** settings, copy its client ID and register the loopback
   callback URI, for example `http://127.0.0.1:8765/callback`. Use the same
   scheme, IP, port, and path in your environment. Supply the public client ID
   only. The app generates its own per-attempt PKCE verifier. [LinkedIn native OAuth setup](https://learn.microsoft.com/en-us/linkedin/shared/authentication/authorization-code-flow-native)
3. From your Linux desktop session, make sure `xdg-open` and an unlocked
   Secret Service keyring are available. Run the commands below, then review
   and approve the requested permission in your default browser.

```sh
export LINKEDIN_CLIENT_ID=your_app_client_id
export LINKEDIN_REDIRECT_URI=http://127.0.0.1:8765/callback
export LINKEDIN_ALLOWED_SCOPES=w_member_social
./target/release/linkedin-mcp auth login
```

The callback listener opens on the configured loopback address for up to five
minutes. The terminal reports success once the token is saved in the OS
keyring. An unavailable keyring produces an error. Run `auth login` again to
renew an expired credential.

## Disconnect LinkedIn

Run `./target/release/linkedin-mcp auth logout` with the same
`LINKEDIN_CLIENT_ID` to delete the local credential. You can also remove the
`linkedin-mcp` entry for that client ID in your desktop keyring manager.
To revoke the app's LinkedIn access, open **Settings & Privacy → Data privacy →
Permitted Services** in LinkedIn and remove the app. [LinkedIn disconnect steps](https://www.linkedin.com/help/linkedin/answer/a521848/)

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
