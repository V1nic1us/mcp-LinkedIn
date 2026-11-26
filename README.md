# LinkedIn MCP

LinkedIn MCP is a local Linux server that speaks MCP over standard input and
output. It lists tools for local profile drafting, connection status, post
preparation, and confirmed public text publishing. Authorize the LinkedIn app
separately from the stdio session.

## Build on Linux

Step-by-step guides: [LinkedIn app setup](docs/linkedin-app-setup.md) and
[MCP client configuration](docs/client-configuration.md).

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
| `LINKEDIN_REDIRECT_URI` | OAuth callback template with a literal loopback IP and a simple path, for example `http://127.0.0.1/callback` or `http://[::1]/callback`. Omit the port. |
| `LINKEDIN_ALLOWED_SCOPES` | Set to `openid w_member_social` for login and publishing. These are the two supported scopes. |
| `LINKEDIN_API_VERSION` | Optional legacy setting. The consumer Share on LinkedIn endpoint uses Rest.li protocol version `2.0.0` directly. |

The server loads and validates these settings at startup. All four variables
can be omitted for a stdio session. A blank value or invalid UTF-8 causes a
startup error that names the variable.

## Authorize a LinkedIn app on Linux

1. Create or select an app in the [LinkedIn Developer Portal](https://www.linkedin.com/developers/apps).
   In the app's **Products** tab, add **Share on LinkedIn** for
   `w_member_social` and **Sign In with LinkedIn using OpenID Connect** for
   `openid`. The API client uses the member's OpenID Connect subject to identify
   the author of a personal post. [Share on LinkedIn setup](https://learn.microsoft.com/en-us/linkedin/consumer/integrations/self-serve/share-on-linkedin)
   [OpenID Connect setup](https://learn.microsoft.com/en-us/linkedin/consumer/integrations/self-serve/sign-in-with-linkedin-v2)
2. Ask LinkedIn Developer Support or your LinkedIn contact to enable **native
   PKCE OAuth** for the app. Wait for confirmation before attempting consent.
   Copy the public client ID from the app's **Auth** settings. Confirm with
   LinkedIn how to register the callback's loopback IP and path for native
   PKCE if your app requires redirect registration. Use the same IP and path
   in your environment. The app adds an OS-assigned ephemeral port to the URI
   for each consent attempt and generates its own PKCE verifier. [LinkedIn native OAuth setup](https://learn.microsoft.com/en-us/linkedin/shared/authentication/authorization-code-flow-native)
3. From your Linux desktop session, make sure `xdg-open`, an unlocked
   Secret Service keyring, and your private `XDG_RUNTIME_DIR` are available.
   Run the commands below, then review and approve the requested permissions
   in your default browser.

```sh
export LINKEDIN_CLIENT_ID=your_app_client_id
export LINKEDIN_REDIRECT_URI=http://127.0.0.1/callback
export LINKEDIN_ALLOWED_SCOPES='openid w_member_social'
./target/release/linkedin-mcp auth login
```

The callback listener opens on the configured loopback address for up to five
minutes. The token exchange must finish within that window. The terminal
reports success once the token is saved in the OS keyring. An unavailable
keyring produces an error. Run `auth login` again to renew an expired
credential or upgrade a credential granted before `openid` was requested.

The API client uses `GET /v2/userinfo` to resolve the member and
`POST /v2/ugcPosts` to publish public text posts. Both endpoints use
`https://api.linkedin.com`. [OpenID Connect](https://learn.microsoft.com/en-us/linkedin/consumer/integrations/self-serve/sign-in-with-linkedin-v2)
[Share on LinkedIn](https://learn.microsoft.com/en-us/linkedin/consumer/integrations/self-serve/share-on-linkedin)

The authorization exchange requires a valid nonce-bound OpenID Connect ID
token signed by LinkedIn. The documented native PKCE response lists an access
token and expiry, but does not specify whether it issues an ID token. If the
enabled native flow omits `id_token`, login fails closed and publishing remains
unavailable until LinkedIn documents or enables OIDC ID tokens for that flow.

## Approve every publish call

Configure the MCP host to show the exact public post preview and require human
approval for every call to `publish_post`. The deterministic confirmation digest
binds the preview text, including any appended URL. The server cannot verify
that a human approved a call. Use only hosts that enforce per-call approval for
publishing; disable `publish_post` in hosts without that setting.

The server consumes a matching confirmation before sending the POST. Identical
content for the same app can be attempted once per server process. The registry
is held in memory, so a restart or a separate server process has its own
registry. After an uncertain result, check your LinkedIn activity before taking
any further action. Keep automatic publish retries disabled in the host.

## Disconnect LinkedIn

Run `./target/release/linkedin-mcp auth logout` with the same
`LINKEDIN_CLIENT_ID` and `XDG_RUNTIME_DIR` to delete the local credential and
prevent pending logins for that app from saving a credential. The runtime
directory holds a checksummed per-app logout record. Logout also sets the legacy
coordination file to one byte. This file contains no credentials. Its length is
a compatibility fence that makes older CLIs using that file reject a token save,
even if consent was already underway. Use the current CLI for subsequent logins.
If logout is interrupted, run it again with the current CLI to finish deletion.

To revoke the app's LinkedIn access, open **Settings & Privacy → Data privacy →
Permitted Services** in LinkedIn and remove the app. [LinkedIn disconnect steps](https://www.linkedin.com/help/linkedin/answer/a521848/)

## Configure each MCP client

Use the [client configuration guide](docs/client-configuration.md) for Codex,
Claude Code, Grok CLI, Google Antigravity, and other local stdio clients. It
shows the required environment settings and per-call publishing approval rule.
