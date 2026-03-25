# Action Service: Phase 4 Detailed Plan — Undo

## Goal

Wire undo end-to-end: action completion produces `UndoToken`s based on what actually happened, the app pushes them onto the `UndoStack`, and Ctrl+Z dispatches compensation actions through the service.

## Current State

The infrastructure is 80% scaffolded:

- **`UndoToken`** (`command-palette/src/undo.rs`) — 11 variants covering all email actions. Each captures previous state (`was_starred`, `original_folder_id`, etc.). Has `description()` for toast display. **Never constructed anywhere.**
- **`UndoStack`** (`command-palette/src/undo.rs`) — bounded FIFO (capacity 20). `push()`, `pop()`, `peek()`. Lives on `App.undo_stack`. **Never receives tokens.**
- **`Message::Undo`** — wired to Ctrl+Z. Handler pops the stack and logs + shows toast. **Does not execute compensation.**
- **Toggle rollback** (`handlers/commands.rs`) — `optimistic_toggle` captures `Vec<(account_id, thread_id, previous_value)>` for failure rollback. This is the same data `UndoToken::ToggleStar` etc. would carry — just in a different format.

### What's missing

1. **Token production.** No action function or handler produces `UndoToken`s.
2. **Undo execution.** `Message::Undo` doesn't dispatch compensation actions.
3. **Pending-ops cancellation.** Undoing a retryable `LocalOnly` action should cancel its pending op.
4. **Undo suppression.** Compensation actions must not produce new undo tokens (no "undo undo").

## Design Decisions

### Token production lives in the app handler, not the action service

The action service returns `ActionOutcome` (what happened). The app handler knows the action parameters (which threads, which labels, what the previous state was). The handler combines these to produce the `UndoToken`.

Why not the service? The service operates on single threads. The handler batches multiple threads into one `UndoToken` (e.g., archiving 5 threads → one `UndoToken::Archive { thread_ids: [5 ids] }`). The service doesn't know about the batch.

### Token is produced on `Success` and `LocalOnly`, not on `Failed`

Per Phase 3 breakdown:
- `Success` → token with full reversal data
- `LocalOnly { retryable: true }` → token for local reversal + cancel pending op
- `LocalOnly { retryable: false }` → token for local reversal only
- `Failed` → no token (nothing happened)

For removes-from-view actions (archive, trash, spam, move, permanent_delete): token is produced after all outcomes are collected. If all failed, no token. If any succeeded, token captures the thread IDs that succeeded. `PermanentDelete` never produces a token — irreversible.

For toggle actions (star, read, pin, mute): the existing rollback data `Vec<(account_id, thread_id, previous_value)>` is exactly what `UndoToken` needs. Convert on non-failure.

### Multi-account batches produce one token per account

`UndoToken` carries a single `account_id`. If a batch selection spans multiple accounts, the handler groups succeeded threads by account and pushes one token per account. This avoids silently dropping threads from non-first accounts.

### `ActionCompleted` carries both thread IDs and action params

Token production needs:
- Thread IDs (which threads succeeded) — for the `thread_ids` field
- Action parameters (label_id, source_folder_id, spam direction) — for variant-specific fields

Both are added to `ActionCompleted`:

```rust
Message::ActionCompleted {
    action: CompletedAction,
    outcomes: Vec<ActionOutcome>,
    rollback: Vec<(String, String, bool)>,
    threads: Vec<(String, String)>,  // (account_id, thread_id)
    params: ActionParams,
}
```

The dispatch functions already have both — they iterate over `threads` and carry `params`. Pass them through.

### Token payloads must be exact — no placeholders

Every `UndoToken` variant must carry the exact prior state needed for compensation. No `None` defaults, no empty strings, no hardcoded directions.

| Variant | Required data | Where it comes from |
|---|---|---|
| `Archive` | `thread_ids` | Succeeded threads from `ActionCompleted` |
| `Trash` | `thread_ids`, `original_folder_id` | `original_folder_id` = the sidebar's active label at dispatch time (same as `source_label_id` for `MoveToFolder`). Captured in the handler and passed through `ActionParams`. |
| `MoveToFolder` | `thread_ids`, `source_folder_id` | From `ActionParams::MoveToFolder { source_label_id }`. Must not be `None` or empty for undo — if unknown, don't produce a token. |
| `ToggleSpam` | `thread_ids`, `was_spam` | `was_spam` = the spam state BEFORE the action. The handler resolves this from the sidebar context (same as the existing `is_spam` resolution). Captured in `ActionParams`. |
| `ToggleStar/Read/Pin/Mute` | `thread_ids`, `was_*` | From rollback data — `previous_value` is the pre-action state. |
| `AddLabel` | `thread_ids`, `label_id` | From `ActionParams::Label { label_id }`. |
| `RemoveLabel` | `thread_ids`, `label_id` | From `ActionParams::Label { label_id }`. |

For `Trash`: `ActionParams` gains an `original_folder_id` field (the sidebar's active label, captured at dispatch time alongside `source_label_id` for `MoveToFolder`). If the source is unknown (None), no undo token is produced for trash — the user cannot undo without knowing where to restore to.

For `ToggleSpam`: `ActionParams::Spam` already carries `is_spam: bool`. The `was_spam` for undo is `!is_spam` (the state before the toggle).

### Undo execution dispatches through the action service

`Message::Undo` pops the token and dispatches the inverse action:

| UndoToken variant | Compensation action |
|---|---|
| `Archive` | For each: `add_label(ctx, account, thread, "INBOX")` |
| `Trash` | For each: `move_to_folder(ctx, account, thread, original_folder_id, None)` |
| `MoveToFolder` | For each: `move_to_folder(ctx, account, thread, source_folder_id, None)` |
| `ToggleRead` | For each: `mark_read(ctx, account, thread, was_read)` |
| `ToggleStar` | For each: `star(ctx, account, thread, was_starred)` |
| `TogglePin` | For each: `pin(ctx, account, thread, was_pinned)` |
| `ToggleMute` | For each: `mute(ctx, account, thread, was_muted)` |
| `ToggleSpam` | For each: `spam(ctx, account, thread, was_spam)` |
| `AddLabel` | For each: `remove_label(ctx, account, thread, label_id)` |
| `RemoveLabel` | For each: `add_label(ctx, account, thread, label_id)` |

### Compensation actions must not produce undo tokens

Undo dispatches compensation actions through the normal action service. Those actions will try to enqueue pending ops and the app handler would normally produce undo tokens from the outcomes. Both must be suppressed:

1. **Pending-ops suppression:** Use the existing `suppress_pending_enqueue` flag on `ActionContext`. The undo dispatcher sets it to `true`, same as the pending-ops worker does. Compensation actions don't enqueue.

2. **Undo token suppression:** Compensation actions bypass `ActionCompleted` entirely — `dispatch_undo` calls action functions directly and returns `UndoCompleted`, which has no token-production path. No `is_undo` flag needed on `ActionCompleted`.

### Undo outcomes are collected and reported

Compensation outcomes are not silently discarded. The undo dispatcher collects all `ActionOutcome`s and reports:
- All succeeded → "Undone: {description}"
- Any failed → "Undo partially failed — some changes may revert"
- All failed → "Undo failed: {error}"

### Pending-ops cancellation on undo

Before executing compensation, cancel matching pending ops for the threads being undone:

```rust
for tid in &thread_ids {
    let _ = db_pending_ops_cancel_for_resource(&ctx.db, &account_id, tid, operation_type).await;
}
```

`db_pending_ops_cancel_for_resource` deletes pending ops matching `(account_id, resource_id, operation_type)` with `status IN ('pending', 'executing')`. This prevents future retries. It does NOT stop an in-flight provider call that's already running — if the worker has loaded the op and started the provider request, deleting the DB row can't cancel that network call. The in-flight mutation may complete, and the subsequent undo compensation will reverse it. This is acceptable for best-effort undo.

### Undo is best-effort

If compensation fails (provider is down), the user sees a degraded toast. The compensation action's outcome follows normal `LocalOnly`/`Failed` semantics. No second-level undo.

## Implementation Steps

### Step 1: Expand `ActionCompleted` message

```rust
Message::ActionCompleted {
    action: CompletedAction,
    outcomes: Vec<ActionOutcome>,
    rollback: Vec<(String, String, bool)>,
    threads: Vec<(String, String)>,  // (account_id, thread_id)
    params: ActionParams,
}
```

Update `dispatch_action_service_with_params` and `dispatch_toggle_action` to pass `threads` and `params` through.

### Step 2: Produce tokens in `handle_action_completed`

After outcome analysis, if `!is_undo && !all_failed`:

**For removes-from-view and label actions:**
```rust
let succeeded: Vec<&(String, String)> = threads.iter()
    .zip(outcomes.iter())
    .filter(|(_, o)| !o.is_failed())
    .map(|(t, _)| t)
    .collect();

// Group by account_id, push one token per account
let mut by_account: HashMap<&str, Vec<String>> = HashMap::new();
for (aid, tid) in &succeeded {
    by_account.entry(aid.as_str()).or_default().push(tid.clone());
}
for (account_id, thread_ids) in by_account {
    if let Some(token) = build_undo_token(action, account_id, thread_ids, &params) {
        self.undo_stack.push(token);
    }
}
```

**For toggle actions:**
```rust
if !rollback.is_empty() {
    // Only include threads whose outcome was not Failed.
    // Failed toggles are already rolled back immediately by rollback_toggles —
    // including them in the undo token would re-flip threads that never changed.
    let succeeded_rollback: Vec<&(String, String, bool)> = rollback.iter()
        .zip(outcomes.iter())
        .filter(|(_, o)| !o.is_failed())
        .map(|(r, _)| r)
        .collect();

    // Group by account_id
    let mut by_account: HashMap<&str, Vec<(String, bool)>> = HashMap::new();
    for (aid, tid, prev) in &succeeded_rollback {
        by_account.entry(aid.as_str()).or_default().push((tid.clone(), *prev));
    }
    for (account_id, entries) in by_account {
        if let Some(token) = build_toggle_undo_token(action, account_id, entries) {
            self.undo_stack.push(token);
        }
    }
}
```

### Step 3: Implement `build_undo_token` and `build_toggle_undo_token`

```rust
fn build_undo_token(
    action: CompletedAction,
    account_id: &str,
    thread_ids: Vec<String>,
    params: &ActionParams,
) -> Option<UndoToken> {
    let account_id = account_id.to_string();
    match action {
        CompletedAction::Archive => Some(UndoToken::Archive { account_id, thread_ids }),
        CompletedAction::Trash => {
            // original_folder_id captured in ActionParams at dispatch time
            if let ActionParams::Trash { source_label_id } = params {
                Some(UndoToken::Trash {
                    account_id, thread_ids,
                    original_folder_id: source_label_id.clone(),
                })
            } else { None }
        }
        CompletedAction::Spam => {
            if let ActionParams::Spam { is_spam } = params {
                Some(UndoToken::ToggleSpam {
                    account_id, thread_ids,
                    was_spam: !is_spam, // was_spam = state BEFORE the action
                })
            } else { None }
        }
        CompletedAction::MoveToFolder => {
            if let ActionParams::MoveToFolder { source_label_id, .. } = params {
                let source = source_label_id.as_ref()?; // No source = no undo
                Some(UndoToken::MoveToFolder {
                    account_id, thread_ids,
                    source_folder_id: source.clone(),
                })
            } else { None }
        }
        CompletedAction::PermanentDelete => None, // Irreversible
        CompletedAction::AddLabel => {
            if let ActionParams::Label { label_id } = params {
                Some(UndoToken::AddLabel {
                    account_id, thread_ids, label_id: label_id.clone(),
                })
            } else { None }
        }
        CompletedAction::RemoveLabel => {
            if let ActionParams::Label { label_id } = params {
                Some(UndoToken::RemoveLabel {
                    account_id, thread_ids, label_id: label_id.clone(),
                })
            } else { None }
        }
        _ => None,
    }
}
```

### Step 4: Add `ActionParams::Trash` with source

```rust
enum ActionParams {
    None,
    Spam { is_spam: bool },
    MoveToFolder { folder_id: String, source_label_id: Option<String> },
    Label { label_id: String },
    Trash { source_label_id: Option<String> },  // new
}
```

The trash dispatch site captures `self.sidebar.selected_label.clone()` as the source, same as `MoveToFolder` already does.

### Step 5: Update trash dispatch to pass source

In `handle_email_action`, the trash dispatch currently calls `dispatch_action_service(CompletedAction::Trash, &selected_threads)` with `ActionParams::None`. Change to:

```rust
EmailAction::Trash => {
    let source_label_id = self.sidebar.selected_label.clone();
    return self.dispatch_action_service_with_params(
        CompletedAction::Trash,
        &selected_threads,
        ActionParams::Trash { source_label_id },
    );
}
```

Same pattern as `MoveToFolder` and `ToggleSpam` already use.

### Step 6: Implement undo dispatch

```rust
fn dispatch_undo(&mut self, token: UndoToken) -> Task<Message> {
    let Some(ref action_ctx) = self.action_ctx else {
        return Task::none();
    };
    let mut ctx = action_ctx.clone();
    ctx.suppress_pending_enqueue = true; // Don't re-enqueue during undo
    let desc = token.description();

    Task::perform(
        async move {
            let outcomes = execute_compensation(&ctx, &token).await;
            (desc, outcomes)
        },
        |(desc, outcomes)| Message::UndoCompleted { desc, outcomes },
    )
}
```

`execute_compensation` dispatches the inverse actions and collects outcomes:

```rust
async fn execute_compensation(
    ctx: &ActionContext,
    token: &UndoToken,
) -> Vec<ActionOutcome> {
    // Cancel pending ops first
    cancel_pending_ops_for_token(ctx, token).await;

    // Execute inverse actions
    match token {
        UndoToken::Archive { account_id, thread_ids } => {
            let mut outcomes = Vec::with_capacity(thread_ids.len());
            for tid in thread_ids {
                outcomes.push(add_label(ctx, account_id, tid, "INBOX").await);
            }
            outcomes
        }
        // ... all other variants
    }
}
```

### Step 7: Add `UndoCompleted` message and handler

```rust
Message::UndoCompleted { desc, outcomes } => {
    let all_failed = outcomes.iter().all(ActionOutcome::is_failed);
    let any_failed = outcomes.iter().any(ActionOutcome::is_failed);

    if all_failed {
        self.status_bar.show_confirmation(
            format!("\u{26A0} Undo failed: {desc}"),
        );
    } else if any_failed {
        self.status_bar.show_confirmation(
            format!("\u{26A0} Undo partially failed \u{2014} some changes may revert"),
        );
    } else {
        self.status_bar.show_confirmation(format!("Undone: {desc}"));
    }

    // Reload to reflect the reversed action
    Task::batch([
        self.fire_navigation_load(),
        self.fire_thread_load(),
    ])
}
```

### Step 8: Implement `cancel_pending_ops_for_token`

Add `db_pending_ops_cancel_for_resource` to `pending_ops.rs`:

```rust
pub async fn db_pending_ops_cancel_for_resource(
    db: &DbState,
    account_id: String,
    resource_id: String,
    operation_type: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM pending_operations
             WHERE account_id = ?1 AND resource_id = ?2 AND operation_type = ?3
               AND status IN ('pending', 'executing')",
            params![account_id, resource_id, operation_type],
        )
        .map_err(|e| format!("cancel pending op: {e}"))?;
        Ok(())
    })
    .await
}
```

`cancel_pending_ops_for_token` maps the token to the operation type and calls this for each thread.

### Step 9: Verify

- Ctrl+Z after archive → thread reappears in inbox
- Ctrl+Z after star → star state reverts
- Ctrl+Z after label add → label removed
- Ctrl+Z after trash from folder → thread returns to original folder
- Ctrl+Z after spam → spam state reverts (direction-aware)
- Ctrl+Z after archive with provider failure → archive undone locally + pending op cancelled
- Ctrl+Z with empty stack → no-op
- Permanent delete → no undo token produced
- Multi-account batch → one token per account, all accounts undoable
- Undo of undo → does not happen (no token produced for compensation)
- Failed undo → degraded toast

## Exit Criteria

1. Email actions (archive, trash, spam, move, star, read, pin, mute, label) produce `UndoToken`s on `Success` and `LocalOnly`. One token per account for multi-account batches.
2. `Failed` produces no token. `PermanentDelete` produces no token.
3. Token payloads carry exact prior state: `original_folder_id` for trash, `source_folder_id` for move, `was_spam` direction for spam, `label_id` for label ops, `previous_value` for toggles.
4. Ctrl+Z pops the token and dispatches the inverse action through the service with `suppress_pending_enqueue = true`.
5. Compensation actions do not produce undo tokens (dispatch bypasses `ActionCompleted`, returns `UndoCompleted` directly).
6. Undo of a retryable `LocalOnly` action cancels matching pending ops (including `'executing'` state for race safety).
7. Undo outcomes are collected and reported — all-succeeded, partially-failed, or all-failed toasts.
8. `UndoCompleted` refreshes thread list + navigation.

## What Phase 4 Does NOT Do

- **Redo.** No Ctrl+Shift+Z. The undo stack is one-directional.
- **Undo for non-email actions.** Send, folder, calendar, contact undo are not in scope.
- **Multi-level undo audit.** No verification that the 20-deep stack is sufficient.
- **Undo staleness detection.** Compensation actions are valid regardless of sync state.
- ~~**NoOp suppression.**~~ Implemented. `ActionOutcome::NoOp` skips undo token production. Archive and star use affected-row counts for detection.
