# Calendar: Discrepancies

Audit of `docs/calendar/problem-statement.md` vs codebase. 2026-03-22.

## A. Data Model / Backend

| # | Gap | Details |
|---|-----|---------|
| A1 | **v63 schema fields not surfaced in types** | `DbCalendarEvent` missing: `title`, `timezone`, `recurrence_rule`, `organizer_name`, `rsvp_status`, `created_at`. `DbCalendar` missing: `sort_order`, `is_default`, `provider_id`. Columns exist in SQLite but are invisible to Rust. |
| A2 | **`SELECT *` in calendar queries** | ~10 queries in `calendars.rs` use `SELECT *` — fragile if columns added/reordered. Should use explicit column lists. |
| A3 | **No FK constraints / CASCADE on attendees+reminders** | `calendar_attendees` and `calendar_reminders` have no `FOREIGN KEY ... ON DELETE CASCADE`. `db_delete_calendar_event` doesn't clean up children → orphaned rows. |
| A4 | **Availability + Visibility fields missing entirely** | Not in schema, not in types. Spec requires free/busy/tentative/OOO and public/private on events. |
| A5 | **Recurrence expansion missing** | `recurrence_rule TEXT` column exists (v63) but is never parsed or expanded. Recurring events appear as single instances. No RRULE expansion logic anywhere. |
| A6 | **`calendar_default_view` setting never read** | Seeded in DB (migration v63) but `CalendarState::new()` hardcodes `CalendarView::Month`. Should read from settings table at boot. |

## B. Mode Switcher / Navigation

| # | Gap | Details |
|---|-----|---------|
| B1 | **View switcher buttons in wrong location** | Spec: D/WW/W/M buttons in sidebar header row, right of mode toggle, full height (~76px). Actual: in calendar sidebar body panel (`calendar.rs:769-799`), not in the mail sidebar header. |
| B2 | **No `Ctrl+1` for Mail** | Only `Ctrl+2` (CalendarToggle) exists in command palette registry. No dedicated "Switch to Mail" shortcut. |
| B3 | **No "Switch to Calendar" / "Switch to Mail" named commands** | Only "Toggle Calendar" exists (`registry.rs:673-680`). Spec calls for two distinct named commands. |
| B4 | **Mail state preservation unverified** | Calendar state survives mode toggle (fields stay in `CalendarState`). Mail scroll position has no persistence mechanism — no `scroll_to()` API in this iced fork (UI.md:60). Selected thread likely survives in memory but not tested. |

## C. Calendar Views

| # | Gap | Details |
|---|-----|---------|
| C1 | **No drag-to-select time range** | Click selects a single hour slot (`calendar_time_grid.rs:438-466`). Drag across empty slots to select a range is not implemented. No drag state tracking in `CalendarMessage`. |
| C2 | **No event drag-to-move** | Events render as clickable buttons only. No drag handlers, no position persistence, no move messages. |
| C3 | **No event edge resize** | No edge-hover cursor change, no drag handlers for start/end time adjustment. |
| C4 | **No scroll-to-now / working-hours** | Time grid always renders 00:00–23:59 starting at midnight. No auto-scroll to current time or business hours. Blocked: iced fork lacks `scrollable::scroll_to()` API (UI.md:60). |
| C5 | **Month view: No ISO week numbers** | Spec requires narrow leftmost column with week numbers (1–53), clickable to switch to week view. Not implemented. |
| C6 | **Month view: Multi-day events not spanning as horizontal bars** | Multi-day events are distributed to each spanned day and rendered as individual chips per day cell. Spec requires continuous horizontal bars pinned to top of cell row. |
| C7 | **Weekend columns not narrower in week view** | All 7 columns use identical width. Spec notes weekend columns are "often" narrower. |

## D. Event Detail Popover

| # | Gap | Details |
|---|-----|---------|
| D1 | **Detail is modal, not popover** | Spec: click event → anchored popover (~300px, right/left of block, vertically centered). Actual: centered 420px modal overlay with backdrop dimming. |
| D2 | **No ↗ expand-to-modal button** | Spec has two-tier interaction: popover for quick glance, ↗ opens full modal. Actual: single-tier modal only. |
| D3 | **No two-panel modal layout** | Spec: ~70% event details + ~30% mini day view showing scheduling conflicts. Actual: single full-width card. |
| D4 | **Modal dimensions not responsive** | Fixed 420px width, 560px max-height. Spec: max width 80% of window, full height minus ~30px margins. |
| D5 | **No organizer display** | `organizer_email` exists in DB (v5 schema), `organizer_name` added in v63. Neither surfaced in `CalendarEventData` or rendered in UI. |
| D6 | **No attendees display** | `DbCalendarAttendee` with `rsvp_status` exists in core (`calendars.rs:321-334`). Not loaded into app layer, not rendered. Spec requires per-person status icons (✓/?/~/✗). |
| D7 | **No reminders display or edit** | `DbCalendarReminder` exists in core. Not loaded or rendered. |
| D8 | **No RSVP actions** | No Accept/Decline/Tentative/Dismiss buttons. No context-dependent action button logic (spec cases a/b/c/d). |
| D9 | **No "Email organizer" checkbox** | RSVP-adjacent, not implemented. |
| D10 | **No recurrence display or icon** | No 🔁 icon on event blocks. No recurrence info in detail view. `recurrence_rule` not in app-layer types. |
| D11 | **No calendar selector in event editor** | `calendar_id` stored but no dropdown to choose which calendar to create/move an event to. |
| D12 | **No timezone picker** | No timezone fields anywhere in time picker UI. Spec requires start/end timezone with expansion to 6 fields. |
| D13 | **No recurrence editor** | No recurring toggle, no repeat-interval dropdown, no day-of-week buttons, no month/year radio options, no weekend/holiday avoidance, no end-condition selector. |
| D14 | **No double-click to create event** | Only single-click selects slot → manual "New Event" button. Spec: double-click empty slot opens creation dialog with time pre-filled. |
| D15 | **No recurring event edit/delete prompts** | "This / this and following / all events in the series" not implemented for either edit or delete. |
| D16 | **No ✕ close button on modal** | Closes only via Escape or backdrop click. Spec requires ✕ in top-right. |
| D17 | **No unsaved changes prompt** | Closing editor with modifications doesn't warn. Spec: "closing without saving prompts for confirmation if changes have been made." |
| D18 | **No attendee input field in event creation** | Event creation form has title, date/time, location, description. No attendee field, no autocomplete from contacts. |
| D19 | **No availability field** | free/busy/tentative/out-of-office not in editor or data model. |
| D20 | **No visibility field** | public/private not in editor or data model. |
| D21 | **Location not clickable** | Rendered as plain text. Spec: "clickable link if URL / meeting link." |

## E. Pop-Out Calendar Window

| # | Gap | Details |
|---|-----|---------|
| E1 | **Pop-out calendar window not implemented** | No `PopOutWindow::Calendar` variant in `pop_out/mod.rs`. No ↗ button in calendar UI. No "Pop Out Calendar" command in palette registry. |
| E2 | **No window rules enforcement** | No "one calendar pop-out" limit. No ↗ badge on mode-toggle button when calendar is popped out. No bring-to-foreground behavior on toggle click. |

## F. Email ↔ Calendar Integration

| # | Gap | Details |
|---|-----|---------|
| F1 | **No 📅 button on expanded messages** | Reading pane actions (`reading_pane.rs:17-46`): Reply, ReplyAll, Forward, PopOut. No "Create Event from Email." |
| F2 | **No meeting invite detection** | No iCalendar attachment parsing in email view. No provider-native invite detection. |
| F3 | **No inline RSVP in reading pane** | No event details or RSVP actions rendered when viewing a meeting invitation email. |
| F4 | **No calendar indicator on thread cards** | Meeting invite emails not flagged with 📅 in thread list. |

## G. Calendar Sidebar

| # | Gap | Details |
|---|-----|---------|
| G1 | **Calendar list is a placeholder** | `calendar.rs:740-746` shows "Calendars" text only. No list of calendars with color indicators and visibility checkboxes. Core has `db_get_calendars_for_account()` and `db_set_calendar_visibility()` ready. |
| G2 | **No event dots on mini-month days** | Spec: "Days with events get a subtle dot indicator." Not implemented in mini-month grid. |
| G3 | **Agenda items not clickable** | Right sidebar agenda (`right_sidebar.rs:93-111`) renders event time + title but items have no `on_press` handler. Spec: click should either switch to calendar mode or open a popover. |
| G4 | **No "Open Calendar" entry point in sidebar** | Mini-calendar/agenda area has no button or clickable label to switch to full calendar mode. Only the mode-toggle icon button exists. |

## Summary

**~50 gaps total.** Backend sync is solid (Google, Graph, CalDAV, JMAP all functional). Four views render. Basic event create/edit/delete works. The gaps are:

- **Data model wiring** (A1–A6): v63 fields exist in schema but aren't in Rust types, blocking everything that depends on recurrence, attendees, organizer, timezone, availability, visibility.
- **Event interaction overhaul** (D1–D21): Current modal is a minimal single-panel card. Spec requires two-tier popover→modal with two-panel layout, attendees, reminders, RSVP, recurrence editor, timezone picker, calendar selector, unsaved-changes guard.
- **Drag interactions** (C1–C3): None implemented. Move, resize, range-select all missing.
- **Calendar sidebar** (G1–G4): Calendar list with visibility toggles is a placeholder. Event dots and clickable agenda missing.
- **Pop-out window** (E1–E2): Entire feature missing.
- **Email integration** (F1–F4): Entire feature missing. No 📅 button, no invite detection, no inline RSVP.
- **Navigation polish** (B1–B4, C4–C7): View switcher placement, shortcuts, scroll-to-now, week numbers, multi-day spanning bars.
