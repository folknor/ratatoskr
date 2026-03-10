# Microsoft Graph → Rust Migration Plan

**Date**: March 2026
**Status**: Deferred (next major migration phase; blocked on consolidation decisions from phases 1 and 2)
**Goal**: Implement Microsoft Graph Mail API as a Rust-native email provider. This is step 3 in the execution order from `docs/rust-provider-crate-research.md`.

Unlike Gmail (migrating existing TS → Rust) and JMAP (new provider, no production TS code), Graph is the first provider planned after multiple Rust providers already exist. In practice, that means phase 3 now starts with consolidation work: extract the shared Rust provider shape from Gmail + JMAP, decide whether commands become provider-agnostic, and then implement the Graph provider itself.

---

## What Changed After Phases 1 And 2

This document was planned before the Gmail and JMAP migrations were actually implemented. Parts of the original plan assumed infrastructure and abstractions that do not exist yet, or that landed in a different form. This section is the correction layer. Treat it as the current baseline for Graph work.

### 1. Trait extraction did not happen as part of phases 1 and 2

The original framing of Graph was: Gmail and JMAP land first, then a shared Rust `EmailProvider` trait is extracted, and Graph becomes the first provider built against that trait.

That is **not** the current repo state.

- Rust still has provider-specific state and command surfaces:
  - `gmail_*` commands backed by `GmailState`
  - `jmap_*` commands backed by `JmapState`
- There is **no shared Rust provider trait** in the current tree.
- There are **no provider-agnostic Tauri commands** yet.

What this means for phase 3:

- Graph cannot be treated as "just implement the trait" work.
- There is now an explicit **phase 3a prerequisite** before Graph itself:
  - extract the Rust trait from real Gmail + JMAP code
  - decide whether the Tauri command layer becomes provider-agnostic now or later
  - resolve the label/mailbox/folder operation boundary before Graph depends on it
- If we skip that step, Graph becomes a third one-off Rust provider with its own `graph_*` surface, and we defer reconciliation again.

In other words: the document should no longer describe trait extraction as something effectively done by earlier phases. It is still outstanding work.

### 2. Shared HTTP/token infrastructure exists, but in a narrower form than this plan assumed

The original Graph plan assumed Gmail phase 1 would naturally leave behind a fairly complete shared provider infrastructure:

- `provider/token.rs` with generalized refresh and coalesced refresh state
- `provider/http.rs` with a reusable API client builder and retry middleware
- `provider/message.rs` for shared RFC 5322 construction

What actually landed is more limited:

- `provider/token.rs` **was generalized** enough to support arbitrary token endpoints via `refresh_oauth_token(...)`
- `provider/http.rs` **does exist**, but it is a light shared utility:
  - `build_http_client()`
  - retry-delay computation helper
- Gmail still owns most of its real request behavior in `gmail/client.rs`
- There is **no** shared `provider/message.rs`

Implications for Graph:

- The Graph plan should stop assuming a ready-made `build_api_client()` abstraction with middleware-level behavior already extracted.
- Graph will either:
  - hand-roll a client similarly to Gmail, reusing the thin shared pieces that exist now, or
  - first do a small infrastructure extraction step to promote Gmail's request/retry behavior into something truly shared
- The message-composition layer is not available in Rust today, so any Graph send design that depends on shared Rust MIME building is a new architectural change, not reuse of existing infrastructure.

### 3. OAuth is no longer blocked on multi-provider generalization

The old plan treated generalized OAuth as a prerequisite that should "probably already be done" after Gmail.

That assumption is stale. The current codebase already has most of the generic OAuth plumbing needed for Graph:

- Rust OAuth token exchange and refresh commands already accept arbitrary token URLs and client details
- the local callback server is provider-neutral
- TS OAuth provider config already includes Microsoft endpoints and scopes for Outlook IMAP/SMTP OAuth

So the remaining Graph-related OAuth work is **not**:

- "generalize OAuth first"

It is:

- decide the Graph-specific app registration model
- choose Graph scopes/endpoints for Mail API access
- wire a Graph account setup flow on top of the generic OAuth plumbing
- decide whether Graph uses `/common`, `/organizations`, or `/consumers`
- validate tenant behavior against the app-registration strategy

That is a materially smaller and more concrete problem than the document currently implies.

### 4. The send contract discussion must start from the real current boundary

The implemented Gmail migration explicitly kept RFC 5322 composition in TypeScript:

- TS composer builds raw MIME
- Rust provider commands accept pre-built `raw_base64url`

JMAP was implemented against that same boundary.

So the current provider boundary is:

- **composition in TS**
- **transport/provider dispatch in Rust**

This matters because the Graph plan currently discusses options as if shared Rust-side message construction either exists already or is the obvious next layer. It does not exist today.

That changes how the Graph send decision should be framed:

- **Option A**: keep the current app-wide boundary and make Graph adapt from raw MIME internally
  - this preserves existing TS composer behavior
  - but forces Rust to parse/adapt MIME into Graph's JSON model or a draft-send flow
- **Option B**: deliberately redesign the provider contract around structured send input
  - this is a cross-provider architectural rewrite, not a local Graph detail
  - Gmail and JMAP would need to be moved to the new contract too
- **Option C**: rely on undocumented raw MIME behavior in Graph
  - lowest migration cost if it works
  - highest product risk

The important correction is: Graph is not choosing between existing abstractions. It is either adapting to the current MIME boundary or forcing a new boundary across all Rust providers.

### 5. The auth-method normalization issue — DONE

Fixed via migration v24 (Rust) / v26 (TS): `UPDATE accounts SET auth_method = 'oauth2' WHERE auth_method = 'oauth'`. No longer a Graph blocker.

### 6. Recommended reframing of phase 3

Given the repo state after phases 1 and 2, phase 3 should be thought of as three pieces, not one:

1. **Phase 3a: Rust provider consolidation — DONE**
   - `ProviderOps` trait extracted from Gmail + JMAP (17 async methods)
   - Provider-agnostic `provider_*` Tauri commands route via `get_ops()` → `Box<dyn ProviderOps>`
   - Raw MIME send boundary kept; Graph adapts internally
   - `add_tag`/`remove_tag` + `move_to_folder` replaces overloaded `addLabel`/`removeLabel`
   - `emailActions.ts` simplified from 3 dispatch paths to 2 (Rust providers + IMAP TS)
   - See `docs/phase-3a-proposal.md` for design decisions and rationale
2. **Phase 3b: Graph Rust provider**
   - Graph client
   - Graph sync engine
   - Graph actions (implement `ProviderOps` for `GraphOps`)
   - Graph folder/category mapping
   - Add one arm to `get_ops()` in `provider/router.rs`
3. **Phase 3c: Graph TS/UI integration**
   - account setup
   - syncManager routing (already simplified — just add `"graph"` to non-IMAP path)
   - startup/init behavior

---

## Table of Contents

- [What Changed After Phases 1 And 2](#what-changed-after-phases-1-and-2)
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

1. **Depends on provider consolidation** — Gmail and JMAP now both exist in Rust, but the shared Rust trait and provider-agnostic command layer were not extracted as part of phases 1 and 2. Graph is the provider that forces that decision.
2. **Generic OAuth plumbing already exists; Graph-specific OAuth decisions remain** — the callback/token-exchange plumbing is already provider-neutral, and TS already has Microsoft OAuth config. What remains is Graph-specific app registration, scopes, tenant strategy, and account-setup integration.
3. **Folder-centric model is the hardest to reconcile** — Gmail is label-centric (messages have multiple labels). JMAP uses mailboxes (messages can belong to multiple mailboxes). Graph is folder-centric (messages live in exactly one folder). This is the most restrictive model and the hardest to map onto our Gmail-style label UI. Seeing how the trait handles Gmail labels vs JMAP mailboxes will inform how Graph folders fit.
4. **Lower priority user base** — Outlook.com/Exchange users can already connect via IMAP+OAuth2 (see quick win in [What We Defer](#what-we-defer)). Graph adds richer features (categories, delta sync, focused inbox) but is not a blocker for basic access.

---

## Prerequisites

These must be complete or in progress before Graph work begins:

### 1. Gmail Rust migration (complete)

All Gmail API logic lives in Rust. Patterns established:
- `provider/token.rs` — generalized token refresh exists via `refresh_oauth_token(...)`, though Gmail still owns most refresh coordination in `gmail/client.rs`
- `provider/http.rs` — thin shared HTTP utilities exist (`build_http_client()`, retry-delay helper), but not a full shared API client abstraction
- `GmailState` — `RwLock<HashMap<String, GmailClient>>` pattern for Tauri-managed state
- Sync progress events — `app_handle.emit("*-sync-progress", ...)`
- Pending-ops conflict check in delta sync

### 2. JMAP Rust migration (both phases complete)

JMAP provider exists in Rust AND has been validated through TS integration (Phase 2). Patterns established:
- Mailbox/folder → label mapping in Rust (`jmap/mailbox_mapper.rs`)
- Thread-level action semantics — enumerate emails, batch-mutate (JMAP has no thread-level mutations, same as Graph)
- `JmapState` — same `RwLock<HashMap>` pattern
- Basic auth client lifecycle (Graph uses OAuth, not Basic, but the state management pattern transfers)

The important consequence is that Graph should use Gmail + JMAP as the extraction source for the shared Rust provider shape, rather than assuming that extraction already happened as part of the earlier migrations.

### 3. Shared `EmailProvider` trait extraction

This is still outstanding work. Before the Graph provider itself starts, phase 3 should decide whether to extract a shared Rust `EmailProvider` trait and provider-agnostic routing from the current Gmail + JMAP implementations.

If that extraction happens first, the trait should cover:
- Client lifecycle (`init`/`remove`)
- Sync (initial + delta)
- Email actions (archive, trash, star, read, move, delete)
- Send + drafts
- Attachment download

**Critical caveat on label/folder semantics**: Even if the trait is extracted from Gmail + JMAP, Graph's folder-centric model may still force the abstraction to change. Gmail labels are multi-label, JMAP mailboxes are multi-mailbox, but Graph messages live in exactly one folder, so "add label" may mean either "add category" or "move to folder" depending on the label type. This means:

- The trait as extracted from Gmail + JMAP may not be the right abstraction for Graph's label/folder operations.
- Graph may be the provider that forces the trait to split `add_label`/`remove_label` into more granular operations (e.g., `move_to_folder` + `add_category`), or to accept provider-specific interpretation of label semantics.
- This is expected. Graph being the first consumer of the trait is precisely what validates whether the abstraction is correct. If the trait needs to change, better to discover it now than after a fourth provider.

See [Folder-to-Label Mapping: What this means for the trait](#what-this-means-for-the-trait) for the full analysis.

If the trait is NOT extracted before Graph work begins, Graph becomes another one-off `graph_*`-prefixed provider, and we'll have three providers to reconcile later. This is explicitly the scenario we're trying to avoid.

### 4. OAuth generalized for multi-provider

This is mostly satisfied already at the plumbing level. The callback server and token exchange/refresh commands are provider-neutral, and TS already includes Microsoft OAuth provider configuration for Outlook IMAP/SMTP flows.

What remains before Graph starts is provider-specific:

- choose Graph OAuth scopes/endpoints for Mail API access
- decide tenant strategy (`/common` vs `/consumers` vs `/organizations`)
- decide the app-registration model
- wire Graph account setup on top of the existing OAuth plumbing

### 5. `auth_method` column normalization — DONE

Resolved. Migration v26 (TS) / v24 (Rust) normalizes all existing `'oauth'` values to `'oauth2'`. All runtime code already checks `"oauth2"`. Mocks updated.

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

---

## Open Questions

These must be resolved before the command surface, sync contracts, or trait assumptions are treated as stable. The Phase 1/2 designs in this document are provisional sketches — they assume specific resolutions to these questions and will need to be updated once decisions are made.

### 1. App registration model

Gmail uses user-provided Client IDs (configured in Settings). Microsoft Graph requires an Azure AD app registration. Options:
- **Ship a default app registration** — simpler for users, but we'd need to manage it (including publisher verification for organizational accounts).
- **User provides their own** — same as Gmail, but Azure portal is more complex than Google Cloud Console.
- **Both** — ship a default for personal accounts, allow override for organizational.

### 2. Folder-centric to label-centric mapping

Graph messages live in exactly one folder. Our UI is label-centric (threads can have multiple labels). See [Folder-to-Label Mapping](#folder-to-label-mapping) for detailed analysis. This is a product decision, not just an adapter detail.

### 3. Thread identity model

Graph has a `conversationId` field that groups related messages, but it's not as reliable as Gmail's threading. Graph also has `conversationIndex` (binary threading data from Exchange). Options:
- Use `conversationId` as `threadId` (simplest, may produce different groupings than users expect).
- Use our JWZ threading algorithm on `Message-ID`/`References`/`In-Reply-To` headers (more accurate, more work — reusing `src/services/threading/threadBuilder.ts` logic ported to Rust).
- Use `conversationId` as primary, fall back to JWZ for edge cases.

**This decision has cascading effects throughout the plan.** The command surface (thread-level actions accept `thread_id`), the sync engine (delta sync resolves messages to thread IDs for pending-ops checks), and the queue coordination (pending_operations uses `thread_id` as `resource_id`) all depend on what "thread ID" means for Graph. The command signatures themselves are stable — they take `thread_id` regardless. But the internal implementation of `query_thread_message_ids()`, the sync conflict check, and the thread-to-message enumeration all change depending on this choice:

- **If conversationId**: enumeration is `$filter=conversationId eq '{id}'` on the Graph API, or `WHERE thread_id = ?` locally. Simple.
- **If JWZ**: thread IDs are locally computed. There is no Graph API filter for "messages in this JWZ thread." Enumeration MUST be local DB. Thread IDs must be computed during sync parse and stored in the `messages` table.
- **If hybrid**: need fallback logic and a way to know which mode a given thread uses.

Investigate `conversationId` reliability across real Outlook.com and Exchange Online accounts before deciding. This is a blocking decision for implementation.

### 4. Rate limit handling

Graph allows only **4 concurrent requests per app per mailbox**. This is far more restrictive than Gmail (10+ parallel) and changes the sync architecture:

- Gmail: parallel `getThread()` at concurrency=10 via Semaphore
- JMAP: server-side batching (50 IDs per request), no parallel fetch needed
- Graph: max 2-3 concurrent requests (leave headroom for user-initiated actions during sync)

The sync engine must use `tokio::sync::Semaphore` with a much lower permit count. Per-folder delta sync is already serial by nature (one delta query per folder, paginated), but initial sync fetching individual messages needs throttling.

Additionally: 10,000 API requests per 10 minutes per app per mailbox. This is generous for delta sync but could be hit during large initial syncs. Track request count and back off if approaching the limit.

### 5. Shared trait readiness — DECIDED (Phase 3a complete)

The `ProviderOps` trait has been extracted from Gmail + JMAP. Graph will be the first provider built against it. See `docs/phase-3a-proposal.md` for the full design.

Key files:
- `src-tauri/src/provider/ops.rs` — trait definition (17 async methods)
- `src-tauri/src/provider/router.rs` — `get_ops()` resolves account → `Box<dyn ProviderOps>`
- `src-tauri/src/provider/commands.rs` — 17 `provider_*` Tauri commands
- `src-tauri/src/gmail/ops.rs` — `GmailOps` implementing the trait
- `src-tauri/src/jmap/ops.rs` — `JmapOps` implementing the trait

Graph implementation: add `graph/ops.rs` with `GraphOps` implementing `ProviderOps`, add one arm to `get_ops()`.

### 6. Send and draft contract — DECIDED

**Decision: Option A (create-draft-then-send).** Graph accepts `raw_base64url` like Gmail and JMAP. Internally uses `POST /me/messages` (MIME → JSON) then `POST /me/messages/{id}/send`. This preserves the trait contract. See `docs/phase-3a-proposal.md` Decision 3 for rationale and known complexity areas (inline attachments, multipart, threading, address normalization — estimated 100-200 lines of adapter code, verify during Phase 3b).

### 7. Thread action scope: local vs remote enumeration

When a user archives/trashes/stars a thread, should the action affect only locally-synced messages, or all messages in the conversation on the server?

**The problem is specific to Graph.** Initial sync is windowed by `days_back`. Folder sync is prioritized (some folders only synced every 20th cycle). The 4-concurrent limit makes syncing slower. This means a Graph account will routinely have threads where some messages exist locally and others don't. For Gmail and JMAP, this is a minor edge case. For Graph, it's the normal state.

**Options:**

- **A. Remote enumeration (whole-conversation mutation)**: Thread-level actions query the Graph API to enumerate all messages in the conversation: `GET /me/messages?$filter=conversationId eq '{id}'&$select=id`. This guarantees the action applies to every message on the server, including ones we haven't synced. Costs an extra API call per action (counted against the 4-concurrent limit). If `conversationId` filter isn't supported on all endpoints, falls back to local DB.
- **B. Local enumeration (mutate only synced messages)**: Query the local DB: `SELECT id FROM messages WHERE thread_id = ? AND account_id = ?`. Faster, no API call. But older or unsynced messages remain untouched on the server. A user who archives a thread may find unarchived messages in the same conversation next time they check Outlook. This is a behavioral regression from what users expect — "archive" should mean "the whole conversation is archived."

This is a product decision. JMAP uses local enumeration (Option B) but JMAP's sync model is global (all emails, not per-folder), so the partial-sync problem is less severe.

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

- **Send format**: Graph's `/me/sendMail` accepts a JSON message object, NOT raw RFC 822. This breaks the current provider send contract — see [Open Question 6](#6-send-and-draft-contract).
- **Large attachments**: Files >3MB require upload sessions (`/me/messages/{id}/attachments/createUploadSession`). This is a multi-step process unlike Gmail/JMAP where attachments are part of the message payload.
- **OData pagination**: All list endpoints use `@odata.nextLink` / `@odata.deltaLink`. Need a generic `ODataCollection<T>` wrapper struct with `#[serde(rename = "@odata.nextLink")]`.
- **Focused Inbox**: Graph exposes `inferenceClassification` (Focused/Other). Could map to our category system. Optional enrichment.
- **`$select` efficiency**: Graph supports `$select` to request only specific fields. This is critical for performance — a full `Message` object is large. Always use `$select` to request only the fields we need (id, subject, from, toRecipients, receivedDateTime, body, conversationId, flag, categories, parentFolderId, isRead, isDraft, hasAttachments, internetMessageHeaders).
- **`internetMessageHeaders`**: Not included in default responses — must be explicitly requested via `$select`. These contain `Message-ID`, `References`, `In-Reply-To`, `Authentication-Results`, `List-Unsubscribe`. Essential for threading, auth display, and unsubscribe.
- **No `mail-builder` for send**: Unlike Gmail and JMAP, `provider/message.rs` (`mail-builder`) is NOT used for sending via Graph. Graph needs its own message-to-JSON serializer — or the create-draft-then-send path. See [Open Question 6](#6-send-and-draft-contract).

---

## Current State

### No production TS code

Like JMAP, there is no existing production TypeScript Graph implementation. The Exchange assessment doc (`docs/microsoft-exchange-assessment.md`) is research only. There is nothing to migrate from — this is a new provider, same as JMAP.

### DB schema additions needed

```sql
-- New table: per-folder delta tokens
CREATE TABLE graph_folder_delta_tokens (
    account_id TEXT NOT NULL,
    folder_id TEXT NOT NULL,           -- Opaque Graph folder ID (e.g., AAMkAGI2TG93AAA=)
    delta_link TEXT NOT NULL,          -- @odata.deltaLink URL
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    PRIMARY KEY (account_id, folder_id),
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);
```

Also:
- `accounts` table may need a `graph_user_id` column (Graph uses a user principal name, not just email)
- Accounts with `provider = "graph"` use `auth_method` (must be normalized first — see [Prerequisite 5](#5-auth_method-column-normalization))
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
├── folder_mapper.rs    # Graph folder → Gmail-style label mapping (opaque IDs + well-known resolution)
├── sync.rs             # Per-folder delta sync + initial sync
└── commands.rs         # Tauri commands (graph_* or provider-agnostic)
```

### Infrastructure reused from Gmail + JMAP migrations

| Module | What Graph uses |
|--------|---------------|
| `provider/token.rs` | `TokenState`, `refresh_oauth_token()` — with Microsoft token endpoint |
| `provider/http.rs` | `build_api_client()` — reqwest-middleware with retry (respects 429 + `Retry-After`) |
| `provider/message.rs` | See [Open Question 6](#6-send-and-draft-contract). May be used if create-draft-then-send requires MIME parsing. NOT used for direct send. |
| `db/` | All DB write commands — `upsert_thread()`, `set_thread_labels()`, `upsert_message()`, etc. |
| `body_store/` | `body_store_put()`, `body_store_get()` — same compressed body storage |
| `search/` | `index_message()` — same Tantivy indexing |

### Tauri command surface (provisional)

**These signatures depend on the resolution of Open Questions 3, 6, and 7. They will change once those decisions are made.**

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
// thread_id meaning depends on Open Question 3 (conversationId vs JWZ)
graph_archive(account_id, thread_id)
graph_trash(account_id, thread_id)
graph_permanent_delete(account_id, message_ids)
graph_mark_read(account_id, thread_id, read)
graph_star(account_id, thread_id, starred)       // maps to flag.flagStatus
graph_spam(account_id, thread_id, is_spam)        // move to Junk folder
graph_move_to_folder(account_id, thread_id, folder_id)
graph_add_category(account_id, thread_id, category)
graph_remove_category(account_id, thread_id, category)

// Send + drafts — signature depends on Open Question 6
// Option A (create-draft-then-send): takes structured message data
// Option B (split trait): takes structured message data
// Option C (raw MIME): takes raw_base64url like Gmail/JMAP
graph_send_email(account_id, ???)
graph_create_draft(account_id, ???)
graph_update_draft(account_id, draft_id, ???)
graph_delete_draft(account_id, draft_id)

// Attachments
graph_fetch_attachment(account_id, message_id, attachment_id)

// Profile
graph_get_profile(account_id)
```

~22 commands. The send/draft input format is unresolved.

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

**Note**: The code samples in this section are provisional sketches. They assume specific resolutions to the open questions (conversationId for threading, create-draft-then-send for sending, remote enumeration for thread actions). These will need to be updated once the open questions are resolved.

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

No new account columns needed — Graph accounts use `provider = "graph"` and existing encrypted token columns. `auth_method` value depends on normalization (see [Prerequisite 5](#5-auth_method-column-normalization)).

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
```

### 1c. Folder mapper (`graph/folder_mapper.rs`)

Maps Graph folders to Gmail-style label IDs. Analogous to JMAP's `mailbox_mapper.rs` and IMAP's `folderMapper.ts`.

**Key design point**: Graph folders have opaque IDs (e.g., `AAMkAGI2TG93AAA=`), NOT human-readable names. The Graph API accepts well-known name aliases in URL paths (e.g., `GET /me/mailFolders/inbox`), but the actual folder objects returned have opaque `id` fields, and `parentFolderId` on messages uses the opaque ID. The mapper must work with opaque IDs at runtime and resolve well-known names during folder sync.

**Well-known folder resolution must NOT rely on display name matching.** Display names can be localized (e.g., "Éléments envoyés" for Sent Items in French), user-renamed, or duplicated. Instead, resolve each well-known alias by calling `GET /me/mailFolders/{alias}` directly — this returns the canonical folder object with its opaque ID. Then match those opaque IDs against the general folder list to tag system folders.

```rust
/// Well-known folder aliases that Graph accepts as URL path segments.
/// These are NOT folder IDs and NOT display names — they are API-level
/// aliases that resolve to the canonical system folder regardless of locale.
const WELL_KNOWN_ALIASES: &[(&str, &str, &str)] = &[
    // (graph_alias,     label_id,    label_name)
    ("inbox",           "INBOX",     "Inbox"),
    ("drafts",          "DRAFT",     "Drafts"),
    ("sentitems",       "SENT",      "Sent"),
    ("deleteditems",    "TRASH",     "Trash"),
    ("junkemail",       "SPAM",      "Spam"),
    ("archive",         "archive",   "Archive"),
];

/// Built during folder sync. Maps opaque folder IDs to label info.
/// This is the runtime lookup table — all message processing uses this.
pub struct FolderMap {
    /// opaque_folder_id → FolderLabelMapping
    by_id: HashMap<String, FolderLabelMapping>,
    /// label_id → opaque_folder_id (reverse lookup for actions)
    by_label: HashMap<String, String>,
}

pub struct FolderLabelMapping {
    pub folder_id: String,       // opaque Graph ID
    pub label_id: String,        // Gmail-style label ID
    pub label_name: String,
    pub label_type: &'static str,  // "system" or "user"
    pub well_known_alias: Option<String>,  // "inbox", "sentitems", etc. if this is a system folder
}

impl FolderMap {
    /// Build the folder map from the full folder tree.
    ///
    /// Resolution strategy (two phases):
    ///
    /// Phase 1 — Resolve well-known aliases to opaque IDs:
    ///   For each alias in WELL_KNOWN_ALIASES, call
    ///   GET /me/mailFolders/{alias} (e.g., /me/mailFolders/inbox).
    ///   This returns the canonical folder object with its opaque ID.
    ///   Collect into a HashMap<opaque_id, (alias, label_id, label_name)>.
    ///   If a request 404s (e.g., "archive" doesn't exist on all accounts),
    ///   skip — that system folder doesn't exist for this mailbox.
    ///
    /// Phase 2 — Merge with the full folder tree:
    ///   Walk the complete folder list (including recursively fetched children).
    ///   For each folder, check if its opaque ID was resolved in Phase 1.
    ///   If yes: system folder — use the label_id from WELL_KNOWN_ALIASES.
    ///   If no: user folder — use "graph-{folderId}" as label_id,
    ///          displayName as label_name.
    ///
    /// This never matches on displayName. Localized, renamed, or
    /// duplicated folder names cannot cause misclassification.
    pub async fn build(
        client: &GraphClient,
        all_folders: &[GraphMailFolder],
        db: &DbState,
    ) -> Result<Self, String>;

    /// Look up a folder's label info by its opaque ID.
    /// Used during message parsing (parentFolderId → label).
    pub fn get_by_folder_id(&self, folder_id: &str) -> Option<&FolderLabelMapping>;

    /// Resolve a Gmail-style label ID to an opaque folder ID.
    /// Used by action commands (e.g., "archive" → resolve archive folder ID).
    pub fn resolve_folder_id(&self, label_id: &str) -> Option<&str>;

    /// Derive label IDs from a message's folder + categories + flags.
    pub fn get_labels_for_message(
        &self,
        parent_folder_id: &str,
        categories: &[String],
        is_read: bool,
        flag_status: &str,
    ) -> Vec<String>;
}
```

The `FolderMap` is built once during initial sync (folder fetch phase, after the full tree is traversed) and cached in `GraphClient` or `GraphState`. It's rebuilt when `graph_list_folders` detects changes. All runtime lookups use opaque IDs — well-known aliases are only used during the Phase 1 resolution step of `build()`.

### 1d. Message parsing (`graph/parse.rs`)

Converts Graph `Message` response to our internal DB-ready struct:

```rust
pub fn parse_graph_message(
    msg: &GraphMessage,
    folder_map: &FolderMap,
) -> Result<ParsedGraphMessage, String>;
```

**Graph vs Gmail/JMAP parsing differences**:
- Body comes as a string (HTML or text) in `body.content`. No base64 decoding, no MIME part walking.
- `uniqueBody` provides the deduped body (excludes quoted replies). Could use for body store if reliable.
- `internetMessageHeaders` must be explicitly requested via `$select`. Contains `Message-ID`, `References`, `In-Reply-To`, `Authentication-Results`, `List-Unsubscribe`.
- Thread ID derivation depends on [Open Question 3](#3-thread-identity-model). Provisionally uses `conversationId`.
- `categories` are supplementary labels (not mailbox membership like JMAP).
- `flag.flagStatus` maps to STARRED pseudo-label (`"flagged"` → STARRED, `"notFlagged"` → no STARRED).
- `isRead` directly maps to UNREAD pseudo-label (inverted: `!isRead` → UNREAD).
- `parentFolderId` determines the primary folder label — resolved via `FolderMap.get_by_folder_id()` using the opaque ID.
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

### 1f. Email action commands (provisional)

Each action maps to Graph REST calls. Thread-level actions enumerate messages first (see [Thread-Level Action Semantics](#thread-level-action-semantics)).

**Archive** — move all thread messages from inbox to archive folder:
```rust
#[tauri::command]
pub async fn graph_archive(
    account_id: String, thread_id: String,
    db: State<'_, DbState>, graph: State<'_, GraphState>,
) -> Result<(), String> {
    let client = graph.get(&account_id).await?;
    let folder_map = client.folder_map();

    // 1. Resolve archive folder's opaque ID
    let archive_id = folder_map.resolve_folder_id("archive")
        .ok_or("No archive folder found")?;

    // 2. Enumerate messages in thread (method depends on Open Question 7)
    let message_ids = query_thread_message_ids(&client, &thread_id, &db).await?;

    // 3. Batch move via /$batch (up to 20 per batch request)
    batch_move_messages(&client, &message_ids, archive_id, &db).await?;
    Ok(())
}
```

**JSON batching for thread-level actions**: Graph has no batch mutation endpoint for mail, but supports [JSON batching](https://learn.microsoft.com/en-us/graph/json-batching) — up to 20 requests in a single POST to `/$batch`:

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

Graph returns attachment content in two ways depending on size:
- **Small attachments** (<3MB typically): `contentBytes` field is populated with base64-encoded data in the JSON response.
- **Large attachments** (≥3MB): `contentBytes` is `null`/absent. The raw bytes must be fetched separately via `GET /me/messages/{id}/attachments/{id}/$value`.

The download command must branch on the response shape:

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

    let data = if let Some(ref content_bytes) = attachment.content_bytes {
        // Small attachment: contentBytes is base64-encoded inline
        BASE64_STANDARD.decode(content_bytes)
            .map_err(|e| format!("Failed to decode attachment content: {}", e))?
    } else {
        // Large attachment: fetch raw bytes via /$value endpoint
        let raw = client.get_bytes(
            &format!("me/messages/{}/attachments/{}/$value", message_id, attachment_id),
            &db,
        ).await?;
        if raw.is_empty() {
            return Err(format!(
                "Attachment {} has no contentBytes and /$value returned empty",
                attachment_id
            ));
        }
        raw
    };

    Ok(AttachmentData {
        data: BASE64_STANDARD.encode(&data),
        size: data.len(),
    })
}
```

This requires a `get_bytes()` method on `GraphClient` that returns raw `Vec<u8>` instead of deserializing JSON. Large attachment upload uses session-based upload (deferred to post-MVP).

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

1. **OAuth sign-in**: User clicks "Sign in with Microsoft" → launch OAuth2 flow (same as Gmail: open browser → localhost redirect → token exchange). On success, save account to DB with `provider = "graph"`, encrypted tokens. `auth_method` value must match whatever normalization was applied in [Prerequisite 5](#5-auth_method-column-normalization).
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

**This depends on the resolution of [Open Question 6](#6-send-and-draft-contract).**

The current provider contract is: composer builds raw RFC 822 MIME via TS, passes `rawBase64Url` to the provider's `sendMessage()`. Gmail and JMAP both accept this format.

Graph's `/me/sendMail` does NOT accept raw MIME. The composer→send path must change for Graph, and this affects the trait:

- **If Option A (create-draft-then-send)**: Rust accepts raw MIME, parses it, creates a Graph draft (JSON) from the parsed data, then sends the draft. The TS composer doesn't change — the adaptation happens in Rust. But the MIME→JSON conversion is non-trivial (inline images, multipart/alternative).
- **If Option B (structured message data)**: The trait's send interface changes from `rawBase64Url` to a structured message type. All providers adapt. The composer serializes to the new format. This is a broader change affecting Gmail and JMAP too.

Until this is decided, the composer change scope is unknown.

### 2e. App startup

In `App.tsx` startup sequence:
- `getAllAccounts()` → for Graph accounts, call `graph_init_client` (same pattern as Gmail/JMAP).
- Sync timer already handles multi-provider via `syncManager.ts` routing.

### Phase 2 deliverable

Graph accounts can be added via OAuth, synced, and acted on through the full UI. The sync timer includes Graph accounts. Email actions work through the offline queue.

---

## Thread-Level Action Semantics

Same fundamental problem as JMAP — see `docs/jmap-rust-migration.md` for the full design rationale.

### The problem

Our app's action model is thread-centric (archive a thread, star a thread, trash a thread). Graph has no thread-level mutations. All operations are per-message.

Two additional complications compared to JMAP:
1. **Thread identity is unsettled** — see [Open Question 3](#3-thread-identity-model). The enumeration strategy depends on whether thread IDs are `conversationId` values or JWZ-computed.
2. **Partial sync is the norm** — see [Open Question 7](#7-thread-action-scope-local-vs-remote-enumeration). Graph's windowed, per-folder sync means many threads will have messages not yet in the local DB.

### The design (provisional)

Thread-level Graph actions enumerate messages in the thread and mutate each one:

```rust
/// Get all message IDs in a thread.
/// The implementation of this function depends on Open Questions 3 and 7.
///
/// If using remote enumeration (Open Question 7, Option A):
///   GET /me/messages?$filter=conversationId eq '{thread_id}'&$select=id
///   This requires conversationId as the thread model (Open Question 3).
///
/// If using local enumeration (Option B):
///   SELECT id FROM messages WHERE thread_id = ? AND account_id = ?
///   Works with any thread model, but only affects synced messages.
///
/// If using JWZ threading (Open Question 3):
///   Remote enumeration is NOT possible (no Graph API for JWZ thread queries).
///   Must use local enumeration. This means JWZ + remote enumeration is
///   an incompatible combination — the thread model choice constrains
///   the enumeration strategy.
async fn query_thread_message_ids(
    client: &GraphClient,
    thread_id: &str,
    db: &DbState,
) -> Result<Vec<String>, String>;
```

Every thread-level action (`graph_archive`, `graph_trash`, `graph_star`, `graph_mark_read`, `graph_spam`, `graph_move_to_folder`, `graph_add_category`, `graph_remove_category`) calls this helper first, then applies the operation to all returned message IDs using JSON batching (up to 20 per batch request).

### Interaction between Open Questions 3 and 7

| Thread model (OQ 3) | Enumeration scope (OQ 7) | Viable? | Notes |
|---------------------|--------------------------|---------|-------|
| conversationId | Remote (Graph API) | **Yes** | `$filter=conversationId eq '{id}'` — whole-conversation mutation |
| conversationId | Local (DB only) | Yes | Fast, but partial for windowed sync |
| JWZ | Remote (Graph API) | **No** | No Graph API for "messages in this JWZ thread" |
| JWZ | Local (DB only) | Yes | Only option for JWZ, but always partial |
| Hybrid | Remote + local fallback | Partial | Remote for conversationId-based, local for JWZ-computed |

This table shows that **choosing JWZ threading forces local-only enumeration**, which means accepting the partial-mutation behavior described in [Open Question 7](#7-thread-action-scope-local-vs-remote-enumeration). This is a constraint on the thread model decision.

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

### The folder ID model

Graph folders have two identifier systems:

1. **Opaque IDs** (e.g., `AAMkAGI2TG93AAA=`) — the actual `id` field on folder objects, used in `parentFolderId` on messages, stored in `graph_folder_delta_tokens`. These are the runtime identifiers.
2. **Well-known aliases** (e.g., `inbox`, `sentitems`, `deleteditems`) — API-level shorthand that can be used in URL paths (`GET /me/mailFolders/inbox`). NOT the same as the opaque ID. NOT the same as the `displayName` (which is localizable and user-renamable). Used only during the `FolderMap.build()` resolution step (see [1c](#1c-folder-mapper-graphfolder_mapperrs)) by calling `GET /me/mailFolders/{alias}` directly and collecting the returned opaque ID.

All runtime operations — message parsing, label derivation, action dispatch, delta token tracking — use opaque IDs. Well-known aliases are resolved to opaque IDs once during folder sync via direct API calls and cached in the `FolderMap`. Display names are never used for system folder identification.

### Recommended approach: Hybrid (folder + categories)

1. **Folder → primary location label**: Each folder maps to a label. Well-known folders (identified by resolving aliases to opaque IDs) get system label IDs (`INBOX`, `SENT`, `TRASH`, `SPAM`, `DRAFT`, `archive`). User folders get `graph-{folderId}` label IDs. A message's `parentFolderId` (opaque ID) is resolved via the `FolderMap` to determine its one folder label.

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

Graph delta sync: must query each folder separately. `GET /me/mailFolders/{id}/messages/delta` for each folder. A typical account has 10-15 top-level folders, but nested subfolders can push the total to 20-50+. That's 20-50 API calls per full sync cycle if every folder is checked. With the 4-concurrent limit, these must be serialized or lightly parallelized. Folder sync ordering (see below) mitigates this by prioritizing high-traffic folders.

### Initial sync

1. **Folders** (full tree traversal):
   - `GET /me/mailFolders?$top=100` returns only top-level folders. Graph mailboxes can have arbitrary nesting — subfolders are NOT included in the top-level response.
   - For each folder with `childFolderCount > 0`, recursively fetch children: `GET /me/mailFolders/{id}/childFolders?$top=100`
   - Continue recursively until all levels are traversed.
   - Resolve well-known system folders by fetching each alias directly (see [1c](#1c-folder-mapper-graphfolder_mapperrs)).
   - Build `FolderMap` from the complete tree. Persist all folders as labels (nested user folders get `graph-{folderId}` label IDs regardless of depth).
   - The full folder tree is required because: delta tokens are per-folder (every folder needs one), `parentFolderId` on messages can reference any folder at any depth, sidebar rendering needs the tree, and move-target resolution must include subfolders.
2. **Messages per folder**: For each folder in the tree (prioritize Inbox, Sent, Drafts; see folder sync ordering below):
   - `GET /me/mailFolders/{opaque_id}/messages?$filter=receivedDateTime ge {sinceDate}&$select={fields}&$top=50&$orderby=receivedDateTime desc`
   - Paginate via `@odata.nextLink`
   - For each message: `parse_graph_message()` → DB writes (same pipeline as Gmail/JMAP)
   - Must request `internetMessageHeaders` for each message to get threading headers, auth results, unsubscribe headers
3. **Bootstrap delta tokens** (per-folder, must page to completion):
   - After the initial message fetch for a folder, establish a delta baseline: `GET /me/mailFolders/{opaque_id}/messages/delta?$select={fields}`
   - **This is NOT a single request.** The delta endpoint returns the full current state on first call. You MUST page through all `@odata.nextLink` responses until the server issues a final `@odata.deltaLink` (no more `nextLink`). Only the final `deltaLink` is valid for subsequent incremental sync.
   - Store the final `@odata.deltaLink` in `graph_folder_delta_tokens` keyed by the opaque folder ID.
   - For folders with many messages, this bootstrap pass may return thousands of entries. The messages themselves were already persisted in step 2 — the purpose of this pass is solely to obtain the delta token. An optimization: use `$select=id` to minimize payload since we don't need the message bodies again.
   - This must be done for every folder in the tree that we intend to delta-sync.

### Delta sync

1. **For each folder with a stored delta link**:
   - `GET {deltaLink}` (the stored `@odata.deltaLink` URL)
   - Paginate via `@odata.nextLink` if results span multiple pages
   - Returns created/updated messages (full objects) and deleted message IDs (with `@removed` annotation)
   - For each message, resolve thread ID (method depends on [Open Question 3](#3-thread-identity-model)), check `pending_operations` for that thread (see [Sync vs Queue: Write Ordering](#sync-vs-queue-write-ordering))
   - Parse and persist (same path as initial sync)
   - Delete removed messages from local DB
   - Store new `@odata.deltaLink` for next sync
2. **New folders**: Periodically re-traverse the full folder tree (same recursive strategy as initial sync) to detect new folders (including new subfolders at any depth). Any folder not yet in `graph_folder_delta_tokens` needs an initial message fetch + delta token bootstrap.
3. **Folder changes**: The re-traversal also detects renames and deletes. Rebuild `FolderMap` when changes detected. Remove delta tokens for deleted folders.

### Folder sync ordering

Not all folders need to be synced equally often. With nested folders, a typical mailbox may have 20-50+ folders — syncing all of them every 60s is not feasible under the 4-concurrent limit.

- **High priority** (every sync cycle): Inbox, Sent, Drafts
- **Medium priority** (every 5th sync cycle): Archive, Trash, Spam
- **Low priority** (every 20th sync cycle): Other user folders (including nested subfolders)
- **Folder tree re-traversal**: Every 10th sync cycle (discover new/renamed/deleted folders)

This reduces the per-cycle API call count to 3-5 for most sync cycles. The folder tree re-traversal adds O(depth) calls when it runs.

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

**Before overwriting a message's state during delta sync, resolve its thread ID and check `pending_operations` for that thread. If any pending ops exist for the thread, skip all messages in that thread — the queue processor will reconcile when the op flushes.**

### How thread ID resolution works

The thread ID resolution method depends on [Open Question 3](#3-thread-identity-model):

- **If conversationId**: `conversationId` is included in the Graph delta response — no extra lookup needed. Check `pending_operations WHERE resource_id = message.conversationId`.
- **If JWZ**: thread ID must be computed from message headers during parsing. The JWZ-computed `thread_id` is what gets stored in `messages.thread_id` and what `pending_operations.resource_id` references. Requires parsing `internetMessageHeaders` before the conflict check.
- **If hybrid**: use `conversationId` as the primary thread key for delta sync conflict checks. JWZ is only a fallback for edge cases where `conversationId` is missing.

In all cases, the check is a read from the same `ratatoskr.db` that both Rust and TS write to, consistent via SQLite's `Mutex<Connection>` serialization.

### The flow

1. Delta response includes messages (with `conversationId` and optionally `internetMessageHeaders`)
2. Resolve thread ID using the chosen thread model
3. For each message, check `pending_operations WHERE resource_id = {thread_id}`
4. If pending ops exist for the thread, skip all messages with that thread ID
5. Otherwise, persist normally

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

- **Unit tests**: Rust tests for `folder_mapper.rs`, `parse.rs`, `types.rs` (mock JSON from real Graph API responses). Test `FolderMap` resolution with real opaque IDs, not well-known aliases.
- **Integration tests**: Tauri command tests with mock HTTP server serving Graph API responses (`wiremock-rs`). OData pagination, delta responses with `@odata.nextLink`/`@odata.deltaLink`, `@removed` annotations, JSON batching responses, well-known folder alias resolution.
- **Real account testing**: Test against a personal Outlook.com account (free) and an Exchange Online account (if available). Delta sync round-trip, thread-level actions, attachment download, send.
- **Rate limit testing**: Verify Semaphore enforcement at concurrency=3. Simulate 429 responses with `Retry-After`.
- **Manual testing**: Graph account setup → OAuth → initial sync → delta sync → archive/trash/star/send → verify round-trip

### Estimated scope

| Phase | New Rust lines (est.) | New TS lines | Difficulty |
|-------|----------------------|-------------|------------|
| Phase 1: Full Rust provider | ~1,600-2,200 | ~0 | Moderate-High — hand-rolled REST (like Gmail), but per-folder delta sync and JSON batching add complexity |
| Phase 2: TS integration + UI | ~0 | ~600 (UI + thin adapters + sync wiring) | Low-Moderate — thin glue, but composer change scope depends on send contract decision |
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

### Phase 3a: Consolidation Work — DONE

All four items resolved and implemented. See `docs/phase-3a-proposal.md` for design decisions.

1. ~~**Shared Rust provider trait extraction**~~ — `ProviderOps` trait in `provider/ops.rs`, implemented by `GmailOps` and `JmapOps`. ✅
2. ~~**Provider-agnostic Tauri commands**~~ — 17 `provider_*` commands dispatch via `get_ops()` → `Box<dyn ProviderOps>`. ✅
3. ~~**Send contract decision**~~ — raw MIME boundary kept. Graph adapts internally (create-draft-then-send). ✅
4. ~~**Folder/label operation semantics**~~ — `add_tag`/`remove_tag` + `move_to_folder` replaces overloaded `addLabel`/`removeLabel`. ✅

### Graph-specific

5. **Graph OAuth account flow** — build the Graph account setup flow on top of the existing provider-neutral OAuth plumbing; choose scopes, endpoints, and tenant strategy appropriate for Graph Mail.
6. **Per-folder delta token storage** — `graph_folder_delta_tokens` table. Schema defined above.
7. **Graph-to-label mapping strategy** — product decision on folders + categories → labels. Preliminary design in [Folder-to-Label Mapping](#folder-to-label-mapping), needs validation with real accounts.
8. **Thread ID strategy** — `conversationId` vs JWZ threading. Needs investigation of `conversationId` reliability across real accounts. Blocks command surface finalization. See [Open Question 3](#3-thread-identity-model).
9. **Thread action scope decision** — remote vs local enumeration. Affects product semantics for partially-synced mailboxes. See [Open Question 7](#7-thread-action-scope-local-vs-remote-enumeration).
10. **Large attachment upload sessions** — multi-step upload for >3MB files. Not critical for initial implementation (can limit to inline/small attachments), but needed for full parity.
11. **Webhook subscriptions** — Graph supports push notifications via webhooks for real-time sync. Requires a reachable endpoint (problem for desktop apps). Polling via delta sync is the initial approach. Investigate if Tauri can expose a local webhook receiver via the existing localhost server.
12. **Azure AD app registration** — create and configure the app registration. Publisher verification for organizational access. Decide on default-shipped vs user-provided model.
13. **Focused Inbox integration** — map Graph's `inferenceClassification` to our category tabs (Primary/Other mapping). Optional enrichment after basic sync works.
14. **Exchange on-premises via EWS** — only if significant demand. `ews-rs` from Thunderbird provides types, but no client. SOAP/XML complexity is high. On-prem users can use IMAP.
15. **JSON batching optimization** — the `/$batch` endpoint supports up to 20 requests per batch. Investigate using it for initial sync (batch message fetches) in addition to thread-level actions.
16. **`$expand` for attachments** — `GET /me/messages/{id}?$expand=attachments` can inline attachment metadata in message responses. May eliminate separate attachment list calls during sync.
17. **`uniqueBody` usage** — Graph's `uniqueBody` field returns the message body without quoted replies. Could improve body store efficiency and thread display. Investigate reliability.
18. ~~**`auth_method` normalization**~~ — DONE. Migration v24/v26 normalizes to `"oauth2"`.

### Quick win (can happen before full Graph)

19. **IMAP + OAuth2 for Outlook.com** — add Microsoft OAuth2 flow, use XOAUTH2 SASL with our existing IMAP provider. Gives Outlook users immediate access without building the full Graph provider. Most of the provider-neutral OAuth plumbing already exists; remaining work is app registration, the Microsoft account flow, and IMAP account wiring. This is independent of the full Graph provider and could ship at any time.

---

## References

- `docs/phase-3a-proposal.md` — Phase 3a consolidation design decisions (ProviderOps trait, tag/folder split, send contract)
- ~~`docs/microsoft-exchange-assessment.md`~~ — removed (content folded into this doc)
- `docs/rust-provider-crate-research.md` — crate decisions and strategic plan (Graph endpoints table, architecture decisions)
- `docs/gmail-rust-migration.md` — Gmail patterns that Graph will follow (token management, reqwest setup, sync-with-DB-writes)
- `docs/jmap-rust-migration.md` — JMAP patterns (thread-level action semantics, mailbox mapping, trait extraction trigger)
