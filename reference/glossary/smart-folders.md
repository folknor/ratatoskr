# Smart Folders Glossary

A **smart folder** is a saved search query that lives in the sidebar permanently and re-evaluates its results every time it is opened. Smart folders are the persistent, live-re-evaluated form of a search; pinned searches are the temporary, snapshot form. The user creates a smart folder by graduating an active pinned search through the command palette ("Save Search"); see `reference/glossary/pinned-search.md` for the graduation flow.

If code disagrees with this document, the code is wrong.

## The Rule

A smart folder is a **query string plus presentation metadata**, not a thread set.

- The `smart_folders` row stores `name`, `query`, optional `icon`, `color`, `account_id`, `sort_order`, and an `is_default` marker. There is no thread list - clicking the folder runs the query against live data.
- Three default rows ship with the schema and reappear on any fresh DB: `sf-unread` (`is:unread`), `sf-attachments` (`has:attachment`), and `sf-starred-recent` (`is:starred after:-7`). All three have `account_id IS NULL` and `is_default = 1`.
- A smart folder is **not** a `folders` row. It rides through the sidebar as a `NavigationFolder` with `folder_kind: FolderKind::SmartFolder`, joined onto the navigation state alongside universal folders and label groups. See `reference/glossary/folders-labels.md` for the folder-vs-label rule that smart folders sit outside of.

## Storage

`crates/db/src/db/schema/07_smart.sql`:

```sql
CREATE TABLE IF NOT EXISTS smart_folders (
    id TEXT PRIMARY KEY,
    account_id TEXT,
    name TEXT NOT NULL,
    query TEXT NOT NULL,
    icon TEXT DEFAULT 'Search',
    color TEXT,
    sort_order INTEGER DEFAULT 0,
    is_default INTEGER DEFAULT 0,
    created_at INTEGER DEFAULT (unixepoch()),
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);
```

`account_id` is the only foreign key; deleting an account cascades its smart folders. The three default rows have `account_id IS NULL`, marking them as global.

## Code Layout

| Concern | Location |
|---|---|
| Schema + default rows | `crates/db/src/db/schema/07_smart.sql` |
| DB types (`DbSmartFolder`) | `crates/db/src/db/types.rs` |
| DB helpers (`db_get_smart_folders`, `db_get_smart_folder_by_id`, `db_insert_smart_folder_sync`, `db_update_smart_folder`, `db_delete_smart_folder`, `db_update_smart_folder_sort_order`) | `crates/db/src/db/queries_extra/filters_smart.rs` |
| Unread-count query | `crates/smart-folder/src/lib.rs` (`count_smart_folder_unread`) |
| Navigation builder (`build_smart_folders`, `query_all_smart_folders_sync`) | `crates/core/src/db/queries_extra/navigation.rs` |
| Service IPC handler | `crates/service/src/handlers/smart_folder.rs` |
| IPC wire types (`SmartFolderCreateParams`, `SmartFolderCreateAck`) | `crates/service-api/src/smart_folder.rs` |
| Sidebar rendering | `crates/app/src/ui/sidebar/smart.rs` |
| Click → search routing (`handle_smart_folder_selected`, `SearchIntent::SmartFolder`) | `crates/app/src/handlers/search.rs` |

## Default Smart Folders

The schema's `INSERT OR IGNORE` block seeds three rows so the section is non-empty out of the box:

| `id` | Name | Query | Icon |
|---|---|---|---|
| `sf-unread` | Unread | `is:unread` | `MailOpen` |
| `sf-attachments` | Has Attachments | `has:attachment` | `Paperclip` |
| `sf-starred-recent` | Starred This Week | `is:starred after:-7` | `Star` |

The `is_default = 1` flag exists but no code reads it today; it is a marker for future "reset to defaults" or "hide built-ins" affordances.

## Click → Search

The sidebar renders a smart folder as a nav button whose press emits `SidebarMessage::SelectSmartFolder { id, query }`. The handler in `sidebar/mod.rs` sets `SidebarSelection::SmartFolder { id }` and bubbles up as `SidebarEvent::SmartFolderSelected { id, query }`. The app's `handle_smart_folder_selected` then:

1. Clears pinned-search context (active selection, editing-in-place flag, staleness label).
2. Sets the search bar to the smart folder's query string.
3. Builds `SearchIntent::SmartFolder { id, query }` and runs it through `resolve_search_intent`.

The resolver routes `SmartFolder` through the same execution path as `AdHoc`, with one important difference in the `SearchScope`:

- `AdHoc` uses the current sidebar account scope.
- `SmartFolder` uses `SearchScope::QueryIntrinsic`: the scope is governed entirely by the query string itself. An `account:` operator in the saved query scopes the result; absence of one means all accounts.

The resolver also emits `SearchPinnedStateBehavior::SmartFolder { id }`, which clears `sidebar.active_pinned_search` and `editing_pinned_search` so the sidebar's pinned-search section deselects when the user navigates to a smart folder.

## Account Scope

Smart folders are **scope-exempt** in two distinct senses:

- **Listing**: `query_all_smart_folders_sync` is a flat `SELECT *` with no `WHERE`. The sidebar shows every smart folder regardless of the current account scope, including account-scoped folders that name a different account.
- **Unread count**: `build_smart_folders` calls `count_smart_folder_unread(conn, &sf.query, &AccountScope::All)` unconditionally. The badge reflects matching threads across the whole database, not just the currently-scoped account.

The `account_id` column is read into `NavigationFolder.account_id` and forwarded to the search execution as informational metadata, but no current code branches on it.

## Sort Order, Icon, Color

`sort_order` orders the list (`ORDER BY sort_order, created_at` in `query_all_smart_folders_sync`). `db_update_smart_folder_sort_order` accepts a batch of `(id, sort_order)` pairs. There is **no** sidebar UI affordance for reordering yet; the field is writable from code but never written by the user.

`icon` and `color` are stored but **not rendered**. The sidebar's smart-folder button passes `None` for the icon override and applies no color styling, so the schema defaults (icon `'Search'`, color NULL) and the three default rows' icon overrides are all unused at render time. Plumbing them through is a UI-side change with no schema or IPC work required.

## IPC Surface

Only one Service IPC method exists: `smart_folder.create`. It mints the UUID Service-side and inserts the row with the hardcoded defaults (icon `"search"`, no color, no account scope - matching the graduation flow's needs). Update, delete, and sort-order writes have async DB helpers but no IPC entry point yet, so the app cannot currently invoke them.

Any future "rename smart folder" / "delete smart folder" / "reorder smart folders" UI will need a corresponding IPC method added.

## What This Glossary Does Not Cover

- The search pipeline that executes a smart folder's query - see `docs/search/` and the resolver in `crates/app/src/handlers/search.rs`.
- The pinned-search side of the create flow (graduation) - see `reference/glossary/pinned-search.md`.
- The folder-vs-label rule that establishes smart folders as a third category - see `reference/glossary/folders-labels.md`.
