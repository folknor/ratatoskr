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
- **Message collapse rules** — rules 1 (unread), 2 (most recent), 3 (initial), and 4 (own messages) all implemented in `ReadingPane::apply_message_expansion()`. Rule 4 uses `is_own_message` from core's `get_thread_detail`.
- **Expand/collapse all toggle** — implemented via `ToggleAllMessages`.
- **Attachment group** with collapse toggle — implemented with deduplication by filename and version count badge.
- **Attachment collapse persistence** — wired to core's `thread_ui_state` table via `persist_attachments_collapsed()`. In-memory cache also maintained for fast reads.
- **Star toggle** — visual-only button in thread header with `StarActive` button class.
- **Date display setting** — `DateDisplay::RelativeOffset` / `Absolute` enum, wired to settings UI.
- **Empty states** — "No conversation selected", "No conversations" / "No results" — all present.
- **Pop-out message windows** — implemented via `PopOutWindow::MessageView` with separate window IDs.
- **Per-message Reply/ReplyAll/Forward** — buttons wired to emit `ReadingPaneEvent::ReplyToMessage`, `ReplyAllToMessage`, `ForwardMessage`. Events handled in App (compose wiring pending).

### Thread Detail from Core
- **`get_thread_detail()` wired** — App loads thread detail via `db::threads::load_thread_detail()` which calls core's `get_thread_detail()` with both main DB and body store connections.
- **Body text from BodyStore** — messages include `body_html` and `body_text` from zstd-decompressed body store.
- **Message ownership detection** — `is_own_message` field populated by core's identity email matching.
- **Quote-stripped collapsed summaries** — `collapsed_summary` from core used as `snippet` in collapsed message rows.
- **Resolved label colors** — `thread_labels` on ReadingPane populated with colors from core's `resolve_label_color()`.

### HTML Email Rendering
- **DOM-to-widget pipeline** — `html_render.rs` parses HTML and emits native iced widgets for: paragraphs, headings, preformatted blocks, blockquotes, ordered/unordered lists, horizontal rules, and image alt text.
- **Complexity heuristic** — `assess_complexity()` detects deeply nested tables and heavy style blocks, falling back to plain text for complex marketing emails.
- **Fallback** — when HTML is too complex or only `body_text` is available, renders as plain text.

### Keyboard Navigation
- **j/k thread navigation** — `NavNext`/`NavPrev` commands wired to `ThreadList::SelectNext`/`SelectPrevious`.
- **Enter to open** — `NavOpen` command wired to `ThreadList::ActivateSelected`.
- **Escape to deselect** — `NavEscape` already existed; thread list supports `Deselect` message.
- **Arrow Up/Down, Home/End** — `SelectPrevious`/`SelectNext`/`SelectFirst`/`SelectLast` messages on ThreadList.

### Search Context
- **"All" scope indicator** — search mode shows `"{count} results"` on left and `"All"` scope-widening link on right. Emits `ThreadListEvent::WidenSearchScope`.

### Window State Persistence
- `WindowState` has `sidebar_width`, `thread_list_width`, `right_sidebar_open` fields with `#[serde(default)]` — matches spec exactly.
- `sanitize()` clamps sidebar to 180.0 (deviation documented in iced-impl spec).
- Panel widths saved on window close — confirmed in `handle_window_close()`.

### Backend (implementation-spec.md)
- **Slice 1** (label color fallback): Implemented in `crates/core/src/label_colors.rs` (per status table).
- **Slice 2** (thread detail data layer): Implemented in `crates/core/src/db/queries_extra/thread_detail.rs` (per status table). Now wired to app.
- **Slice 3** (attachment collapse persistence): Migration v59, `thread_ui_state` table, `get/set_attachments_collapsed` functions — all complete. App wired to persist via `ReadingPaneEvent::AttachmentCollapseChanged`.
- **Slice 4** (`FocusedRegion` on `CommandContext`): Implemented. `FocusedRegion` enum exists in `ratatoskr_command_palette`, App tracks `focused_region: Option<FocusedRegion>`.

### Right Sidebar
- `right_sidebar.rs` replaces `contact_sidebar.rs` — old file fully removed (no references found).
- Scaffold with "CALENDAR" and "PINNED ITEMS" placeholder sections — matches spec.

---

## Divergences

### D1: RESOLVED — Core's `get_thread_detail()` Now Wired
~~The app used local DB shims instead of core's `get_thread_detail()`.~~

**Resolved.** The app now uses `db::threads::load_thread_detail()` which calls core's `get_thread_detail()` with both the main DB connection and the body store connection. This provides body text, ownership detection, collapsed summaries, resolved label colors, and persisted attachment collapse state.

**Files:** `crates/app/src/db/threads.rs` (new bridge module), `crates/app/src/main.rs` (ThreadDetailLoaded message)

### D2: No PaneGrid — Custom Divider Implementation
The problem statement and ecosystem survey reference `PaneGrid` and shadcn-rs resizable panels. The actual implementation uses manual divider widgets with `mouse_area` drag tracking. This is a conscious divergence — the custom approach works and avoids PaneGrid's complexity — but means features like `auto_save_id` persistence and drag constraints from the panel ecosystem are not available.

### D3: Thread List Uses App-Local DB for Thread Loading (Partially)
Thread listing uses *both* the app-local `Db::get_threads()` (raw SQL in `connection.rs`) **and** core's `get_threads_scoped()`. The import at line 32 of `main.rs` shows `get_threads_scoped` is used for the primary thread loading path, but the app-local `Db` still has its own `get_threads()` method with duplicated SQL.

### D4: RESOLVED — Calendar CRUD Bypasses Core
~~`crates/app/src/db/connection.rs` contained raw SQL for calendar event CRUD and schema management.~~

**Resolved.** Calendar CRUD now delegates to synchronous core functions (`create_calendar_event_sync`, `update_calendar_event_sync`, `delete_calendar_event_sync`, `get_calendar_event_sync`, `load_calendar_events_for_view_sync`) in `crates/core/src/db/queries_extra/calendars.rs`. Contact CRUD delegates to core's `contacts.rs` and `contact_groups.rs`. Schema creation for `pinned_searches`, `contact_groups`, and contact extended columns moved to core migration 64. App-level `connection.rs` no longer runs DDL.

### D5: RESOLVED — Search "All" Scope Indicator Added
~~The search context line showed only result count with no scope indicator.~~

**Resolved.** The search mode context row now shows `"{count} results"` on the left and an `"All"` scope-widening link on the right. The link emits `ThreadListEvent::WidenSearchScope` (wiring to actual scope change is pending).

### D6: Right Sidebar Still Shows Placeholder Content
The problem statement describes a mini calendar + today's agenda + pinned/starred items. The right sidebar (`right_sidebar.rs`) still renders static placeholder text ("Calendar placeholder", "No pinned items"). The calendar feature has been built as a separate full-page mode (`AppMode::Calendar`) rather than integrated into the right sidebar.

---

## What's Missing (from Specs)

### M1: Phase 3 — Interaction Flow (Partially Implemented)
The iced-impl spec explicitly deferred Phase 3. Status of individual items:
- **Keyboard shortcuts** — j/k navigation, Enter, Escape wired via command palette bindings. Email action shortcuts (e for archive, # for trash, s for star) available through existing command palette infrastructure.
- **Auto-advance** after archive/trash/move — not implemented. No `get_adjacent_thread()` call or advance logic.
- **Multi-select** in thread list (Shift+click range, Ctrl+click toggle) — not implemented. Single selection only.
- **Inline reply composer** — not implemented. No composer embedded in the reading pane.
- **Context-dependent shortcut dispatch** — `FocusedRegion` exists on `CommandContext` and on `App`, but the spec's table of region-specific key behaviors is not fully implemented.

### M2: RESOLVED — HTML Email Body Rendering
~~Bodies were rendered as snippet text only.~~

**Resolved.** `html_render.rs` implements a DOM-to-widget pipeline that parses HTML and emits native iced widgets. Handles paragraphs, headings (h1-h6), preformatted blocks, blockquotes, ordered/unordered lists, horizontal rules, and image alt text. Includes complexity heuristic for fallback. Falls back to plain text for complex marketing emails with deeply nested tables or heavy CSS.

**Not yet handled:** CID image references, remote image loading/caching, link click handling, table data rendering. These are future enhancements.

### M3: Scroll Virtualization
The thread list renders all cards in a `column![]` inside a `scrollable`. No virtualization for large lists. The fixed `THREAD_CARD_HEIGHT` is in place to enable future virtualization, but the implementation is not started.

### M4: PARTIALLY RESOLVED — Per-Message Reply/Reply All/Forward Actions
~~The expanded message card action buttons emitted `Noop`.~~

**Partially resolved.** Buttons now emit `ReadingPaneEvent::ReplyToMessage`, `ReplyAllToMessage`, `ForwardMessage` with the message index. The App handles these events but compose integration is pending (events currently produce `Task::none()`).

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
- `thread_generation` (u64) — guards `ThreadDetailLoaded`. Incremented on thread selection and navigation changes.
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

### (f) Core CRUD Bypass
**Status: Partially resolved.**

Thread detail loading now goes through core's `get_thread_detail()` (via `db::threads::load_thread_detail()`). Attachment collapse persistence goes through core's `set_attachments_collapsed()`.

Still bypassing core:
- Account listing (`SELECT ... FROM accounts`)
- Label listing (`SELECT ... FROM labels`)
- Thread listing (`SELECT ... FROM threads` — partially, `get_threads_scoped` from core is primary path)
- Calendar event full CRUD
- Pinned search management
- Contact search and group management

### (g) Dead Code
**Status: Minimal dead code found.**

- `PendingChord::started` field is `#[allow(dead_code)]` (line 124 of `main.rs`) — stored but never read for timeout comparison.
- `AVATAR_THREAD_CARD` and `AVATAR_CONTACT_HERO` constants in `layout.rs` may be unused now that avatars were removed from thread cards and contact sidebar was deleted.
- `Db::get_thread_messages()` and `Db::get_thread_attachments()` in `connection.rs` are now unused (replaced by `load_thread_detail`). Can be removed in a future cleanup.
- Several TODO comments mark unfinished wiring (re-authentication flow, cc_addresses in pop-out, real SearchState initialization).
