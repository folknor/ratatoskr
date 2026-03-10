# Rust Provider Crate Research — Decisions Made

**Completed**: March 2026

## Summary

Evaluated Rust crate ecosystem for moving Gmail API, JMAP, and Microsoft Graph providers into the Rust backend alongside IMAP. Core decision: use `jmap-client` for JMAP, hand-roll reqwest-based clients for Gmail and Graph, keep existing hand-rolled OAuth.

## Crate decisions

| Crate / Area | Recommendation | Actually adopted? | Notes |
|---|---|---|---|
| `jmap-client` 0.4 (Stalwart) | Use it — only viable Rust JMAP client | Yes | In Cargo.toml with `default-features = false, features = ["async"]` |
| Gmail API client | Hand-roll on `reqwest` | Yes | ~17 REST endpoints, ported from TS `GmailClient` |
| Microsoft Graph client | Hand-roll on `reqwest` | Yes | ~18 REST endpoints with OData query params |
| OAuth2 | Keep hand-rolled `oauth.rs` | Yes | Extended for Microsoft endpoints; `oauth2` crate not added |
| `mail-builder` 0.4 | Recommended for RFC 5322 message construction | No | Message construction stayed in TypeScript |
| `reqwest-middleware` + `reqwest-retry` | Recommended for shared retry | No | Hand-rolled retry in `provider/http.rs` instead |
| `email_address` | Optional | No | |
| `ammonia` | Optional/future | No | HTML sanitization stays in frontend (DOMPurify) |
| `html2text` | Optional/future | No | |

## Rejected crates — rationale

- **`google-gmail1`** (Byron/google-apis-rs): Maintenance mode, uses hyper (not reqwest), brings `yup-oauth2` baggage. Not worth it for ~17 endpoints.
- **`graph-rs-sdk`**: Covers entire Graph API (not just Mail), single maintainer, 8-month inactivity, brings own OAuth layer. Too heavy for ~18 endpoints.
- **`yup-oauth2`**: Google-focused, opinionated file-based token storage, pulls in Hyper as HTTP server.
- **`openidconnect`**: OIDC discovery overkill — we need access tokens, not ID tokens.
- **`melib`**: Has JMAP backend but GPLv3 — license incompatible.
- **`msgraph-rs`**: GPL-3.0, incomplete. Not viable.
- **`tauri-plugin-oauth`**: Does what our `oauth.rs` already does. Lateral move.

## Architecture decisions — divergences from plan

### Shared trait timing
The doc recommended "defer shared trait until 2 providers exist in Rust." The `ProviderOps` trait (`provider/ops.rs`) was extracted after Gmail + JMAP (Phase 3a). It now has 3 implementations:
- `GmailOps` (`gmail/ops.rs`)
- `JmapOps` (`jmap/ops.rs`)
- `GraphOps` (`graph/ops.rs`)

### Unified commands
The doc recommended provider-prefixed commands (`gmail_*`, `jmap_*`, `graph_*`) with routing in TS. Instead, unified `provider_*` commands exist in `provider/commands.rs` (~17 commands: sync, archive, trash, star, spam, move, tag, send, draft CRUD, attachment fetch, list folders) that route via the `ProviderOps` trait in Rust. Provider-specific commands still exist alongside these.

### Stalwart ecosystem
`mail-parser` 0.11 and `jmap-client` 0.4 are both adopted from the Stalwart ecosystem. `mail-builder` was not adopted — outgoing message construction remained in TypeScript.

## Crates that were already adequate (unchanged)

`mail-parser` 0.11, `lettre` 0.11, `base64` 0.22, `reqwest` 0.13, `serde`/`serde_json` 1.0, `zstd` 0.13, `tantivy` 0.25 — all kept as-is per the original recommendation.
