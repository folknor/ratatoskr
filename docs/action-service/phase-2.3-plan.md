# Action Service: Phase 2.3 Detailed Plan

## Goal

Bring the send path through the action service so that clicking Send actually delivers the message to the provider. Today, Send builds MIME, inserts a `local_drafts` row with `sync_status = 'queued'`, closes the compose window, and shows "Message queued for sending." Nothing picks up the queued draft. The message is never sent.

Phase 2.3 closes the gap: the action service owns staging (validation, MIME build, local persistence) AND immediate dispatch (provider `send_email` call, draft lifecycle transitions). Draft auto-save and provider draft sync (`create_draft`/`update_draft`) are deferred — they are separate features that don't block sending.

## Current State

### What exists

1. **`handle_compose_send()`** (`crates/app/src/handlers/pop_out.rs:949-1110`) — validates recipients, builds `SendRequest`, calls `build_mime_message_base64url()`, inserts into `local_drafts` with `sync_status = 'queued'`, closes the compose window. No provider call.

2. **`SendRequest` + `build_mime_message()`** (`crates/core/src/send.rs`) — MIME construction from structured fields. 9 tests. Works.

3. **Draft lifecycle helpers** (`crates/core/src/send.rs:220-275`) — `mark_draft_sending()`, `mark_draft_sent()`, `mark_draft_failed()`. Exist but are never called.

4. **Draft query helpers** (`crates/core/src/db/queries_extra/compose.rs:514-610`) — `db_save_local_draft()`, `db_get_local_draft()`, `db_get_unsynced_drafts()`, `db_mark_draft_synced()`, `db_delete_local_draft()`. Exist but are never called.

5. **`ProviderOps::send_email()`** — implemented across all four providers, never called from app code. Takes `(ctx, raw_base64url, thread_id, mentions)`, returns the sent message ID.

6. **`ProviderOps::create_draft()`/`update_draft()`/`delete_draft()`** — implemented, never called. Out of scope for this phase.

### What doesn't exist

- **No send dispatch.** Nothing calls `send_email()`. The "sync pipeline will pick up queued drafts" path was never built.
- **No draft auto-save.** `DRAFT_AUTO_SAVE_INTERVAL` (30s) is defined but unused. Compose state is in-memory only — closing without sending discards work.
- **No outbox/failed send UI.** If a send fails, there's no way for the user to see or retry it.
- **No mentions storage.** `local_drafts` has no column for `@-mentions`. `ProviderOps::send_email` accepts `mentions: &[(String, String)]` but compose doesn't populate them (the @-autocomplete feature doesn't exist yet per TODO).

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
3. The action service: builds MIME, persists to `local_drafts` as `'sending'`, calls `ProviderOps::send_email()`, marks `'sent'` or `'failed'`.
4. Returns `ActionOutcome` to the app. The app shows feedback and closes the compose window only on success.

**What about shutdown mid-flight?** If the app crashes between `mark_draft_sending()` and the provider response, the draft is stuck in `'sending'` state. On next boot, the app can detect this and either retry or surface it. That's Phase 3 (failure policy) territory. For now, the draft remains in `'sending'` and the user must re-compose. This is better than the current state where the draft is `'queued'` and the message is never sent.

### Compose window closure is deferred to outcome

The current code closes the compose window *before* the send completes (optimistic). This is wrong for immediate dispatch — if the send fails, the user has lost their compose state and has no way to fix and retry.

**New behavior:**
- On Send click: disable the Send button, show "Sending..." status in the compose window.
- On `Success`: close the compose window, show confirmation toast.
- On `LocalOnly` or `Failed`: re-enable the Send button, show error in the compose window status area. The user can fix and retry.

This is a UX regression from the current "instant close" behavior if the provider is slow. That's the correct tradeoff — the alternative is losing messages.

### MIME is built in the action service, not the caller

The current code builds MIME in the app handler. The action service should own MIME construction because:
- The service needs the MIME to send it. Passing pre-built MIME means the caller could pass stale MIME that doesn't match the draft state.
- If the service persists the draft before sending, it needs the structured `SendRequest` anyway (for auto-save, retry from draft fields, etc.).

The app passes `SendRequest`. The service calls `build_mime_message_base64url()` internally.

### The `attachments` column stores MIME only for `'sending'`/`'sent'` drafts

The current code stores MIME base64url in the `attachments` column for queued drafts. This column name is misleading, but renaming it is a migration. Instead:
- For `'pending'` drafts (auto-save, future): `attachments` stores a JSON array of attachment metadata (filename, mime_type, size). The actual file bytes live in the attachment file cache (already in `ratatoskr-stores`).
- For `'sending'`/`'sent'` drafts: `attachments` stores the finalized MIME base64url, as it does today.
- The column serves dual purpose keyed on `sync_status`. This is documented, not renamed.

Phase 2.3 only creates `'sending'`/`'sent'`/`'failed'` drafts (no auto-save), so only the MIME usage matters for now.

### Mentions are passed through, not stored

`SendRequest` doesn't currently have a `mentions` field. `ProviderOps::send_email()` takes `mentions: &[(String, String)]`. Since compose @-autocomplete doesn't exist yet, mentions are always empty. The action service passes `&[]` for now. When @-autocomplete is built, `SendRequest` gains a `mentions` field and the action service threads it through. No schema change needed — mentions are transient (only needed at send time, not for draft persistence).

### Draft delete goes through the service

Deleting a draft (discard from compose, or cleanup of `'sent'`/`'failed'` rows) should go through the action service for the same reasons as other actions. If the draft has a `remote_draft_id`, the service calls `ProviderOps::delete_draft()` to clean up the server-side draft.

### Draft auto-save and provider draft sync are NOT in scope

Auto-save (`save_draft()` → `db_save_local_draft()`) and provider draft sync (`create_draft()`/`update_draft()`) are deferred. They are independent features:
- Auto-save is local-only persistence — it doesn't need the action service pattern (no provider dispatch).
- Provider draft sync is a sync concern (periodic push of local draft state to the server), not an action concern.
- Neither blocks the immediate goal: making Send actually work.

When auto-save is built, it calls `db_save_local_draft()` directly (local-only, no action service involvement). When provider draft sync is built, it can be a sync worker or a separate action — that decision is deferred.

## Action Function Signatures

```rust
// crates/core/src/actions/send.rs

/// Send an email: build MIME, persist draft, dispatch to provider.
///
/// Returns Success with the sent message ID, LocalOnly if provider
/// failed (draft remains in 'failed' state), or Failed if MIME build
/// or local persistence failed.
pub async fn send_email(
    ctx: &ActionContext,
    request: SendRequest,
) -> ActionOutcome

/// Delete a local draft. If it has a remote_draft_id, also deletes
/// the server-side draft (best-effort).
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
    // 1. Build MIME
    let mime_base64url = match build_mime_message_base64url(&request) {
        Ok(encoded) => encoded,
        Err(e) => return ActionOutcome::Failed { error: format!("MIME build: {e}") },
    };

    // 2. Persist draft as 'sending' (prevents duplicate sends, provides crash recovery point)
    let db = ctx.db.clone();
    let draft_id = request.draft_id.clone();
    let account_id = request.account_id.clone();
    let thread_id = request.thread_id.clone();
    let mime_clone = mime_base64url.clone();
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        conn.execute(
            "INSERT INTO local_drafts \
             (id, account_id, to_addresses, cc_addresses, bcc_addresses, \
              subject, body_html, reply_to_message_id, thread_id, \
              from_email, attachments, updated_at, sync_status) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, \
                     unixepoch(), 'sending')
             ON CONFLICT(id) DO UPDATE SET
               attachments = ?11, sync_status = 'sending', updated_at = unixepoch()",
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
                mime_clone,
            ],
        )
        .map_err(|e| format!("draft persist: {e}"))?;
        Ok(())
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))
    .and_then(|r| r);

    if let Err(e) = local_result {
        return ActionOutcome::Failed { error: e };
    }

    // 3. Provider dispatch
    let provider = match create_provider(&ctx.db, &account_id, ctx.encryption_key).await {
        Ok(p) => p,
        Err(e) => {
            log::warn!("Send local-only (provider create failed): {e}");
            // Mark draft as failed so it's visible for retry
            let _ = mark_draft_failed(&ctx.db, draft_id).await;
            return ActionOutcome::LocalOnly { remote_error: e };
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
            ActionOutcome::LocalOnly { remote_error: msg }
        }
    }
}
```

`delete_draft`:

```rust
pub async fn delete_draft(
    ctx: &ActionContext,
    account_id: &str,
    draft_id: &str,
) -> ActionOutcome {
    // 1. Check if there's a remote draft to delete
    let db = ctx.db.clone();
    let did = draft_id.to_string();
    let remote_id = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        conn.query_row(
            "SELECT remote_draft_id FROM local_drafts WHERE id = ?1",
            rusqlite::params![did],
            |row| row.get::<_, Option<String>>(0),
        )
        .map_err(|e| format!("draft lookup: {e}"))
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))
    .and_then(|r| r);

    // 2. Delete locally
    let db = ctx.db.clone();
    let did = draft_id.to_string();
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        conn.execute(
            "DELETE FROM local_drafts WHERE id = ?1",
            rusqlite::params![did],
        )
        .map_err(|e| format!("draft delete: {e}"))?;
        Ok(())
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))
    .and_then(|r| r);

    if let Err(e) = local_result {
        return ActionOutcome::Failed { error: e };
    }

    // 3. Provider delete (best-effort, only if remote_draft_id exists)
    if let Ok(Some(remote_draft_id)) = remote_id {
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
                // Don't return LocalOnly — the local delete succeeded and that's
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

### Step 3: Add `Send` and `DeleteDraft` to `CompletedAction`

In `crates/app/src/main.rs`:

```rust
pub enum CompletedAction {
    // ... existing variants ...
    Send,
    DeleteDraft,
}
```

`Send` is not removes-from-view (compose window, not thread list) and not a toggle. `success_label()` returns `"Message sent"`. `DeleteDraft` returns `"Draft discarded"`.

### Step 4: Restructure `handle_compose_send`

The app handler becomes:

1. Validate recipients and from_account (stays in app — UI validation).
2. Build `SendRequest` from compose state (stays in app — compose state extraction).
3. Set compose window to "sending" state (disable Send button, show spinner).
4. Dispatch `actions::send_email(ctx, request)` via `Task::perform`.
5. On `ActionCompleted(Send, outcomes, _)`:
   - `Success` → close compose window, show "Message sent" toast.
   - `LocalOnly`/`Failed` → re-enable Send button, show error in compose status.

The current `handle_compose_send` does steps 1-2, builds MIME (moves to service), inserts into DB (moves to service), and closes the window (moves to completion handler). What remains in the app handler is validation, state extraction, and UI state management.

### Step 5: Add sending state to compose window

`ComposeState` needs a `sending: bool` field. When `true`:
- Send button is disabled (or shows a spinner).
- Recipient/subject/body fields are read-only (prevent edits during send).
- Status area shows "Sending...".

On completion, `sending` is set back to `false` if the send failed.

### Step 6: Handle `ActionCompleted` for `Send`

The send completion doesn't fit neatly into the existing `handle_action_completed` because it operates on a compose window, not the thread list. Options:

**Option A:** Handle `Send` completion before the generic handler, with compose-specific logic (close window on success, restore on failure).

**Option B:** Use a separate message variant (`Message::SendCompleted`).

**Decision: Option A.** Keep `ActionCompleted` as the single completion path. Add a `Send`-specific early return in `handle_action_completed` that handles compose window state. This is similar to how `MarkRead` has its own nav-refresh logic at the end of the handler.

```rust
// In handle_action_completed, before the removes_from_view check:
if matches!(action, CompletedAction::Send) {
    if all_failed || any_local_only {
        // Re-enable compose, show error
        // ... find compose window, set sending = false, show error ...
        return Task::none();
    }
    // Success — close compose window
    // ... close window, show "Message sent" toast ...
    return Task::none();
}
```

The compose window ID needs to be threaded through `ActionCompleted`. Add an optional `window_id: Option<iced::window::Id>` field to the `ActionCompleted` message, or encode it in `ActionParams`. The former is cleaner — it's metadata about the completion context, not a parameter of the action.

### Step 7: Delete legacy send code

- Remove the MIME build, DB insert, and window close from `handle_compose_send`. What remains: validation, `SendRequest` construction, dispatch to action service.
- The `tokens_to_csv` helper may still be needed for other compose paths — check before deleting.

### Step 8: Wire `delete_draft` for compose discard

When the user closes a compose window without sending (discard), if the draft has been persisted (auto-save, future), call `actions::delete_draft()`. For now (no auto-save), discard just closes the window — there's nothing in `local_drafts` to delete. Wire the action so it's ready when auto-save lands.

### Step 9: Verify

- `cargo check --workspace`
- `cargo clippy -p app -p ratatoskr-core`
- Verify `handle_compose_send` no longer calls `build_mime_message_base64url()` or inserts into `local_drafts` directly.
- Manual smoke test: send an email, verify it reaches the provider (check sent folder on server).
- Verify failed send re-enables the compose window (simulate by disconnecting network).
- Verify `mark_draft_sending`/`mark_draft_sent`/`mark_draft_failed` are called (log output).

## What This Produces

- `crates/core/src/actions/send.rs` — `send_email()` and `delete_draft()`
- Modified `crates/core/src/actions/mod.rs` — registers send module
- Modified `crates/app/src/main.rs` — `CompletedAction::Send`, `CompletedAction::DeleteDraft`
- Modified `crates/app/src/handlers/pop_out.rs` — `handle_compose_send` delegates to action service
- Modified `crates/app/src/handlers/commands.rs` — `handle_action_completed` handles `Send` completion
- Modified `crates/app/src/pop_out/compose.rs` — `sending: bool` field on `ComposeState`

## Exit Criteria

1. Clicking Send in compose dispatches through `actions::send_email()` which calls `ProviderOps::send_email()`.
2. Messages actually reach the provider. This is the first time `send_email()` has ever been called.
3. The compose window stays open during send. On success: window closes, toast shown. On failure: Send button re-enabled, error shown in compose.
4. `local_drafts` rows transition through `'sending'` → `'sent'`/`'failed'` lifecycle.
5. `delete_draft()` deletes locally and calls `ProviderOps::delete_draft()` if a remote ID exists.
6. The app crate no longer builds MIME or inserts into `local_drafts` directly.
7. Workspace compiles and passes clippy.

## What Phase 2.3 Does NOT Do

- **Draft auto-save.** Local persistence of in-progress compose state. Separate feature, no action service involvement (local-only).
- **Provider draft sync.** Pushing local drafts to the server via `create_draft()`/`update_draft()`. Sync concern, not action concern.
- **Outbox/failed send UI.** Listing failed sends and offering retry. Needs UI design. The `'failed'` draft rows are there for when this UI is built.
- **Retry on failure.** Automatic retry of failed sends. Phase 3 (failure policy) or Phase 5 (retry).
- **Shutdown recovery.** Detecting `'sending'` drafts on boot and retrying them. Phase 3.
- **Mentions.** `@-autocomplete` doesn't exist. Mentions are `&[]` until it does.
- **Scheduled send.** Separate feature with its own deferred delivery mechanism. Out of scope.

## Resequencing Note

The implementation-phases doc noted: *"If the local staging vs remote dispatch semantics prove entangled with failure policy, this sub-phase may be better sequenced after Phase 3."*

This plan avoids that entanglement by choosing immediate dispatch over deferred queue. The `local_drafts` table is used for crash recovery bookkeeping (draft state machine), not as a durable outbox. Deferred queue semantics (retry, shutdown persistence, outbox UI) are explicitly punted to Phases 3 and 5.

If this decision proves wrong — if immediate dispatch causes UX problems (slow providers blocking the compose window, no offline send) — the path forward is: Phase 3 adds a background dispatch mode where `send_email()` returns after local persist and a worker handles provider dispatch. The action function signature doesn't change; the internal implementation does. The app-side code (compose window management, completion handling) doesn't change either.
