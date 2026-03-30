# Contract #9: Action Descriptor Table — Design

## Problem

Adding a new email action requires 8 coordinated edits across 4 files. Missing any one silently degrades (no undo, wrong toast, batch routing failure). The edits are:

1. `EmailAction` variant (command_dispatch.rs)
2. `CompletedAction` variant + `removes_from_view()` + `success_label()` (main.rs)
3. `BatchAction` variant (batch.rs)
4. `to_batch_action` mapping (commands.rs)
5. `to_toggle_batch` mapping — toggles only (commands.rs)
6. `handle_action_completed` arm (commands.rs)
7. `UndoToken` variant + undo execution arm (commands.rs)
8. `handle_email_action` arm (commands.rs)

## Key Observation: Three Action Classes

Actions aren't uniform — they fall into 3 dispatch classes with different UI behavior:

### 1. Removes-from-view (Archive, Trash, Spam, MoveToFolder, PermanentDelete, Snooze)
- Dispatched via `dispatch_action_service_with_params` → `batch_execute`
- On completion: auto-advance cursor, show toast
- On failure: show error toast
- Undo: produces token, reverse operation

### 2. Toggles (Star, MarkRead, Pin, Mute)
- Dispatched via `dispatch_toggle_action` → `dispatch_toggle_batch`
- Optimistic UI: flip immediately, rollback on failure
- On completion: refresh nav (for read count), reading pane sync (for star)
- Undo: produces token with rollback data

### 3. Labels (AddLabel, RemoveLabel)
- Dispatched via `dispatch_action_service_with_params` → `batch_execute`
- Non-removes-from-view: no auto-advance
- On completion: show toast
- Undo: produces reverse-label token

## Proposed Design: Static Descriptor Table

Replace the scattered match arms with a single descriptor per action:

```rust
struct ActionDescriptor {
    /// How this action dispatches and what UI behavior it gets.
    class: ActionClass,
    /// Toast text on success.
    success_label: &'static str,
    /// How to build a BatchAction from ActionParams.
    to_batch: fn(&ActionParams) -> Option<BatchAction>,
    /// How to build an UndoToken from (account_id, thread_ids, params, rollback).
    to_undo: Option<fn(&str, Vec<String>, &ActionParams, &[(String, String, bool)]) -> UndoToken>,
}

enum ActionClass {
    /// Thread leaves the current view. Auto-advance on success.
    RemovesFromView,
    /// Optimistic flip with rollback on failure.
    Toggle { get_field: fn(&mut Thread) -> &mut bool },
    /// Non-view-changing batch action (labels).
    Batch,
}
```

Then `CompletedAction` carries an index into the descriptor table (or is the key itself), and the dispatch/completion/undo logic becomes generic:

```rust
// In handle_email_action — one arm per class, not per action
match descriptor.class {
    ActionClass::RemovesFromView => dispatch_action_service_with_params(...),
    ActionClass::Toggle { get_field } => {
        let rollback = self.optimistic_toggle(&threads, get_field);
        dispatch_toggle_action(...)
    },
    ActionClass::Batch => dispatch_action_service_with_params(...),
}

// In handle_action_completed — same
match descriptor.class {
    ActionClass::RemovesFromView => { auto_advance(); show_toast(); }
    ActionClass::Toggle { .. } => { rollback_failed(); refresh_nav_if_read(); }
    ActionClass::Batch => { show_toast(); }
}

// Undo token production — generic
if let Some(to_undo) = descriptor.to_undo {
    // ... produce token using the closure
}
```

## What This Eliminates

| Before | After |
|--------|-------|
| Match arm in `handle_email_action` | Entry in descriptor table |
| `removes_from_view()` method | `ActionClass::RemovesFromView` |
| `success_label()` method | `success_label` field |
| `to_batch_action` match arm | `to_batch` closure |
| `to_toggle_batch` match arm | `ActionClass::Toggle` |
| Undo token match arm | `to_undo` closure |

Adding a new action = one struct literal. Missing a field = compile error (no `Option` on required fields).

## Implementation Steps

1. **Define `ActionDescriptor` and `ActionClass`** in a new `crates/app/src/action_registry.rs`
2. **Build the static table** — one `ActionDescriptor` per `CompletedAction` variant
3. **Replace `removes_from_view()` and `success_label()`** with table lookups
4. **Replace `to_batch_action()` and `to_toggle_batch()`** with `descriptor.to_batch()`
5. **Replace undo token construction** with `descriptor.to_undo()`
6. **Replace `handle_email_action` match** with class-based dispatch
7. **Replace `handle_action_completed` match** with class-based completion handling

Steps 3-5 can be done incrementally (one function at a time). Steps 6-7 are the big payoff.

## Open Questions

- **Should `EmailAction` and `CompletedAction` be unified?** They're nearly identical. The difference is that `EmailAction` carries parameters inline (`MoveToFolder { folder_id }`) while `CompletedAction` is a bare enum and parameters live in `ActionParams`. Unifying would mean one enum, but the parameter handling would need to change.

- **Should `BatchAction` be generated from the descriptor?** Currently `BatchAction` is defined in `core`. If the descriptor table owns the `to_batch` mapping, `BatchAction` could stay as-is — the closure constructs it. Or `BatchAction` could be replaced by `(CompletedAction, ActionParams)` pairs passed to `batch_execute`.

- **Thread field accessor for toggles** — The `ActionClass::Toggle { get_field }` approach uses `fn(&mut Thread) -> &mut bool`, which ties the descriptor to the UI-layer `Thread` type. This is fine since the descriptor table lives in the app crate.

## Risk Assessment

**Low risk.** The refactor is internal to the app crate's command handler. Core's action service, provider trait, and batch executor are unchanged. The only public API change is replacing match arms with table lookups — same behavior, different structure.

**Testing approach:** Compile-time verification (exhaustive match removed = compiler can't catch missing entries). Mitigation: the descriptor table constructor can assert at startup that every `CompletedAction` variant has an entry.
