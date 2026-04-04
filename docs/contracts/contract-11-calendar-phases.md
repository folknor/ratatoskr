# Contract #11: Calendar Architecture — Phased Implementation Overview

This document divides the work described in the [problem statement](contract-11-calendar-architecture.md) into natural phases. Each phase is independently compilable and shippable. The phases are ordered by dependency: later phases depend on the types and boundaries established by earlier ones.

---

## Phase A: Typed Workflow State

**What it does:** Introduces a single typed enum that represents where the user is in the event lifecycle — viewing, editing (new or existing), confirming discard, confirming delete. The workflow state becomes the authoritative source of truth for what is happening. Surfaces (`active_popover`, `active_modal`) continue to exist as separate fields but are synchronized from workflow state rather than being independently mutated to encode workflow meaning.

**Where it lives:** The workflow state enum lives inside `CalendarState`, replacing the role that `active_modal` currently plays for workflow semantics. `active_popover` and `active_modal` remain as separate fields for rendering purposes — the handler updates workflow state first, then sets surface state to match. Surface derivation (Phase D) eliminates that duplication later.

**Transitional invariant:** During Phases A through C, workflow state is the source of truth for *what is happening*; surface state is the source of truth for *what is rendered*. If they ever disagree, workflow state wins and the surface is considered stale.

**Why it goes first:** Every other phase depends on having a stable, typed representation of "what is happening right now." The `"__discard__"` sentinel, the `is_new: bool` flag, and the identity-recovery-from-modal-state pattern all stem from the same root: workflow meaning is currently encoded inside surface state. Until that is fixed, changes to the editor model, mutation pipeline, or ownership rules will keep re-entangling with surface wiring.

**What it eliminates:**
- `"__discard__"` sentinel — discard becomes a distinct workflow state
- `is_new: bool` on `EventEditor` — create vs edit become distinct workflow states
- identity reconstruction from `active_modal` — the workflow state carries stable event/account/calendar identity through all transitions
- `ConfirmDelete` overloaded for discard — confirm-discard and confirm-delete become separate states

**What it does NOT change:**
- `CalendarEventData` structure stays the same for now
- Dirty detection stays partial (fixed in Phase B)
- Save/delete dispatch stays as-is (fixed in Phase C)
- The handler still owns all the same logic — this phase is about typing the state, not moving code
- `active_popover` and `active_modal` still exist as stored fields (derived in Phase D)

---

## Phase B: Editor Session State

**What it does:** Introduces a dedicated editor session that owns a draft/original pair and undo buffers as a single unit. This is not just "full dirty detection" — it is the introduction of editor-session-local state, where the draft model, the original snapshot, and the undo history all live together inside the editing workflow state from Phase A.

The original snapshot covers only editable persisted fields (title, description, location, time, all-day, calendar, timezone, recurrence, availability, visibility). Derived/display fields (attendees, reminders, organizer metadata, color, calendar name) are read-only display context carried alongside the editor session but outside the draft/original pair.

Undo buffers are editor-session metadata adjacent to the draft/original pair — not inside the draft model itself. The draft is the current editable state; undo buffers are the history of how it got there. They live together in the editor session struct but are separate concerns within it.

**Why it goes second:** Phase A gives the editor a proper workflow state to live inside (`EditingExisting` / `CreatingNew`). Phase B fills that state with a structurally sound editor session model. Without Phase A, there is no clean place for the draft/original pair and undo buffers to live — they would end up as more sibling state on `CalendarModal` or `CalendarState`, which is the current problem.

**What it eliminates:**
- `original_title: String` as the only dirty-detection baseline
- Partial dirty detection (title/description/location only)
- Undo buffers (`editor_undo_title`, `editor_undo_location`, `editor_undo_description`) floating on `CalendarState` disconnected from the editor session they belong to

**What it does NOT change:**
- Save/delete dispatch — still uses the same `handle_save_event` / `DeleteEvent` paths
- Account/calendar ownership fallback — still defaults from sidebar

---

## Phase C: Mutation Intents and Ownership

**What it does:** Introduces typed mutation intents (create, update, delete) that carry explicit account/calendar/event identity at dispatch time. Eliminates the pattern of recovering ownership from whatever modal happens to be open, and the fallback to `sidebar.accounts.first()` during save.

Create vs update is decided by workflow state only. If the workflow state says `CreatingNew`, it is a create. If it says `EditingExisting`, it is an update. The draft's `event.id` should be consistent with the workflow state, but the workflow state is authoritative — no branching on draft fields to decide the operation type.

**Why it goes third:** Phases A and B establish stable identity in the workflow state and a complete draft model. Phase C makes that identity authoritative at the mutation boundary. Without the earlier phases, mutation intents would still need to reach back into surface state for identity, which is the current failure mode.

**What it eliminates:**
- Account fallback from `sidebar.accounts.first()` during save
- `calendar_id.unwrap_or_default()` producing empty string for new events
- Delete path recovering `account_id` from `active_modal` match
- `handle_save_event` branching on `event.id.is_some()` to decide create vs update — the workflow state from Phase A already knows which one this is

**What it does NOT change:**
- View/navigation state — that layer is already clean
- Surface rendering — still the same popovers and modals

**Blocking dependency:** Phase C cannot begin implementation until the new-event ownership rule is decided. The core product question is: what happens when a user tries to save a new event without having selected a calendar?

The recommended rule is: **block save.** If no calendar is selected, the save action is disabled and the calendar selector field shows a visual indicator. No silent defaults, no fallback to first account or first visible calendar. This is the only rule that makes the contract actually enforced rather than "enforced except when we guess." However, this depends on the calendar selector being functional — the current editor path does not give the user a working calendar selector (discrepancy #1 in [calendar discrepancies](../calendar/discrepancies.md)).

If "block save" is chosen, Phase C also requires that the calendar selector be fixed or that a calendar is pre-assigned at editor-open time when unambiguous (e.g., single-account user with one calendar). Those are implementation details for Phase C's design doc, but the decision itself must precede implementation.

---

## Phase D: Surface Derivation

**What it does:** Makes `CalendarPopover` and `CalendarModal` derived from workflow state rather than independently stored and mutated. Closing a surface becomes a typed workflow transition rather than a raw `active_modal = None`.

The target is that surface state is computed from workflow state, not independently mutated. Whether that means a method that returns the surface enum on the fly or a cached field updated write-through is an implementation detail — either satisfies the contract. If some lightweight derived surface cache remains for practical rendering reasons, that still counts as success. The contract is about the direction of authority (workflow → surface), not about eliminating all surface storage.

**Why it goes last:** This is the tightest coupling to view code. It is also the least urgent — the earlier phases eliminate the dangerous failure modes (identity loss, semantic overloading, partial dirty detection). Phase D is about making the code structurally clean, not about fixing correctness gaps.

**What it eliminates:**
- Direct `active_modal = None` / `active_popover = None` as workflow transitions
- Surface state that can drift from workflow state
- The implicit convention that "closing a modal" means different things depending on which modal it is

---

## Phase ordering rationale

The phases are ordered by what unblocks what:

```
A (workflow state) ──► B (editor session) ──► C (mutation intents) ──► D (surface derivation)
```

A is prerequisite for B because the editor session needs a workflow state to live inside. B is prerequisite for C because mutation intents need a complete draft to dispatch from. C is prerequisite for D only loosely — D could technically happen after A — but D is lowest priority and benefits from all three prior phases being stable.

Each phase produces a compilable, testable intermediate state. No phase requires a flag day where the entire calendar handler is rewritten at once.
