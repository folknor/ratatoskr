# Action Service: Phase 4 Detailed Plan — Undo

## Goal

Wire undo end-to-end: action functions produce `UndoToken`s based on what actually happened, the app pushes them onto the `UndoStack`, and Ctrl+Z dispatches compensation actions through the service.

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

For removes-from-view actions (archive, trash, spam, move, permanent_delete): token is produced after all outcomes are collected. If all failed, no token. If any succeeded, token captures the thread IDs that succeeded.

For toggle actions (star, read, pin, mute): the existing rollback data `Vec<(account_id, thread_id, previous_value)>` is exactly what `UndoToken` needs. Convert on non-failure.

### Undo execution dispatches through the action service

`Message::Undo` pops the token and dispatches the inverse action:

| UndoToken variant | Compensation action |
|---|---|
| `Archive { thread_ids }` | For each: `add_label(ctx, account, thread, "INBOX")` — restore inbox |
| `Trash { thread_ids, original_folder_id }` | For each: if original folder known, `move_to_folder`; else `add_label(ctx, account, thread, "INBOX")` |
| `MoveToFolder { thread_ids, source_folder_id }` | For each: `move_to_folder(ctx, account, thread, source, None)` |
| `ToggleRead { thread_ids, was_read }` | For each: `mark_read(ctx, account, thread, was_read)` |
| `ToggleStar { thread_ids, was_starred }` | For each: `star(ctx, account, thread, was_starred)` |
| `TogglePin { thread_ids, was_pinned }` | For each: `pin(ctx, account, thread, was_pinned)` |
| `ToggleMute { thread_ids, was_muted }` | For each: `mute(ctx, account, thread, was_muted)` |
| `ToggleSpam { thread_ids, was_spam }` | If was_spam: `spam(ctx, account, thread, true)`; else `spam(ctx, account, thread, false)` |
| `AddLabel { thread_ids, label_id }` | For each: `remove_label(ctx, account, thread, label)` |
| `RemoveLabel { thread_ids, label_id }` | For each: `add_label(ctx, account, thread, label)` |
| `Snooze { thread_ids }` | Not yet implemented (snooze not in action service) |

### Pending-ops cancellation on undo

When the user undoes an action that had `retryable: true` LocalOnly outcomes, the pending ops for those threads should be cancelled rather than retried. The undo handler deletes matching pending ops:

```sql
DELETE FROM pending_operations
WHERE account_id = ?1 AND resource_id = ?2 AND operation_type = ?3 AND status = 'pending'
```

This prevents: user archives → provider fails → pending op queued → user undoes → archive is back in inbox → pending op fires and re-archives.

### Undo is best-effort

Undo compensation goes through the action service like any other action. If the compensation fails (e.g., provider is down), the user gets the same `LocalOnly`/`Failed` feedback. Undo itself doesn't produce undo tokens (no "undo undo").

### Staleness: undo after sync has reconciled

If the user archives a thread, then sync runs and reconciles (the thread is now archived on the server), the undo token is technically stale — the local state already matches the server. But executing undo (add back to inbox) is still correct: it's the reverse operation the user expects. Staleness detection is not needed for Phase 4 — the compensation action is valid regardless of whether sync has intervened.

## Implementation Steps

### Step 1: Produce tokens in `handle_action_completed`

In `handle_action_completed`, after determining the outcome mix, produce an `UndoToken` if any threads succeeded:

For **removes-from-view** actions:
```rust
// After the existing outcome analysis and before returning:
if !all_failed {
    let succeeded_ids: Vec<String> = threads_and_outcomes
        .filter(|(_, o)| !o.is_failed())
        .map(|(tid, _)| tid.clone())
        .collect();
    if !succeeded_ids.is_empty() {
        if let Some(token) = build_undo_token(action, account_id, succeeded_ids, params) {
            self.undo_stack.push(token);
        }
    }
}
```

For **toggle** actions (existing rollback path):
```rust
// On non-failure, convert rollback data to UndoToken:
if !all_failed && !rollback.is_empty() {
    // rollback has (account_id, thread_id, previous_value)
    // previous_value is what we need to restore on undo
    if let Some(token) = build_toggle_undo_token(action, rollback) {
        self.undo_stack.push(token);
    }
}
```

For **label** actions (fire-and-report):
```rust
// Similar to removes-from-view but for AddLabel/RemoveLabel
```

The thread IDs need to be available in `handle_action_completed`. Currently the handler doesn't have them — `ActionCompleted` carries `outcomes` and `rollback` but not the thread IDs for removes-from-view actions. **This needs to change:** add `thread_ids: Vec<(String, String)>` to `ActionCompleted` (account_id + thread_id pairs).

### Step 2: Add thread IDs to `ActionCompleted`

```rust
Message::ActionCompleted {
    action: CompletedAction,
    outcomes: Vec<ActionOutcome>,
    rollback: Vec<(String, String, bool)>,
    /// Thread identities for undo token production.
    threads: Vec<(String, String)>,  // (account_id, thread_id)
}
```

The dispatch functions already have this data — they iterate over `threads`. Pass it through.

### Step 3: Implement `build_undo_token` and `build_toggle_undo_token`

Helper functions that convert action + thread data + params into `UndoToken`:

```rust
fn build_undo_token(
    action: CompletedAction,
    threads: &[(String, String)],
    params: &ActionParams,
) -> Option<UndoToken> {
    // Group by account_id (tokens are per-account)
    // For single-account batches (common case), produce one token
    // For mixed accounts, produce token for the first account (simplification)
    let account_id = threads.first()?.0.clone();
    let thread_ids: Vec<String> = threads.iter().map(|(_, tid)| tid.clone()).collect();

    match action {
        CompletedAction::Archive => Some(UndoToken::Archive { account_id, thread_ids }),
        CompletedAction::Trash => Some(UndoToken::Trash {
            account_id, thread_ids, original_folder_id: None, // TODO: capture source
        }),
        CompletedAction::Spam => Some(UndoToken::ToggleSpam {
            account_id, thread_ids, was_spam: false,
        }),
        CompletedAction::MoveToFolder => {
            if let ActionParams::MoveToFolder { source_label_id, .. } = params {
                Some(UndoToken::MoveToFolder {
                    account_id, thread_ids,
                    source_folder_id: source_label_id.clone().unwrap_or_default(),
                })
            } else { None }
        }
        CompletedAction::PermanentDelete => None, // Permanent delete is irreversible
        CompletedAction::AddLabel => {
            if let ActionParams::Label { label_id } = params {
                Some(UndoToken::AddLabel { account_id, thread_ids, label_id: label_id.clone() })
            } else { None }
        }
        CompletedAction::RemoveLabel => {
            if let ActionParams::Label { label_id } = params {
                Some(UndoToken::RemoveLabel { account_id, thread_ids, label_id: label_id.clone() })
            } else { None }
        }
        _ => None,
    }
}
```

### Step 4: Implement undo execution in `Message::Undo`

Replace the current log-only handler with actual compensation dispatch:

```rust
Message::Undo => {
    let Some(token) = self.undo_stack.pop() else {
        return Task::none();
    };
    self.dispatch_undo(token)
}
```

`dispatch_undo` dispatches the inverse action through the service:

```rust
fn dispatch_undo(&mut self, token: UndoToken) -> Task<Message> {
    let Some(ref action_ctx) = self.action_ctx else {
        return Task::none();
    };
    let ctx = action_ctx.clone();
    let desc = token.description();

    Task::perform(
        async move {
            match token {
                UndoToken::Archive { account_id, thread_ids } => {
                    for tid in &thread_ids {
                        let _ = ratatoskr_core::actions::add_label(
                            &ctx, &account_id, tid, "INBOX",
                        ).await;
                    }
                }
                UndoToken::ToggleStar { account_id, thread_ids, was_starred } => {
                    for tid in &thread_ids {
                        let _ = ratatoskr_core::actions::star(
                            &ctx, &account_id, tid, was_starred,
                        ).await;
                    }
                }
                // ... other variants
                _ => {}
            }
            desc
        },
        |desc| {
            Message::UndoCompleted(desc)
        },
    )
}
```

`Message::UndoCompleted(String)` shows the toast and triggers nav/thread reload.

### Step 5: Cancel pending ops on undo

In `dispatch_undo`, before executing the compensation, cancel matching pending ops:

```rust
// Cancel any pending retry for this action on these threads
for tid in &thread_ids {
    let _ = cancel_pending_op(&ctx.db, &account_id, tid, operation_type).await;
}
```

Where `cancel_pending_op` deletes matching pending operations.

### Step 6: Add `UndoCompleted` message and handler

```rust
Message::UndoCompleted(desc) => {
    self.status_bar.show_confirmation(format!("Undone: {desc}"));
    // Reload thread list + nav to reflect the reversed action
    Task::batch([
        self.fire_navigation_load(),
        self.fire_thread_load(),
    ])
}
```

### Step 7: Verify

- Ctrl+Z after archive → thread reappears in inbox
- Ctrl+Z after star → star state reverts
- Ctrl+Z after label add → label removed
- Ctrl+Z after archive with provider failure → archive undone locally + pending op cancelled
- Ctrl+Z with empty stack → no-op
- Permanent delete → no undo token produced

## Exit Criteria

1. Email actions (archive, trash, spam, move, star, read, pin, mute, label) produce `UndoToken`s on `Success` and `LocalOnly`.
2. `Failed` produces no token. `PermanentDelete` produces no token (irreversible).
3. Ctrl+Z pops the token and dispatches the inverse action through the service.
4. Undo of a retryable `LocalOnly` action cancels the matching pending op.
5. `UndoCompleted` shows toast and refreshes thread list + navigation.
6. Toggle undo restores the previous value (not re-toggles).

## What Phase 4 Does NOT Do

- **Redo.** No Ctrl+Shift+Z. The undo stack is one-directional.
- **Undo for non-email actions.** Send, folder, calendar, contact undo are not in scope.
- **Multi-level undo audit.** No verification that the 20-deep stack is sufficient.
- **Undo staleness detection.** Compensation actions are valid regardless of sync state.
- **NoOp suppression.** If an action was a no-op (e.g., archive of an already-archived thread), it still gets a token. Detecting no-ops requires pre-checking which is deferred.
