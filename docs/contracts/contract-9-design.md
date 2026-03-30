# Contract #9: Unified Action Pipeline — Design

## Problem

Adding a new email action requires ~15 coordinated edits across 5-6 files. But the edit count is a symptom, not the disease. The real problem is that **one action is fractured across 5 types** with lossy conversions between them:

| Type | Location | What it knows |
|------|----------|---------------|
| `EmailAction` | command_dispatch.rs | User intent + inline params |
| `CompletedAction` | main.rs | Bare tag — **loses parameters** |
| `ActionParams` | commands.rs | Parameters as sidecar — **disconnected from action** |
| `BatchAction` | batch.rs (core) | Core operation + params |
| `UndoToken` | undo.rs (command-palette) | Compensation data — **wrong crate** |

Each transition between types is a lossy projection. `CompletedAction` loses the parameters, which then travel separately in `ActionParams`. The completion handler must re-associate them. The undo token carries mail-domain compensation data in a crate that should only know about palette mechanics.

### The 3-class taxonomy is too coarse

The initial design proposed RemovesFromView | Toggle | Batch, but actual behavior is orthogonal:

- Only Star syncs the reading pane
- Only MarkRead refreshes navigation counts
- PermanentDelete is irreversible (no undo)
- Some actions need source-folder recovery for undo
- Toggles are identified implicitly by `rollback.is_empty()`
- Spam is "removes from view" but also a toggle at the provider level

A 3-class enum would force special cases back in immediately.

### Intent vs resolved operation is conflated

`EmailAction::ToggleSpam` is an *intent* — it resolves to "set spam = true" or "set spam = false" depending on the current view. Trash captures `source_label_id` from the sidebar. MoveToFolder needs context from the sidebar's current folder. This resolution happens ad-hoc in `handle_email_action` match arms, scattering UI-context capture across the dispatch code.

### batch_execute forces the toggle special case

`BatchAction` applies one identical operation to all targets. Toggles with mixed target values (some threads starred, some not) must be split into two batches in app code and remerged with index tracking. If the executor accepted per-target resolved operations, toggles wouldn't need a separate dispatch path.

## Proposed Architecture: Intent → Resolve → Execute → Complete

### Phase 1: MailActionIntent

The user's raw intention, carrying inline parameters. This replaces `EmailAction`:

```rust
enum MailActionIntent {
    Archive,
    Trash,
    PermanentDelete,
    ToggleSpam,
    ToggleStar,
    ToggleRead,
    TogglePin,
    ToggleMute,
    MoveToFolder { folder_id: FolderId },
    AddLabel { label_id: TagId },
    RemoveLabel { label_id: TagId },
    Snooze { until: i64 },
    Unsubscribe,
}
```

### Phase 2: ResolvedMailAction

Produced from intent + UI context (sidebar scope, current folder, thread state). All ambiguity resolved — no more "which direction is this toggle?" at execution time:

```rust
enum ResolvedMailAction {
    Archive,
    Trash { source: Option<FolderId> },
    PermanentDelete,
    SetSpam { to: bool },
    SetStarred { to: bool },
    SetRead { to: bool },
    SetPinned { to: bool },
    SetMuted { to: bool },
    MoveToFolder { dest: FolderId, source: Option<FolderId> },
    AddLabel { label_id: TagId },
    RemoveLabel { label_id: TagId },
    Snooze { until: i64 },
}
```

The resolve step captures all UI context at one point:

```rust
fn resolve(intent: MailActionIntent, ctx: &UiContext) -> ResolvedMailAction
```

### Phase 3: ActionExecutionPlan

Produced from the resolved action + target threads. Carries everything needed for execution, optimistic UI, and completion:

```rust
struct ActionExecutionPlan {
    /// Per-target resolved operations (toggles have per-thread target values).
    targets: Vec<(String, String, ResolvedMailAction)>,
    /// Completion behavior — orthogonal flags, not 3 classes.
    policy: CompletionPolicy,
    /// Optimistic UI rollback data, if applicable.
    rollback: Vec<(String, String, bool)>,
}

struct CompletionPolicy {
    removes_from_view: bool,
    refreshes_nav: bool,
    syncs_reading_pane: bool,
    success_label: &'static str,
    reversible: bool,
}
```

`CompletionPolicy` replaces both `removes_from_view()` and `success_label()` and the implicit class-based behavior. Each flag is independent — no "Star is a toggle that also syncs reading pane" special case needed.

### Phase 4: ActionResult

The resolved action survives through execution and arrives in the completion handler intact:

```rust
struct ActionResult {
    plan: ActionExecutionPlan,
    outcomes: Vec<ActionOutcome>,
}
```

The completion handler doesn't need `CompletedAction + ActionParams + rollback` — it has the full `ActionExecutionPlan` with all context preserved. Undo payload construction reads from the resolved actions directly.

### Phase 5: Undo data moves to core (or app)

`UndoToken` leaves the command-palette crate. The palette owns a generic undo *stack* (push/pop/peek), but the payload type is defined in the app or core crate where mail-domain semantics live. The palette just stores `Box<dyn UndoPayload>` or a generic `T`.

## What This Eliminates

| Current | After |
|---------|-------|
| `EmailAction` | `MailActionIntent` (same role, cleaner name) |
| `CompletedAction` + `ActionParams` sidecar | Gone — `ResolvedMailAction` carries everything |
| `to_batch_action` / `to_toggle_batch` mapping | Gone — executor takes per-target resolved ops |
| Toggle split/merge dance in app code | Gone — per-target operations handle mixed values |
| `removes_from_view()` / `success_label()` | `CompletionPolicy` flags |
| `UndoToken` in command-palette | Undo payload in app/core, palette owns the stack |
| `handle_action_completed` 3-branch dispatch | Single handler reads `CompletionPolicy` flags |

Adding a new action:
1. Variant in `MailActionIntent`
2. Variant in `ResolvedMailAction`
3. Arm in `resolve()` (captures UI context)
4. `CompletionPolicy` values (orthogonal flags)
5. Core action function (already needed)
6. Undo builder (if reversible)

Steps 3-4 are in one file. Step 6 is adjacent. The compiler catches missing resolve arms and policy values via exhaustive match.

## Changes to core

### batch_execute accepts per-target operations

```rust
pub async fn batch_execute(
    ctx: &ActionContext,
    operations: Vec<(String, String, ResolvedMailAction)>,
) -> Vec<ActionOutcome>
```

No more `BatchAction` enum — the executor pattern-matches `ResolvedMailAction` directly. Mixed-value toggles are just different operations on different threads in the same batch.

This is the change that eliminates the toggle special case entirely.

## Implementation Strategy

This is a larger refactor than the original descriptor table proposal. Suggested phasing:

### Phase A: Introduce ResolvedMailAction + resolve step
- Add the new types alongside the existing ones
- Wire `handle_email_action` to resolve first, then convert to existing `BatchAction` for dispatch
- No behavior change — just adds the resolve layer

### Phase B: Per-target batch execution
- Change `batch_execute` to accept per-target operations
- Remove toggle split/merge in app code
- `dispatch_toggle_action` and `dispatch_action_service_with_params` merge into one path

### Phase C: Unified completion
- `ActionResult` replaces `Message::ActionCompleted { action, outcomes, rollback, threads, params }`
- `CompletionPolicy` replaces the 3-branch handler
- Single completion handler reads flags

### Phase D: Move undo out of command-palette
- Define undo payload type in app crate
- Palette owns `Vec<Box<dyn Any>>` or generic stack
- Undo construction reads from `ResolvedMailAction` directly

Each phase is independently shippable and testable.

## Risk Assessment

**Medium risk.** This touches core's batch executor API (Phase B) and the Message enum (Phase C), which are higher-blast-radius changes than the original descriptor table proposal. But:

- Each phase is independently compilable
- No provider changes — `ResolvedMailAction` maps to existing action functions
- The palette crate change (Phase D) is additive — old `UndoToken` can coexist during migration
- Compile-time safety is preserved (exhaustive matches on `ResolvedMailAction`)
