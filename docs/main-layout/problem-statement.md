# Main Layout: Problem Statement

## Overview

Ratatoskr's main window is where users spend 90% of their time. The layout must serve enterprise power users processing 200+ emails/day across 3+ accounts — people who are stuck on Outlook because nothing else handles their volume. The design takes cues from Superhuman's speed and focus, but adds the depth (folder trees, multi-account, bulk operations) that enterprise users require.

This document covers the main window's structure, the thread list, the conversation/reading pane, and the interaction model that ties them together. It does not cover the sidebar (see `docs/sidebar/problem-statement.md`) or the command palette (see `docs/command-palette/problem-statement.md`), though both are integral to the experience.

## Design Principles

1. **The palette is the primary interface.** Visible chrome exists for reading and orientation, not for triggering actions. Every button, menu, and toolbar is a convenience shortcut to something the palette already does.

2. **Keyboard-first, mouse-compatible.** The fastest path through email is keyboard-driven (Superhuman proved this). But the UI must not punish mouse users — hover states, click targets, and contextual actions must all work without a keyboard.

3. **No empty chrome.** If a UI element has no function in the current context, it shouldn't be visible. No disabled buttons, no greyed-out toolbars, no placeholder panels. The window shows what's relevant now.

4. **Density is a feature.** Enterprise users process volume. The default density should show more threads per screen than consumer email clients, without feeling cramped. Compact/comfortable modes are a setting, not a redesign.

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

The current fixed widths (sidebar 180px, thread list 280px) are appropriate for the sidebar and thread list. The reading pane fills remaining space. On a 4K display at 1.5x scale, this gives roughly:

```
[ 180px | 280px | ~820px reading pane ]
```

The thread list could be wider for a more Superhuman feel (where thread list and reading pane are closer to equal), but this depends on thread card design — a denser card with less horizontal waste can show the same information in 280px that a spacious card needs 400px for. Thread card design should drive width, not the other way around.

### Resizable Dividers

Already implemented. Users can drag the sidebar and thread list dividers. Minimum widths prevent collapsing panels to unusable sizes. Panel widths are not persisted yet — they should be, alongside window geometry.

## Thread List

### Purpose

The thread list answers: "what do I need to deal with?" It's a triage surface — the user scans, decides, and acts. Speed of scanning is the primary metric.

### Thread Card Content

Each card shows, in priority order:

1. **Sender** — who is this from? Most important signal for triage. Show the most recent sender for multi-message threads. Bold if unread.
2. **Subject** — what is this about? Truncated with ellipsis. Bold if unread.
3. **Snippet** — preview of the most recent message body. Secondary text color. Single line, truncated.
4. **Date/time** — when? Relative format: time today ("2:34 PM"), day this week ("Tue"), date this year ("Mar 12"), year for older ("Dec 2024").
5. **Message count** — for threads with multiple messages, show a count badge. Indicates conversation depth at a glance.
6. **Unread indicator** — a dot or bold treatment, not a separate column. Must be visible without reading the text.
7. **Avatar** — colored circle with sender initial. Provides visual anchoring and scan targets. Color derived from sender name (deterministic hash).
8. **Star/flag indicator** — if starred, show inline. Small, non-dominant.

### What Thread Cards Do NOT Show

- **Labels/tags** — these are filing metadata, not triage signals. The user already filtered by label to get here. Showing them is redundant noise.
- **Attachment indicators** — low-value for triage decisions. Available in the reading pane.
- **Account indicator** — the scope selector already tells you which account(s) you're viewing. Per-thread account badges add noise in unified view.

These can be reconsidered if user feedback shows they're needed, but the default should be minimal.

### Thread Card Layout

```
┌─────────────────────────────────────────┐
│ [●] Sender Name              Mar 12  2 │
│     Subject line truncated with ell...  │
│     Snippet preview text in second...   │
└─────────────────────────────────────────┘
```

- `[●]` = avatar circle (colored, with initial)
- `2` = message count badge (only if > 1)
- Unread threads: sender and subject in semibold, avatar has a small unread dot
- Read threads: normal weight, muted text colors

### Density Modes

Three modes, controlled by a setting:

| Mode | Card height | What changes |
|------|------------|--------------|
| **Compact** | ~48px | No snippet line. Sender + subject on one line, date on right. |
| **Default** | ~64px | Sender + date on first line, subject + snippet on second. |
| **Comfortable** | ~80px | Current three-line layout with more padding. |

Default is the right starting point. Compact is for power users with high volume. Comfortable is for users who want the visual breathing room.

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

### Conversation View

A thread is displayed as a vertical stack of message cards, newest at bottom (chronological order). Each message card shows:

1. **Sender** — name + email, with avatar
2. **Recipients** — "to me", "to me, 3 others", expandable to full list
3. **Date/time** — absolute format ("Mar 12, 2026 at 2:34 PM")
4. **Body** — rendered message content
5. **Attachments** — if any, listed below the body

### Message Collapsing

In long threads, older messages are collapsed by default. The collapse model:

- **Most recent message**: always expanded
- **Unread messages**: always expanded
- **User's own messages**: collapsed (you know what you wrote)
- **Everything else**: collapsed, showing a one-line summary (sender + date + first line of body)

Clicking a collapsed message expands it. A "expand all" / "collapse all" toggle is available but not prominent.

This is critical for enterprise threads that run to 30+ messages. Showing everything is overwhelming; the user needs to see what's new and optionally drill into history.

### Action Bar

When a thread is selected, a contextual action bar appears at the top of the reading pane. This is NOT a toolbar — it's contextual chrome that appears because there's something to act on.

```
┌─────────────────────────────────────────────┐
│  ↩ Reply   ↩↩ Reply All   ↪ Forward   ···  │
├─────────────────────────────────────────────┤
│                                             │
│  [Message cards...]                         │
│                                             │
└─────────────────────────────────────────────┘
```

The action bar shows:
- **Reply / Reply All / Forward** — the primary response actions
- **Overflow (···)** — opens a compact menu with: Archive, Trash, Star, Snooze, Mark Unread, Move to Folder, Add Label, Print, View Source

Every action in the bar is also a palette command with a keyboard shortcut. The bar exists for mouse discoverability, not as the primary interaction path.

### Reply Interaction

Clicking Reply (or pressing `r`) should open an inline reply composer at the bottom of the conversation, below the last message. This keeps context visible — the user can see the message they're replying to while composing.

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
- Refine thread card layout and density
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

### Phase 4: Density and Polish
- Compact/default/comfortable density modes
- Panel width persistence
- Empty states
- Transitions and micro-interactions

## Open Questions

1. **Thread list width**: Should the default be wider than 280px to give thread cards more room? Depends on card design — a well-designed compact card may not need it. Prototype both and compare.

2. **Reading pane position**: The settings UI has a "Reading Pane Position" option (Right/Bottom/Off). Should we build Bottom mode? It's common in Outlook/Thunderbird. It gives the thread list full width but halves vertical space for both panels. This is a post-Phase 1 consideration.

3. **Conversation order**: Newest-at-bottom (chronological, like chat) or newest-at-top (reverse chrono, like traditional email)? Superhuman uses newest-at-bottom. Outlook uses newest-at-top. This is a strong user preference — probably needs to be a setting.

4. **Inline reply vs compose window**: Should `r` always open inline, or should it depend on context (inline for short replies, compose window for long-form)? Superhuman always does inline with a pop-out option. This seems right.

5. **Thread list grouping**: Should threads be grouped by date ("Today", "Yesterday", "This Week", "Earlier")? Superhuman does this. It helps orientation but takes vertical space. Could be a density-dependent feature (shown in comfortable, hidden in compact).

6. **Avatar in reading pane vs thread list**: Should avatars appear in both places, or only in the thread list (for scan speed) with the reading pane using the extra space for content? Superhuman shows avatars in both. Enterprise users may prefer density over decoration in the reading pane.

## Dependencies

- **Message body rendering**: The conversation view needs rendered email bodies. This is a separate technical challenge (HTML email in iced via iced_webview_v2 or litehtml). Phase 2 can proceed with snippet-only rendering while body rendering is developed in parallel.
- **Command palette iced integration**: Keyboard shortcuts and action dispatch depend on the palette's iced integration (see `docs/command-palette/roadmap.md`, "Future: Iced Integration"). Phase 1-2 can use direct message dispatch; palette integration lands in Phase 3.
- **Compose window**: The inline reply composer (Phase 3) is a simplified version of the full compose window. Full compose is a separate feature with its own design considerations (rich text editing, attachments, signature management).
