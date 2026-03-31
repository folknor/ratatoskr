# Iced Ecosystem Cross-Reference

For each spec/problem-statement in `docs/`, this document maps which patterns
from the [iced ecosystem survey](iced-ecosystem-survey.md) apply, how they
apply, and where the gaps are.

---

## Table of Contents

1. [Accounts](#accounts)
2. [Calendar](#calendar)
3. [Command Palette — Problem Statement](#cmdk--problem-statement)
4. [Command Palette — Roadmap](#cmdk--roadmap)
5. [Contacts — Import Spec](#contacts--import-spec)
6. [Contacts — Problem Statement](#contacts--problem-statement)
7. [Iced Ecosystem Decisions](#iced-ecosystem-decisions)
8. [Main Layout — Iced Implementation Spec](#main-layout--iced-implementation-spec)
9. [Main Layout — Backend Implementation Spec](#main-layout--backend-implementation-spec)
10. [Main Layout — Problem Statement](#main-layout--problem-statement)
11. [Pop-Out Windows](#pop-out-windows)
12. [Read Receipts](#read-receipts)
13. [Search — Implementation Spec](#search--implementation-spec)
14. [Search — Pinned Searches](#search--pinned-searches)
15. [Search — Problem Statement](#search--problem-statement)
16. [Sidebar](#sidebar)
17. [Status Bar](#status-bar)

---

## Accounts

**Doc**: `docs/accounts/problem-statement.md`

### Requirements → Survey Matches

| Requirement | Primary Source | How It Applies |
|---|---|---|
| First-launch centered modal | shadcn-rs overlays | `place_overlay_centered()` handles viewport-aware placement; dialog component manages focus trapping and Escape-to-close |
| Multi-step wizard (email→discovery→auth→inbox) | trebuchet Component trait + rustcast Page enum | Model as `AddAccountStep` enum. Each variant holds its own state. trebuchet's `(Task, ComponentEvent)` tuples handle async transitions (discovery) cleanly |
| Async discovery with 15s timeout | bloom generational tracking + pikeru subscriptions | Tag discovery task with generation counter; use `tokio::select!` over discovery future and timeout |
| Protocol selection cards | iced-plus props-builder + shadcn-rs data table | Build `SelectionCard` with fluent API; use phantom-type variants for card styles |
| IMAP/SMTP password form | shadcn-rs input/select/checkbox | Props-builder keeps multi-field form readable; security dropdown maps to `select` component |
| Account settings with editor slide-in | bloom config+editing_config | Clone account settings into shadow on click; edit shadow; commit on save, discard on cancel |
| Account reordering via drag | iced_drop | Wrap cards in `Droppable`, use `Operation` trait for tree traversal, chained ops for swap |
| Account color picker | pikeru responsive grid + shadcn-rs tokens | Render 25-color palette as grid; highlight selected; auto-assign next unused on creation |
| Account selector dropdown | shadcn-rs overlay positioning | Dropdown as overlay widget with auto-flip positioning |
| Error states in status bar | pikeru subscriptions + iced-plus platform | Subscription watches account health; emits `TokenExpired`/`ConnectionFailed` messages |

### Gaps

- **Animated panel transitions** (slide-in editor): No surveyed project demonstrates animated transitions in iced
- **OAuth browser handoff**: Purely backend (already in rtsk)

---

## Calendar

**Doc**: `docs/calendar/problem-statement.md`

### Requirements → Survey Matches

| Requirement | Primary Source | How It Applies |
|---|---|---|
| Drag-and-drop events | iced_drop | Wrap event blocks in `Droppable`, time slots as drop zones. `Operation` trait for hit testing |
| Custom time grid widget | bloom custom Widget | `advanced::Widget` trait impls (Timeline, Histogram) show pattern for absolute positioning, drag state, custom draw |
| Stale load cancellation | bloom generational tracking | `load_generation` counter prevents displaying events from a previous date when navigating rapidly |
| Event popover/modal | shadcn-rs overlay positioning | `place_overlay_centered()` for viewport-aware placement; focus/keyboard modules for Escape-to-close |
| Resizable sidebar+main | shadcn-rs resizable panels | Draggable splitters with auto-save, percentage sizing, min/max constraints |
| Mode switching (mail↔calendar) | rustcast Page enum + trebuchet Component | Both states always in model; view renders active one. Component trait keeps subscriptions alive for both |
| Month view grid | pikeru responsive grid | Dynamic row calculation from viewport; RefCell measurement caching |
| Command palette integration | raffi query routing + cedilla key bindings | Enum dispatch for calendar commands; HashMap-based shortcut lookup |
| Mouse interaction | pikeru custom MouseArea | Click vs double-click, drag start/move/end, edge-hover for resize cursors |
| Theme tokens for calendar colors | shadcn-rs + iced-plus | Token registry defines how colors are applied; actual values from provider sync |
| Settings with cancel | bloom config shadow | Shadow event data during edit; commit on save, discard on cancel |
| Sync subscriptions | pikeru + rustcast | `subscription::channel` + `Subscription::batch()` for concurrent provider sync |

### Gaps

- **Overlapping event layout algorithm** (interval graph coloring): Algorithmic, not an iced pattern
- **Recurring event expansion** (RRULE): Domain-specific (`rrule` crate)
- **Multi-day spanning bars in month view**: Specific layout challenge not addressed
- **Time picker with timezone**: Complex form widget with no survey precedent
- **Mini month calendar widget**: No surveyed project includes a month grid date picker
- **Pop-out window**: No surveyed project uses iced multi-window
- **Continuous drag position mapping** (pixel offset → time): iced_drop handles discrete zones, not continuous

---

## Command Palette — Problem Statement

**Doc**: `docs/cmdk/problem-statement.md`

### Requirements → Survey Matches

| Requirement | Primary Source | How It Applies |
|---|---|---|
| Overlay widget | shadcn-rs command palette + overlay positioning | `place_overlay_centered()` for placement; focus trapping; props-builder for `CommandMatch` descriptors |
| Stage-1 vs stage-2 search routing | raffi `route_query()` | Enum dispatch: stage-1 queries `CommandRegistry::query()`, stage-2 queries `CommandInputResolver::get_options()` |
| MRU/recency ranking | raffi `MruEntry` | `HashMap<CommandId, MruEntry>` with count+timestamp, persisted to disk |
| Keyboard subscription batching | rustcast `Subscription::batch()` | Batches hotkeys, keyboard, and palette-internal subscriptions |
| Raw keyboard interception | feu `subscription::events_with` | Intercept KeyPressed before widget processing; modal keybinds without text input interference |
| Panel-aware dispatch | trebuchet Component trait | Each panel returns `(Task, ComponentEvent)`; `FocusedRegion` routes keyboard to correct component |
| Binding registration | cedilla declarative key bindings | Macro + HashMap lookup decoupling menu structure from handlers |
| Stale option resolution | bloom generational tracking | Cancel stale `CommandInputResolver::get_options()` results when user switches commands |

### Gaps

- **Two-key chord sequences** (`g then i`): No surveyed project handles pending chord state with timeouts
- **User-customizable keybindings with conflict detection**: Beyond anything in the survey
- **The core registry architecture** (`CommandId` enum, `CommandContext` predicates, `CommandInputResolver` trait, typed `CommandArgs`): Original to Ratatoskr, no analogues

---

## Command Palette — Roadmap

**Doc**: `docs/cmdk/roadmap.md`

### Requirements → Survey Matches

| Roadmap Slice | Primary Source | How It Applies |
|---|---|---|
| Slice 4 (Ranking) | raffi MRU | `MruEntry` data model for per-command recency tracking |
| Slice 5 (Undo) | cedilla patch history | Circular buffer concept (bounded queue, oldest evicted), but domain differs: action-compensation vs text-diff |
| Slice 6 (Palette UI) | shadcn-rs overlay + raffi query routing | Overlay placement math; prefix-based mode switching between command search and option picking |
| Slice 6 (Keyboard dispatch) | feu/cedilla keyboard subscriptions | `subscription::events_with` for global key capture before widget processing |
| Slice 6 (App architecture) | trebuchet Component trait | Palette as Component emitting `CommandSelected(CommandId)` and `Dismissed` events |
| Slice 6 (Event wiring) | rustcast `Subscription::batch()` | Combines keyboard, palette-internal, and timer subscriptions |
| Slice 6 (Resolver races) | bloom generational tracking | Guards against stale async option results |

### Gaps

- **Slices 1-3 (already complete)**: Backend-only, framework-agnostic, no survey matches needed
- **Two-chord pending indicator with timeout**: No survey precedent

---

## Contacts — Import Spec

**Doc**: `docs/contacts/import-spec.md`

### Requirements → Survey Matches

| Requirement | Primary Source | How It Applies |
|---|---|---|
| Preview data table | shadcn-rs `data_table` | Column renderers, row iteration; adapt for dynamic columns (unknown at compile time) |
| File selection | shadcn-rs/pikeru (`rfd`) | `rfd::AsyncFileDialog` with format filter — solved problem |
| Multi-step wizard | raffi query routing | `ImportStep` enum state machine: FileSelect → SheetSelect → Preview → Importing → Summary |
| Column mapping dropdowns | shadcn-rs/iced-plus props-builder | `ColumnRole` enum with iced `pick_list` per column header |
| Import progress/cancel | bloom generational tracking + pikeru subscriptions | Tag import task with generation; stream row-by-row progress |
| Account selector | bloom config shadow | Entire wizard state is transient editing state; only commits on "Import" |
| Drag-and-drop file import | iced_drop + shadcn-rs file-drop-zone | Optional enhancement using OS-level drag events |

### Gaps

- **Encoding detection** (UTF-8, UTF-16, Windows-1252): Library crate concern (`chardetng`, `encoding_rs`)
- **vCard parsing**: Internal (existing CardDAV parser)
- **Duplicate handling**: Library crate concern

---

## Contacts — Problem Statement

**Doc**: `docs/contacts/problem-statement.md`

### Requirements → Survey Matches

| Requirement | Primary Source | How It Applies |
|---|---|---|
| DnD tokens between To/Cc/Bcc + list-to-grid DnD | iced_drop | Wrap tokens in `Droppable`; assign fields/grid as drop zones; chained ops for move |
| Right-click context menu on tokens | pikeru custom MouseArea | Per-button press detection; emit `TokenRightClicked(id, position)` |
| Popover positioning (inline edit, context menu) | shadcn-rs overlay positioning | `place_overlay_centered()` adapted for anchor-relative placement; focus management for popover fields |
| Contact/group search + autocomplete | rustcast AppIndex prefix-search + raffi query routing | Rayon-parallel fuzzy filtering; enum dispatch for person vs group search |
| Wrapping tile grid for group members | pikeru responsive grid | Viewport-aware column count; RefCell measurement caching |
| Slide-in editor panel | bloom config shadow + trebuchet Component | State transition `List → EditContact(id)`; Component trait for isolated update/view |
| Contact avatar loading | Lumin async icon loading + bloom generational tracking | Batched `Task::perform` for photos; discard stale results on scroll |
| Account selector dropdown | shadcn-rs select/props-builder | iced `pick_list` with consistent styling |
| Confirmation dialogs | shadcn-rs dialog + overlay positioning | Centered overlay with focus trapping |
| Card styling with labels | shadcn-rs + iced-plus token theming | Consistent spacing tokens; label color from existing palette |

### Gaps

- **Token-based input fields** (chip/tag input): No surveyed project implements this. Must build as custom `advanced::Widget`. **Largest custom widget effort for contacts.**
- **Dismissible inline banners**: Simple conditional rendering, no survey reference
- **Immediate-save fields** (no Save/Cancel): Architecturally different from all surveyed projects
- **Autocomplete dropdown lifecycle** (disappear/reappear on new token): Careful state management needed

---

## Iced Ecosystem Decisions

**Doc**: `docs/iced-ecosystem-decisions.md`

### Requirements → Survey Matches

| Decision | Primary Source | How It Applies |
|---|---|---|
| Theme system (6-seed) | shadcn-rs tokens + iced-plus Token-to-Catalog bridge + bloom styling helpers | Token registry separates tokens from consumption; `AppTheme` wraps tokens and implements Catalog traits; `tint()`/`with_alpha()` helpers fill email-specific gaps |
| Spacing/layout (geometric scale) | iced-plus Spacing/Breakpoints + shadcn-rs Spacing enum | Validates enum approach; Ratatoskr's role-based naming is more specific (novel in ecosystem) |
| iced_fontello (don't adopt) | verglas `define_icons!` macro | Could enhance existing `icon.rs` with typed icon functions and IDE autocomplete |
| libcosmic (reference only) | cedilla validates approach | Patterns portable (frostmark, undo); widget code tied to libcosmic fork |
| CEF vs litehtml (open question) | cedilla/frostmark | DOM-to-widget pipeline is a **third option**: pure iced rendering of sanitized HTML for simple/medium emails |

### Survey Patterns Not Covered by Decisions Doc

1. **Generational load tracking** (bloom): No decision covers async resource management
2. **Subscription orchestration** (pikeru, rustcast): No decision covers subscription architecture
3. **Component trait** (trebuchet): No decision addresses message architecture / nested enum problem
4. **Patch-based undo** (cedilla): No decision covers compose editor undo strategy
5. **iced_drop for DnD** (Tier 1): No decision covers drag-and-drop

---

## Main Layout — Iced Implementation Spec

**Doc**: `docs/main-layout/iced-implementation-spec.md`

### Requirements → Survey Matches

| Requirement | Primary Source | How It Applies |
|---|---|---|
| Resizable panels (sidebar, thread list) | shadcn-rs resizable panels | `auto_save_id` could replace manual persistence; min/max constraints more robust than `sanitize()` clamp |
| Starred thread card golden tint | rustcast `tint()`/`with_alpha()` | Validates spec's existing `mix()` helper approach |
| Stale thread detail responses | bloom generational tracking | Replace thread_id staleness check with `load_generation` counter for robustness (handles re-selecting same thread) |
| Phase 3 keyboard shortcuts | raffi query routing + trebuchet Component trait + cedilla key bindings + feu raw keyboard | Component trait is highest-impact — prevents Message enum explosion |
| Data table selection model | shadcn-rs data table | `selected_indices: HashSet`, `anchor_index` for shift-range, `active_index` for keyboard nav |
| Attachment collapse toggle | bloom config shadow | HashMap cache is correct for interim; bloom pattern informs SQLite migration |

### Gaps

- **Thread list virtualization**: No surveyed project implements virtualized scrolling for iced (fixed `THREAD_CARD_HEIGHT` enables future virtualization)
- **Auto-collapse right sidebar below 1200px**: One-directional collapse policy is custom; shadcn-rs panels don't encode this

---

## Main Layout — Backend Implementation Spec

**Doc**: `docs/main-layout/implementation-spec.md`

This is a **backend-only spec**. Survey overlap is limited to how backend data will be consumed by the iced frontend.

| Spec Slice | Survey Pattern | Action |
|---|---|---|
| Slice 2 (`get_thread_detail`) | bloom generational load tracking | Implement generation counter in iced app's thread selection handler |
| Slice 4 (`FocusedRegion`) | trebuchet Component trait + raffi query routing | Structure panel system around Component trait; filter commands by `focused_region` |
| Slice 1 (label colors) | shadcn-rs/iced-plus theming | Register 25 presets as named tokens; build `hex_to_iced_color()` utility |
| Slice 3+2 (attachments) | shadcn-rs resizable panels | Auto-save resizable panels for attachment panel |
| Auto-advance | pikeru subscriptions | Multiplex provider mutation and local DB update channels |

No changes to the backend spec are warranted based on the survey.

---

## Main Layout — Problem Statement

**Doc**: `docs/main-layout/problem-statement.md`

### Requirements → Survey Matches

| Requirement | Primary Source | How It Applies |
|---|---|---|
| Three/four-panel resizable layout | shadcn-rs resizable panels | `auto_save_id` persistence, min/max constraints, percentage sizing |
| Rapid thread switching staleness | bloom generational tracking | Tag each `get_thread_detail()` call with generation counter |
| Multi-select (Shift+click, Ctrl+click) | pikeru custom MouseArea | Granular modifier-key detection on click events |
| Panel architecture | trebuchet Component trait | Each panel as Component with `(Task, ComponentEvent)` return |
| Background sync/search/loading | pikeru + rustcast subscriptions | `subscription::channel` + `Subscription::batch()` for concurrent tasks |
| Token-based theming | shadcn-rs + iced-plus | Centralized palette; Token-to-Catalog bridge for automatic styling |
| Settings | bloom config shadow | Shadow config for live preview with commit/cancel |
| Keyboard shortcuts | feu + cedilla | Raw interception + declarative HashMap bindings |
| Typeahead popups (search bar) | shadcn-rs overlay positioning | Anchored overlay with auto-flip; adapt for left-aligned placement |
| Auto-collapse right sidebar | iced-plus Breakpoints + ShowOn | Gate visibility by window width breakpoint |
| Avatar/attachment loading | Lumin async batching + bloom generational | Batch `Task::perform` for avatars; discard stale on rapid navigation |
| HTML email body rendering | cedilla/frostmark | DOM-to-widget pipeline: html5ever parse → visitor pattern → iced widgets |
| Drag to label (future) | iced_drop | Wrap threads in `Droppable`, sidebar labels as drop zones |

### Gaps

- **Scroll virtualization**: No iced ecosystem solution for 1000+ thread lists
- **Inline reply composer**: No surveyed project embeds a text editor inside a scrollable content list
- **Pop-out windows**: No surveyed project demonstrates multi-window iced with shared state

---

## Pop-Out Windows

**Doc**: `docs/pop-out-windows/problem-statement.md`

### Requirements → Survey Matches

| Requirement | Primary Source | How It Applies |
|---|---|---|
| Session restore (positions, sizes) | shadcn-rs `auto_save_id` + rustcast TOML config | `SessionState` struct (serde) with `Vec<WindowState>`; shadcn-rs's auto-save-by-ID concept |
| Rich text compose (undo) | cedilla patch-based undo | `dissimilar` crate circular buffer for draft history |
| Rich text compose (editor) | cedilla custom TextEditor | Fork iced's `text_editor` to add styled runs |
| HTML rendering in message view | cedilla/frostmark | DOM-to-widget pipeline for sanitized HTML |
| DnD attachments (inline vs attachment zones) | iced_drop + shadcn-rs file-drop-zone + bloom clipboard | iced_drop for two-zone overlay; iced's native `FilesHovered`/`FilesDropped` for OS drops; bloom's clipboard fallback |
| Contact autocomplete | shadcn-rs command palette + raffi query routing + pikeru MouseArea | Searchable dropdown; enum dispatch for contacts/groups/recent; right-click on pills |
| Rendering mode toggle | trebuchet Component + bloom config shadow | Mode as Component; system default in config, per-window override in local state |
| Keyboard shortcuts per window type | cedilla key bindings + feu raw keyboard + trebuchet Component | Per-window-type `handle_key_event()`; subscription for Escape capture |
| Auto-save drafts | pikeru subscriptions + cedilla undo + bloom generational | `iced::time::every(30s)` subscription; undo history as change detector |

### Gaps

- **Multi-window management**: No surveyed project uses iced multi-window. Window lifecycle, per-window routing, cascade-on-main-close are entirely custom. **Largest gap.**
- **WYSIWYG HTML compose**: Confirmed as unsolved. cedilla's editor fork + frostmark pipeline are closest building blocks but far short of rich text editing
- **Token/pill input**: Custom widget build
- **OS print dialog**: Platform-specific code needed (no iced precedent)
- **PDF export from rendered email**: cedilla uses server-side Gotenberg; local solution needed

---

## Read Receipts

**Doc**: `docs/read-receipts.md`

Read receipts is a **protocol-layer feature**. The outgoing side (Phase 1) requires zero UI work — it's a header addition in the provider send path. Survey overlap is minimal.

| Requirement | Primary Source | Applicability |
|---|---|---|
| Async MDN send | pikeru/rustcast subscriptions | Low — generic async; real work is RFC 8098 |
| Receipt policy settings UI | bloom config shadow | Medium — if/when settings panel is built |
| Global policy storage | rustcast TOML config defaults | Low — single enum field |
| Per-message receipt prompt | trebuchet Component trait | Medium — isolates prompt state from reading pane |

**Bottom line**: The heavy lifting is RFC 8098 compliance, database schema, and provider integration — none of which the survey addresses.

---

## Search — Implementation Spec

**Doc**: `docs/search/implementation-spec.md`

Backend-only pipeline (parser, SQL builder, Tantivy, router, smart folders). Most survey patterns target the UI layer.

| Spec Slice | Primary Source | How It Applies |
|---|---|---|
| Slice 4 (3-way router) | raffi `route_query()` | Validates enum dispatch; consider `SearchMode` enum if modes grow beyond 3 |
| Slice 5 (app integration) | bloom generational tracking | **Critical**: Add generation counter to prevent stale results during incremental typing |
| Slice 5 (app integration) | pikeru subscriptions | Consider parallelizing SQL and Tantivy queries in combined path |
| Slice 5 (results display) | shadcn-rs data table | Sort/filter patterns for search result list |
| Slice 6 (smart folders) | Lumin module trait | If backends proliferate beyond SQL+Tantivy, formalize with trait registry |

**Most impactful finding**: bloom's generational load tracking for Slice 5. The spec treats app integration as "trivial wiring," but without stale-result cancellation, the search UX will break for incremental typing.

---

## Search — Pinned Searches

**Doc**: `docs/search/pinned-searches.md`

| Requirement | Primary Source | How It Applies |
|---|---|---|
| Card/chip styling | shadcn-rs + iced-plus tokens | Token palette for elevation, active state, text colors |
| Race on rapid navigation | bloom generational tracking | Generational counter for thread metadata queries |
| Edit-in-place state machine | bloom config shadow | Conceptual: config shadowing inspires approach; custom `navigated_away` flag needed |
| Command palette integration | raffi query routing + trebuchet Component | Context-sensitive commands; Component events for graduation-to-smart-folder flow |
| Escape key state restoration | feu raw keyboard | Raw keyboard interception; mostly custom state logic |
| Thread list with fixed ID set | shadcn-rs data table | Data table patterns for the list (query is simple `IN (...)`) |

### Gaps

- **Relative timestamps** ("2 hours ago"): No surveyed project does this; use `chrono-humanize`
- **Tree rendering for hierarchical folders**: Noted as broader gap

---

## Search — Problem Statement

**Doc**: `docs/search/problem-statement.md`

| Requirement | Primary Source | How It Applies |
|---|---|---|
| Operator typeahead mode switching | raffi `route_query()` | Enum dispatch: `from:` → Contact, `label:` → Label, `after:` → DatePreset |
| Stale query cancellation | bloom generational tracking | **Critical**: Increment generation per keystroke, discard stale results |
| Typeahead popup positioning | shadcn-rs overlay positioning | Anchored below search bar with auto-flip |
| Search bar as component | trebuchet Component trait | Encapsulate query state, typeahead state; emit `SearchExecuted(query)` events |
| Right-click "Search here" | pikeru custom MouseArea | Right-click on sidebar labels/folders |
| Concurrent search pipeline | pikeru subscriptions | `subscription::channel` for off-main-thread queries |
| Declarative keybindings (`/`, Escape) | cedilla key bindings | HashMap<KeyCombo, SearchAction> |
| Search result thread list | shadcn-rs data table | Sort/filter patterns; dual sorting (relevance vs date) |

### Gaps

- **Per-token routing within a single query**: raffi routes entire queries; search needs cursor-local token routing
- **Intra-widget anchoring for typeahead**: Overlay positioning relative to cursor position within search bar

---

## Sidebar

**Doc**: `docs/sidebar/problem-statement.md`

| Requirement | Primary Source | How It Applies |
|---|---|---|
| Panel architecture | trebuchet Component trait | Sidebar as Component with `(Task, ComponentEvent)` return |
| Live unread counts | pikeru subscriptions | Subscribe to DB changes; re-query `get_navigation_state()` on change |
| Stale navigation state | bloom generational tracking | Tag queries with generation; discard on rapid scope switching |
| Resizable width | shadcn-rs resizable panels | Auto-save, min/max constraints (when dimensions work begins) |
| Token-based styling | shadcn-rs + iced-plus | Nav items, badges, section headers from centralized palette |
| Drag-and-drop (future) | iced_drop | Thread-to-label filing; pinned item reorder |
| MRU label ranking | raffi MruEntry | Surface frequently-accessed labels higher (design decision, not in spec) |
| Scope selector popover | shadcn-rs overlay positioning | Auto-flip positioning for dropdown |

### Gaps

- **Tree rendering for hierarchical folders** (Exchange/IMAP/JMAP): **Significant gap**. No surveyed project provides a collapsible tree view. shadcn-rs has a `tree-viewer` mentioned in features but not detailed in survey — needs further inspection.

---

## Status Bar

**Doc**: `docs/status-bar/problem-statement.md`

| Requirement | Primary Source | How It Applies |
|---|---|---|
| Cycling timer + concurrent sync | pikeru subscriptions + rustcast `Subscription::batch()` | Multiplex timer ticks, sync events, and confirmation expiry in one subscription |
| Encapsulated panel | trebuchet Component trait | StatusBar component with own state, view, subscription; emits `RequestReauth(account_id)` upward |
| Visual styling | shadcn-rs + iced-plus tokens | Tokens for muted text, warning color, chrome background |
| Clickable warnings + cursor change | bloom custom Widget + pikeru MouseArea | Custom `Widget` with conditional `mouse_interaction()` returning Pointer for warnings |
| Priority content switching | rustcast Page enum | `StatusContent` enum (Idle/SyncProgress/Warning/Confirmation) with automatic priority resolution |
| Stale sync state | bloom generational tracking | Per-account generation map (extended from bloom's single counter) |

### Gaps

- **Priority-based preemption logic** (warnings interrupt sync, confirmations briefly interrupt then yield back): Bespoke state machine, no survey precedent

---

## Cross-Cutting Gaps

These requirements appear across multiple specs with **no solution in the surveyed ecosystem**:

| Gap | Affected Specs |
|---|---|
| **Scroll virtualization** | Main layout, search, contacts |
| **Multi-window management** | Pop-out windows, calendar |
| **WYSIWYG HTML compose** | Pop-out windows, contacts |
| **Token/pill input widget** | Contacts, pop-out windows |
| **Animated panel transitions** | Accounts, contacts |
| **Hierarchical tree view** | Sidebar |
| **OS print dialog integration** | Pop-out windows |

## Cross-Cutting Strengths

These patterns appear as solutions across **many specs** and should be prioritized for adoption:

| Pattern | Source | Used By |
|---|---|---|
| **Generational load tracking** | bloom | Accounts, calendar, main layout, search, pinned searches, sidebar, status bar, cmd palette |
| **Component trait** | trebuchet | Calendar, main layout, sidebar, status bar, search, cmd palette, pop-out, contacts |
| **Token-based theming + Catalog bridge** | shadcn-rs + iced-plus | All UI specs |
| **Subscription orchestration** | pikeru + rustcast | Calendar, main layout, search, status bar, accounts, pop-out |
| **Overlay positioning** | shadcn-rs | Accounts, calendar, cmd palette, contacts, search, sidebar |
| **Drag-and-drop** | iced_drop | Calendar, contacts, accounts, sidebar, main layout, pop-out |
| **Query routing dispatch** | raffi | Cmd palette, search, contacts |
| **Config shadowing** | bloom | Accounts, calendar, contacts, pop-out, pinned searches |
