# Main Layout: Problem Statement

## Overview

Ratatoskr's main window is where users spend 90% of their time. The layout must serve enterprise power users processing 200+ emails/day across 3+ accounts — people who are stuck on Outlook because nothing else handles their volume. The design takes cues from Superhuman's speed and focus, but adds the depth (folder trees, multi-account, bulk operations) that enterprise users require.

This document covers the main window's structure, the thread list, the conversation/reading pane, and the interaction model that ties them together. It does not cover the sidebar (see `docs/sidebar/problem-statement.md`) or the command palette (see `docs/cmdk/problem-statement.md`), though both are integral to the experience.

## Design Principles

1. **The palette is the primary interface.** Visible chrome exists for reading and orientation, not for triggering actions. Every button, menu, and toolbar is a convenience shortcut to something the palette already does.

2. **Keyboard-first, mouse-compatible.** The fastest path through email is keyboard-driven (Superhuman proved this). But the UI must not punish mouse users — hover states, click targets, and contextual actions must all work without a keyboard.

3. **No empty chrome.** If a UI element has no function in the current context, it shouldn't be visible. No disabled buttons, no greyed-out toolbars, no placeholder panels. The window shows what's relevant now.

4. **Density is a feature.** Enterprise users process volume. The layout should show more threads per screen than consumer email clients, without feeling cramped. One density, designed right. *(Remove the "Email Density" select from the settings UI — `settings.rs` still has a `density` field and Compact/Default/Spacious dropdown that isn't wired to anything.)*

## Current State

The prototype renders three columns with drag-resizable dividers:

```
[ Sidebar 180px | Thread List 280px | Reading Pane (fill) ]
```

- Sidebar: scope selector, compose button, settings, universal folders, labels. Functional.
- Thread list: scrollable list of thread cards with avatar, sender, subject, snippet, date, unread indicator. Data from seeded DB. Functional but visually rough.
- Reading pane: shows the selected thread's snippet and basic metadata. Placeholder — no real message rendering, no conversation view.
- No toolbar or top bar. Search is above the thread list.

The layout decisions are directionally sound, but the width budget is not finalized — see "Width Budget" below.

## Layout Structure

### No Top Bar

There is no application-level toolbar spanning the window. Every element that would traditionally live in a toolbar already has a home:

| Element | Location |
|---------|----------|
| Search | Above the thread list |
| Account selector | Sidebar (scope dropdown) |
| Compose | Sidebar button |
| Settings | Sidebar button |
| Email actions (archive, trash, star, etc.) | Command palette + keyboard shortcuts |
| Contextual actions (reply, forward, etc.) | Reading pane action bar (appears when a thread is selected) |

A top bar would be empty chrome. The space is better used for content.

### Panel Proportions

The sidebar stays at 180px fixed. The thread list and reading pane split the remaining space, with the thread list taking roughly 40% — closer to equal partners than the traditional narrow-list/wide-detail split. On a 4K display at 1.5x scale:

```
[ 180px | ~400px thread list | ~700px reading pane ]
```

The thread list default width increases from the current 280px to ~400px. This gives thread cards room for sender, subject, and snippet without aggressive truncation. The dividers remain draggable.

#### Width Budget

**Resolved.** The right sidebar replaces the contact sidebar — same panel slot, same default width (240px). The thread list width does not change when the right sidebar is toggled; the reading pane absorbs the squeeze.

| Panel | Default width | Min width |
|-------|--------------|-----------|
| Sidebar | 180px (unchanged) | 200px (existing `SIDEBAR_MIN_WIDTH`) |
| Thread list | 400px (was 280px) | 250px (existing `THREAD_LIST_MIN_WIDTH`) |
| Reading pane | Fill | — |
| Right sidebar | 240px (was `CONTACT_SIDEBAR_WIDTH`) | 240px (fixed, not resizable) |

Common case (right sidebar off):

```
180 + 400 = 580px → 700px reading pane at 1280px
```

With right sidebar on:

```
180 + 400 + 240 = 820px → 460px reading pane at 1280px
```

`layout.rs` changes: `THREAD_LIST_WIDTH` → 400, rename `CONTACT_SIDEBAR_WIDTH` to `RIGHT_SIDEBAR_WIDTH` (value stays 240). The contact sidebar pane in the PaneGrid is repurposed for the right sidebar.

### Resizable Dividers

Already implemented. Users can drag the sidebar and thread list dividers. Minimum widths prevent collapsing panels to unusable sizes. Panel widths are not persisted yet — they should be, alongside window geometry.

## Thread List

### Purpose

The thread list answers: "what do I need to deal with?" It's a triage surface — the user scans, decides, and acts. Speed of scanning is the primary metric.

### Thread Card Content

Each card is a fixed-height three-line layout. All text lines start from the same left offset — no avatar column or leading icons that push text inward.

**Line 1: Sender + Date**
- **Sender name** (left) — most important triage signal. Bold/semibold if unread, normal weight if read. Shows the most recent sender for multi-message threads.
- **Date/time** (right) — relative format: time today ("3:42 PM"), day this week ("Tue"), date this year ("Mar 12"), year for older ("Dec 2024").

**Line 2: Subject**
- **Subject line** — truncated with ellipsis. Accent/primary color if unread, muted if read. This is a stronger unread signal than bold alone.

**Line 3: Snippet + Indicators**
- **Snippet** (left) — preview of the most recent message body. Muted/tertiary text color. Truncated to make room for indicators.
- **Indicators** (right, inline) — small colored label dots + attachment icon (📎). Indicators are right-aligned; snippet truncates earlier when indicators are present. If no labels and no attachment, the snippet gets the full width.

### Thread Card States

**Unread:** sender in semibold, subject in accent/primary color.

**Read:** sender in normal weight, subject and snippet in muted colors.

**Starred:** the entire card has a warm/golden background color. There is no star icon — the background *is* the indicator. The star action lives in the reading pane and command palette, not the thread card.

### Thread Card Indicators

**Label colors:** small colored dots (6-8px circles) in the lower right of line 3. Each label the thread has produces one dot in that label's color. No label text — the color is the identifier. Users learn the color-to-label mapping from the reading pane; the thread list is for pattern matching. Most threads have 0-2 labels; the dots are compact enough to handle 3-4 without crowding.

**Label color availability:** Gmail syncs real `color_bg`/`color_fg` hex values from the API (see `gmail/sync/labels.rs`). All other providers — IMAP, JMAP, and Graph (Exchange) — store `color_bg: None, color_fg: None`. No deterministic hash-based color assignment exists in the codebase. **Requires new work:** a fallback color assignment strategy for non-Gmail labels (e.g., deterministic hash of label name → color from a fixed palette). Without this, label dots in the thread list will only work for Gmail accounts.

**Attachment icon:** a small paperclip (📎) in the lower right of line 3, after any label dots. Only shown if the thread has attachments.

**No hover effects on indicators.** No tooltips. The thread card is a pure scan surface.

### What Thread Cards Do NOT Show

- **Avatars** — avatars take horizontal space from text and slow scanning. Sender name typography is sufficient for identification. Avatars appear in the reading pane where there's room.
- **Message count** — thread depth is not a reliable triage signal. Available in the reading pane.
- **Label text** — label colors are shown as dots; full names are in the reading pane.
- **Account indicator** — the scope selector and search scoping handle account context.
- **AI summary** — not ruled out for the future (would replace the snippet on line 3), but not in V1.

### Thread Card Layout

```
┌──────────────────────────────────────────────────┐
│ Sender Name                            3:42 PM  │
│ Subject line truncated with ellipsis...          │
│ Snippet preview text in muted...    🔵 🟢 📎    │
└──────────────────────────────────────────────────┘
```

Starred variant (golden background):
```
┌══════════════════════════════════════════════════┐
║ Sender Name                            3:42 PM  ║
║ Subject line truncated with ellipsis...          ║
║ Snippet preview text in muted...    🔵 📎       ║
└══════════════════════════════════════════════════┘
```

### Selection and Navigation

- **Single selection**: clicking a thread selects it and shows it in the reading pane. Arrow keys (or j/k) move selection.
- **Multi-select**: Shift+click for range, Ctrl/Cmd+click for toggle. Multi-select enables bulk actions via palette ("Archive 5 conversations").
- **Active vs selected**: the active thread (highlighted row) determines what the reading pane shows. In multi-select, the last-selected thread is active.

### Search Integration

Search lives directly above the thread list, not in a toolbar or dialog. It's a text input that filters the thread list in place. When focused, the thread list shows search results instead of the current folder's threads. When cleared, the thread list reverts.

This is the same pattern as Superhuman — search is spatial ("I'm filtering this list") not modal ("I opened a search dialog"). The command palette's search is for commands; the thread list's search is for email.

#### Search Bar Area

The search bar area is two lines: the input field and a context line below it. **Both lines are always visible** — the area has fixed height whether idle or searching. No layout jump when entering/leaving search.

```
Idle:
┌─────────────────────────────────────┐
│ 🔍 Search...                       │
│ Inbox                     Foo Corp │
├─────────────────────────────────────┤
│ [thread cards...]                   │

Searching:
┌─────────────────────────────────────┐
│ 🔍 from:alice meeting              │
│ 47 results                    All ↗│
├─────────────────────────────────────┤
│ [search results...]                 │

Idle, all accounts scope:
┌─────────────────────────────────────┐
│ 🔍 Search...                       │
│ Inbox                          All │
├─────────────────────────────────────┤
│ [thread cards...]                   │
```

**Context line (idle):** current folder/view name on the left, account scope on the right. This is the persistent "what am I looking at" indicator — the sidebar selection tells you too, but the context line keeps it visible without eye travel.

**Context line (searching):** result count on the left, scope on the right with a clickable "All ↗" to widen to all accounts if currently scoped to one.

**Typeahead popups** (for `from:`, `to:`, `account:`, `label:`, `folder:`, `before:`/`after:` — see `docs/search/problem-statement.md`) overlay the thread list below. They don't push the list down.

**Smart folder display:** when a smart folder is selected in the sidebar, the search bar shows the smart folder's query string (editable). The context line shows the smart folder name on the left. Modifying the query updates results live; saving is explicit via palette.

## Conversation / Reading Pane

### Purpose

The reading pane answers: "what does this conversation say, and what should I do about it?" It shows the full thread as a stack of messages, with actions available in context.

### Thread Attachments

Attachments are first-class citizens in the reading pane — they're the reason half of enterprise email exists. Attachments are displayed as a consolidated, deduplicated list at the top of the conversation, above all message cards.

#### Attachment Cards

Attachments are individual multi-line cards inside a group container:

```
┌─ Attachments (3) ──────────────────── [Save All ↓] ┐
│                                                     │
│  ┌─────────────────────────────────────────────┐    │
│  │ 📄 Q2 Report.pdf                            │    │
│  │ PDF · 2.4 MB · Mar 14 from Alice            │    │
│  └─────────────────────────────────────────────┘    │
│                                                     │
│  ┌─────────────────────────────────────────────┐    │
│  │ 📊 Budget.xlsx              ▸ 2 versions    │    │
│  │ Excel · 847 KB · Mar 14 from Alice          │    │
│  └─────────────────────────────────────────────┘    │
│                                                     │
│  ┌─────────────────────────────────────────────┐    │
│  │ 🖼 Site Photo.jpg                            │    │
│  │ Image · 1.1 MB · Mar 12 from Bob            │    │
│  └─────────────────────────────────────────────┘    │
│                                                     │
└─────────────────────────────────────────────────────┘
```

Each card shows:
- **Line 1:** file type icon + filename (+ "▸ N versions" if deduplicated)
- **Line 2:** type label + file size + date + sender name

#### Attachment Dates

Each attachment card shows when the attachment arrived, alongside the sender ("Mar 14 from Alice").

**V1:** The date is the parent message's `date` field, which is always available. `AttachmentWithContext` already joins this from the messages table.

**Future enhancement:** Richer dates from `Content-Disposition` headers (`modification-date` parameter, RFC 2183) or extractable file metadata (EXIF, PDF creation date, Office document properties). This requires: (1) parsing and storing Content-Disposition parameters during sync (currently not extracted), (2) a `modification_date` column on the `attachments` table, (3) optional metadata extraction for images/PDFs/Office documents at index time. None of this exists in the current data model (`DbAttachment` stores filename, MIME type, size, IDs, cache path, and content hash only). Not a V1 blocker — message date is sufficient for versioning.

The date is especially important in the versioning view, where dates distinguish versions:

```
  ┌─────────────────────────────────────────────┐
  │ 📊 Budget.xlsx              ▾ 2 versions    │
  │ Excel · 847 KB · Mar 14 from Alice          │
  │ ┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄  │
  │   847 KB · Mar 14 from Alice (latest)       │
  │   692 KB · Mar 8 from Bob                   │
  └─────────────────────────────────────────────┘
```

#### Collapsible

The attachment group is collapsible. Collapsed state is persisted per thread ID — if you collapse the attachments because you've already saved them, they stay collapsed next time you open that thread. Stored as a lightweight key-value map (thread ID → bool), negligible disk usage even after years.

The group is expanded by default for threads with attachments.

#### Deduplication and Versioning

In a 30-message thread where `Budget.xlsx` is attached in message 3 and a revised `Budget.xlsx` in message 8, the attachment list shows **one entry** — the latest version (from message 8). A small "2 versions" indicator on the card expands to show all versions with their message dates.

Version detection uses strict same-filename matching. `Budget.xlsx` and `Budget.xlsx` = same document, latest wins. `Budget_v2.xlsx` and `Budget.xlsx` = treated as separate documents (fuzzy matching is too error-prone for enterprise documents where naming conventions vary).

The attachment list is collapsed by default to show only the latest version of each unique filename. Expanding "all versions" reveals the full history.

#### Image Hover Preview

Hovering over an image attachment card shows a fixed-size preview in a fixed position — either to the left of the attachment list or below it, overlaying the reading pane. Not a lightbox, not a cursor-following tooltip. Fixed position, fixed size, appears on hover, disappears on mouse-out.

This may extend to PDF previews (rendered first page as image) in the future. Office documents don't get in-app preview — they open externally.

#### Save/Open Behavior

- **Single click** — selects the card (visual highlight), does not open
- **Double click** — opens the attachment with the system's default handler
- **Open button** (per-card) — same as double click, opens with default handler
- **Save button** (per-card) — save to disk with file picker
- **Save All** (on the group container header) — saves all *currently visible* attachments. If the list is showing deduplicated latest-versions-only, Save All saves those. If the user has expanded "all versions" first, Save All includes everything. The button does what the current view shows — no surprises.

### Conversation View

A thread is displayed as a vertical stack of message cards below the attachment list, newest at top. This is a universal rule — newest first everywhere (thread list, conversation view).

**Pop-out window:** Double-clicking an individual message card in the conversation view opens that message in a separate window (Outlook/Thunderbird behavior). The pop-out shows the single message with its full content, headers, and attachments. Essential for multi-monitor workflows — reference one message while composing a reply to another, or keep a message with instructions visible while working.

Each message card shows:

1. **Sender** — name + email, with avatar
2. **Recipients** — "to me", "to me, 3 others", expandable to full list
3. **Date/time** — absolute format ("Mar 12, 2026 at 2:34 PM"), possibly with a relative offset from the initial message ("+14d"). Two options under consideration — decide during implementation when we can see it on screen:
   - **Option A:** Show relative offset by default ("+14d"), absolute date on hover. Denser, shows conversation rhythm at a glance.
   - **Option B:** Show absolute date by default, relative offset as a secondary indicator. More conventional, no hover needed.
   - This warrants a user setting. Prototype both behind a setting flag, default to Option A (denser). The setting can live in the existing settings infrastructure.
4. **Body** — rendered message content

### Message Collapsing

Enterprise threads routinely hit 30+ messages. A thread about a contract negotiation might have 40 replies spanning three weeks, with half of them being one-line acknowledgements ("Thanks", "Sounds good", "+1") and the rest being substantive. Showing all 40 fully expanded is unusable — the user scrolls past walls of quoted text and signatures hunting for the two messages that matter. Collapsing everything except what's relevant is not a nice-to-have, it's a core requirement.

#### Collapse Rules

When a thread is opened, each message is either expanded (full content visible) or collapsed (one-line summary) based on these rules, evaluated in priority order:

1. **Unread messages** — always expanded. These are the reason the user opened the thread. If there are three unread messages in a 20-message thread, those three are expanded and everything else is collapsed. This is the primary signal.

2. **Most recent message** — always expanded, even if read. The latest message is the current state of the conversation. If the user has already read it, they still want to see it in context when revisiting the thread.

3. **Initial message** — always expanded, unless it's the user's own message (rule 4). The first message is the context for everything that follows — collapsing it forces the user to click just to remember what the thread is about.

4. **User's own messages** — collapsed by default. You wrote it, you know what it says. Expanding your own replies adds noise between the messages from others that you actually need to re-read. If the user wants to verify what they said, one click expands it.

5. **Everything else** — collapsed. Old, read messages from other people. Available on demand but not taking up space.

#### Collapsed Message Appearance

A collapsed message is a single-line row showing enough context to decide whether to expand:

```
│ ─ Alice Smith · Mar 8 · "Thanks for sending, I'll review by..."  │
```

Sender name, date, and the first ~60 characters of the body (stripped of quotes and signatures). This gives enough context to decide "do I need to re-read this?" without expanding.

Clicking a collapsed message expands it inline. Clicking an expanded message (other than unread ones) collapses it back.

#### Expand/Collapse All

A subtle toggle in the conversation header area — "Expand all" / "Collapse all". Not prominent, not a button you'd accidentally hit. It's there for the user who wants to read the entire thread history or who wants to collapse everything back down after exploring.

#### Why Not Persist Collapse State?

Unlike the attachment collapse state (which is persisted per thread), message collapse state is **not persisted**. It resets each time the thread is opened, re-evaluating the rules above. The reasoning: unread status changes between visits. A message that was unread (and therefore expanded) last time might be read now. Persisting the old state would show stale expand/collapse decisions. Re-evaluating on each open ensures the user always sees what's currently relevant.

### Actions

Actions are split across three levels based on what they act on and how often they're used.

#### Thread-Level Actions (reading pane, prominent)

**Star** and **label toggles** are the only thread-level actions with dedicated UI in the reading pane. They appear somewhere prominent in the reading pane header area (exact placement — above or below attachments, toolbar row or pill cloud — deferred to prototyping).

- **Star toggle** — one button, toggles starred state. Visually matches the golden card treatment in the thread list.
- **Label toggles** — one button per defined label, all visible, all toggleable. Click to add/remove that label from the thread. Colors match the label dots in the thread list. If a user has 15 labels, all 15 are shown — if they defined that many, they want them accessible.

All other thread-level actions (archive, trash, move, snooze, mark unread) live exclusively in the command palette and keyboard shortcuts. No buttons — `e` to archive, `#` to trash, `Cmd+K` for everything else.

#### Per-Message Actions (bottom of each expanded message, inline)

Reply, Reply All, and Forward appear at the bottom of each expanded message card. They're styled inline with the message content — same background color, adapted text color, hover effect. They don't look like a toolbar; they look like part of the message.

```
┌─────────────────────────────────────────────┐
│ Alice Smith · Mar 14, 2:34 PM          📅   │
│ to me, Bob                                  │
│                                             │
│ Hey, can we reschedule to Thursday?         │
│                                             │
│  ↩ Reply   ↩↩ Reply All   ↪ Forward        │
└─────────────────────────────────────────────┘
```

These are convenience shortcuts for palette actions, scoped to *this* message. The same actions in the palette act on the currently selected message (see message selection below).

#### Calendar Event Creation (per-message header, expanded only)

The "create calendar event" action (📅) appears in the message header area, right-aligned, only on expanded messages. It's elevated to header level because it's a different kind of action — it creates something in another system, not just email manipulation. Not every message warrants it, but when an email says "let's meet Thursday at 2," the action needs to be right there.

#### Message Selection

Clicking a message card in the conversation view gives it a visual selection state (outline or highlight). Palette actions that are message-specific (reply, forward, create event) act on the selected message. If no message is explicitly selected, they act on the most recent message.

This means the palette's Reply command works whether you click the inline button on a specific message or just press `r` to reply to the latest.

### Reply Interaction

Clicking Reply on a message (or pressing `r`) opens an inline reply composer directly below that message. This keeps context visible — the user can see the message they're replying to while composing. Since newest messages are at the top, replying to the most recent message means the composer appears near the top of the conversation.

For Reply All and Forward, the same inline composer appears with the appropriate recipients/content prefilled.

A "pop out" button on the inline composer opens it in a separate window for long-form composition. The command palette's Compose command (`c`) opens a standalone compose window directly.

### Right Sidebar (Calendar + Pinned Items)

A collapsible right sidebar that shows cross-cutting state alongside the main email view. Off by default. Toggled via keyboard shortcut or command palette ("Toggle Right Sidebar"). When open, the reading pane narrows to make room. When closed, the reading pane reclaims the space.

**Fixed width, not resizable.** Unlike the sidebar and thread list dividers, the right sidebar has a set width (~240-280px). No drag handle. This is a reference panel, not a primary work surface.

**Auto-collapse:** When the window width drops below ~1200px, the right sidebar automatically collapses if it's open. The reading pane needs the space more than a glance panel does at that size.

**Layout (stacked, both visible):**

```
┌──────────────────────────┐
│  ◀ March 2026            │
│  Mo Tu We Th Fr Sa Su    │
│         1  2  3  4  5    │
│   6  7  8  9 10 11 12    │
│  13 14 15 16 17 18 19    │
│  20 21 22 23 24 25 26    │
│  27 28 29 30 31          │
│──────────────────────────│
│  Today                   │
│  10:00  Standup          │
│  14:00  Client call      │
│  16:30  1:1 with Alice   │
│──────────────────────────│
│  ★ Pinned Items          │
│  Contract review (Inbox) │
│  Q2 budget sign-off      │
│  Reply to legal re: NDA  │
└──────────────────────────┘
```

**Top section: Mini calendar** — month grid + today's agenda (upcoming events). Shows enough to answer "what's my day look like" without leaving email. Clicking an event could open it in detail (TBD — depends on calendar UI).

**Bottom section: Pinned/starred items** — threads the user has starred or pinned. A persistent list of "things I need to deal with" that stays visible while triaging the inbox. Clicking an item navigates to that thread.

This replicates Outlook's To Do bar behavior — the two most common reasons enterprise users glance away from email are "when's my next meeting" and "what have I flagged for follow-up."

### Empty State

When no thread is selected, the reading pane shows a minimal empty state — the app logo or a one-liner, not a feature tour or tips. The space should feel intentionally empty, not broken.

## Interaction Model

### Keyboard-First Flow

The primary triage flow is entirely keyboard-driven:

```
j/k          — move through thread list
Enter        — expand/focus reading pane (if not already showing)
r            — reply
a / e        — archive
#            — trash
s            — star/unstar
z            — undo last action
Cmd+K        — open command palette (for everything else)
/            — focus search
Escape       — clear search / deselect / close palette
```

This matches Superhuman and Gmail's keyboard model. The shortcuts are registered in the command palette's keybinding system (Slice 3, already built) — they're not hardcoded in the UI.

### Triage Workflow

The target workflow for processing a full inbox:

1. User opens Ratatoskr, sees unified inbox sorted by date
2. First unread thread is auto-selected, conversation visible in reading pane
3. User reads → presses `e` to archive → selection auto-advances to next thread
4. User reads → presses `r` to reply → types response → presses Cmd+Enter to send → auto-archives and advances
5. User reads → presses `s` to star (deal with later) → advances
6. Repeat until inbox is empty

**Auto-advance** is critical: after any destructive/filing action (archive, trash, move), the selection moves to the next thread automatically. This eliminates the dead state of "I acted on a thread and now nothing is selected."

Auto-advance direction (next/previous) should be a user setting, defaulting to "next" (older).

### Context-Dependent Shortcuts

Some shortcuts change meaning based on context:

| Key | Thread list focused | Reading pane focused | Composer focused |
|-----|-------------------|---------------------|-----------------|
| `Enter` | Open thread in reading pane | Toggle expand on focused message | — |
| `Escape` | Deselect thread | Return focus to thread list | Close composer |
| `e` | Archive selected thread(s) | Archive current thread | — |

**Requires new work:** The command palette's `CommandContext` (`core/src/command_palette/context.rs`) does not currently model focus region. It tracks selected threads, active message, current view, account/provider info, and composer state — but has no `focused_region` field. Focus-sensitive dispatch requires: (1) adding a `focused_region` enum (ThreadList / ReadingPane / Composer / SearchBar / Sidebar) to `CommandContext`, (2) updating the iced UI to set this field on focus changes, (3) routing shortcuts through the palette's context system based on region. This is a Phase 3 prerequisite.

## What Superhuman Gets Right (and We Borrow)

1. **Split-pane with equal weight** — thread list and reading pane feel equally important, not "list" and "detail."
2. **Inline reply** — reply within the conversation, not in a separate window.
3. **Auto-advance** — after acting on a thread, move to the next one.
4. **Keyboard shortcuts** — every action has a shortcut, and the shortcuts are displayed in the command palette.
5. **Minimal chrome** — no toolbar, no menu bar, no status bar. Just content.
6. **Search as filter** — search filters the current view, not a separate mode.
7. **Speed** — every interaction feels instant. No loading spinners, no layout shifts.

## Where We Diverge from Superhuman

1. **Full folder/label tree** — Superhuman has splits (inbox sections) but no traditional folder navigation. Enterprise users need their folder trees, accessed via sidebar + palette.
2. **Multi-account depth** — Superhuman supports multiple accounts but doesn't have the scope selector / unified view model we've designed. Our sidebar handles this.
3. **Bulk operations** — Superhuman is one-at-a-time by design. Enterprise users need multi-select + bulk archive/move/label.
4. **Offline-first** — Superhuman is online-only. Ratatoskr works offline with an action queue.
5. **Open/local** — Superhuman is a cloud service. Ratatoskr is a local application with local data. No server dependency, no subscription.
6. **Command palette as primary** — Superhuman has Cmd+K but it's a feature. For us it's *the* interface — every action, every navigation, every setting.

## Implementation Sequence

Given that the sidebar is functional, the build order for the main window should be:

### Phase 1: Thread List Polish + Layout
- Update `layout.rs` constants: `THREAD_LIST_WIDTH` → 400, rename `CONTACT_SIDEBAR_WIDTH` to `RIGHT_SIDEBAR_WIDTH` (240)
- Refine thread card layout to three-line spec (no avatars, label dots, starred background)
- Label color fallback for non-Gmail providers (deterministic hash → color palette) — see `docs/main-layout/implementation-spec.md` Slice 1
- Wire up real unread/read styling from DB
- Scaffold right sidebar (replaces contact sidebar pane, off by default, placeholder content — calendar and pinned items are future features)
- Extend `WindowState` with panel widths and right-sidebar-open state for persistence
- Search bar deferred. Keyboard shortcuts (j/k) deferred to Phase 3.

### Phase 1.5: Thread Detail Data Layer (prerequisite for Phase 2)

The conversation view's collapse rules and collapsed-message summaries depend on data not currently exposed in a single query:

- **Per-message read state** — collapse rules (§ Message Collapsing) need `is_read` per message. Currently available in the DB but not surfaced in a thread-detail query.
- **Message ownership** — rule 4 (collapse user's own messages) needs matching `from_address` against the account's identity addresses. The `identities` table exists but there's no utility to test "is this message mine?"
- **Quote/signature stripping** — collapsed message appearance (§ Collapsed Message Appearance) shows "first ~60 characters of the body, stripped of quotes and signatures." This requires a text extraction pass that strips `>` quoted lines and signature blocks (`-- \n` delimiter). Neither utility exists.
- **Body text access** — even the stripped-summary path needs body text. Bodies live in `bodies.db` (compressed), accessed via `BodyStore`. The conversation view needs a query that joins thread → messages → bodies, or a denormalized snippet that's pre-stripped.

**Work required:** A `get_thread_detail()` query/function in core — see `docs/main-layout/implementation-spec.md` Slice 2. This is backend work, not UI work.

### Phase 2: Conversation View (snippet-only)
- Stacked message cards with sender, date, recipients
- Message collapsing rules applied using snippet as body placeholder (real body rendering is a separate effort; full `get_thread_detail()` integration comes when the backend slice is complete)
- Date display: prototype both Option A (relative offset) and Option B (absolute) behind a user setting, default to Option A
- Contextual action bar (Reply, Reply All, Forward, overflow) — visual only, not wired to mutations yet

### Phase 3: Interaction Flow
- Add `focused_region` enum to `CommandContext` and wire up focus tracking in iced — see `docs/main-layout/implementation-spec.md` Slice 4
- Keyboard shortcuts (j/k navigation, Enter, Escape, e/r/s/#) wired via direct iced message dispatch (command palette integration is a later migration)
- Auto-advance after archive/trash/move
- Multi-select in thread list
- Inline reply composer (basic — full composer is a separate feature)

### Phase 4: Polish
- Panel width persistence (see "Persistence Boundary" below)
- Attachment collapse state persistence
- Empty states
- Transitions and micro-interactions

## Open Questions

1. **Thread list width**: Resolved. Default increases to ~400px (~40% of remaining space after sidebar). Thread list and reading pane are closer to equal partners.

2. **Reading pane position**: Resolved. Always right. No bottom mode, no toggle. Remove the "Reading Pane Position" setting from the settings UI.

3. **Conversation order**: Resolved. Newest at top — universal rule across the entire app (thread list and conversation view). No setting, no option.

4. **Inline reply vs compose window**: Resolved. In the main reading pane, reply is always inline (below the message being replied to) with a pop-out button for long-form. In a popped-out message window, reply always opens a full compose window — no inline composer inside a pop-out.

5. **Thread list grouping**: Resolved. No. No date section headers, no visual breaks in the list. A uniform list of identical-height cards is faster to scan. The date on each card is sufficient. Section headers break the scanning rhythm the same way a horizontal colored strip on a card would.

6. **Avatars**: Resolved. No avatars in the thread list (density wins). Avatars appear in the reading pane message cards where there's room.

7. **AI summary**: Not ruled out. If added, it would replace the snippet on line 3 of the thread card. Deferred — not in V1.

## Persistence Boundary

This document introduces several categories of persisted and non-persisted UI state. Currently, `WindowState` (`crates/app/src/window_state.rs`) stores window geometry plus panel widths and right sidebar state. The following clarifies what lives where:

**Window-level state** (persisted in `window.json`, loaded on app start):
- Window geometry (exists)
- Panel widths — sidebar, thread list, right sidebar (not yet implemented, same file)
- Right sidebar open/closed (not yet implemented)

**Per-thread UI state** (persisted in SQLite, survives app restarts):
- Attachment group collapse state (thread_id → bool) — lightweight KV, described in § Collapsible

**Ephemeral UI state** (not persisted, recomputed on each thread open):
- Message expand/collapse state — recomputed from read status and collapse rules each time (§ Why Not Persist Collapse State)

**Implementation:** Panel widths can extend `WindowState` (add `sidebar_width`, `thread_list_width`, `right_sidebar_width`, `right_sidebar_open` fields with defaults). Attachment collapse state needs a small SQLite table or a KV store entry. No new persistence infrastructure is needed — just extending what exists.

## Dependencies

- **Message body rendering**: The conversation view needs rendered email bodies. This is a separate technical challenge (HTML email in iced via iced_webview_v2 or litehtml). Phase 2 can proceed with snippet-only rendering while body rendering is developed in parallel.
- **Command palette iced integration**: Keyboard shortcuts and action dispatch depend on the palette's iced integration (see `docs/cmdk/roadmap.md`, "Future: Iced Integration"). Phase 1-2 can use direct message dispatch; palette integration lands in Phase 3.
- **Compose window**: The inline reply composer (Phase 3) is a simplified version of the full compose window. Full compose is a separate feature with its own design considerations (rich text editing, attachments, signature management). The rich text editor architecture is documented in `docs/editor/architecture.md`.

## Ecosystem Patterns

How requirements in this spec map to patterns found in the [iced ecosystem survey](../iced-ecosystem-survey.md). See also the full [cross-reference](../iced-ecosystem-cross-reference.md).

### Requirements to Survey Matches

| Requirement | Primary Source | How It Applies |
|---|---|---|
| Three/four-panel resizable layout | shadcn-rs resizable panels | `auto_save_id` persistence, min/max constraints, percentage sizing |
| Rapid thread switching staleness | bloom generational tracking | Tag each `get_thread_detail()` call with generation counter; discard stale responses when user navigates faster than data loads |
| Multi-select (Shift+click, Ctrl+click) | pikeru custom MouseArea | Granular modifier-key detection on click events enables range and toggle selection |
| Panel architecture | trebuchet Component trait | Each panel (sidebar, thread list, reading pane, right sidebar) as Component with `(Task, ComponentEvent)` return, preventing Message enum explosion |
| Background sync/search/loading | pikeru + rustcast subscriptions | `subscription::channel` + `Subscription::batch()` for concurrent background tasks without blocking the UI thread |
| Token-based theming | shadcn-rs + iced-plus | Centralized palette with Token-to-Catalog bridge for automatic styling of thread cards, message cards, and action buttons |
| Settings (panel widths, preferences) | bloom config shadow | Shadow config for live preview with commit/cancel semantics |
| Keyboard shortcuts | feu + cedilla | Raw interception via `subscription::events_with` before widget processing; declarative HashMap bindings for shortcut registration |
| Typeahead popups (search bar) | shadcn-rs overlay positioning | Anchored overlay with auto-flip; adapt for left-aligned placement below the search input |
| Auto-collapse right sidebar | iced-plus Breakpoints + ShowOn | Gate right sidebar visibility by window width breakpoint (~1200px threshold) |
| Avatar/attachment loading | Lumin async batching + bloom generational | Batch `Task::perform` for avatar images; discard stale results on rapid thread navigation |
| HTML email body rendering | cedilla/frostmark | DOM-to-widget pipeline: html5ever parse, visitor pattern, iced widget tree — a third option alongside CEF and litehtml |
| Drag to label (future) | iced_drop | Wrap thread cards in `Droppable`, sidebar labels as drop zones; `Operation` trait for hit testing |

### Gaps

These requirements have no solution in the surveyed iced ecosystem and will require custom implementation:

- **Scroll virtualization**: No iced ecosystem project implements virtualized scrolling. The thread list needs to handle 1000+ fixed-height cards efficiently. The fixed `THREAD_CARD_HEIGHT` design enables a future virtualization layer, but the implementation is entirely custom.
- **Inline reply composer**: No surveyed project embeds a text editor inside a scrollable content list. The reply composer appearing below a specific message card, within the scrollable conversation view, is a novel layout challenge. The editor itself is a custom WYSIWYG widget — see `docs/editor/architecture.md`.
- **Pop-out windows**: No surveyed project demonstrates multi-window iced with shared state. Double-click-to-pop-out on message cards and the compose pop-out button both depend on this unsolved pattern.
