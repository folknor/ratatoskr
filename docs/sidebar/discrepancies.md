# Sidebar: Spec vs Implementation Discrepancies

Audit date: 2026-03-21. Compared `docs/sidebar/problem-statement.md` and `docs/sidebar/implementation-spec.md` against actual code.

---

## What matches the spec

### Phase 1A: Live Data Wiring -- COMPLETE

- **`NavigationLoaded` message variant** with generational counter (`u64`) exists in `crates/app/src/main.rs:139`. Stale-load rejection at line 487 matches the spec exactly.
- **`Sidebar` struct** uses `nav_state: Option<NavigationState>` instead of separate `labels` field, matching the spec's 1A.2 design (`crates/app/src/ui/sidebar.rs:54`).
- **`nav_items()`** reads from `NavigationState` and filters by `FolderKind::Universal`. Spam/All Mail filtering for unified view is correct (lines 281-288).
- **`smart_folders()`** reads from `NavigationState`, filters by `FolderKind::SmartFolder`, displays real unread counts (lines 303-331).
- **`labels()`** reads from `NavigationState`, filters by `FolderKind::AccountLabel` (lines 333-359).
- **`current_scope()`** in `main.rs:2035` matches the spec verbatim.
- **`load_navigation()` async function** exists as a free function (`main.rs:2719`), calls `get_navigation_state`.
- **Scope dropdown** (Option A) is implemented with avatar-based entries per account (`scope_dropdown()`, lines 230-267).
- **`view()` structure** matches: header (mode toggle + scope dropdown), pinned searches, compose button, nav items, smart folders, conditional labels section.
- **Labels only shown when scoped** -- `show_labels = self.selected_account.is_some()` at line 168.

### Phase 1B: Smart Folder Scoping Fix -- COMPLETE

- `build_smart_folders()` in `navigation.rs:153` takes `_scope: &AccountScope` (underscore-prefixed, ignored). It calls `query_all_smart_folders_sync(conn)` which queries all smart folders without scope filtering (line 192).
- Smart folder unread counts use `AccountScope::All` as mandated by the spec (line 166).

### Phase 1C: Unread Counts -- COMPLETE

- **Smart folder unread counts** are live via `ratatoskr_smart_folder::count_smart_folder_unread()` call in `navigation.rs:163`. No longer scaffolded as 0.
- **Per-label unread counts** implemented via `get_label_unread_counts()` batched GROUP BY query in `navigation.rs:267-294`. Matches the spec's SQL pattern.
- **Universal folder unread counts** are live from `get_unread_counts_by_folder()` in `scoped_queries.rs`.
- **Draft count includes local drafts** via `get_draft_count_with_local()` (navigation.rs:119).

### Phase 1D: Hierarchy Support -- COMPLETE

- **`NavigationFolder`** has `parent_id: Option<String>` and `label_semantics: Option<LabelSemantics>` (navigation.rs:39-48). Matches spec 1D.1.
- **`LabelSemantics` enum** with `Tag` and `Folder` variants exists (navigation.rs:28-34). Matches spec 1D.3.
- **`label_semantics_for_provider()`** maps `gmail_api` to `Tag`, everything else to `Folder` (navigation.rs:244-249). Matches spec.
- **`build_account_labels()`** populates `parent_id` from `label.parent_label_id` with system folder filtering, and sets `label_semantics` (navigation.rs:201-241). System-folder parent IDs are filtered to `None` to prevent orphaning (line 226).
- **Tree rendering** implemented: `tree_sort()`, `is_hidden_by_collapsed_ancestor()`, `render_label_tree()`, `render_flat_labels()` (sidebar.rs:446-642). Auto-detects hierarchy vs flat based on data.
- **`collapsed_folders: HashSet<String>`** state and `ToggleFolderExpand` message exist. Matches spec 1D.5.
- **Orphan recovery** in `tree_sort()` adds items with missing parents as depth-0 roots (lines 499-509). Matches spec.

### Phase 1E: Pinned Searches -- COMPLETE

- **`pinned_searches: Vec<PinnedSearch>`** and `active_pinned_search: Option<i64>` on `Sidebar` struct (lines 63-65).
- **`SelectPinnedSearch` / `DismissPinnedSearch`** messages and corresponding events exist.
- **Pinned search cards** rendered with query text, timestamp, dismiss button (lines 362-428).
- **Positioning** matches: pinned searches appear above compose button in the view (line 198).

### Component Trait -- COMPLETE

- `Sidebar` implements `Component` with `type Message = SidebarMessage` and `type Event = SidebarEvent` (sidebar.rs:94-97).
- `update()` returns `(Task<SidebarMessage>, Option<SidebarEvent>)` -- matches the trait signature.
- `view()` returns `Element<'_, SidebarMessage>`.

### Backend (core crate)

- **`get_navigation_state()`** returns `NavigationState` with universal folders, smart folders, and account labels -- all with live unread counts. Matches the spec's final state.
- **`AccountScope`** (`Single`/`Multiple`/`All`) is used throughout scoped queries. Matches spec.
- **Starred/Snoozed** use predicate-based queries (`is_starred`/`is_snoozed` flags), not label joins. Matches spec.
- **No raw SQL in the app crate** for sidebar state. All queries go through core functions.

---

## Divergences from spec

### 1. Pinned search date formatting

**Spec (1E.4):** Uses `format_relative_time()` producing strings like "2 hours ago", "3 days ago".

**Code:** Uses `format_pinned_search_date()` producing absolute timestamps like "Mar 19, 14:32" (sidebar.rs:431-435). This is a deliberate design change -- the problem statement's ASCII art also shows "2 hours ago" / "3 days ago" but the implementation chose absolute dates.

### 2. Pinned search event signature

**Spec (1E.3):** `PinnedSearchSelected(i64, String)` -- carries both ID and query string.

**Code:** `PinnedSearchSelected(i64)` -- carries only the ID (sidebar.rs:45). The parent `App` handles the query lookup separately. This is a reasonable simplification -- the event emitter does not need to carry redundant data the parent already has.

### 3. Pinned search card styling

**Spec (1E.4):** Uses `theme::ContainerClass::Elevated.style()` for card background and `theme::ButtonClass::Nav { active }` for the outer button.

**Code:** Uses `theme::ButtonClass::PinnedSearch { active }.style()` (sidebar.rs:425). This is a purpose-built style class rather than reusing the Nav class, which is arguably better design.

### 4. Pinned search card layout

**Spec (1E.5):** Pinned searches appear between scope dropdown and compose button.

**Code:** Pinned searches appear between the header row (mode toggle + scope dropdown) and compose button (sidebar.rs:197-200). Functionally equivalent but the header now includes the mode toggle button which was not in the original spec layout.

### 5. Calendar mode toggle button

**Spec/Problem statement:** Mentions a calendar toggle button in the sidebar header area.

**Code:** Implemented as `ToggleMode` message/event with `in_calendar_mode` state on the sidebar (sidebar.rs:67, 160-162). The mode toggle button sits to the left of the scope dropdown (line 184). This matches the problem statement's description but was not detailed in the implementation spec phases.

### 6. Chevron icon styling in tree view

**Spec (1D.5):** `chevron.size(ICON_XS).style(theme::TextClass::Tertiary.style())` -- uses Tertiary text style.

**Code:** `chevron.size(ICON_XS)` with no explicit text style on the icon (sidebar.rs:578). Minor visual difference.

### 7. Flat labels cap

**Spec (1D, existing labels section):** Takes up to 12 labels with `.take(12)`.

**Code:** `render_flat_labels()` uses `.take(12)` (sidebar.rs:631) -- matches. But `render_label_tree()` has no cap -- it renders all tree nodes. This is intentional: tree views with expand/collapse don't need an artificial cap.

---

## What is missing (not yet implemented)

### 1. Phase 2: Strip Actions -- NOT STARTED

Per spec, this is blocked on Command Palette Slice 6 (app integration). No label editing, context menus, or inline modals exist in the sidebar currently, so there is nothing to strip. The `is_system_label` guard mentioned in the spec as Phase 2 removal target does not exist in the code (already removed or never added to the app crate).

### 2. Scope persistence

Problem statement open question #6: scope does not survive app restart. `selected_account` is in-memory state, resets to `None` (All Accounts) on launch. This is documented as deferred.

### 3. NavigationTarget enum

Spec 1A transitional note acknowledges that `selected_label: Option<String>` is semantically muddy -- universal folders, smart folders, and account labels all share one `Option<String>`. The spec defers this to a future `NavigationTarget` enum. Still deferred; `selected_label` remains the flat marker.

### 4. Mixed drafts list view

Problem statement documents that clicking Drafts should show both server-synced draft threads and local-only drafts in a mixed view. The count path (`get_draft_count_with_local`) handles both sources. The list path (`get_draft_threads` in scoped_queries.rs:370) still only returns server-synced drafts, with a doc comment acknowledging this limitation (line 366-369).

---

## Cross-cutting concern status

### a. Generational load tracking

**Status: IMPLEMENTED.** `nav_generation: u64` in `App` (main.rs:245). Incremented on scope changes (lines 667, 1711, 1720, 1751, 2014, 2318). `NavigationLoaded(g, _)` is rejected when `g != self.nav_generation` (line 487). Same pattern applied to `AccountsLoaded`, `ThreadsLoaded`, `PinnedSearchThreadIdsLoaded`, `PinnedSearchThreadsLoaded`. Matches the spec's "Generational Load Tracking" section exactly.

### b. Component trait

**Status: FULLY COMPONENTIZED.** `Sidebar` implements `Component` with `Message = SidebarMessage` and `Event = SidebarEvent` (sidebar.rs:94-97). The trait is defined in `crates/app/src/component.rs` with `update`, `view`, and optional `subscription` methods. All sidebar interactions flow through the component boundary: internal messages stay in `update()`, outward signals emit as events to the parent `App`.

### c. Token-to-Catalog theming

**Status: NAMED STYLE CLASSES USED THROUGHOUT.** No inline closures for styling. All style applications use named classes:
- `theme::ButtonClass::Ghost.style()` (line 182)
- `theme::ContainerClass::Sidebar.style()` (line 223)
- `theme::ButtonClass::PinnedSearch { active }.style()` (line 425)
- `theme::ButtonClass::BareIcon.style()` (line 414)
- `theme::ButtonClass::Nav { active }.style()` (line 614)
- `theme::TextClass::Muted.style()` (lines 393, 408)
- Text styles use `fn(&Theme) -> Style` function pointers like `text::primary`, `text::secondary`, `text::base` (lines 385-393, 590-593).

No `|theme| { ... }` inline closures detected. Clean token-based theming.

### d. iced_drop drag-and-drop

**Status: NOT IMPLEMENTED.** No drag-and-drop in the sidebar. No `iced_drop`, `Droppable`, or drag-related code. The spec mentions this as a future capability ("if we ever support it") and the ecosystem cross-reference lists it as a possible pattern. Not part of any current phase.

### e. Subscription orchestration

**Status: DEFAULT (NONE).** The `Sidebar` does not override the `subscription()` method from the `Component` trait, so it returns `Subscription::none()`. The sidebar is entirely demand-driven (data pushed in via `nav_state` updates from the parent), not event-stream-driven. This is appropriate -- the sidebar has no independent background tasks.

### f. Core CRUD bypassed

**Status: NO BYPASS.** The sidebar app code (`sidebar.rs`) contains zero SQL queries or direct database access. All data flows through core crate functions: `get_navigation_state()`, `load_navigation()`, `get_threads_scoped()`, etc. The `Sidebar` struct receives data via its `nav_state` field, which is populated by the parent `App` after calling core functions.

### g. Dead code

**Status: MINOR ITEMS.**
- **`SidebarMessage::Noop`** (sidebar.rs:33) -- exists as a message variant, handler returns `(Task::none(), None)`. Could be dead if nothing emits it. No `Noop` message emission found in sidebar.rs.
- **`SidebarEvent::CycleAccount`** (sidebar.rs:41) -- emitted by `SidebarMessage::CycleAccount` handler. The parent handles it with `Task::none()` (main.rs:1725) -- a no-op handler. The event exists for keyboard shortcut dispatch via the command palette but the parent does not act on it (the sidebar internally handles the account cycling in its `update()` and emits `AccountSelected` instead via recursive `self.update()`). The `CycleAccount` event is effectively unreachable because `SidebarMessage::CycleAccount` recursively calls `self.update(SidebarMessage::SelectAccount(next))`, which emits `AccountSelected`, not `CycleAccount`. The `CycleAccount` arm in `update()` never reaches the outer `(Task::none(), Some(SidebarEvent::CycleAccount))` return because the recursive call returns first.
- **`truncate_query()`** (sidebar.rs:438) is `pub` but only used internally. Could be `pub(crate)` or private.
