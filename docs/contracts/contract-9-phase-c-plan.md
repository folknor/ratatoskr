# Contract #9 Phase C: Unified Completion + Undo Migration — Implementation Plan (v2)

## Goal

1. Replace `CompletedAction + ActionParams + rollback tuple` with `ActionExecutionPlan` as the completion handler's input
2. CompletionBehavior established at plan construction (not derived at completion time)
3. Build undo payloads directly from `plan.operations + plan.compensation + plan.optimistic`
4. Move undo payload type to app crate, make palette's stack generic
5. Fix toggle undo grouping: one user action = one Ctrl+Z

`EmailAction` removal is deferred to Phase C2 (cleanup, not coupled to completion/undo).

## Design Decisions (resolved from review)

### D1: CompletionBehavior lives on the plan, not derived from first operation

`ActionExecutionPlan` carries a `CompletionBehavior` set at construction time by `build_execution_plan`. The completion handler reads `plan.behavior` — no derivation, no first-operation assumption. This encodes the single-operation-kind invariant structurally: the plan declares its behavior once, and all operations must be consistent with it.

`completion_behavior()` is a free function in the app crate (not on `MailOperation` in core) — view effects, toast text, and undo policy are UI concerns.

### D2: Star sync responsibilities

Three distinct responsibilities, no overlap:

- **Dispatch time** (build_execution_plan): optimistic star flip in thread list + reading pane `update_star(!previous)`. Already happens in Phase B's `handle_email_action`.
- **Rollback** (handle_action_completed on failure): restore previous values via `rollback_optimistic`. Already happens in Phase B.
- **Completion success**: no star sync needed — optimistic state is already correct.

`PostSuccessEffect::SyncReadingPaneStar` is removed. The only `PostSuccessEffect` is `RefreshNav` (for MarkRead).

```rust
pub enum PostSuccessEffect {
    None,
    RefreshNav,
}
```

### D3: Toast text in a dedicated helper

`format_outcome_toast(behavior: &CompletionBehavior, outcomes: &[ActionOutcome]) -> String` generates all summary variants (all-success, mixed, all-local-only, partial failure). `CompletionBehavior.success_label` is the base label used by this helper.

### D4: UndoEntry carries Vec of payloads — one user action = one undo step

```rust
pub struct UndoEntry<T> {
    pub description: String,
    pub payloads: Vec<T>,
}
```

Toggle actions that affect threads with mixed prior state produce multiple `MailUndoPayload` items (one per account+direction group), but all go in one `UndoEntry`. One Ctrl+Z dispatches all payloads. This fixes the current bug where mixed-direction toggles produce multiple stack entries.

### D5: Description computed at push time, not on payload

`MailUndoPayload` has no `description()` method. The caller computes the description string from the `CompletionBehavior.success_label` and pushes `UndoEntry { description, payloads }`.

### D6: peek/pop return UndoEntry<T>

All call sites (`dispatch_undo`, `Message::Undo` handler) consume `UndoEntry<MailUndoPayload>`. `dispatch_undo` iterates `entry.payloads` and dispatches each compensation. `entry.description` is passed to `Message::UndoCompleted`.

### D7: EmailAction removal deferred to Phase C2

Not coupled to completion/undo. If Phase C hits trouble, this bundling would make rollback harder. Phase C2 is a simple mechanical rename after completion/undo is stable.

---

## New Types

### CompletionBehavior (app crate, action_resolve.rs)

```rust
pub struct CompletionBehavior {
    pub view_effect: ViewEffect,
    pub post_success: PostSuccessEffect,
    pub undo: UndoBehavior,
    pub success_label: &'static str,
}

pub enum ViewEffect {
    Stays,
    LeavesCurrentView,
}

pub enum PostSuccessEffect {
    None,
    RefreshNav,
}

pub enum UndoBehavior {
    Irreversible,
    Reversible,
}
```

Derived via exhaustive match — compiler forces a decision for every `MailOperation` variant:

```rust
pub fn completion_behavior(op: &MailOperation) -> CompletionBehavior {
    match op {
        MailOperation::Archive => CompletionBehavior {
            view_effect: ViewEffect::LeavesCurrentView,
            post_success: PostSuccessEffect::None,
            undo: UndoBehavior::Reversible,
            success_label: "Archived",
        },
        MailOperation::SetRead { .. } => CompletionBehavior {
            view_effect: ViewEffect::Stays,
            post_success: PostSuccessEffect::RefreshNav,
            undo: UndoBehavior::Reversible,
            success_label: "Read status toggled",
        },
        MailOperation::PermanentDelete => CompletionBehavior {
            view_effect: ViewEffect::LeavesCurrentView,
            post_success: PostSuccessEffect::None,
            undo: UndoBehavior::Irreversible,
            success_label: "Permanently deleted",
        },
        // ... exhaustive for all 12 variants
    }
}
```

### MailUndoPayload (app crate, action_resolve.rs)

```rust
pub enum MailUndoPayload {
    Archive { account_id: String, thread_ids: Vec<String> },
    Trash { account_id: String, thread_ids: Vec<String>, source: Option<FolderId> },
    MoveToFolder { account_id: String, thread_ids: Vec<String>, source: FolderId },
    SetSpam { account_id: String, thread_ids: Vec<String>, was_spam: bool },
    SetStarred { account_id: String, thread_ids: Vec<String>, was_starred: bool },
    SetRead { account_id: String, thread_ids: Vec<String>, was_read: bool },
    SetPinned { account_id: String, thread_ids: Vec<String>, was_pinned: bool },
    SetMuted { account_id: String, thread_ids: Vec<String>, was_muted: bool },
    AddLabel { account_id: String, thread_ids: Vec<String>, label_id: TagId },
    RemoveLabel { account_id: String, thread_ids: Vec<String>, label_id: TagId },
    Snooze { account_id: String, thread_ids: Vec<String> },
}
```

No `description()` method (D5).

### Updated ActionExecutionPlan

```rust
pub struct ActionExecutionPlan {
    pub operations: Vec<(String, String, MailOperation)>,
    pub behavior: CompletionBehavior,
    pub compensation: CompensationContext,
    pub optimistic: Vec<OptimisticMutation>,
}
```

`behavior` is set by `build_execution_plan` at construction. The completion handler reads it directly.

### Generic UndoStack (palette crate)

```rust
pub struct UndoStack<T> {
    entries: VecDeque<UndoEntry<T>>,
    capacity: usize,
}

pub struct UndoEntry<T> {
    pub description: String,
    pub payloads: Vec<T>,
}

impl<T> UndoStack<T> {
    pub fn push(&mut self, description: String, payloads: Vec<T>);
    pub fn pop(&mut self) -> Option<UndoEntry<T>>;
    pub fn peek(&self) -> Option<&UndoEntry<T>>;
    pub fn is_empty(&self) -> bool;
    pub fn len(&self) -> usize;
    pub fn clear(&mut self);
}
```

---

## Edit Sequence

### Step 1: Define CompletionBehavior + completion_behavior() + MailUndoPayload

File: `crates/app/src/action_resolve.rs`

Add types and the exhaustive `completion_behavior()` function. Add `build_undo_payloads()`:

```rust
pub fn build_undo_payloads(
    plan: &ActionExecutionPlan,
    outcomes: &[ActionOutcome],
) -> Vec<MailUndoPayload>
```

Logic:
- If `plan.behavior.undo == Irreversible`, return empty
- For toggles (non-empty `plan.optimistic`): group by `(account_id, previous)`, filter to success/local_only outcomes. Produce one `MailUndoPayload` per group.
- For non-toggles: group `plan.operations` by `account_id`, filter to success/local_only outcomes. Derive payload from operation + compensation.
- **Trash with `CompensationContext::SourceFolder(None)`: skip** (safer than bad undo).
- **Spam: derive `was_spam = !to`** from `MailOperation::SetSpam { to }`.

Also add `format_outcome_toast()`:
```rust
pub fn format_outcome_toast(behavior: &CompletionBehavior, outcomes: &[ActionOutcome]) -> String
```

### Step 2: Update ActionExecutionPlan to carry behavior

File: `crates/app/src/action_resolve.rs`

Add `behavior: CompletionBehavior` field. Update `build_execution_plan` to set it:
- For `Resolved`: from `completion_behavior(&resolved.operation)`.
- For `PerThreadToggle`: from `completion_behavior(&toggle_to_operation(field, true))` — the `to` value doesn't affect the behavior class, only the operation semantics.

### Step 3: Make UndoStack generic in palette crate

File: `crates/command-palette/src/undo.rs`, `crates/command-palette/src/lib.rs`

- Replace concrete `UndoStack` with `UndoStack<T>`, `UndoEntry<T>`
- Remove `UndoToken` enum
- Update lib.rs re-exports: `pub use undo::{UndoStack, UndoEntry}` (drop `UndoToken`)
- `push` takes `(description: String, payloads: Vec<T>)`
- `pop` returns `Option<UndoEntry<T>>`
- Remove `UndoToken::description()` method

### Step 4: Change Message::ActionCompleted to carry the plan

File: `crates/app/src/main.rs`

```rust
ActionCompleted {
    plan: crate::action_resolve::ActionExecutionPlan,
    outcomes: Vec<ratatoskr_core::actions::ActionOutcome>,
}
```

Remove `action`, `rollback`, `threads`, `params` fields.

### Step 5: Rewrite handle_action_completed

File: `crates/app/src/handlers/commands.rs`

```rust
pub(crate) fn handle_action_completed(
    &mut self,
    plan: &ActionExecutionPlan,
    outcomes: &[ActionOutcome],
) -> Task<Message>
```

Flow:
1. Read `plan.behavior` directly (no derivation)
2. Compute outcome summary (all_failed, any_failed, any_local_only, all_noop)
3. Early return for all-NoOp or empty plan
4. If all_failed AND LeavesCurrentView → show error, return
5. **Delegate ALL toast text to `format_outcome_toast(&plan.behavior, outcomes)`** — no string construction in the handler. All user-facing text policy is centralized in this one function.
6. Rollback failed toggles via `rollback_optimistic` (typed OptimisticMutation). Reading-pane star updates happen only here and at dispatch time (C9).
7. Build undo payloads via `build_undo_payloads(plan, outcomes)`
8. If non-empty and Reversible: push one `UndoEntry` with description from `plan.behavior.success_label` (action-level, not outcome-summary — C7) + payloads
9. Post-success: LeavesCurrentView → auto-advance, RefreshNav → reload nav

### Step 6: Rewrite execute_undo_compensation for MailUndoPayload

File: `crates/app/src/handlers/commands.rs`

`dispatch_undo` pops `UndoEntry<MailUndoPayload>`, dispatches all payloads:

```rust
pub(crate) fn dispatch_undo(&mut self, entry: UndoEntry<MailUndoPayload>) -> Task<Message> {
    let ctx = ...;
    let desc = entry.description.clone();
    Task::perform(
        async move {
            let mut all_outcomes = Vec::new();
            for payload in &entry.payloads {
                all_outcomes.extend(execute_undo_compensation(&ctx, payload).await);
            }
            (desc, all_outcomes)
        },
        |(desc, outcomes)| Message::UndoCompleted { desc, outcomes },
    )
}
```

`execute_undo_compensation` matches on `MailUndoPayload` variants (same logic as current, typed IDs).

### Step 7: Update dispatch_plan + remove legacy types

File: `crates/app/src/handlers/commands.rs`, `crates/app/src/main.rs`

`dispatch_plan` sends plan + outcomes directly. Empty plans (all threads excluded) return `Task::none()` without dispatching:

```rust
if plan.operations.is_empty() {
    return Task::none();
}
// ...
Message::ActionCompleted { plan, outcomes }
```

Remove: `CompletedAction` enum, `ActionParams` enum, `completed_action_from_operation`, `action_params_from_plan`, `sync_reading_pane_after_toggle`, `rollback_toggles`.

### Step 8: Update App.undo_stack type + main.rs dispatch

File: `crates/app/src/main.rs`

```rust
undo_stack: ratatoskr_command_palette::UndoStack<crate::action_resolve::MailUndoPayload>,
```

Update `Message::Undo` handler to pop `UndoEntry<MailUndoPayload>` and call `dispatch_undo(entry)`.

Update `Message::ActionCompleted` dispatch arm.

### Step 9: Clean up

- Remove `UndoToken` references (it no longer exists after Step 3)
- Remove `CompletedAction` impl block (removes_from_view, success_label)
- Verify pop_out.rs still works (it uses `dispatch_plan` which now sends new Message shape)

---

## Compilation Strategy

**Steps 1-2** compile independently (new types + field added to plan).

**Steps 3-8** are one atomic pass. Generic UndoStack, new Message shape, new completion handler, undo dispatch all depend on each other.

**Step 9** is cleanup.

---

## Invariants

### C1: CompletionBehavior is exhaustive and set once

`completion_behavior()` is an exhaustive match on `MailOperation`. `ActionExecutionPlan.behavior` is set at construction by `build_execution_plan`. The completion handler reads it — no derivation, no first-operation assumption.

All operations in a plan share the same behavior class even if their concrete values differ (e.g., `SetStarred { to: true }` and `SetStarred { to: false }` both produce the same `CompletionBehavior`). This is why a single behavior on the plan is sound.

### C2: Undo payloads built without UI re-read

`build_undo_payloads` reads only from `plan.operations`, `plan.compensation`, `plan.optimistic`, and `outcomes`. No access to thread_list, sidebar, or any other UI state.

### C3: Description set at push time only

`UndoEntry<T>` carries `description: String`. `MailUndoPayload` has no `description()` method. Description is computed from `behavior.success_label` at push time.

### C4: One user action = one undo step

`UndoEntry<T>` carries `Vec<T>` payloads. Toggle actions with mixed prior state produce multiple payloads in one entry. One Ctrl+Z dispatches all.

### C5: Trash undo skipped when source unknown

`build_undo_payloads` does not produce a `MailUndoPayload::Trash` when `plan.compensation` is `CompensationContext::SourceFolder(None)`. Safer than creating an undo that moves to the wrong place.

### C6: Missing-thread toggles are already excluded

Phase B's fix skips threads not found in thread_list entirely — no operation, no optimistic mutation. `build_undo_payloads` will not encounter operations without matching optimistic entries because such operations don't exist.

If ALL selected threads are excluded (all missing from thread_list), `build_execution_plan` returns a plan with zero operations. `dispatch_plan` checks `plan.operations.is_empty()` and returns `Task::none()` — an empty plan is never dispatched.

### C7: Undo descriptions are action-level, not outcome summaries

`UndoEntry.description` is derived from `behavior.success_label` (e.g., "Archived", "Star toggled"). It does NOT include outcome counts or failure details. Toast text may say "Archived 2 of 3 threads" while the undo entry says "Archived". This is intentional — undo describes what will be reversed, not what happened.

### C8: Undo payload execution order is entry order

`dispatch_undo` iterates `entry.payloads` in stored order. This is deterministic and matches the grouping order from `build_undo_payloads`. If one payload fails and another succeeds, the outcomes are collected in payload order and passed to `UndoCompleted`.

### C9: Reading-pane star updates have exactly two call sites

After Phase C, reading-pane star updates exist only in:
1. `handle_email_action` — optimistic apply (before dispatch)
2. `rollback_optimistic` — failure rollback

`sync_reading_pane_after_toggle` is removed. The completion handler does NOT touch the reading pane — optimistic state is already correct on success.

---

## Risk Areas

1. **Toggle undo grouping.** `build_undo_payloads` groups optimistic mutations by `(account_id, previous)`. The zip between `plan.optimistic` and `outcomes` must be index-aligned. Phase B guarantees this via `build_execution_plan`'s strict ordering — the i-th optimistic mutation corresponds to the i-th toggle operation. But this only holds for toggle plans (non-toggle plans have empty optimistic). The grouping logic must check `plan.optimistic.is_empty()` to decide which branch to use.

2. **Palette crate re-exports.** Step 3 changes `lib.rs` to export `UndoStack<T>` and `UndoEntry<T>` instead of `UndoStack` and `UndoToken`. Any external consumer of the palette crate breaks. In this codebase, only the app crate consumes it.

3. **dispatch_undo async boundary.** `UndoEntry<MailUndoPayload>` must be `Send + 'static` for `Task::perform`. All fields are `String`, `Vec<String>`, `FolderId`, `TagId` — all `Send + 'static`.

4. **Pop-out dispatch.** `dispatch_pop_out_action` calls `dispatch_plan` which now produces the new `Message::ActionCompleted` shape. No additional changes needed in pop_out.rs — it's already on the new path from Phase B.

5. **UndoCompleted handler.** Currently receives `{ desc, outcomes }`. `dispatch_undo` constructs this from `entry.description` and the flattened outcomes. No change to the handler needed.
