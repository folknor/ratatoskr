# Contract #10: Scope State Unification

## Problem

The active scope — which account, shared mailbox, or public folder the user is viewing — is split across multiple independent fields with no single source of truth:

- `sidebar.selected_account: Option<usize>` — index into accounts vec
- `sidebar.selected_label: Option<String>` — folder/label ID
- `sidebar.selected_shared_mailbox: Option<String>` — mailbox ID
- `app.navigation_target: Option<NavigationTarget>` — view type enum (no shared mailbox or public folder variants)

`current_scope()` only reads `selected_account` and returns `AccountScope::Single` or `AccountScope::All`. Shared mailbox and public folder selection is completely ignored — selecting a shared mailbox in the sidebar fires an event that the app handles with `Task::none()`.

## Data Model Constraints

**Shared mailbox threads** are stored in the regular `threads` table with the parent account's `account_id`. There is no column distinguishing them from personal threads. The sync pipeline tracks which folders belong to which shared mailbox via `graph_shared_mailbox_delta_tokens` (Graph) and shared mailbox detection in `crates/jmap/src/sync/mod.rs` (JMAP). This distinction is lost at query time.

**Public folder items** are stored in a separate `public_folder_items` table with their own schema (id, folder_id, subject, sender, received_at, body_preview, is_read, item_class). They are NOT in the `threads` table. They are single messages (no threading), have no star/snooze/pin/mute flags, and `item_class` can be `IPM.Note`, `IPM.Post`, `IPM.Contact`, etc.

This means:
- `Scope::SharedMailbox` cannot filter the threads table today — it needs a new `shared_mailbox_id` column (see Phase 2)
- `Scope::PublicFolder` requires a completely different query path and display model

## Design

### Two-part state model

Scope is two-part: a **container** (which account/mailbox/folder) and a **destination** (which folder/label within that container). These are independent axes:

- **Container:** `ViewScope` — `AllAccounts`, `Account`, `SharedMailbox`, `PublicFolder`
- **Destination:** `selected_label` — Inbox, Sent, Trash, custom label, etc.

The invariant: `selected_label` is valid within `AllAccounts`, `Account`, and `SharedMailbox` scopes (each has their own folder namespace). For `PublicFolder`, `selected_label` must be `None` — the folder IS the destination. Scope transitions must clear `selected_label` when crossing container boundaries (shared mailbox folders don't share IDs with personal labels).

### New type: `ViewScope`

Defined in `crates/core/src/scope.rs` (NOT in the db crate — it's a routing concept, not a query type).

Performance note: `ViewScope` contains owned `String`s, so `.clone()` allocates. It's cloned on every `current_scope()` call (sidebar clicks, sync completions). Mitigate by passing `&ViewScope` where possible. If profiling shows this matters, intern the IDs or wrap in `Arc`. Not urgent — `AccountScope` has the same pattern today.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewScope {
    /// All personal accounts.
    AllAccounts,
    /// Single personal account by ID.
    Account(String),
    /// Shared mailbox, identified by (parent_account_id, mailbox_id).
    SharedMailbox { account_id: String, mailbox_id: String },
    /// Pinned public folder, identified by (parent_account_id, folder_id).
    PublicFolder { account_id: String, folder_id: String },
}
```

`AccountScope` survives unchanged as the internal query-layer type in `crates/db/src/db/types.rs`. The `ViewScope → AccountScope` translation is business logic in core. Query functions in `scoped_queries.rs` continue to accept `AccountScope` — they never see `ViewScope`.

Note: `AccountScope::Multiple(Vec<String>)` exists but has no current call sites. Verify during Phase 1 — if truly dead, remove it to avoid maintaining a mapping path that nothing uses.

### Sidebar state consolidation

Replace in `Sidebar`:
```rust
// Before
selected_account: Option<usize>,
selected_shared_mailbox: Option<String>,

// After
selected_scope: ViewScope,
```

Sidebar event payloads must change: `SelectSharedMailbox` and `SelectPublicFolder` currently emit a single `String`. They need to emit `(account_id, mailbox_id)` / `(account_id, folder_id)` tuples so `ViewScope` can be constructed without looking back into side tables.

### Query routing

`current_scope()` returns `ViewScope`. The call sites dispatch:

- `AllAccounts` / `Account(_)` → convert to `AccountScope`, call existing `get_threads_scoped()`. Add `WHERE shared_mailbox_id IS NULL` to exclude shared mailbox threads from personal counts.
- `SharedMailbox { .. }` → call new `get_threads_for_shared_mailbox()` (Phase 2)
- `PublicFolder { .. }` → call new `get_public_folder_items()` (Phase 3)

### Navigation routing

`fire_navigation_load()` also dispatches on `ViewScope`:

- `AllAccounts` / `Account(_)` → existing `get_navigation_state()` with `AccountScope`
- `SharedMailbox { .. }` → new `get_shared_mailbox_navigation()` returning the mailbox's folder tree
- `PublicFolder { .. }` → no sub-navigation (the folder is the terminal node)

`NavigationState.scope` currently stores `AccountScope`. This will need to accommodate the new scope types or be replaced with `ViewScope` in Phases 2/3.

### NavigationTarget

Add variants:
```rust
SharedMailbox { account_id: String, mailbox_id: String },
PublicFolder { account_id: String, folder_id: String },
```

Consider whether `NavigationTarget` should contain a `ViewScope` rather than duplicating the same fields — otherwise every new scope variant must be added in two places.

### Action service implications (Phase 2/3)

**Shared mailbox threads:** The action service currently takes `(account_id, thread_id)` and dispatches via the parent account's credentials. For shared mailboxes:
- Rights enforcement: `MailboxRightsInfo` exists in `NavigationFolder` but is never checked during action dispatch. Actions must check `may_delete`, `may_set_seen`, etc. before proceeding.
- Provider routing: Graph shared mailbox actions may require different API calls (acting on behalf of the shared mailbox). The `ViewScope` context must flow into the action layer.
- Undo tokens: Currently capture `(account_id, thread_id)`. If shared mailbox actions require different provider calls, undo needs the mailbox context too.

**Public folder items:** Most actions (star, snooze, archive, pin) are meaningless. Phase 3 must either:
- Gate actions: disable action buttons when scope is `PublicFolder` (using existing rights infrastructure)
- Or use a separate selection type that doesn't participate in action dispatch

### Compose identity

When composing from shared mailbox scope, the From identity should auto-select the shared mailbox address. `send_identity.rs` already has `shared_mailbox_id: Option<String>` in `SendIdentityContext`. The scope should feed this — downstream consumer, not part of this plan, but noting the dependency.

## Implementation Phases

### Phase 1: Type + sidebar state (preserve current behavior)

1. Define `ViewScope` in `crates/core/src/scope.rs`
2. Replace `selected_account` + `selected_shared_mailbox` with `selected_scope: ViewScope` in `Sidebar`
3. Update sidebar event payloads: `SelectSharedMailbox` and `SelectPublicFolder` must emit `(account_id, id)` tuples
4. Update all sidebar selection handlers to set `selected_scope` and clear `selected_label` on cross-container transitions
5. Build `SelectPublicFolder` handler from scratch (currently sets nothing — no state to migrate)
6. Update `current_scope()` to read `selected_scope`, map `AllAccounts`/`Account` to `AccountScope`
7. Update `update_thread_list_context_from_sidebar()` to dispatch on `ViewScope` (look up display names from `sidebar.shared_mailboxes` / `sidebar.pinned_public_folders`)
8. Update `reset_view_state()` to clear/set scope properly
9. Add `SharedMailbox`/`PublicFolder` variants to `NavigationTarget`
10. Update `handlers/navigation.rs` `handle_navigate_to()` for new variants
11. Update `command_dispatch.rs` `active_account_info()` (`command_dispatch.rs:268`) — currently derives active account and provider kind from `selected_account` directly. Must dispatch on `ViewScope` instead: `SharedMailbox` → parent account's provider kind, `PublicFolder` → parent account's provider kind, `AllAccounts` → None. Without this, the command palette misclassifies provider capabilities and account-scoped command availability in non-account scopes.
12. Update search handlers: `search.rs:560` scope fallback hardcodes `AccountScope::All`, and "widen search scope" (`main.rs:1513`) mutates `selected_account` directly. Both must read/write `selected_scope` instead. Decide: when scoped to a shared mailbox and the user searches, does search scope to the mailbox or go global? (Recommend: search within the current scope by default, with "widen" expanding to `AllAccounts`.)
13. **Do NOT wire `SharedMailboxSelected`/`PublicFolderSelected` to thread loading yet** — keep `Task::none()` until Phases 2/3. Phase 1 is a representation change only.

**Verification:** `cargo check --workspace`. Sidebar account switching and label navigation behave identically. Shared mailbox / public folder selection still does nothing (intentionally).

### Phase 2: Shared mailbox thread loading

**Schema change (Option B — schema column):**

1. Migration: `ALTER TABLE threads ADD COLUMN shared_mailbox_id TEXT`
2. Migration: `CREATE INDEX idx_threads_shared_mailbox ON threads(account_id, shared_mailbox_id, last_message_at DESC)`
3. Update shared mailbox sync to populate `shared_mailbox_id` on insert — both Graph (`crates/graph/src/shared_mailbox_sync.rs`) and JMAP (`crates/jmap/src/sync/mod.rs`) paths

**Backfill strategy:** The column defaults to NULL for existing rows. Backfill runs as a post-migration fixup during the next shared mailbox sync (not inside the migration itself, to keep it fast). On first sync after upgrade, `shared_mailbox_id` is populated for all threads belonging to each shared mailbox. For Graph: join `thread_labels` against `graph_shared_mailbox_delta_tokens` folder IDs. For JMAP: similar join against JMAP shared mailbox folder state. Edge case: if delta tokens have been cleared (forced resync), the backfill misses those threads — a full resync resolves it.

**Data integrity invariant:** A thread's `shared_mailbox_id` is either NULL (personal) or a single mailbox ID. A single TEXT column (not a join table) is sufficient because:
- Each shared mailbox syncs its own copies of messages — the provider assigns distinct message/thread IDs per mailbox. A thread in shared mailbox A and the "same" conversation in shared mailbox B are separate rows in `threads` with different provider-assigned IDs.
- If a user is CC'd on a conversation that also exists in a shared mailbox they delegate, the personal copy and shared copy are separate thread rows with different IDs.
- A sync bug that erroneously stamps a personal thread with a `shared_mailbox_id` would cause it to disappear from personal views (if queries use `WHERE shared_mailbox_id IS NULL`). Guard: the sync pipeline should only set `shared_mailbox_id` when it is explicitly syncing a shared mailbox context — never during personal account sync.

**Query changes:**

4. New `get_threads_for_shared_mailbox(conn, account_id, mailbox_id, label_id, limit)` — must NOT reuse `LATEST_MESSAGE_SUBQUERY` as-is (it scans the entire messages table with no WHERE clause). Use a CTE that pre-filters thread IDs by `shared_mailbox_id`, then scope the messages subquery to those threads.
5. Personal-account queries: add `WHERE shared_mailbox_id IS NULL` to `get_threads_scoped()` and all navigation unread count queries (`get_unread_counts_by_folder`, `get_label_unread_counts`, `get_draft_count_with_local`, `build_all_account_tags`, flag-based Starred/Snoozed counts) to exclude shared mailbox threads from personal counts.
6. New `get_shared_mailbox_navigation(conn, account_id, mailbox_id)` returning the mailbox's folder list with unread counts scoped to `shared_mailbox_id = ?`

**App wiring:**

7. Wire `SharedMailboxSelected` event → `reset_view_state()` → thread loading via new query
8. Wire navigation loading for shared mailbox scope

**Action service (minimum viable):**

9. Gate shared mailbox actions behind rights checks from `MailboxRightsInfo`. At minimum, check `may_delete`/`may_set_seen` before dispatching archive/trash/mark-read. If rights data is unavailable, allow actions optimistically (provider will reject if unauthorized, pending-ops handles retry).

**Verification:** Select a shared mailbox → thread list shows only that mailbox's threads. Personal account view excludes shared mailbox threads. Unread counts are correct in both scopes. Verify that `get_thread_detail()` does NOT filter by `shared_mailbox_id IS NULL` — clicking a shared mailbox thread in the list must still load its detail. Thread detail takes `(account_id, thread_id)` and should work unchanged, but confirm.

### Phase 3: Public folder item loading

Public folder items are fundamentally different from threads — single messages, no threading, no star/snooze/pin/mute, variable `item_class`. The thread list needs a display abstraction.

**Display model:**

1. Define a `ThreadListItem` enum at the app display layer:
```rust
enum ThreadListItem {
    Thread(Thread),
    PublicFolderItem(PublicFolderItem),
}
```
2. Thread list stores `Vec<ThreadListItem>` instead of `Vec<Thread>`
3. `thread_card()` rendering dispatches on the enum — one branch per variant per frame (negligible cost)
4. Selection of a `PublicFolderItem` loads a simplified reading pane (body_preview, no message expansion, no action buttons) or is explicitly unsupported in the first cut

**Query:**

5. New `get_public_folder_items(conn, account_id, folder_id, limit)` querying `public_folder_items`
6. Must filter on BOTH `account_id` AND `folder_id` (folder IDs are provider-assigned opaque strings that could collide across accounts)

**App wiring:**

7. Wire `PublicFolderSelected` event → `reset_view_state()` (clearing `selected_label` to None) → item loading
8. Navigation: no sub-navigation for public folders

**Action gating:**

9. Disable all email actions (archive, star, trash, move, etc.) when selection is a `PublicFolderItem`. Either grey out action buttons or filter commands from the palette via `CommandContext`.

**Verification:** Click a pinned public folder → items appear in thread list. No action buttons enabled. Switching back to an account scope restores normal behavior.

## Files to Modify

### Phase 1 (type + state)
- `crates/core/src/scope.rs` *(new)* — define `ViewScope`
- `crates/core/src/lib.rs` — re-export scope module
- `crates/app/src/ui/sidebar.rs` — replace scope fields, update selection handlers, fix event payloads
- `crates/app/src/main.rs` — update `current_scope()`, `update_thread_list_context_from_sidebar()`, `reset_view_state()`, keep `Task::none()` for new scope events
- `crates/app/src/command_dispatch.rs` — add `NavigationTarget` variants, update `active_account_info()`
- `crates/app/src/handlers/navigation.rs` — update `handle_navigate_to()` for new variants
- `crates/app/src/handlers/search.rs` — migrate scope references from `selected_account` to `selected_scope`

### Phase 2 (shared mailbox threads)
- `crates/db/src/db/migrations.rs` — add `shared_mailbox_id` column + index
- `crates/graph/src/shared_mailbox_sync.rs` — populate column during sync
- `crates/graph/src/sync/persistence.rs` — pass `shared_mailbox_id` to store function
- `crates/jmap/src/sync/mod.rs` — populate column for JMAP shared mailboxes
- `crates/core/src/db/queries_extra/scoped_queries.rs` — new shared mailbox query, add `shared_mailbox_id IS NULL` to personal queries
- `crates/core/src/db/queries_extra/navigation.rs` — new `get_shared_mailbox_navigation()`, scope unread counts by `shared_mailbox_id`
- `crates/core/src/db/sql_fragments.rs` — scoped variant of `LATEST_MESSAGE_SUBQUERY` for shared mailbox context
- `crates/app/src/main.rs` — wire shared mailbox event → thread loading

### Phase 3 (public folder items)
- `crates/app/src/db/types.rs` — define `ThreadListItem` enum
- `crates/core/src/db/queries_extra/` — new public folder query function
- `crates/app/src/main.rs` — thread loading dispatch for public folder scope
- `crates/app/src/ui/thread_list.rs` — adapt to render `ThreadListItem` enum
- `crates/app/src/command_dispatch.rs` — disable actions for public folder items

## Risks

- **Phase 1** is low risk — representation change only, no behavioral change to existing query paths. Shared mailbox / public folder selection remains a no-op until Phases 2/3.
- **Phase 2** migration is straightforward (`ALTER TABLE ADD COLUMN` + index). Backfill runs lazily during next sync, not in the migration. Main risk: `shared_mailbox_id IS NULL` filter must be added to ALL personal-account query paths — missing any one silently mixes shared mailbox threads into personal counts. The `LATEST_MESSAGE_SUBQUERY` full table scan must not be carried into the new shared mailbox query.
- **Phase 3** requires a display abstraction (`ThreadListItem` enum) that touches the thread list widget — the app's main rendering bottleneck (no virtualization, 1000-item tree). The enum match is negligible cost per frame, but this is the right time to also consider whether scroll virtualization should land alongside or before Phase 3.
- **Generation counters** must guard all new load paths. `reset_view_state()` already bumps `nav_generation` and `thread_generation` on every scope change. The new async query tasks for shared mailbox and public folder loading must capture and validate these same counters — this is exactly the convention-based gap that contract #15 warns about.
- **Stale public folder data:** `public_folder_items` and `public_folder_pins` tables lack CASCADE foreign keys on `account_id` (contract #20). Queries must defensively join against active accounts, or fix the schema first.
