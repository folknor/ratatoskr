# Contract #9 Phase C: Unified Completion + Undo Migration — Implementation Plan

## Goal

1. Replace `CompletedAction + ActionParams + rollback tuple` with `ActionExecutionPlan` as the completion handler's input
2. Derive completion behavior from `MailOperation::completion_behavior()` (exhaustive match)
3. Build undo payloads directly from `plan.operations + plan.compensation + plan.optimistic`
4. Move undo payload type to app crate, make palette's stack generic
5. Remove `EmailAction` → use `MailActionIntent` in Message enum

## What Phase C Removes

| Legacy type | Replacement |
|-------------|-------------|
| `CompletedAction` enum | `MailOperation::completion_behavior()` → `CompletionBehavior` |
| `ActionParams` enum | Eliminated — data lives in plan.operations + plan.compensation |
| `action_params_from_plan()` | Eliminated |
| `completed_action_from_operation()` | Replaced by `completion_behavior()` |
| `rollback: Vec<(String, String, bool)>` | `plan.optimistic: Vec<OptimisticMutation>` |
| `UndoToken` in command-palette | `MailUndoPayload` in app crate |
| `UndoStack` (concrete) in palette | `UndoStack<T>` (generic) in palette |
| `EmailAction` in command_dispatch | `MailActionIntent` used directly |
| `email_action_to_intent()` adapter | Eliminated |

## New Types

### CompletionBehavior (app crate, action_resolve.rs)

Derived from `MailOperation` via exhaustive match. Compiler forces a decision for every variant.

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
    SyncReadingPaneStar,
}

pub enum UndoBehavior {
    Irreversible,
    Reversible,
}
```

### MailUndoPayload (app crate, action_resolve.rs)

Mail-domain undo compensation data. Built from `plan.operations + plan.compensation + plan.optimistic` without re-reading UI state.

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

### Generic UndoStack (palette crate)

```rust
pub struct UndoStack<T> {
    entries: VecDeque<UndoEntry<T>>,
    capacity: usize,
}

pub struct UndoEntry<T> {
    pub description: String,
    pub payload: T,
}
```

The app instantiates `UndoStack<MailUndoPayload>`.

## Edit Sequence

### Step 1: Define CompletionBehavior + implement MailOperation::completion_behavior()

File: `crates/app/src/action_resolve.rs` (or `crates/core/src/actions/operation.rs` if behavior is universal)

Decision: `completion_behavior()` lives on `MailOperation` in core? Or as a free function in the app crate?

**Recommendation: app crate.** `ViewEffect`, `PostSuccessEffect`, and `success_label` are UI concerns. Core's `MailOperation` should not know about UI toast text. Define as:

```rust
// In action_resolve.rs
pub fn completion_behavior(op: &MailOperation) -> CompletionBehavior {
    match op {
        MailOperation::Archive => CompletionBehavior { ... },
        // ... exhaustive
    }
}
```

### Step 2: Define MailUndoPayload in app crate

File: `crates/app/src/action_resolve.rs`

Also implement `MailUndoPayload::description(&self) -> String` for the undo stack entry.

Also implement `build_undo_payloads()`:

```rust
/// Build undo payloads from a completed plan + outcomes.
/// Filters to success/local_only outcomes only.
/// Groups by account.
/// Returns None for irreversible actions (PermanentDelete).
pub fn build_undo_payloads(
    plan: &ActionExecutionPlan,
    outcomes: &[ActionOutcome],
) -> Vec<MailUndoPayload>
```

This replaces `produce_undo_tokens`. The grouping logic (by account, by previous value for toggles) moves here.

### Step 3: Make UndoStack generic in palette crate

File: `crates/command-palette/src/undo.rs`

Change `UndoStack` to `UndoStack<T>` with `UndoEntry<T>`. Remove `UndoToken` enum (replaced by `MailUndoPayload`).

The `description()` method moves from `UndoToken` to `UndoEntry::description` (set at push time).

Preserve: `push`, `pop`, `peek`, `is_empty`, `len`, `clear`, `capacity`.

### Step 4: Change Message::ActionCompleted to carry the plan

File: `crates/app/src/main.rs`

```rust
ActionCompleted {
    plan: ActionExecutionPlan,
    outcomes: Vec<ActionOutcome>,
}
```

Remove `action`, `rollback`, `threads`, `params` fields. Update the dispatch arm.

### Step 5: Rewrite handle_action_completed

File: `crates/app/src/handlers/commands.rs`

New signature:
```rust
pub(crate) fn handle_action_completed(
    &mut self,
    plan: &ActionExecutionPlan,
    outcomes: &[ActionOutcome],
) -> Task<Message>
```

Flow:
1. Derive `CompletionBehavior` from first operation
2. Compute outcome summary (all_failed, any_failed, any_local_only)
3. Early return for all-NoOp
4. If all_failed AND ViewEffect::LeavesCurrentView → show error, return
5. Show toast using `behavior.success_label`
6. Rollback failed toggles using `plan.optimistic` (typed `OptimisticMutation`)
7. Build undo payloads using `build_undo_payloads(plan, outcomes)`
8. Push undo entries to stack
9. Post-success effects:
   - `ViewEffect::LeavesCurrentView` → auto-advance
   - `PostSuccessEffect::RefreshNav` → reload navigation
   - `PostSuccessEffect::SyncReadingPaneStar` → reading pane star sync

### Step 6: Rewrite execute_undo_compensation for MailUndoPayload

File: `crates/app/src/handlers/commands.rs`

Same structure as current but matches on `MailUndoPayload` variants instead of `UndoToken`. Uses `FolderId`/`TagId` directly (no raw string wrapping).

### Step 7: Replace EmailAction with MailActionIntent in Message

File: `crates/app/src/main.rs`, `crates/app/src/command_dispatch.rs`

Change `Message::EmailAction(EmailAction)` to `Message::EmailAction(MailActionIntent)`.

Update `dispatch_command` and `dispatch_parameterized` in `command_dispatch.rs` to construct `MailActionIntent` directly (remove `EmailAction` enum).

Remove `email_action_to_intent()` adapter.

### Step 8: Update dispatch_plan to use ActionExecutionPlan directly

File: `crates/app/src/handlers/commands.rs`

`dispatch_plan` no longer derives legacy `CompletedAction` or `ActionParams`. It sends the plan + outcomes straight through `Message::ActionCompleted`.

Remove: `completed_action_from_operation`, `action_params_from_plan`, `ActionParams` enum.

### Step 9: Update App.undo_stack type

File: `crates/app/src/main.rs`

```rust
undo_stack: ratatoskr_command_palette::UndoStack<crate::action_resolve::MailUndoPayload>,
```

Update `dispatch_undo` to read `MailUndoPayload` from the stack.

### Step 10: Clean up

- Remove `CompletedAction` enum from main.rs
- Remove `sync_reading_pane_after_toggle` (replaced by CompletionBehavior-driven logic)
- Remove `rollback_toggles` (replaced by `rollback_optimistic` which already exists)
- Remove old `UndoToken` description methods

## Compilation Strategy

**Steps 1-2** compile independently (new types, unused).

**Steps 3-9** are one atomic pass. The generic UndoStack, new Message shape, new completion handler, and undo compensation all depend on each other. Similar to Phase B — edit everything, compile once.

**Step 10** is cleanup that compiles independently.

## Invariants

### C1: CompletionBehavior is exhaustive

`completion_behavior()` is an exhaustive match on `MailOperation`. Adding a new operation variant without defining its behavior is a compiler error.

### C2: Undo payloads built without UI re-read

`build_undo_payloads` reads only from `plan.operations`, `plan.compensation`, and `plan.optimistic`. It does not access `self.thread_list`, `self.sidebar`, or any other UI state. This is the Phase B data sufficiency rule validated.

### C3: Undo description set at push time

`UndoEntry<T>` carries a `description: String` set when the payload is pushed. The palette can display "Undo: Archive" without knowing about `MailUndoPayload`.

### C4: Rollback uses typed OptimisticMutation

`rollback_optimistic` (already exists from Phase B) replaces `rollback_toggles`. Star sync happens as part of rollback, not as a separate step.

### C5: Stable outcome ordering preserved

The completion handler indexes `plan.operations[i]` and `outcomes[i]` in parallel. No regrouping.

## Risk Areas

1. **Toggle undo grouping by previous value.** Current `produce_undo_tokens` groups toggle threads by `(account_id, previous_value)` so threads that were starred get one undo token and threads that were unstarred get another. `build_undo_payloads` must preserve this grouping using `plan.optimistic`.

2. **Trash source_folder comes from CompensationContext, not the operation.** MoveToFolder source is in the operation. Trash source is in compensation. `build_undo_payloads` must read from the right place for each.

3. **UndoStack<T> requires T: Send + 'static** for the async undo dispatch. `MailUndoPayload` contains `String`, `FolderId`, `TagId` — all are `Send + 'static`. No issue.

4. **Palette crate changes.** Making `UndoStack` generic touches the palette crate, which is shared with the command registry. Ensure no palette code depends on `UndoToken`'s concrete fields.

5. **EmailAction removal.** `Message::EmailAction(EmailAction)` is matched in main.rs `update()`. `EmailAction` is constructed in `command_dispatch.rs`. Both must change atomically. The `EmailAction` enum in `command_dispatch.rs` also has methods — check if any are called besides construction.
