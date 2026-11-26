# MCP client configuration

Build on Linux with Rust 1.84 or newer:

```sh
cargo build --locked --release
```

Register the absolute path to `target/release/linkedin-mcp` as a local **stdio** MCP server. Do not configure OAuth tokens in the MCP client; use `auth login` separately so the credential stays in the OS keyring.

The server reads `LINKEDIN_CLIENT_ID` and `LINKEDIN_ALLOWED_SCOPES` when the MCP host starts it. Pass those variables to the host configuration even if you exported them in a different login shell. Also pass the same private `XDG_RUNTIME_DIR` used by `auth login` and `auth logout`; it must belong to the current user and have mode `0700`. Example values below are placeholders; use your desktop session's actual runtime directory.

## Codex

```sh
codex mcp add linkedin \
  --env LINKEDIN_CLIENT_ID=your-app-client-id \
  --env LINKEDIN_ALLOWED_SCOPES='openid w_member_social' \
  --env XDG_RUNTIME_DIR=/run/user/1000 \
  -- /absolute/path/to/linkedin-mcp
```

The CLI command registers the server and environment. After running it, add the per-tool approval setting below to `~/.codex/config.toml` so `publish_post` always prompts.

Or add to `~/.codex/config.toml`:

```toml
[mcp_servers.linkedin]
command = "/absolute/path/to/linkedin-mcp"
args = []
default_tools_approval_mode = "prompt"

[mcp_servers.linkedin.env]
LINKEDIN_CLIENT_ID = "your-app-client-id"
LINKEDIN_ALLOWED_SCOPES = "openid w_member_social"
XDG_RUNTIME_DIR = "/run/user/1000"

[mcp_servers.linkedin.tools.publish_post]
approval_mode = "prompt"
```

Codex supports per-tool approval settings. See [Codex MCP configuration](https://developers.openai.com/codex/mcp).

## Claude Code

```sh
claude mcp add \
  --env LINKEDIN_CLIENT_ID=your-app-client-id \
  --env LINKEDIN_ALLOWED_SCOPES='openid w_member_social' \
  --env XDG_RUNTIME_DIR=/run/user/1000 \
  --transport stdio linkedin -- /absolute/path/to/linkedin-mcp
```

## Grok CLI

The local Grok CLI accepts stdio servers:

```sh
grok mcp add linkedin -- /absolute/path/to/linkedin-mcp
grok mcp list
```

Set the server environment in `~/.grok/config.toml`:

```toml
[mcp_servers.linkedin]
command = "/absolute/path/to/linkedin-mcp"
args = []
env = { LINKEDIN_CLIENT_ID = "your-app-client-id", LINKEDIN_ALLOWED_SCOPES = "openid w_member_social", XDG_RUNTIME_DIR = "/run/user/1000" }
```

The Grok web custom connector requires a publicly reachable remote MCP endpoint, while this project currently implements local stdio. A tunnel or remote transport requires a separate deployment design. See [Grok MCP server documentation](https://docs.x.ai/build/features/mcp-servers).

## Google Antigravity

Add this server to Antigravity's `mcp_config.json` under `mcpServers` (global or workspace configuration):

```json
{
  "mcpServers": {
    "linkedin": {
      "command": "/absolute/path/to/linkedin-mcp",
      "args": [],
      "env": {
        "LINKEDIN_CLIENT_ID": "your-app-client-id",
        "LINKEDIN_ALLOWED_SCOPES": "openid w_member_social",
        "XDG_RUNTIME_DIR": "/run/user/1000"
      }
    }
  }
}
```

Antigravity CLI uses `~/.gemini/config/mcp_config.json` globally or `<workspace>/.agents/mcp_config.json` for a workspace. The Antigravity app and IDE provide an MCP server manager. See [Google Antigravity MCP documentation](https://antigravity.google/docs/mcp).

## Other stdio clients

For clients that use a `mcpServers` JSON object, including compatible Claude Desktop versions:

```json
{
  "mcpServers": {
    "linkedin": {
      "command": "/absolute/path/to/linkedin-mcp",
      "args": [],
      "env": {
        "LINKEDIN_CLIENT_ID": "your-app-client-id",
        "LINKEDIN_ALLOWED_SCOPES": "openid w_member_social",
        "XDG_RUNTIME_DIR": "/run/user/1000"
      }
    }
  }
}
```

Other local-agent clients can use the same executable and environment if they support stdio MCP protocol version `2025-11-25`. Before use, configure a visible human approval for every `publish_post` call and disable automatic retries. If the host cannot enforce per-call approval, disable `publish_post`. The content digest binds exact text, but cannot prove that a human approved it. Publishing is marked destructive and non-idempotent in the tool metadata.

The server provides `draft_profile_update`, `linkedin_connection_status`, `prepare_post`, and `publish_post`. Profile drafting never edits LinkedIn. See [LinkedIn app setup](linkedin-app-setup.md) for product, scope, OAuth, and login requirements.
