# Contract #11: Calendar Architecture

## Status

Implemented. Phases A–D completed. This document describes the architectural boundaries the calendar feature now enforces.

## Architecture

Calendar separates four concerns with explicit typed boundaries:

1. **View/navigation state** — selected date, selected hour, active view, mini-month position, visible calendars. Lives on `CalendarState`. No workflow or editing semantics.

2. **Workflow state** — `CalendarWorkflow` enum: `Idle`, `ViewingEvent`, `CreatingEvent`, `EditingEvent`, `ConfirmingDiscard`, `ConfirmingDelete`. Source of truth for what the user is doing. All lifecycle meaning and identity comes from this enum.

3. **Editor session state** — `EditorSession` on `CreatingEvent` / `EditingEvent`: mutable draft, original snapshot (`EventSnapshot` with 14 editable fields), per-field undo buffers. Single source of truth for all editable event state during editing.

4. **Surface state** — `CalendarPopover` / `CalendarModal` on `CalendarState`. Presentation caches written exclusively by `CalendarState::sync_surfaces()`, derived from workflow state. Never independently mutated by handlers.

## Invariants

1. **Workflow-first:** Handlers update workflow state, then call `sync_surfaces()`. No handler writes `active_modal` or `active_popover` directly.

2. **Workflow-authoritative:** Reads of lifecycle meaning and identity come from workflow state only. Surfaces are never used to recover workflow semantics.

3. **Editor session owns editable state:** No handler reads editable event data from `active_modal`. The `CalendarModal::EventEditor` variant is a unit marker — the draft lives on `EditorSession` in the workflow.

4. **Dirty detection is full-struct:** `EditorSession::is_dirty()` compares `EventSnapshot` (14 editable fields) against the original snapshot. No partial field checks.

5. **`EventEditor` modal without `CreatingEvent`/`EditingEvent` workflow is a contract violation.** View code asserts this.

## Identity Ownership

- **`event_id`**: Workflow state is authoritative (`EditingEvent.event_id`, `ConfirmingDelete.event_id`). `session.draft.id` is a carried display copy, consistent by construction, never read for lifecycle decisions.

- **`account_id`**: Workflow state is authoritative for mutation dispatch (`CreatingEvent.account_id`, `EditingEvent.account_id`). `session.draft.account_id` is a synced display copy — updated alongside the workflow field by the `CalendarSelected` handler.

- **`calendar_id`**: The draft (`session.draft.calendar_id`) is the authoritative editable source. Not carried on workflow variants. `handle_save_event` reads it from the session draft.

## Ownership Rules

1. **Existing events** carry stable identity on their workflow variant.

2. **New event drafts** must acquire ownership before save. If `calendar_id` is `None`, save is blocked. Pre-assignment occurs when unambiguous (single eligible calendar).

3. **Surface transitions** (popover → full modal → editor) preserve identity through the workflow state. `ViewingSurface` captures the popover-vs-modal distinction for `ExpandPopoverToModal` — the one transition where workflow identity stays the same while presentation changes.

4. **Calendar picker is disabled for existing events.** Moving an event between calendars requires provider-specific semantics not yet implemented.

## Editor Contract

The editor owns an `EditorSession` containing:
- `draft: CalendarEventData` — mutable, updated as the user types
- `original: EventSnapshot` — snapshot at editor open time
- `undo_title`, `undo_location`, `undo_description: UndoableText` — scoped to the session

Discard confirmation (`ConfirmingDiscard`) preserves the full session so cancel-discard can restore the editor.

## Loading Contract

Calendar loads use generation tokens for async staleness detection. Loaded event data (including enriched attendees, reminders, calendar name, color) is stored on `ViewingEvent.event_data` in the workflow state. Surface derivation reads from this workflow data.

## What This Contract Eliminated

- Sentinel identifiers (`"__discard__"`)
- Delete semantics overloaded for discard confirmation
- `is_new: bool` on the editor modal
- `original_title: String` as the only dirty-detection baseline
- Account/calendar fallback from `sidebar.accounts.first()` during save
- Identity reconstruction from whichever modal or popover is currently open
- Direct surface mutation by handlers (now centralized in `sync_surfaces()`)

## Open Questions

1. ~~Account/calendar ownership for new events~~ **Resolved.** Pre-assign when unambiguous, block save otherwise.
2. ~~Calendar pop-out state~~ Not addressed by this contract. Pop-out may need its own workflow/session state in the future.
3. Attendee/reminder editing remains out of scope. These are read-only display fields excluded from `EventSnapshot`. No architectural distortion observed from their omission.
