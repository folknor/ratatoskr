# Contract #9 Phase B: Per-Target Batch Execution — Implementation Plan (v2)

## Goal

Merge the two dispatch paths (toggle vs non-toggle) into one. The app builds a flat `Vec<(String, String, MailOperation)>` and hands it to core. Core's `batch_execute` accepts per-target operations, groups internally, and returns outcomes in original order. The toggle split/merge dance in the app is eliminated entirely.

## Design Decisions (resolved from review)

### D1: MoveToFolder source is execution data, not undo-only

`move_local()` calls `remove_label(source)` when source is present. Passing `None` leaves the thread in both folders. This is a correctness bug, not a design trade-off.

**Decision:** Add `source: Option<FolderId>` to `MailOperation::MoveToFolder`.

```rust
MailOperation::MoveToFolder { dest: FolderId, source: Option<FolderId> }
```

`CompensationContext::SourceFolder` remains for `Trash` only (trash_local doesn't use source for its local mutation — it adds TRASH label).

### D2: Single typed planning input, not intent + Option sidecar

`build_execution_plan(intent, resolved: Option<ResolvedIntent>, ...)` repeats the `CompletedAction + ActionParams` problem. Valid combinations are implicit.

**Decision:** `resolve_intent` returns `ResolveOutcome`, not `Option<ResolvedIntent>`:

```rust
pub enum ResolveOutcome {
    /// Fully resolved — same operation for all targets.
    Resolved(ResolvedIntent),
    /// Toggle — requires per-thread state to resolve direction.
    PerThreadToggle { field: ToggleField, compensation: CompensationContext },
    /// Fire-and-forget — no core operation (e.g., Unsubscribe).
    NoOp,
}

pub enum ToggleField { Star, Read, Pin, Mute }
```

Then `build_execution_plan` takes `ResolveOutcome` — one parameter, no ambiguity:

```rust
pub fn build_execution_plan(
    outcome: ResolveOutcome,
    threads: &[(String, String)],
    thread_list: &mut ThreadList,
) -> Option<ActionExecutionPlan>
```

Returns `None` only for `ResolveOutcome::NoOp`.

### D3: CompletedAction derived from plan, not stored alongside

Carrying both `CompletedAction` and `ActionExecutionPlan` in `Message::ActionCompleted` creates a dual source of truth.

**Decision:** Derive `CompletedAction` from the plan's first operation at the completion handler boundary:

```rust
fn completed_action_from_plan(plan: &ActionExecutionPlan) -> CompletedAction {
    match &plan.operations[0].2 {
        MailOperation::Archive => CompletedAction::Archive,
        MailOperation::Trash => CompletedAction::Trash,
        // ... exhaustive
    }
}
```

`Message::ActionCompleted` carries only the plan + outcomes:

```rust
ActionCompleted {
    plan: ActionExecutionPlan,
    outcomes: Vec<ActionOutcome>,
}
```

The completion handler calls `completed_action_from_plan` once at the top. Phase C replaces this with `CompletionBehavior` derived from `MailOperation::completion_behavior()`.

### D4: pop_out.rs migrates in Phase B

`dispatch_pop_out_action` must use the new path. Keeping legacy dispatch functions alive prevents removing `BatchAction`. The function is small (~8 lines of logic) and maps cleanly to the new API.

### D5: EmailAction and email_action_to_intent survive Phase B

`Message::EmailAction(EmailAction)` is the Message variant type. Replacing it would touch main.rs dispatch and all command_dispatch callers — too much churn for Phase B. The adapter survives; Phase C removes it.

---

## Current State (after Phase A)

Two dispatch paths:

```
Non-toggle: handle_email_action → resolve_intent → resolved_to_legacy → to_batch_action → batch_execute(ctx, BatchAction, targets)
Toggle:     handle_email_action → optimistic_toggle → dispatch_toggle_action → dispatch_toggle_batch → batch_execute × 2 → merge
```

Phase B collapses into:
```
handle_email_action → email_action_to_intent → resolve_intent → build_execution_plan → batch_execute(ctx, operations)
```

---

## Edit Sequence

### Step 1: Update MailOperation::MoveToFolder to carry source

File: `crates/core/src/actions/operation.rs`

```rust
MoveToFolder { dest: FolderId, source: Option<FolderId> },
```

Update `resolve_intent` in `action_resolve.rs` to put source in the operation instead of compensation:

```rust
MailActionIntent::MoveToFolder { folder_id } => {
    let source = ctx.selected_label.clone().map(FolderId::from);
    ResolveOutcome::Resolved(ResolvedIntent {
        operation: MailOperation::MoveToFolder { dest: folder_id, source },
        compensation: CompensationContext::None,
    })
}
```

Update `resolved_to_legacy` adapter to extract source from the operation.

**Checkpoint:** compiles after updating all MailOperation::MoveToFolder match sites.

### Step 2: Add ResolveOutcome, ToggleField, OptimisticMutation, ActionExecutionPlan

File: `crates/app/src/action_resolve.rs`

Define all new types. Update `resolve_intent` to return `ResolveOutcome`:

```rust
pub fn resolve_intent(intent: MailActionIntent, ctx: &UiContext) -> ResolveOutcome {
    match intent {
        MailActionIntent::Archive => ResolveOutcome::Resolved(ResolvedIntent { ... }),
        MailActionIntent::ToggleStar => ResolveOutcome::PerThreadToggle {
            field: ToggleField::Star,
            compensation: CompensationContext::None,
        },
        MailActionIntent::Unsubscribe => ResolveOutcome::NoOp,
        // ... exhaustive
    }
}
```

**Checkpoint:** compiles (new types unused except by resolve_intent, old callers updated to use ResolveOutcome).

### Step 3: Implement build_execution_plan

File: `crates/app/src/action_resolve.rs`

```rust
pub fn build_execution_plan(
    outcome: ResolveOutcome,
    threads: &[(String, String)],
    thread_list: &mut ThreadList,
) -> Option<ActionExecutionPlan> {
    match outcome {
        ResolveOutcome::Resolved(resolved) => {
            // Same operation for all targets
            let operations = threads.iter()
                .map(|(a, t)| (a.clone(), t.clone(), resolved.operation.clone()))
                .collect();
            Some(ActionExecutionPlan {
                operations,
                compensation: resolved.compensation,
                optimistic: vec![],
            })
        }
        ResolveOutcome::PerThreadToggle { field, compensation } => {
            // Per-thread: read prior → compute op → record mutation → flip UI
            let mut operations = Vec::with_capacity(threads.len());
            let mut optimistic = Vec::with_capacity(threads.len());
            for (account_id, thread_id) in threads {
                if let Some(t) = thread_list.threads.iter_mut().find(
                    |t| t.account_id == *account_id && t.id == *thread_id,
                ) {
                    let (prev, op, mutation) = resolve_toggle(field, t, account_id, thread_id);
                    operations.push((account_id.clone(), thread_id.clone(), op));
                    optimistic.push(mutation);
                    // Step 4: flip UI
                    set_toggle_field(field, t, !prev);
                }
            }
            Some(ActionExecutionPlan { operations, compensation, optimistic })
        }
        ResolveOutcome::NoOp => None,
    }
}
```

Helper functions `resolve_toggle` and `set_toggle_field` encapsulate the per-field logic.

**Checkpoint:** compiles (build_execution_plan defined but not called yet).

### Step 4: Change batch_execute signature in core

File: `crates/core/src/actions/batch.rs`

```rust
pub async fn batch_execute(
    ctx: &ActionContext,
    operations: Vec<(String, String, MailOperation)>,
) -> Vec<ActionOutcome>
```

Rewrite internals:
- `execute_account_group` receives `Vec<(usize, String, MailOperation)>`
- `dispatch_with_provider` matches on `&MailOperation`
- `action_local` matches on `&MailOperation`
- `enqueue_params` matches on `&MailOperation` (MoveToFolder now has source for correct serialization)
- `action_name` matches on `&MailOperation`
- Local-only detection: `matches!(op, SetPinned { .. } | SetMuted { .. } | Snooze { .. })`

### Step 5: Rewrite handle_email_action + remove old dispatch functions

File: `crates/app/src/handlers/commands.rs`

```rust
pub(crate) fn handle_email_action(&mut self, action: EmailAction) -> Task<Message> {
    // ... public folder guard, collect threads ...
    let intent = email_action_to_intent(action);
    let ui_ctx = UiContext { selected_label: self.sidebar.selected_label.clone() };
    let outcome = resolve_intent(intent, &ui_ctx);
    let Some(plan) = build_execution_plan(outcome, &threads, &mut self.thread_list) else {
        // NoOp (Unsubscribe)
        self.status_bar.show_confirmation("Unsubscribed".to_string());
        return Task::none();
    };
    // Star optimistic UI → sync reading pane
    if plan.optimistic.iter().any(|m| matches!(m, OptimisticMutation::SetStarred { .. })) {
        self.sync_reading_pane_from_optimistic(&plan.optimistic, true);
    }
    self.dispatch_plan(plan)
}
```

New `dispatch_plan` method replaces both `dispatch_action_service_with_params` and `dispatch_toggle_action`:

```rust
fn dispatch_plan(&mut self, plan: ActionExecutionPlan) -> Task<Message> {
    let Some(ctx) = self.action_ctx() else { ... };
    let operations = plan.operations.clone();
    Task::perform(
        async move { batch_execute(&ctx, operations).await },
        move |outcomes| Message::ActionCompleted { plan, outcomes },
    )
}
```

**Remove:**
- `dispatch_toggle_action`
- `dispatch_toggle_batch`
- `to_batch_action`
- `to_toggle_batch`
- `resolved_to_legacy`

**Keep:**
- `email_action_to_intent` (EmailAction → MailActionIntent adapter, removed in Phase C)
- `dispatch_action_service` / `dispatch_action_service_with_params` — **only if pop_out.rs is updated separately**. Otherwise remove them too.

### Step 6: Migrate pop_out.rs dispatch

File: `crates/app/src/handlers/pop_out.rs`

Rewrite `dispatch_pop_out_action` to use the new path:

```rust
fn dispatch_pop_out_action(&mut self, window_id, action: CompletedAction) -> Task<Message> {
    // ... extract threads, source_label_id, close menu ...
    let intent = completed_action_to_mail_intent(action);
    let ui_ctx = UiContext { selected_label: source_label_id };
    let outcome = resolve_intent(intent, &ui_ctx);
    let Some(plan) = build_execution_plan(outcome, &threads, &mut self.thread_list) else {
        return Task::none();
    };
    self.dispatch_plan(plan)
}
```

Small helper `completed_action_to_mail_intent` maps the pop-out's `CompletedAction` to `MailActionIntent`. Only Archive/Trash/PermanentDelete are reachable from pop-out overflow.

After this, `dispatch_action_service` and `dispatch_action_service_with_params` have no callers and can be removed.

### Step 7: Update Message::ActionCompleted + handle_action_completed

File: `crates/app/src/main.rs`, `crates/app/src/handlers/commands.rs`

```rust
ActionCompleted {
    plan: ActionExecutionPlan,
    outcomes: Vec<ActionOutcome>,
}
```

In `handle_action_completed`, derive `CompletedAction` from the plan:

```rust
let action = completed_action_from_plan(&result.plan);
```

Then the rest of the handler uses `action` for toast/auto-advance logic (unchanged from current). Uses `plan.optimistic` for rollback instead of anonymous tuples. Uses `plan.compensation` for undo token construction.

**Sub-step: rewrite `rollback_toggles`** to accept `&[OptimisticMutation]` instead of `&[(String, String, bool)]`.

**Sub-step: rewrite `produce_undo_tokens`** to use `plan.compensation` and `plan.optimistic` instead of `ActionParams` and `rollback`.

**Sub-step: rewrite `sync_reading_pane_after_toggle`** to accept `&[OptimisticMutation]` and check for `SetStarred` variants.

### Step 8: Update tests + remove BatchAction

File: `crates/core/src/actions/tests.rs`, `crates/core/src/actions/batch.rs`, `crates/core/src/actions/mod.rs`

- Update 4 test functions to use `MailOperation` + per-target operations
- Delete `BatchAction` enum
- Remove `pub use batch::BatchAction` from mod.rs

---

## Compilation Strategy

**Steps 1-2 compile independently** (type changes + new types with warnings).

**Steps 3-8 are one atomic pass.** Once `batch_execute`'s signature changes, all callers must change simultaneously. This includes:
- Core batch.rs internals (Step 4)
- App handle_email_action + dispatch_plan (Step 5)
- App pop_out.rs (Step 6)
- Message::ActionCompleted + completion handler (Step 7)
- Tests (Step 8)

**Recommended workflow:** Edit Steps 4-8 in sequence without attempting to compile between them. Compile once at the end. Fix any issues.

---

## Risk Areas

1. **enqueue_params for MoveToFolder:** Now that `MailOperation::MoveToFolder` carries `source`, `enqueue_params` can serialize it correctly. The pending ops retry path in `pending.rs` already deserializes `sourceLabelId` from JSON and passes it through — this path is unaffected by the `BatchAction` removal.

2. **In-flight guard:** Uses `thread_id` from the target tuple — same position `(String, String, MailOperation)` vs old `(String, String)`. No issue.

3. **sync_reading_pane_after_toggle:** Currently called both pre-dispatch (in handle_email_action) and during rollback (in handle_action_completed). Both paths must be updated to use `OptimisticMutation`.

4. **produce_undo_tokens:** Currently reads `CompletedAction`, `ActionParams`, and `rollback`. All three are replaced. This function is the most complex single rewrite in Phase B — trace through every arm carefully.
