# Sidebar: Problem Statement

## Overview

Ratatoskr's sidebar must serve multi-account users (typically 3+ accounts across different providers) without degenerating into Outlook's per-account folder tree. The sidebar is **navigation only** — all actions (move, label, archive) go through the command palette. Its job is answering "what am I looking at?" and providing glanceable state (unread counts, active view).

This document covers the sidebar's content model and the scope selector that controls it. It does not cover visual styling, dimensions, or animation.

The sidebar also contains the **calendar mode toggle button** (see `docs/calendar/problem-statement.md` § Mode Switcher) and the **pinned searches section** (see `docs/search/pinned-searches.md`).

## Current State

The sidebar is implemented as an iced view function (`crates/app/src/ui/sidebar.rs`) following iced's Elm architecture. `SidebarModel` holds a reference to the current accounts, labels, selected scope, and section-expand state; the `view()` function renders the sidebar from that model; user interactions produce `Message` variants that the top-level `update()` handles.

The sidebar already implements much of the design in this document:

1. **Scope dropdown** — A dropdown at the top of the sidebar (Option A from below) lets the user choose "All Accounts" or a specific account. Scope state lives in the app's main model (`selected_account: Option<usize>`).

2. **Universal folders** — Inbox, Starred, Snoozed, Sent, Drafts, Trash are always shown. Spam and All Mail appear only when scoped to a specific account.

3. **Smart Folders** — Collapsible section, always visible regardless of scope.

4. **Labels** — Collapsible section, shown only when scoped to a specific account. System labels are filtered out.

Tasks and Attachments are already absent from the sidebar — they are palette-first destinations as specified below. Calendar has its own full-view mode accessed via a toggle button in the sidebar header (see `docs/calendar/problem-statement.md`).

### What's Still Missing

- **Live unread counts**: Nav items currently use placeholder counts (hardcoded in the view function). They need to be driven by `get_navigation_state()` from `crates/core/src/db/queries_extra/navigation.rs`.
- **Smart folder unread counts**: Scaffolded as 0 in the backend — not yet wired.
- **Per-label unread counts**: Also scaffolded as 0.
- **Hierarchy support for labels**: The `NavigationFolder` struct has no parent ID or path — Exchange/IMAP/JMAP folder trees are flattened (see details in the "Specific Account" section below).
- **Actions still present in sidebar**: Label creation/editing, context menus — these should move to the command palette (Phase 2).

## Design: Scope Selector + Lean Navigation

### The Model

The sidebar has two layers:

1. **Scope selector** — chooses which account(s) the sidebar reflects. Options: "All Accounts" (unified) or a specific account.
2. **Navigation list** — the folders/labels shown, determined by the active scope.

### Scope as State

Scope is **app model state**, not a command-palette action. Changing scope does not navigate — it re-filters the sidebar and the thread list behind it. If the user is viewing "Inbox" and switches scope from "All" to "Foo Corp," they stay on Inbox; the content narrows.

The command palette can *set* scope (e.g., `Navigate > Switch Account > Foo Corp`), just as it can toggle the sidebar or switch themes — these are model state mutations exposed as commands for keyboard accessibility, not proof that they belong in the command system's dispatch model. The scope value lives in a single place in the iced app model (`selected_account: Option<usize>` in `crates/app/`) and the sidebar reads it via `SidebarModel`. There is no second implementation path.

### Tasks and Attachments

These are intentionally absent from the sidebar. They are separate product areas, not mailbox filters, and showing them unconditionally in the folder list conflates navigation contexts.

They become **palette-first destinations**: `Navigate > Tasks`, `Navigate > Attachments`. If usage data later shows users reaching for them frequently enough to justify persistent sidebar presence, they can be added back — but as an explicit decision, not a default.

**Calendar** is not a palette destination — it has its own full-view mode toggled via a button in the sidebar header area. See `docs/calendar/problem-statement.md` for the complete calendar spec.

### Sidebar Content by Scope

#### All Accounts (unified)

```
[📅] [Scope Dropdown    ]
[  ] [   Compose        ]

 from:alice ha..  ✕       ← Pinned searches
 2 hours ago               (see docs/search/pinned-searches.md)
 is:unread aft..  ✕
 3 days ago

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

The calendar toggle button sits in the top-left of the header area, to the left of the scope dropdown and compose button (see `docs/calendar/problem-statement.md` § Mode Switcher for dimensions). Pinned searches appear below the header, above the universal folders.

- Universal folders aggregate across all accounts. Unread counts are summed. See "Universal Folder Semantics" below for how aggregation works per folder.
- Smart Folders always appear in the sidebar regardless of scope. They are saved searches — their queries run through the unified search pipeline exactly as written. A smart folder with `account:FooCorp in:inbox is:unread` searches only Foo Corp; one with `is:unread after:-7` spans everything. The scope selector has no effect on smart folders — neither their visibility nor their query execution. The sidebar content is therefore: scope-filtered universal folders + scope-filtered labels + all Smart Folders (unscoped).

  **Requires code change:** `query_smart_folders_sync` in `crates/core/src/db/queries_extra/navigation.rs` currently filters smart folders by scope/account_id. This must change to always return all smart folders regardless of the active scope.
- **No labels section.** Labels are per-account (Gmail labels, Exchange folders, JMAP mailboxes) and mixing them in a unified view creates noise. Users who need a label navigate via the command palette or scope to a specific account. **Prerequisite**: the command palette must support `Navigate > [Label]` with cross-account disambiguation (showing which account each label belongs to) before the sidebar's label browse path can be removed. Until then, removing labels from the unified sidebar creates a discoverability regression.
- Spam and All Mail are omitted from the unified view — they're high-volume, rarely browsed, and their semantics differ across providers (Gmail's "All Mail" has no equivalent in Exchange). Available when scoped to a specific account.

#### Specific Account

```
[📅] [Account Name  ▾]
[  ] [   Compose     ]

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

  **Requires backend work:** The current `NavigationFolder` struct (`crates/core/src/db/queries_extra/navigation.rs`) has no hierarchy support — no parent ID, no path, and no label-vs-folder discriminator. All provider-specific items are flattened to `FolderKind::AccountLabel`. To support provider-adaptive display, the navigation model needs: (1) a `parent_id: Option<String>` or `path: Option<String>` for tree rendering (Exchange/IMAP/JMAP), (2) a discriminator indicating whether the item is a tag (Gmail label — non-exclusive, multiple per message) or a folder (Exchange/IMAP/JMAP — exclusive, one per message). The view function uses the discriminator for "also in" indicators and future drag-and-drop semantics, not just rendering. The `DbLabel` type already has the raw data (`label_type`, `imap_folder_path`) — the navigation query needs to expose it.

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
- **Drafts have a local component.** A draft may exist only locally (unsent compose), only on the server (composed on another device), or both (synced draft). The "Drafts" count must include local-only drafts, and **clicking Drafts must show them all** — the list is a mixed view of server-synced draft threads and local-only drafts. **Requires new work:** `get_draft_threads()` currently returns only server-synced drafts (which have `DbThread` representations via the DRAFT label). Local-only drafts in the `local_drafts` table have a different schema and no thread representation. To deliver a mixed drafts list, either: (a) define a union result type (e.g., `DraftItem` enum with `ServerDraft(DbThread)` and `LocalDraft(DbLocalDraft)` variants) that the thread list can render, or (b) promote local drafts to a lightweight `DbThread`-compatible shape at query time. The count path (`get_draft_count_with_local`) already handles both sources — the list path must match.
- **Trash retention differs.** Gmail auto-purges after 30 days. Exchange follows org retention policy. JMAP/IMAP vary by server. The unified Trash view aggregates, but "empty trash" is per-account because the semantics differ.
- **Sent is straightforward.** All providers have a clear Sent concept. Aggregation is a simple union.

The backend normalizes these into a **unified query model** where each sidebar destination maps to a provider-agnostic query predicate (e.g., `is_starred = 1` across accounts), not a provider-specific folder/label ID. This is implemented in `crates/core/src/db/queries_extra/scoped_queries.rs`:

- **`AccountScope`** enum (`Single`/`Multiple`/`All`) controls which accounts a query spans.
- **Starred and Snoozed** use predicate-based queries against boolean flags on the `threads` table (`get_starred_threads`, `get_snoozed_threads`), not label joins.
- **Inbox, Sent, Drafts, Trash, Spam** use the existing `thread_labels` join with well-known label IDs.
- **Drafts count** includes local-only drafts from the `local_drafts` table via `get_draft_count_with_local()`.
- **`get_navigation_state()`** (`crates/core/src/db/queries_extra/navigation.rs`) returns the full sidebar state in one call: universal folders with unread counts, smart folders, and (when scoped to a single account) that account's non-system labels.
- **Smart folder unread counts and per-label unread counts** are scaffolded as 0 — not yet implemented.

### Navigation Contract

When a user clicks a sidebar item, the result must be consistent regardless of provider. The sidebar normalizes all destinations into one contract:

**Clicking a sidebar item = "show me all messages matching this predicate, within the current scope."**

This means:

- **Universal folders** are predicate-based queries: `in:inbox`, `is:starred`, `is:snoozed`, `in:sent`, `is:draft`, `in:trash`. The `in:` operator is the universal folder shorthand defined in the search spec (`docs/search/problem-statement.md`); `folder:` is reserved for provider-specific folder paths. The predicate is evaluated against all accounts in the current scope.
- **Account-specific labels** (Gmail) filter by label tag: `label:Clients account:foo`. A message may appear in multiple label views because Gmail labels are non-exclusive tags.
- **Account-specific folders** (Exchange, IMAP, JMAP) filter by folder membership: `folder:Clients account:foo`. A message appears in exactly one folder view because these are exclusive containers.

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

3. **Provider-specific folder display**: Gmail's label model (tags, flat, multiple per message) is fundamentally different from Exchange's folder model (tree, exclusive, one per message). When scoped to a Gmail account, should the "Labels" section look and behave differently from the "Folders" section when scoped to an Exchange account? Or should the sidebar normalize them into one visual pattern? The Navigation Contract section above defines the click semantics. The section header will use "Labels" universally — Gmail has trained most users to understand this term, and conditional rendering for a section header isn't worth the complexity. The underlying click semantics still differ per provider (tags vs exclusive containers) as documented in the Navigation Contract.

4. **Pinned labels/folders**: Should users be able to pin specific labels/folders so they appear in the unified view alongside the universal folders? This would let a user promote "Foo Corp > Clients" to top-level visibility without scoping to Foo Corp. It could blur the clean separation between unified and scoped views, but it's a common power-user request.

5. **Smart Folder interaction with scope**: Resolved. The scope indicator stays unchanged when a Smart Folder is clicked. Smart Folders are saved searches — their queries run through the unified search pipeline exactly as written, independent of the scope selector. The thread list shows whatever the query produces. Scope controls universal folders and label visibility; Smart Folders are exempt.

6. **Scope persistence**: Should the active scope be persisted so that relaunching the app restores the user's account context? Currently scope lives in the iced app model as in-memory state and resets on restart. Options: persist to the SQLite settings table, or treat "All Accounts" as a reasonable default on launch.

7. **Default sender account for compose**: When the user opens a new compose window, which account's address should be the default "From"? This matters most in "All Accounts" scope where there's no single obvious answer. The resolution order:

   1. **Explicit selection** — the user picks a sender in the compose window. This choice is honored unconditionally and becomes the "last manually selected" value for step 3.
   2. **Thread context** — if a thread is selected, default to the account involved in that thread (specifically, the account that most recently received a message in the thread — not just any participant, which matters when the user has forwarded between their own accounts).
   3. **Last manually selected sender** — a sticky preference from the most recent time the user explicitly chose a sender in step 1. Persisted across sessions.
   4. **Current scope** — if the sidebar is scoped to a specific account, use that account.
   5. **First account** — if none of the above apply (essentially first launch in unified view before any activity), fall back to whatever the account ordering produces. No special tracking needed.

   This cascade covers the common cases (replying, composing in a scoped view) in steps 2 and 4, and the edge cases (fresh unified compose) degrade gracefully without requiring additional state beyond the sticky "last manually selected" preference.

8. **Label navigation from unified view**: When the user navigates to a label via the command palette while in "All Accounts" scope, the scope should not auto-narrow to a single account. The label filter is applied across all accounts that have a matching label — if "Clients" exists on both Gmail and Exchange, the thread list shows threads from both. The scope stays on "All." This is consistent with how universal folders work (Inbox shows all accounts' inboxes) and avoids the jarring implicit scope switch. The palette's cross-account disambiguation (see `docs/command-palette/problem-statement.md`, "Cross-Account Label/Folder Disambiguation") lets the user pick a specific account's label if they want to narrow, but the default behavior is additive.

## Dependencies

- **Command palette Slice 2** (`docs/command-palette/roadmap.md`): The `NavigateToLabel` parameterized command with cross-account disambiguation must be implemented before labels can be removed from the unified sidebar (Phase 2). The resolver, `OptionItem` structure, and fuzzy search infrastructure are already scaffolded — what remains is the real `CommandInputResolver` implementation that queries account labels/folders from `DbState`. See "Cross-Account Label/Folder Disambiguation" in `docs/command-palette/problem-statement.md`.
- **Command palette Slice 6** (`docs/command-palette/roadmap.md`): Phase 2 (stripping actions from sidebar) is gated on the palette implementation being far enough along to absorb label creation/editing/deletion and context menu actions.

Phase 1 has no palette dependency and can proceed independently.

## Implementation Phases

### Phase 1: Scoped sidebar (no palette dependency)

Ship the scoped sidebar in the iced app. The sidebar already has scope awareness and the basic structure (scope dropdown, universal folders, smart folders, labels). This phase wires it to live data from the core crate. No action removal yet — label editing, context menus, etc. stay in place until the palette can absorb them.

**Backend wiring:**
- The app calls core functions directly (no IPC bridge — the Tauri shell has been removed). The sidebar view function (`crates/app/src/ui/sidebar.rs`) receives its data via `SidebarModel`, which the top-level `update()` populates by calling `get_navigation_state()` from `crates/core/src/db/queries_extra/navigation.rs`.
- Wire up smart folder unread counts in `get_navigation_state` (currently scaffolded as 0). This requires calling `count_smart_folder_unread` from the smart folder engine (`crates/smart-folder/`) for each folder — evaluate whether the cost is acceptable per sidebar refresh or whether it needs caching.

**Scope state:**
- Already implemented: `selected_account: Option<usize>` in the iced app model, toggled via `Message::SelectAccount` / `Message::SelectAllAccounts`. The scope dropdown (Option A) is built. Persistence to survive app restart is deferred (see open question #6).

**Sidebar data flow:**
- Replace the hardcoded placeholder counts in `nav_items()` with live data from `get_navigation_state(scope)`. The `NavigationState` response drives the entire sidebar: universal folders with real unread counts, smart folders, and (when scoped) account labels.
- Tasks, Calendar, Attachments are already absent from the sidebar.

**Thread list wiring:**
- When viewing a universal folder, call the appropriate scoped query (`get_threads_scoped` for label-based folders, `get_starred_threads` for Starred, etc.) with the current `AccountScope` derived from `selected_account`.
- The thread list view function doesn't need to know about scoping — it receives threads the same way it does today. The change is in what the parent `update()` passes down.

**What ships:**
- Unified inbox across accounts (the headline feature).
- Scope selector to narrow to one account.
- Account-specific labels visible when scoped.
- Correct unread counts for universal folders (aggregated or per-account). Smart folder and per-label unread counts remain scaffolded as 0 — implementing them is separate work (see backend wiring notes above).
- Sidebar actions (label edit, context menus) still work as before — no regression.

### Phase 2: Strip actions from sidebar (palette dependency)

Remove action-related code from the sidebar once the command palette handles those responsibilities. This phase is gated on the command palette being functional enough to replace:

- Label creation/editing/deletion -> palette commands
- Context menus (sync folder, delete label) -> palette commands
- Any inline modals triggered from the sidebar -> palette second-stage UI

**What changes:**
- Remove any inline edit affordances and label management widgets from the sidebar view function (`crates/app/src/ui/sidebar.rs`).
- Remove any sidebar context menu handling from the `update()` function.
- The sidebar becomes a pure read-only navigation list — click to navigate, nothing else.

**Prerequisite:** The command palette must support `Navigate > [Label]` with cross-account disambiguation (see unified view prerequisite note above). Without this, removing labels from the unified sidebar creates a discoverability regression.

### Phase 3: Scope selector iteration (optional, post-ship)

Once phase 1 is in users' hands, evaluate whether the dropdown is sufficient or whether a different form factor (icon rail, vertical tabs, chips) better serves the 3+ account use case. This is informed by real usage patterns — how often users switch scope, whether they stay scoped or return to "All", whether they miss the visual account indicator.

This phase is pure UI polish with no backend or data model changes.
