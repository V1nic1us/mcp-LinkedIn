# LinkedIn MCP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a local Rust MCP server that helps AI clients prepare profile edits and publish personal LinkedIn text/link posts through the official API after explicit confirmation.

**Architecture:** Start with an MCP stdio binary for Linux. Keep MCP tools, OAuth/token handling, and LinkedIn REST calls in separate modules. Profile assistance remains local text preparation plus an official edit-page link; publishing stays disabled until the LinkedIn app has the official Share on LinkedIn product and `w_member_social` permission.

**Tech Stack:** Rust stable; Tokio async runtime; an MCP Rust SDK compatible with the chosen protocol; Reqwest with Rustls; Serde; OAuth 2.0 Authorization Code with PKCE if supported by LinkedIn for this client flow; OS keyring integration for tokens. Select maintained crate versions during implementation and pin them in `Cargo.toml`/`Cargo.lock`.

**Spec:** `docs/superpowers/specs/2026-11-25-linkedin-mcp-design.md`

## Global Constraints

- Local MCP transport is stdio; initial platform is Linux.
- Use only documented LinkedIn APIs, OAuth and permissions actually approved for the app.
- No scraping, browser automation, UI injection, private endpoints, engagement automation, or bypass of rate limits.
- Never publish without explicit confirmation tied to the exact content being sent.
- Do not edit the LinkedIn profile via API or browser; only prepare suggested text and open the official edit page.
- Keep tokens, OAuth codes, and private content out of logs.
- If a publish attempt has an ambiguous outcome, do not retry automatically.

## Review Focus

- Missing Share on LinkedIn product or `w_member_social`: publish tool reports setup/access unavailable and performs no network write.
- OAuth state mismatch, invalid callback, or denied consent: reject authentication and do not save credentials.
- Confirmation does not match the exact post text: reject publication.
- LinkedIn timeout after request transmission: report an unknown result and do not retry.
- Missing OS keyring or corrupt stored token: fail closed and explain reauthentication; never fall back to plaintext silently.

---

### Task 1: Establish the Rust MCP project and stdio lifecycle

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`
- Create: `src/lib.rs`
- Create: `src/config.rs`
- Create: `README.md`
- Test: `tests/server_protocol.rs`

**Interfaces:**
- Produces: `config::AppConfig::from_env() -> Result<AppConfig, ConfigError>` and an MCP server entry point that initializes stdio, registers tools, and exits cleanly on client disconnect.
- Builder chooses maintained crate versions, pinned in the lockfile, and documents Linux setup and per-client stdio configuration.

- [ ] Define the package, binary, library modules, and config structure. Keep client secrets out of config output and diagnostics.
- [ ] Add a protocol-level test that starts the server, completes MCP initialization, lists tools, and verifies clean shutdown.
- [ ] Run `cargo test --test server_protocol`; expect the new test to fail until initialization and registration are implemented.
- [ ] Implement stdio initialization and a temporary empty tool registry; make protocol errors go to stderr and reserve stdout for MCP frames.
- [ ] Run `cargo test --test server_protocol`; expect initialization, tool listing, and shutdown assertions to pass.
- [ ] Document Rust toolchain, build/run commands, and stdio configuration examples for Codex, Claude, and generic MCP clients; mark other clients as supported only when they accept this transport.

### Task 2: Add safe OAuth and local token storage

**Files:**
- Create: `src/linkedin/oauth.rs`
- Create: `src/linkedin/token_store.rs`
- Create: `src/linkedin/mod.rs`
- Modify: `src/config.rs`
- Modify: `Cargo.toml`
- Test: `tests/oauth_flow.rs`
- Test: `tests/token_store.rs`

**Interfaces:**
- Produces: `OAuthClient::authorize_url()`, `OAuthClient::exchange_code(callback) -> Result<TokenSet, OAuthError>`, `TokenStore::load() -> Result<Option<TokenSet>, TokenStoreError>`, and `TokenStore::save(&TokenSet) -> Result<(), TokenStoreError>`.
- Callback validation must bind to the pending authorization using a cryptographically random `state`; use PKCE if supported by LinkedIn's documented flow.

- [ ] Write tests for valid callback, mismatched state, denied consent, expired token handling, token redaction, and unavailable keyring behavior.
- [ ] Run `cargo test --test oauth_flow --test token_store`; expect failures before implementation.
- [ ] Implement OAuth configuration from environment variables without printing client secrets; validate redirect URL and scopes against the configured allowlist.
- [ ] Implement a loopback callback listener bound only to localhost and validate state before token exchange.
- [ ] Implement OS keyring storage; on unavailable keyring, return an actionable error rather than silently writing plaintext.
- [ ] Run `cargo test --test oauth_flow --test token_store`; expect all callback validation and storage behavior assertions to pass.
- [ ] Document app registration, callback URL setup, consent and local disconnect/token deletion steps.

### Task 3: Implement LinkedIn API client and connection status

**Files:**
- Create: `src/linkedin/client.rs`
- Create: `src/linkedin/models.rs`
- Modify: `src/linkedin/mod.rs`
- Modify: `src/config.rs`
- Test: `tests/linkedin_client.rs`

**Interfaces:**
- Produces: `LinkedInClient::connection_status() -> ConnectionStatus` and `LinkedInClient::create_text_post(text: &str) -> Result<PublishedPost, LinkedInApiError>`.
- `PublishedPost` contains the returned post URN and optional URL; `ConnectionStatus` contains connected state and approved scopes, never token values.

- [ ] Add HTTP-mocked tests for missing credentials, missing `w_member_social`, successful 201 response, 401/403, rate limiting, malformed response, and timeout-after-send handling.
- [ ] Run `cargo test --test linkedin_client`; expect tests to fail before the client exists.
- [ ] Implement a Reqwest client that targets only the documented LinkedIn API host, attaches OAuth bearer token and required version/protocol headers, and uses timeouts.
- [ ] Implement scope gating before any write; unsupported or unapproved access must return an access-required result without sending a POST.
- [ ] Map API response headers and error bodies to safe typed results; redact bearer tokens and avoid retrying writes on timeout or ambiguous network failure.
- [ ] Run `cargo test --test linkedin_client`; expect mocked API and no-write scope checks to pass.

### Task 4: Add profile drafting and status MCP tools

**Files:**
- Create: `src/tools/profile.rs`
- Create: `src/tools/status.rs`
- Create: `src/tools/mod.rs`
- Modify: `src/main.rs`
- Test: `tests/profile_tools.rs`
- Test: `tests/status_tool.rs`

**Interfaces:**
- Produces MCP tool `draft_profile_update(field: String, current_text: Option<String>, proposed_text: String) -> ProfileDraftResult`.
- Produces MCP tool `linkedin_connection_status() -> ConnectionStatus`.
- `ProfileDraftResult` returns the field, proposed text, and official profile edit URL; it does not call LinkedIn APIs or read account data.

- [ ] Test supported field validation, empty/oversized content, result includes official profile URL, and tool has no LinkedIn HTTP side effect.
- [ ] Run `cargo test --test profile_tools --test status_tool`; expect missing tool/validation failures.
- [ ] Implement profile draft tool as local transformation/formatting only; do not claim profile update succeeded.
- [ ] Implement connection status using the token store and approved-scope metadata without exposing credentials.
- [ ] Register both tools with MCP and test schemas and responses through the protocol harness.
- [ ] Run `cargo test --test profile_tools --test status_tool`; expect tool validation and no-network assertions to pass.

### Task 5: Add reviewed post preparation and confirmed publishing

**Files:**
- Create: `src/tools/posts.rs`
- Modify: `src/tools/mod.rs`
- Modify: `src/linkedin/client.rs`
- Test: `tests/post_tools.rs`
- Test: `tests/publish_safety.rs`

**Interfaces:**
- Produces MCP tool `prepare_post(text: String, url: Option<String>) -> PreparedPost`.
- Produces MCP tool `publish_post(text: String, confirmation: String) -> PublishResult`.
- `publish_post` accepts only an exact confirmation string derived from the exact content, for example `PUBLISH:` followed by a SHA-256 digest of the normalized text and optional URL. Server computes the expected confirmation and returns it from `prepare_post`; publishing is rejected if confirmation is absent or mismatched.

- [ ] Test preview normalization, exact confirmation success, missing/mismatched confirmation, no scope/no credentials, successful post, and ambiguous timeout with no retry.
- [ ] Run `cargo test --test post_tools --test publish_safety`; expect tests to fail before implementation.
- [ ] Implement deterministic preview and confirmation digest over the exact serialized content that the API client will submit.
- [ ] Implement the publish tool so the only write path invokes `LinkedInClient::create_text_post` after validating confirmation and scope.
- [ ] Ensure the MCP client receives a clear access/setup error when the official product or scope is missing; never substitute browser automation.
- [ ] Run `cargo test --test post_tools --test publish_safety`; expect all confirmation binding and no-write cases to pass.

### Task 6: Finish operations documentation and end-to-end verification

**Files:**
- Modify: `README.md`
- Create: `docs/linkedin-app-setup.md`
- Create: `docs/client-configuration.md`
- Test: `tests/end_to_end_mock.rs`

**Interfaces:**
- Documents required LinkedIn app products, OAuth redirect setup, local environment, client configuration, scope requirements, disconnection, and known unsupported profile API editing.
- End-to-end test uses a mocked LinkedIn API; no real account or credentials are used in automated verification.

- [ ] Write an end-to-end mocked test for initialize → status → prepare post → reject altered content → publish confirmed text, asserting exactly one mocked API write.
- [ ] Run `cargo test`; expect all unit, integration, and protocol tests to pass.
- [ ] Run `cargo fmt --check` and `cargo clippy --all-targets --all-features -- -D warnings`; fix formatting and lint findings.
- [ ] Review docs and error paths for accidental token/content logging; add a regression assertion that captured logs do not contain the fixture token.
- [ ] Document that actual publishing remains unavailable until the app has the official product and granted scope; describe a manual account-level smoke check the user can perform after configuring credentials.
