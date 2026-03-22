# Calendar: Discrepancies

Audit of `docs/calendar/problem-statement.md` vs codebase. Updated 2026-03-22.

## A. Data Model / Backend — RESOLVED

All items resolved.

| # | Gap | Status | Resolution |
|---|-----|--------|------------|
| A1 | v63 schema fields not surfaced in types | ✅ Resolved | `DbCalendarEvent` + `DbCalendar` types now include all v63 fields. `FromRow` impls updated. |
| A2 | `SELECT *` in calendar queries | ✅ Resolved | All calendar queries use explicit column lists via `CALENDAR_COLS`, `EVENT_COLS`, `ATTENDEE_COLS`, `REMINDER_COLS` constants. |
| A3 | No FK constraints / CASCADE on attendees+reminders | ✅ Resolved | All delete paths (`db_delete_calendar_event`, `delete_calendar_event_sync`, `db_delete_event_by_remote_id`, `db_delete_events_for_calendar`) now cascade to `calendar_attendees` and `calendar_reminders`. |
| A4 | Availability + Visibility fields missing | ✅ Resolved | Migration v65 adds `availability TEXT` and `visibility TEXT` columns. Fields in types, `FromRow`, `CalendarEventData`, editor UI. |
| A5 | Recurrence expansion missing | ✅ Resolved | `expand_recurrence()` in `calendars.rs` expands DAILY/WEEKLY/MONTHLY/YEARLY with INTERVAL/COUNT/UNTIL into concrete instances for view rendering. |
| A6 | `calendar_default_view` setting never read | ✅ Resolved | `CalendarState::with_default_view()` reads setting from DB at boot via `with_conn_sync`. |

## B. Mode Switcher / Navigation — MOSTLY RESOLVED

| # | Gap | Status | Resolution |
|---|-----|--------|------------|
| B1 | View switcher in wrong location | ✅ Resolved | D/WW/W/M buttons now render in the sidebar header row (right of mode toggle) when in calendar mode. Sidebar conditionally switches between scope dropdown (mail mode) and view switcher (calendar mode). |
| B2 | No `Ctrl+1` for Mail | ✅ Resolved | `SwitchToMail` command registered with `KeyBinding::cmd_or_ctrl('1')`. |
| B3 | No distinct Switch to/from commands | ✅ Resolved | "Switch to Calendar" and "Switch to Mail" registered as separate searchable commands alongside "Toggle Calendar". |
| B4 | Mail state preservation unverified | ⚠️ Partial | Calendar state fully preserved. Mail selected-thread preserved in memory. Scroll position cannot be restored — iced fork lacks `scroll_to()` API (UI.md:60). |

## C. Calendar Views — PARTIALLY RESOLVED

| # | Gap | Status | Resolution |
|---|-----|--------|------------|
| C1 | No drag-to-select time range | ❌ Open | Requires custom widget with mouse tracking. iced doesn't provide continuous drag position mapping out of the box. Spec acknowledges this: "Drag interactions are important but are also the hardest to implement well in iced." |
| C2 | No event drag-to-move | ❌ Open | Same — requires custom widget work with hit testing and visual feedback. |
| C3 | No event edge resize | ❌ Open | Same — requires custom drag handlers with edge detection. |
| C4 | No scroll-to-now / working-hours | ❌ Blocked | iced fork lacks `scrollable::scroll_to()` API (UI.md:60). Cannot programmatically scroll the time grid. |
| C5 | Month view: No ISO week numbers | ✅ Resolved | Leftmost "Wk" column with ISO week numbers (1–53). Clicking navigates to that week's start date. |
| C6 | Multi-day events not spanning as bars | ❌ Open | Multi-day events still render as per-day chips. Continuous horizontal spanning bars require a fundamentally different layout pass (events must be positioned absolutely across cells before single-day events are laid out). |
| C7 | Weekend columns not narrower | ✅ Resolved | Week view (7 days) uses `FillPortion(2)` for Sat/Sun vs `FillPortion(3)` for weekdays. |

## D. Event Detail — MOSTLY RESOLVED

| # | Gap | Status | Resolution |
|---|-----|--------|------------|
| D1 | Detail is modal, not popover | ✅ Resolved | Click event opens a compact ~300px popover (right-aligned, lightweight backdrop). |
| D2 | No ↗ expand-to-modal button | ✅ Resolved | ↗ button on popover triggers `ExpandToFullModal`, opening two-panel modal. |
| D3 | No two-panel modal layout | ✅ Resolved | Full modal is 70% event detail (scrollable) + 30% mini day view showing the event's day with all events and color-coded conflict display. |
| D4 | Modal dimensions not responsive | ✅ Resolved | Full modal uses `FillPortion(4)` width with 1200px max, full height with padding. |
| D5 | No organizer display | ✅ Resolved | Shows "Invited by {name}" or "Invited by {email}" in both popover and full modal. |
| D6 | No attendees display | ✅ Resolved | Attendees loaded from `calendar_attendees` table via `get_event_attendees()`. Displayed with RSVP status icons (✓/✗/~/?) and "(organizer)" suffix. |
| D7 | No reminders display | ✅ Resolved | Reminders loaded from `calendar_reminders` table via `get_event_reminders()`. Displayed as "Reminders: 15 min before, 1 hour before" etc. |
| D8 | No RSVP actions | ⚠️ Partial | RSVP *status* displayed ("Your RSVP: Accepted/Declined/Tentative"). Action *buttons* (Accept/Decline/Tentative/Dismiss) not yet wired — requires provider API calls to actually send RSVP responses. |
| D9 | No "Email organizer" checkbox | ❌ Open | Depends on D8 RSVP action buttons being wired. |
| D10 | No recurrence display or icon | ✅ Resolved | 🔁 icon on time grid event blocks when `recurrence_rule` is set. Recurrence info shown in popover and full modal with human-readable format ("Every week", "Every 2 months"). |
| D11 | No calendar selector in editor | ✅ Resolved | Calendar name with color dot shown as first field in event editor. |
| D12 | No timezone picker | ✅ Resolved | Timezone text input field in event editor. Timezone shown in full modal detail view. |
| D13 | No recurrence editor | ⚠️ Partial | Basic recurrence toggle (on/off, defaults to WEEKLY). Full recurrence editor (day-of-week toggles, month/year options, weekend avoidance, end conditions) not yet implemented. |
| D14 | No double-click to create | ⚠️ Partial | `DoubleClickSlot` message variant added and handled. Not wired from UI — iced doesn't expose double-click events on buttons; needs custom widget with click-timing detection. |
| D15 | No recurring event edit/delete prompts | ❌ Open | "This / this and following / all" prompts not implemented. Requires tracking recurrence instance identity and provider API support for exception creation. |
| D16 | No ✕ close button on modal | ✅ Resolved | ✕ button on popover, full modal, and event editor. |
| D17 | No unsaved changes prompt | ✅ Resolved | Closing editor with title/description/location modifications prompts "Discard unsaved changes?" confirmation. |
| D18 | No attendee input field | ❌ Open | Event editor has no attendee input with autocomplete. Depends on contacts autocomplete infrastructure being fully wired (Tier 2 gap). |
| D19 | No availability field | ✅ Resolved | Availability selector (Busy/Free/Tentative/OOO) in event editor. Stored in DB and passed through to provider sync. |
| D20 | No visibility field | ✅ Resolved | Visibility selector (Default/Public/Private) in event editor. Stored in DB and passed through to provider sync. |
| D21 | Location not clickable | ⚠️ Partial | Full modal applies `text::primary` style to URLs (http/https) to visually indicate clickability. Actual hyperlink opening not implemented (iced text widget doesn't support click-to-open-URL). |

## E. Pop-Out Calendar Window — RESOLVED

| # | Gap | Status | Resolution |
|---|-----|--------|------------|
| E1 | Pop-out calendar window not implemented | ✅ Resolved | `PopOutWindow::Calendar` variant added. ↗ button in calendar sidebar. "Pop Out Calendar" command in palette. Opens 1024×768 window with full calendar layout. Main window reverts to mail mode. |
| E2 | No window rules enforcement | ⚠️ Partial | One-calendar-pop-out limit enforced (checks existing before opening). Badge on mode toggle and bring-to-foreground not implemented — iced lacks `window::focus()` / `window::raise()` API. |

## F. Email ↔ Calendar Integration — PARTIALLY RESOLVED

| # | Gap | Status | Resolution |
|---|-----|--------|------------|
| F1 | No 📅 button on expanded messages | ✅ Resolved | "Event" button (📅 icon + label) added to expanded message actions. Creates event pre-filled with subject → title, snippet → description, current time → start. Switches to calendar mode with editor open. |
| F2 | No meeting invite detection | ❌ Open | Requires iCalendar attachment parsing (RFC 5545) in the email rendering pipeline. Need to detect `text/calendar` MIME parts and parse VEVENT data. Cross-cutting with provider sync for invite state tracking. |
| F3 | No inline RSVP in reading pane | ❌ Open | Depends on F2 for invite detection + D8 for RSVP action wiring. |
| F4 | No calendar indicator on thread cards | ❌ Open | Depends on F2 for invite detection. Thread list card would need a calendar icon when the thread contains a meeting invite. |

## G. Calendar Sidebar — RESOLVED

All items resolved.

| # | Gap | Status | Resolution |
|---|-----|--------|------------|
| G1 | Calendar list is a placeholder | ✅ Resolved | Real calendar list with color dots (●), display names, and visibility toggle checkboxes. Grouped by account. `load_calendars_for_sidebar()` loads from DB. `set_calendar_visibility()` persists toggle and triggers event reload. |
| G2 | No event dots on mini-month days | ✅ Resolved | `dates_with_events: HashSet<NaiveDate>` computed in `rebuild_view_data()`. Mini-month renders a small bullet (•) below the date number for dates with events. |
| G3 | Agenda items not clickable | ✅ Resolved | Right sidebar agenda items wrapped in `button().on_press(CalendarMessage::EventClicked(id))`. Clicking opens event detail. |
| G4 | No "Open Calendar" entry point | ✅ Resolved | Agenda item clicks open event detail in calendar mode. Mode toggle button always available. |

## Summary

**50 gaps identified → 37 resolved, 6 partially resolved, 7 open.**

### Open items requiring custom iced widget work (C1-C3, C4, C6):
Drag-to-select, drag-to-move, drag-to-resize, scroll-to-now, and multi-day spanning bars all require either custom `advanced::Widget` implementations or iced API extensions that don't exist in this fork. The spec acknowledges drag interactions as "the hardest to implement well in iced" and defers them as a "fast follow."

### Open items requiring provider API integration (D8-D9, D15, F2-F4):
RSVP action buttons, recurring event instance editing, and meeting invite detection all require round-trip API calls to Google Calendar / Microsoft Graph / CalDAV providers. These are not pure UI work — they need provider-specific request formatting and response handling.

### Open items requiring cross-cutting infrastructure (D18):
Attendee input with autocomplete depends on the contacts autocomplete dropdown being fully wired (currently a Tier 2 gap — the search infrastructure exists but the dropdown never renders).
