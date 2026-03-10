# Microsoft Graph → Rust Migration Plan

**Date**: March 2026
**Status**: Deferred (blocked on JMAP completion + trait extraction)
**Goal**: Implement Microsoft Graph Mail API as a Rust-native email provider. This is step 3 in the execution order from `docs/rust-provider-crate-research.md`.

Unlike Gmail (migrating existing TS → Rust) and JMAP (new provider, no production TS code), Graph is the first provider built **against an existing shared Rust trait**. The `EmailProvider` trait will be extracted from Gmail + JMAP after JMAP Phase 1 completes. Graph's implementation should validate that trait — if Graph can implement it cleanly, the abstraction is correct. If not, it exposes leaks that need fixing before a fourth provider.

---

## Table of Contents

- [Why Graph Third](#why-graph-third)
- [Prerequisites](#prerequisites)
- [Known Decisions](#known-decisions)
- [Open Questions](#open-questions)
- [Key Differences from Gmail and JMAP](#key-differences-from-gmail-and-jmap)
- [Current State](#current-state)
- [Target State (Rust)](#target-state-rust)
- [Phase 1: Rust Graph Provider (Client + Actions + Sync)](#phase-1-rust-graph-provider-client--actions--sync)
- [Phase 2: TS Integration + UI](#phase-2-ts-integration--ui)
- [Thread-Level Action Semantics](#thread-level-action-semantics)
- [Folder-to-Label Mapping](#folder-to-label-mapping)
- [Per-Folder Delta Sync Design](#per-folder-delta-sync-design)
- [Sync vs Queue: Write Ordering](#sync-vs-queue-write-ordering)
- [Migration Strategy](#migration-strategy)
- [What We Defer](#what-we-defer)

---

## Why Graph Third

1. **Depends on trait extraction** — Graph is the first provider that should be built AGAINST the shared `EmailProvider` trait extracted from Gmail + JMAP. Building it before that trait exists would mean another one-off implementation to refactor.
2. **OAuth infrastructure must be multi-provider first** — Graph requires Microsoft OAuth2 (Entra ID, `/common/oauth2/v2.0/` endpoints). The existing `oauth.rs` is Google-specific. It needs to be generalized for at least two providers before Graph can use it. This generalization should happen naturally during Gmail Rust migration, but must be verified.
3. **Folder-centric model is the hardest to reconcile** — Gmail is label-centric (messages have multiple labels). JMAP uses mailboxes (messages can belong to multiple mailboxes). Graph is folder-centric (messages live in exactly one folder). This is the most restrictive model and the hardest to map onto our Gmail-style label UI. Seeing how the trait handles Gmail labels vs JMAP mailboxes will inform how Graph folders fit.
4. **Lower priority user base** — Outlook.com/Exchange users can already connect via IMAP+OAuth2 (see quick win in [What We Defer](#what-we-defer)). Graph adds richer features (categories, delta sync, focused inbox) but is not a blocker for basic access.

---

## Prerequisites

These must be complete or in progress before Graph work begins:

### 1. Gmail Rust migration (complete)

All Gmail API logic lives in Rust. Patterns established:
- `provider/token.rs` — token refresh, coalesced via `Shared<BoxFuture>`, parameterized by endpoint
- `provider/http.rs` — `build_api_client()` with reqwest-middleware retry
- `provider/message.rs` — `mail-builder` RFC 5322 construction
- `GmailState` — `RwLock<HashMap<String, GmailClient>>` pattern for Tauri-managed state
- Sync progress events — `app_handle.emit("*-sync-progress", ...)`
- Pending-ops conflict check in delta sync

### 2. JMAP Rust migration (complete)

JMAP provider exists in Rust. Patterns established:
- Mailbox/folder → label mapping in Rust (`jmap/mailbox_mapper.rs`)
- Thread-level action semantics — enumerate emails, batch-mutate (JMAP has no thread-level mutations, same as Graph)
- `JmapState` — same `RwLock<HashMap>` pattern
- Basic auth client lifecycle (Graph uses OAuth, not Basic, but the state management pattern transfers)

### 3. Shared `EmailProvider` trait extracted

The trait exists, extracted from real Gmail + JMAP code. Graph is the first provider built against it. The trait should cover:
- Client lifecycle (`init`/`remove`)
- Sync (initial + delta)
- Email actions (archive, trash, star, read, labels, move, delete)
- Send + drafts
- Attachment download

If the trait is NOT ready when Graph work begins, Graph becomes another one-off `graph_*`-prefixed provider, and we'll have three providers to reconcile later. This is explicitly the scenario we're trying to avoid.

### 4. OAuth generalized for multi-provider

`oauth.rs` must support at least Google and Microsoft endpoints. The flow is identical (PKCE + localhost redirect), only endpoints, scopes, and token endpoint URLs differ. This should already be done — verify before starting Graph.

---

## Known Decisions

These carry forward from `docs/rust-provider-crate-research.md` and `docs/microsoft-exchange-assessment.md`:

### 1. Hand-roll on reqwest, no `graph-rs-sdk`

~18 REST endpoints. `graph-rs-sdk` covers the entire Graph API surface (not just Mail), has single-maintainer risk (last commit Aug 2025), and brings its own OAuth layer. Not worth the dependency. Same rationale as Gmail's rejection of `google-gmail1`.

### 2. Microsoft Graph API, not EWS

EWS is deprecated for Exchange Online (Oct 2026 block, Apr 2027 permanent removal). Graph works with both Exchange Online and personal Outlook.com/Hotmail accounts. REST/JSON vs SOAP/XML. No contest.

### 3. OAuth2 via Entra ID (formerly Azure AD)

- Authorization Code + PKCE flow, same pattern as Gmail
- Token endpoint: `https://login.microsoftonline.com/common/oauth2/v2.0/token`
- Auth endpoint: `https://login.microsoftonline.com/common/oauth2/v2.0/authorize`
- Scopes: `Mail.ReadWrite`, `Mail.Send`, `MailboxSettings.ReadWrite`, `offline_access`
- Localhost redirect (port 17248-17251, same server as Gmail)
- Multi-tenant + personal accounts (use `/common` tenant)

### 4. Commands: `graph_*` prefixed OR provider-agnostic

If the shared `EmailProvider` trait and provider-agnostic Tauri commands are ready, Graph is the first provider to use them directly. In that case, TS calls generic commands (e.g., `provider_sync_delta(accountId)`) and Rust routes internally based on `account.provider`.

If the trait isn't ready, fall back to `graph_*` prefixed commands — same pattern as `gmail_*` and `jmap_*`.

### 5. Delta sync per folder, not global

Graph's delta endpoint is per-folder: `GET /me/mailFolders/{id}/messages/delta`. Returns `@odata.deltaLink` for next sync. Delta tokens don't expire (unlike Gmail's ~30-day History API window). Must track delta tokens per folder in DB — similar to IMAP's per-folder UIDVALIDITY tracking but simpler (no UIDVALIDITY invalidation, just token updates).

### 6. On-premises Exchange is out of scope

On-prem Exchange supports IMAP — users can connect via our existing IMAP provider. EWS for on-prem is niche and the SOAP/XML complexity isn't justified. If demand emerges, revisit later with `ews-rs` types from Thunderbird.

### 7. Token management reuses `provider/token.rs`

Graph's OAuth2 token refresh is functionally identical to Gmail's — same PKCE flow, same refresh token exchange, different endpoints. The `GraphClient` wraps `Arc<RwLock<TokenState>>` with Microsoft-specific endpoint configuration, reusing the refresh infrastructure from the Gmail migration.

The key difference from JMAP: JMAP Phase 1 uses Basic auth (static, no refresh). Graph ALWAYS uses OAuth2 (dynamic tokens, refresh cycle). Graph's client lifecycle is closer to Gmail's — `Arc<RwLock<TokenState>>` with concurrent refresh coalescing — not JMAP's immutable client.

### 8. No `mail-builder` for send — Graph takes JSON

Graph's `/me/sendMail` accepts a JSON `Message` object, not raw RFC 822. This means `provider/message.rs` (`mail-builder`) is NOT used for sending via Graph. Instead, Graph needs its own message-to-JSON serializer.

`mail-builder` may still be useful if we discover that sending raw MIME via Graph works reliably (the `Content-Type: text/plain` raw MIME path is undocumented). But the default path is JSON send. See [Open Question 6](#6-send-format).

---

## Open Questions

These must be resolved before writing final implementation code:

### 1. App registration model

Gmail uses user-provided Client IDs (configured in Settings). Microsoft Graph requires an Azure AD app registration. Options:
- **Ship a default app registration** — simpler for users, but we'd need to manage it (including publisher verification for organizational accounts).
- **User provides their own** — same as Gmail, but Azure portal is more complex than Google Cloud Console.
- **Both** — ship a default for personal accounts, allow override for organizational.

### 2. Folder-centric to label-centric mapping

Graph messages live in exactly one folder. Our UI is label-centric (threads can have multiple labels). See [Folder-to-Label Mapping](#folder-to-label-mapping) for detailed analysis. This is a product decision, not just an adapter detail.

### 3. Thread model

Graph has a `conversationId` field that groups related messages, but it's not as reliable as Gmail's threading. Graph also has `conversationIndex` (binary threading data from Exchange). Options:
- Use `conversationId` as `threadId` (simplest, may produce different groupings than users expect).
- Use our JWZ threading algorithm on `Message-ID`/`References`/`In-Reply-To` headers (more accurate, more work — reusing `src/services/threading/threadBuilder.ts` logic).
- Use `conversationId` as primary, fall back to JWZ for edge cases.

Investigate `conversationId` reliability across real Outlook.com and Exchange Online accounts before deciding.

### 4. Rate limit handling

Graph allows only **4 concurrent requests per app per mailbox**. This is far more restrictive than Gmail (10+ parallel) and changes the sync architecture:

- Gmail: parallel `getThread()` at concurrency=10 via Semaphore
- JMAP: server-side batching (50 IDs per request), no parallel fetch needed
- Graph: max 2-3 concurrent requests (leave headroom for user-initiated actions during sync)

The sync engine must use `tokio::sync::Semaphore` with a much lower permit count. Per-folder delta sync is already serial by nature (one delta query per folder, paginated), but initial sync fetching individual messages needs throttling.

Additionally: 10,000 API requests per 10 minutes per app per mailbox. This is generous for delta sync but could be hit during large initial syncs. Track request count and back off if approaching the limit.

### 5. Shared trait readiness

By the time Graph starts, will the `EmailProvider` trait be extracted from Gmail + JMAP? If yes, Graph is the first provider built against it. If no, Graph is another one-off and we have three providers to reconcile later.

### 6. Send format

Graph's `/me/sendMail` accepts a JSON message body, NOT raw RFC 822. Two paths:

- **JSON send** (documented): Build the message as a Graph `Message` JSON object. Fields: `subject`, `body { contentType, content }`, `toRecipients`, `ccRecipients`, `bccRecipients`, `attachments`. No MIME construction needed. But: handling inline images, Content-ID references, and multipart/alternative becomes our problem (the JSON API flattens this).
- **MIME send** (undocumented but works): POST to `/me/sendMail` with `Content-Type: text/plain` and raw MIME bytes. Would allow reusing `mail-builder`. But: undocumented API surface is risky.

There's also a third option: **create draft + send draft**. `POST /me/messages` creates a draft (JSON), then `POST /me/messages/{id}/send` sends it. This is two API calls but gives us an ID for the sent message (useful for tracking).

Test all three paths before deciding.

---

## Key Differences from Gmail and JMAP

| Aspect | Gmail | JMAP | Graph |
|--------|-------|------|-------|
| **Membership model** | Labels (multi-label) | Mailboxes (multi-mailbox) | Folders (single folder) + Categories |
| **Threading** | Native `threadId` | Native `threadId` | `conversationId` (less reliable) |
| **Delta sync** | Global History API (expires ~30 days) | Global `Email/changes` state strings | Per-folder delta tokens (don't expire) |
| **Concurrency** | Generous (10+ parallel) | Server-side batching | **4 concurrent max** |
| **Auth** | Google OAuth2 + PKCE | Basic or Bearer | Microsoft OAuth2 (Entra ID) + PKCE |
| **Send** | POST raw RFC 822 base64url | Email/import + EmailSubmission/set | POST `/me/sendMail` (JSON body, not raw) |
| **Attachments** | Part of message payload | Blob download by ID | Separate `/attachments/{id}` endpoint, upload sessions for >3MB |
| **Sync state storage** | `history_id` on account | `jmap_sync_state` table (per type) | Per-folder delta tokens (new table needed) |
| **Message body in API** | base64url in payload parts | `fetchHTMLBodyValues`/`fetchTextBodyValues` inline UTF-8 | `body.content` string (HTML or text), `uniqueBody` for deduped |
| **Trait used** | Gmail-specific client (`GmailClient`) | `jmap-client` crate wrapping | Shared `EmailProvider` trait (first consumer) |

### Graph-specific concerns

- **Send format**: Graph's `/me/sendMail` accepts a JSON message object, NOT raw RFC 822. We can't reuse `mail-builder` output directly for sending. See [Open Question 6](#6-send-format).
- **Large attachments**: Files >3MB require upload sessions (`/me/messages/{id}/attachments/createUploadSession`). This is a multi-step process unlike Gmail/JMAP where attachments are part of the message payload.
- **OData pagination**: All list endpoints use `@odata.nextLink` / `@odata.deltaLink`. Need a generic `ODataCollection<T>` wrapper struct with `#[serde(rename = "@odata.nextLink")]`.
- **Focused Inbox**: Graph exposes `inferenceClassification` (Focused/Other). Could map to our category system. Optional enrichment.
- **`$select` efficiency**: Graph supports `$select` to request only specific fields. This is critical for performance — a full `Message` object is large. Always use `$select` to request only the fields we need (id, subject, from, toRecipients, receivedDateTime, body, conversationId, flag, categories, parentFolderId, isRead, isDraft, hasAttachments, internetMessageHeaders).
- **`internetMessageHeaders`**: Not included in default responses — must be explicitly requested via `$select`. These contain `Message-ID`, `References`, `In-Reply-To`, `Authentication-Results`, `List-Unsubscribe`. Essential for threading, auth display, and unsubscribe.

---

## Current State

### No production TS code

Like JMAP, there is no existing production TypeScript Graph implementation. The Exchange assessment doc (`docs/microsoft-exchange-assessment.md`) is research only. There is nothing to migrate from — this is a new provider, same as JMAP.

### DB schema additions needed

```sql
-- New table: per-folder delta tokens
CREATE TABLE graph_folder_delta_tokens (
    account_id TEXT NOT NULL,
    folder_id TEXT NOT NULL,           -- Graph folder ID
    delta_link TEXT NOT NULL,          -- @odata.deltaLink URL
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    PRIMARY KEY (account_id, folder_id),
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);
```

Also:
- `accounts` table may need a `graph_user_id` column (Graph uses a user principal name, not just email)
- Accounts with `provider = "graph"` use the existing `auth_method = "oauth"` column
- OAuth tokens stored encrypted in the same columns as Gmail tokens

### Integration points to wire up (Phase 2)

- `providerFactory.ts` — route `account.provider === "graph"` to Rust-backed commands (or provider-agnostic commands if trait-based routing exists)
- `syncManager.ts` — add `syncGraphAccount()` calling Rust sync commands
- `emailActions.ts` / `queueProcessor.ts` — dispatch Graph actions to `graph_*` Rust commands
- `AddGraphAccount.tsx` — account setup UI (OAuth flow → test connection → save)

### Reference implementations to study

- [EmailEngine](https://github.com/postalsys/emailengine) (Node.js) — unifies IMAP, Gmail API, and Graph API behind a single REST API. Good reference for Graph mail integration patterns.
- Microsoft's official [Graph SDKs](https://github.com/microsoftgraph) (C#, JS, Go, Python) — generated via Kiota. Reference for auth flows, paging, delta sync, error handling.
- [Tauri discussion #5534](https://github.com/tauri-apps/tauri/discussions/5534) — signing in users and calling Microsoft Graph from a Tauri desktop app.

---

## Target State (Rust)

### Module structure

```
src-tauri/src/graph/
├── mod.rs              # Re-exports
├── types.rs            # Graph API serde types (Message, MailFolder, Attachment, ODataCollection)
├── client.rs           # GraphClient — Arc<RwLock<TokenState>>, reqwest, &self methods
├── api.rs              # Graph REST endpoint methods (~18 calls)
├── parse.rs            # Graph Message → internal message types (for DB persistence)
├── folder_mapper.rs    # Graph folder → Gmail-style label mapping (well-known folder IDs + categories)
├── sync.rs             # Per-folder delta sync + initial sync
└── commands.rs         # Tauri commands (graph_* or provider-agnostic)
```

### Infrastructure reused from Gmail + JMAP migrations

| Module | What Graph uses |
|--------|---------------|
| `provider/token.rs` | `TokenState`, `refresh_oauth_token()` — with Microsoft token endpoint |
| `provider/http.rs` | `build_api_client()` — reqwest-middleware with retry (respects 429 + `Retry-After`) |
| `provider/message.rs` | NOT used for send (Graph takes JSON). May be used for draft import if MIME path works. |
| `db/` | All DB write commands — `upsert_thread()`, `set_thread_labels()`, `upsert_message()`, etc. |
| `body_store/` | `body_store_put()`, `body_store_get()` — same compressed body storage |
| `search/` | `index_message()` — same Tantivy indexing |

### Tauri command surface

If provider-agnostic commands exist (trait-based routing), Graph may not need its own command surface. But if `graph_*` prefixed:

```rust
// Lifecycle
graph_init_client(account_id)
graph_remove_client(account_id)
graph_test_connection(account_id)

// Sync
graph_sync_initial(account_id, days_back)
graph_sync_delta(account_id)

// Folder operations
graph_list_folders(account_id)
graph_create_folder(account_id, display_name, parent_id?)
graph_rename_folder(account_id, folder_id, new_name)
graph_delete_folder(account_id, folder_id)

// Email actions (thread-level — internally enumerates messages)
graph_archive(account_id, thread_id)
graph_trash(account_id, thread_id)
graph_permanent_delete(account_id, message_ids)
graph_mark_read(account_id, thread_id, read)
graph_star(account_id, thread_id, starred)       // maps to flag.flagStatus
graph_spam(account_id, thread_id, is_spam)        // move to Junk folder
graph_move_to_folder(account_id, thread_id, folder_id)
graph_add_category(account_id, thread_id, category)
graph_remove_category(account_id, thread_id, category)

// Send + drafts
graph_send_email(account_id, message_json)        // JSON message, not raw RFC 822
graph_create_draft(account_id, message_json)
graph_update_draft(account_id, draft_id, message_json)
graph_delete_draft(account_id, draft_id)

// Attachments
graph_fetch_attachment(account_id, message_id, attachment_id)

// Profile
graph_get_profile(account_id)
```

~22 commands. Note the key difference from Gmail/JMAP: `graph_send_email` takes a JSON message object, not `raw_base64url`. The TS composer will need a code path that builds Graph's JSON message format instead of raw RFC 822.

### Graph client design

```rust
pub struct GraphClient {
    http: ClientWithMiddleware,       // reqwest-middleware, Clone
    account_id: String,
    token: Arc<RwLock<TokenState>>,   // same pattern as GmailClient
    client_id: String,
    semaphore: Arc<Semaphore>,        // concurrency=3 (leave 1 for user actions)
}

impl GraphClient {
    pub async fn from_account(db: &DbState, account_id: &str) -> Result<Self, String> {
        // 1. Load account from DB
        // 2. Decrypt tokens
        // 3. Build TokenState with Microsoft token endpoint
        // 4. Create reqwest client with build_api_client()
        // 5. Semaphore with 3 permits (4 concurrent max, reserve 1)
    }

    /// Authenticated GET. Acquires semaphore permit, refreshes token if needed.
    pub async fn get<T: DeserializeOwned>(
        &self, endpoint: &str, db: &DbState,
    ) -> Result<T, String>;

    /// Authenticated POST.
    pub async fn post<T: DeserializeOwned>(
        &self, endpoint: &str, body: &impl Serialize, db: &DbState,
    ) -> Result<T, String>;

    /// Authenticated PATCH (for message updates).
    pub async fn patch<T: DeserializeOwned>(
        &self, endpoint: &str, body: &impl Serialize, db: &DbState,
    ) -> Result<T, String>;

    // Retry logic:
    // - 401 → refresh token, retry once
    // - 429 → respect Retry-After header, backoff
    // - 5xx → handled by reqwest-middleware
    // - 503 with Retry-After → service unavailable, backoff
}
```

**Key difference from Gmail**: `Semaphore` for concurrency control. Gmail uses `Semaphore(10)` for parallel thread fetch. Graph uses `Semaphore(3)` globally — every API call acquires a permit. This enforces the 4-concurrent-request limit at the client level.

**Same as Gmail**: `Arc<RwLock<TokenState>>` for concurrent token access, `&self` on all methods, `Clone` for sharing across tasks.

**Different from JMAP**: JMAP uses `jmap-client` crate (wraps its own reqwest). Graph is hand-rolled like Gmail — direct reqwest calls with serde types.

---

## Phase 1: Rust Graph Provider (Client + Actions + Sync)

**Goal**: Build the complete Graph provider in Rust — client, all email actions, AND sync engine. Same structure as JMAP: no TS-orchestrated-sync phase because there's no existing TS code to migrate from.

**Prerequisite**: Gmail and JMAP migrations are complete. Shared `EmailProvider` trait is extracted (or at minimum, the patterns are clear enough to build Graph consistently).

### 1a. DB migration

Add to `src/services/db/migrations.ts`:

```sql
-- Migration N (after current latest)
CREATE TABLE graph_folder_delta_tokens (
    account_id TEXT NOT NULL,
    folder_id TEXT NOT NULL,
    delta_link TEXT NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    PRIMARY KEY (account_id, folder_id),
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);
```

No new account columns needed — Graph accounts use `provider = "graph"`, `auth_method = "oauth"`, and existing encrypted token columns.

### 1b. Graph API types (`graph/types.rs`)

OData wrapper + Graph Mail API response types:

```rust
/// Generic OData collection wrapper for all list endpoints.
#[derive(Debug, Deserialize)]
pub struct ODataCollection<T> {
    pub value: Vec<T>,
    #[serde(rename = "@odata.nextLink")]
    pub next_link: Option<String>,
    #[serde(rename = "@odata.deltaLink")]
    pub delta_link: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphMessage {
    pub id: String,
    pub conversation_id: Option<String>,
    pub subject: Option<String>,
    pub body_preview: Option<String>,
    pub body: Option<GraphBody>,
    pub unique_body: Option<GraphBody>,
    pub from: Option<GraphRecipient>,
    pub to_recipients: Option<Vec<GraphRecipient>>,
    pub cc_recipients: Option<Vec<GraphRecipient>>,
    pub bcc_recipients: Option<Vec<GraphRecipient>>,
    pub reply_to: Option<Vec<GraphRecipient>>,
    pub received_date_time: Option<String>,
    pub sent_date_time: Option<String>,
    pub is_read: Option<bool>,
    pub is_draft: Option<bool>,
    pub has_attachments: Option<bool>,
    pub importance: Option<String>,
    pub parent_folder_id: Option<String>,
    pub categories: Option<Vec<String>>,
    pub flag: Option<GraphFlag>,
    pub inference_classification: Option<String>,
    pub internet_message_headers: Option<Vec<GraphInternetHeader>>,
    pub internet_message_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphBody {
    pub content_type: String,   // "html" or "text"
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphRecipient {
    pub email_address: GraphEmailAddress,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEmailAddress {
    pub name: Option<String>,
    pub address: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphFlag {
    pub flag_status: String,  // "notFlagged", "flagged", "complete"
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphInternetHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphMailFolder {
    pub id: String,
    pub display_name: String,
    pub parent_folder_id: Option<String>,
    pub child_folder_count: Option<i32>,
    pub total_item_count: Option<i32>,
    pub unread_item_count: Option<i32>,
    pub is_hidden: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphAttachment {
    pub id: String,
    pub name: String,
    pub content_type: String,
    pub size: i64,
    pub is_inline: Option<bool>,
    pub content_id: Option<String>,
    pub content_bytes: Option<String>,   // base64 encoded, only for small attachments
}

// Send types (JSON message body)
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphSendMessage {
    pub subject: String,
    pub body: GraphSendBody,
    pub to_recipients: Vec<GraphSendRecipient>,
    pub cc_recipients: Option<Vec<GraphSendRecipient>>,
    pub bcc_recipients: Option<Vec<GraphSendRecipient>>,
    pub attachments: Option<Vec<GraphSendAttachment>>,
    pub internet_message_headers: Option<Vec<GraphInternetHeader>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphSendBody {
    pub content_type: String,  // "HTML" or "Text"
    pub content: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphSendRecipient {
    pub email_address: GraphSendEmailAddress,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphSendEmailAddress {
    pub name: Option<String>,
    pub address: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphSendAttachment {
    #[serde(rename = "@odata.type")]
    pub odata_type: String,  // "#microsoft.graph.fileAttachment"
    pub name: String,
    pub content_type: String,
    pub content_bytes: String,  // base64
}
```

### 1c. Folder mapper (`graph/folder_mapper.rs`)

Maps Graph well-known folder IDs and display names to Gmail-style label IDs. Analogous to JMAP's `mailbox_mapper.rs` and IMAP's `folderMapper.ts`.

```rust
/// Graph well-known folder IDs.
/// These are constant strings returned by the Graph API.
const WELL_KNOWN_FOLDERS: &[(&str, &str, &str)] = &[
    // (well_known_name,   label_id,    label_name)
    ("inbox",           "INBOX",     "Inbox"),
    ("drafts",          "DRAFT",     "Drafts"),
    ("sentitems",       "SENT",      "Sent"),
    ("deleteditems",    "TRASH",     "Trash"),
    ("junkemail",       "SPAM",      "Spam"),
    ("archive",         "archive",   "Archive"),
];

pub struct FolderLabelMapping {
    pub label_id: String,
    pub label_name: String,
    pub label_type: &'static str,  // "system" or "user"
}

/// Map a Graph folder to a Gmail-style label.
pub fn map_folder_to_label(
    folder: &GraphMailFolder,
    well_known_name: Option<&str>,
) -> FolderLabelMapping;

/// Derive label IDs from a message's folder + categories.
/// For Graph, a message has exactly one folder (primary label)
/// and zero or more categories (supplementary labels).
pub fn get_labels_for_message(
    parent_folder_id: &str,
    categories: &[String],
    is_read: bool,
    flag_status: &str,
    folder_map: &HashMap<String, FolderLabelMapping>,
) -> Vec<String>;

/// Reverse lookup: Gmail-style label ID → Graph folder ID.
pub fn label_id_to_folder_id(
    label_id: &str,
    folders: &[(String, Option<String>, String)],  // (id, well_known_name, display_name)
) -> Option<String>;
```

### 1d. Message parsing (`graph/parse.rs`)

Converts Graph `Message` response to our internal DB-ready struct:

```rust
pub fn parse_graph_message(
    msg: &GraphMessage,
    folder_map: &HashMap<String, FolderLabelMapping>,
) -> Result<ParsedGraphMessage, String>;
```

**Graph vs Gmail/JMAP parsing differences**:
- Body comes as a string (HTML or text) in `body.content`. No base64 decoding, no MIME part walking.
- `uniqueBody` provides the deduped body (excludes quoted replies). Could use for body store if reliable.
- `internetMessageHeaders` must be explicitly requested via `$select`. Contains `Message-ID`, `References`, `In-Reply-To`, `Authentication-Results`, `List-Unsubscribe`.
- `conversationId` as thread ID (unless we implement JWZ — see Open Question 3).
- `categories` are supplementary labels (not mailbox membership like JMAP).
- `flag.flagStatus` maps to STARRED pseudo-label (`"flagged"` → STARRED, `"notFlagged"` → no STARRED).
- `isRead` directly maps to UNREAD pseudo-label (inverted: `!isRead` → UNREAD).
- `parentFolderId` determines the primary folder label.
- Auth results parsed from `internetMessageHeaders` → `Authentication-Results` (same `auth_parser.rs` logic as Gmail, but header must be explicitly requested).

### 1e. Sync engine (`graph/sync.rs`)

See [Per-Folder Delta Sync Design](#per-folder-delta-sync-design) for the full design.

```rust
/// Initial sync: folders → per-folder paginated message fetch → DB writes.
pub async fn graph_initial_sync(
    client: &GraphClient,
    account_id: &str,
    days_back: i64,
    db: &DbState,
    body_store: &BodyStoreState,
    search: &SearchState,
    app_handle: &AppHandle,
) -> Result<(), String>;

/// Delta sync: per-folder delta queries → targeted updates → DB writes.
pub async fn graph_delta_sync(
    client: &GraphClient,
    account_id: &str,
    db: &DbState,
    body_store: &BodyStoreState,
    search: &SearchState,
    app_handle: &AppHandle,
) -> Result<GraphDeltaSyncResult, String>;

pub struct GraphDeltaSyncResult {
    pub new_inbox_message_ids: Vec<String>,
    pub affected_thread_ids: Vec<String>,
}
```

### 1f. Email action commands

Each action maps to Graph REST calls. Thread-level actions enumerate messages first (see [Thread-Level Action Semantics](#thread-level-action-semantics)).

**Archive** — move all thread messages from inbox to archive folder:
```rust
#[tauri::command]
pub async fn graph_archive(
    account_id: String, thread_id: String,
    db: State<'_, DbState>, graph: State<'_, GraphState>,
) -> Result<(), String> {
    let client = graph.get(&account_id).await?;

    // 1. Resolve archive folder ID
    let archive_id = resolve_folder_id(&client, "archive", &db).await?;

    // 2. Enumerate messages in thread that are in inbox
    let message_ids = query_thread_message_ids(&client, &thread_id, &db).await?;

    // 3. Move each message to archive (POST /me/messages/{id}/move)
    // Graph's move is per-message, no batch endpoint
    for msg_id in &message_ids {
        client.post::<GraphMessage>(
            &format!("me/messages/{}/move", msg_id),
            &serde_json::json!({ "destinationId": archive_id }),
            &db,
        ).await?;
    }
    Ok(())
}
```

**Key difference from JMAP**: JMAP can batch-mutate in a single request (`Email/set` on multiple IDs). Graph has no batch mutation for mail — each move/update is a separate request. With the 4-concurrent limit, thread-level actions on large threads will be slower.

**Mitigation**: Graph supports [JSON batching](https://learn.microsoft.com/en-us/graph/json-batching) — up to 20 requests in a single POST to `/$batch`. Use this for thread-level actions:

```rust
async fn batch_move_messages(
    client: &GraphClient,
    message_ids: &[String],
    destination_folder_id: &str,
    db: &DbState,
) -> Result<(), String> {
    // Chunk into batches of 20 (Graph batch limit)
    for chunk in message_ids.chunks(20) {
        let batch_requests: Vec<_> = chunk.iter().enumerate().map(|(i, id)| {
            serde_json::json!({
                "id": i.to_string(),
                "method": "POST",
                "url": format!("/me/messages/{}/move", id),
                "body": { "destinationId": destination_folder_id },
                "headers": { "Content-Type": "application/json" }
            })
        }).collect();
        client.post::<serde_json::Value>(
            "$batch",
            &serde_json::json!({ "requests": batch_requests }),
            db,
        ).await?;
    }
    Ok(())
}
```

### 1g. Attachment download

```rust
#[tauri::command]
pub async fn graph_fetch_attachment(
    account_id: String, message_id: String, attachment_id: String,
    db: State<'_, DbState>, graph: State<'_, GraphState>,
) -> Result<AttachmentData, String> {
    let client = graph.get(&account_id).await?;
    let attachment: GraphAttachment = client.get(
        &format!("me/messages/{}/attachments/{}", message_id, attachment_id),
        &db,
    ).await?;
    // content_bytes is base64 encoded by Graph
    let data = BASE64_STANDARD.decode(&attachment.content_bytes.unwrap_or_default())
        .map_err(|e| e.to_string())?;
    Ok(AttachmentData {
        data: BASE64_STANDARD.encode(&data),
        size: data.len(),
    })
}
```

**Note**: For attachments >3MB, Graph may not include `contentBytes` inline. Use `GET /me/messages/{id}/attachments/{id}/$value` to stream the raw bytes. Large attachment upload uses session-based upload (deferred to post-MVP).

### 1h. Tauri state registration

In `lib.rs`:
```rust
.manage(GraphState::new())
```

Register all `graph_*` commands in the `.invoke_handler()` list.

### Phase 1 deliverable

The complete Graph provider exists in Rust: client with OAuth2 token management, all email actions (with JSON batching for thread-level ops), per-folder delta sync. Auth is OAuth2 via Entra ID, tokens refreshed by Rust. Sync writes directly to DB, body store, and search index. But no TS wiring yet — Phase 2 connects the UI.

---

## Phase 2: TS Integration + UI

**Goal**: Wire the Rust Graph provider into the TS application layer.

### 2a. Account setup UI

**`AddGraphAccount.tsx`** — new component, 2-step flow:

1. **OAuth sign-in**: User clicks "Sign in with Microsoft" → launch OAuth2 flow (same as Gmail: open browser → localhost redirect → token exchange). On success, save account to DB with `provider = "graph"`, `auth_method = "oauth"`, encrypted tokens.
2. **Test connection**: Call `graph_test_connection`. On success, trigger initial sync.

Simpler than JMAP (no manual URL entry) and Gmail (no client ID setup — if we ship a default app registration). The OAuth flow handles everything.

### 2b. Provider factory + email actions

- **`providerFactory.ts`**: Route `account.provider === "graph"` to a thin `GraphProvider` that delegates all methods to `graph_*` Tauri commands. If provider-agnostic commands exist, this routing may happen in Rust instead.
- **`emailActions.ts`**: Add Graph cases in the action dispatcher. Thread-level actions pass `threadId` to Rust — Rust handles message enumeration and JSON batching internally.
- **`queueProcessor.ts`**: Graph action cases call `graph_*` Tauri commands. The `resource_id` in `pending_operations` remains a `threadId` (same as Gmail/IMAP/JMAP).

### 2c. Sync manager

Add `syncGraphAccount()` to `syncManager.ts`:

```typescript
async function syncGraphAccount(accountId: string) {
  try {
    const result = await invoke('graph_sync_delta', { accountId });
    // Post-sync hooks (still TS)
    await applyFiltersToNewMessageIds(accountId, result.newInboxMessageIds);
    await applySmartLabelsToNewMessageIds(accountId, result.newInboxMessageIds);
    // ... notifications, categorization
  } catch (err) {
    const msg = typeof err === 'string' ? err : '';
    if (msg === 'GRAPH_NO_DELTA_STATE') {
      await invoke('graph_sync_initial', { accountId, daysBack: syncDays });
    } else throw err;
  }
}
```

### 2d. Composer changes

The composer currently builds raw RFC 822 messages (via TS) and passes `raw_base64url` to Gmail/JMAP send commands. Graph takes a JSON message body instead.

Two options:
- **Build Graph JSON in TS**: The composer already has structured message data (recipients, subject, body HTML, attachments). Serialize to Graph's JSON format before invoking `graph_send_email`.
- **Build Graph JSON in Rust**: Pass the same structured data to Rust, let Rust serialize to Graph format.

The first option is simpler and keeps the existing composer→send contract per-provider. The TS `emailActions.ts` already dispatches per-provider — adding a Graph-specific JSON serializer is straightforward.

### 2e. App startup

In `App.tsx` startup sequence:
- `getAllAccounts()` → for Graph accounts, call `graph_init_client` (same pattern as Gmail/JMAP).
- Sync timer already handles multi-provider via `syncManager.ts` routing.

### Phase 2 deliverable

Graph accounts can be added via OAuth, synced, and acted on through the full UI. The sync timer includes Graph accounts. Email actions work through the offline queue. Composer handles Graph's JSON send format.

---

## Thread-Level Action Semantics

Same fundamental problem as JMAP — see `docs/jmap-rust-migration.md` for the full design rationale.

### The problem

Our app's action model is thread-centric (archive a thread, star a thread, trash a thread). Graph has no thread-level mutations. All operations are per-message. Additionally, Graph's `conversationId` may group messages differently than our UI's thread view.

### The design

Thread-level Graph actions enumerate messages in the thread and mutate each one:

```rust
/// Shared helper: get all message IDs in a thread.
async fn query_thread_message_ids(
    client: &GraphClient,
    thread_id: &str,  // conversationId
    db: &DbState,
) -> Result<Vec<String>, String> {
    // Option A: Query Graph API
    // GET /me/messages?$filter=conversationId eq '{thread_id}'&$select=id
    // Note: conversationId filter may not be supported on all endpoints.
    //
    // Option B: Query local DB
    // SELECT id FROM messages WHERE thread_id = ? AND account_id = ?
    // Faster, no API call, but may miss messages not yet synced.
    //
    // Use Option B (local DB) — consistent with JMAP's approach.
    // If a message isn't in our DB, it hasn't been synced yet and
    // shouldn't be affected by the action.
}
```

Every thread-level action (`graph_archive`, `graph_trash`, `graph_star`, `graph_mark_read`, `graph_spam`, `graph_move_to_folder`, `graph_add_category`, `graph_remove_category`) calls this helper first, then applies the operation to all returned message IDs using JSON batching (up to 20 per batch request).

### Differences from JMAP thread actions

1. **No server-side batching**: JMAP's `Email/set` mutates multiple emails in one API call. Graph requires individual REST calls per message, mitigated by JSON batching (20 per `/$batch`).
2. **Move vs patch**: JMAP changes mailbox membership via `Email/set` patches. Graph uses `POST /me/messages/{id}/move` (changes folder) and `PATCH /me/messages/{id}` (changes flags/categories). Different HTTP methods for different mutation types.
3. **Star semantics**: Gmail uses `STARRED` label. JMAP uses `$flagged` keyword. Graph uses `flag.flagStatus = "flagged"`. All map to the same STARRED pseudo-label in our UI.

### Edge case: new messages arriving after action

Same as JMAP — the action applies to messages that existed at execution time. The next delta sync will reconcile.

---

## Folder-to-Label Mapping

This is a product design section, not just an adapter detail. The mapping strategy affects what users see in the sidebar and how they interact with Graph accounts.

### The constraint

Gmail: a message can have multiple labels. JMAP: a message can be in multiple mailboxes. Graph: a message lives in **exactly one folder**. But Graph messages can also have **categories** (color-coded tags, up to 25 per message).

### Recommended approach: Hybrid (folder + categories)

1. **Folder → primary location label**: Each folder maps to a label. Well-known folders get system label IDs (`INBOX`, `SENT`, `TRASH`, `SPAM`, `DRAFT`, `archive`). User folders get `graph-{folderId}` label IDs. A message's `parentFolderId` determines its one folder label.

2. **Categories → supplementary labels**: Graph categories map to user labels with a `graph-cat-{name}` label ID prefix. Categories are additive — a message in the Inbox folder with categories "Project X" and "Urgent" would have labels: `INBOX`, `graph-cat-Project X`, `graph-cat-Urgent`.

3. **Pseudo-labels from flags**: `isRead = false` → `UNREAD`. `flag.flagStatus = "flagged"` → `STARRED`. Same as Gmail/JMAP.

### What this means for the UI

- Sidebar shows Graph folders (like IMAP) + categories (like Gmail labels)
- Thread list for a folder shows messages in that folder
- Thread list for a category shows messages with that category (across all folders)
- "Archive" action moves to Archive folder (removes from Inbox)
- "Label" action adds/removes categories (not folders — a message can't be in two folders)
- "Move to" action changes the folder (moves the message)

### What this means for the trait

The shared `EmailProvider` trait must accommodate:
- `add_label(thread_id, label_id)` → for Graph, if label is a category: add category. If label is a folder: move to folder.
- `remove_label(thread_id, label_id)` → for Graph, if label is a category: remove category. If label is a folder: error or no-op (can't remove folder without specifying destination).

This is the hardest part of the trait to get right. Gmail's "add label INBOX" is idempotent. Graph's "move to inbox" is a side-effecting folder change. The trait must either:
- Accept this semantic difference and let each provider interpret `add_label`/`remove_label` differently, OR
- Split into separate `move_to_folder` and `add_category` operations, with provider-specific mapping.

Seeing how Gmail labels and JMAP mailboxes interact with the trait will inform the right design.

---

## Per-Folder Delta Sync Design

This is the most significant architectural difference from Gmail and JMAP sync.

### Why per-folder is harder

Gmail delta sync: one `history.list()` call returns all changes globally. JMAP delta sync: one `Email/changes()` call returns all changed email IDs globally. Both are O(1) API calls to discover what changed.

Graph delta sync: must query each folder separately. `GET /me/mailFolders/{id}/messages/delta` for each folder. For a typical account with 10-15 folders, that's 10-15 API calls per sync cycle. With the 4-concurrent limit, these must be serialized or lightly parallelized.

### Initial sync

1. **Folders**: `GET /me/mailFolders?$top=100` → persist as labels via folder mapper → cache folder IDs
2. **Messages per folder**: For each folder (prioritize Inbox, Sent, Drafts):
   - `GET /me/mailFolders/{id}/messages?$filter=receivedDateTime ge {sinceDate}&$select={fields}&$top=50&$orderby=receivedDateTime desc`
   - Paginate via `@odata.nextLink`
   - For each message: `parse_graph_message()` → DB writes (same pipeline as Gmail/JMAP)
   - Must request `internetMessageHeaders` for each message to get threading headers, auth results, unsubscribe headers
3. **Store initial delta links**: After fetching all messages for a folder, request `GET /me/mailFolders/{id}/messages/delta?$select={fields}` and store the returned `@odata.deltaLink` in `graph_folder_delta_tokens`

### Delta sync

1. **For each folder with a stored delta link**:
   - `GET {deltaLink}` (the stored `@odata.deltaLink` URL)
   - Paginate via `@odata.nextLink` if results span multiple pages
   - Returns created/updated messages (full objects) and deleted message IDs (with `@removed` annotation)
   - For each message, resolve `conversationId` → thread_id, check `pending_operations` for that thread
   - Parse and persist (same path as initial sync)
   - Delete removed messages from local DB
   - Store new `@odata.deltaLink` for next sync
2. **New folders**: If a new folder appears (not in `graph_folder_delta_tokens`), do an initial fetch for that folder
3. **Folder changes**: Periodically re-fetch folder list to detect renames/deletes/new folders

### Folder sync ordering

Not all folders need to be synced equally often:

- **High priority** (every sync cycle): Inbox, Sent, Drafts
- **Medium priority** (every 5th sync cycle): Archive, Trash, Spam
- **Low priority** (every 20th sync cycle): Other user folders

This reduces the per-cycle API call count from 10-15 to 3-5 for most sync cycles.

### Progress reporting

```rust
app_handle.emit("graph-sync-progress", &GraphSyncProgress {
    account_id,
    phase: "delta",
    folder_name: "Inbox",
    current_folder: 1,
    total_folders: 12,
    messages_processed: 42,
})?;
```

---

## Sync vs Queue: Write Ordering

Same principle as Gmail and JMAP — see `docs/gmail-rust-migration.md` for the full explanation.

### The rule

**Before overwriting a message's state during delta sync, resolve its `conversationId` (thread ID) and check `pending_operations` for that thread. If any pending ops exist for the thread, skip all messages in that thread — the queue processor will reconcile when the op flushes.**

### Graph-specific nuance

Graph delta sync returns full message objects (not just IDs like JMAP's `Email/changes`). The `conversationId` is included in the response, so there's no extra lookup needed — the thread ID resolution is free.

The flow:
1. Delta response includes messages with `conversationId`
2. For each message, check `pending_operations WHERE resource_id = message.conversationId`
3. If pending ops exist for the thread, skip all messages with that `conversationId`
4. Otherwise, persist normally

This is a read from the same `ratatoskr.db` that both Rust and TS write to, consistent via SQLite's `Mutex<Connection>` serialization.

---

## Migration Strategy

### Per-account cutover

Same as Gmail/JMAP: once `graph_init_client` succeeds, all operations for that Graph account go through Rust. No mixed mode.

### Rollback strategy

Same as JMAP — there is no TS Graph fallback because there was never a TS Graph implementation.

- **Account-level disable**: Users can remove the Graph account and re-add via IMAP+OAuth2 (if available by then).
- **Feature flag in Rust**: `graph_sync_enabled` DB setting. Kill switch for sync without code change.
- **Incremental rollout**: Graph only activates for accounts explicitly added as `provider = "graph"`. Existing accounts are not affected.

### Testing strategy

- **Unit tests**: Rust tests for `folder_mapper.rs`, `parse.rs`, `types.rs` (mock JSON from real Graph API responses)
- **Integration tests**: Tauri command tests with mock HTTP server serving Graph API responses (`wiremock-rs`). OData pagination, delta responses with `@odata.nextLink`/`@odata.deltaLink`, `@removed` annotations, JSON batching responses.
- **Real account testing**: Test against a personal Outlook.com account (free) and an Exchange Online account (if available). Delta sync round-trip, thread-level actions, attachment download, send.
- **Rate limit testing**: Verify Semaphore enforcement at concurrency=3. Simulate 429 responses with `Retry-After`.
- **Manual testing**: Graph account setup → OAuth → initial sync → delta sync → archive/trash/star/send → verify round-trip

### Estimated scope

| Phase | New Rust lines (est.) | New TS lines | Difficulty |
|-------|----------------------|-------------|------------|
| Phase 1: Full Rust provider | ~1,600-2,200 | ~0 | Moderate-High — hand-rolled REST (like Gmail), but per-folder delta sync and JSON batching add complexity |
| Phase 2: TS integration + UI | ~0 | ~600 (UI + thin adapters + composer JSON path + sync wiring) | Low-Moderate — thin glue, but composer needs a Graph-specific message serializer |
| **Total** | **~1,600-2,200** | **~600** | |

Larger than JMAP (~1,400-1,800 Rust lines) because:
1. Hand-rolled REST (no `jmap-client` crate doing the heavy lifting)
2. Per-folder delta sync is more complex than JMAP's global `Email/changes`
3. JSON batching for thread-level actions (JMAP batches natively)
4. OData pagination types and handling
5. Concurrency control (Semaphore-based throttling)
6. Folder + category mapping is more complex than JMAP's mailbox mapping

Smaller than Gmail (~2,700-3,500 Rust lines) because:
1. No existing TS code to maintain backwards compatibility with (no Phase 3 teardown)
2. Simpler body handling (no base64url decoding, no MIME part walking)
3. No History API complexity (delta tokens don't expire)
4. Shared infrastructure (token refresh, HTTP client, DB writes) already exists

---

## What We Defer

### Prerequisites from earlier migrations

1. **Shared `EmailProvider` trait** — extract from Gmail + JMAP after JMAP Phase 1 is complete. This is the prerequisite for Graph.
2. **Provider-agnostic Tauri commands** — depends on the trait. Graph may be the first consumer.

### Graph-specific

3. **Microsoft OAuth2 in `oauth.rs`** — extend the existing OAuth server to handle Microsoft endpoints and scopes. The flow is identical (PKCE + localhost redirect), only endpoints and scopes differ. May happen during Gmail Rust migration if `oauth.rs` is generalized.
4. **Per-folder delta token storage** — `graph_folder_delta_tokens` table. Schema defined above.
5. **Graph-to-label mapping strategy** — product decision on folders + categories → labels. Preliminary design in [Folder-to-Label Mapping](#folder-to-label-mapping), needs validation with real accounts.
6. **Thread ID strategy** — `conversationId` vs JWZ threading. Needs investigation of `conversationId` reliability across real accounts.
7. **Send format investigation** — JSON message body vs raw MIME vs create-draft-then-send. Test all three paths, pick the one that handles attachments and encoding correctly.
8. **Large attachment upload sessions** — multi-step upload for >3MB files. Not critical for initial implementation (can limit to inline/small attachments), but needed for full parity.
9. **Webhook subscriptions** — Graph supports push notifications via webhooks for real-time sync. Requires a reachable endpoint (problem for desktop apps). Polling via delta sync is the initial approach. Investigate if Tauri can expose a local webhook receiver via the existing localhost server.
10. **Azure AD app registration** — create and configure the app registration. Publisher verification for organizational access. Decide on default-shipped vs user-provided model.
11. **Focused Inbox integration** — map Graph's `inferenceClassification` to our category tabs (Primary/Other mapping). Optional enrichment after basic sync works.
12. **Exchange on-premises via EWS** — only if significant demand. `ews-rs` from Thunderbird provides types, but no client. SOAP/XML complexity is high. On-prem users can use IMAP.
13. **JSON batching optimization** — the `/$batch` endpoint supports up to 20 requests per batch. Investigate using it for initial sync (batch message fetches) in addition to thread-level actions.
14. **`$expand` for attachments** — `GET /me/messages/{id}?$expand=attachments` can inline attachment metadata in message responses. May eliminate separate attachment list calls during sync.
15. **`uniqueBody` usage** — Graph's `uniqueBody` field returns the message body without quoted replies. Could improve body store efficiency and thread display. Investigate reliability.

### Quick win (can happen before full Graph)

16. **IMAP + OAuth2 for Outlook.com** — add Microsoft OAuth2 flow, use XOAUTH2 SASL with our existing IMAP provider. Gives Outlook users immediate access without building the full Graph provider. Requires only: OAuth2 endpoint configuration in `oauth.rs`, Azure AD app registration, IMAP AUTHENTICATE with OAuth token (already supported in `connection.rs`). This is independent of the Graph migration and could ship at any time.

---

## References

- `docs/microsoft-exchange-assessment.md` — full ecosystem assessment (EWS vs Graph, crate evaluation, auth details, rate limits)
- `docs/rust-provider-crate-research.md` — crate decisions and strategic plan (Graph endpoints table, architecture decisions)
- `docs/gmail-rust-migration.md` — Gmail patterns that Graph will follow (token management, reqwest setup, sync-with-DB-writes)
- `docs/jmap-rust-migration.md` — JMAP patterns (thread-level action semantics, mailbox mapping, trait extraction trigger)
