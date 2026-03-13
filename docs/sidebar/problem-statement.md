# Sidebar: Problem Statement

## Overview

Ratatoskr's sidebar must serve multi-account users (typically 3+ accounts across different providers) without degenerating into Outlook's per-account folder tree. The sidebar is **navigation only** — all actions (move, label, archive) go through the command palette. Its job is answering "what am I looking at?" and providing glanceable state (unread counts, active view).

This document covers the sidebar's content model and the scope selector that controls it. It does not cover visual styling, dimensions, or animation.

## Current State

The React sidebar (`src/components/layout/Sidebar.tsx`, 838 lines) renders three sections unconditionally:

1. **Standard folders** — Inbox, Starred, Snoozed, Sent, Drafts, Trash, Spam, All Mail, Tasks, Calendar, Attachments. Hardcoded, always visible regardless of account state.

2. **Smart Folders** — Dynamic, loaded from DB. Custom icon, unread count badge. Inline creation via modal.

3. **Labels** — Dynamic, loaded per-account. Color dots, inline editing, accordion overflow. Filters out system labels.

An account switcher at the top cycles through accounts, but the folder list below doesn't change shape — it always shows the same items. Labels load for the active account but there's no unified view. There is no concept of scoping the sidebar to "all accounts" vs a specific account.

### What's Wrong

- **No unified view**: Users with 3 accounts can't see a combined inbox count or combined starred view. They cycle through accounts one at a time.
- **Folder list is static**: The same 11 items show whether the user has zero accounts or five. Items like "Tasks" and "Calendar" are shown even if those features aren't relevant to the active account.
- **Labels are single-account**: Only the currently selected account's labels are visible. Switching accounts to browse another account's labels requires cycling the account switcher.
- **Actions leak into the sidebar**: Label creation/editing, context menus for sync and delete — these are actions, not navigation. They belong in the command palette.
- **No account awareness in navigation**: "Inbox" means "inbox for whichever account is selected." There's no way to express "show me all inboxes" vs "show me just Foo Corp's inbox."

## Design: Scope Selector + Lean Navigation

### The Model

The sidebar has two layers:

1. **Scope selector** — chooses which account(s) the sidebar reflects. Options: "All Accounts" (unified) or a specific account.
2. **Navigation list** — the folders/labels shown, determined by the active scope.

### Sidebar Content by Scope

#### All Accounts (unified)

```
Inbox              12
Starred
Snoozed
Sent
Drafts
Trash

SMART FOLDERS
├ VIP               3
└ Newsletters
```

- Universal folders aggregate across all accounts. Unread counts are summed.
- Smart Folders are cross-account by design — they appear here naturally.
- **No labels section.** Labels are per-account (Gmail labels, Exchange folders, JMAP mailboxes) and mixing them in a unified view creates noise. Users who need a label navigate via the command palette or scope to a specific account.
- Spam and All Mail are omitted from the unified view — they're high-volume, rarely browsed, and their semantics differ across providers (Gmail's "All Mail" has no equivalent in Exchange). Available when scoped to a specific account.

#### Specific Account

```
Inbox               7
Starred
Snoozed
Sent
Drafts
Trash
Spam
All Mail

SMART FOLDERS
├ VIP               2
└ Newsletters

LABELS
├ Clients
├ Invoices
└ Projects
```

- Same universal folders, but counts scoped to this account.
- Account-specific labels/folders/mailboxes appear in a "Labels" section. The display adapts to the provider:
  - **Gmail**: Flat label list (labels are tags, not a hierarchy, despite Gmail's visual nesting).
  - **Exchange/Graph**: Folder tree (Exchange folders are hierarchical and a message lives in exactly one).
  - **JMAP**: Mailbox list (JMAP mailboxes can be hierarchical, similar to Exchange).
  - **IMAP**: Folder tree (IMAP LSUB hierarchy).
- Smart Folders still appear — they work cross-account and remain useful when scoped.
- Items like "All Mail" only appear if the provider supports the concept.

### Scope Selector: Open Question

The scope selector's **function** is defined: it switches between "All Accounts" and individual accounts. Its **form factor** is not. Options under consideration:

#### Option A: Dropdown / Popover

A single element at the top of the sidebar. Shows the current scope (account name + avatar, or "All Accounts"). Clicking opens a popover with the list of accounts.

- **Pro**: Minimal space — one line when closed.
- **Pro**: Familiar pattern (most multi-account apps use this).
- **Con**: Two clicks to switch (open dropdown → select account). Not great for frequent switching.
- **Con**: Current scope is only visible as text — easy to miss which account you're viewing.

#### Option B: Vertical Icon Rail

A thin (~40px) vertical strip to the left of the sidebar. Shows an "All" icon at the top, then one avatar/icon per account below it. Clicking an icon scopes the sidebar.

```
┌──┬──────────────┐
│⊕ │  Inbox    12 │
│  │  Starred     │
│🟢│  Snoozed     │
│  │  Sent        │
│🔵│  Drafts      │
│  │  Trash       │
│🟣│              │
│  │  SMART ...   │
└──┴──────────────┘
```

- **Pro**: All accounts always visible — instant switching, one click.
- **Pro**: Scales to 5+ accounts without growing the main sidebar.
- **Pro**: Color-coded accounts give at-a-glance orientation.
- **Con**: Uses ~40px of horizontal space permanently.
- **Con**: Less conventional for email — more associated with chat apps (Slack, Discord, Teams).

#### Option C: Vertical Tabs

Horizontal tabs rotated 90°, stacked vertically at the top of the sidebar or along its left edge. Each tab shows the account name or abbreviation. The active tab is visually highlighted.

- **Pro**: Tabs are a well-understood "scope" metaphor — users know exactly one is active.
- **Pro**: Can show truncated account names, not just icons — less ambiguous than avatars.
- **Con**: Takes significant vertical space if account names are long.
- **Con**: Competes with the navigation list for vertical real estate.

#### Option D: Horizontal Segmented Control / Chips

A row of chips or a segmented control at the top of the sidebar: `[All] [Foo] [Gmail] [Bar]`.

- **Pro**: All options visible at once, single click to switch.
- **Pro**: Familiar "filter" pattern.
- **Con**: Doesn't scale — 4+ accounts overflow the sidebar width.
- **Con**: Long account names get truncated aggressively at 180px sidebar width.

### What the Sidebar Does NOT Do

These are explicitly out of scope for the sidebar, handled by the command palette instead:

- **Filing/moving emails** — "Move to Folder" is a palette command with account-aware second stage
- **Creating/editing/deleting labels** — palette commands
- **Sync operations** — palette command ("Sync" / "Sync This Folder")
- **Context menus** — the palette replaces right-click menus
- **Folder management** — creating, renaming, reordering folders
- **Account management** — adding, removing, reordering accounts

## Constraints

- The sidebar must not grow proportionally with account count. Adding a 4th account should not add a 4th section of folders.
- The sidebar's content is derived from the same data the command palette uses (labels, folders, accounts, unread counts). It is a read-only view, not a separate data path.
- The scope selector must be operable via keyboard (the command palette can also switch scope: `Navigate > Switch Account > Foo Corp`).
- Smart Folders are always visible regardless of scope — they are a user-defined cross-account concept.

## Open Questions

1. **Scope selector form factor**: See options A–D above. This is a product/UX decision that should be informed by prototyping. The backend and data model are identical regardless of which option is chosen.

2. **Unread count granularity in unified view**: Should "Inbox 12" in the unified view be expandable to show per-account breakdown (Foo: 7, Gmail: 3, Bar: 2), or is the total sufficient? A breakdown helps triage ("the 7 are all work") but adds visual complexity.

3. **Provider-specific folder display**: Gmail's label model (tags, flat, multiple per message) is fundamentally different from Exchange's folder model (tree, exclusive, one per message). When scoped to a Gmail account, should the "Labels" section look and behave differently from the "Folders" section when scoped to an Exchange account? Or should the sidebar normalize them into one visual pattern?

4. **Pinned labels/folders**: Should users be able to pin specific labels/folders so they appear in the unified view alongside the universal folders? This would let a user promote "Foo Corp > Clients" to top-level visibility without scoping to Foo Corp. It could blur the clean separation between unified and scoped views, but it's a common power-user request.
