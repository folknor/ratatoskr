# Phase 3a: Provider Consolidation Proposal

**Date**: March 2026
**Status**: Implemented (steps 1-8 complete)
**Prerequisite for**: Graph provider (Phase 3b/3c)

---

## Context

Gmail and JMAP both exist as Rust providers. The question is what (if any) consolidation to do before adding Graph as a third provider. This document proposes concrete answers to the four open design decisions.

### Current architecture (the real one, not the planned one)

**Rust layer**: Two independent provider modules with nearly identical shapes:

| | Gmail | JMAP |
|---|---|---|
| **State** | `GmailState { RwLock<HashMap<String, GmailClient>>, key }` | `JmapState { RwLock<HashMap<String, JmapClient>>, key }` |
| **Client** | `GmailClient { Arc<ClientInner> }` with `RwLock<TokenState>` for OAuth refresh | `JmapClient { Arc<jmap_client::Client> }`, immutable (Basic auth) |
| **Sync ctx** | `SyncCtx { client, account_id, db, body_store, search, app_handle }` | Identical struct |
| **Sync result** | `GmailSyncResult { new_inbox_message_ids, affected_thread_ids }` | `JmapSyncResult { new_inbox_email_ids, affected_thread_ids }` |
| **Commands** | 23, `gmail_*` prefix | 25, `jmap_*` prefix |
| **Actions** | `gmail_modify_thread(add_labels, remove_labels)` — generic | `jmap_archive`, `jmap_trash`, etc. — one per action |

**TS layer**: Three-path dispatch in `emailActions.ts`:
- Gmail → `executeViaGmailRust()` → `invoke('gmail_*')`
- JMAP → `executeViaJmapRust()` → `invoke('jmap_*')`
- IMAP → `executeViaImapProvider()` → TS `EmailProvider` interface → Tauri IMAP/SMTP commands

Local DB mutations are already provider-agnostic (`email_action_*` commands). The `EmailProvider` TS interface exists but is only used for IMAP dispatch. Gmail and JMAP bypass it.

---

## Decision 1: Narrow Rust trait for common operations + separate states

### Recommendation: Extract a `ProviderOps` trait for the ~10 common operations. Keep `GmailState`, `JmapState`, `GraphState` as separate Tauri managed states with their own auth/client lifecycles.

### What the trait covers (and what it doesn't)

The trait **does not** unify:
- State ownership or initialization (`GmailState` vs `JmapState` vs `GraphState`)
- Client types or auth lifecycles (OAuth refresh, static credentials, OAuth+semaphore)
- Provider-specific APIs (`gmail_list_drafts`, `jmap_discover_url`, etc.)

The trait **does** unify:
- The ~10 common email operations that every provider must support
- The return types for those operations (standardized `SyncResult`, `()` for actions)
- The dispatch surface (one `match` in the router, not one `match` per command)

### The trait

```rust
// provider/ops.rs
use async_trait::async_trait;

#[async_trait]
pub trait ProviderOps: Send + Sync {
    // Sync
    async fn sync_initial(&self, ctx: &SyncCtx<'_>, days_back: u32) -> Result<SyncResult, String>;
    async fn sync_delta(&self, ctx: &SyncCtx<'_>) -> Result<SyncResult, String>;

    // Actions (thread-level)
    async fn archive(&self, ctx: &SyncCtx<'_>, thread_id: &str) -> Result<(), String>;
    async fn trash(&self, ctx: &SyncCtx<'_>, thread_id: &str) -> Result<(), String>;
    async fn permanent_delete(&self, ctx: &SyncCtx<'_>, thread_id: &str) -> Result<(), String>;
    async fn mark_read(&self, ctx: &SyncCtx<'_>, thread_id: &str, read: bool) -> Result<(), String>;
    async fn star(&self, ctx: &SyncCtx<'_>, thread_id: &str, starred: bool) -> Result<(), String>;
    async fn spam(&self, ctx: &SyncCtx<'_>, thread_id: &str, is_spam: bool) -> Result<(), String>;
    async fn move_to_folder(&self, ctx: &SyncCtx<'_>, thread_id: &str, folder_id: &str) -> Result<(), String>;
    async fn add_tag(&self, ctx: &SyncCtx<'_>, thread_id: &str, tag_id: &str) -> Result<(), String>;
    async fn remove_tag(&self, ctx: &SyncCtx<'_>, thread_id: &str, tag_id: &str) -> Result<(), String>;

    // Send + Drafts
    async fn send_email(&self, ctx: &SyncCtx<'_>, raw_base64url: &str, thread_id: Option<&str>) -> Result<String, String>;
    async fn create_draft(&self, ctx: &SyncCtx<'_>, raw_base64url: &str, thread_id: Option<&str>) -> Result<String, String>;
    async fn update_draft(&self, ctx: &SyncCtx<'_>, draft_id: &str, raw_base64url: &str, thread_id: Option<&str>) -> Result<(), String>;
    async fn delete_draft(&self, ctx: &SyncCtx<'_>, draft_id: &str) -> Result<(), String>;

    // Attachments
    async fn fetch_attachment(&self, ctx: &SyncCtx<'_>, message_id: &str, attachment_id: &str) -> Result<Vec<u8>, String>;

    // Folders
    async fn list_folders(&self, ctx: &SyncCtx<'_>) -> Result<Vec<ProviderFolder>, String>;
}
```

### How providers implement it

Each provider wraps its own client and delegates to existing functions:

```rust
// gmail/ops.rs
pub struct GmailOps {
    pub client: GmailClient,  // Arc<ClientInner> with RwLock<TokenState>
}

#[async_trait]
impl ProviderOps for GmailOps {
    async fn archive(&self, ctx: &SyncCtx<'_>, thread_id: &str) -> Result<(), String> {
        // Calls existing gmail::api::modify_thread with remove_labels=["INBOX"]
        self.client.modify_thread(thread_id, &[], &["INBOX"], ctx.db).await
    }
    // ...
}
```

```rust
// jmap/ops.rs
pub struct JmapOps {
    pub client: JmapClient,  // Arc<jmap_client::Client>
}

#[async_trait]
impl ProviderOps for JmapOps {
    async fn archive(&self, ctx: &SyncCtx<'_>, thread_id: &str) -> Result<(), String> {
        // Calls existing jmap::api::archive
        self.client.archive(thread_id).await
    }
    // ...
}
```

### Why this works where `Box<dyn EmailProvider>` doesn't

The trait is narrow: it covers **operations**, not **lifecycle**. Each provider still:
- Manages its own `*State` as a Tauri managed state
- Handles its own auth (OAuth refresh, static credentials, etc.)
- Exposes provider-specific commands under its own prefix
- Uses its own client type internally

The router resolves account → provider → `&dyn ProviderOps` at call time, not at startup:

```rust
// provider/router.rs
fn get_ops(
    provider: &str,
    account_id: &str,
    gmail: &GmailState,
    jmap: &JmapState,
    // graph: &GraphState,  — added in Phase 3b
) -> Result<Box<dyn ProviderOps>, String> {
    match provider {
        "gmail_api" => {
            let client = gmail.get(account_id)?;
            Ok(Box::new(GmailOps { client }))
        }
        "jmap" => {
            let client = jmap.get(account_id)?;
            Ok(Box::new(JmapOps { client }))
        }
        // "graph" => { ... }  — Phase 3b
        "imap" => Err("IMAP uses TS provider path".into()),
        other => Err(format!("Unknown provider: {other}")),
    }
}
```

### What this means for Graph

Graph implements `ProviderOps` in `graph/ops.rs`. Adding Graph means:
1. Add `GraphOps { client: GraphClient }` implementing the trait
2. Add one arm to `get_ops()` match
3. Done — all `provider_*` commands work for Graph automatically

Compare this with the v1 proposal where Graph required adding a new match arm to **every** `provider_*` command function.

---

## Decision 2: Provider-agnostic commands via trait dispatch

### Recommendation: Add ~17 provider-agnostic Tauri commands that dispatch through `ProviderOps`. Keep existing `gmail_*` and `jmap_*` commands for provider-specific features only.

### The new commands

```rust
// provider/commands.rs — thin wrappers that resolve provider and call trait methods

#[tauri::command]
pub async fn provider_archive(
    account_id: String, thread_id: String,
    db: State<'_, DbState>, gmail: State<'_, GmailState>, jmap: State<'_, JmapState>,
    body_store: State<'_, BodyStoreState>, search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let provider = get_provider_type(&db, &account_id)?;
    let ops = get_ops(&provider, &account_id, &gmail, &jmap)?;
    let ctx = SyncCtx::new(&account_id, &db, &body_store, &search, &app_handle);
    ops.archive(&ctx, &thread_id).await
}
```

Since every command follows this same pattern (resolve provider → get ops → build ctx → call trait method), we can use a macro to reduce boilerplate:

```rust
macro_rules! provider_command {
    ($name:ident, $method:ident $(, $param:ident: $ty:ty)*) => {
        #[tauri::command]
        pub async fn $name(
            account_id: String, $($param: $ty,)*
            db: State<'_, DbState>, gmail: State<'_, GmailState>, jmap: State<'_, JmapState>,
            body_store: State<'_, BodyStoreState>, search: State<'_, SearchState>,
            app_handle: AppHandle,
        ) -> Result<(), String> {
            let provider = get_provider_type(&db, &account_id)?;
            let ops = get_ops(&provider, &account_id, &gmail, &jmap)?;
            let ctx = SyncCtx::new(&account_id, &db, &body_store, &search, &app_handle);
            ops.$method(&ctx, $(&$param,)*).await
        }
    };
}

provider_command!(provider_archive, archive, thread_id: String);
provider_command!(provider_trash, trash, thread_id: String);
// etc.
```

### Full command list

```
// Sync
provider_sync_initial(account_id, days_back)
provider_sync_delta(account_id)

// Actions (thread-level)
provider_archive(account_id, thread_id)
provider_trash(account_id, thread_id)
provider_permanent_delete(account_id, thread_id)
provider_mark_read(account_id, thread_id, read)
provider_star(account_id, thread_id, starred)
provider_spam(account_id, thread_id, is_spam)
provider_move_to_folder(account_id, thread_id, folder_id)
provider_add_tag(account_id, thread_id, tag_id)
provider_remove_tag(account_id, thread_id, tag_id)

// Send + Drafts
provider_send_email(account_id, raw_base64url, thread_id?)
provider_create_draft(account_id, raw_base64url, thread_id?)
provider_update_draft(account_id, draft_id, raw_base64url, thread_id?)
provider_delete_draft(account_id, draft_id)

// Attachments
provider_fetch_attachment(account_id, message_id, attachment_id)

// Folders
provider_list_folders(account_id)
```

### Why keep the `gmail_*`/`jmap_*` commands

Some operations are provider-specific with no cross-provider equivalent:
- `gmail_list_drafts`, `gmail_fetch_send_as`, `gmail_get_history` — Gmail-only APIs
- `jmap_discover_url` — JMAP auto-discovery
- `gmail_list_threads`, `gmail_get_thread` — Gmail's native thread API (no equivalent in JMAP/Graph)
- Future Graph-specific: `graph_get_profile` with Graph-specific fields

These stay as provider-prefixed commands. The `provider_*` commands handle the common operations via the trait.

### TS changes

`emailActions.ts` simplifies from three dispatch paths to one:

```typescript
// Before: 3 provider-specific dispatchers
if (provider === "gmail_api") executeViaGmailRust(action);
else if (provider === "jmap") executeViaJmapRust(action);
else executeViaImapProvider(action);

// After: 1 generic dispatcher (for Gmail, JMAP, Graph)
// IMAP still uses TS provider path until it's ported to Rust
if (provider === "imap") {
  executeViaImapProvider(action);
} else {
  await invoke(`provider_${action.type}`, { accountId, ...params });
}
```

The `executeViaGmailRust` and `executeViaJmapRust` functions can be deleted.

---

## Decision 3: Keep raw MIME boundary — Graph adapts internally via create-draft-then-send

### Recommendation: Graph accepts `raw_base64url` like Gmail and JMAP, then internally creates a draft (parsing MIME → JSON) and sends it.

### Status: Recommended boundary. Implementation complexity TBD pending Graph API verification.

### Rationale

1. **The current contract works for all existing providers.** Gmail accepts raw MIME natively. JMAP accepts it via `Email/import`. IMAP sends via SMTP (raw MIME). Changing this contract to structured input would require modifying all four providers and the TS composer.

2. **The MIME→JSON conversion is Graph's problem, not the trait's.** Graph's `/me/sendMail` endpoint wants JSON, but the create-draft path (`POST /me/messages` with MIME content) can accept MIME. The two-call path (create draft → send draft) is well-documented and gives us the sent message ID (which `/me/sendMail` doesn't return).

3. **Structured send input is a larger rewrite with unclear benefit.** The TS composer already builds MIME. Moving to structured input means either: (a) the composer builds both MIME and structured data (duplication), or (b) all providers accept structured data and serialize internally (reimplementing MIME construction in Rust for Gmail/JMAP/IMAP). Neither is worth it.

### What Graph does internally

```rust
// graph/api.rs
pub async fn send_email(
    &self, raw_base64url: &str, thread_id: Option<&str>, db: &DbState,
) -> Result<GraphSendResult, String> {
    // 1. Decode raw MIME from base64url
    // 2. Parse MIME to extract: from, to, cc, bcc, subject, body (html/text), attachments
    // 3. POST /me/messages — create draft from parsed data (JSON body)
    //    - Returns draft message ID
    // 4. POST /me/messages/{id}/send — send the draft
    //    - We already have the message ID from step 3
    Ok(GraphSendResult { id: draft_id })
}
```

The MIME parsing step uses `mail-parser` (already a dependency for IMAP).

### Known complexity areas

The "~50-80 lines" estimate from v1 was optimistic for full feature parity. Areas that need verification against the Graph API:

- **Inline attachments**: MIME `Content-Disposition: inline` with `Content-ID` → Graph's `contentBytes` + `isInline` + `contentId` mapping
- **Multipart alternatives**: `text/plain` + `text/html` → Graph's `body.contentType` selection
- **Reply threading**: `In-Reply-To` / `References` headers → Graph's `conversationId` linkage
- **Address normalization**: RFC 5322 `"Display Name" <email>` → Graph's `{name, address}` objects
- **Draft parity**: Draft create/update must preserve the same fields that send uses

A realistic estimate is 100-200 lines of adapter code. The `mail-parser` crate handles MIME parsing; the complexity is in mapping parsed fields to Graph's JSON schema. This should be verified during Phase 3b implementation with integration tests against the Graph API sandbox.

### What stays the same

- `provider_send_email(account_id, raw_base64url, thread_id?)` — same signature for all providers (via trait)
- TS composer builds raw MIME — no changes
- `provider_create_draft` / `provider_update_draft` — same `raw_base64url` input

---

## Decision 4: Split tag operations from folder moves — clean semantic boundary

### Recommendation: Replace the overloaded `addLabel`/`removeLabel` with three distinct operations: `add_tag`, `remove_tag`, and `move_to_folder`. Each has clear, consistent semantics across providers.

### Why the v1 "provider-interpreted addLabel" approach was wrong

In v1, `provider_add_label("INBOX")` meant:
- Gmail: add a label
- JMAP: add a mailbox
- IMAP: move to folder
- Graph: move to folder

And `provider_add_label("user-thing")` meant:
- Gmail: add a label
- JMAP: add a mailbox
- IMAP: no-op
- Graph: if `graph-cat-` prefix → add category, else → move to folder

That's not "one operation with provider-specific implementation." That's multiple distinct operations hidden behind one name, with a `graph-cat-` prefix leaking provider encoding into a supposedly generic contract.

### The three operations

#### `add_tag(thread_id, tag_id)` / `remove_tag(thread_id, tag_id)`

**Semantics**: Add/remove a lightweight classification to a thread. The thread's location does not change.

| Provider | What "tag" means | Example |
|----------|------------------|---------|
| Gmail | User label | `Label_123` |
| JMAP | Keyword | `$flagged`, custom keywords |
| IMAP | IMAP flag | `\Flagged`, custom flags |
| Graph | Category | `"Blue category"`, `"Red category"` |

**What tags are NOT**: System labels (INBOX, TRASH, SPAM, SENT). Those are handled by dedicated action commands (`provider_archive`, `provider_trash`, `provider_spam`). Tags are user-created classifications.

#### `move_to_folder(thread_id, folder_id)`

**Semantics**: Move a thread to a different folder/container. Already exists as a separate operation.

| Provider | What happens |
|----------|-------------|
| Gmail | Add folder label, remove INBOX label |
| JMAP | Change mailbox membership |
| IMAP | IMAP MOVE command |
| Graph | `POST /me/messages/{id}/move` |

### What this means for the local DB

The local DB already separates these concepts:
- `thread_labels` table stores both labels and folder membership
- `email_action_add_label` / `email_action_remove_label` — tag operations
- `email_action_move_to_folder` — folder move

The `provider_*` commands align with this existing separation.

### What this means for the UI

The label picker in the sidebar/move dialog can show:
- Gmail: labels (full list from `gmail_list_labels`)
- JMAP: mailboxes (from `jmap_list_mailboxes`)
- IMAP: folders (from `imap_list_folders`)
- Graph: folders for `move_to_folder`, categories for `add_tag`/`remove_tag`

The UI is already partially provider-aware (it shows different things for Gmail vs IMAP). Making the tag/folder split explicit makes the UI logic cleaner, not harder.

---

## Shared types

```rust
// provider/types.rs

pub struct SyncResult {
    pub new_inbox_message_ids: Vec<String>,
    pub affected_thread_ids: Vec<String>,
}

pub struct SyncCtx<'a> {
    pub account_id: &'a str,
    pub db: &'a DbState,
    pub body_store: &'a BodyStoreState,
    pub search: &'a SearchState,
    pub app_handle: &'a AppHandle,
}

pub struct ProviderFolder {
    pub id: String,
    pub name: String,
    pub path: String,
    pub special_use: Option<String>,  // "inbox", "sent", "trash", "spam", "drafts", "archive"
}
```

---

## Implementation Order

1. ~~**Add `provider/types.rs`**~~ — `SyncResult`, `ProviderCtx`, `ProviderFolder`, `AttachmentData` ✅
2. ~~**Add `provider/ops.rs`**~~ — `ProviderOps` trait (17 async methods via `async-trait`) ✅
3. ~~**Add `gmail/ops.rs`**~~ — `GmailOps` implementing `ProviderOps` ✅
4. ~~**Add `jmap/ops.rs`**~~ — `JmapOps` implementing `ProviderOps` ✅
5. ~~**Add `provider/router.rs`**~~ — `get_provider_type()` DB lookup + `get_ops()` dispatch ✅
6. ~~**Add `provider/commands.rs`**~~ — 17 `provider_*` Tauri commands ✅
7. ~~**Register commands in `lib.rs`**~~ ✅
8. ~~**Simplify `emailActions.ts`**~~ — `executeViaProviderRust()` replaces Gmail+JMAP dispatchers ✅
9. **Build Graph** (Phase 3b) — add `GraphOps` + one arm in `get_ops()`

Steps 1-8 are Phase 3a (complete). Step 9 is Phase 3b.

### Actual scope

| Step | Lines | Notes |
|------|-------|-------|
| 1. `provider/types.rs` | 43 | `ProviderCtx` instead of `SyncCtx` (clearer name) |
| 2. `provider/ops.rs` | 92 | `async-trait` for dyn dispatch |
| 3. `gmail/ops.rs` | 211 | Delegates to `GmailClient` methods |
| 4. `jmap/ops.rs` | 304 | Delegates to existing JMAP helpers (made `pub(crate)`) |
| 5. `provider/router.rs` | 42 | |
| 6. `provider/commands.rs` | 370 | No macro — explicit commands for clarity, `#[allow(too_many_arguments)]` |
| 7. `lib.rs` registration | 17 | |
| 8. `emailActions.ts` | -139 net | Deleted `executeViaGmailRust` + `executeViaJmapRust` |
| **Total** | ~1080 new Rust, -139 net TS | |

The commands file ended up larger than estimated (~370 vs ~100) because we used explicit command functions instead of a macro. Each command is ~20 lines of boilerplate (resolve provider, get ops, build ctx, call trait method). This is more readable and debuggable than a macro, and the boilerplate is mechanical — adding Graph still only requires one `GraphOps` impl + one router arm.

---

## What we explicitly don't do

1. **No state unification** — `GmailState`, `JmapState`, `GraphState` stay as separate Tauri managed states. The trait covers operations, not lifecycle.
2. **No structured send input** — raw MIME stays as the contract. Graph adapts internally. Complexity is acknowledged and scoped to Phase 3b.
3. **No overloaded label semantics** — `addLabel`/`removeLabel` are replaced with `add_tag`/`remove_tag` (lightweight classification) and `move_to_folder` (location change). No `graph-cat-` prefix hacks.
4. **No IMAP migration to Rust routing** — IMAP still uses the TS `EmailProvider` interface. Porting IMAP to `ProviderOps` is a separate effort.

---

## Changes from v1

| Area | v1 | v2 | Why |
|------|----|----|-----|
| Rust trait | Rejected entirely | Narrow `ProviderOps` for common ops | Avoids N×M match boilerplate; Graph adds one impl instead of N match arms |
| Dispatch | Match per command function | Single `get_ops()` → trait dispatch | One place to add a new provider |
| Label semantics | `addLabel`/`removeLabel` provider-interpreted | `add_tag`/`remove_tag` + `move_to_folder` | Eliminates `graph-cat-` hack, clear semantic boundary |
| Graph send complexity | "~50-80 lines" | "100-200 lines, verify in Phase 3b" | Honest about inline attachments, multipart, threading |
| Command boilerplate | ~350 lines manual | ~100 lines via macro | Trait enables mechanical generation |
