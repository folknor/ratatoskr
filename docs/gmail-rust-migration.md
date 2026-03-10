# Gmail API → Rust Migration Plan

**Date**: March 2026
**Status**: Planning
**Goal**: Move all Gmail API logic from TypeScript to Rust, establishing patterns that JMAP and Microsoft Graph providers will follow.

---

## Table of Contents

- [Why Gmail First](#why-gmail-first)
- [Decisions Up Front](#decisions-up-front)
- [Current State (TypeScript)](#current-state-typescript)
- [Target State (Rust)](#target-state-rust)
- [Phase 1: Rust Gmail Client + Tauri Commands](#phase-1-rust-gmail-client--tauri-commands)
- [Phase 2: Sync Engine in Rust](#phase-2-sync-engine-in-rust)
- [Phase 3: TS Layer Teardown](#phase-3-ts-layer-teardown)
- [Sync vs Queue: Write Ordering](#sync-vs-queue-write-ordering)
- [Migration Strategy](#migration-strategy)
- [What We Defer](#what-we-defer)

---

## Why Gmail First

1. **Known-good reference** — existing TS `GmailClient` is a 1:1 blueprint to port
2. **Most-used provider** — must be solid before adding new providers
3. **IMAP pattern exists** — `src-tauri/src/imap/` is a proven Tauri command structure to replicate
4. **Builds transferable patterns** — token refresh, reqwest+retry HTTP client, `mail-builder` integration, body store writes from Rust, sync progress via Tauri events. These patterns inform JMAP and Graph implementations, but we do NOT extract shared traits or abstractions until a second provider exists to validate them.

---

## Decisions Up Front

These are not open questions. They must be settled before writing code, because they affect every subsequent design choice.

### 1. Token ownership: Rust from day one

Rust owns Gmail tokens from the moment an account is migrated. There is no dual control plane.

- **TS `tokenManager.ts` is retired for Gmail accounts.** It no longer creates or caches `GmailClient` instances for Gmail. It may remain for other providers until they are migrated.
- **Rust `GmailState`** holds the canonical token state per account. Tokens are loaded from DB on first use, refreshed by Rust, persisted to DB by Rust.
- **TS involvement is limited to**: (a) initial OAuth flow (getting the first token pair and writing it to DB), (b) calling Tauri commands that reference an `account_id` — never passing raw tokens.
- **No per-command fallback.** Once a Gmail account's first Tauri command succeeds, all subsequent operations for that account go through Rust. If a Rust command has a bug, we fix it in Rust — we don't route back to TS. The fallback model is: keep the TS code around during development so we can compare behavior, but the runtime path is Rust-only per account.

This eliminates the token-refresh race condition between TS and Rust. There is exactly one writer of token state per account at any time.

### 2. Client concurrency: immutable transport + synchronized token

The `GmailClient` struct must support concurrent API requests (Phase 2 needs parallel `getThread()` at concurrency 10). This rules out `&mut self` on API methods.

**Design:**

```rust
/// Immutable HTTP transport — can be cloned and shared across tasks.
/// Token state lives behind Arc<RwLock> so concurrent reads don't block,
/// and the rare refresh takes a brief write lock.
pub struct GmailClient {
    http: ClientWithMiddleware,       // reqwest-middleware, Clone
    account_id: String,
    token: Arc<RwLock<TokenState>>,   // shared, separately synchronized
    client_id: String,
    client_secret: Option<String>,
}

struct TokenState {
    access_token: String,
    refresh_token: String,
    expires_at: i64,                  // unix timestamp
    refreshing: Option<Shared<BoxFuture<'static, Result<(), String>>>>,
}
```

**How concurrent access works:**
- API methods take `&self`, not `&mut self`.
- Before each request, read-lock `token` to get the current `access_token`.
- If token is expired or within 5min of expiry, upgrade to write lock, check again (double-check pattern), and refresh if still needed. The `refreshing` future is shared via `futures::future::Shared` so concurrent callers coalesce on one refresh.
- After refresh, persist new tokens to DB (via `DbState`).

**`GmailState` in Tauri:**

```rust
pub struct GmailState {
    clients: RwLock<HashMap<String, GmailClient>>,
}
```

`RwLock` (not `Mutex`) on the outer map because most access is reads (get existing client). Write lock only for adding/removing clients. Each `GmailClient` is `Clone` (due to `Arc` internals), so sync code can clone a client handle and use it freely across spawned tasks.

### 3. No shared `EmailProvider` trait yet

We do NOT define a Rust `EmailProvider` trait during the Gmail migration. Reasons:

- Gmail is label-centric. Graph is folder-centric. JMAP uses mailboxes + EmailSubmission + `jmap-client` (a third-party crate with its own API surface). Forcing them into a common trait before two implementations exist will produce a leaky abstraction we'll spend time fighting.
- The IMAP provider already exists as raw Tauri commands without a Rust trait. Adding Gmail as another set of Tauri commands is consistent.
- Once Gmail AND one of {JMAP, Graph} exist in Rust, we can see what actually overlaps and extract a trait from real code. This is extract-from-two, not design-from-one.

What we DO build as reusable (non-trait) infrastructure:
- `TokenState` + refresh logic (parameterized by token endpoint, reusable for Google and Microsoft)
- `build_api_client()` (shared reqwest-middleware setup)
- `mail-builder` message construction utilities
- `ParsedMessage` output struct (the common internal representation that all providers produce — this already exists as the shape written to DB)

### 4. Commands are provider-specific during migration

All Tauri commands exposed during the Gmail migration are explicitly `gmail_*` prefixed. No generic `sync_account` or `send_message` routing. Reasons:

- Provider routing happens in TS (`syncManager.ts`, `providerFactory.ts`, `emailActions.ts`) which already knows the account type.
- Provider-agnostic Tauri commands require a Rust-side router, which requires the trait we're deferring.
- `gmail_*` commands are honest about what they do and easy to grep/remove.

When a second provider is added in Rust and the trait is extracted, we can introduce generic commands that route internally. That's a future refactor, not a Phase 1 concern.

---

## Current State (TypeScript)

**9 files, ~3,022 lines** in `src/services/gmail/`:

| File | Lines | Purpose |
|------|-------|---------|
| `client.ts` | 461 | `GmailClient` — token refresh (5min pre-expiry, mutex-protected), rate-limit retry (exponential backoff), all Gmail REST API calls |
| `auth.ts` | 212 | OAuth2 + PKCE flow — generates verifier/challenge, starts Rust localhost server, opens browser, exchanges code, fetches userinfo |
| `tokenManager.ts` | 151 | In-memory `Map<accountId, GmailClient>` cache, bulk init on startup, re-auth flow, reads/writes encrypted tokens in DB |
| `syncManager.ts` | 555 | Sync orchestration — 60s interval, routes Gmail/IMAP, post-sync hooks (filters, smart labels, notifications, categorization) |
| `sync.ts` | 547 | Three-phase initial sync (labels → threads → messages, parallel concurrency=10). Delta sync via History API (concurrency=5, fallback to full on HISTORY_EXPIRED) |
| `messageParser.ts` | 175 | Gmail API response → `ParsedMessage` (header extraction, base64url body decoding, attachment collection, auth result parsing) |
| `authParser.ts` | 175 | SPF/DKIM/DMARC parsing from Authentication-Results headers |
| `sendAs.ts` | 41 | Fetch send-as aliases from Gmail API |
| `draftDeletion.ts` | 20 | Delete drafts for a thread |

**Supporting TS files**:

| File | Lines | Purpose |
|------|-------|---------|
| `email/gmailProvider.ts` | 261 | `EmailProvider` adapter wrapping `GmailClient` |
| `email/types.ts` | 86 | `EmailProvider` interface (24 methods), `SyncResult`, `EmailFolder` |
| `email/providerFactory.ts` | 60 | Routes `account.provider` → provider instance, in-memory cache |

**Data flow**:
```
GmailClient (token mgmt + API calls)
  → sync.ts (initial/delta logic)
    → messageParser.ts (Gmail struct → ParsedMessage)
      → processAndStoreThread()
        ├─ upsertThread() + setThreadLabels()
        ├─ upsertMessage() + bodyStorePut()
        ├─ indexMessage() → tantivy
        ├─ upsertAttachment()
        └─ post-sync hooks (filters, smart labels, notifications, categorization)
```

---

## Target State (Rust)

### Module structure

```
src-tauri/src/gmail/
├── mod.rs           # Re-exports
├── types.rs         # Gmail API request/response serde types
├── client.rs        # GmailClient — Arc<RwLock<TokenState>>, reqwest, &self methods
├── api.rs           # Gmail REST endpoint methods (list, get, send, modify, history, etc.)
├── parse.rs         # Gmail API response → internal message types
├── auth_parser.rs   # SPF/DKIM/DMARC header parsing
└── sync.rs          # Initial sync + delta sync (Phase 2)
```

### Reusable (non-trait) infrastructure

```
src-tauri/src/provider/
├── mod.rs
├── token.rs         # TokenState, refresh logic (parameterized by endpoint)
├── http.rs          # build_api_client() — shared reqwest-middleware setup
└── message.rs       # mail-builder RFC 5322 construction utilities
```

These are plain functions and structs, not a trait. Gmail uses them directly. JMAP and Graph will import the same utilities when they're built.

### Tauri command surface (gmail-specific)

```rust
// Sync (Phase 2)
gmail_sync_initial(account_id, days_back)
gmail_sync_delta(account_id)

// Actions (called by TS queue processor and emailActions)
gmail_send_email(account_id, raw_base64url, thread_id?)
gmail_modify_messages(account_id, message_ids, add_labels, remove_labels)
gmail_trash_messages(account_id, message_ids)
gmail_untrash_messages(account_id, message_ids)
gmail_delete_messages(account_id, message_ids)
gmail_create_draft(account_id, raw_base64url, thread_id?)
gmail_update_draft(account_id, draft_id, raw_base64url, thread_id?)
gmail_delete_draft(account_id, draft_id)
gmail_fetch_attachment(account_id, message_id, attachment_id)
gmail_list_labels(account_id)
gmail_create_label(account_id, name, ...)
gmail_fetch_send_as(account_id)

// Connection / lifecycle
gmail_test_connection(account_id)
gmail_init_client(account_id)      // load tokens from DB, create client
gmail_remove_client(account_id)    // evict on account deletion or re-auth
```

TS calls `gmail_init_client` after OAuth completes or on app startup. All subsequent commands reference `account_id` only — no tokens cross the IPC boundary.

---

## Phase 1: Rust Gmail Client + Tauri Commands

**Goal**: Port `GmailClient` and all Gmail REST API calls to Rust. TS stops making HTTP requests to Google. TS sync logic still orchestrates (calling Rust commands for each API call), but token management and HTTP are entirely Rust.

**Migration unit**: per-account. Once `gmail_init_client` succeeds for an account, that account uses Rust for all Gmail API calls. The TS `GmailClient` for that account is not created.

### 1a. Token + HTTP infrastructure (`provider/`)

Built first because `GmailClient` depends on it.

**`provider/token.rs`**:

```rust
pub struct TokenState {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
}

/// Refresh tokens via the given endpoint. Provider-agnostic.
pub async fn refresh_oauth_token(
    http: &reqwest::Client,
    token_endpoint: &str,
    refresh_token: &str,
    client_id: &str,
    client_secret: Option<&str>,
) -> Result<TokenState, String>;
```

**`provider/http.rs`**:

```rust
pub fn build_api_client() -> ClientWithMiddleware {
    let retry_policy = ExponentialBackoff::builder()
        .retry_bounds(Duration::from_secs(1), Duration::from_secs(30))
        .build_with_max_retries(3);

    ClientBuilder::new(reqwest::Client::new())
        .with(RetryTransientMiddleware::new_with_policy(retry_policy))
        .build()
}
```

### 1b. Gmail API types (`gmail/types.rs`)

Serde structs for Gmail API request/response shapes:

```rust
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailMessage {
    pub id: String,
    pub thread_id: String,
    pub label_ids: Option<Vec<String>>,
    pub snippet: Option<String>,
    pub history_id: Option<String>,
    pub internal_date: Option<String>,
    pub payload: Option<GmailPayload>,
    pub size_estimate: Option<i64>,
    pub raw: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailPayload {
    pub mime_type: Option<String>,
    pub headers: Option<Vec<GmailHeader>>,
    pub body: Option<GmailBody>,
    pub parts: Option<Vec<GmailPayload>>,
}

// GmailHeader, GmailBody, GmailThread, GmailThreadStub, GmailLabel,
// GmailHistoryResponse, GmailHistoryItem, ListResponse<T>,
// GmailDraft, GmailSendAs, GmailAttachmentData, GmailProfile
```

### 1c. Gmail client (`gmail/client.rs`)

```rust
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct GmailClient {
    http: ClientWithMiddleware,
    account_id: String,
    token: Arc<RwLock<TokenState>>,
    client_id: String,
    client_secret: Option<String>,
}

impl Clone for GmailClient { /* Arc fields, cheap clone */ }

impl GmailClient {
    /// Create from DB account record. Called once per account on init.
    pub async fn from_account(db: &DbState, account_id: &str) -> Result<Self, String>;

    /// Get a valid access token, refreshing if needed.
    /// Multiple concurrent callers coalesce on one refresh.
    async fn access_token(&self, db: &DbState) -> Result<String, String>;

    /// Authenticated GET request to Gmail API
    pub async fn get<T: DeserializeOwned>(
        &self, endpoint: &str, db: &DbState,
    ) -> Result<T, String>;

    /// Authenticated POST request to Gmail API
    pub async fn post<T: DeserializeOwned>(
        &self, endpoint: &str, body: &impl Serialize, db: &DbState,
    ) -> Result<T, String>;

    // Retry logic:
    // - 401 → refresh token, retry once
    // - 429 → respect Retry-After, exponential backoff, max 3 attempts
    // - 5xx → handled by reqwest-middleware retry
}
```

All methods take `&self`. Concurrent API calls share the client freely. Token refresh is synchronized internally via the `RwLock<TokenState>`.

### 1d. Gmail API methods (`gmail/api.rs`)

Direct ports of the ~17 REST calls from `client.ts`. All take `&self`:

```rust
impl GmailClient {
    pub async fn get_profile(&self, db: &DbState) -> Result<GmailProfile, String>;
    pub async fn list_labels(&self, db: &DbState) -> Result<Vec<GmailLabel>, String>;
    pub async fn list_threads(&self, db: &DbState, query: &str, page_token: Option<&str>) -> Result<ListResponse<GmailThreadStub>, String>;
    pub async fn get_thread(&self, db: &DbState, thread_id: &str, format: &str) -> Result<GmailThread, String>;
    pub async fn get_message(&self, db: &DbState, message_id: &str, format: &str) -> Result<GmailMessage, String>;
    pub async fn modify_message(&self, db: &DbState, message_id: &str, add: &[String], remove: &[String]) -> Result<(), String>;
    pub async fn batch_modify(&self, db: &DbState, ids: &[String], add: &[String], remove: &[String]) -> Result<(), String>;
    pub async fn batch_delete(&self, db: &DbState, ids: &[String]) -> Result<(), String>;
    pub async fn trash_message(&self, db: &DbState, message_id: &str) -> Result<(), String>;
    pub async fn untrash_message(&self, db: &DbState, message_id: &str) -> Result<(), String>;
    pub async fn get_history(&self, db: &DbState, start_history_id: &str, page_token: Option<&str>) -> Result<GmailHistoryResponse, String>;
    pub async fn send_message(&self, db: &DbState, raw_base64url: &str, thread_id: Option<&str>) -> Result<GmailMessage, String>;
    pub async fn get_attachment(&self, db: &DbState, message_id: &str, attachment_id: &str) -> Result<GmailAttachmentData, String>;
    pub async fn create_draft(&self, db: &DbState, raw_base64url: &str, thread_id: Option<&str>) -> Result<GmailDraft, String>;
    pub async fn update_draft(&self, db: &DbState, draft_id: &str, raw_base64url: &str, thread_id: Option<&str>) -> Result<GmailDraft, String>;
    pub async fn delete_draft(&self, db: &DbState, draft_id: &str) -> Result<(), String>;
    pub async fn list_send_as(&self, db: &DbState) -> Result<Vec<GmailSendAs>, String>;
}
```

### 1e. Message parsing (`gmail/parse.rs`)

Port of `messageParser.ts`. Pure function, no I/O:

```rust
pub fn parse_gmail_message(
    msg: &GmailMessage,
    thread_label_ids: &[String],
) -> Result<ParsedGmailMessage, String>;
```

Output struct matches the shape written to the `messages` DB table + body store. Fields: `id`, `thread_id`, `from_address`, `from_name`, `to_addresses`, `cc_addresses`, `bcc_addresses`, `reply_to`, `subject`, `snippet`, `date`, `is_read`, `is_starred`, `body_html`, `body_text`, `raw_size`, `internal_date`, `label_ids`, `has_attachments`, `attachments`, `list_unsubscribe`, `list_unsubscribe_post`, `auth_results`, `message_id_header`, `references_header`, `in_reply_to_header`.

### 1f. Auth header parsing (`gmail/auth_parser.rs`)

Port of `authParser.ts`. Regex-based SPF/DKIM/DMARC extraction from `Authentication-Results` headers. ~100 lines.

### 1g. Tauri commands + Tauri state

**`GmailState` registration** (in `lib.rs`):

```rust
.manage(GmailState::new())
```

**Commands** (in `commands.rs`):

```rust
#[tauri::command]
pub async fn gmail_init_client(
    account_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<(), String> {
    let client = GmailClient::from_account(&db, &account_id).await?;
    gmail.insert(account_id, client).await;
    Ok(())
}

#[tauri::command]
pub async fn gmail_list_labels(
    account_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<Vec<GmailLabel>, String> {
    let client = gmail.get(&account_id).await?;
    client.list_labels(&db).await
}

// ... same pattern for all gmail_* commands
```

### 1h. TS-side changes

- `syncManager.ts`: Gmail sync path calls `gmail_*` Tauri commands instead of `GmailClient` methods.
- `tokenManager.ts`: `initializeClients()` calls `gmail_init_client` for Gmail accounts instead of creating TS `GmailClient` instances. `getGmailClient()` is no longer called for Gmail accounts.
- `email/gmailProvider.ts`: Methods delegate to `gmail_*` Tauri commands.
- `emailActions.ts` / `queueProcessor.ts`: Queue processor calls `gmail_*` Tauri commands for Gmail accounts.

### 1i. Cargo.toml additions

```toml
mail-builder = "0.4"
reqwest-middleware = "0.5"
reqwest-retry = "0.9"
```

### Phase 1 deliverable

All Gmail HTTP calls happen in Rust. Token refresh is Rust-owned. TS is reduced to orchestration (sync timing, queue processing, post-sync hooks) and UI. No per-command fallback — an account is either fully TS or fully Rust.

---

## Phase 2: Sync Engine in Rust

**Goal**: Move initial sync and delta sync logic to Rust. Sync writes directly to `ratatoskr.db` and `bodies.db` from Rust — eliminating IPC round-trips for each message persisted.

**This is the hardest phase.** It includes: label persistence, paginated thread discovery, parallel thread fetch (concurrency=10), message parsing, DB writes, body store writes, search indexing, attachment writes, history checkpointing, delta-sync reconciliation with conflict handling, and error recovery (HISTORY_EXPIRED → full sync fallback). The scope is significantly larger than Phase 1.

### 2a. Gmail sync module (`gmail/sync.rs`)

```rust
/// Initial sync: labels → threads → messages (parallel).
/// Writes directly to DB, body store, and search index.
pub async fn gmail_initial_sync(
    client: &GmailClient,   // &self, not &mut self — supports concurrent use
    account_id: &str,
    days_back: i64,
    db: &DbState,
    body_store: &BodyStoreState,
    search: &SearchState,
    app_handle: &AppHandle,
) -> Result<(), String>;

/// Delta sync via History API.
/// Returns new inbox message IDs for TS post-sync hooks.
pub async fn gmail_delta_sync(
    client: &GmailClient,
    account_id: &str,
    db: &DbState,
    body_store: &BodyStoreState,
    search: &SearchState,
    app_handle: &AppHandle,
) -> Result<GmailDeltaSyncResult, String>;

pub struct GmailDeltaSyncResult {
    pub new_inbox_message_ids: Vec<String>,
    pub affected_thread_ids: Vec<String>,
}
```

Note: `client: &GmailClient` (not `&mut`). All API methods are `&self`. Parallel `getThread()` calls clone the client handle (cheap — `Arc` internals) and run concurrently via `tokio::sync::Semaphore`.

**Initial sync** (3-phase, same logic as TS `sync.ts`):
1. Fetch + persist labels via `gmail_list_labels` → DB writes
2. Paginated `list_threads(q: "after:YYYY/M/D")`, collect thread IDs
3. Parallel `get_thread()` (concurrency=10 via Semaphore), for each:
   - `parse_gmail_message()` on all messages in thread
   - DB writes: `upsert_thread()`, `set_thread_labels()`, `upsert_message()` via `DbState.with_conn()`
   - Body writes: `body_store_put()` via `BodyStoreState.with_conn()`
   - Search index: `index_message()` via `SearchState`
   - Attachment writes: `upsert_attachment()`
   - Track highest `historyId`
4. Persist `historyId` as account sync state

**Delta sync** (same logic as TS):
1. Paginate History API from `last_history_id`
2. Collect affected thread IDs from messagesAdded/Deleted, labelsAdded/Removed
3. Skip threads with pending local ops (query `pending_operations` table)
4. Re-fetch affected threads in parallel (concurrency=5)
5. Same DB/body/search writes as initial
6. Return `GmailDeltaSyncResult` with new inbox message IDs

**Error handling**:
- History 404 / expired → return error code `HISTORY_EXPIRED`, TS calls `gmail_sync_initial` as fallback
- API errors → propagate to TS for error display

**Progress reporting**: Emit Tauri events (same pattern as IMAP Rust sync):
```rust
app_handle.emit("gmail-sync-progress", &GmailSyncProgress {
    account_id, phase, current, total
})?;
```

### 2b. Sync Tauri commands

```rust
#[tauri::command]
pub async fn gmail_sync_initial(
    account_id: String,
    days_back: i64,
    db: State<'_, DbState>,
    body_store: State<'_, BodyStoreState>,
    search: State<'_, SearchState>,
    gmail: State<'_, GmailState>,
    app_handle: AppHandle,
) -> Result<(), String>;

#[tauri::command]
pub async fn gmail_sync_delta(
    account_id: String,
    db: State<'_, DbState>,
    body_store: State<'_, BodyStoreState>,
    search: State<'_, SearchState>,
    gmail: State<'_, GmailState>,
    app_handle: AppHandle,
) -> Result<GmailDeltaSyncResult, String>;
```

### 2c. TS-side changes

`syncManager.ts` simplifies to:

```typescript
async function syncGmailAccount(accountId: string) {
  if (account.history_id) {
    try {
      const result = await invoke('gmail_sync_delta', { accountId });
      // Post-sync hooks (still TS)
      await applyFiltersToNewMessageIds(accountId, result.newInboxMessageIds);
      await applySmartLabelsToNewMessageIds(accountId, result.newInboxMessageIds);
      // ... notifications, categorization
    } catch (err) {
      if (err === 'HISTORY_EXPIRED') {
        await invoke('gmail_sync_initial', { accountId, daysBack: syncDays });
      } else throw err;
    }
  } else {
    await invoke('gmail_sync_initial', { accountId, daysBack: syncDays });
  }
}
```

Post-sync hooks stay in TS. The boundary is: Rust does all I/O (API calls, DB writes, body store, search indexing) and returns a summary. TS runs application-layer hooks that depend on many TS services.

### Phase 2 deliverable

Gmail sync runs entirely in Rust with direct DB access. TS is reduced to: 60s timer, invoking one Tauri command per sync cycle, running post-sync hooks on the result. Initial sync performance improves significantly (no IPC serialization per message).

---

## Phase 3: TS Layer Teardown

**Goal**: Remove Gmail-specific TS code that's been replaced by Rust.

### 3a. Delete TS Gmail files

- `src/services/gmail/client.ts` — replaced by Rust `GmailClient`
- `src/services/gmail/sync.ts` — replaced by Rust sync
- `src/services/gmail/messageParser.ts` — replaced by Rust parser
- `src/services/gmail/authParser.ts` — replaced by Rust auth parser
- `src/services/gmail/sendAs.ts` — replaced by Rust command
- `src/services/gmail/draftDeletion.ts` — replaced by Rust command

### 3b. Simplify remaining TS

- **`tokenManager.ts`** — remove Gmail client cache. Retained only for non-Rust providers (IMAP, until migrated). `initializeClients()` calls `gmail_init_client` for Gmail accounts.
- **`auth.ts`** — stays (OAuth flow requires browser interaction, Tauri localhost server orchestration from TS).
- **`syncManager.ts`** — Gmail path becomes two lines: invoke Rust sync command, run hooks.
- **`email/gmailProvider.ts`** — thin wrapper calling `gmail_*` Tauri commands.
- **`email/providerFactory.ts`** — routes `"gmail_api"` → Tauri-backed `GmailApiProvider`.

### 3c. What stays in TS permanently (for now)

- OAuth flow initiation (browser interaction)
- Sync timer orchestration (60s interval, multi-account, queue coalescing)
- Post-sync hooks (filters, smart labels, notifications, AI categorization)
- `emailActions.ts` (optimistic UI, local DB writes, offline queue)
- `queueProcessor.ts` (dequeue + dispatch to `gmail_*` Rust commands)
- All UI components and Zustand stores

---

## Sync vs Queue: Write Ordering

This is the critical correctness concern when sync writes come from Rust but action writes come from TS.

### The problem

Two writers mutate local state for the same account:

1. **Rust sync** (delta sync every 60s): fetches remote state, writes to DB (messages, threads, labels, bodies)
2. **TS queue processor** (every 30s): replays locally-queued actions (archive, trash, star, label changes) by calling `gmail_*` Rust commands, then updates local DB optimistically

If a user archives a thread (TS queues the action, updates local DB optimistically) and a delta sync runs before the queue flushes (Rust fetches the thread from Gmail, sees it still in INBOX because the archive hasn't been sent yet), the sync will overwrite the local state back to "in INBOX."

### The existing solution

This problem already exists today and is already handled. `sync.ts` delta sync (line ~350 in current TS) skips threads that have pending local ops:

```typescript
const pendingOps = await getPendingOpsForResource(accountId, threadId);
if (pendingOps.length > 0) continue; // skip — local state is authoritative
```

### The rule

**The same rule must be enforced in Rust Phase 2**: before overwriting a thread's state during delta sync, query `pending_operations` for that thread. If any pending ops exist, skip the thread — the queue processor will reconcile it when the op flushes.

This is a read from the same `ratatoskr.db` that both Rust and TS write to, so it's consistent (SQLite serializes writes via the existing `Mutex<Connection>`).

### What this means concretely

- Rust sync code must call `db.with_conn(|conn| { /* check pending_operations */ })` per thread before writing
- The pending_operations table is the coordination point between TS queue writes and Rust sync writes
- No new locking mechanism is needed — SQLite's existing serialization is sufficient
- The TS queue processor's DB writes and Rust sync's DB writes never run truly concurrently against the same thread because of the pending_ops check

### Future consideration

When the queue processor itself moves to Rust (beyond Phase 3), both writers will be in Rust and can share an in-process coordination mechanism. But that's not in scope for this migration.

---

## Migration Strategy

### Per-account cutover, not per-command

Once `gmail_init_client` succeeds for an account, ALL operations for that account go through Rust. There is no mixed mode where some commands go to Rust and others to TS for the same account. This eliminates token-state races.

During development, the TS code remains in the codebase for reference and comparison. It's deleted in Phase 3 after Rust is validated.

### Testing strategy

- **Unit tests**: Rust tests for `parse.rs`, `auth_parser.rs`, type deserialization (mock JSON fixtures from real Gmail API responses)
- **Integration tests**: Tauri command tests with mock HTTP server (`wiremock-rs` or `httpmock`)
- **Existing TS tests**: Keep running until Phase 3 deletion
- **Manual testing**: Gmail account sync end-to-end after each phase
- **A/B comparison**: During Phase 1 development, can run both TS and Rust paths for the same API call and compare results (not in production — in dev/test only)

### Estimated scope

| Phase | New Rust lines (est.) | TS lines removed | Difficulty |
|-------|----------------------|-----------------|------------|
| Phase 1: Client + Commands | ~1,200-1,500 | 0 (additive) | Moderate — straightforward port of HTTP calls + serde types + token management |
| Phase 2: Sync Engine | ~1,500-2,000 | ~700 (sync.ts) | **High** — parallel fetching, DB/body/search writes, history reconciliation, pending-ops conflict check, error recovery, progress events |
| Phase 3: TS Teardown | ~0 | ~1,500 | Low — deletion and simplification |
| **Total** | **~2,700-3,500** | **~2,200** | |

Phase 2 is the riskiest and largest. It touches data integrity (DB writes, body store, search index) and correctness (sync state, conflict handling). Budget accordingly.

### Rollback

- **Phase 1**: If Rust client has issues, don't call `gmail_init_client` for that account — TS path remains functional. Per-account, not per-command.
- **Phase 2**: Keep TS sync path available behind a DB setting (`use_rust_gmail_sync`). If Rust sync has issues, flip the flag and the TS sync path runs instead (it calls Phase 1 Rust commands for API access, but orchestrates sync in TS). This is a real rollback path, not a wishful one, because Phase 1 Rust commands are individually tested.

---

## What We Defer

These are explicitly out of scope for this migration:

1. **Shared Rust `EmailProvider` trait** — extract only after Gmail + one more provider exist in Rust
2. **Provider-agnostic Tauri commands** — requires the trait; use `gmail_*` prefix for now
3. **Moving post-sync hooks to Rust** — depends on many TS services (AI, filters, smart labels, notifications)
4. **Moving queue processor to Rust** — works fine in TS, coordinates via pending_operations table
5. **Moving sync timer to Rust** — TS timer handles multi-account orchestration well
6. **Moving OAuth flow to Rust** — browser interaction is natural in TS
7. **`mail-builder` for compose** — Phase 1-3 don't change how the composer works; messages are still built in TS and passed as raw base64url. `mail-builder` integration happens when we need Rust-originated messages (e.g., Rust-side auto-reply, or when compose moves to Rust)
