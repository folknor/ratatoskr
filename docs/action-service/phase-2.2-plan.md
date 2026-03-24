# Action Service: Phase 2.2 Detailed Plan

## Goal

Migrate label apply/remove into the action service, owning the `label_kind` dispatch. After this phase: the app crate has no `label_kind` branches, no `provider_label_write_back`, and no direct `thread_labels` SQL. Label operations flow through `core::actions` with the same outcome-based feedback as all other actions.

## Current State

Label apply/remove lives entirely in the app crate (`handlers/commands.rs:521-723`):

1. **`apply_label_to_selected_threads`** / **`remove_label_from_selected_threads`** — collects selected threads, does local DB mutation (`INSERT OR IGNORE` / `DELETE` on `thread_labels`), then calls `provider_label_write_back` per thread.

2. **`provider_label_write_back`** — the routing function. Looks up `(name, label_kind)` from the `labels` table, then:
   - `label_kind = "tag"` → `apply_category(ctx, thread_id, label_name)` / `remove_category(ctx, thread_id, label_name)`
   - `label_kind = "container"` → `add_tag(ctx, thread_id, label_id)` / `remove_tag(ctx, thread_id, label_id)`

3. **Re-initializes stores on every call.** `provider_label_write_back` calls `BodyStoreState::init()`, `SearchState::init()`, `InlineImageStoreState::init()` — all filesystem I/O — even though these already exist on `App` and are `Clone`. The action service eliminates this by using `ActionContext`'s pre-initialized stores.

4. **No structured outcome.** Local DB errors are logged, provider failures are warned, the UI shows a premature "Label applied" / "Label removed" confirmation *before* either step runs. The user gets a success toast even when the operation fails.

5. **Encryption key gating.** Provider write-back is skipped entirely when `encryption_key` is `None`. This means accounts without encryption silently get local-only label operations with no indication to the user. The action service surfaces this as `LocalOnly`.

## Provider Audit

The four `ProviderOps` methods involved:

| Method | Gmail | Graph | JMAP | IMAP |
|--------|-------|-------|------|------|
| `apply_category(ctx, msg_id, name)` | Finds label ID by name, `modify_message` for all messages | Reads current categories, PATCHes message | Sets keyword on message | Sets keyword flag via STORE +FLAGS |
| `remove_category(ctx, msg_id, name)` | Finds label ID by name, `modify_message` for all messages | Reads current categories, PATCHes message | Removes keyword from message | Removes keyword flag |
| `add_tag(ctx, thread_id, tag_id)` | `modify_thread` with tag ID | Parses `cat:` prefix, reads/updates all messages | Resolves mailbox ID, updates all emails | **No-op** |
| `remove_tag(ctx, thread_id, tag_id)` | `modify_thread` with tag ID | Parses `cat:` prefix, reads/updates all messages | Resolves mailbox ID, updates all emails | **No-op** |

Key observations:

- **Tags use name-based dispatch** (`apply_category`/`remove_category`). The service must look up the label name from the DB.
- **Containers use ID-based dispatch** (`add_tag`/`remove_tag`). The label ID is passed directly.
- **IMAP `add_tag`/`remove_tag` are no-ops.** IMAP folders can't be manipulated via tag semantics — they use `move_to_folder` (already handled by Phase 2.1). This is correct behavior, not a gap. But it means a container-type label apply on IMAP will succeed locally and "succeed" remotely (the no-op returns `Ok(())`). The service should return `Success`, not `LocalOnly` — the provider accepted the call.
- **`apply_category`/`remove_category` have default no-op implementations** on the trait. Providers that don't support categories silently succeed. Same reasoning applies — `Success`, not `LocalOnly`.
- **thread_id/message_id mismatch.** `apply_category`/`remove_category` declare their second parameter as `message_id` in the trait signature, but the existing app code (and this plan) passes `thread_id`. This works because all four providers resolve thread-level operations internally: Gmail finds messages by thread, Graph/JMAP iterate over messages in the thread, IMAP parses a provider-specific format. The service preserves this existing behavior. Fixing the trait contract is a Phase 6 concern.

## Design Decisions

### Label actions are a third interaction pattern

Labels are not removes-from-view (no auto-advance) and not toggles (no optimistic UI flip). They are fire-and-report: do local DB + provider dispatch, then surface the outcome.

**The existing `handle_action_completed` does NOT handle this pattern.** The current handler has two paths: removes-from-view (toast + auto-advance) and toggle (rollback + optional nav refresh). A label action with `removes_from_view = false` and empty rollback falls through both paths silently — no toast is shown. This must be fixed.

The fix: add a feedback section for non-toggle, non-removes-from-view actions. After the removes-from-view early return and before the toggle rollback logic, check for empty rollback (meaning: not a toggle). If the action is not a toggle, show outcome-based feedback:

```rust
// ── Non-toggle, non-removes-from-view actions (labels) ──
// Toggle actions have rollback data; their optimistic UI IS the feedback.
// Label-type actions have no rollback and need an explicit toast.
if rollback.is_empty() {
    if all_failed {
        let errors: Vec<&str> = outcomes
            .iter()
            .filter_map(|o| match o {
                ActionOutcome::Failed { error } => Some(error.as_str()),
                _ => None,
            })
            .collect();
        self.status_bar.show_confirmation(
            format!("\u{26A0} {} failed: {}", action.success_label(), errors.join("; ")),
        );
    } else if any_local_only {
        self.status_bar.show_confirmation(
            format!("\u{26A0} {} locally \u{2014} sync may revert this", action.success_label()),
        );
    } else {
        self.status_bar.show_confirmation(action.success_label().to_string());
    }
    return Task::none();
}
```

This goes in `handle_action_completed` immediately after the `removes_from_view()` early return (before the toggle rollback section). The condition `rollback.is_empty()` distinguishes labels from toggles — toggles always have rollback data, labels never do. This is a general mechanism, not label-specific — any future fire-and-report action gets feedback automatically.

### The service owns the label_kind lookup

The caller passes `label_id`. The service looks up `(name, label_kind)` from the `labels` table inside `spawn_blocking`, then uses that to route the provider call. The app crate never sees `label_kind`.

The label metadata lookup happens once per action function call. Since the caller loops over threads and calls `add_label`/`remove_label` per thread, this means one lookup per thread — not one per batch. The label metadata is identical across all threads in a batch (it's a property of the label, not the thread), so this is redundant work. Acceptable for now; Phase 5 batching can hoist the lookup.

If the label ID doesn't exist in the `labels` table for the given account, `query_row` fails and the action returns `ActionOutcome::Failed`. The error message will be rusqlite's "query returned no rows" — not a friendly label-not-found message. This is acceptable for Phase 2.2; Phase 3 introduces structured error types.

### Account-scoped label resolution

A label ID is scoped to an account (`(account_id, label_id)` is the canonical identity per the glossary). The current code queries `SELECT name, label_kind FROM labels WHERE id = ?1 LIMIT 1` — this is wrong for cross-account scenarios where the same label ID could exist on multiple accounts with different kinds. The correct query is `WHERE id = ?1 AND account_id = ?2`.

However: in practice, label IDs are globally unique because they carry provider prefixes (`cat:`, `kw:`, `graph-`, `jmap-`, `folder-`). The current query works by accident. The service should still scope by account for correctness, since the label is always applied in the context of a specific account.

### Mixed-account selections

`ActionParams::Label { label_id }` carries a single label ID. If the selected threads span multiple accounts, the same label ID is used for all threads. Since label identity is `(account_id, label_id)`, the label may not exist on every account — a Gmail label ID won't be found in an Exchange account's `labels` rows.

**The service treats this as partial failure per thread.** The account-scoped query (`WHERE id = ?1 AND account_id = ?2`) will return "no rows" for accounts that don't have this label. Those threads get `ActionOutcome::Failed`. Threads on the correct account succeed. The completion handler shows mixed-outcome feedback: "⚠ Label applied 3 of 5 threads — 2 failed."

This is the correct behavior — the service does not need to pre-partition by account or reject mixed selections. The per-thread outcome reporting already handles it. The label picker UI (not yet built) should ideally only offer labels applicable to the selected threads' accounts, but that's a UI concern outside this phase.

### No consolidation of apply_category/add_tag yet

The Phase 2.2 spec asks: "Decide whether Phase 2.2 consolidates `apply_category`/`remove_category` into `add_tag`/`remove_tag` or preserves the current split."

**Decision: preserve the current split.** Consolidation means changing the `ProviderOps` trait signature and updating all four provider implementations — that's a cross-crate refactor that should be its own phase (Phase 6 per the labels unification spec). Phase 2.2's job is to move the routing logic out of the app crate, not to redesign the provider interface.

The service calls the same four methods the app crate does today. The only change is *where* the routing lives.

### Completion feedback

Labels use the same `ActionCompleted` message and `handle_action_completed` handler as all other actions:

- `Success` → "Label applied" / "Label removed"
- `LocalOnly` → "⚠ Label applied locally — sync may revert this"
- `Failed` → "⚠ Label apply failed: {error}"

The premature confirmation toast (shown before the operation runs) is removed. Feedback is deferred to `handle_action_completed`, consistent with all other actions.

## Action Function Signatures

```rust
// crates/core/src/actions/label.rs

pub async fn add_label(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    label_id: &str,
) -> ActionOutcome

pub async fn remove_label(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    label_id: &str,
) -> ActionOutcome
```

No `label_name` or `label_kind` parameter — the service resolves these internally.

## Implementation Steps

### Step 1: Create `crates/core/src/actions/label.rs`

Two public functions, each following this structure:

```rust
pub async fn add_label(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    label_id: &str,
) -> ActionOutcome {
    // 1. Look up label metadata + do local DB mutation in one spawn_blocking call
    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let lid = label_id.to_string();
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;

        // Look up label metadata (name + kind) for provider routing
        let (label_name, label_kind) = conn.query_row(
            "SELECT name, label_kind FROM labels WHERE id = ?1 AND account_id = ?2 LIMIT 1",
            rusqlite::params![lid, aid],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        ).map_err(|e| format!("label lookup: {e}"))?;

        // Local DB mutation
        crate::email_actions::insert_label(&conn, &aid, &tid, &lid)?;

        Ok((label_name, label_kind))
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))
    .and_then(|r| r);

    let (label_name, label_kind) = match local_result {
        Ok(info) => info,
        Err(e) => return ActionOutcome::Failed { error: e },
    };

    // 2. Provider dispatch
    let provider = match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(p) => p,
        Err(e) => {
            log::warn!("AddLabel local-only (provider create failed): {e}");
            return ActionOutcome::LocalOnly { remote_error: e };
        }
    };

    let provider_ctx = ProviderCtx {
        account_id,
        db: &ctx.db,
        body_store: &ctx.body_store,
        inline_images: &ctx.inline_images,
        search: &ctx.search,
        progress: &NoopProgressReporter,
    };

    // Route by label_kind: tags use name-based category ops,
    // containers use ID-based tag ops
    let result = if label_kind == "tag" {
        provider.apply_category(&provider_ctx, thread_id, &label_name).await
    } else {
        provider.add_tag(&provider_ctx, thread_id, label_id).await
    };

    match result {
        Ok(()) => ActionOutcome::Success,
        Err(e) => {
            let msg = e.to_string();
            log::warn!("AddLabel remote failed for {account_id}/{thread_id}: {msg}");
            ActionOutcome::LocalOnly { remote_error: msg }
        }
    }
}
```

`remove_label` is the same structure with `DELETE` instead of `INSERT` and `remove_category`/`remove_tag` instead of `apply_category`/`add_tag`.

### Step 2: Register in `crates/core/src/actions/mod.rs`

```rust
mod label;
pub use label::{add_label, remove_label};
```

### Step 3: Add `AddLabel` and `RemoveLabel` to `CompletedAction`

In `crates/app/src/main.rs`:

```rust
pub enum CompletedAction {
    Archive,
    Trash,
    Spam,
    MoveToFolder,
    PermanentDelete,
    Star,
    MarkRead,
    Pin,
    Mute,
    AddLabel,     // new
    RemoveLabel,  // new
}
```

`removes_from_view()` returns `false` for both. `success_label()` returns `"Label applied"` and `"Label removed"`.

### Step 4: Add `Label` variant to `ActionParams`

In `crates/app/src/handlers/commands.rs`:

```rust
enum ActionParams {
    None,
    Spam { is_spam: bool },
    MoveToFolder { folder_id: String, source_label_id: Option<String> },
    Label { label_id: String },  // new
}
```

### Step 5: Wire label actions into `dispatch_action_service_with_params`

Add match arms in the async dispatch block:

```rust
(CompletedAction::AddLabel, ActionParams::Label { label_id }) => {
    ratatoskr_core::actions::add_label(&ctx, account_id, thread_id, label_id).await
}
(CompletedAction::RemoveLabel, ActionParams::Label { label_id }) => {
    ratatoskr_core::actions::remove_label(&ctx, account_id, thread_id, label_id).await
}
```

### Step 6: Migrate `handle_email_action` arms

Replace:

```rust
// ── Legacy path: labels (Phase 2.2) ──
EmailAction::AddLabel { label_id } => {
    self.status_bar.show_confirmation("Label applied".to_string());
    return self.apply_label_to_selected_threads(&label_id);
}
EmailAction::RemoveLabel { label_id } => {
    self.status_bar.show_confirmation("Label removed".to_string());
    return self.remove_label_from_selected_threads(&label_id);
}
```

With:

```rust
EmailAction::AddLabel { label_id } => {
    return self.dispatch_action_service_with_params(
        CompletedAction::AddLabel,
        &selected_threads,
        ActionParams::Label { label_id },
    );
}
EmailAction::RemoveLabel { label_id } => {
    return self.dispatch_action_service_with_params(
        CompletedAction::RemoveLabel,
        &selected_threads,
        ActionParams::Label { label_id },
    );
}
```

No premature toast. Feedback comes from `handle_action_completed`.

### Step 7: Add feedback path to `handle_action_completed`

Insert the non-toggle, non-removes-from-view feedback section (described in the design decisions) into `handle_action_completed`, immediately after the `removes_from_view()` early return. This is the `rollback.is_empty()` guard that shows outcome-based toasts for label actions. Without this, label operations complete silently.

### Step 8: Delete legacy code

- Delete `apply_label_to_selected_threads` (lines 521-592)
- Delete `remove_label_from_selected_threads` (lines 597-665)
- Delete `provider_label_write_back` (lines 676-723)

### Step 9: Verify

- `cargo check --workspace`
- `cargo clippy -p app -p ratatoskr-core`
- Grep the app crate for `label_kind`, `provider_label_write_back`, `apply_label_to_selected`, `remove_label_from_selected` — all should be gone.
- Grep the app crate for `apply_category`, `remove_category`, `add_tag`, `remove_tag` — none should appear (these are now called only from core).

## What This Produces

- `crates/core/src/actions/label.rs` — `add_label()` and `remove_label()`
- Modified `crates/core/src/actions/mod.rs` — registers label module
- Modified `crates/app/src/main.rs` — `CompletedAction::AddLabel`, `CompletedAction::RemoveLabel`
- Modified `crates/app/src/handlers/commands.rs` — label dispatch through service, feedback path for non-toggle/non-removes-from-view actions, legacy code deleted

## Exit Criteria

1. `actions::add_label()` and `actions::remove_label()` perform local DB mutation + provider dispatch with `label_kind` routing.
2. The app crate does not contain `label_kind` string comparisons.
3. `provider_label_write_back`, `apply_label_to_selected_threads`, and `remove_label_from_selected_threads` are deleted.
4. Label operations surface outcomes via `handle_action_completed` — no premature confirmation toast.
5. `handle_action_completed` shows outcome-based feedback for label actions (not silent). The feedback path is generic (keyed on empty rollback), not label-specific.
6. The label metadata query is scoped by `account_id` for correctness.
7. Mixed-account selections produce partial failure — threads on accounts without the label get `Failed`, others succeed.
8. `apply_category`/`remove_category` and `add_tag`/`remove_tag` on `ProviderOps` are unchanged — consolidation deferred.
9. Workspace compiles and passes clippy.

## What Phase 2.2 Does NOT Do

- **Consolidate `apply_category`/`remove_category` into `add_tag`/`remove_tag`.** That's a provider trait refactor (labels unification Phase 6).
- **Batch optimization.** Label metadata lookup happens per thread (redundant — metadata is per-label, not per-thread). Hoisting the lookup and batching dispatch across threads deferred to Phase 5.
- **IMAP no-op representation.** IMAP's `add_tag`/`remove_tag` return `Ok(())` (no-op). The service reports `Success`. Making this explicit (e.g., `ActionOutcome::NoOp` or a `local_only_by_design` flag) is deferred — same limitation as pin/mute in Phase 2.1.
