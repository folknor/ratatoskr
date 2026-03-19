# Calendar: Problem Statement

## Overview

Ratatoskr needs a full calendar as a first-class view alongside email. Enterprise users live in their calendar as much as their inbox — an email client without calendar is not a serious tool for enterprise adoption. This is not a "nice to have" feature; it is a blocker.

The calendar exists at two layers:

1. **Glance layer** — the right sidebar mini-calendar (already spec'd in `docs/main-layout/problem-statement.md` § Right Sidebar). Shows a month grid and today's agenda while the user is in email mode. Always available, answers "what's coming up?" without leaving the inbox.

2. **Full calendar view** — replaces the entire mail UI (sidebar, thread list, reading pane) with a dedicated calendar experience. Day, work week, week, and month views. Full event creation and editing. This document covers this layer.

The two layers are connected: the mini-calendar sidebar is the entry point to the full calendar view (click "Open Calendar" or use a keyboard shortcut), and the full calendar can be popped out into a separate window for multi-monitor setups.

## Design Principles

1. **Outlook parity is the floor.** The four standard views (day, work week, week, month) are non-negotiable. Enterprise users expect them. Every interaction pattern Outlook provides — click to create, drag to resize, drag to move, overlapping event layout — must exist. Creative or non-traditional views are additions, not replacements.

2. **Mail and calendar are peer modes, not parent-child.** Switching between them is a top-level navigation action, not drilling into a submenu. State is preserved in both directions — switching to calendar and back should not lose your place in email.

3. **Speed over chrome.** A week view with 50 events must render instantly. No loading spinners, no progressive rendering. The calendar is a glance surface — if it's slow, users will keep a browser tab open to Google Calendar instead.

4. **The calendar is not a separate app.** It shares the window, the theme, the command palette, the keyboard shortcut system. Creating an event from an email (the 📅 button) should feel like a single action, not a context switch.

## Mode Switcher

### The Button

A single square icon button in the top-left of the sidebar header area. In mail mode it shows a calendar icon (📅). In calendar mode it shows a mail icon (✉). Click to toggle between modes. That's it — no tab bar, no mode labels, no extra chrome.

**Mail mode:** The button sits to the left of the existing account dropdown and compose button. It is a square whose height matches the combined height of the dropdown + gap + compose button (~76px). The dropdown and compose button are squeezed into the remaining sidebar width to its right.

```
┌────────────────────────┬──────────┬─────────────────────┐
│ [📅]  Account ▾        │          │                     │
│ [    ] [ Compose     ] │ Thread   │ Reading             │
│────────────────────────│ List     │ Pane                │
│ Inbox                  │          │                     │
│ Starred                │          │                     │
│ Sent                   │          │                     │
│ ...                    │          │                     │
└────────────────────────┴──────────┴─────────────────────┘
```

**Calendar mode:** The button shows a mail icon (✉) and occupies the same square dimensions in the same position. To its right, the space freed by the absent dropdown/compose is filled with the view switcher buttons and the pop-out button — all full height (~76px), arranged horizontally.

```
┌───────────────────────────────┬─────────────────────────┐
│ [✉] [ D ][WW][ W ][ M ] [↗] │                          │
│───────────────────────────────│  Week View              │
│ ◀ March 2026 ▶                │                          │
│ Mo Tu We Th Fr Sa Su          │                          │
│ ...                           │                          │
│───────────────────────────────│                          │
│ ☑ 🔵 Work                    │                          │
│ ☑ 🟢 Personal                │                          │
└───────────────────────────────┴─────────────────────────┘
```

The view switcher buttons (D, WW, W, M) share the remaining width after the mode button. The active view is highlighted. The Today button and pop-out button (↗) also need to live somewhere accessible — ideally in this header row if they fit, otherwise above the mini-cal or in the calendar main view header. **Resolve placement during implementation** when actual rendered sizes are known.

Keyboard shortcut (tentative): `Ctrl+1` for Mail, `Ctrl+2` for Calendar. Registered in the command palette system. The command palette also exposes "Switch to Calendar" / "Switch to Mail" as searchable commands.

### State Preservation

Switching modes does not destroy state. When the user switches from Mail to Calendar and back:

- The selected thread, scroll position, expanded messages — all preserved
- The calendar's selected date, active view (day/week/month), scroll position — all preserved
- This is not "navigating away" — it's flipping between two live views

Implementation: both the mail UI tree and the calendar UI tree exist in the app model simultaneously. The mode switcher controls which one renders in the content area. The inactive mode's state is simply not drawn but remains in memory.

## Calendar Layout

When in calendar mode, the window has two panels:

1. **Calendar sidebar** (left, same width as the mail sidebar — currently 180px, fixed width)
2. **Calendar main view** (fill remaining width)

No right sidebar in calendar mode. No thread list. No reading pane. The calendar owns the full content area.

### Calendar Sidebar

The calendar sidebar serves the same role as the mail sidebar — navigation and orientation. It contains three sections stacked vertically:

#### Mini Month Calendar

A small month grid for date navigation. Clicking a date navigates the main view to that date. The current date is highlighted. The selected date (or date range visible in the main view) is indicated.

```
◀  March 2026  ▶
Mo Tu We Th Fr Sa Su
                  1
 2  3  4  5  6  7  8
 9 10 11 12 13 14 15
16 17 18 19 20 21 22
23 24 25 26 27 28 29
30 31
```

Left/right arrows navigate months. Clicking the month/year header could open a month/year picker for faster navigation (e.g., jumping to a date six months out).

Days with events get a subtle dot indicator (same pattern as the right sidebar mini-cal spec).

#### View Switcher

Four buttons arranged horizontally in the sidebar header row, to the right of the mode toggle button. Full height (~76px), splitting the available width. Abbreviated labels: D (Day), WW (Work Week), W (Week), M (Month). The active view is highlighted.

See § Mode Switcher for the layout diagram. The pop-out button (↗) and Today button also belong in this area — exact placement TBD during implementation (see § Mode Switcher note).

**Keyboard shortcuts (tentative):** `D` for Day, `W` for Work Week, `K` for Week, `M` for Month (when calendar mode is active and no text input is focused). Shortcut assignments are not final — finalize during command palette keybinding work.

#### Calendar List

A list of all calendars the user has, with color indicators and visibility toggles. Each calendar is a row:

```
☑ 🔵 Work (alice@corp.com)
☑ 🟢 Personal (alice@gmail.com)
☐ 🟡 Team Calendar
☑ 🔴 Holidays
```

Toggling a calendar's checkbox shows/hides its events in the main view. Calendar colors are synced from the provider where available, with deterministic fallback (same approach as label colors — see `docs/main-layout/implementation-spec.md` Slice 1).

Calendars are grouped by account if multiple accounts are connected.

### Today Button

A "Today" button somewhere prominent (sidebar top or main view header) that jumps the main view back to the current date. Essential for recovering after navigating to a distant date.

## Event Block

The event block is the fundamental visual element across all calendar views. It renders identically everywhere — only its size changes depending on the view and time span.

- **Background color**: the calendar's color (synced from provider or deterministic fallback)
- **Top-left**: start time + title, e.g. "10:00 Standup". Full-day events show title only (no start time)
- **Top-right**: repeating/recurrence icon (🔁) if the event is part of a recurring series
- **Overflow**: text is not truncated with ellipsis. It simply clips at the block boundary — text flows under adjacent UI elements and disappears. No wrapping, no ellipsis
- **Click**: opens the event detail popover (see § Event Interaction)

The block is the same widget in day, week, work week, and month views. Day/week views position it on a time grid with height proportional to duration. Month view renders it as a single-line entry in a date cell.

## Main Calendar Views

All four views share common patterns:

- **Time grid**: vertical axis is time (typically 00:00–23:59, but the visible range defaults to working hours with scroll for the rest)
- **Event blocks**: positioned on the grid according to start time and duration
- **All-day events**: displayed in a separate bar above the time grid
- **Current time indicator**: a horizontal line (often red) showing "now" in today's column
- **Click empty slot**: selects that time slot
- **Drag on empty slots**: selects a time range
- **Click event**: opens event detail popover (see § Event Interaction)
- **Drag event**: moves it to a new time/day
- **Drag event edge**: resizes (changes start or end time)

### Day View

A single column showing one day's timeline.

```
┌─ Wednesday, March 19, 2026 ──────────────────────┐
│ All day  │ Company Holiday                        │
├──────────┤────────────────────────────────────────┤
│  8:00    │                                        │
│  8:30    │                                        │
│  9:00    │ ┌─ Standup ──────────────────────┐     │
│  9:30    │ │ 9:00 – 9:30 · Work calendar    │     │
│          │ └────────────────────────────────┘     │
│ 10:00    │ ┌─ Sprint Planning ──────────────┐     │
│ 10:30    │ │                                │     │
│ 11:00    │ │ 10:00 – 11:30 · Work calendar  │     │
│ 11:30    │ └────────────────────────────────┘     │
│ 12:00    │                                        │
```

The time labels are on the left. The events fill the column. Overlapping events split the column width (side by side), same as Outlook/Google Calendar.

The day view is the widest single-event view — event blocks have room for title, time, location, and calendar name. This is where detailed event cards make sense.

### Work Week View

Five columns (Monday–Friday), each a narrower version of the day view timeline.

```
┌─ Mar 16–20, 2026 ────────────────────────────────────────────┐
│          │  Mon    │  Tue    │  Wed    │  Thu    │  Fri      │
│ All day  │         │ Holiday │         │         │           │
├──────────┼─────────┼─────────┼─────────┼─────────┼───────────┤
│  9:00    │ Standup │ Standup │ Standup │ Standup │ Standup   │
│  9:30    │         │         │         │         │           │
│ 10:00    │         │ Sprint  │         │ 1:1     │           │
│ 10:30    │         │ Plan.   │         │         │           │
│ 11:00    │         │         │         │         │           │
```

Event blocks show less detail than day view (just title, maybe time). Color-coded by calendar.

This is the default view for enterprise users — it's what Outlook opens to.

### Week View

Seven columns (Monday–Sunday or Sunday–Saturday depending on locale). Identical to work week but with weekend columns. Often the weekend columns are narrower if weekends are typically empty (Outlook does this).

### Month View

A grid of date cells, typically 5–6 rows of 7 columns.

```
┌─ March 2026 ──────────────────────────────────────────────────┐
│  Mon    │  Tue    │  Wed    │  Thu    │  Fri    │ Sat  │ Sun  │
├─────────┼─────────┼─────────┼─────────┼─────────┼──────┼──────┤
│         │         │         │         │         │      │  1   │
│         │         │         │         │         │      │      │
├─────────┼─────────┼─────────┼─────────┼─────────┼──────┼──────┤
│  2      │  3      │  4      │  5      │  6      │  7   │  8   │
│ Standup │ Standup │ Standup │ Standup │ Standup │      │      │
│ Sprint  │ Design  │         │ 1:1     │ Demo    │      │      │
│ +2 more │         │         │         │         │      │      │
├─────────┼─────────┼─────────┼─────────┼─────────┼──────┼──────┤
```

Each cell shows as many event blocks as fit: `N = cell_height / (text_height + padding + margin)`. Cell height is dynamic — it depends on how many week rows the month has (5 vs 6 rows) and the window height. If there are N or fewer events, they all render. If there are more than N, the bottom row becomes a "+X more" button that opens a popover with the full day's events.

Month view does not have a time grid — events are listed vertically in each cell, ordered by start time. All-day events appear first.

A narrow leftmost column shows ISO week numbers (1–53). Clicking a week number switches to week view for that week.

Multi-day events span across cells as a horizontal bar (same as Google Calendar / Outlook). They are laid out first, pinned to the top of the cell row before any single-day events. This ensures spanning bars don't interleave with day-specific entries.

## Event Interaction

### Event Detail Popover

Clicking an event block opens a popover. Clicking outside or pressing Escape dismisses it. Clicking another event block dismisses the current popover and opens a new one for the clicked event.

#### Popover Anchoring

- **Horizontal**: anchors to the right of the event block if space allows, left if not.
- **Vertical**: the popover's vertical center aligns with the event block's vertical center.

#### Popover Size

- **Fixed width**: approximately the sidebar width (~180px).
- **Dynamic height**: grows to fit content. Elements with no data are hidden entirely, so a minimal event (title + time only) produces a compact popover.

#### Popover Contents (top to bottom, all full-width)

```
┌─────────────────────────┐
│ Sprint Planning       ↗ │  ← title + expand-to-modal button (top-right)
│ 10:00–11:30  🔁         │  ← abbreviated time/span + recurrence icon (right-pinned)
│ Room 4B                 │  ← location (hidden if empty)
│ Invited by Alice Smith  │  ← organizer (hidden if own event)
│ Bob, Charlie, +3 others │  ← other attendees (hidden if none)
│ Discuss Q2 roadmap...   │  ← description (truncated ~200 words, hidden if empty)
│                         │
│ ☐ Email organizer       │  ← checkbox (only shown when RSVP actions are present)
│ [Accept] [Decline] [?]  │  ← action buttons (context-dependent, see below)
└─────────────────────────┘
```

Every content row is hidden if its data is empty — no blank rows, no placeholders.

#### Popover Action Buttons (context-dependent)

The action buttons at the bottom of the popover change based on the user's relationship to the event:

**(a) Event from someone else's calendar (shared/public calendar):**
- "Add to my calendar", "Edit" (if permissions allow), "Cancel" (if permissions allow)

**(b) User's own event (no invitation involved):**
- No action buttons. The popover is read-only; editing happens in the modal via ↗.

**(c) User's own event where they have already responded to an invitation:**
- Shows current RSVP status (e.g., "Accepted" / "Tentative" / "Declined") and a "Change" button that reveals the full RSVP options.

**(d) User's own event where they have NOT responded to an invitation:**
- "Accept", "Decline", "Tentative", "Dismiss"

When RSVP action buttons are present (cases a, c, d), an **"Email organizer" checkbox** appears directly above the action buttons. When checked, the RSVP response is also sent as an email to the organizer.

#### Expand to Modal

The **↗ button** in the popover's top-right corner opens the full event detail modal (see below). The popover closes when the modal opens.

### Event Detail Modal

The modal dims and blocks interaction with the rest of the calendar window. It shows the complete event detail with all fields, and is also the surface for event editing and event creation.

#### Modal Size and Layout

- **Fixed width**: 1200px
- **Height**: full window height minus ~30px margin on each side
- **Two-panel layout**: ~850px left panel (event details), ~350px right panel (day view)

```
┌────────────────────────────────────────────────────┬──────────────────┐
│ Event Details (left panel, ~850px)                  │ Day View (~350px)│
│                                                    │                  │
│ Calendar: [Work Calendar ▾]                        │  8:00            │
│                                                    │  9:00 ┌────────┐│
│ Title                                              │       │Standup ││
│ ─────────────────────────────────────────          │  9:30 └────────┘│
│ Date: Wed, Mar 19, 2026                            │ 10:00 ┌────────┐│
│ Time: 10:00 – 11:30           🔁 Weekly            │       │THIS    ││
│ Location: Room 4B                                  │       │EVENT   ││
│                                                    │ 11:30 └────────┘│
│ Organizer: Alice Smith                             │ 12:00            │
│ Attendees:                                         │ 13:00            │
│   ✓ Alice Smith (organizer)                        │ 14:00 ┌────────┐│
│   ✓ Bob Jones                                      │       │Client  ││
│   ? Charlie (no response)                          │       │call    ││
│   ✗ Diana (declined)                               │ 15:00 └────────┘│
│                                                    │ 16:00            │
│ Description:                                       │                  │
│ Discuss Q2 roadmap priorities and resource          │                  │
│ allocation for the new platform migration...       │                  │
│                                                    │                  │
│ Reminders: 15 min before                           │                  │
│                                                    │                  │
│ ☐ Email organizer                                  │                  │
│ [Accept] [Decline] [Tentative] [Dismiss]           │                  │
└────────────────────────────────────────────────────┴──────────────────┘
```

**Left panel**: scrolls vertically if content overflows. Contains all event fields and action buttons at the bottom.

**Right panel**: a mini day view showing the event's date, with the current event highlighted. Shows other events on the same day so the user can see scheduling conflicts. Uses the same event block rendering as the main day view, just narrower.

#### Left Panel Contents (read mode, top to bottom)

- Calendar selector (name + color, dropdown in edit mode)
- Title
- Date and time (with timezone if different from local)
- Recurrence rule (if recurring)
- Location (with clickable link if URL / meeting link)
- Organizer
- Attendees with per-person RSVP status (✓ accepted, ? no response, ~ tentative, ✗ declined)
- Description (full, not truncated)
- Reminders
- "Email organizer" checkbox (when RSVP actions are present)
- Context-dependent action buttons (same a/b/c/d rules as the popover), plus "Edit" and "Delete"

Empty fields are hidden in read mode.

#### Edit Mode

Clicking "Edit" switches the left panel to edit mode — fields become editable in place. The right panel day view stays visible (useful for checking conflicts while adjusting times). Save and Cancel buttons replace the action buttons at the bottom. The event creation dialog (see § Event Creation) uses this same modal in edit mode.

#### Close

A close button (✕) in the top-right corner of the modal. Escape also closes. In edit mode, closing without saving prompts for confirmation if changes have been made.

This two-tier approach keeps quick checks fast (popover) while giving complex events the space they need (modal).

### Time Picker Popover

The date/time field in the modal (both read and edit mode) is **not a standard text input**. It displays the formatted time range as a clickable label. Clicking it opens a time picker popover. This popover is used in both the event detail modal (edit mode) and the event creation dialog.

#### Base Layout (simple, no timezone, non-recurring)

```
┌───────────────────────────────────────┐
│ Start date     Start time             │
│ [Mar 19, 2026] [10:00]               │
│                                       │
│ End time (default: +30min from start) │
│ [10:30]                               │
│                                       │
│ [🌐 Timezone]                         │
│                                       │
│ ☐ All day                             │
│ ○ Recurring                           │
└───────────────────────────────────────┘
```

- **Start date**: date picker
- **Start time**: time picker
- **End time**: time picker, defaults to 30 minutes after start time
- **Timezone button**: expands the fields (see below)
- **All day checkbox**: when checked, hides time fields (start time, end time), keeps date fields
- **Recurring toggle**: when enabled, expands recurrence options (see below)

The end date is not shown by default — it is the same as the start date. The timezone expansion reveals it.

#### Timezone Expansion

Clicking the timezone button expands the 3 fields (start date, start time, end time) to 6 fields across two rows:

```
┌───────────────────────────────────────┐
│ [Mar 19, 2026] [10:00] [Europe/Oslo] │
│ [Mar 19, 2026] [10:30] [US/Eastern]  │
│                                       │
│ [🌐 Timezone]  (collapse)             │
│                                       │
│ ☐ All day                             │
│ ○ Recurring                           │
└───────────────────────────────────────┘
```

Row 1: start date + start time + start timezone. Row 2: end date + end time + end timezone. This enables events that span timezones (e.g., a flight departing Oslo at 10:00 CET arriving New York at 10:30 EST) and multi-day events (end date differs from start date).

#### Recurrence Options

Toggling "Recurring" on expands the popover with recurrence configuration:

```
┌───────────────────────────────────────┐
│ [Start date] [Start time]            │
│ [End time]                            │
│                                       │
│ ☐ All day                             │
│ ● Recurring                           │
│ ┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄ │
│ Repeat every [1] [Week       ▾]      │
│                                       │
│ [Mo][Tu][We][Th][Fr][Sa][Su]          │  ← day-of-week toggles (day/week only)
│                                       │
│ ☐ Never on weekends or holidays       │
│   (shifts to closest work day)        │
│                                       │
│ Ends: [Never ▾]                       │
│       [On date: ___________]          │
│       [After N occurrences: ___]      │
└───────────────────────────────────────┘
```

**Repeat interval**: a number input + dropdown (Day / Week / Month / Year).

**Day-of-week toggles**: 7 buttons (Mon–Sun), shown when the dropdown is Day or Week. Multiple can be active (e.g., every week on Mon, Wed, Fri). Not shown for Month or Year.

**Month recurrence**: when the dropdown is Month, two radio options appear instead of day-of-week toggles:

```
│ ○ On day 19                           │  ← the date number from the start date
│ ○ On the third Wednesday              │  ← computed from start date's position in the month
```

The second option is computed: if the start date is the third Wednesday of the month, it says "On the third Wednesday." If it's the first Monday, "On the first Monday," etc.

**Year recurrence**: same two radio options as Month, but each includes the month name:

```
│ ○ On March 19                         │
│ ○ On the third Wednesday of March     │
```

**Weekend/holiday avoidance**: a checkbox "Never on weekends or holidays." When checked, if a recurring instance would land on a weekend or holiday, it is automatically moved to the closest work day (pulled earlier or pushed later, whichever is closer). This applies to all recurrence types.

**End condition**: dropdown with three options:
- **Never** — recurs indefinitely
- **On date** — recurs until a specific end date (date picker)
- **After N occurrences** — recurs N times then stops (number input)

### Event Creation

**Time selection:** Clicking an empty time slot selects that slot. Dragging across empty slots selects a time range. The selection is visually highlighted but does not open anything.

**Creating an event — two paths:**

1. **Double-click an empty slot**: opens the event creation dialog with that slot's time pre-filled. This only works for single-slot creation — double-clicking after a drag-selection does not preserve the drag range (the first click of the double-click resets the selection to a single slot).
2. **Keyboard shortcut or command palette** ("New Event"): opens the event creation dialog with the current selection pre-filled. This is the only way to create an event from a drag-selected time range. If no selection exists, defaults to the next available slot (or the current time).

**The event creation dialog** is the same full modal as the event detail expanded view (dimmed background, blocks interaction), opened in edit mode. It contains all event fields:

- Calendar selector (which calendar to create in) — **top of the form**, first field
- Title
- Date/time (clickable label that opens the Time Picker Popover — see § Time Picker Popover. Handles date, time, timezone, all-day, and recurrence)
- Location
- Description (rich text or plain text — start with plain)
- Attendees (email address input with autocomplete from contacts)
- Reminders (notification timing)
- Availability (free / busy / tentative / out of office)
- Visibility (public / private)

This is a dense form. It uses the same full modal as the event detail expanded view (dimmed background, blocks interaction).

### Event Editing

Clicking "Edit" on an event detail opens the full event editor pre-filled with the event's data.

For **recurring events**, editing prompts: "Edit this event / Edit this and following events / Edit all events in the series." This is standard calendar UX and non-negotiable for enterprise use.

### Event Deletion

Delete prompts for confirmation. For recurring events: "Delete this event / Delete this and following events / Delete all events in the series."

### Drag Interactions

- **Drag event to new time** (same day): updates start/end time, preserving duration
- **Drag event to new day** (week/work week view): moves to the same time on the target day
- **Drag top/bottom edge**: changes start/end time (resize)
- **Drag in month view**: moves to a different date

All drag operations on recurring events should prompt the same "this / this and following / all" choice.

Drag interactions are important but are also the hardest to implement well in iced. They require custom widget work with hit testing and visual feedback. This should not block the initial calendar release — keyboard/click-based creation and editing is sufficient for V1. Drag support is a fast follow.

### RSVP

For events the user is invited to (not the organizer), RSVP actions appear in the event detail:

- **Accept** — marks attendance as accepted, sends response to organizer
- **Tentative** — marks as tentative
- **Decline** — marks as declined, optionally hides event from view
- **Propose New Time** — opens a time picker, sends counter-proposal (provider-dependent)

RSVP is especially important because meeting invites arrive as email. The 📅 button on a meeting invitation email should open the event in the calendar with RSVP actions ready.

## Pop-Out Window

The full calendar view can be popped out into a separate window. Discoverable two ways:

1. **A ↗ button in the calendar UI** (exact placement TBD — header row, mini-cal area, or calendar main view header, depending on what fits). Visible whenever the user is in calendar mode.
2. **Command palette**: "Pop Out Calendar" as a searchable command. Works from either mode.

The pop-out window contains the complete calendar UI — sidebar, main view, event detail panel. It is fully functional and independent of the main window.

### Window Rules

The app has strict window limits:

- **One main window** — always exists. Can be in mail or calendar mode.
- **One pop-out calendar window** — optional. When open, the main window stays in mail mode.
- **Multiple pop-out message windows** — allowed (already spec'd in `docs/main-layout/problem-statement.md` for double-clicking a message card). These are read-only detail views, not full app instances.

No duplicate mail windows, no duplicate calendar windows. The app is not multi-instance — one process, one set of database connections, one window per function.

Use case: multi-monitor users keep the calendar on one screen and email on the other. This is the #1 power-user workflow in enterprise Outlook.

When the calendar is popped out:
- The main window reverts to mail mode
- The mode toggle button in the sidebar shows a visual indicator that the calendar is open in another window (e.g., a small "↗" badge on the calendar icon)
- Clicking the mode toggle button in the main window brings the pop-out window to the foreground (rather than opening a duplicate)
- Closing the pop-out window returns the calendar to being available via the mode toggle button

## Email ↔ Calendar Integration

### Create Event from Email (📅 button)

Already spec'd in `docs/main-layout/problem-statement.md` § Calendar Event Creation. The 📅 button on expanded messages opens the event creation form pre-filled with:

- **Title**: email subject
- **Description**: link to the email thread (or pasted snippet)
- **Attendees**: email participants (from/to/cc)
- **Date/time**: extracted from email body if possible (NLP date extraction), otherwise defaults to "next available slot"

If the user is in mail mode when they click 📅, the behavior should be: open the event creation panel inline (not switch to calendar mode). The event is created without leaving email context.

### Meeting Invites in Email

When a meeting invite (iCalendar attachment, or provider-native invite) arrives as an email:

- The thread list card shows a calendar indicator (📅) alongside or instead of the normal snippet
- The reading pane shows the event details inline (time, location, attendees) with RSVP buttons directly in the email view
- RSVP actions in the email view are equivalent to RSVP in the calendar — they update the event and send the response

This means meeting invites are actionable without ever opening the calendar. The calendar shows the same event from the calendar perspective; the email shows it from the email perspective.

### Agenda in Right Sidebar

The right sidebar mini-calendar (spec'd in main-layout) shows today's upcoming events. These are the same events visible in the full calendar — same data source, same colors, same calendar visibility toggles. Clicking an event in the mini-sidebar could either:

- Switch to calendar mode and select that event
- Open a minimal popover with event details

The mini-sidebar also serves as the "Open Calendar" entry point — a button or clickable area that switches to full calendar mode.

## What We Must Ship (Outlook Parity)

The minimum viable calendar for enterprise adoption:

1. **Four views**: Day, Work Week, Week, Month
2. **Event CRUD**: Create, read, edit, delete — including recurring events
3. **RSVP**: Accept, tentative, decline meeting invitations
4. **Multiple calendars**: Toggle visibility, color-coded
5. **Multi-account**: Calendars from all connected accounts in one view
6. **Drag interactions**: Move and resize events (V1 or fast follow)
7. **Meeting invite handling**: RSVP from email, event displayed in both email and calendar
8. **Shared calendar editing**: Full CRUD on calendars shared with the user (team calendars, delegate access). Permission enforcement is the provider's responsibility — if the API accepts the operation, we allow it; if it rejects, we show the error.
9. **Pop-out window**: For multi-monitor workflows

## What We Don't Ship in V1

- Room/resource booking (requires Exchange room lists or Google resource calendars — complex provider-specific work)
- Natural language event creation ("Coffee with Bob tomorrow at 3" → parsed into an event)
- Availability/free-busy lookup for external attendees
- Calendar sharing/publishing (CalDAV publish, ICS export)
- Integration with video conferencing (auto-adding Zoom/Teams links)
- Non-traditional views (timeline, schedule, kanban-style) — these are the interesting future work

## Open Questions

1. ~~**Week start day**~~ **Resolved.** User setting, default Monday (ISO 8601). Users in locales that prefer Sunday can change it.

2. ~~**Working hours**~~ **Resolved.** Not relevant for V1. The time grid shows 00:00–23:59 and scrolls. No need to define or dim "working hours."

3. ~~**Default view**~~ **Resolved.** Month. User setting to change it.

4. ~~**Event detail panel vs popover**~~ **Resolved.** Popover for quick glance, ↗ expands to full modal (see § Event Detail Panel).

5. ~~**Timezone handling**~~ **Resolved.** Display in local time everywhere. If the event was created in a different timezone, show the original timezone as secondary info in the popover and detail modal. Store in UTC internally.

6. ~~**Offline event creation**~~ **Resolved.** There is no offline mode. Calendar operations require connectivity.

7. ~~**Calendar data sync frequency**~~ **Resolved.** Same approach as email sync — use each provider's push/delta mechanism (Google Calendar push notifications, Graph subscriptions/delta tokens, JMAP state changes, CalDAV sync tokens). Not periodic polling.

## Dependencies

- **Provider calendar APIs**: Google Calendar API, Microsoft Graph Calendar API, CalDAV. Each needs a sync implementation in its respective provider crate (`crates/gmail/`, `crates/graph/`, `crates/jmap/`, `crates/imap/`). CalDAV might warrant its own crate.
- **Calendar data model**: New SQLite tables for calendars, events, attendees, recurrence rules, reminders. Significant schema work in `crates/core/`.
- **iCalendar parsing**: RFC 5545 parsing for CalDAV sync and meeting invite handling. Crate: `ical` or `icalendar`.
- **Recurrence expansion**: RFC 5545 RRULE expansion (turning "every Tuesday" into concrete event instances). Crate: `rrule`.
- **Date/time infrastructure**: Heavy use of `chrono` and `chrono-tz` for timezone-aware event handling.
- **iced custom widgets**: The week view time grid with positioned event blocks, drag handles, and overlap layout is a custom widget — it does not map to iced's built-in layout primitives.
- **Command palette integration**: Calendar actions (create event, navigate to date, switch view) need to be registered in the command palette alongside email actions.
- **Pop-out window**: iced multi-window support (`iced::window::open`). Already used for the message pop-out spec in main-layout.
