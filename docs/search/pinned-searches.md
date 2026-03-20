# Pinned Searches

## Overview

Pinned searches are ephemeral, user-curated search result sets that live at the top of the sidebar. They fill the gap between throwaway searches and persistent smart folders — a lightweight way to park a set of threads as a working context without committing to a permanent smart folder.

The motivating use case: a user searches for threads they need to act on today. They don't want a smart folder that re-evaluates every time — they want a static list of "these 12 threads" that they can work through and dismiss. This is the Thunderbird "search results tab" pattern, adapted to Ratatoskr's single-window layout.

## Model

A pinned search stores:

```
PinnedSearch {
    id: u64,
    query: String,              // the search query string
    thread_ids: Vec<(String, String)>,  // (thread_id, account_id) snapshot
    created_at: i64,            // when the search was first run
    updated_at: i64,            // when results were last refreshed
}
```

- **`query`** is the full query string (same syntax as the search bar and smart folders).
- **`thread_ids`** is a snapshot of matching threads at the time the search was run. This is the result set — it does not re-evaluate automatically.
- **Thread metadata is always live.** When the user clicks a pinned search, the thread list fetches current state (read/unread, starred, snippet, message count) for the stored thread IDs from the database. The snapshot determines *which* threads to show; the database determines *how* they look right now.

## Lifecycle

### Creation

Every search automatically creates a pinned search. There is no explicit "pin" action.

1. User types a query in the search bar and executes it (Enter or debounce fires)
2. Results appear in the thread list
3. A new pinned search entry appears at the top of the sidebar's pinned searches section, showing the query string and "Just now"

### Editing in Place

If the user modifies the query in the search bar *without navigating away* (no folder or sidebar clicks in between), the existing pinned search is updated rather than creating a new one.

- The query string is replaced with the new query
- The thread ID snapshot is replaced with the new results
- `updated_at` is refreshed
- The entry stays in the same position (top of the list, since it's most recent)

"Navigating away" means clicking a folder, label, smart folder, or another pinned search in the sidebar. Once the user navigates away, any subsequent search creates a new pinned search.

### Refreshing

When the user clicks a pinned search:
1. The search bar fills with the stored query string
2. The thread list shows the stored thread IDs with their current metadata from the database
3. A label below the search bar (or inline) shows "Last updated 3 days ago" (or "Just now", "2 hours ago", etc.)
4. The user can hit Enter / click search to re-execute the query, which refreshes the thread ID snapshot and updates the timestamp

Re-executing does not create a new pinned search — it updates the active one (same as editing in place).

### Dismissal

Each pinned search has an X button. Clicking it removes the entry immediately. No confirmation dialog.

A "Clear all" action is available (in the section header or via command palette) for bulk cleanup.

### Graduation to Smart Folder

A pinned search can be promoted to a smart folder via the command palette ("Save as Smart Folder" action, available when a pinned search is active). This:

1. Prompts for a name and optional icon
2. Saves the query string as a new smart folder
3. Removes the pinned search from the sidebar
4. The new smart folder appears in the Smart Folders section and re-evaluates its query live, like any other smart folder

This gives a natural progression: search → pinned search (automatically) → smart folder (explicitly promoted).

## Sidebar Placement and Visual Design

Pinned searches occupy the **top of the sidebar**, above the compose button, universal folders, smart folders, and labels. They are visually distinct from the rest of the sidebar to signal their ephemeral, task-oriented nature.

### Layout

```
┌──────────────────────┐
│ [Scope Dropdown]     │
│                      │
│ ┌──────────────────┐ │  ← Pinned searches section
│ │ from:alice has:.. ✕│ │    (visually distinct)
│ │ 2 hours ago       │ │
│ │──────────────────│ │
│ │ is:unread after:-7✕│ │
│ │ 3 days ago        │ │
│ └──────────────────┘ │
│                      │
│ [+ Compose]          │
│                      │
│ Inbox            12  │  ← Normal sidebar starts here
│ Starred           3  │
│ Snoozed           1  │
│ Sent                 │
│ Drafts            2  │
│ Trash                │
│ ──────────────────── │
│ ▶ SMART FOLDERS      │
│ ──────────────────── │
│ ▶ LABELS             │
│                      │
│ [⚙ Settings]        │
└──────────────────────┘
```

### Visual Treatment

Pinned searches should look different from navigation items to reinforce that they are temporary working contexts, not permanent destinations:

- **Background**: a subtle card or chip-like container, slightly elevated from the sidebar background (one palette step up — e.g., `weakest` if sidebar rests on `base`)
- **Query text**: primary text color, truncated with ellipsis if the query is long. Single line.
- **Timestamp**: secondary/muted text, small (`TEXT_XS`), below the query text. Relative format: "Just now", "2 hours ago", "3 days ago".
- **Dismiss button**: small X icon, right-aligned, visible on hover or always visible (TBD based on density). Uses `text_muted()` color, `text_secondary` on hover.
- **Active state**: when a pinned search is selected, the card gets the same active highlight as `nav_button(active: true)` — stronger background, accent text.
- **No unread badge**: pinned searches are not live queries, so unread counts don't apply.
- **No icon**: the query string is the entire identity. No folder icon, no custom emoji.

### Section Header

The pinned searches section has no visible header when items exist — the visual distinction of the cards is enough. When the section is empty (no pinned searches), nothing is rendered — no placeholder, no empty state message.

If a "Clear all" affordance is needed, it can live in the command palette rather than taking up sidebar space.

### Scrolling Behavior

The pinned searches section is part of the sidebar's scrollable area. If the user accumulates many pinned searches, they scroll with the rest of the sidebar. There is no separate scroll region or fixed-position pinning — the section simply grows.

No cap on the number of pinned searches. If the sidebar fills up, that's a signal to the user to curate their list. This is intentional — it promotes engagement with the feature and encourages dismissing searches that are no longer relevant.

**Auto-creation risk:** Users who search heavily (the target audience) may accumulate pinned searches faster than they curate them, making the feature feel like clutter rather than support. The current design accepts this tradeoff. If post-launch data shows the list grows unmanageably, the first mitigation is auto-expiry: pinned searches older than N days (e.g., 14) that haven't been clicked since creation are silently removed. This preserves the zero-friction creation model while preventing indefinite accumulation. A harder intervention — requiring an explicit "pin" action — is a last resort because it undermines the feature's core value of automatic parking.

## Search Bar Interaction

When a pinned search is active (selected in the sidebar):

1. **Search bar** shows the stored query string, fully editable
2. **Staleness label** appears near the search bar: "Last updated 3 days ago". This is a subtle, non-intrusive label — not a banner or alert. It could sit below the search bar or right-aligned within it.
3. **Thread list** shows the stored threads with live metadata
4. **Editing the query** and executing updates the pinned search in place (no new entry)
5. **Pressing Escape** clears the search bar and returns to the previously active folder view, but does **not** dismiss the pinned search — it remains in the sidebar for later

### Interaction with Normal Search

When no pinned search is selected (the user is browsing a folder and starts searching):
- Typing and executing a search creates a new pinned search
- The new entry appears at the top of the sidebar and becomes the active selection

When a pinned search is selected and the user navigates to a folder:
- The pinned search is deselected (no longer active/highlighted)
- The search bar clears and shows the folder's thread list
- The pinned search remains in the sidebar

## Keyboard Interaction

No new keyboard shortcuts. Pinned searches are accessed by clicking in the sidebar, like any other sidebar item. The search bar's existing keyboard model (`/` to focus, Escape to clear, Enter to execute) applies unchanged.

## Persistence

Pinned searches persist across app restarts. They are stored in SQLite (main database or a separate local state table — implementation detail).

They do **not** sync across devices. Pinned searches are local working state, specific to the machine and session. Smart folders (which sync as part of account config) are the persistent, cross-device equivalent.

## Data Model (SQLite)

```sql
CREATE TABLE pinned_searches (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    query TEXT NOT NULL,
    created_at INTEGER NOT NULL,  -- unix timestamp
    updated_at INTEGER NOT NULL   -- unix timestamp
);

CREATE TABLE pinned_search_threads (
    pinned_search_id INTEGER NOT NULL REFERENCES pinned_searches(id) ON DELETE CASCADE,
    thread_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    PRIMARY KEY (pinned_search_id, thread_id, account_id)
);
```

Thread metadata is not stored — it's fetched live from the threads table when the pinned search is displayed.

## Resolved Questions

1. **Deduplication**: if the user runs the exact same query again after navigating away, the existing pinned search is updated (new timestamp, refreshed results) rather than creating a duplicate. Matching is by query string equality.
2. **Dismiss button visibility**: always visible. Faster to use, more discoverable.

## Open Questions

1. ~~**Entry display format**~~ **Resolved: date+time primary, truncated query as subtitle.** Each pinned search entry shows two lines: the date+time as the primary label (e.g., "Mar 19, 14:32") in normal text, and the query string as a muted subtitle truncated with ellipsis. Date+time only is too anonymous — users can't distinguish entries without clicking. Query only is unreadable at 180px. The two-line format gives enough context to scan while keeping entries compact. The full query is always accessible in the search bar when clicked.

## Ecosystem Patterns

How requirements in this spec map to patterns from the [iced ecosystem survey](../iced-ecosystem-survey.md).

### Requirements to Survey Matches

| Requirement | Primary Source | How It Applies |
|---|---|---|
| Card/chip styling | shadcn-rs + iced-plus tokens | Token palette for elevation, active state, text colors |
| Race on rapid navigation | bloom generational tracking | Generational counter for thread metadata queries — prevents stale results when the user clicks through pinned searches quickly |
| Edit-in-place state machine | bloom config shadow | Config shadowing inspires the approach: shadow query/results during edit, commit on execute, discard on Escape. Custom `navigated_away` flag needed beyond what bloom provides |
| Command palette integration | raffi query routing + trebuchet Component | Context-sensitive commands ("Save as Smart Folder" available only when a pinned search is active); Component events for the graduation-to-smart-folder flow |
| Escape key state restoration | feu raw keyboard | Raw keyboard interception to capture Escape before widget processing; actual restoration logic (return to previous folder view) is custom state management |
| Thread list with fixed ID set | shadcn-rs data table | Data table patterns for rendering the stored thread list (query is a simple `WHERE thread_id IN (...)`) |

### Gaps

- **Relative timestamps** ("Just now", "2 hours ago", "3 days ago"): No surveyed project implements human-friendly relative time formatting. Use `chrono-humanize` crate.
- **Tree rendering for hierarchical folders**: Noted as a broader sidebar gap — relevant here if pinned searches ever need nested grouping, but not a blocker for the current flat-list design.
