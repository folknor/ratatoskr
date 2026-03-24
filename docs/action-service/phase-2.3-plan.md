# Action Service: Phase 2.3 Detailed Plan

## Goal

Bring the send path through the action service so that clicking Send actually delivers the message to the provider. Today, Send builds MIME, inserts a `local_drafts` row with `sync_status = 'queued'`, closes the compose window, and shows "Message queued for sending." Nothing picks up the queued draft. The message is never sent.

Phase 2.3 closes the gap: the action service owns staging (validation, MIME build, local persistence) AND immediate dispatch (provider `send_email` call, draft lifecycle transitions). Draft auto-save and provider draft sync (`create_draft`/`update_draft`) are deferred — they are separate features that don't block sending.

## Current State

### What exists

1. **`handle_compose_send()`** (`crates/app/src/handlers/pop_out.rs:949-1110`) — validates recipients, builds `SendRequest`, calls `build_mime_message_base64url()`, inserts into `local_drafts` with `sync_status = 'queued'`, closes the compose window. No provider call.

2. **`SendRequest` + `build_mime_message()`** (`crates/core/src/send.rs`) — MIME construction from structured fields. 9 tests. Works.

3. **Draft lifecycle helpers** (`crates/core/src/send.rs:220-275`) — `mark_draft_sending(db, draft_id: String)`, `mark_draft_sent(db, draft_id: String, sent_message_id: String)`, `mark_draft_failed(db, draft_id: String)`. All take owned `String` args. Exist but are never called. `mark_draft_sending` validates the current state is in `{'pending', 'synced', 'finalized', 'failed'}` — it will reject a draft that doesn't already exist or is already in `'sending'`/`'sent'` state.

4. **Draft query helpers** (`crates/core/src/db/queries_extra/compose.rs:514-610`) — `db_save_local_draft()` (inserts with `sync_status = 'pending'`), `db_get_local_draft()`, `db_get_unsynced_drafts()`, `db_mark_draft_synced()`, `db_delete_local_draft()`. Exist but are never called.

5. **`ProviderOps::send_email()`** — implemented across all four providers, never called from app code. Takes `(ctx, raw_base64url, thread_id, mentions)`, returns `Result<String, ProviderError>` where the `String` is the provider-assigned sent message ID.

6. **`ProviderOps::create_draft()`/`update_draft()`/`delete_draft()`** — implemented, never called. `delete_draft` is relevant to this phase; the others are out of scope.

### What doesn't exist

- **No send dispatch.** Nothing calls `send_email()`. The "sync pipeline will pick up queued drafts" path was never built.
- **No draft auto-save.** `DRAFT_AUTO_SAVE_INTERVAL` (30s) is defined but unused. Compose state is in-memory only — closing without sending discards work.
- **No outbox/failed send UI.** If a send fails, there's no way for the user to see or retry it.
- **No mentions storage.** `local_drafts` has no column for `@-mentions`. `ProviderOps::send_email` accepts `mentions: &[(String, String)]` but compose doesn't populate them (the @-autocomplete feature doesn't exist yet per TODO).

### Orphaned `'queued'` drafts

The old path sets `sync_status = 'queued'`. After this migration, no code will ever set that status or poll for it. Existing `'queued'` rows from previous app runs are orphaned. These may be the only persisted copy of messages users believed were sent — deleting them silently would be data loss. They must be resurfaced, not discarded — see Step 8.

## Design Decisions

### Action service owns immediate dispatch, not deferred queue

The previous phases followed a pattern: action service does local DB mutation + immediate provider dispatch + returns `ActionOutcome`. Send should follow the same pattern, not introduce a separate queue worker.

**Why not a deferred queue:**
- A queue worker requires a periodic poll loop, retry policy, and shutdown persistence — all Phase 3/Phase 5 concerns.
- The user expects Send to either work or fail, not silently queue. The current "Message queued for sending" with no actual send is the worst of both worlds.
- Every other action service function dispatches immediately. A deferred queue is a different execution model that shouldn't be introduced mid-Phase-2.

**The flow:**
1. App builds `SendRequest` from compose state (stays in the app crate — compose state extraction is UI logic).
2. App calls `actions::send_email(ctx, send_request)`.
3. The action service: builds MIME (on `spawn_blocking` — may be CPU-heavy with large attachments), persists draft as `'pending'`, transitions to `'sending'`, calls `ProviderOps::send_email()`, marks `'sent'` or `'failed'`.
4. Returns `ActionOutcome` to the app. The app shows feedback and closes the compose window only on success.

### Two recovery models: in-window retry vs crash recovery

Phase 2.3 supports **in-window retry only**. Crash recovery is deferred.

**In-window retry (Phase 2.3):** If the provider call fails, the compose window is still open. The Send button is re-enabled. The user can edit and retry. The draft is in `'failed'` state in `local_drafts` — this is bookkeeping for the state machine, not a recovery mechanism. The user retries from the open compose window, not from the persisted draft.

**Crash recovery (Phase 3+):** If the app crashes between `mark_draft_sending()` and the provider response, the draft is stuck in `'sending'` state with no open compose window. Recovery requires: (a) detecting stale `'sending'` drafts on boot, (b) deciding whether to auto-retry or surface them in an outbox UI. This is Phase 3 (failure policy) territory. For now, the draft remains in `'sending'` and the user must re-compose if the app crashes mid-send.

These are distinct models. In-window retry uses the live compose state. Crash recovery uses the persisted draft. Phase 2.3 only implements the first.

### Use existing draft lifecycle helpers, not raw SQL

The existing helpers (`db_save_local_draft`, `mark_draft_sending`, `mark_draft_sent`, `mark_draft_failed`, `db_delete_local_draft`) form a state machine API. The action service uses them rather than writing raw SQL:

1. `db_save_local_draft(...)` — inserts the draft with `sync_status = 'pending'`. This stores the structured fields (to, cc, bcc, subject, body, etc.) and the finalized MIME in the `attachments` column.
2. `mark_draft_sending(db, draft_id)` — transitions to `'sending'`. Validates current state is non-terminal. Prevents duplicate sends.
3. On success: `mark_draft_sent(db, draft_id, sent_message_id)`.
4. On failure: `mark_draft_failed(db, draft_id)`.

**Note on `db_save_local_draft` field mapping:** `SendRequest` field names don't match `local_drafts` column names (`from` → `from_email`, `to` → `to_addresses`, `in_reply_to` → `reply_to_message_id`). The mapping happens at the call site in the action function, not inside `db_save_local_draft` (which takes column-named parameters). A comment at the call site documents the mapping.

### Compose window closure is deferred to outcome

The current code closes the compose window *before* the send completes (optimistic). This is wrong for immediate dispatch — if the send fails, the user has lost their compose state and has no way to fix and retry.

**New behavior:**
- On Send click: disable the Send button, show "Sending..." status in the compose window.
- On `Success`: close the compose window, show confirmation toast.
- On `Failed`: re-enable the Send button, show error in the compose window status area. The user can fix and retry.

This is a UX regression from the current "instant close" behavior if the provider is slow. That's the correct tradeoff — the alternative is losing messages.

### `ActionOutcome::Failed` for provider send failure, not `LocalOnly`

For thread-level actions (archive, star, label), `LocalOnly` means "the desired local state was achieved but the server wasn't notified — sync may revert it." For send, the desired outcome is message delivery, not local persistence. A `'failed'` draft is not the desired state. Therefore:

- MIME build failure → `Failed` (nothing happened)
- Draft persist failure → `Failed` (nothing happened)
- Provider creation failure → `Failed` (draft marked `'failed'`)
- Provider `send_email()` failure → `Failed` (draft marked `'failed'`)
- Provider `send_email()` success → `Success` (draft marked `'sent'`)

`LocalOnly` is not used for send. The draft persists in `'failed'` state regardless — it's bookkeeping for future outbox UI, not a meaningful local-only success.

### MIME build runs on `spawn_blocking`

`build_mime_message_base64url()` is synchronous and CPU-bound. With large attachments (multi-MB files), it can be slow and memory-intensive. It must run on `spawn_blocking` to avoid blocking the async runtime. The MIME build and the initial draft persist can share the same `spawn_blocking` call.

### Send uses `Message::SendCompleted`, not `ActionCompleted`

Send operates on a compose window, not the thread list. The existing `ActionCompleted` is thread-list-centric: it carries `outcomes: Vec<ActionOutcome>` (one per thread) and `rollback: Vec<(String, String, bool)>` (toggle state). Send has one outcome (not per-thread), no rollback, and needs compose-window context (window ID) that's irrelevant for the other 11 action types.

**Decision:** Use a dedicated message variant:

```rust
Message::SendCompleted {
    window_id: iced::window::Id,
    outcome: ActionOutcome,
}
```

This avoids polluting `ActionCompleted` with compose-specific fields. The handler for `SendCompleted` is in `handlers/pop_out.rs` alongside the other compose handlers, not in `handle_action_completed`.

### Send has its own dispatch method

`dispatch_action_service_with_params` collects selected threads from the thread list and loops over them. Send doesn't operate on thread selections — it operates on a compose window. Send gets its own dispatch method:

```rust
fn dispatch_send(
    &mut self,
    window_id: iced::window::Id,
    request: SendRequest,
) -> Task<Message> {
    let Some(ref action_ctx) = self.action_ctx else {
        // Show error in compose window status
        return Task::none();
    };
    let ctx = action_ctx.clone();
    Task::perform(
        async move {
            let outcome = ratatoskr_core::actions::send_email(&ctx, request).await;
            (window_id, outcome)
        },
        move |(window_id, outcome)| Message::SendCompleted { window_id, outcome },
    )
}
```

### `delete_draft` is forward-looking, not critical path

`delete_draft` is included in this phase because the draft context is fresh and the implementation is small. However, it has no call site in Phase 2.3 — auto-save doesn't exist yet, so there are no persisted drafts to discard. It becomes useful when auto-save or outbox UI land. It is not an exit criterion for making Send work.

### Draft auto-save boundary is intentional

The plan defers auto-save and declares it "local-only, no action service involvement." This is an intentional boundary choice, not an obvious one. The action service's stated purpose is to own *all* compose-related state mutations, which would include auto-save. The boundary is drawn here because:

- Auto-save is a write to `local_drafts` with no provider dispatch. The action service pattern (local + provider + structured outcome) is overhead for a local-only operation.
- Auto-save frequency (every 30s) would flood the action service with calls that always return `Success` and never need user feedback.
- If auto-save later needs provider draft sync, it can be wrapped in an action function at that point.

If this boundary proves wrong — if auto-save needs failure handling, deduplication, or outcome tracking — it can be pulled into the action service later without changing the `send_email` contract.

### Mentions are passed through, not stored

`SendRequest` doesn't currently have a `mentions` field. `ProviderOps::send_email()` takes `mentions: &[(String, String)]`. Since compose @-autocomplete doesn't exist yet, mentions are always empty. The action service passes `&[]` for now. When @-autocomplete is built, `SendRequest` gains a `mentions` field and the action service threads it through. No schema change needed — mentions are transient (only needed at send time, not for draft persistence).

### The `attachments` column stores MIME only for `'sending'`/`'sent'` drafts

The current code stores MIME base64url in the `attachments` column for queued drafts. This column name is misleading, but renaming it is a migration. Instead:
- For `'pending'` drafts (auto-save, future): `attachments` stores a JSON array of attachment metadata (filename, mime_type, size). The actual file bytes live in the attachment file cache (already in `ratatoskr-stores`).
- For `'sending'`/`'sent'` drafts: `attachments` stores the finalized MIME base64url, as it does today.
- The column serves dual purpose keyed on `sync_status`. This is documented, not renamed.

Phase 2.3 only creates `'sending'`/`'sent'`/`'failed'` drafts (no auto-save), so only the MIME usage matters for now.

## Action Function Signatures

```rust
// crates/core/src/actions/send.rs

/// Send an email: build MIME, persist draft, dispatch to provider.
///
/// On success, the provider-assigned sent message ID is stored in
/// `local_drafts.remote_draft_id` via `mark_draft_sent()` — the caller
/// does not need it. Returns plain `ActionOutcome::Success`.
/// On any failure (MIME build, DB, or provider), returns `Failed` and
/// marks the draft as `'failed'` if it was persisted.
pub async fn send_email(
    ctx: &ActionContext,
    request: SendRequest,
) -> ActionOutcome

/// Delete a local draft. If it has a remote_draft_id, also deletes
/// the server-side draft (best-effort).
///
/// Forward-looking: no call site in Phase 2.3 (no auto-save yet).
pub async fn delete_draft(
    ctx: &ActionContext,
    account_id: &str,
    draft_id: &str,
) -> ActionOutcome
```

`send_email` takes ownership of `SendRequest` (it's consumed by MIME build). `delete_draft` takes references (the draft may or may not exist on the server).

## Implementation Steps

### Step 1: Create `crates/core/src/actions/send.rs`

```rust
pub async fn send_email(
    ctx: &ActionContext,
    request: SendRequest,
) -> ActionOutcome {
    // 1. Build MIME + persist draft in one spawn_blocking call.
    // MIME build is CPU-bound (large attachments); draft persist is DB I/O.
    // Both are sync operations — combine them to avoid two spawn_blocking round-trips.
    let db = ctx.db.clone();
    let draft_id = request.draft_id.clone();
    let account_id = request.account_id.clone();
    let thread_id = request.thread_id.clone();
    let local_result = tokio::task::spawn_blocking(move || {
        // Build MIME
        let mime_base64url = build_mime_message_base64url(&request)
            .map_err(|e| format!("MIME build: {e}"))?;

        // Persist draft as 'pending' via existing helper.
        // Field mapping: SendRequest → local_drafts columns
        //   request.from       → from_email
        //   request.to         → to_addresses (joined)
        //   request.cc         → cc_addresses (joined)
        //   request.bcc        → bcc_addresses (joined)
        //   request.in_reply_to → reply_to_message_id
        //   mime_base64url      → attachments
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        conn.execute(
            "INSERT INTO local_drafts \
             (id, account_id, to_addresses, cc_addresses, bcc_addresses, \
              subject, body_html, reply_to_message_id, thread_id, \
              from_email, attachments, updated_at, sync_status) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, \
                     unixepoch(), 'pending')",
            rusqlite::params![
                draft_id,
                account_id,
                request.to.join(", "),
                request.cc.join(", "),
                request.bcc.join(", "),
                request.subject,
                request.body_html,
                request.in_reply_to,
                thread_id,
                request.from,
                mime_base64url,
            ],
        )
        .map_err(|e| format!("draft persist: {e}"))?;

        // Transition to 'sending' via existing lifecycle helper.
        // This validates the state machine (rejects already-sent drafts).
        let rows = conn.execute(
            "UPDATE local_drafts SET sync_status = 'sending' \
             WHERE id = ?1 AND sync_status IN ('pending', 'synced', 'finalized', 'failed')",
            rusqlite::params![draft_id],
        )
        .map_err(|e| format!("mark sending: {e}"))?;
        if rows == 0 {
            return Err(format!("Draft {draft_id} not found or already sending/sent"));
        }

        Ok(mime_base64url)
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))
    .and_then(|r| r);

    let mime_base64url = match local_result {
        Ok(mime) => mime,
        Err(e) => return ActionOutcome::Failed { error: e },
    };

    // 2. Provider dispatch
    let provider = match create_provider(&ctx.db, &account_id, ctx.encryption_key).await {
        Ok(p) => p,
        Err(e) => {
            log::warn!("Send failed (provider create): {e}");
            let _ = mark_draft_failed(&ctx.db, draft_id.clone()).await;
            return ActionOutcome::Failed { error: e };
        }
    };

    let provider_ctx = ProviderCtx {
        account_id: &account_id,
        db: &ctx.db,
        body_store: &ctx.body_store,
        inline_images: &ctx.inline_images,
        search: &ctx.search,
        progress: &NoopProgressReporter,
    };

    // Mentions are empty until @-autocomplete is built
    match provider.send_email(
        &provider_ctx, &mime_base64url, thread_id.as_deref(), &[],
    ).await {
        Ok(sent_message_id) => {
            let _ = mark_draft_sent(&ctx.db, draft_id, sent_message_id).await;
            ActionOutcome::Success
        }
        Err(e) => {
            let msg = e.to_string();
            log::warn!("Send failed for {account_id}: {msg}");
            let _ = mark_draft_failed(&ctx.db, draft_id).await;
            ActionOutcome::Failed { error: msg }
        }
    }
}
```

**Note on lifecycle helpers vs raw SQL:** Both `db_save_local_draft` and `mark_draft_sending` are async functions that take `&DbState`. Inside `spawn_blocking` we already hold the `Mutex<Connection>` lock, so we cannot call either async helper. The initial INSERT and the state-machine UPDATE are inlined with identical logic to their helper counterparts — same SQL, same state check, same column mapping. `mark_draft_sent` and `mark_draft_failed` are called outside `spawn_blocking` and use the existing async helpers directly.

`delete_draft` — lookup and delete in a single `spawn_blocking`:

```rust
pub async fn delete_draft(
    ctx: &ActionContext,
    account_id: &str,
    draft_id: &str,
) -> ActionOutcome {
    // 1. Look up remote_draft_id and delete locally in one call
    let db = ctx.db.clone();
    let did = draft_id.to_string();
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;

        let remote_id: Option<String> = conn.query_row(
            "SELECT remote_draft_id FROM local_drafts WHERE id = ?1",
            rusqlite::params![did],
            |row| row.get(0),
        )
        .ok()
        .flatten();

        conn.execute(
            "DELETE FROM local_drafts WHERE id = ?1",
            rusqlite::params![did],
        )
        .map_err(|e| format!("draft delete: {e}"))?;

        Ok(remote_id)
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))
    .and_then(|r| r);

    let remote_id = match local_result {
        Ok(id) => id,
        Err(e) => return ActionOutcome::Failed { error: e },
    };

    // 2. Provider delete (best-effort, only if remote_draft_id exists)
    if let Some(remote_draft_id) = remote_id {
        if let Ok(provider) = create_provider(&ctx.db, account_id, ctx.encryption_key).await {
            let provider_ctx = ProviderCtx {
                account_id,
                db: &ctx.db,
                body_store: &ctx.body_store,
                inline_images: &ctx.inline_images,
                search: &ctx.search,
                progress: &NoopProgressReporter,
            };
            if let Err(e) = provider.delete_draft(&provider_ctx, &remote_draft_id).await {
                log::warn!("Remote draft delete failed for {account_id}/{draft_id}: {e}");
                // Don't return Failed — the local delete succeeded and that's
                // what matters. The orphaned server draft will be cleaned up by sync.
            }
        }
    }

    ActionOutcome::Success
}
```

### Step 2: Register in `crates/core/src/actions/mod.rs`

```rust
mod send;
pub use send::{send_email, delete_draft};
```

### Step 3: Add `SendCompleted` message variant

In `crates/app/src/main.rs`:

```rust
Message::SendCompleted {
    window_id: iced::window::Id,
    outcome: ratatoskr_core::actions::ActionOutcome,
}
```

This is a dedicated variant, not added to `ActionCompleted`. Send operates on a compose window, not a thread list. `ActionCompleted` carries per-thread outcomes and toggle rollback data that are irrelevant for send.

Also add `DeleteDraft` to `CompletedAction` for future use (fire-and-report, same pattern as labels):

```rust
pub enum CompletedAction {
    // ... existing variants ...
    DeleteDraft,
}
```

`DeleteDraft`: `removes_from_view()` returns `false`, `success_label()` returns `"Draft discarded"`.

### Step 4: Add `dispatch_send` method

Send bypasses `dispatch_action_service_with_params` (which is thread-list-centric). Add a dedicated dispatch method in `handlers/pop_out.rs`:

```rust
fn dispatch_send(
    &mut self,
    window_id: iced::window::Id,
    request: SendRequest,
) -> Task<Message> {
    let Some(ref action_ctx) = self.action_ctx else {
        if let Some(PopOutWindow::Compose(state)) = self.pop_out_windows.get_mut(&window_id) {
            state.sending = false;
            state.status = Some("Send unavailable — action service not initialized".to_string());
        }
        return Task::none();
    };
    let ctx = action_ctx.clone();
    Task::perform(
        async move {
            let outcome = ratatoskr_core::actions::send_email(&ctx, request).await;
            (window_id, outcome)
        },
        move |(window_id, outcome)| Message::SendCompleted { window_id, outcome },
    )
}
```

### Step 5: Restructure `handle_compose_send`

The app handler becomes:

1. Validate recipients and from_account (stays in app — UI validation).
2. Build `SendRequest` from compose state (stays in app — compose state extraction).
3. Set `state.sending = true` (disables Send button, shows "Sending..." status).
4. Call `self.dispatch_send(window_id, request)`.

The current `handle_compose_send` does steps 1-2, builds MIME (moves to service), inserts into DB (moves to service), and closes the window (moves to completion handler). What remains in the app handler is validation, state extraction, and UI state management.

### Step 6: Add `sending` state to compose window

`ComposeState` needs a `sending: bool` field (default `false`). When `true`:
- Send button is disabled (or shows a spinner).
- Recipient/subject/body fields are read-only (prevent edits during send).
- Status area shows "Sending...".

### Step 7: Handle `SendCompleted`

In `main.rs` update dispatch, add a handler in `handlers/pop_out.rs`:

```rust
pub(crate) fn handle_send_completed(
    &mut self,
    window_id: iced::window::Id,
    outcome: &ActionOutcome,
) -> Task<Message> {
    match outcome {
        ActionOutcome::Success => {
            // Close compose window, show toast
            self.pop_out_windows.remove(&window_id);
            self.composer_is_open = false;
            self.status_bar.show_confirmation("Message sent".to_string());
            iced::window::close(window_id)
        }
        ActionOutcome::Failed { error } | ActionOutcome::LocalOnly { remote_error: error } => {
            // LocalOnly should not occur for send (send uses Failed for all
            // failures), but handle it defensively as failure for safety.
            if let Some(PopOutWindow::Compose(state)) = self.pop_out_windows.get_mut(&window_id) {
                state.sending = false;
                state.status = Some(format!("Send failed: {error}"));
            }
            Task::none()
        }
    }
}
```

### Step 8: Resurface orphaned `'queued'` drafts

Existing `'queued'` rows from the old code path were never sent. They may contain messages users believed were sent. On startup, transition them to `'failed'` so they're visible to the future outbox/failed-send UI:

```sql
UPDATE local_drafts SET sync_status = 'failed' WHERE sync_status = 'queued'
```

This runs once during app boot. The drafts are preserved with their full content (MIME in `attachments`, structured fields in other columns). When outbox UI is built, these will appear as failed sends that the user can retry or discard.

### Step 9: Delete legacy send code

- Remove the MIME build, DB insert, and window close from `handle_compose_send`. What remains: validation, `SendRequest` construction, `state.sending = true`, dispatch to action service.
- The `tokens_to_csv` helper may still be needed for other compose paths — check before deleting.

### Step 10: Wire `delete_draft` for future use

Register `delete_draft` in the action service. No call site in Phase 2.3. It becomes useful when auto-save or outbox UI land.

### Step 11: Verify

- `cargo check --workspace`
- `cargo clippy -p app -p ratatoskr-core`
- Verify `handle_compose_send` no longer calls `build_mime_message_base64url()` or inserts into `local_drafts` directly.
- Manual smoke test: send an email, verify it reaches the provider (check sent folder on server).
- Verify failed send re-enables the compose window (simulate by disconnecting network).
- Verify `mark_draft_sent`/`mark_draft_failed` are called (log output).
- Verify orphaned `'queued'` drafts are transitioned to `'failed'` on boot (not deleted).

## What This Produces

- `crates/core/src/actions/send.rs` — `send_email()` and `delete_draft()`
- Modified `crates/core/src/actions/mod.rs` — registers send module
- Modified `crates/app/src/main.rs` — `Message::SendCompleted`, `CompletedAction::DeleteDraft`
- Modified `crates/app/src/handlers/pop_out.rs` — `handle_compose_send` delegates to action service, `dispatch_send`, `handle_send_completed`
- Modified `crates/app/src/pop_out/compose.rs` — `sending: bool` field on `ComposeState`

## Exit Criteria

1. Clicking Send in compose dispatches through `actions::send_email()` which calls `ProviderOps::send_email()`.
2. Messages actually reach the provider. This is the first time `send_email()` has ever been called.
3. The compose window stays open during send. On success: window closes, toast shown. On failure: Send button re-enabled, error shown in compose.
4. `local_drafts` rows transition through `'pending'` → `'sending'` → `'sent'`/`'failed'` lifecycle using existing helpers.
5. Send failure returns `ActionOutcome::Failed`, not `LocalOnly`.
6. MIME build runs on `spawn_blocking` (not on the async runtime).
7. Orphaned `'queued'` drafts from the old path are transitioned to `'failed'` on boot (preserved, not deleted).
8. The app crate no longer builds MIME or inserts into `local_drafts` directly.
9. Workspace compiles and passes clippy.

**Not exit criteria** (forward-looking, no call site yet):
- `delete_draft()` exists in the service but has no wired call site until auto-save or outbox UI land.

## What Phase 2.3 Does NOT Do

- **Draft auto-save.** Local persistence of in-progress compose state. This is an intentional boundary choice: auto-save is local-only with no provider dispatch, so the action service pattern (local + provider + structured outcome) is overhead. If auto-save later needs failure handling or provider draft sync, it can be pulled into the action service without changing the `send_email` contract.
- **Provider draft sync.** Pushing local drafts to the server via `create_draft()`/`update_draft()`. Sync concern, not action concern.
- **Outbox/failed send UI.** Listing failed sends and offering retry. Needs UI design. The `'failed'` draft rows are there for when this UI is built.
- **Retry on failure.** Automatic retry of failed sends. Phase 3 (failure policy) or Phase 5 (retry).
- **Crash recovery.** Detecting stale `'sending'` drafts on boot and retrying them. Phase 3. Phase 2.3 only supports in-window retry (compose window still open after failure).
- **Mentions.** `@-autocomplete` doesn't exist. Mentions are `&[]` until it does.
- **Scheduled send.** Separate feature with its own deferred delivery mechanism. Out of scope.

## Resequencing Note

The implementation-phases doc noted: *"If the local staging vs remote dispatch semantics prove entangled with failure policy, this sub-phase may be better sequenced after Phase 3."*

This plan avoids that entanglement by choosing immediate dispatch over deferred queue. The `local_drafts` table is used for state-machine bookkeeping (preventing duplicate sends, recording outcomes), not as a durable outbox. Deferred queue semantics (retry, shutdown persistence, outbox UI) are explicitly punted to Phases 3 and 5.

If this decision proves wrong — if immediate dispatch causes UX problems (slow providers blocking the compose window, no offline send) — the path forward is: Phase 3 adds a background dispatch mode where `send_email()` returns after local persist and a worker handles provider dispatch. The action function signature doesn't change; the internal implementation does. The app-side code (compose window management, completion handling) doesn't change either.
