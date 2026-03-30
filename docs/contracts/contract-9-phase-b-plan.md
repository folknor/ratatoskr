# Contract #9 Phase B: Per-Target Batch Execution — Implementation Plan

## Goal

Merge the two dispatch paths (toggle vs non-toggle) into one. The app builds a flat `Vec<(String, String, MailOperation)>` and hands it to core. Core's `batch_execute` accepts per-target operations, groups internally, and returns outcomes in original order. The toggle split/merge dance in the app is eliminated.

## Current State (after Phase A)

Two dispatch paths exist:

**Non-toggle path:**
```
handle_email_action → resolve_intent → resolved_to_legacy → to_batch_action → batch_execute(ctx, BatchAction, targets)
```

**Toggle path:**
```
handle_email_action → optimistic_toggle → dispatch_toggle_action → dispatch_toggle_batch → [partition by value] → to_toggle_batch → batch_execute × 2 → merge outcomes
```

Phase B collapses both into:
```
handle_email_action → resolve_intent / per-target toggle resolution → build_execution_plan → batch_execute(ctx, operations)
```

## Edit Sequence

### Step 1: Define OptimisticMutation in app crate

File: `crates/app/src/action_resolve.rs`

```rust
/// What was flipped in the UI before execution. Typed per field.
/// Phase C builds MailUndoPayload from MailOperation + CompensationContext +
/// OptimisticMutation without re-reading UI state.
#[derive(Debug, Clone)]
pub enum OptimisticMutation {
    SetStarred { account_id: String, thread_id: String, previous: bool },
    SetRead { account_id: String, thread_id: String, previous: bool },
    SetPinned { account_id: String, thread_id: String, previous: bool },
    SetMuted { account_id: String, thread_id: String, previous: bool },
}
```

### Step 2: Define ActionExecutionPlan in app crate

File: `crates/app/src/action_resolve.rs`

```rust
pub struct ActionExecutionPlan {
    /// Per-target operations — always flat, even for uniform actions.
    pub operations: Vec<(String, String, MailOperation)>,
    /// Compensation context from resolution (source folder, etc.).
    pub compensation: CompensationContext,
    /// Optimistic UI mutations applied, if any. Empty for non-toggles.
    pub optimistic: Vec<OptimisticMutation>,
}
```

No `CompletionBehavior` yet — that's Phase C. Phase B focuses on the execution path.

### Step 3: Implement build_execution_plan

File: `crates/app/src/action_resolve.rs`

Two cases:

**Non-toggle** (resolve_intent returned Some): Every thread gets the same operation.
```rust
operations = threads.iter().map(|(a, t)| (a.clone(), t.clone(), resolved.operation.clone())).collect();
optimistic = vec![];
```

**Toggle** (resolve_intent returned None for ToggleStar/etc.): Per-thread resolution with strict ordering.

```rust
// For each thread:
// 1. Read prior state
// 2. Compute operation
// 3. Record OptimisticMutation
// 4. Flip UI
```

The function needs `&mut ThreadList` for step 4 (optimistic mutation) and read access to thread state for step 1.

Signature:
```rust
pub fn build_execution_plan(
    intent: &MailActionIntent,
    resolved: Option<ResolvedIntent>,
    threads: &[(String, String)],
    thread_list: &mut crate::ui::thread_list::ThreadList,
) -> Option<ActionExecutionPlan>
```

Returns `None` for `Unsubscribe` (fire-and-forget).

For toggles, `intent` tells us which field to toggle. `resolved` is `None`. We build operations + mutations by iterating threads.

For non-toggles, `resolved` is `Some`. We build uniform operations. `thread_list` is unused.

### Step 4: Change batch_execute signature in core

File: `crates/core/src/actions/batch.rs`

FROM:
```rust
pub async fn batch_execute(
    ctx: &ActionContext,
    action: BatchAction,
    targets: Vec<(String, String)>,
) -> Vec<ActionOutcome>
```

TO:
```rust
pub async fn batch_execute(
    ctx: &ActionContext,
    operations: Vec<(String, String, MailOperation)>,
) -> Vec<ActionOutcome>
```

**Internal changes:**
- Group by `account_id` (same as before)
- Pass per-thread `MailOperation` into `execute_account_group`
- `execute_account_group` receives `Vec<(usize, String, MailOperation)>` (index + thread_id + operation)
- Local-only detection: check if operation is Pin/Mute/Snooze (no provider needed)
- `dispatch_with_provider` takes `&MailOperation` instead of `&BatchAction`
- `action_local` takes `&MailOperation` instead of `&BatchAction`
- `enqueue_params` takes `&MailOperation` instead of `&BatchAction`
- `action_name` takes `&MailOperation` instead of `&BatchAction`

**Regrouping:** For Phase B, the executor dispatches per-thread sequentially within an account (same as current behavior). Regrouping identical operations for provider-level batching is a future optimization — the `PartialEq` on `MailOperation` enables it but we don't implement it yet.

### Step 5: Rewrite dispatch_with_provider for MailOperation

File: `crates/core/src/actions/batch.rs`

Exhaustive match on `MailOperation`:
```rust
MailOperation::Archive => archive::archive_with_provider(ctx, provider, account_id, thread_id).await,
MailOperation::Trash => trash::trash_with_provider(ctx, provider, account_id, thread_id).await,
MailOperation::SetSpam { to } => spam::spam_with_provider(ctx, provider, account_id, thread_id, *to).await,
MailOperation::MoveToFolder { dest } => move_to_folder::move_to_folder_with_provider(ctx, provider, account_id, thread_id, dest, None).await,
MailOperation::SetStarred { to } => star::star_with_provider(ctx, provider, account_id, thread_id, *to).await,
MailOperation::SetRead { to } => mark_read::mark_read_with_provider(ctx, provider, account_id, thread_id, *to).await,
MailOperation::PermanentDelete => permanent_delete::permanent_delete_with_provider(ctx, provider, account_id, thread_id).await,
MailOperation::AddLabel { label_id } => label::add_label_with_provider(ctx, provider, account_id, thread_id, label_id).await,
MailOperation::RemoveLabel { label_id } => label::remove_label_with_provider(ctx, provider, account_id, thread_id, label_id).await,
// Pin/Mute/Snooze are local-only — unreachable in the provider path
MailOperation::SetPinned { .. } | MailOperation::SetMuted { .. } | MailOperation::Snooze { .. } => unreachable!(),
```

**Note on MoveToFolder:** The `source_label_id` parameter to `move_to_folder_with_provider` is passed as `None` here because `MailOperation::MoveToFolder` doesn't carry the source (it's undo metadata in `CompensationContext`). The `source_label_id` in `move_to_folder_with_provider` is only used for the local DB mutation (`remove_label` from source), which `move_local` handles. Check that passing `None` for source doesn't break the local mutation — if it does, we may need to add `source` to `MailOperation::MoveToFolder` after all, accepting the design trade-off.

### Step 6: Rewrite action_local, enqueue_params, action_name for MailOperation

File: `crates/core/src/actions/batch.rs`

Same pattern — exhaustive match on `MailOperation` instead of `BatchAction`. Mechanical rewrite.

### Step 7: Rewrite handle_email_action to use build_execution_plan

File: `crates/app/src/handlers/commands.rs`

Replace the entire body with:
```rust
1. Convert EmailAction → MailActionIntent
2. Build UiContext
3. resolve_intent(intent, &ui_ctx) — returns Some for non-toggles, None for toggles
4. build_execution_plan(intent, resolved, threads, &mut self.thread_list)
5. If plan includes star optimistic mutations: sync_reading_pane_after_toggle
6. Dispatch: Task::perform(batch_execute(ctx, plan.operations), ...)
```

**Remove:**
- `dispatch_toggle_action`
- `dispatch_toggle_batch`
- `to_batch_action`
- `to_toggle_batch`
- `resolved_to_legacy`
- `email_action_to_intent` (MailActionIntent constructed directly from EmailAction or EmailAction replaced)

**Keep for now:**
- `dispatch_action_service` / `dispatch_action_service_with_params` — other callers (pop_out.rs) still use these. Will be replaced when those callers migrate. Alternatively, make them use the new path too in this step.

### Step 8: Update Message::ActionCompleted

File: `crates/app/src/main.rs`

The completion message needs to carry enough for the completion handler. For Phase B, carry the plan:

```rust
ActionCompleted {
    plan: ActionExecutionPlan,
    outcomes: Vec<ActionOutcome>,
}
```

But `ActionExecutionPlan` doesn't have `CompletionBehavior` yet (Phase C). So for Phase B, we still need `CompletedAction` for the completion handler to know what toast to show. **Compromise:** carry both the plan (for optimistic rollback) and the `CompletedAction` (for the completion handler) during Phase B. Phase C removes `CompletedAction` entirely.

```rust
ActionCompleted {
    action: CompletedAction,
    plan: ActionExecutionPlan,
    outcomes: Vec<ActionOutcome>,
}
```

### Step 9: Update handle_action_completed

File: `crates/app/src/handlers/commands.rs`

Replace `rollback: Vec<(String, String, bool)>` usage with `plan.optimistic: Vec<OptimisticMutation>`. The `rollback_toggles` function is rewritten to accept `&[OptimisticMutation]`.

Replace `params: ActionParams` usage with `plan.compensation: CompensationContext` for undo token construction. (Phase C will clean this up further.)

### Step 10: Update tests

File: `crates/core/src/actions/tests.rs`

4 test functions call `batch_execute` with `BatchAction`. Update to use `MailOperation` + per-target operations.

### Step 11: Remove BatchAction

File: `crates/core/src/actions/batch.rs`, `crates/core/src/actions/mod.rs`

Delete the `BatchAction` enum. Remove `pub use batch::BatchAction` from mod.rs.

## Compilation checkpoints

The edit sequence above is NOT independently compilable at each step. The safe compilation checkpoints are:

1. **After Steps 1-2:** New types defined but unused → compiles with dead code warnings
2. **After Steps 3-6:** Core and app both changed, old dispatch functions removed → first compilation checkpoint where the new path is live
3. **After Steps 7-9:** App's handle_email_action and completion handler use new path → full integration checkpoint
4. **After Steps 10-11:** Tests updated, BatchAction removed → clean final state

**Recommendation:** Do Steps 4-6 (core changes) in one editing pass, then Steps 7-9 (app changes) in the next pass, then compile. Steps 1-2 can be done first as a warmup that compiles independently.

## Risk areas

1. **MoveToFolder source_label_id:** The `move_to_folder_with_provider` function takes `source_label_id: Option<&FolderId>` for the local mutation. MailOperation doesn't carry source (it's compensation data). Need to verify that passing `None` doesn't skip removing the source label locally. If it does, we have two choices: (a) add source to MailOperation (pollutes execution with undo data), or (b) have the local mutation path look up the source from the DB. Investigate before implementing.

2. **pop_out.rs dispatch:** The pop-out overflow menu calls `dispatch_action_service_with_params` directly. Phase B should either update this to use the new path, or keep the legacy function temporarily. If kept, `dispatch_action_service_with_params` needs to stay working — which means `to_batch_action` must survive or be replaced.

3. **pending.rs:** The pending ops retry path calls individual action functions directly, not `batch_execute`. This path is unaffected by Phase B — it doesn't use `BatchAction`. But verify it still compiles after `BatchAction` removal.

4. **In-flight guard:** The per-thread in-flight guard in `execute_account_group` must still work with per-target operations. It currently uses `thread_id` from the target tuple — same position in the new tuple, so no issue expected.
