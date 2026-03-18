# Main Layout: Problem Statement

## Overview

Ratatoskr's main window is where users spend 90% of their time. The layout must serve enterprise power users processing 200+ emails/day across 3+ accounts — people who are stuck on Outlook because nothing else handles their volume. The design takes cues from Superhuman's speed and focus, but adds the depth (folder trees, multi-account, bulk operations) that enterprise users require.

This document covers the main window's structure, the thread list, the conversation/reading pane, and the interaction model that ties them together. It does not cover the sidebar (see `docs/sidebar/problem-statement.md`) or the command palette (see `docs/command-palette/problem-statement.md`), though both are integral to the experience.

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

The layout decisions are sound. The structure doesn't need to change — it needs to be built out.

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

Label colors come from the provider where available (Gmail label colors, Exchange category colors) or are assigned by Ratatoskr for providers without color support (IMAP folders get deterministic hash-based colors).

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

Each attachment card shows when the attachment arrived or was last modified:

- **Primary:** file modification date from `Content-Disposition` headers (`modification-date` parameter, RFC 2183) or extractable file metadata (EXIF, PDF creation date, Office document properties) — when available
- **Fallback:** the date of the email message that contained the attachment — always available

The date is shown alongside the sender who sent that attachment ("Mar 14 from Alice"). This is especially important in the versioning view, where dates distinguish versions:

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
   - This may warrant a user setting. The right default isn't obvious without seeing it in context.
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

The command palette's context system (CommandContext) already models this — `focused_region` is part of the context struct.

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

Given that the layout structure is settled and the sidebar is functional, the build order for the main window should be:

### Phase 1: Thread List Polish
- Refine thread card layout
- Implement keyboard navigation (j/k, Enter to select)
- Wire up real unread/read styling from DB
- Search bar above thread list (visual only — real search is a later feature)

### Phase 2: Conversation View
- Stacked message cards with sender, date, recipients
- Message collapsing (newest expanded, older collapsed)
- Placeholder for message body (snippet for now, real body rendering is a separate effort)
- Contextual action bar (Reply, Reply All, Forward, overflow)

### Phase 3: Interaction Flow
- Auto-advance after archive/trash/move
- Keyboard shortcuts wired to command palette
- Multi-select in thread list
- Inline reply composer (basic — full composer is a separate feature)

### Phase 4: Polish
- Panel width persistence
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

## Dependencies

- **Message body rendering**: The conversation view needs rendered email bodies. This is a separate technical challenge (HTML email in iced via iced_webview_v2 or litehtml). Phase 2 can proceed with snippet-only rendering while body rendering is developed in parallel.
- **Command palette iced integration**: Keyboard shortcuts and action dispatch depend on the palette's iced integration (see `docs/command-palette/roadmap.md`, "Future: Iced Integration"). Phase 1-2 can use direct message dispatch; palette integration lands in Phase 3.
- **Compose window**: The inline reply composer (Phase 3) is a simplified version of the full compose window. Full compose is a separate feature with its own design considerations (rich text editing, attachments, signature management).
