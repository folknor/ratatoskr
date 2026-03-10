# Microsoft Graph Provider — Phase 3b Complete

**Completed**: March 2026

Rust-native Microsoft Graph Mail API provider using hand-rolled `reqwest` REST client (~18 endpoints). Implements `ProviderOps` trait (Phase 3a). 4 Graph-specific Tauri commands (`graph_*` prefix), all email actions routed through 17 provider-agnostic `provider_*` commands. Per-folder delta sync with priority-based scheduling. OAuth2 via Entra ID (PKCE, `/common` tenant). ~2,913 lines across 8 modules.

### What was built

- `src-tauri/src/graph/client.rs` (605 lines) — `GraphClient` wrapping `Arc<ClientInner>` with `reqwest::Client`, token refresh via `RwLock<TokenState>`, `Semaphore(3)` for Graph's 4-concurrent-request limit, 60s TTL cached `FolderMap`, sync cycle counter
- `src-tauri/src/graph/sync.rs` (1070 lines) — initial sync (folder traversal + per-folder message fetch in batches of 50 + delta token bootstrap) and delta sync with priority-based folder scheduling (INBOX/SENT/DRAFT every cycle, TRASH/SPAM/archive every 5th, user folders every 20th, folder tree re-traversal every 10th)
- `src-tauri/src/graph/ops.rs` (589 lines) — `GraphOps` implementing `ProviderOps` (17 async methods). Thread actions use local DB enumeration. Category add/remove via read-modify-write on `categories` array. Send via create-draft-then-send pattern
- `src-tauri/src/graph/parse.rs` (228 lines) — Graph JSON message → `ParsedGraphMessage` for DB persistence, attachment metadata extraction
- `src-tauri/src/graph/types.rs` (213 lines) — `GraphMessage`, `ODataCollection<T>`, `GraphMailFolder`, `GraphAttachment`, request/patch structs
- `src-tauri/src/graph/folder_mapper.rs` (130 lines) — `FolderMap` mapping opaque Graph folder IDs to Gmail-style label IDs via well-known API aliases (`inbox`, `sentitems`, `deleteditems`, `junkemail`, `drafts`, `archive`)
- `src-tauri/src/graph/commands.rs` (71 lines) — 4 Tauri commands: `graph_init_client`, `graph_remove_client`, `graph_test_connection`, `graph_get_profile`
- `src-tauri/src/graph/mod.rs` (7 lines) — module declarations
- DB migration v25: `graph_folder_delta_tokens` table (account_id + folder_id → delta token)

### Key design decisions (still relevant)

- **Hand-rolled on reqwest, no graph-rs-sdk** — ~18 REST endpoints, thin wrapper with retry + semaphore
- **Microsoft Graph API, not EWS** (deprecated) — `/v1.0/me/messages`, `/me/mailFolders`
- **OAuth2 via Entra ID** — PKCE flow, `/common` tenant, same localhost callback server as Gmail (ports 17248-17251)
- **Folder → label mapping** — well-known folder aliases resolved via API (`GET /me/mailFolders/inbox`), not display name matching. Categories mapped to tag labels, flags to pseudo-labels
- **Thread ID uses `conversationId`** — not JWZ threading. Available directly in Graph API responses
- **Thread actions use local DB enumeration** — not remote Graph API query. Loops per-message (no JSON `$batch`)
- **Per-folder delta sync** (not global like Gmail/JMAP) — delta tokens stored in `graph_folder_delta_tokens` table, one per folder
- **Priority-based folder scheduling** — Tier 0 (INBOX/SENT/DRAFT) every cycle, Tier 1 (TRASH/SPAM/archive) every 5th, Tier 2 (user folders) every 20th
- **Folder tree re-traversal every 10th cycle** with delta token bootstrap for new folders and cleanup for removed ones
- **60s TTL cache on FolderMap** — avoids redundant API calls from `list_folders`
- **Concurrency: `Semaphore(3)`** — Graph allows 4 concurrent requests per app per mailbox; reserve 1 for user-initiated actions during sync
- **Send: create-draft-then-send** — accepts `raw_base64url` per trait contract, parses MIME via `mail-parser`, creates draft (`POST /me/messages`), attaches files, then sends (`POST /me/messages/{id}/send`)
- **`update_draft` is delete-then-create** — Graph has no draft PATCH. Returns new ID; `draftAutoSave.ts` handles ID change
- **Attachment metadata via `$expand=attachments`** — enumerated inline during sync, not via separate API calls
- **`raw_size` always 0** — Graph API doesn't expose message byte size
- **Delta response deserialized as `serde_json::Value`** — handles `@removed` annotation without `#[serde(flatten)]` issues. `ODataDeltaItem<T>` type exists but delta pages use `Value` in practice
- **Blob/attachment IDs stored in existing `gmail_attachment_id` column** — same pattern as JMAP's `blob_id`
- **Category add/remove is read-modify-write** — Graph has no atomic array operation, so race condition is unavoidable

### Implementation divergences from original plan

- **No `api.rs` module** — all action functions live in `ops.rs`, sync functions in `sync.rs`
- **`GraphClient` wraps `Arc<ClientInner>`** (like Gmail), not middleware stack — uses raw `reqwest::Client`
- **No JSON `$batch` for thread actions** — loops per-message instead. Simpler, works within semaphore limits
- **4 graph-specific commands, not ~27** — most operations route through the 17 provider-agnostic `provider_*` commands (Phase 3a)
- **~2,913 lines actual vs 1,600-2,200 estimated** — `client.rs` larger due to embedded OAuth + semaphore + folder map caching; `sync.rs` larger due to priority scheduling + folder re-traversal logic

### What remains (Phase 3c: TS/UI integration)

- `AddGraphAccount.tsx` — account setup UI (OAuth flow → test → save)
- `syncManager.ts` — add `syncGraphAccount()` branch
- `AccountProvider` type — add `"graph"` to union
- `App.tsx` startup — call `graph_init_client` for Graph accounts
- `providerFactory.ts` — routing (emailActions already handles non-IMAP automatically)
- Azure AD app registration — create, configure, choose default-shipped vs user-provided model
- `microsoft_client_id` settings UI

---

## Deferred Work

1. **App registration model** (ship default vs user-provided) — open question
2. **Large attachment upload sessions** (>3MB) — multi-step upload not yet implemented
3. **Webhook subscriptions for real-time sync** — polling via delta sync is the initial approach
4. **Focused Inbox integration** (`inferenceClassification` → category tabs) — optional enrichment
5. **Exchange on-premises via EWS** — use IMAP instead
6. **JSON `$batch` optimization for thread actions** — currently loops per-message
7. **Category add/remove race condition** — unavoidable (Graph has no atomic array op)
8. **IMAP + OAuth2 for Outlook.com** — quick win, independent of full Graph provider

## References

- `docs/phase-3a-proposal.md` — consolidation design (ProviderOps trait, tag/folder split, send contract)
- `docs/jmap-rust-migration.md` — JMAP patterns that Graph follows
- `docs/gmail-rust-migration.md` — Gmail patterns (token management, sync-queue coordination)
