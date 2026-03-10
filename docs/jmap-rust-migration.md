# JMAP → Rust Migration Plan

**Date**: March 2026
**Status**: Planning (blocked on Gmail migration completion)
**Goal**: Implement JMAP (RFC 8620/8621) as a Rust-native email provider, reusing infrastructure established by the Gmail migration. This is step 2 in the execution order from `docs/rust-provider-crate-research.md`.

---

## Table of Contents

- [Why JMAP Second](#why-jmap-second)
- [Decisions Up Front](#decisions-up-front)
- [Current State](#current-state)
- [Target State (Rust)](#target-state-rust)
- [Phase 1: Rust JMAP Provider (Client + Actions + Sync)](#phase-1-rust-jmap-provider-client--actions--sync)
- [Phase 2: TS Integration + UI](#phase-2-ts-integration--ui)
- [Thread-Level Action Semantics](#thread-level-action-semantics)
- [Sync vs Queue: Write Ordering](#sync-vs-queue-write-ordering)
- [Migration Strategy](#migration-strategy)
- [What We Defer](#what-we-defer)

---

## Why JMAP Second

1. **Stalwart alignment** — many of our users run Stalwart Mail Server. `jmap-client` is from the same ecosystem as `mail-parser` (already used) and `mail-builder` (added during Gmail migration). Same author, tested against the same server.
2. **HTTP-based like Gmail** — reuses the reqwest-middleware retry infrastructure and body store write patterns established during the Gmail migration. No TCP connection management like IMAP.
3. **Simpler sync model** — JMAP's `Email/changes` + `Mailbox/changes` with state strings is more straightforward than Gmail's History API (no HISTORY_EXPIRED edge case, no per-type history parsing). `cannotCalculateChanges` is the only fallback trigger, and it's a clean error.
4. **Native threading** — JMAP provides `threadId` on every email object. No JWZ threading algorithm needed (unlike IMAP). This simplifies the sync→DB path.
5. **Uses a real crate** — `jmap-client` handles session discovery, typed API methods, batched requests, and blob operations. We write significantly less HTTP plumbing than Gmail's hand-rolled reqwest calls.

---

## Decisions Up Front

These are not open questions. They must be settled before writing code.

### 1. Use `jmap-client` crate, vendor if needed

Unlike Gmail (hand-rolled reqwest), JMAP uses the `jmap-client` 0.4 crate from Stalwart Labs. It provides typed methods for `Email/get`, `Email/set`, `Email/query`, `Email/changes`, `Mailbox/*`, `EmailSubmission/*`, blob upload/download, and session discovery.

**Known risk**: Issue #18 — `Email/set` uses `false` instead of `null` to remove `mailboxIds`/`keywords` patch entries, violating the JMAP spec. This affects email move/archive/trash operations. If the upstream fix isn't merged by the time we implement, we vendor the crate and patch it ourselves. The fix is a one-line change in the crate's `set.rs` — serialize removal patches as `null` instead of `false`.

**Feature flags**: `default-features = false, features = ["async"]`. We don't need websockets or blocking mode.

### 2. Auth: Basic only. Bearer/OAuth deferred.

JMAP servers support two auth modes:
- **Basic** (username:password) — most self-hosted (Stalwart, Cyrus).
- **Bearer** (OAuth2 token) — Fastmail and other hosted providers.

**Phase 1 supports Basic auth only.** Reasons:

- Our target users for JMAP are self-hosted Stalwart users. They use username/password.
- Bearer/OAuth JMAP requires a provider-specific OAuth acquisition flow that doesn't exist. Fastmail has its own OAuth endpoints, token endpoint URLs, scope definitions, and app registration process. None of this is built, and `.well-known/jmap` doesn't advertise OAuth metadata.
- The `jmap-client` crate binds credentials at client construction time via `Credentials::basic(user, pass)`. Basic auth is a static credential — no refresh cycle, no token expiry, no concurrency concerns. The client is immutable after construction, which is the simplest possible design.
- Bearer support is a clean addition later: swap `Credentials::basic()` for `Credentials::bearer(token)`, add token refresh (reusing `provider/token.rs` from the Gmail migration), and rebuild the client on refresh. But this needs a real OAuth acquisition UI and per-provider endpoint configuration. That's a separate feature, not a migration concern.

For Basic auth, the password is stored encrypted in SQLite using the same AES-256-GCM encryption as IMAP passwords (`imap_password` column, `auth_method = "password"`).

### 3. No TS-orchestrated sync phase — Rust-native from day one

The Gmail migration has three phases: (1) Rust HTTP client with TS sync orchestration, (2) Rust sync engine, (3) TS teardown. Gmail needs this because there's a production TS sync engine to migrate incrementally.

JMAP has no production TS code. The upstream TS reference (`docs/jmap.md`) was never merged. There is nothing to migrate incrementally from. So JMAP skips the TS-orchestrated-sync phase entirely:

- **Phase 1** delivers the full Rust provider: client, actions, AND sync.
- **Phase 2** is TS integration (wiring up providerFactory, syncManager, emailActions, account setup UI).

This also means there is **no TS sync fallback path**. The TS JMAP implementation was never shipped — it can't serve as a rollback target. Rollback is handled differently (see [Migration Strategy](#migration-strategy)).

### 4. Commands are `jmap_*` prefixed

Same principle as Gmail — all Tauri commands are `jmap_*` prefixed. No generic routing until the shared trait is extracted (which happens after both Gmail and JMAP exist in Rust).

### 5. Mailbox → label mapping lives in Rust

The TS `mailboxMapper.ts` maps JMAP mailbox roles to Gmail-style label IDs (e.g., role `inbox` → `INBOX`, role `trash` → `TRASH`, user mailboxes → `jmap-{id}`). Keywords map to pseudo-labels (`$seen` → removes `UNREAD`, `$flagged` → `STARRED`).

This mapping lives in Rust so the sync engine can produce label IDs directly without IPC. The mapping logic is simple (a role→labelId lookup table + keyword→pseudo-label rules) and doesn't depend on any TS services.

### 6. `jmap-client` reqwest vs our reqwest-middleware

`jmap-client` 0.4 uses `reqwest` 0.12 internally with its own HTTP client. We do NOT wrap it with our `reqwest-middleware` retry layer because:
- The crate manages its own session lifecycle and error handling.
- JMAP batch requests (multiple method calls in one HTTP POST) make per-request retry semantics different from Gmail's one-endpoint-per-request pattern.
- The crate's `reqwest::Client` can be configured with our timeouts and connection settings via `ClientBuilder` passed to `jmap-client`'s constructor.

For blob upload/download (separate HTTP calls outside the JMAP API endpoint), we use our `reqwest-middleware` client directly — these are simple GET/POST calls that benefit from automatic retry.

### 7. Session state tracking

The `jmap-client` crate handles session discovery (`/.well-known/jmap` → session resource). The session contains API URL, upload/download URL templates, account ID, and capabilities.

Rust caches the session per-client. When the server returns a different `sessionState` in an API response, the crate automatically invalidates and re-fetches. We don't need to manage this ourselves.

---

## Current State

### TS reference implementation (from `docs/jmap.md`)

The upstream JMAP implementation exists as a complete TS reference (11 source + 6 test files, **never merged into production**). Key files:

| File | Lines | Purpose |
|------|-------|---------|
| `jmap/types.ts` | 128 | Full JMAP type definitions (Session, Email, Mailbox, BodyPart, Request/Response, Changes) |
| `jmap/client.ts` | 336 | `JmapClient` — session discovery, auth (Basic/Bearer), batched API calls, blob upload/download |
| `jmap/clientFactory.ts` | 33 | Creates `JmapClient` from DB account record |
| `jmap/autoDiscovery.ts` | 58 | `.well-known/jmap` discovery + known providers (Fastmail) |
| `jmap/mailboxMapper.ts` | 145 | Role-based mailbox↔label mapping, keyword↔pseudo-label mapping |
| `jmap/jmapSync.ts` | 386 | Initial sync (batched Email/query → Email/get) + delta sync (Email/changes + Mailbox/changes) |
| `email/jmapProvider.ts` | 463 | Full `EmailProvider` implementation (17 methods) |

**Total**: ~1,549 TS lines of reference. This is porting reference, not production code to migrate.

### DB schema additions needed

From the upstream migration 18:
- `accounts.jmap_url` column (TEXT, nullable) — JMAP session URL
- `jmap_sync_state` table — per-account, per-type (`Email`, `Mailbox`) state string tracking

### Integration points to wire up (Phase 2)

- `providerFactory.ts` — route `account.provider === "jmap"` to Rust-backed commands
- `syncManager.ts` — add `syncJmapAccount()` calling Rust sync commands
- `emailActions.ts` / `queueProcessor.ts` — dispatch JMAP actions to `jmap_*` Rust commands
- `AddJmapAccount.tsx` — account setup UI (3-step: email/password → auto-discover → test connection)

---

## Target State (Rust)

### Module structure

```
src-tauri/src/jmap/
├── mod.rs              # Re-exports
├── types.rs            # Supplementary types (where jmap-client types need adaptation)
├── client.rs           # JmapState, client lifecycle (init/remove), Basic auth credential
├── mailbox_mapper.rs   # Role→label mapping, keyword→pseudo-label mapping
├── parse.rs            # jmap-client Email → internal message types (for DB persistence)
├── sync.rs             # Initial sync + delta sync
└── auto_discovery.rs   # .well-known/jmap + known provider list
```

### Infrastructure reused from Gmail migration

These already exist in `provider/` after the Gmail migration:

| Module | What JMAP uses |
|--------|---------------|
| `provider/http.rs` | `build_api_client()` — for blob upload/download retries (not for core JMAP API calls) |
| `provider/message.rs` | `mail-builder` RFC 5322 construction — for `Email/import` (send/draft) |

Note: `provider/token.rs` is NOT used in Phase 1 (Basic auth is static). It will be used when Bearer/OAuth JMAP is added later.

### Tauri command surface (`jmap_*` prefixed)

```rust
// Lifecycle
jmap_init_client(account_id)
jmap_remove_client(account_id)
jmap_test_connection(account_id)
jmap_discover_url(email)

// Sync
jmap_sync_initial(account_id, days_back)
jmap_sync_delta(account_id)

// Folder (mailbox) operations
jmap_list_folders(account_id)
jmap_create_folder(account_id, name, parent_id?)
jmap_rename_folder(account_id, folder_id, new_name)
jmap_delete_folder(account_id, folder_id)

// Email actions (called by TS queue processor)
// Note: thread-level actions (archive, trash, star, etc.) accept a thread_id
// and internally enumerate emails via Email/query. See "Thread-Level Action
// Semantics" section for the full design and edge cases.
jmap_archive(account_id, thread_id)
jmap_trash(account_id, thread_id)
jmap_permanent_delete(account_id, email_ids)
jmap_mark_read(account_id, thread_id, read)
jmap_star(account_id, thread_id, starred)
jmap_spam(account_id, thread_id, is_spam)
jmap_move_to_folder(account_id, thread_id, folder_id)
jmap_add_label(account_id, thread_id, label_id)
jmap_remove_label(account_id, thread_id, label_id)

// Send + drafts
jmap_send_email(account_id, raw_base64url, thread_id?)
jmap_create_draft(account_id, raw_base64url, thread_id?)
jmap_update_draft(account_id, draft_id, raw_base64url, thread_id?)
jmap_delete_draft(account_id, draft_id)

// Attachments
jmap_fetch_attachment(account_id, email_id, blob_id)

// Profile
jmap_get_profile(account_id)
```

~23 commands. Thread-level actions accept `thread_id` to match the existing TS queue contract (see [Thread-Level Action Semantics](#thread-level-action-semantics)).

---

## Phase 1: Rust JMAP Provider (Client + Actions + Sync)

**Goal**: Build the complete JMAP provider in Rust — client, all email actions, AND sync engine. Unlike Gmail, there is no intermediate "TS orchestrates sync via Rust HTTP commands" phase because there is no existing TS sync code to migrate from.

**Prerequisite**: Gmail migration is far enough that `provider/http.rs` and `provider/message.rs` exist. Gmail does NOT need to be fully complete — JMAP doesn't depend on Gmail's sync code, only the shared infrastructure.

### 1a. DB migration

Add to `src/services/db/migrations.ts`:

```sql
-- Migration N (after current latest)
ALTER TABLE accounts ADD COLUMN jmap_url TEXT;

CREATE TABLE jmap_sync_state (
    account_id TEXT NOT NULL,
    type TEXT NOT NULL,           -- 'Email' or 'Mailbox'
    state TEXT NOT NULL,          -- JMAP state string
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    PRIMARY KEY (account_id, type),
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);
```

Also add `jmap_url` to the Rust `db/types.rs` `DbAccount` struct and the account query/insert SQL.

### 1b. JMAP client state (`jmap/client.rs`)

```rust
use jmap_client::client::Client as JmapClientInner;
use jmap_client::client::Credentials;

/// Wraps jmap-client's Client. For Basic auth, the client is fully immutable
/// after construction — no token refresh, no credential mutation.
pub struct JmapClient {
    inner: JmapClientInner,
    account_id: String,
}

impl JmapClient {
    /// Create from DB account record. Performs session discovery.
    /// Currently only supports Basic auth (password accounts).
    pub async fn from_account(db: &DbState, account_id: &str) -> Result<Self, String> {
        // 1. Load account from DB
        // 2. Decrypt password from imap_password column
        // 3. Build credentials: Credentials::basic(&email, &password)
        // 4. Create jmap-client Client with session URL from account.jmap_url
        // 5. Session discovery happens automatically on first API call
    }

    /// Direct access to the underlying jmap-client Client.
    /// For Basic auth this is trivial — the client is immutable.
    pub fn inner(&self) -> &JmapClientInner { &self.inner }

    pub fn account_id(&self) -> &str { &self.account_id }
}

/// Tauri-managed state.
pub struct JmapState {
    clients: RwLock<HashMap<String, JmapClient>>,
}

impl JmapState {
    pub fn new() -> Self { Self { clients: RwLock::new(HashMap::new()) } }

    pub async fn get(&self, account_id: &str) -> Result<&JmapClient, String> {
        // Read lock, return reference. Error if not initialized.
    }

    pub async fn insert(&self, account_id: String, client: JmapClient) {
        // Write lock, insert.
    }

    pub async fn remove(&self, account_id: &str) {
        // Write lock, remove.
    }
}
```

**Key simplification vs Gmail**: No `Arc<RwLock<TokenState>>`, no concurrent refresh coalescing, no `Shared<BoxFuture>`. Basic auth means the credential is baked into `jmap-client`'s internal `reqwest::Client` at construction time and never changes. The `JmapClient` struct is plain and immutable.

When Bearer auth is added later, the design will need to change: either rebuild the `JmapClientInner` on token refresh (since `jmap-client` binds credentials at construction), or patch the crate to accept a credential callback. That design decision is deferred until Bearer is actually needed.

### 1c. Auto-discovery (`jmap/auto_discovery.rs`)

Port of `autoDiscovery.ts`. Exposed as a Tauri command for the account setup UI:

```rust
const KNOWN_PROVIDERS: &[(&str, &str)] = &[
    ("fastmail.com", "https://api.fastmail.com/jmap/session"),
    ("messagingengine.com", "https://api.fastmail.com/jmap/session"),
];

pub struct JmapDiscoveryResult {
    pub session_url: String,
    pub source: String,  // "well-known" | "known-provider"
}

pub async fn discover_jmap_url(email: &str) -> Option<JmapDiscoveryResult> {
    let domain = email.split('@').nth(1)?.to_lowercase();

    // Check known providers
    if let Some(&(_, url)) = KNOWN_PROVIDERS.iter().find(|&&(d, _)| d == domain) {
        return Some(JmapDiscoveryResult {
            session_url: url.to_string(),
            source: "known-provider".into(),
        });
    }

    // Try .well-known/jmap
    let well_known = format!("https://{}/.well-known/jmap", domain);
    if let Ok(resp) = reqwest::get(&well_known).await {
        if resp.status().is_success() {
            return Some(JmapDiscoveryResult {
                session_url: well_known,
                source: "well-known".into(),
            });
        }
    }

    None
}
```

### 1d. Mailbox mapper (`jmap/mailbox_mapper.rs`)

Port of `mailboxMapper.ts`. Pure functions, no I/O:

```rust
use std::collections::HashMap;

const ROLE_MAP: &[(&str, &str, &str)] = &[
    //  (jmap_role,  label_id,    label_name)
    ("inbox",     "INBOX",     "Inbox"),
    ("archive",   "archive",   "Archive"),
    ("drafts",    "DRAFT",     "Drafts"),
    ("sent",      "SENT",      "Sent"),
    ("trash",     "TRASH",     "Trash"),
    ("junk",      "SPAM",      "Spam"),
    ("important", "IMPORTANT", "Important"),
];

pub struct MailboxLabelMapping {
    pub label_id: String,
    pub label_name: String,
    pub label_type: &'static str,  // "system" or "user"
}

/// Map a JMAP mailbox to a Gmail-style label ID.
pub fn map_mailbox_to_label(role: Option<&str>, mailbox_id: &str, name: &str) -> MailboxLabelMapping;

/// Derive label IDs from an email's mailbox membership and keywords.
pub fn get_labels_for_email(
    mailbox_ids: &HashMap<String, bool>,
    keywords: &HashMap<String, bool>,
    mailbox_map: &HashMap<String, (Option<String>, String)>,  // id → (role, name)
) -> Vec<String>;

/// Reverse lookup: Gmail-style label ID → JMAP mailbox ID.
pub fn label_id_to_mailbox_id(
    label_id: &str,
    mailboxes: &[(String, Option<String>, String)],  // (id, role, name)
) -> Option<String>;
```

### 1e. Message parsing (`jmap/parse.rs`)

Port of `jmapEmailToParsedMessage()`. Converts `jmap-client`'s `Email` response to our internal DB-ready struct:

```rust
pub fn parse_jmap_email(
    email: &jmap_client::email::Email,
    mailbox_map: &HashMap<String, (Option<String>, String)>,
) -> Result<ParsedJmapMessage, String>;
```

**JMAP vs Gmail parsing differences**:
- No base64 decoding — JMAP returns body values as UTF-8 strings via `fetchHTMLBodyValues`/`fetchTextBodyValues`.
- No recursive MIME part walking — JMAP flattens `textBody`, `htmlBody`, `attachments` for us.
- No `Authentication-Results` header — JMAP doesn't expose transport-level auth headers. `auth_results` field will be NULL for JMAP messages.
- `messageId`, `inReplyTo`, `references` are typed arrays (not raw header strings to parse).
- `threadId` is provided natively (no JWZ threading needed).

Output struct matches the shape written to the `messages` DB table + body store.

### 1f. Sync engine (`jmap/sync.rs`)

```rust
/// Initial sync: mailboxes → batched Email/query + Email/get → DB writes.
pub async fn jmap_initial_sync(
    client: &JmapClient,
    account_id: &str,
    days_back: i64,
    db: &DbState,
    body_store: &BodyStoreState,
    search: &SearchState,
    app_handle: &AppHandle,
) -> Result<(), String>;

/// Delta sync: Email/changes + Mailbox/changes → targeted re-fetch → DB writes.
/// Returns new inbox email IDs for TS post-sync hooks.
pub async fn jmap_delta_sync(
    client: &JmapClient,
    account_id: &str,
    db: &DbState,
    body_store: &BodyStoreState,
    search: &SearchState,
    app_handle: &AppHandle,
) -> Result<JmapDeltaSyncResult, String>;

pub struct JmapDeltaSyncResult {
    pub new_inbox_email_ids: Vec<String>,
    pub affected_thread_ids: Vec<String>,
}
```

**Initial sync** (2-phase — simpler than Gmail's 3-phase):

1. **Mailboxes**: `Mailbox/get` → persist as labels via mailbox mapper → store `Mailbox` state string in `jmap_sync_state`
2. **Emails**: Paginated `Email/query` (filter: `after: sinceDate`, sort: `receivedAt DESC`) → batched `Email/get` (50 per batch, with `fetchHTMLBodyValues` + `fetchTextBodyValues`) → for each email:
   - `parse_jmap_email()` → `ParsedJmapMessage`
   - DB writes: `upsert_thread()`, `set_thread_labels()`, `upsert_message()` via `DbState`
   - Body writes: `body_store_put()` via `BodyStoreState` (bodies come inline, no extra fetch)
   - Search index: `index_message()` via `SearchState`
   - Attachment writes: `upsert_attachment()` (blob IDs stored for later download)
3. Store final `Email` state string to `jmap_sync_state`

**Delta sync**:

1. `Mailbox/changes(sinceState)` — if state changed, re-fetch all mailboxes and update labels
   - On `cannotCalculateChanges` error → full mailbox refresh
2. `Email/changes(sinceState)` — returns `created`, `updated`, `destroyed` ID lists
   - Loop while `hasMoreChanges`
   - Batch-fetch `created` + `updated` IDs via `Email/get` (50 per batch)
   - For each fetched email, resolve its `threadId` and check `pending_operations` for that thread (see [Sync vs Queue: Write Ordering](#sync-vs-queue-write-ordering))
   - Parse and persist (same path as initial sync)
   - Delete `destroyed` emails from local DB
   - Update state string
3. On `cannotCalculateChanges` → return error code `JMAP_STATE_EXPIRED`, TS triggers full sync

**No parallel fetch needed**: Unlike Gmail where we fetch individual threads by ID (concurrency=10), JMAP's `Email/get` accepts a batch of IDs in a single request. The server returns all requested emails in one response. Batches of 50 IDs are sequential — parallelism is unnecessary because the batching is server-side.

**Progress reporting**: Same Tauri event pattern as Gmail/IMAP:
```rust
app_handle.emit("jmap-sync-progress", &JmapSyncProgress {
    account_id, phase, current, total
})?;
```

### 1g. Email action commands

Each action maps to `jmap-client` API calls. Thread-level actions enumerate emails first (see [Thread-Level Action Semantics](#thread-level-action-semantics)).

**Archive** — enumerate thread emails, remove from inbox, add to archive:
```rust
#[tauri::command]
pub async fn jmap_archive(
    account_id: String, thread_id: String,
    db: State<'_, DbState>, jmap: State<'_, JmapState>,
) -> Result<(), String> {
    let client = jmap.get(&account_id).await?;
    let inner = client.inner();

    // 1. Find inbox and archive mailbox IDs from cached mailboxes
    let inbox_id = /* resolve from mailbox cache */;
    let archive_id = /* resolve from mailbox cache, optional */;

    // 2. Enumerate emails in thread
    let email_ids = query_thread_email_ids(inner, &thread_id).await?;

    // 3. Batch Email/set on all emails
    let mut request = inner.build();
    let set_request = request.email_set();
    for email_id in &email_ids {
        let update = set_request.update(email_id);
        update.mailbox_id(&inbox_id, false);  // remove from inbox
        if let Some(ref archive) = archive_id {
            update.mailbox_id(archive, true);  // add to archive
        }
    }
    request.send().await.map_err(|e| e.to_string())?;
    Ok(())
}
```

**Send** — upload blob + Email/import + EmailSubmission/set in one batch:
```rust
#[tauri::command]
pub async fn jmap_send_email(
    account_id: String, raw_base64url: String, thread_id: Option<String>,
    db: State<'_, DbState>, jmap: State<'_, JmapState>,
) -> Result<JmapSendResult, String> {
    let client = jmap.get(&account_id).await?;
    let inner = client.inner();

    // Decode base64url → raw RFC 822 bytes
    let raw_bytes = base64_url_decode(&raw_base64url)?;

    // Upload blob
    let blob_id = inner.upload(None, raw_bytes, None).await
        .map_err(|e| e.to_string())?
        .blob_id();

    // Batch: Email/import + EmailSubmission/set
    let mut request = inner.build();
    let import_ref = request.email_import()
        .email(blob_id)
        .keyword("$seen", true)
        .create_id("draft1");
    let submission = request.email_submission_create()
        .email_id_reference(import_ref)
        .create_id("sub1");
    // On success, remove $draft keyword
    request.email_set()
        .on_success_update_email(submission)
        .keyword("$draft", false)
        .keyword("$seen", true);

    let response = request.send().await.map_err(|e| e.to_string())?;
    // Extract created email ID from response
    Ok(JmapSendResult { id: /* ... */ })
}
```

**Note on Issue #18**: The `update.mailbox_id(&id, false)` call on `jmap-client` may produce `"mailboxIds/xxx": false` instead of `"mailboxIds/xxx": null` in the JSON. If this breaks against Stalwart or other servers, we vendor the crate and fix `Email/set` serialization.

### 1h. Draft operations

JMAP has no draft mutation — update = delete old + create new:

```rust
#[tauri::command]
pub async fn jmap_update_draft(
    account_id: String, draft_id: String, raw_base64url: String,
    thread_id: Option<String>,
    db: State<'_, DbState>, jmap: State<'_, JmapState>,
) -> Result<JmapDraftResult, String> {
    // Delete old draft
    jmap_delete_draft_inner(&account_id, &draft_id, &db, &jmap).await?;
    // Create new draft
    jmap_create_draft_inner(&account_id, &raw_base64url, thread_id.as_deref(), &db, &jmap).await
}
```

### 1i. Attachment download

Blob download via `jmap-client`'s `download()` method:

```rust
#[tauri::command]
pub async fn jmap_fetch_attachment(
    account_id: String, _email_id: String, blob_id: String,
    db: State<'_, DbState>, jmap: State<'_, JmapState>,
) -> Result<AttachmentData, String> {
    let client = jmap.get(&account_id).await?;
    let data = client.inner().download(&blob_id).await
        .map_err(|e| e.to_string())?;
    Ok(AttachmentData {
        data: BASE64_STANDARD.encode(&data),
        size: data.len(),
    })
}
```

### 1j. Tauri state registration

In `lib.rs`:
```rust
.manage(JmapState::new())
```

Register all `jmap_*` commands in the `.invoke_handler()` list.

### Phase 1 deliverable

The complete JMAP provider exists in Rust: client, all email actions, sync engine. All JMAP HTTP calls go through `jmap-client`. Auth is Basic (password), credentials are static and Rust-owned. Sync writes directly to DB, body store, and search index. But no TS wiring yet — Phase 2 connects the UI.

---

## Phase 2: TS Integration + UI

**Goal**: Wire the Rust JMAP provider into the TS application layer. This is the thin glue code.

### 2a. Account setup UI

**`AddJmapAccount.tsx`** — new component, 3-step flow:

1. **Email + password**: User enters email address and password (Basic auth only).
2. **Auto-discover**: Call `jmap_discover_url` Tauri command. If found, pre-fill. If not, user enters JMAP session URL manually.
3. **Test connection**: Call `jmap_test_connection`. On success, save account to DB with `provider = "jmap"`, `auth_method = "password"`, `jmap_url = sessionUrl`, encrypted password in `imap_password` column.

### 2b. Provider factory + email actions

- **`providerFactory.ts`**: Route `account.provider === "jmap"` to a thin `JmapProvider` that delegates all methods to `jmap_*` Tauri commands.
- **`emailActions.ts`**: Add JMAP cases in the action dispatcher. Thread-level actions (archive, trash, star, etc.) pass `threadId` to Rust — Rust handles email enumeration internally.
- **`queueProcessor.ts`**: JMAP action cases call `jmap_*` Tauri commands. The `resource_id` in `pending_operations` remains a `threadId` (same as Gmail/IMAP).

### 2c. Sync manager

Add `syncJmapAccount()` to `syncManager.ts`:

```typescript
async function syncJmapAccount(accountId: string) {
  // Check if we have a sync state (stored in Rust-managed jmap_sync_state table)
  // For initial check, try delta first — if it fails with JMAP_STATE_EXPIRED
  // or JMAP_NO_STATE, fall back to initial sync.
  try {
    const result = await invoke('jmap_sync_delta', { accountId });
    // Post-sync hooks (still TS)
    await applyFiltersToNewMessageIds(accountId, result.newInboxEmailIds);
    await applySmartLabelsToNewMessageIds(accountId, result.newInboxEmailIds);
    // ... notifications, categorization
  } catch (err) {
    const msg = typeof err === 'string' ? err : '';
    if (msg === 'JMAP_STATE_EXPIRED' || msg === 'JMAP_NO_STATE') {
      await invoke('jmap_sync_initial', { accountId, daysBack: syncDays });
    } else throw err;
  }
}
```

### 2d. App startup

In `App.tsx` startup sequence:
- `getAllAccounts()` → for JMAP accounts, call `jmap_init_client` (same pattern as `gmail_init_client` for Gmail).
- Sync timer already handles multi-provider via `syncManager.ts` routing.

### Phase 2 deliverable

JMAP accounts can be added, synced, and acted on through the full UI. The sync timer includes JMAP accounts. Email actions work through the offline queue. Account setup UI guides users through discovery and connection testing.

---

## Thread-Level Action Semantics

This is a product-level design decision that must be explicit, not hidden in adapter code.

### The problem

Our app's action model is thread-centric: archive a thread, star a thread, trash a thread. This matches Gmail's API, which natively supports thread-level operations. The TS queue stores `threadId` as the `resource_id` for all thread actions (see `emailActions.ts:262` — `getResourceId()` returns `action.threadId`).

JMAP has no thread-level mutations. Mailbox membership and keywords are per-email, not per-thread. `Email/set` operates on individual email IDs.

### The design

Thread-level JMAP actions enumerate emails in the thread and mutate each one:

```rust
/// Shared helper: get all email IDs in a thread.
async fn query_thread_email_ids(
    client: &JmapClientInner,
    thread_id: &str,
) -> Result<Vec<String>, String> {
    // Email/query with filter: { inThread: thread_id }
    // Returns all email IDs belonging to this thread.
}
```

Every thread-level action (`jmap_archive`, `jmap_trash`, `jmap_star`, `jmap_mark_read`, `jmap_spam`, `jmap_move_to_folder`, `jmap_add_label`, `jmap_remove_label`) calls this helper first, then applies `Email/set` to all returned email IDs in a single batch request.

### Edge case: new messages arriving after action

If a new message arrives in a thread between the time the user triggers an action and the queue processes it, that message will NOT be affected by the action. This is the same behavior as Gmail when using per-message `modify()` calls — the action applies to messages that existed at execution time.

This is acceptable because:
- The next delta sync will reconcile state.
- For critical actions (trash, archive), the user will notice if new messages appear and can act again.
- JMAP's `Email/query(inThread)` at execution time captures the correct snapshot.

### Why not per-email actions in the queue?

We could change the queue to store individual email IDs instead of thread IDs for JMAP. We don't because:
- The optimistic UI update model is thread-centric (ThreadStore, UI shows threads).
- `pending_operations.resource_id` being a `threadId` is what delta sync checks to skip conflicting writes. Changing this would break the conflict coordination model.
- The per-thread abstraction is the right product behavior — users think in threads.

---

## Sync vs Queue: Write Ordering

Same principle as Gmail — see `docs/gmail-rust-migration.md` for the full explanation.

### The rule

**Before overwriting an email's state during delta sync, resolve its `threadId` and check `pending_operations` for that thread. If any pending ops exist for the thread, skip all emails in that thread — the queue processor will reconcile when the op flushes.**

### Why thread-level, not email-level

The existing queue contract is thread-centric. `emailActions.ts:262` always stores `threadId` as `resource_id` via `getResourceId()`. The sync conflict check in `sync.ts:424` queries `getPendingOpsForResource(accountId, threadId)`.

JMAP `Email/changes` returns individual email IDs (not thread IDs). But the pending-ops check MUST be by thread, because that's what the queue stores. The delta sync flow is:

1. `Email/changes` → get list of changed email IDs
2. `Email/get` on changed IDs → each response includes `threadId`
3. For each email, check `pending_operations WHERE resource_id = email.threadId`
4. If pending ops exist for the thread, skip ALL emails in that thread
5. Otherwise, persist normally

This is a read from the same `ratatoskr.db` that both Rust and TS write to, consistent via SQLite's `Mutex<Connection>` serialization.

---

## Migration Strategy

### Per-account cutover

Same as Gmail: once `jmap_init_client` succeeds, all operations for that JMAP account go through Rust. No mixed mode.

### Rollback strategy

There is no TS JMAP fallback — the TS reference was never shipped. Rollback is Rust-only:

- **Account-level disable**: If the JMAP provider has a critical bug, users can remove and re-add the account as IMAP (if the server supports both). This is a manual workaround, not an automated fallback.
- **Feature flag in Rust**: The `jmap_sync_initial` / `jmap_sync_delta` commands can check a DB setting (`jmap_sync_enabled`) and return early. This lets us ship the provider with a kill switch — if sync is broken in production, disable it via settings without a code change.
- **Incremental rollout**: JMAP only activates for accounts explicitly added as `provider = "jmap"`. Existing IMAP accounts are not affected. Users opt in by adding a JMAP account.

This is more conservative than Gmail's rollback (which has a real TS fallback path). The trade-off is justified: JMAP is a new provider, not a migration of an existing one. There's no user data at risk from a sync bug in a newly-added account — the worst case is "sync doesn't work yet, remove the account and re-add as IMAP."

### Testing strategy

- **Unit tests**: Rust tests for `mailbox_mapper.rs`, `parse.rs`, type deserialization (mock JSON from real JMAP servers — Stalwart responses)
- **Integration tests**: Tauri command tests with mock HTTP server serving JMAP responses (`wiremock-rs`)
- **Stalwart local testing**: Stand up a local Stalwart instance via Docker for end-to-end sync testing. This is the primary integration test target.
- **Manual testing**: JMAP account setup → initial sync → delta sync → archive/trash/star/send → verify round-trip

### Estimated scope

| Phase | New Rust lines (est.) | New TS lines | Difficulty |
|-------|----------------------|-------------|------------|
| Phase 1: Full Rust provider | ~1,400-1,800 | ~0 | Moderate — `jmap-client` does the heavy HTTP lifting, but sync + actions + parse all land at once |
| Phase 2: TS integration + UI | ~0 | ~500 (UI + thin adapters + sync wiring) | Low — thin glue code |
| **Total** | **~1,400-1,800** | **~500** | |

Smaller than Gmail migration (~2,700-3,500 Rust lines) because:
1. `jmap-client` handles HTTP, session management, and typed API construction
2. No parallel thread fetch complexity (JMAP batches server-side)
3. Simpler delta sync (state strings vs History API)
4. Bodies come inline (no separate fetch/decode step)
5. No auth header parsing (JMAP doesn't expose `Authentication-Results`)

---

## What We Defer

1. **Bearer/OAuth JMAP support** — requires per-provider OAuth endpoint configuration (Fastmail has its own OAuth URLs and scopes), an OAuth acquisition UI flow, and a client rebuild-on-refresh strategy (since `jmap-client` binds credentials at construction). Add when a concrete hosted JMAP provider is needed. Design considerations: either rebuild the `JmapClientInner` on each token refresh, or patch `jmap-client` to accept a credential callback.
2. **Shared Rust `EmailProvider` trait** — after JMAP is fully complete (both phases — Rust provider AND TS integration validated in the real app), extract the trait from real code. Both Gmail and JMAP must be validated end-to-end before the abstraction is stable enough for Graph to build against. See the strategic plan in `docs/rust-provider-crate-research.md`.
3. **Provider-agnostic Tauri commands** — depends on the trait extraction above.
4. **JMAP push notifications** — `jmap-client` supports WebSocket push (`EventSource`). Could replace polling for real-time sync. Defer until basic sync is solid.
5. **JMAP Sieve filter management** — `jmap-client` supports full Sieve CRUD. Could expose server-side filter management to UI. Not in initial scope.
6. **List-Unsubscribe from raw headers** — JMAP's `Email/get` can fetch arbitrary headers via `header:List-Unsubscribe:asText`. Add after basic sync works.
7. **Authentication-Results** — JMAP doesn't standardize this. Some servers may expose it via `header:Authentication-Results`. Investigate after basic sync.
8. **JMAP for Calendars** — `jmap-client` has no calendar support (Issue #3). Not blocking for email.

### Trait extraction trigger

When JMAP is fully complete (both Phase 1 and Phase 2 validated), both Gmail and JMAP exist as Rust providers with:
- Client lifecycle (`init`/`remove`)
- Sync (initial + delta)
- Email actions (archive, trash, star, read, labels, move, delete)
- Send + drafts
- Attachment download

At that point, look at what actually overlaps between `src-tauri/src/gmail/` and `src-tauri/src/jmap/`. Extract a `trait EmailProvider` from the real code. This will inform how Microsoft Graph (step 3) is structured from the start.
