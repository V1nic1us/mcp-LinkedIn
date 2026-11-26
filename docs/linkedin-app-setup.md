# LinkedIn app setup

This MCP uses LinkedIn's official member APIs for public text posts. It does not automate the LinkedIn website.

## Products and permissions

In the LinkedIn Developer Portal, create/select an app and request both products:

- **Share on LinkedIn**, which grants `w_member_social` for posting.
- **Sign In with LinkedIn using OpenID Connect**, which grants `openid` for identifying the signed-in member.

Set `LINKEDIN_ALLOWED_SCOPES='openid w_member_social'`. The app fails closed unless both scopes are configured and granted. Older credentials need a fresh `auth login`.

## Native PKCE and redirect

LinkedIn Developer Support must enable native PKCE OAuth for the app. Set `LINKEDIN_CLIENT_ID` to the app's public client ID and `LINKEDIN_REDIRECT_URI` to a loopback URI with a literal address and path, without a port, such as `http://127.0.0.1/callback`. Confirm redirect registration requirements with LinkedIn for your app. The program binds an ephemeral loopback port and uses PKCE with a browser consent flow.

The current LinkedIn native PKCE documentation does not say whether its token response includes the OpenID Connect ID token this implementation requires. Login rejects a missing ID token. Confirm that LinkedIn enables ID tokens for this flow before relying on publishing.

## Linux prerequisites and login

Use a Linux desktop session with `xdg-open`, an unlocked Secret Service keyring, and a private `XDG_RUNTIME_DIR` owned by the current user. Build the release binary and run:

```sh
export LINKEDIN_CLIENT_ID='your-app-client-id'
export LINKEDIN_REDIRECT_URI='http://127.0.0.1/callback'
export LINKEDIN_ALLOWED_SCOPES='openid w_member_social'
./target/release/linkedin-mcp auth login
```

Approve consent in the browser. Access tokens are stored in the OS keyring, not in the MCP client configuration. To disconnect locally, run `./target/release/linkedin-mcp auth logout` with the same client ID and runtime directory. To revoke LinkedIn-side access, remove the app under LinkedIn **Settings & Privacy → Data privacy → Permitted Services**.

## Manual smoke check

After login, start the MCP server from a supported host and call `linkedin_connection_status`. Confirm it reports connected with both scopes. Call `prepare_post` with a harmless draft and inspect its returned text. Configure the host to require per-call human approval for `publish_post`; approve only after inspecting the exact preview. Verify the post in LinkedIn activity. If the outcome is uncertain, check activity before taking any further action; do not retry automatically. The confirmation replay registry is in memory and resets when the process restarts.

Profile changes are drafts only. `draft_profile_update` returns an official edit link; apply the change manually in LinkedIn. No profile write API is implemented.
