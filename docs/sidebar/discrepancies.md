# Sidebar: Spec vs Implementation Discrepancies

Audit date: 2026-03-21. Updated: 2026-03-21.
Compared `docs/sidebar/problem-statement.md` and `docs/sidebar/implementation-spec.md` against actual code.

---

## What matches the spec

### Phase 1A: Live Data Wiring -- COMPLETE

- **`NavigationLoaded` message variant** with generational counter (`u64`) exists in `crates/app/src/main.rs:139`. Stale-load rejection at line 487 matches the spec exactly.
- **`Sidebar` struct** uses `nav_state: Option<NavigationState>` instead of separate `labels` field, matching the spec's 1A.2 design (`crates/app/src/ui/sidebar.rs:54`).
- **`nav_items()`** reads from `NavigationState` and filters by `FolderKind::Universal`. Spam/All Mail filtering for unified view is correct.
- **`smart_folders()`** reads from `NavigationState`, filters by `FolderKind::SmartFolder`, displays real unread counts.
- **`labels()`** reads from `NavigationState`, filters by `FolderKind::AccountLabel`.
- **`current_scope()`** in `main.rs:2035` matches the spec verbatim.
- **`load_navigation()` async function** exists as a free function (`main.rs:2719`), calls `get_navigation_state`.
- **Scope dropdown** (Option A) is implemented with avatar-based entries per account (`scope_dropdown()`).
- **`view()` structure** matches: header (mode toggle + scope dropdown), pinned searches, compose button, nav items, smart folders, conditional labels section.
- **Labels only shown when scoped** -- `show_labels = self.selected_account.is_some()`.

### Phase 1B: Smart Folder Scoping Fix -- COMPLETE

- `build_smart_folders()` in `navigation.rs:153` takes `_scope: &AccountScope` (underscore-prefixed, ignored). It calls `query_all_smart_folders_sync(conn)` which queries all smart folders without scope filtering.
- Smart folder unread counts use `AccountScope::All` as mandated by the spec.

### Phase 1C: Unread Counts -- COMPLETE

- **Smart folder unread counts** are live via `ratatoskr_smart_folder::count_smart_folder_unread()` call in `navigation.rs:163`. No longer scaffolded as 0.
- **Per-label unread counts** implemented via `get_label_unread_counts()` batched GROUP BY query in `navigation.rs:267-294`. Matches the spec's SQL pattern.
- **Universal folder unread counts** are live from `get_unread_counts_by_folder()` in `scoped_queries.rs`.
- **Draft count includes local drafts** via `get_draft_count_with_local()` (navigation.rs:119).

### Phase 1D: Hierarchy Support -- COMPLETE

- **`NavigationFolder`** has `parent_id: Option<String>` and `label_semantics: Option<LabelSemantics>`. Matches spec 1D.1.
- **`LabelSemantics` enum** with `Tag` and `Folder` variants exists. Matches spec 1D.3.
- **`label_semantics_for_provider()`** maps `gmail_api` to `Tag`, everything else to `Folder`. Matches spec.
- **`build_account_labels()`** populates `parent_id` from `label.parent_label_id` with system folder filtering, and sets `label_semantics`. System-folder parent IDs are filtered to `None` to prevent orphaning.
- **Tree rendering** implemented: `tree_sort()`, `is_hidden_by_collapsed_ancestor()`, `render_label_tree()`, `render_flat_labels()`. Auto-detects hierarchy vs flat based on data.
- **`collapsed_folders: HashSet<String>`** state and `ToggleFolderExpand` message exist. Matches spec 1D.5.
- **Orphan recovery** in `tree_sort()` adds items with missing parents as depth-0 roots. Matches spec.
- **Chevron icon styling** uses `TextClass::Tertiary` as specified (spec 1D.5). FIXED.
- **O(n) HashMap in `is_hidden_by_collapsed_ancestor`** -- HashMap is now built once in `render_label_tree` and passed in, avoiding O(n^2) rebuilds. FIXED.

### Phase 1E: Pinned Searches -- COMPLETE

- **`pinned_searches: Vec<PinnedSearch>`** and `active_pinned_search: Option<i64>` on `Sidebar` struct.
- **`SelectPinnedSearch` / `DismissPinnedSearch`** messages and corresponding events exist.
- **Pinned search cards** rendered with query text (primary), relative timestamp (secondary), dismiss button. Matches spec 1E.4 layout order (query primary, date secondary). FIXED.
- **Relative time format** uses `format_relative_time()` producing "5 min ago", "2 hours ago", "3 days ago". Matches spec 1E.4. FIXED (was absolute "Mar 19, 14:32").
- **Positioning** matches: pinned searches appear above compose button in the view.
- **Magic number replaced** -- `PINNED_SEARCH_QUERY_MAX_CHARS` named constant used instead of raw `28`. FIXED.

### Phase 2: Strip Actions -- COMPLETE (no-op)

The sidebar contains no action affordances to strip:
- No inline label editing UI exists.
- No context menu handlers exist.
- No `is_system_label` guard exists (filtering is done in the backend's `build_account_labels`).
- The sidebar is already a pure read-only navigation surface: click to navigate, compose button, settings button, and pinned search dismiss (which curates sidebar content, not email actions).

The spec's Phase 2 prerequisites (Command Palette Slices 6a-6d) are complete, and there is nothing remaining to remove. Phase 2 is effectively satisfied.

### Component Trait -- COMPLETE

- `Sidebar` implements `Component` with `type Message = SidebarMessage` and `type Event = SidebarEvent`.
- `update()` returns `(Task<SidebarMessage>, Option<SidebarEvent>)` -- matches the trait signature.
- `view()` returns `Element<'_, SidebarMessage>`.

### Backend (core crate)

- **`get_navigation_state()`** returns `NavigationState` with universal folders (including Spam and All Mail), smart folders, and account labels -- all with live unread counts. Matches the spec's final state.
- **Spam and All Mail** are now included in `SIDEBAR_UNIVERSAL_FOLDERS` in the backend. The sidebar filters them out when in "All Accounts" mode, showing them only when scoped to a single account. Matches the spec. FIXED.
- **`AccountScope`** (`Single`/`Multiple`/`All`) is used throughout scoped queries. Matches spec.
- **Starred/Snoozed** use predicate-based queries (`is_starred`/`is_snoozed` flags), not label joins. Matches spec.
- **No raw SQL in the app crate** for sidebar state. All queries go through core functions.

---

## Divergences from spec (intentional)

### 1. Pinned search card styling

**Spec (1E.4):** Uses `theme::ButtonClass::Nav { active }` for the outer button.

**Code:** Uses `theme::ButtonClass::PinnedSearch { active }.style()`. This is a purpose-built style class rather than reusing the Nav class, which provides better visual distinction between pinned searches and navigation items. The spec notes pinned searches should be "visually distinct from folders/labels" and "card-like containers with elevated background, not nav-button style items" (spec section 1E), supporting this choice.

### 2. Pinned search event signature

**Spec (1E.3):** `PinnedSearchSelected(i64, String)` -- carries both ID and query string.

**Code:** `PinnedSearchSelected(i64)` -- carries only the ID. The parent `App` handles the query lookup separately. This is a reasonable simplification -- the event emitter does not need to carry redundant data the parent already has.

### 3. Pinned search card layout

**Spec (1E.5):** Pinned searches appear between scope dropdown and compose button.

**Code:** Pinned searches appear between the header row (mode toggle + scope dropdown) and compose button. Functionally equivalent but the header now includes the mode toggle button which was not in the original spec layout.

### 4. Calendar mode toggle button

**Spec/Problem statement:** Mentions a calendar toggle button in the sidebar header area.

**Code:** Implemented as `ToggleMode` message/event with `in_calendar_mode` state on the sidebar. The mode toggle button sits to the left of the scope dropdown. This matches the problem statement's description but was not detailed in the implementation spec phases.

### 5. Flat labels cap

**Spec (1D, existing labels section):** Takes up to 12 labels with `.take(12)`.

**Code:** `render_flat_labels()` uses `.take(12)` -- matches. But `render_label_tree()` has no cap -- it renders all tree nodes. This is intentional: tree views with expand/collapse don't need an artificial cap.

### 6. `SidebarEvent::CycleAccount` variant

The `CycleAccount` variant remains in `SidebarEvent` for API compatibility with `main.rs`, but is never emitted. The `SidebarMessage::CycleAccount` handler now directly updates state and emits `SidebarEvent::AccountSelected(next)` instead of using recursive `self.update()` (which made the old event unreachable). The parent's `CycleAccount` arm in `handle_sidebar_event` is dead code -- it maps to `Task::none()` -- and can be removed when `main.rs` is next refactored.

---

## What is missing (not yet implemented)

### 1. Scope persistence

Problem statement open question #6: scope does not survive app restart. `selected_account` is in-memory state, resets to `None` (All Accounts) on launch. This is documented as deferred.

### 2. NavigationTarget enum -- IMPLEMENTED

`NavigationTarget` enum is now defined in `command_dispatch.rs` with variants for all universal folders (Inbox, Starred, Sent, Drafts, Snoozed, Trash, Spam, AllMail), categories (Primary, Updates, Promotions, Social, Newsletters), Tasks, Attachments, SmartFolder, Label, Search, and PinnedSearch. `Message::NavigateTo(NavigationTarget)` is wired to `handle_navigate_to()` in `handlers/navigation.rs`, which updates sidebar selection, clears search/pinned search context, and loads threads. The `App` struct tracks `navigation_target: Option<NavigationTarget>` for view type derivation. The sidebar's `selected_label: Option<String>` is kept in sync via `NavigationTarget::to_label_id()` for backward compatibility with sidebar highlight logic.

### 3. Mixed drafts list view

Problem statement documents that clicking Drafts should show both server-synced draft threads and local-only drafts in a mixed view. The count path (`get_draft_count_with_local`) handles both sources. The list path (`get_draft_threads` in scoped_queries.rs) still only returns server-synced drafts.

**Blocked on design decision:** The problem statement proposes two options -- (a) a `DraftItem` enum with `ServerDraft(DbThread)` / `LocalDraft(DbLocalDraft)` variants, or (b) promoting local drafts to a `DbThread`-compatible shape at query time. Neither option has been specified. This affects the thread list renderer (outside sidebar ownership) and `scoped_queries.rs` (also outside sidebar ownership). The sidebar correctly displays the combined count; the list view fix requires a cross-cutting design decision.

---

## Cross-cutting concern status

### a. Generational load tracking

**Status: IMPLEMENTED.** `nav_generation: u64` in `App` (main.rs:245). Incremented on scope changes. `NavigationLoaded(g, _)` is rejected when `g != self.nav_generation`. Same pattern applied to `AccountsLoaded`, `ThreadsLoaded`, `PinnedSearchThreadIdsLoaded`, `PinnedSearchThreadsLoaded`. Matches the spec's "Generational Load Tracking" section exactly.

### b. Component trait

**Status: FULLY COMPONENTIZED.** `Sidebar` implements `Component` with `Message = SidebarMessage` and `Event = SidebarEvent`. The trait is defined in `crates/app/src/component.rs` with `update`, `view`, and optional `subscription` methods. All sidebar interactions flow through the component boundary: internal messages stay in `update()`, outward signals emit as events to the parent `App`.

### c. Token-to-Catalog theming

**Status: NAMED STYLE CLASSES USED THROUGHOUT.** No inline closures for styling. All style applications use named classes:
- `theme::ButtonClass::Ghost.style()`
- `theme::ContainerClass::Sidebar.style()`
- `theme::ButtonClass::PinnedSearch { active }.style()`
- `theme::ButtonClass::BareIcon.style()`
- `theme::ButtonClass::Nav { active }.style()`
- `theme::TextClass::Muted.style()`
- `theme::TextClass::Tertiary.style()` (chevron icons)
- Text styles use `fn(&Theme) -> Style` function pointers like `text::primary`, `text::secondary`, `text::base`.

No `|theme| { ... }` inline closures detected. Clean token-based theming.

### d. iced_drop drag-and-drop

**Status: NOT IMPLEMENTED.** No drag-and-drop in the sidebar. Not part of any current phase.

### e. Subscription orchestration

**Status: DEFAULT (NONE).** The `Sidebar` does not override the `subscription()` method from the `Component` trait, so it returns `Subscription::none()`. The sidebar is entirely demand-driven (data pushed in via `nav_state` updates from the parent). This is appropriate -- the sidebar has no independent background tasks.

### f. Core CRUD bypassed

**Status: NO BYPASS.** The sidebar app code (`sidebar.rs`) contains zero SQL queries or direct database access. All data flows through core crate functions.

### g. Dead code

**Status: CLEANED UP.**
- **`SidebarMessage::Noop`** -- REMOVED. Was never emitted.
- **`SidebarEvent::CycleAccount`** -- Retained for `main.rs` API compatibility but never emitted. The `CycleAccount` handler was fixed to avoid recursive `self.update()` and now directly updates state + emits `AccountSelected`. See divergence #6 above.
- **`truncate_query()`** -- `pub` because it is used from `main.rs`. Not dead code.
- **Spam/All Mail filter code** -- No longer dead code. Backend now includes these folders in `SIDEBAR_UNIVERSAL_FOLDERS`, so the sidebar's filter for `"SPAM" | "ALL_MAIL"` is active.
