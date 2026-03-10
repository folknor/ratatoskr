# Rust Crate Research: Email Provider Backend

**Date**: March 2026
**Context**: Moving all email providers (Gmail API, JMAP, Microsoft Graph) into the Rust backend alongside IMAP. This document evaluates the Rust crate ecosystem for each provider and supporting infrastructure.

---

## Strategic Plan

### Current state

Ratatoskr has four email provider paths:

| Provider | Protocol | Implementation | Status |
|----------|----------|---------------|--------|
| **IMAP/SMTP** | TCP/TLS custom protocol | **Rust** (`src-tauri/src/imap/`, `smtp/`) | Production |
| **Gmail API** | REST/JSON over HTTPS | **TypeScript** (`src/services/gmail/`) | Production, moving to Rust |
| **JMAP** | JSON-over-HTTP (RFC 8620/8621) | **Not yet implemented** | Planned — reference impl in `docs/jmap.md` |
| **Microsoft Graph** | REST/JSON over HTTPS | **Not yet implemented** | Planned — research in `docs/microsoft-exchange-assessment.md` |

The problem: Gmail lives in TypeScript, which means token refresh, HTTP calls, message parsing, sync logic, and DB writes all cross the IPC boundary repeatedly. JMAP and Graph would add two more TS provider implementations with the same overhead. Meanwhile IMAP already proves the Rust-native pattern works — direct DB access, direct body store writes, direct search indexing, no serialization per message.

### Target state

All four providers implemented in Rust as Tauri commands. The TypeScript layer is reduced to:

- **UI**: components, stores, rendering
- **Orchestration**: 60s sync timer, post-sync hooks (filters, smart labels, notifications, AI categorization)
- **Offline queue**: optimistic UI updates, operation queueing, dispatch to Rust commands
- **OAuth flow initiation**: browser interaction for initial token acquisition

Everything below the Tauri command boundary — HTTP calls, token management, message parsing, sync logic, DB writes, body compression, search indexing — lives in Rust.

### Execution order

1. **Gmail API → Rust** (first, see `docs/gmail-rust-migration.md`): Establishes patterns for token management, reqwest+retry HTTP client, message parsing, sync-with-direct-DB-writes, Tauri event progress reporting. Most-used provider, known-good TS reference to port from.

2. **JMAP → Rust** (second): Uses `jmap-client` crate (Stalwart). HTTP-based like Gmail, so it reuses the same token management and retry infrastructure. JMAP is simpler than Gmail in some ways (native `threadId`, state-string delta sync, no History API quirks) but adds mailbox↔label mapping and `EmailSubmission` for sending.

3. **Microsoft Graph → Rust** (third): Same HTTP/JSON pattern as Gmail. Adds Microsoft OAuth2 endpoints (Entra ID, `/common/oauth2/v2.0/`), OData query parameters, per-folder delta tokens, and folder-centric (not label-centric) semantics. Reuses token refresh infrastructure with Microsoft-specific endpoint configuration.

4. **IMAP stays as-is**: Already in Rust. May benefit from shared infrastructure built during the above (e.g., `mail-builder` for message construction), but requires no migration.

### What we do NOT do prematurely

- **No shared `EmailProvider` trait until two providers exist in Rust.** Gmail is label-centric, Graph is folder-centric, JMAP uses mailboxes + `jmap-client`'s API. Forcing a common trait before seeing real implementations will produce a leaky abstraction. Build Gmail-specific Rust services first, extract shared traits after Gmail + one more provider are working.
- **No provider-agnostic Tauri commands until the trait exists.** Commands are `gmail_*`, `jmap_*`, `graph_*` prefixed. Provider routing stays in TS until Rust has a trait to route against.

### Crate strategy

For each provider, the approach depends on ecosystem maturity:

| Provider | Approach | Rationale |
|----------|----------|-----------|
| **JMAP** | Use `jmap-client` crate | Only viable Rust JMAP client. Full RFC 8620/8621 coverage. Same Stalwart ecosystem as `mail-parser`. |
| **Gmail API** | Hand-roll on `reqwest` | ~17 REST endpoints. The auto-generated `google-gmail1` crate is in maintenance mode and uses hyper (not reqwest). Not worth the dependency. |
| **Microsoft Graph** | Hand-roll on `reqwest` | ~18 REST endpoints. `graph-rs-sdk` covers the entire Graph API (not just Mail), has single-maintainer risk, and brings its own OAuth layer. Too heavy. |
| **OAuth2** | Keep existing hand-rolled `oauth.rs` | Already handles PKCE, localhost redirect, token exchange/refresh, CSRF validation. Extend with Microsoft endpoints. |

The rest of this document evaluates each crate option in detail.

---

## Table of Contents

- [Current Rust Dependencies](#current-rust-dependencies)
- [JMAP Provider](#jmap-provider)
- [Gmail API Provider](#gmail-api-provider)
- [Microsoft Graph Provider](#microsoft-graph-provider)
- [OAuth2](#oauth2)
- [Email Utilities](#email-utilities)
- [Recommended Cargo.toml Additions](#recommended-cargotoml-additions)
- [Architecture Decisions](#architecture-decisions)

---

## Current Rust Dependencies

Already in `Cargo.toml`: `async-imap` 0.11, `lettre` 0.11, `mail-parser` 0.11, `base64` 0.22, `reqwest` 0.13, `serde`/`serde_json` 1.0, `zstd` 0.13, `tantivy` 0.25, `tokio`, `rusqlite` 0.32, `chrono` 0.4.

---

## JMAP Provider

### `jmap-client` (Stalwart Labs) — THE CHOICE

| Field | Value |
|-------|-------|
| **Version** | 0.4.0 |
| **GitHub** | [stalwartlabs/jmap-client](https://github.com/stalwartlabs/jmap-client) |
| **Stars** | 103 |
| **License** | Apache-2.0 OR MIT |
| **Downloads** | ~1,361/month |
| **Last commit** | 2025-10-19 (v0.4.0 release) |
| **Maintainer** | Mauro D. (mdecimus) / Stalwart Labs |

**RFC compliance**: RFC 8620 (Core), RFC 8621 (Mail), RFC 8887 (WebSocket), Draft-SIEVE-14.

**Cargo features**: `async` (default, uses reqwest/stream), `websockets` (tokio + tokio-tungstenite), `blocking` (reqwest/blocking via maybe-async). Tokio only pulled in for websockets feature.

**Key dependencies**: `reqwest` 0.12 (with rustls-tls), `serde`/`serde_json`, `chrono` 0.4, `ahash` 0.8, `parking_lot` 0.12, `base64` 0.13, `maybe-async` 0.2.

**API surface**:

| Category | Methods |
|----------|---------|
| **Session** | `connect()`, `getSession()`, `Credentials::basic()`/`bearer()` |
| **Email** | `email_get()`, `email_query()`, `email_set()`, `email_changes()`, `email_query_changes()`, `email_import()`, `email_parse()`, `email_copy()` |
| **Mailbox** | `mailbox_create()`, `mailbox_get()`, `mailbox_query()`, `mailbox_rename()`, `mailbox_move()`, `mailbox_destroy()`, `mailbox_subscribe()`, `mailbox_update_role()`, `mailbox_changes()` |
| **EmailSubmission** | `email_submission_create()`, `email_submission_create_envelope()`, `email_submission_get()`, `email_submission_query()`, `email_submission_changes()`, `email_submission_destroy()` |
| **Blob** | `upload()`, `download()` |
| **Thread** | `thread_get()` |
| **Identity** | `identity_create()`, `identity_get()`, `identity_destroy()`, `identity_changes()` |
| **Push** | WebSocket push, EventSource streaming, PushSubscription management |
| **Batch** | `client.build()` for multi-method JMAP requests |
| **Search** | `search_snippet_get()` |
| **Sieve** | Full CRUD + activate/deactivate/validate |

**Known issues**:
- **Issue #18** (Feb 2026): `Email/set` uses `false` instead of `null` to remove `mailboxIds`/`keywords` patch entries — violates the JMAP spec. **Affects email move operations.** Will likely need a local patch.
- Issue #10: Blob support incomplete
- Issue #4: wasm32 compilation issues
- Issue #2: Documentation incomplete
- No JMAP Calendars or Contacts (Issue #3)

**Commit cadence concern**: ~2 year gap between v0.3.2 (Dec 2023) and v0.4.0 (Oct 2025). However, maintainer is extremely active on Stalwart Mail Server (11.9K stars, updated daily).

**Tauri v2 fit**: Excellent. reqwest → tokio under the hood (same as Tauri v2). rustls-based (no openssl). Same ecosystem as `mail-parser` we already use.

**Verdict**: Use it. Only viable Rust JMAP client. Vendor if Issue #18 or any other bug blocks us.

### Other JMAP crates (not viable)

| Crate | Version | Assessment |
|-------|---------|-----------|
| `jmap-tools` (Stalwart) | 0.1.4 | Object parser with JSON Pointer querying/patching. Companion utility, not a client. |
| `jmap` (Rob Norris) | 0.0.5 | Abandoned (2016). Pre-RFC. Uses `rustc-serialize`. Dead. |
| `libjmap` (WhyNotHugo) | 0.1.1 | Calendars/Contacts only, no email. Panics on errors. Prototype quality. |
| `rusmes-jmap` | 0.1.0 | Server implementation, not a client. |
| `melib` | 0.8.13 | Has JMAP backend but GPLv3 — license incompatible. 67K SLoC, too heavy. |
| Stalwart internal crates | — | `crates/jmap`, `crates/jmap-proto` in Stalwart monorepo. Server-side, not published to crates.io, tightly coupled. Reference only. |

---

## Gmail API Provider

### Approach: Hand-roll on reqwest

**Rationale**: Our existing TypeScript `GmailClient` maps 1:1 to Gmail REST endpoints (~15 API calls). Porting these to reqwest-based Rust functions is straightforward and avoids depending on maintenance-mode code generators.

### `google-gmail1` (Byron/google-apis-rs) — EVALUATED, NOT RECOMMENDED

| Field | Value |
|-------|-------|
| **Version** | 7.0.0+20251215 |
| **GitHub** | [Byron/google-apis-rs](https://github.com/Byron/google-apis-rs) |
| **Stars** | ~1,100 |
| **License** | MIT |
| **Downloads** | ~160K total, ~17K/month |
| **Last commit** | 2026-01-01 |

Full Gmail v1 coverage (messages, threads, labels, history, drafts, attachments, settings). Uses hyper 1 + tokio 1 + yup-oauth2. Auto-generated from Google API discovery docs.

**Why not**: Project is in **maintenance mode** — maintainers seeking successor. Auto-generated verbose builder-pattern API. Uses hyper directly (not reqwest), so we can't share HTTP client. Pulls in `yup-oauth2` which overlaps with our existing OAuth. Large dependency footprint for ~15 API calls we can write ourselves.

### Other Gmail crates (not viable)

| Crate | Assessment |
|-------|-----------|
| `gmail` (libninjacom) v0.18.0 | OpenAPI-generated. 7 stars, 330 monthly downloads. Not serious. |
| `rust-gmail` v0.2.1 | Send-only via service accounts. Last updated 2023. |
| `google-api-proto` | Protobuf/gRPC only. Gmail API is REST — not included. |
| Google Cloud Rust SDK | Covers 140+ Cloud services. Gmail/Workspace APIs not included. |

### Gmail API endpoints to implement

These are the REST calls our TS `GmailClient` makes, to be ported to Rust with `reqwest`:

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/gmail/v1/users/me/messages` | GET | List messages (with `q` filter, pagination) |
| `/gmail/v1/users/me/messages/{id}` | GET | Get message (format=full/metadata/raw) |
| `/gmail/v1/users/me/messages/send` | POST | Send message (raw RFC 822 base64url) |
| `/gmail/v1/users/me/messages/{id}/modify` | POST | Modify labels (add/remove) |
| `/gmail/v1/users/me/messages/{id}/trash` | POST | Trash message |
| `/gmail/v1/users/me/messages/{id}/untrash` | POST | Untrash message |
| `/gmail/v1/users/me/messages/batchModify` | POST | Batch modify labels |
| `/gmail/v1/users/me/messages/batchDelete` | POST | Batch delete |
| `/gmail/v1/users/me/threads` | GET | List threads |
| `/gmail/v1/users/me/threads/{id}` | GET | Get thread |
| `/gmail/v1/users/me/labels` | GET | List labels |
| `/gmail/v1/users/me/labels` | POST | Create label |
| `/gmail/v1/users/me/history` | GET | Delta sync (History API) |
| `/gmail/v1/users/me/drafts` | POST/PUT/DELETE/GET | Draft CRUD |
| `/gmail/v1/users/me/messages/{id}/attachments/{id}` | GET | Download attachment |
| `/gmail/v1/users/me/settings/sendAs` | GET | Send-as aliases |
| `/gmail/v1/users/me/profile` | GET | User profile |

~17 endpoints. Each is a simple reqwest call with JSON ser/de. Token refresh wraps all calls.

---

## Microsoft Graph Provider

### Approach: Hand-roll on reqwest

**Rationale**: Same as Gmail — the Graph Mail API is straightforward REST/JSON. We need ~15-20 endpoints. The only full Rust SDK (`graph-rs-sdk`) is generated from the entire Graph API surface (not just Mail), bringing massive dependency bloat.

### `graph-rs-sdk` — EVALUATED, NOT RECOMMENDED FOR NOW

| Field | Value |
|-------|-------|
| **Version** | 3.0.1 |
| **GitHub** | [sreeise/graph-rs-sdk](https://github.com/sreeise/graph-rs-sdk) |
| **Stars** | ~145 |
| **License** | MIT |
| **Downloads** | ~77K total |
| **Last commit** | August 2025 |
| **Maintainer** | Sean Reeise (single maintainer) |

Generated from Microsoft's OpenAPI specs. Full Graph v1.0 and Beta coverage: mail, OneDrive, Teams, Calendar, Users, Groups. OAuth2/MSAL built-in (PKCE, client credentials, device code, WebView via wry). Async + blocking. Returns reqwest::Response under the hood.

**Why not**: Massive generated codebase covering all of Graph API — we need only mail. Single maintainer, last commit ~8 months ago. Brings its own OAuth and HTTP layers that overlap with ours. In-memory-only token cache. For a focused email client, the dependency size trade-off isn't worth it.

### `msgraph-rs` — NOT VIABLE

3 stars, GPL-3.0 license (incompatible). Much less complete. Skip.

### Graph Mail API endpoints to implement

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/me/messages` | GET | List messages (`$select`, `$filter`, `$orderby`, `$top`) |
| `/me/messages/{id}` | GET | Get message |
| `/me/sendMail` | POST | Send message |
| `/me/messages/{id}` | PATCH | Update message (read status, categories) |
| `/me/messages/{id}/move` | POST | Move to folder |
| `/me/messages/{id}/copy` | POST | Copy to folder |
| `/me/messages/{id}` | DELETE | Delete message |
| `/me/mailFolders` | GET | List folders |
| `/me/mailFolders` | POST | Create folder |
| `/me/mailFolders/{id}` | PATCH/DELETE | Update/delete folder |
| `/me/mailFolders/{id}/messages/delta` | GET | Delta sync (per-folder) |
| `/me/messages/{id}/attachments` | GET | List attachments |
| `/me/messages/{id}/attachments/{id}` | GET | Download attachment |
| `/me/messages/{id}/createReply` | POST | Create reply draft |
| `/me/messages/{id}/createForward` | POST | Create forward draft |
| `/me/messages` | POST | Create draft |
| `/me/messages/{id}/send` | POST | Send draft |
| `/me/inferenceClassification/overrides` | GET | Focused Inbox overrides |

~18 endpoints. Standard REST/JSON with OData query parameters. Delta tokens don't expire (unlike Gmail's History API).

**OData tips**: `@odata.nextLink` / `@odata.deltaLink` pagination handled with `#[serde(rename = "@odata.nextLink")]`. Generic `ODataCollection<T>` wrapper struct for list responses.

---

## OAuth2

### Current state: Hand-rolled, works well

Our existing implementation in `src-tauri/src/oauth.rs` already handles:
- Localhost redirect server with port fallback (ports 17248-17251)
- PKCE (challenge/verifier generation)
- Token exchange
- Token refresh
- CSRF state validation
- Success HTML page served to user

And in `src-tauri/src/imap/connection.rs`:
- XOAUTH2 SASL token construction (`user={email}\x01auth=Bearer {token}\x01\x01`)

### `oauth2` crate (ramosbugs) — OPTIONAL ENHANCEMENT

| Field | Value |
|-------|-------|
| **Version** | 5.0.0 |
| **GitHub** | [ramosbugs/oauth2-rs](https://github.com/ramosbugs/oauth2-rs) |
| **Stars** | ~1,200 |
| **License** | MIT OR Apache-2.0 |
| **Downloads** | 26.8M total, 5.6M/month |

Provider-agnostic OAuth2 client. First-class PKCE via `PkceCodeChallenge::new_random_sha256()`. Works with Google, Microsoft, or any custom provider — just set endpoints. BYO HTTP client (reqwest compatible). Manual token refresh (you call `exchange_refresh_token()` yourself).

**Does NOT provide**: Localhost redirect server, XOAUTH2 SASL, automatic refresh/caching.

**Verdict**: Would add type-safety for auth URL construction and PKCE, but doesn't solve any hard problem we haven't already solved. Consider adopting if we refactor OAuth to be more generic across Google/Microsoft/generic providers. Not urgent.

### Other OAuth crates evaluated

| Crate | Assessment |
|-------|-----------|
| `yup-oauth2` v12.1.2 | Google-focused. Built-in localhost server and auto-refresh. But heavily opinionated (file-based token storage, pulls in Hyper as HTTP server). Our hand-rolled solution is more flexible. **Skip.** |
| `openidconnect` v4.0.1 | OIDC discovery + ID token validation. Overkill — we need access tokens, not ID tokens. Only useful if we want Microsoft's OIDC auto-discovery. **Skip.** |
| `graph-rs-sdk` OAuth | Microsoft-only, part of the full Graph SDK. In-memory token cache only. **Skip** (unless we adopt the full SDK). |
| `azure_identity` v0.33.0 | Server-to-server Azure auth only. No desktop OAuth2 flows (no auth code + PKCE). **Not applicable.** |
| `oxide-auth` v0.6.1 | OAuth2 **server** implementation. Wrong side of the protocol. **Not applicable.** |
| `rsasl` v2.2.1 | SASL framework. XOAUTH2 was in v1 but **missing in v2**. Our 5-line hand-rolled XOAUTH2 is sufficient. **Skip.** |
| `tauri-plugin-oauth` v2.0.0 | Localhost redirect server for Tauri. Does what our `oauth.rs` already does. Lateral move. **Skip.** |
| `tauri-plugin-google-auth` v0.5.1 | Google-specific. 5.9K downloads. Requires client secret (we use PKCE without secret). **Skip.** |
| `clio-auth` v0.8.0 | Supplements `oauth2` with localhost server. 49 recent downloads. Nearly dead. **Skip.** |

### OAuth architecture for multi-provider

For Google + Microsoft + generic providers, the flow is identical:
1. Build auth URL with PKCE challenge + scopes + state
2. Open in default browser
3. Localhost server captures redirect with auth code
4. Exchange code + PKCE verifier for tokens
5. Store tokens in SQLite (encrypted)
6. Refresh before expiry (5 min buffer)

Only the endpoints and scopes differ:

| Provider | Auth endpoint | Token endpoint | Scopes |
|----------|--------------|----------------|--------|
| Google | `accounts.google.com/o/oauth2/v2/auth` | `oauth2.googleapis.com/token` | `gmail.modify`, `gmail.send`, `gmail.readonly` |
| Microsoft | `login.microsoftonline.com/common/oauth2/v2.0/authorize` | `login.microsoftonline.com/common/oauth2/v2.0/token` | `Mail.ReadWrite`, `Mail.Send`, `MailboxSettings.ReadWrite`, `offline_access` |

---

## Email Utilities

### `mail-builder` (Stalwart Labs) — RECOMMENDED NEW ADDITION

| Field | Value |
|-------|-------|
| **Version** | 0.4.4 |
| **GitHub** | [stalwartlabs/mail-builder](https://github.com/stalwartlabs/mail-builder) |
| **Stars** | ~42 |
| **License** | Apache-2.0 OR MIT |
| **Downloads** | ~635K |
| **Last updated** | ~7 months ago |

Full RFC 5322 + MIME (RFC 2045-2049) message construction. Automatic optimal encoding selection per body part. Nested multipart (mixed, alternative, related). Inline attachments via Content-ID. International character support (RFC 6532). Fast base64 encoding (Chromium-based).

**Why we need it**: Gmail API `messages.send`, JMAP `Email/import`, and Graph API `/sendMail` all accept raw RFC 5322 messages. Currently we build these in TypeScript. Moving to Rust means we need `mail-builder` to construct outgoing messages.

**Relation to existing deps**: Companion to `mail-parser` (same author/org). `mail-parser` parses incoming messages, `mail-builder` constructs outgoing. Keep `lettre` for SMTP transport only.

### `reqwest-middleware` + `reqwest-retry` — RECOMMENDED

| Field | Value |
|-------|-------|
| **Crate** | `reqwest-middleware` 0.5 + `reqwest-retry` 0.9 |
| **By** | TrueLayer |
| **License** | MIT OR Apache-2.0 |
| **Downloads** | ~15M (middleware) |

Wraps reqwest `Client` with middleware chain. `reqwest-retry` provides `RetryTransientMiddleware` with configurable exponential backoff. Rather than hand-rolling retry logic per provider, wrap once.

**Why we need it**: Gmail, Graph, and JMAP all have rate limits and transient failures (429, 5xx, network drops). Consistent retry with backoff across all providers.

### `email_address` — OPTIONAL

| Field | Value |
|-------|-------|
| **Version** | 0.2.x |
| **License** | MIT |
| **Downloads** | ~3.7M |

RFC 5321/5322 compliant email address parsing and validation. `EmailAddress` newtype with `FromStr`. Serde support. Useful for validating addresses in account settings and compose fields in the Rust layer.

### `ammonia` — OPTIONAL (FUTURE)

| Field | Value |
|-------|-------|
| **Version** | 4.1.2 |
| **License** | MIT OR Apache-2.0 |
| **Downloads** | ~7.9M |

Whitelist-based HTML sanitization using html5ever. If we move HTML sanitization from frontend (DOMPurify) to Rust, this is the standard choice. Not needed immediately.

### `html2text` — OPTIONAL (FUTURE)

Converts HTML to plain text. Useful for generating `text/plain` alternative from HTML body when composing multipart/alternative messages, and for search indexing. Not critical path.

### Already adequate — no changes needed

| Crate | Status |
|-------|--------|
| `mail-parser` 0.11 | Excellent. Zero-copy, fuzz-tested, handles malformed real-world emails. Keep. |
| `lettre` 0.11 | SMTP transport. Keep for SMTP, use `mail-builder` for message construction. |
| `base64` 0.22 | Has `URL_SAFE_NO_PAD` for Gmail API raw encoding. Keep. |
| `reqwest` 0.13 | JSON, form, streaming, connection pooling, timeouts. May want `multipart` + `stream` features for Graph large attachments. Keep. |
| `serde_json` 1.0 | Use `#[serde(rename = "@odata.nextLink")]` for OData. `RawValue` for JMAP polymorphic responses. Keep. |
| `zstd` 0.13 | Body compression. Keep. |
| `tantivy` 0.25 | Full-text search. Keep. |

### Not needed — skip

| Crate | Why |
|-------|-----|
| `mailparse` | Inferior to `mail-parser` (no zero-copy, less RFC coverage). |
| `json-patch` | Graph uses simple PATCH bodies, not RFC 6902. |
| OData query builder | No good Rust crate exists. Simple enough with `format!()`. |
| `tower` / `tower-http` | Server-focused. `reqwest-middleware` is better for API clients. |
| `mime` | Already pulled transitively. `mail-builder` handles MIME types internally. |

---

## Recommended Cargo.toml Additions

### Immediate (for JMAP + Gmail + Graph providers)

```toml
# JMAP client — full RFC 8620/8621 implementation
jmap-client = { version = "0.4", default-features = false, features = ["async"] }

# RFC 5322 message construction (for raw send via Gmail/JMAP/Graph)
mail-builder = "0.4"

# HTTP retry middleware (rate limits, transient failures)
reqwest-middleware = "0.5"
reqwest-retry = "0.9"
```

### Consider later

```toml
# Type-safe OAuth2 protocol (if refactoring OAuth for multi-provider)
# oauth2 = "5.0"

# Email address validation (if moving compose validation to Rust)
# email_address = "0.2"

# HTML sanitization (if moving from frontend DOMPurify to Rust)
# ammonia = "4"

# HTML to plain text (for multipart/alternative generation)
# html2text = "0.16"
```

---

## Architecture Decisions

### 1. JMAP: Use `jmap-client`, vendor if needed

`jmap-client` is the only viable Rust JMAP client. It provides typed methods for every operation we need. The Issue #18 bug (using `false` instead of `null` for mailbox/keyword removal) may require a local patch — vendor the crate if needed.

The crate uses reqwest internally, matching our existing HTTP stack. Session discovery, auth (Basic/Bearer), batch requests, blob upload/download are all handled.

### 2. Gmail: Hand-roll on reqwest

Port the existing TypeScript `GmailClient` to Rust. ~17 REST endpoints, each a simple reqwest call with serde structs for request/response. Token refresh wraps all calls. `mail-builder` constructs outgoing RFC 5322 messages. `base64::URL_SAFE_NO_PAD` for the raw encoding Gmail expects.

The auto-generated `google-gmail1` crate is in maintenance mode, uses hyper directly (not reqwest), and brings `yup-oauth2` baggage. Not worth the dependency.

### 3. Microsoft Graph: Hand-roll on reqwest

Same pattern as Gmail. ~18 REST endpoints with OData query parameters. Delta sync per-folder (tokens don't expire like Gmail's History API). `mail-builder` for constructing MIME messages. Generic `ODataCollection<T>` wrapper for paginated responses.

`graph-rs-sdk` covers the entire Graph API surface (not just Mail), has a single maintainer with 8-month inactivity, and brings its own OAuth layer. Not worth it for ~18 endpoints.

### 4. OAuth: Keep hand-rolled, extend for Microsoft

Our existing `oauth.rs` (localhost server, PKCE, token exchange, refresh, CSRF validation) works well for Google. Extend it with Microsoft endpoints and scopes — the flow is identical, only endpoints and scopes differ. The `oauth2` crate could add type-safety later but isn't blocking.

### 5. Shared infrastructure

All three REST-based providers (Gmail, JMAP, Graph) share:
- `reqwest` HTTP client (with `reqwest-middleware` retry)
- `mail-builder` for outgoing message construction
- `mail-parser` for incoming message parsing
- `base64` for encoding
- `serde`/`serde_json` for JSON
- Token refresh logic (provider-specific endpoints, shared pattern)
- Body store writes (`bodies.db` with zstd compression)
- Sync state persistence (SQLite)

This shared stack is already in our Rust layer. Adding the three providers means adding provider-specific API call modules and sync logic, not new infrastructure.

### 6. The Stalwart ecosystem alignment

Three Stalwart crates form a coherent stack for our use case:
- `mail-parser` (already used) — parse incoming RFC 5322 messages
- `mail-builder` (adding) — construct outgoing RFC 5322 messages
- `jmap-client` (adding) — JMAP protocol client

Same author (Mauro D.), consistent APIs, production-hardened in Stalwart Mail Server (11.9K GitHub stars). This is the strongest alignment in the Rust email ecosystem.
