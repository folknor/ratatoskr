# Contract #11: Calendar Architecture — Problem Statement

## Problem

Calendar behavior has improved enough that the feature is now meaningfully usable, but it still lacks a clearly enforced architectural contract. Event detail, editing, deletion, seeded data, and surface behavior now work well enough to expose real problems, yet the feature still behaves like a set of working slices rather than a coherent subsystem. That increases the risk that future fixes will be local, correct-looking patches that further entangle view state, editing state, persistence behavior, and ownership rules.

The problem is not simply that calendar has missing features or rough edges. The deeper issue is that calendar sits in an in-between state: substantial enough to need real architectural boundaries, but not yet constrained enough that those boundaries are explicit in the code. Calendar currently depends too heavily on handler-layer conventions about what state is active, which surface is open, and how identity is recovered or preserved. Until those boundaries are explicit, new work will keep succeeding tactically while remaining fragile strategically.

Today those concerns are spread primarily across [crates/app/src/handlers/calendar.rs](/home/folk/Programs/ratatoskr/crates/app/src/handlers/calendar.rs) and [crates/app/src/ui/calendar.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/calendar.rs), where workflow meaning, editor state, and surface state currently live too close together.

## Current Failure Shape

The current implementation mixes four different concerns in one feature path:

1. View/navigation state
   - selected date
   - selected hour
   - active calendar view
   - sidebar mini-month position
   - visible calendars

2. Surface state
   - event detail popover
   - full event modal
   - event editor modal
   - confirm-delete / confirm-discard modal

3. Editor/session state
   - mutable event draft data
   - undo state for text fields
   - dirty detection

4. Persistence and ownership state
   - create/update/delete dispatch
   - account/calendar ownership
   - reload behavior after mutation

Those concerns are not isolated by stable boundaries, so the current code relies on fragile conventions:

- discard confirmation is encoded through the fake delete sentinel `"__discard__"`
- delete semantics are overloaded to mean both “delete persisted event” and “discard unsaved editor changes”
- account ownership is sometimes reconstructed from surface state and sometimes defaulted from `sidebar.accounts.first()`
- create vs edit semantics are carried by `is_new: bool` on the editor modal instead of being represented as a first-class workflow distinction
- event detail loads, editor opening, and modal transitions are intertwined rather than treated as separate lifecycle steps

None of those are isolated bugs. They are symptoms of a missing contract.

## Contract Goals

This contract needs to define:

1. What state belongs to calendar view/navigation
2. What state belongs to event workflow/session
3. What state belongs to UI surfaces only
4. How event identity, account ownership, and calendar ownership are preserved through the feature
5. How editor state and dirty detection are represented
6. What persistence responsibilities calendar may own in `app`, and what must be treated as feature/domain logic

The goal is not to move all calendar logic out of `app`. The goal is to ensure that whatever remains in `app` is presentation logic, while workflow semantics, ownership rules, and mutation boundaries become explicit enough that they cannot silently collapse back into ad hoc state rewriting.

## Ownership Rules

Calendar needs explicit ownership rules in three places:

1. Existing events
   - stable event identity
   - stable account ownership
   - stable calendar ownership when available

2. New event drafts
   - explicit rules for how account/calendar ownership is assigned before save
   - no hidden dependence on unrelated sidebar/default state

3. Surface transitions
   - popover -> full modal -> editor must preserve identity rather than re-derive it from whichever surface happens to be open

Right now those rules are inconsistent. Existing events mostly carry identity correctly, but delete and edit paths still reconstruct ownership from modal state or fall back to defaults when data is missing. That makes event lifecycle correctness depend on current surface wiring rather than stable identity.

## Editor Contract

The editor currently lacks a structurally sound dirty-state model.

The immediate visible bug is that dirty detection only checks a subset of editable fields. But the deeper problem is that the model itself does not capture the full original editable state. `CalendarModal::EventEditor` stores a mutable event draft plus a sibling `original_title: String`, which means the current representation cannot support full-struct dirty comparison even in principle. The issue is not only that some fields were omitted; it is that the model does not represent “original editor state” as a complete object.

As long as that remains true:

- discard confirmation will be incomplete or heuristic
- undo state will remain loosely attached rather than clearly scoped to an editor session
- closing behavior will keep depending on partial field checks instead of a real editor contract

## Loading Contract

Calendar loads already use generation tokens, which is directionally correct. The problem is not the freshness model itself, but what happens immediately after load:

- loaded DB event data is converted into UI/editor data ad hoc
- attendee/reminder enrichment is mixed into surface-opening logic
- surface transitions are driven directly from load completions

So the loading problem is not “stale results win.” It is that loading, conversion, and workflow transition are still too entangled. Calendar needs a clearer boundary between:

- loading event data
- translating it into UI/editor form
- deciding which workflow/session state is entered

## What This Contract Must Eliminate

- sentinel identifiers like `"__discard__"`
- delete semantics overloaded to mean discard confirmation
- partial dirty detection based on an incomplete original-state model
- account/calendar fallback from unrelated sidebar state during mutation
- identity reconstruction from whichever modal or popover is currently open
- direct workflow branching encoded as surface mutation

## Open Questions

1. What is the explicit rule for account/calendar ownership when creating a new event, given that the current editor path does not give the user a real calendar selector and create-event currently falls back to an empty calendar ID? Should the contract require explicit selection before save, or define a stable default-calendar rule?
2. Does this contract need to cover calendar pop-out window state as well, or should pop-out workflow/session behavior be treated as a separate concern?
3. Should attendee/reminder editing remain out of scope until the main event workflow contract is stabilized, or is their omission already causing architectural distortion elsewhere?
