# Gmail API Rust Migration — Completed

**Completed**: March 2026

Rust-native Gmail API provider using hand-rolled `reqwest` REST client (~20 endpoints). Implements `ProviderOps` trait (Phase 3a). 23 Gmail-specific Tauri commands (`gmail_*` prefix), plus email actions routed through 17 provider-agnostic `provider_*` commands. Full sync engine (initial with parallel thread fetch + delta via History API). All email actions, send/drafts, attachments, send-as aliases. TS integration wired through providerFactory, syncManager, emailActions. ~2,836 lines across 8 modules.

### What was built

- `src-tauri/src/gmail/client.rs` (467 lines) — `GmailClient` wrapping `Arc<RwLock<TokenState>>` with `reqwest::Client`, auto-refresh 5min before expiry. `GmailState` holds per-account clients map
- `src-tauri/src/gmail/api.rs` (256 lines) — ~20 Gmail REST methods (labels CRUD, threads list/get/modify/delete, messages get, drafts CRUD, history, send-as, send, attachment fetch)
- `src-tauri/src/gmail/sync.rs` (835 lines) — initial sync (labels → thread list → parallel fetch, concurrency=10) and delta sync (History API, pending-ops conflict filter, full-sync fallback on history expiry)
- `src-tauri/src/gmail/ops.rs` (247 lines) — `GmailOps` implementing `ProviderOps` (17 async methods), delegates to `api.rs` and `sync.rs`
- `src-tauri/src/gmail/parse.rs` (213 lines) — Gmail API response → `ParsedGmailMessage` (MIME tree walk, base64url decoding, attachment extraction)
- `src-tauri/src/gmail/auth_parser.rs` (232 lines) — SPF/DKIM/DMARC parsing from `Authentication-Results` headers
- `src-tauri/src/gmail/commands.rs` (355 lines) — 23 `#[tauri::command]` functions registered in `lib.rs`
- `src-tauri/src/gmail/types.rs` (223 lines) — Gmail API serde structs (~25 types)
- `src-tauri/src/gmail/mod.rs` (8 lines) — module declarations

Shared infrastructure (`src-tauri/src/provider/`, 896 lines):

- `crypto.rs` (96 lines) — AES-256-GCM encrypt/decrypt matching TS format (`base64(iv):base64(ct+tag)`). Used by all providers
- `token.rs` (95 lines) — `TokenState` struct + `refresh_oauth_token()` (generic) + `refresh_google_token()` (Google endpoint convenience)
- `http.rs` (48 lines) — `build_http_client()` (shared reqwest defaults), `RetryConfig`, `compute_retry_delay()` (Retry-After + exponential backoff)
- `ops.rs` (100 lines) — `ProviderOps` trait: 17 async methods (sync, thread actions, send/drafts, attachments, folders)
- `router.rs` (49 lines) — dispatches `provider_*` commands to the correct `ProviderOps` impl based on account provider type
- `commands.rs` (459 lines) — 17 provider-agnostic `#[tauri::command]` functions (`provider_*` prefix)
- `types.rs` (42 lines) — `ProviderCtx`, `SyncResult`, `AttachmentData`, `ProviderFolder`

### TS layer (Rust-backed)

| File | How it uses Rust |
|------|-----------------|
| `gmailProvider.ts` | All methods call `invoke('gmail_*')` |
| `providerFactory.ts` | Creates `GmailApiProvider(accountId)` for non-IMAP accounts |
| `syncManager.ts` | Calls `gmail_sync_initial` / `gmail_sync_delta`, runs post-sync hooks |
| `tokenManager.ts` | Calls `gmail_init_client` on startup for each Gmail account |
| `emailActions.ts` | Routes Gmail operations through `invoke('gmail_*')` |
| `sendAs.ts` | Uses `invoke('gmail_fetch_send_as')` |
| `draftDeletion.ts` | Uses `invoke('gmail_list_drafts')` / `invoke('gmail_delete_draft')` |

### Key design decisions (still relevant)

- **Rust owns tokens** — no dual control plane. `GmailState` holds canonical token state. TS only passes `account_id`
- **`&self` everywhere** — `GmailClient` is `Arc`-wrapped, `Clone` is cheap. Supports concurrent API calls (sync uses concurrency=10 for thread fetch)
- **Sync writes directly to DB** — no IPC per message. Rust writes to `ratatoskr.db` (threads, messages, labels, attachments), `bodies.db` (zstd-compressed), and tantivy search index
- **Pending-ops conflict check** — delta sync skips threads with entries in `pending_operations` table, preventing sync from overwriting optimistic local state
- **No RFC 5322 message construction in Rust** — TS composer builds raw RFC 5322 messages; Rust commands accept pre-built `raw_base64url` bytes
- **`ProviderOps` trait** (Phase 3a) — `GmailOps` implements the shared trait, enabling provider-agnostic `provider_*` commands alongside the 23 Gmail-specific commands

### Sync vs queue write ordering

Two writers mutate local state: Rust sync (every 60s) and TS queue processor (every 30s). The `pending_operations` table is the coordination point — Rust sync checks it before overwriting any thread. SQLite's `Mutex<Connection>` serializes all writes.

### What stays in TS

- OAuth flow (browser interaction, localhost server)
- Sync timer (60s interval, multi-account orchestration)
- Post-sync hooks (filters, smart labels, notifications, AI categorization)
- `emailActions.ts` (optimistic UI, offline queue)
- `queueProcessor.ts` (dequeue + dispatch to Rust commands)
- `authParser.ts` (types + function still used by components)
- `messageParser.ts` (type-only: `ParsedMessage`, `ParsedAttachment` used by IMAP/filters/smart labels)

### `getGmailClient()` status

`client.ts` and `getGmailClient()` are retained only for Google Calendar API calls (same OAuth token, different endpoint). Can be deleted once Calendar gets its own Rust client.

---

## Deferred Work

1. **Google Calendar Rust client** — currently uses TS `GmailClient` for Calendar API calls
2. **RFC 5322 construction in Rust** — would eliminate the TS→Rust serialization boundary for send/draft

## References

- `docs/graph-rust-migration.md` — Graph patterns (follows same ProviderOps trait)
- `docs/jmap-rust-migration.md` — JMAP patterns (follows same ProviderOps trait)
