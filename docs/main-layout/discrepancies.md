# Main Layout: Spec vs Implementation Discrepancies

Audit date: 2026-03-21. Compared `problem-statement.md`, `implementation-spec.md`, and `iced-implementation-spec.md` against the codebase in `crates/app/`.

---

## What Matches the Specs

### Layout Structure
- **Three-panel mail layout** (sidebar + thread list + reading pane + optional right sidebar) is implemented in `main.rs::view_main_window()` as a flat `row![]` — matches the spec.
- **No top bar** — confirmed. No application-level toolbar.
- **Panel widths**: `THREAD_LIST_WIDTH = 400.0`, `RIGHT_SIDEBAR_WIDTH = 240.0`, `SIDEBAR_WIDTH = 180.0` — all match spec. Min widths (`SIDEBAR_MIN_WIDTH = 200`, `THREAD_LIST_MIN_WIDTH = 250`) match.
- **Resizable dividers** — implemented via `DividerDragStart/Move/End` messages with `mouse_area` wrapper. Custom divider (not PaneGrid).
- **Right sidebar auto-collapse** below 1200px — implemented in `WindowResized` handler. `RIGHT_SIDEBAR_AUTO_COLLAPSE_WIDTH` constant in `layout.rs`. One-directional (collapse only, no auto-expand) — matches spec.

### Thread Cards
- **Three-line layout** (sender+date, subject, snippet+indicators) — implemented in `widgets::thread_card()`.
- **No avatars** in thread list — confirmed removed.
- **Starred golden background** — `ButtonClass::ThreadCard { selected, starred }` with `STARRED_BG_ALPHA = 0.12` mixing — matches spec.
- **Label dots** — `label_dot()` widget using `LABEL_DOT_SIZE = 6.0` — matches spec. Currently passed empty `label_colors: &[(Color,)] = &[]` (backend integration pending).
- **Attachment paperclip icon** — present in thread card indicators.
- **`THREAD_CARD_HEIGHT = 68.0`** constant — matches spec.

### Reading Pane / Conversation View
- **Stacked message cards** — expanded and collapsed variants implemented (`expanded_message_card`, `collapsed_message_row`).
- **Message collapse rules** — rules 1 (unread), 2 (most recent), 3 (initial) implemented in `ReadingPane::apply_message_expansion()`. Rule 4 (own messages) intentionally deferred (needs identity matching from core's `get_thread_detail`).
- **Expand/collapse all toggle** — implemented via `ToggleAllMessages`.
- **Attachment group** with collapse toggle — implemented with deduplication by filename and version count badge.
- **Attachment collapse cache** — in-memory `HashMap<String, bool>` keyed by `account_id:thread_id` — matches spec's interim approach (Phase 4.2).
- **Star toggle** — visual-only button in thread header with `StarActive` button class.
- **Date display setting** — `DateDisplay::RelativeOffset` / `Absolute` enum, wired to settings UI.
- **Empty states** — "No conversation selected", "No conversations" / "No results" — all present.
- **Pop-out message windows** — implemented via `PopOutWindow::MessageView` with separate window IDs.

### Window State Persistence
- `WindowState` has `sidebar_width`, `thread_list_width`, `right_sidebar_open` fields with `#[serde(default)]` — matches spec exactly.
- `sanitize()` clamps sidebar to 180.0 (deviation documented in iced-impl spec).
- Panel widths saved on window close — confirmed in `handle_window_close()`.

### Backend (implementation-spec.md)
- **Slice 1** (label color fallback): Implemented in `crates/core/src/label_colors.rs` (per status table).
- **Slice 2** (thread detail data layer): Implemented in `crates/core/src/db/queries_extra/thread_detail.rs` (per status table).
- **Slice 3** (attachment collapse persistence): Migration v59, `thread_ui_state` table, `get/set_attachments_collapsed` functions — all complete.
- **Slice 4** (`FocusedRegion` on `CommandContext`): Implemented. `FocusedRegion` enum exists in `ratatoskr_command_palette`, App tracks `focused_region: Option<FocusedRegion>`.

### Right Sidebar
- `right_sidebar.rs` replaces `contact_sidebar.rs` — old file fully removed (no references found).
- Scaffold with "CALENDAR" and "PINNED ITEMS" placeholder sections — matches spec.

---

## Divergences

### D1: App-Local DB Shim Still Used Instead of Core's `get_thread_detail()`
The iced-impl spec itself flags this as stale: the app uses `crates/app/src/db/connection.rs` with raw SQL queries (`get_thread_messages`, `get_thread_attachments`, `get_threads`, `get_accounts`, `get_labels`) rather than core's `get_thread_detail()`. The core function exists and is complete, but the app has never been wired to use it. This means:
- **No body text from BodyStore** — the app uses `snippet` as body placeholder (no decompressed body HTML/text).
- **No message ownership detection** (collapse rule 4 never fires).
- **No quote/signature-stripped collapsed summaries** — raw snippet truncation at 60 chars instead.
- **No resolved label colors on thread detail** — label dots are always empty.
- **Attachment collapse not persisted to SQLite** — the in-memory HashMap is used instead of `get/set_attachments_collapsed`.

**Files:** `/home/folk/Programs/ratatoskr/crates/app/src/db/connection.rs` (lines 279-347, 163-278)

### D2: No PaneGrid — Custom Divider Implementation
The problem statement and ecosystem survey reference `PaneGrid` and shadcn-rs resizable panels. The actual implementation uses manual divider widgets with `mouse_area` drag tracking. This is a conscious divergence — the custom approach works and avoids PaneGrid's complexity — but means features like `auto_save_id` persistence and drag constraints from the panel ecosystem are not available.

### D3: Thread List Uses App-Local DB for Thread Loading (Partially)
Thread listing uses *both* the app-local `Db::get_threads()` (raw SQL in `connection.rs`) **and** core's `get_threads_scoped()`. The import at line 32 of `main.rs` shows `get_threads_scoped` is used for the primary thread loading path, but the app-local `Db` still has its own `get_threads()` method with duplicated SQL.

### D4: Calendar CRUD Bypasses Core
`crates/app/src/db/connection.rs` contains raw SQL for calendar event CRUD (`create_calendar_event`, `update_calendar_event`, `delete_calendar_event`) including table creation (`CREATE TABLE IF NOT EXISTS pinned_searches`, `contact_groups`, column additions on `contacts`). These bypass `ratatoskr-core` entirely. While calendar and contacts are outside main-layout scope, the pattern establishes a precedent of app-level schema management that the specs discourage.

### D5: Search Context Line — Missing "All" Scope Indicator in Search Mode
The problem statement specifies the search context line should show `"47 results"` on the left and `"All ↗"` scope-widening link on the right. The implementation shows only `"{thread_count} results"` with no scope indicator or widening action.

**File:** `/home/folk/Programs/ratatoskr/crates/app/src/ui/thread_list.rs` (lines 160-166)

### D6: Right Sidebar Still Shows Placeholder Content
The problem statement describes a mini calendar + today's agenda + pinned/starred items. The right sidebar (`right_sidebar.rs`) still renders static placeholder text ("Calendar placeholder", "No pinned items"). The calendar feature has been built as a separate full-page mode (`AppMode::Calendar`) rather than integrated into the right sidebar.

---

## What's Missing (from Specs)

### M1: Phase 3 — Interaction Flow (Deferred)
The iced-impl spec explicitly defers Phase 3. Missing items:
- **Keyboard shortcuts** (j/k navigation, Enter, Escape, e/r/s/#) — the command palette and binding infrastructure exist, but no email-action shortcuts are wired for the thread list or reading pane.
- **Auto-advance** after archive/trash/move — no `get_adjacent_thread()` call or advance logic.
- **Multi-select** in thread list (Shift+click range, Ctrl+click toggle) — single selection only.
- **Inline reply composer** — no composer embedded in the reading pane.
- **Context-dependent shortcut dispatch** — `FocusedRegion` exists on `CommandContext` and on `App`, but the spec's table of region-specific key behaviors is not implemented.

### M2: Real Message Body Rendering
Bodies are rendered as snippet text. No HTML email rendering (no iced_webview, litehtml, or DOM-to-widget pipeline). The expanded message card shows `snippet` not `body_html`/`body_text`.

### M3: Scroll Virtualization
The thread list renders all cards in a `column![]` inside a `scrollable`. No virtualization for large lists. The fixed `THREAD_CARD_HEIGHT` is in place to enable future virtualization, but the implementation is not started.

### M4: Per-Message Reply/Reply All/Forward Actions
The expanded message card widget signature includes action buttons, but they emit `Noop` or are visual-only (not wired to compose flow from the reading pane).

### M5: Image Hover Preview on Attachment Cards
The problem statement specifies a fixed-position image preview on hover over image attachment cards. Not implemented.

### M6: Attachment Save/Open Behavior
Single click, double-click, Save, Save All, Open behaviors for attachments — not wired. Attachment cards are display-only.

### M7: Search Typeahead Popups
The problem statement specifies overlay typeahead for `from:`, `to:`, `label:`, etc. The search bar is a plain `text_input` with no typeahead.

---

## Cross-Cutting Concern Status

### (a) Generational Load Tracking
**Status: Implemented and properly used.**

Three generation counters exist:
- `nav_generation` (u64) — guards `AccountsLoaded`, `NavigationLoaded`, `ThreadsLoaded`, `PinnedSearchThreadIdsLoaded`, `PinnedSearchThreadsLoaded`. Incremented on navigation changes.
- `thread_generation` (u64) — guards `ThreadMessagesLoaded`, `ThreadAttachmentsLoaded`. Incremented on thread selection and navigation changes.
- `search_generation` (u64) — guards `SearchResultsLoaded`. Incremented on new searches and navigation resets.

All stale responses are discarded via `if g != self.xxx_generation => Task::none()` guard patterns. This matches the bloom-style generational tracking recommended by the ecosystem survey.

### (b) Component Trait
**Status: Six panels componentized.**

The `Component` trait (defined in `crates/app/src/component.rs`) is implemented by:
1. `Sidebar` — `SidebarMessage` / `SidebarEvent`
2. `ThreadList` — `ThreadListMessage` / `ThreadListEvent`
3. `ReadingPane` — `ReadingPaneMessage` / `ReadingPaneEvent`
4. `Settings` — `SettingsMessage` / `SettingsEvent`
5. `StatusBar` — `StatusBarMessage` / `StatusBarEvent`
6. `AddAccountWizard` — `AddAccountMessage` / `AddAccountEvent`

**Not componentized:** Right sidebar (stateless `view()` function, no Component impl — appropriate given it has no interaction). Calendar state lives on `App` directly (`CalendarState`) and is rendered via free functions, not Component. The palette overlay (`PaletteState`) is also managed directly by App.

### (c) Token-to-Catalog Theming
**Status: Fully migrated — no inline closures remaining.**

All styles use named class enums:
- `TextClass` (5 variants): Accent, Tertiary, Muted, OnPrimary, Warning
- `ButtonClass` (14 variants): Primary, Secondary, Nav, Dropdown, ThreadCard, Ghost, BareIcon, BareTransparent, Action, CollapsedMessage, StarActive, Chip, PinnedSearch, Experiment*
- `ContainerClass` (28 variants): comprehensive coverage including calendar-specific classes

Usage pattern: `.style(theme::ButtonClass::ThreadCard { selected, starred }.style())`. A `grep` for `.style(|` across `crates/app/src/ui/` returns zero matches — all inline closures have been eliminated.

### (d) iced_drop Drag-and-Drop
**Status: Not implemented.**

No references to `iced_drop`, `Droppable`, or drag-and-drop anywhere in the app crate. Thread reordering and label drag-to-file are not implemented. The problem statement mentions this as a future possibility ("Drag to label (future)"), not a current requirement.

### (e) Subscription Orchestration
**Status: Well-structured batch.**

`App::subscription()` batches:
- **App-level:** appearance changes, window resize, window close requests, window moved
- **Keyboard:** global key press listener dispatched through command system
- **Component:** sidebar, thread list, reading pane, settings, status bar (all via `Component::subscription()`)
- **Conditional:** pending chord timeout (1s timer), search debounce (50ms poll), settings overlay animation (frame-driven)

All subscriptions are collected into a `Vec` and returned via `Subscription::batch(subs)`. This matches the pikeru pattern recommended by the ecosystem survey.

### (f) Core CRUD Bypassed
**Status: Significant bypass pattern exists.**

The app crate (`crates/app/src/db/connection.rs`) contains its own `Db` struct with raw SQL for:
- Account listing (`SELECT ... FROM accounts`)
- Label listing (`SELECT ... FROM labels`)
- Thread listing (`SELECT ... FROM threads` with label join)
- Thread messages (`SELECT ... FROM messages`)
- Thread attachments (`SELECT ... FROM attachments JOIN messages`)
- Message body loading (snippet fallback)
- Message attachments for pop-out view
- Calendar event full CRUD (create, read, update, delete with raw INSERT/UPDATE/DELETE)
- Pinned search management (including table creation)
- Contact search and group management (including ALTER TABLE)

The app also uses core's `get_threads_scoped()` and `get_navigation_state()` for the primary navigation path. This creates a split where some queries go through core and others bypass it. The iced-impl spec acknowledges this as "prototype expedient" that should be replaced by core query surfaces.

### (g) Dead Code
**Status: Minimal dead code found.**

- `PendingChord::started` field is `#[allow(dead_code)]` (line 124 of `main.rs`) — stored but never read for timeout comparison (the timeout uses `iced::time::every` instead).
- `AVATAR_THREAD_CARD` and `AVATAR_CONTACT_HERO` constants in `layout.rs` may be unused now that avatars were removed from thread cards and contact sidebar was deleted, but they could be used elsewhere (message cards use `AVATAR_MESSAGE_CARD`).
- Several TODO comments mark unfinished wiring (re-authentication flow, cc_addresses in pop-out, real SearchState initialization).
- No unreachable match arms found.
- No unused view functions found — all declared functions are called.
