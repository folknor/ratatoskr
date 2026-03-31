# Contract #9: Unified Action Pipeline — Design (v3)

## Problem

Adding a new email action requires ~15 coordinated edits across 5-6 files. But the edit count is a symptom, not the disease. The real problem is that **one action is fractured across 5 types** with lossy conversions between them:

| Type | Location | What it knows |
|------|----------|---------------|
| `EmailAction` | command_dispatch.rs | User intent + inline params |
| `CompletedAction` | main.rs | Bare tag — **loses parameters** |
| `ActionParams` | commands.rs | Parameters as sidecar — **disconnected from action** |
| `BatchAction` | batch.rs (core) | Core operation + params |
| `UndoToken` | undo.rs (cmdk) | Compensation data — **wrong crate** |

Each transition between types is a lossy projection. `CompletedAction` loses the parameters, which then travel separately in `ActionParams`. The completion handler must re-associate them. The undo token carries mail-domain compensation data in a crate that should only know about palette mechanics.

### Further problems identified during review

- **The 3-class taxonomy (RemovesFromView | Toggle | Batch) is too coarse.** Actual behavior axes are orthogonal: only Star syncs the reading pane, only MarkRead refreshes nav, PermanentDelete is irreversible, some actions need source-folder for undo, Spam is "removes from view" but also a provider-level toggle.
- **Intent vs resolved operation is conflated.** `ToggleSpam` resolves differently based on current view. Trash captures `source_label_id` from sidebar context. Resolution happens ad-hoc across dispatch code.
- **batch_execute's uniform opcode forces the toggle special case.** Mixed-value toggles must be split into two batches and remerged with index tracking.
- **Undo provenance is baked into operation semantics.** `Trash { source }` carries undo-only metadata into the execution layer. Core shouldn't pattern-match on fields that exist only for compensation.

---

## Architecture: Intent → Resolve → Plan → Execute → Complete

### Layer 1: MailActionIntent (app crate)

The user's raw intention. Replaces `EmailAction`. Carries inline parameters but no resolved context.

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

### Layer 2: Resolution (app crate)

Two distinct steps. Single-action resolution is separate from per-target planning.

#### Step 2a: resolve_intent

Collapses ambiguity using UI context. Produces a `ResolvedIntent` — a fully specified operation plus compensation context. Consumes the intent (one-shot value).

```rust
/// UI context captured at resolution time.
struct UiContext {
    current_scope: ViewScope,
    selected_label: Option<String>,
    // Per-thread state is NOT here — that's per-target planning.
}

/// Fully resolved intent: what operation + what to remember for undo.
struct ResolvedIntent {
    /// Pure operation semantics — what core executes.
    operation: MailOperation,
    /// Compensation metadata — what undo needs. Not sent to core.
    compensation: CompensationContext,
}

fn resolve_intent(intent: MailActionIntent, ctx: &UiContext) -> ResolvedIntent
```

**`MailOperation`** contains only execution semantics. No undo-only fields. Core owns this type.

```rust
/// Pure mail operation — owned by core. No UI or undo metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
enum MailOperation {
    Archive,
    Trash,
    PermanentDelete,
    SetSpam { to: bool },
    SetStarred { to: bool },
    SetRead { to: bool },
    SetPinned { to: bool },
    SetMuted { to: bool },
    MoveToFolder { dest: FolderId },
    AddLabel { label_id: TagId },
    RemoveLabel { label_id: TagId },
    Snooze { until: i64 },
}
```

**`CompensationContext`** carries undo-only provenance. App owns this type. Never sent to core.

```rust
/// Undo/recovery metadata captured at resolution time.
/// Lives in app crate, never crosses into core.
enum CompensationContext {
    None,
    /// Source folder for undo-trash / undo-move.
    SourceFolder(Option<FolderId>),
    /// Undo label: which label was added/removed.
    Label(TagId),
}
```

#### Step 2b: build_execution_plan

Takes the resolved intent + selected threads. For toggles, reads per-thread state, computes per-thread `MailOperation`, records `OptimisticMutation`, then mutates UI — in that exact order to prevent double-flip or deriving previous from already-mutated state.

```rust
fn build_execution_plan(
    resolved: ResolvedIntent,
    threads: &[(String, String)],
    thread_list: &mut ThreadList,  // for optimistic mutation
) -> ActionExecutionPlan
```

**Ordering contract for toggles:**
1. Read prior thread state (`is_starred`, `is_read`, etc.)
2. Compute per-thread `MailOperation` (e.g., `SetStarred { to: !previous }`)
3. Record typed `OptimisticMutation` with the `previous` value
4. Mutate UI state (flip the bool in the thread list)

Steps 1-4 must happen in this order per thread. Deriving `previous` from already-mutated state is a bug.

```rust
struct ActionExecutionPlan {
    /// Per-target operations — always a flat vec, even for uniform actions.
    /// The executor groups identical operations internally for potential
    /// provider-level batching; the plan doesn't need to distinguish
    /// uniform from per-target.
    operations: Vec<(String, String, MailOperation)>,
    /// How to handle completion — derived via exhaustive match, not ad-hoc.
    completion: CompletionBehavior,
    /// Compensation context from resolution (source folder, etc.).
    compensation: CompensationContext,
    /// Optimistic UI mutations applied, if any.
    optimistic: Vec<OptimisticMutation>,
}
```

**Design note:** No `ActionTargets::Uniform | PerTarget` enum. The plan always builds `Vec<(String, String, MailOperation)>` directly. "Uniform" is just the degenerate case where every entry has the same operation — the executor can detect this as an optimization, but the plan doesn't need two representations. This avoids preserving two code paths in the app when the goal is to collapse them.

### Typed optimistic mutations (not anonymous tuples)

```rust
/// What was flipped in the UI before execution. Typed per field.
enum OptimisticMutation {
    SetStarred { account_id: String, thread_id: String, previous: bool },
    SetRead { account_id: String, thread_id: String, previous: bool },
    SetPinned { account_id: String, thread_id: String, previous: bool },
    SetMuted { account_id: String, thread_id: String, previous: bool },
}
```

**Data sufficiency rule:** Phase C must be able to build `MailUndoPayload` from `MailOperation` + `CompensationContext` + `OptimisticMutation` without re-reading UI state. If Phase B has not captured enough data in these three types, Phase C cannot produce correct undo. This is the key invariant that validates Phase B's completeness.

### Typed completion behavior (not boolean flags)

Derived from `MailOperation` via exhaustive match — adding a variant forces you to define the behavior. The completion handler reads typed effects, not booleans.

```rust
/// Derived from MailOperation via exhaustive match.
struct CompletionBehavior {
    view_effect: ViewEffect,
    post_success: PostSuccessEffect,
    undo: UndoBehavior,
    success_label: &'static str,
}

enum ViewEffect {
    /// Thread stays in current view (toggles, labels).
    Stays,
    /// Thread leaves current view (archive, trash, spam, move, delete, snooze).
    LeavesCurrentView,
}

enum PostSuccessEffect {
    None,
    /// Refresh navigation sidebar (unread counts changed).
    RefreshNav,
    /// Sync star state to reading pane.
    SyncReadingPaneStar,
}

enum UndoBehavior {
    Irreversible,
    Reversible,
}

impl MailOperation {
    /// Exhaustive match — compiler forces a decision for every variant.
    fn completion_behavior(&self) -> CompletionBehavior {
        match self {
            Self::Archive => CompletionBehavior {
                view_effect: ViewEffect::LeavesCurrentView,
                post_success: PostSuccessEffect::None,
                undo: UndoBehavior::Reversible,
                success_label: "Archived",
            },
            Self::SetStarred { .. } => CompletionBehavior {
                view_effect: ViewEffect::Stays,
                post_success: PostSuccessEffect::SyncReadingPaneStar,
                undo: UndoBehavior::Reversible,
                success_label: "Star toggled",
            },
            Self::SetRead { .. } => CompletionBehavior {
                view_effect: ViewEffect::Stays,
                post_success: PostSuccessEffect::RefreshNav,
                undo: UndoBehavior::Reversible,
                success_label: "Read status toggled",
            },
            Self::PermanentDelete => CompletionBehavior {
                view_effect: ViewEffect::LeavesCurrentView,
                post_success: PostSuccessEffect::None,
                undo: UndoBehavior::Irreversible,
                success_label: "Permanently deleted",
            },
            // ... exhaustive for all variants
        }
    }
}
```

### Layer 3: Execution (core crate)

Core's batch executor accepts per-target operations. Core owns `MailOperation`.

```rust
/// Execute operations across multiple threads.
/// Groups by account, reuses one provider per account.
/// Regroups identical operations for potential provider-level batching.
pub async fn batch_execute(
    ctx: &ActionContext,
    operations: Vec<(String, String, MailOperation)>,
) -> Vec<ActionOutcome>
```

**Executor regrouping contract:**
- Groups operations by `(account_id, MailOperation)` using exact enum value equality (`PartialEq` + `Eq`). This means grouping is by full Rust value shape — `SetStarred { to: true }` and `SetStarred { to: false }` are different groups. If "same provider dispatch shape" (ignoring field values) is ever needed, introduce an explicit dispatch-key type rather than relaxing equality.
- Execution order within an account may change due to regrouping, but **outcome ordering is by original input position** (`outcomes[i]` corresponds to `operations[i]`). All undo/rollback bookkeeping must key off original operation order, not regrouped order.
- Within each group, dispatches to the provider (future: multi-target provider calls for IMAP STORE, Graph $batch, JMAP Email/set)
- Provider reuse per account preserved
- Degraded/local-only behavior unchanged

### Layer 4: Completion (app crate)

The completion handler receives the full plan + outcomes. No lossy re-association needed.

```rust
struct ActionResult {
    /// The plan that was executed.
    plan: ActionExecutionPlan,
    /// Per-target outcomes, same order as plan.targets.
    outcomes: Vec<ActionOutcome>,
}
```

The `Message::ActionCompleted` variant carries `ActionResult` directly — no more `{ action, outcomes, rollback, threads, params }` bundle.

The handler reads `plan.completion` flags:
- `ViewEffect::LeavesCurrentView` → auto-advance
- `PostSuccessEffect::RefreshNav` → reload navigation
- `PostSuccessEffect::SyncReadingPaneStar` → update reading pane
- `UndoBehavior::Reversible` → build undo token from `plan.compensation` + `plan.optimistic`

### Layer 5: Undo (app crate + palette crate)

**Undo payload** is a mail-domain type defined in the app crate:

```rust
/// Mail-specific undo compensation data. Lives in app crate.
enum MailUndoPayload {
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

**Palette crate** owns a generic undo stack:

```rust
/// Generic undo stack. Palette doesn't know about mail semantics.
pub struct UndoStack<T> {
    entries: Vec<UndoEntry<T>>,
    max_depth: usize,
}

pub struct UndoEntry<T> {
    pub description: String,
    pub payload: T,
}
```

The app instantiates `UndoStack<MailUndoPayload>`. The palette queries `has_undo: bool` via the command context for the "Undo" command's availability check.

---

## What This Eliminates

| Current | After |
|---------|-------|
| `EmailAction` | `MailActionIntent` (same role, cleaner name) |
| `CompletedAction` + `ActionParams` sidecar | Gone — `MailOperation` + `CompensationContext` |
| `BatchAction` | Gone — core accepts `MailOperation` directly |
| `to_batch_action` / `to_toggle_batch` | Gone — plan builds per-target operations |
| Toggle split/merge dance | Gone — per-target operations in flat vec |
| `removes_from_view()` / `success_label()` | `CompletionBehavior` via exhaustive match |
| Anonymous `rollback: Vec<(String, String, bool)>` | Typed `OptimisticMutation` |
| `UndoToken` in cmdk | `MailUndoPayload` in app, `UndoStack<T>` in palette |
| `handle_action_completed` 3-branch dispatch | Single handler reads typed effects |

Adding a new action:
1. Variant in `MailActionIntent`
2. Variant in `MailOperation` (core) + arm in `completion_behavior()` — compiler-enforced
3. Arm in `resolve_intent()` — compiler-enforced
4. Core action function
5. `MailUndoPayload` variant + compensation builder (if reversible)

Steps 2-3 are compile-time enforced via exhaustive match. No silent degradation possible.

---

## Implementation Phases

### Phase A: Introduce MailOperation + resolve step
- Define `MailActionIntent`, `MailOperation`, `UiContext`, `ResolvedIntent`
- Define `CompensationContext`
- Add `resolve_intent()` in app crate
- Wire `handle_email_action` to resolve first, then convert to existing `BatchAction` for dispatch
- **Pitfall:** The `ResolvedIntent → BatchAction` adapter must be a single exhaustive match in one place

### Phase B: Per-target batch execution + typed optimistic mutations
- Define `OptimisticMutation` (typed per field, not anonymous tuples)
- Implement `build_execution_plan()` with strict ordering: read prior state → compute operation → record mutation → flip UI
- Change `batch_execute` to accept `Vec<(String, String, MailOperation)>`
- Core groups by exact `(account_id, MailOperation)` value for dispatch; outcomes ordered by original input position
- Remove toggle split/merge in app code — toggles are just per-target operations in the flat vec
- Merge `dispatch_toggle_action` and `dispatch_action_service_with_params` into one path
- Remove Phase A toggle placeholder paths from `resolve_intent` (already done: toggles return `None`)
- **Key invariant:** `MailOperation + CompensationContext + OptimisticMutation` must be sufficient for Phase C to build undo without re-reading UI state

### Phase C: Unified completion + undo migration (merged — they're coupled)
- Define `CompletionBehavior`, `ViewEffect`, `PostSuccessEffect`, `UndoBehavior`
- Implement `MailOperation::completion_behavior()` — exhaustive match
- `ActionResult` replaces `Message::ActionCompleted { action, outcomes, rollback, threads, params }`
- Define `MailUndoPayload` in app crate
- Make palette's undo stack generic: `UndoStack<T>`
- Single completion handler reads typed effects
- Undo construction reads from `MailOperation` + `CompensationContext` + `OptimisticMutation`

Each phase is independently compilable and testable. Phase A is purely additive. Phase B changes the core API. Phase C is the largest but is internal to the app crate + palette crate.

---

## Design Review History

- **v1 (descriptor table):** Rejected. Optimized the wrong thing — catalogued the 5-type split instead of eliminating it. Boolean flags lost compile-time safety.
- **v2 (unified pipeline):** Intent → Resolve → Execute → Complete. Correct decomposition. Reviewers identified: undo provenance mixed into operations, boolean flags still too loose, rollback untyped, undo stack should be generic not `dyn Any`.
- **v3 (this version):** Separates `MailOperation` (core, pure semantics) from `CompensationContext` (app, undo metadata). Typed completion behavior via exhaustive match. Typed optimistic mutations. Generic `UndoStack<T>`. Explicit `UiContext` and executor regrouping contract.
