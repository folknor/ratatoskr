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

### Scope as State

Scope is **app UI state**, not router state and not a command-palette action. Changing scope does not navigate — it re-filters the sidebar and the thread list behind it. If the user is viewing "Inbox" and switches scope from "All" to "Foo Corp," they stay on Inbox; the content narrows.

The command palette can *set* scope (e.g., `Navigate > Switch Account > Foo Corp`), just as it can toggle the sidebar or switch themes — these are UI state mutations exposed as commands for keyboard accessibility, not proof that they belong in the command system's dispatch model. The scope value lives in a single place in app state and the sidebar reads it. There is no second implementation path.

### Tasks, Calendar, and Attachments

These are intentionally absent from the proposed sidebar. They are separate product areas, not mailbox filters, and showing them unconditionally in the folder list conflates navigation contexts.

They become **palette-first destinations**: `Navigate > Tasks`, `Navigate > Calendar`, `Navigate > Attachments`. If usage data later shows users reaching for them frequently enough to justify persistent sidebar presence, they can be added back — but as an explicit decision, not a default.

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

- Universal folders aggregate across all accounts. Unread counts are summed. See "Universal Folder Semantics" below for how aggregation works per folder.
- Smart Folders are cross-account by design — they appear here naturally. Smart Folders are **exempt from scoping**: they always query across all accounts, even when the sidebar is scoped to a specific account. This is intentional — a Smart Folder like "VIP" is a user-defined cross-account concept and filtering it to one account would defeat its purpose. The sidebar content is therefore: scope-filtered universal folders + scope-filtered labels + unscoped Smart Folders.
- **No labels section.** Labels are per-account (Gmail labels, Exchange folders, JMAP mailboxes) and mixing them in a unified view creates noise. Users who need a label navigate via the command palette or scope to a specific account. **Prerequisite**: the command palette must support `Navigate > [Label]` with cross-account disambiguation (showing which account each label belongs to) before the sidebar's label browse path can be removed. Until then, removing labels from the unified sidebar creates a discoverability regression.
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
- Smart Folders still appear — they work cross-account and remain useful when scoped (see note on Smart Folder scoping exemption above).
- Items like "All Mail" only appear if the provider supports the concept.

### Universal Folder Semantics

The unified view treats Inbox, Starred, Snoozed, Sent, Drafts, and Trash as universal folders. But "universal" overstates the equivalence — these concepts map differently across providers, and the aggregation layer must account for this:

| Folder | Gmail API | Exchange/Graph | JMAP | IMAP |
|--------|-----------|----------------|------|------|
| **Inbox** | `INBOX` label | Inbox well-known folder | Inbox role mailbox | INBOX |
| **Starred** | `STARRED` label (tag, multiple per message) | Flag status on message (not a folder) | `$flagged` keyword | \Flagged flag |
| **Snoozed** | Local feature — not a provider concept. Ratatoskr implements snooze locally across all providers. | Same | Same | Same |
| **Sent** | `SENT` label | Sent Items well-known folder | Sent role mailbox | Sent (varies, SPECIAL-USE \Sent) |
| **Drafts** | `DRAFT` label + local unsent state | Drafts well-known folder + local unsent | Drafts role mailbox + local unsent | Drafts (\Drafts) + local unsent |
| **Trash** | `TRASH` label (30-day auto-purge) | Deleted Items folder (retention policy varies) | Trash role mailbox | Trash (\Trash, behavior varies by server) |

Key differences that affect aggregation:

- **Starred is not a folder everywhere.** Gmail and JMAP treat it as a label/keyword (a message can be "starred" and in "Inbox" simultaneously). Exchange and IMAP treat it as a flag on a message that lives in a folder. The sidebar's "Starred" destination must be a **virtual query** ("all messages with the starred/flagged attribute across all accounts"), not a folder listing.
- **Drafts have a local component.** A draft may exist only locally (unsent compose), only on the server (composed on another device), or both (synced draft). The "Drafts" count must include local-only drafts.
- **Trash retention differs.** Gmail auto-purges after 30 days. Exchange follows org retention policy. JMAP/IMAP vary by server. The unified Trash view aggregates, but "empty trash" is per-account because the semantics differ.
- **Sent is straightforward.** All providers have a clear Sent concept. Aggregation is a simple union.

The backend normalizes these into a **unified query model** where each sidebar destination maps to a provider-agnostic query predicate (e.g., `is_starred = 1` across accounts), not a provider-specific folder/label ID. This is implemented in `core/src/db/queries_extra/scoped_queries.rs`:

- **`AccountScope`** enum (`Single`/`Multiple`/`All`) controls which accounts a query spans.
- **Starred and Snoozed** use predicate-based queries against boolean flags on the `threads` table (`get_starred_threads`, `get_snoozed_threads`), not label joins.
- **Inbox, Sent, Drafts, Trash, Spam** use the existing `thread_labels` join with well-known label IDs.
- **Drafts count** includes local-only drafts from the `local_drafts` table via `get_draft_count_with_local()`.
- **`get_navigation_state()`** (`core/src/db/queries_extra/navigation.rs`) returns the full sidebar state in one call: universal folders with unread counts, smart folders, and (when scoped to a single account) that account's non-system labels.
- **Smart folder unread counts and per-label unread counts** are scaffolded as 0 — not yet implemented.

### Navigation Contract

When a user clicks a sidebar item, the result must be consistent regardless of provider. The sidebar normalizes all destinations into one contract:

**Clicking a sidebar item = "show me all messages matching this predicate, within the current scope."**

This means:

- **Universal folders** are predicate-based queries: `folder:inbox`, `is:starred`, `is:snoozed`, `folder:sent`, `is:draft`, `folder:trash`. The predicate is evaluated against all accounts in the current scope.
- **Account-specific labels** (Gmail) filter by label tag: `label:Clients AND account:foo`. A message may appear in multiple label views because Gmail labels are non-exclusive tags.
- **Account-specific folders** (Exchange, IMAP, JMAP) filter by folder membership: `folder:Clients AND account:foo`. A message appears in exactly one folder view because these are exclusive containers.

The sidebar does not need to expose this difference to the user — "Clients" looks the same whether it's a Gmail label or an Exchange folder. But the routing layer must know the difference because:

1. **Gmail label**: Clicking "Clients" queries for messages with the `Clients` label. A message can appear here AND in Inbox simultaneously. There is no concept of "moving out of" a label view — removing the label is a separate action.
2. **Exchange/IMAP/JMAP folder**: Clicking "Clients" queries for messages in the `Clients` folder. A message is in exactly one folder. Moving it here removes it from its previous folder.

This distinction matters for the thread list display (should it show "also in: Inbox" for Gmail labels?) and for drag-and-drop if we ever support it, but it does not affect the sidebar's own rendering — the sidebar is a list of clickable destinations, and the click always means "filter to this."

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

3. **Provider-specific folder display**: Gmail's label model (tags, flat, multiple per message) is fundamentally different from Exchange's folder model (tree, exclusive, one per message). When scoped to a Gmail account, should the "Labels" section look and behave differently from the "Folders" section when scoped to an Exchange account? Or should the sidebar normalize them into one visual pattern? The Navigation Contract section above defines the click semantics, but the visual question remains: does a Gmail-scoped sidebar say "LABELS" and an Exchange-scoped sidebar say "FOLDERS", or do we use a single neutral term?

4. **Pinned labels/folders**: Should users be able to pin specific labels/folders so they appear in the unified view alongside the universal folders? This would let a user promote "Foo Corp > Clients" to top-level visibility without scoping to Foo Corp. It could blur the clean separation between unified and scoped views, but it's a common power-user request.

5. **Smart Folder interaction with scope**: When a user is scoped to "Foo Corp" and clicks a Smart Folder (which is cross-account by definition), what happens to the scope indicator? Options: (a) scope visually switches to "All" for the duration, (b) scope stays on "Foo Corp" but the content pane shows cross-account results with an indicator, (c) Smart Folders filter to the current scope (breaking their cross-account nature). This affects whether scope is a global filter or a sidebar-local concern.

6. **Scope in URL/router state**: Should the active scope be part of the URL so that deep links and browser back/forward preserve it? If scope is purely in-memory UI state, refreshing the app loses the user's account context. If it's in the URL, the routing model gets more complex. The current React app uses TanStack Router with hash history — scope could be a search param (`#/inbox?scope=foo-corp`) without affecting the route structure.
