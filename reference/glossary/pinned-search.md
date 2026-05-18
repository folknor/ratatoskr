# Pinned Searches Glossary

A **pinned search** is a query string plus a frozen snapshot of the threads it matched. Every search the user runs from the search bar becomes one - there is no other kind of search. Pinned searches appear at the top of the sidebar and stay there until the user dismisses them, graduates them to a smart folder, or 14 days pass without the user touching them (auto-expiry). There is no cap on how many can exist at once; auto-expiry is the only thing that removes them without the user asking. Graduation is the path to a persistent equivalent: a smart folder saves the query and re-evaluates it live.

If code disagrees with this document, the code is wrong.

## The Rule

A pinned search is an **ephemeral, local, query-keyed snapshot of thread IDs**, not a saved query.

- The snapshot determines *which* threads appear. The live `threads` table determines *how* they look (read/unread, starred, snippet, message count). Thread metadata is never copied into the snapshot.
- Pinned searches are **local state**, like `thread_ui_state`. They do not sync across devices. Smart folders are the cross-device equivalent.
- A pinned search is **query-keyed**. Running the same query string a second time updates the existing row; it does not create a duplicate. The unique constraint is on `query`, not `(query, scope_account_id)`.
- A pinned search is the outer concept; a search is an event in its lifecycle. The `SearchIntent` resolver in the search bar classifies each search as one of four events (new, in-place edit, activation, refresh) and emits the matching write.

## Storage

`crates/db/src/db/schema/07_smart.sql` defines two tables in the main `ratatoskr.db`:

- `pinned_searches(id, query, created_at, updated_at, scope_account_id)` - one row per snapshot. `scope_account_id` is NULL for cross-account searches and records the sidebar scope at execution time so refresh hits the same account set. `UNIQUE(query)` enforces dedup.
- `pinned_search_threads(pinned_search_id, thread_id, account_id)` - the snapshot. `ON DELETE CASCADE` from the parent. Not a foreign key into `threads` - threads can be deleted by sync while a pinned search persists; the join simply returns fewer rows.

Migrations land in the v100 baseline; there is no separate pinned-search migration version.

## Code Layout

| Concern | Location |
|---|---|
| Schema | `crates/db/src/db/schema/07_smart.sql` |
| DB-side sync helpers (`db_create_pinned_search_sync`, `db_update_pinned_search_sync`, `db_delete_pinned_search_sync`, `db_delete_all_pinned_searches_sync`, `db_expire_stale_pinned_searches_sync`, `db_list_pinned_searches`, `db_get_pinned_search_thread_ids`) | `crates/db/src/db/pinned_searches.rs` |
| Service IPC handlers (`pinned_search.create_or_update`, `pinned_search.update`, `pinned_search.delete`, `pinned_search.delete_all`, `pinned_search.kick`) | `crates/service/src/handlers/pinned_search.rs` |
| IPC wire types | `crates/service-api/src/pinned_search.rs` |
| App-side `PinnedSearch` type and client wrappers | `crates/app/src/db/pinned_searches.rs` |
| Lifecycle resolver + handlers (`resolve_search_intent`, `handle_search_result_landed`, `apply_search_pinned_state`, `clear_pinned_search_context`, `handle_smart_folder_saved`, `handle_dismiss_pinned_search`) | `crates/app/src/handlers/search.rs` |
| Sidebar rendering (`pinned_searches_section`, `pinned_search_card`, `format_relative_time`, `is_results_stale`) | `crates/app/src/ui/sidebar/pinned_searches.rs` |
| Search bar staleness label (`pinned_search_updated_at` on `ThreadList`) | `crates/app/src/ui/thread_list.rs` |
| Palette entries (`CommandId::SmartFolderSave`, `CommandId::PinnedSearchesClearAll`) | `crates/cmdk/src/registry/smart_folders.rs` |

All writes go through the Service IPC layer (`WriterPool` / `WriteDbState`). The app crate does not hold a writable connection of its own for pinned searches.

## State Flags

Two fields on the app track which pinned search is which:

- `App.sidebar.active_pinned_search: Option<i64>` - the pinned search the user has selected in the sidebar. Drives card highlighting, the staleness label under the search bar, and the `CommandContext.active_pinned_search` gate on Save-as-Smart-Folder.
- `App.editing_pinned_search: Option<i64>` - the pinned search that the next search should overwrite instead of creating a new row. `clear_pinned_search_context` clears it on any navigate-away.

They usually hold the same value. They diverge briefly during transitions: clicking a folder clears `editing_pinned_search` immediately (so a search typed afterwards creates a new pinned search), then the navigation step that follows clears `active_pinned_search`.

A third field, `CommandContext.has_pinned_searches: bool`, exists only to gate "Clear All Pinned Searches" in the palette. It cannot be replaced by `active_pinned_search.is_some()` because the user may have many pinned searches without one being selected.

## Lifecycle (SearchIntent)

`SearchIntent` in `crates/app/src/handlers/search.rs` lists the four events that the search bar can produce:

- `AdHoc { query, scope }` - the user typed and executed a search. Resolves to `CreatePinnedSnapshot` when `editing_pinned_search` is `None`, or `UpdatePinnedSnapshot` when it is `Some` (the user is editing in place).
- `PinnedActivation { id }` - the user clicked an existing pinned search. Resolves to no persistence write; the snapshot is read, not rewritten.
- `PinnedRefresh { id }` - the user re-executed the query on the active pinned search. Resolves to `RefreshPinnedSnapshot`.
- `SmartFolder { id, query }` - the user clicked a smart folder. Not itself a pinned-search action, but the resolver routes it through the same code path and emits `SearchPinnedStateBehavior::Clear` so the active pinned search is deselected.

Each `SearchIntent` resolves up front to a `SearchCompletionBehavior`, which bundles three side-effects: `persistence` (the write to issue, if any), `pinned_state` (what to set `active_pinned_search` and `editing_pinned_search` to), and `post_success` (whether to reload the sidebar list with `RefreshPinnedSearchList` after the IPC completes).

There are no `SearchExecuted` or `PinnedSearchSaved` message variants. An earlier design that proposed them was replaced by the resolver before this code landed.

## Graduation to Smart Folder

`CommandId::SmartFolderSave` is gated on `CommandContext.active_pinned_search.is_some()` - never on raw search-bar contents. On successful save, `handle_smart_folder_saved` takes `editing_pinned_search` and routes through `handle_dismiss_pinned_search`; the dismiss-ack handler clears local state and calls `restore_folder_view`. A parallel `fire_navigation_load` surfaces the new smart folder in the sidebar.

The pinned search is always deleted on graduation. There is no "keep both" option.

## Auto-Expiry

Pinned searches older than 14 days that have never been touched (`updated_at = created_at`) are silently removed. Any user interaction - click, refresh, edit - bumps `updated_at` and exempts the row forever.

The trigger is the Service-side `pinned_search.kick` notification (`Drop`-class self-heal). There is no app-side `expiry_ran` boot guard - the Service decides the cadence.

## Refresh Button

Each pinned search card shows a refresh button when `is_results_stale(updated_at)` returns true (`updated_at` is at least an hour old; see `crates/app/src/ui/sidebar/pinned_searches.rs`). Clicking it issues a `PinnedRefresh` intent, which resolves to `RefreshPinnedSnapshot` and rewrites the thread ID set. After a refresh the button is hidden for at least an hour because `updated_at` is now "just now".

The sidebar does not re-render on a timer, so the button does not pop back in at the one-hour mark on its own; it appears the next time the sidebar redraws for any other reason.

## Sidebar Placement

The sidebar has a fixed header (account scope dropdown, calendar mode toggle, and compose button) and a scrollable content area below it. Pinned searches occupy the top of the scrollable area, above chats, universal folders, smart folders, and labels.

Each pinned search renders as a card with a slightly lighter background than the rest of the sidebar (one palette step up) so it reads as temporary rather than as a permanent destination. Cards carry no unread badge and no icon - they are labeled by the query string alone, with a relative time and an account-scope tag on the second line.

When there are no pinned searches, the section renders nothing - no header, no placeholder.

## Account Scope

Every pinned search records the sidebar's account scope at write time in the `scope_account_id` column. NULL means the search ran in the **All Accounts** scope; a non-NULL value names a single account. Shared-mailbox and public-folder scopes collapse to their owning account when recorded.

The sidebar lists **every** pinned search regardless of the current account scope. The scope dropdown does not filter the pinned-search section. Instead, each card shows its scope as a second-line tag - "12 min ago • Acme Mail" or "12 min ago • All Accounts" - so users can tell at a glance which account a card came from.

The stored scope drives refresh, not activation:

- **Activation** (clicking a card) reads the stored thread-ID snapshot. The current sidebar scope is irrelevant; the snapshot is by ID, not by query.
- **Refresh** re-runs the query against the **stored** scope on the row, not against the current sidebar scope.
- **In-place edit** (editing the query while a pinned search is active) overwrites the row with the **current** sidebar scope. Editing a single-account pinned search while scoped to All Accounts widens the row to All Accounts.

Activating a pinned search does not change the sidebar's scope dropdown. The user may be viewing an All Accounts pinned search while the sidebar is scoped to Acme Mail; that is intentional.

## What This Glossary Does Not Cover

- The general search pipeline (query parsing, ranking, the `search()` function in `rtsk::search`) - see `docs/search/`.
- Smart folders, which are persistent queries with no thread-ID snapshot. See `crates/db/src/db/schema/07_smart.sql` for the `smart_folders` table.
- The Service IPC framing (`WriterPool`, `WriteDbState`, the Phase 6a refactor) - see `reference/architecture.md`.
