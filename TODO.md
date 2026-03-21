# TODO

## Outdated dependencies

rusqlite           0.32.1 → 0.39.0            minor
css-inline         0.14.5 → 0.20.0            minor
tokio-tungstenite  0.26.2 → 0.29.0            minor
mundy              0.1.10 → 0.2.2             minor
toml               0.8.23 → 1.0.7+spec-1.1.0  MAJOR
libheif-rs          1.1.0 → 2.7.0             MAJOR
lopdf              0.39.0 → 0.40.0            minor
zip                 2.4.2 → 8.3.0             MAJOR
html5ever          0.35.0 → 0.39.0            minor
markup5ever        0.35.0 → 0.39.0            minor

Have to be careful with toml. Should use the non-spec version probably.

## Inline Image Store Eviction

- [ ] **Wire up user-configurable eviction for `inline_images.db`** — The Rust backend has the building blocks (`prune_to_size()`, `delete_unreferenced()`, `stats()`, `clear()`), but eviction is not yet exposed in the UI.

  **What's missing**:
  1. **Settings UI**: No user-facing control for inline image store size. The 128 MB cap is hardcoded in Rust.
  2. **Scheduled eviction**: No periodic sweep to catch edge cases (e.g., if `MAX_INLINE_STORE_BYTES` is lowered in a future update).

## Cross-Cutting Architecture (from ecosystem survey)

These patterns appeared across 6-8+ specs and should be adopted as foundational infrastructure before feature work builds on top of them. Full rationale in `docs/iced-ecosystem-cross-reference.md`.

- [ ] **Generational load tracking** *(verified 2026-03-21)*

  Pattern established and well-applied. Three generation counters in `crates/app/src/main.rs`: `nav_generation` (accounts/labels/threads/pinned searches), `thread_generation` (messages/attachments), `search_generation` (search results). All stale results discarded via `g != self.xxx_generation` guard arms. Palette uses its own `option_load_generation` on `PaletteState`. Sidebar navigation uses `nav_generation` correctly on scope switch. Pinned search loads use `nav_generation`.

  **Remaining sites** (apply the same pattern as these features are built):
  - ~~Status bar sync progress~~ Done (2026-03-21 multi-agent session) — `sync_generations` map with `begin_sync_generation()`, `is_sync_stale()`, `prune_stale_sync()`.
  - ~~Signature loading~~ Done (2026-03-21 multi-agent session) — now async via `Task::perform` (no generation counter, but no longer synchronous).
  - ~~Pop-out window data loads~~ Done (2026-03-21 multi-agent session) — `pop_out_generation` counter guards stale body/attachment loads.
  - Attachment/body store loading — `docs/main-layout/problem-statement.md`
  - Calendar event loading on date navigation — `docs/calendar/problem-statement.md`

---

- [ ] **Component trait for panel isolation** *(verified 2026-03-21)*

  Trait defined in `crates/app/src/component.rs` with `Message`/`Event` associated types, `update()`, `view()`, and `subscription()` (default `Subscription::none()`). Six components extracted: Sidebar, ThreadList, ReadingPane, Settings, StatusBar, AddAccountWizard. All follow the standard pattern: internal messages stay in `update()`, outward signals emit as events to the parent `App`.

  **Remaining panels to componentize** (as these features are built):
  - **Compose** — currently uses free functions (`update_compose`), not `Component` trait
  - **Calendar** — state lives on `App` directly, rendered via free functions
  - **Command palette** — `PaletteState` managed directly by App, no `PaletteEvent` type. May be intentional (tight coupling to registry/resolver)
  - **Pop-out windows** — use free functions (`view_message_window`, `view_compose_window`), inconsistent with main window components
  - **Right sidebar** — stateless view function (appropriate given no interaction)

---

- [ ] **Token-to-Catalog bridge for theming** *(verified 2026-03-21)*

  Style migration complete. 8+ class enums defined in `theme.rs`: `ButtonClass` (14 variants), `ContainerClass` (28 variants), `TextClass` (5 variants), `RuleClass`, `TextInputClass`, `SliderClass`, `RadioClass`, `TogglerClass`, `PickListClass`. All style functions centralized behind enum dispatch with `.style()` methods. A grep for `.style(|` across `crates/app/src/ui/` returns zero matches — all inline closures eliminated.

  **Known exceptions** (not violations — architecturally correct):
  - Rich text editor passes colors via builder methods (standalone widget, not themed via Catalog)
  - Token input widget draws directly via `renderer.fill_quad()` (custom `advanced::Widget`)
  - ~~Two inline closures in palette~~ Resolved (2026-03-21 multi-agent session) — palette now uses `TextClass` variants throughout

  **Future enhancements** (optional, evaluate as needed):
  - **Token registry** (from shadcn-rs): `ThemeTokenRegistry` (BTreeMap) separating token definition from consumption. Useful if user-customizable themes beyond the 6-seed system are needed.

---

- [x] **Vendor iced_drop for drag-and-drop** *(Done 2026-03-21)* — Vendored into `crates/iced-drop/`, adapted to our iced fork, added to workspace. Not yet wired to any UI.

  Add iced_drop (623 lines, zero external dependencies beyond iced) as a vendored crate in the workspace. This is the only "steal the code" repo from the survey — everything else is "steal the pattern."

  **The library**: iced_drop provides a `Droppable<Message>` widget wrapper that makes any iced element draggable, with drop zone detection via widget `Id`s. Features: visual feedback during drag (cursor change, optional overlay rendering, optional source hiding), configurable drag threshold, drag-axis constraints, nested droppables. The entire library is 623 lines with no dependencies beyond `iced_core`, `iced_widget`, and optionally `iced_runtime`.

  **The `Operation` trait pattern** (`research/iced_drop/src/widget/operation/drop.rs` lines 10-86): This is arguably more valuable than the drag-and-drop itself. iced_drop defines structs implementing `Operation<T>` that traverse the widget tree to find drop zones by `Id` and collect their bounds. The pattern generalizes to any tree query: finding the focused widget, collecting all widgets of a type, swapping state between two widgets. The chained operations example (`research/iced_drop/examples/todo/src/operation.rs`) shows `FindTargets` → `Chain(SwapModify)` for finding two widget states and modifying them in sequence.

  **Where needed**:
  - **Thread reordering** (drag threads to reorder in thread list) — `docs/main-layout/problem-statement.md`
  - **Label drag-to-file** (drag thread onto sidebar label) — `docs/sidebar/problem-statement.md`
  - **Account reordering** (drag accounts in settings) — `docs/accounts/problem-statement.md`
  - **Compose token DnD** (drag recipient between To/Cc/Bcc) — `docs/contacts/problem-statement.md`
  - **Group editor member DnD** (drag contacts from list to member grid) — `docs/contacts/problem-statement.md`
  - **Calendar event dragging** (drag events between time slots / days) — `docs/calendar/problem-statement.md`
  - **Attachment drag zones** (drag file over compose → "inline" vs "attachment" zones) — `docs/pop-out-windows/problem-statement.md`

  **Vendoring vs dependency**: Vendor into `crates/iced-drop/` rather than depending on the crate directly. Reasons: (1) we use Halloy's iced fork, so the iced version must match exactly; (2) we'll likely need to modify the library for calendar-specific needs (continuous position mapping instead of discrete zone detection); (3) 623 lines is small enough to own.

  **Gap for calendar**: iced_drop handles discrete drop zones (which zone did I land in?). The calendar time grid needs continuous position mapping (pixel offset → time). The library's zone-finding `Operation` needs augmentation with proportional time calculation from cursor Y position within the zone.

  **Interaction with other items**: The Component trait determines which component handles drop events. The overlay positioning (from shadcn-rs) interacts with drag overlay rendering.

  **Reference**: `research/iced_drop/src/widget/droppable.rs` (main widget), `research/iced_drop/src/widget/operation/drop.rs` (Operation trait), `research/iced_drop/examples/todo/` (complete example with chained operations).

---

- [ ] **Subscription orchestration pattern** *(verified 2026-03-21)*

  Infrastructure well-established. `App::subscription()` batches all component subscriptions alongside app-level ones (mundy appearance, window resize/close/move). Active conditional subscriptions: global keyboard listener (`iced::event::listen_with`), pending chord timeout (1s timer), search debounce (50ms poll), settings overlay animation, status bar cycling (3s timer, conditional on active content). Status bar `subscription()` is correctly conditional — only ticks when cycling or expiry is needed.

  **Remaining work** (as these features are built):
  - ~~Sync pipeline events (4 providers)~~ Partially done (2026-03-21 multi-agent session) — `IcedProgressReporter` type and `SyncEvent` enum implemented, `create_sync_progress_channel()` factory built, `Message::SyncProgress` wired. Remaining: connect sync orchestrator to the reporter.
  - ~~Compose auto-save timer (30s)~~ Done (2026-03-21 multi-agent session) — `iced::time::every(30s)` subscription fires `Message::ComposeDraftTick` when any compose window has `draft_dirty` set. Saves to `local_drafts` table.
  - File system watches (draft changes, attachment modifications)
  - Provider push notifications (IMAP IDLE, JMAP push, Graph webhooks, Gmail watch)
  - GAL polling refresh (hourly, for contacts)

---

- [x] **DOM-to-widget pipeline for HTML email rendering** — V1 implemented (2026-03-21 multi-agent session) in `crates/app/src/ui/html_render.rs`. Handles paragraphs, headings, lists, blockquotes, pre, hr, image alt text. Complexity heuristic falls back to plain text for CSS-heavy marketing emails. Remaining: CID image loading, remote images, link clicks, table rendering.

  Evaluate frostmark's approach (from cedilla) as a native-iced alternative to CEF and litehtml for rendering HTML email bodies. This is the most important rendering decision for the app — it determines whether email bodies are rendered inside iced's widget tree (native scrolling, selection, theming) or in an embedded browser view (full CSS fidelity but integration friction).

  **The three options** (from `docs/iced-ecosystem-decisions.md`):
  1. **CEF** (Chromium Embedded Framework): Full browser fidelity. Handles any HTML/CSS. But: 100MB+ binary size increase, complex IPC for selection/scrolling/theming integration, platform-specific build complexity.
  2. **litehtml** (via iced_webview_v2): Lightweight HTML/CSS renderer. Good table layout. But: C++ dependency, limited CSS3 support, no JavaScript.
  3. **DOM-to-widget pipeline** (frostmark approach): Parse HTML → walk DOM → emit iced widgets. Fully native. But: no CSS engine, limited to structural HTML (paragraphs, images, links, lists, basic tables).

  **The frostmark pattern** (`research/cedilla/` — frostmark is a separate crate in cedilla's workspace):
  - Parse HTML with `html5ever` / `markup5ever` into a DOM tree
  - Walk the DOM with a visitor pattern
  - For each node, emit the corresponding iced widget:
    - `<p>`, `<div>`, `<span>` → `text()` or `rich_text()` with styled spans
    - `<img>` → `image()` with async loading from inline image store (CID references) or remote URLs
    - `<a>` → styled `button` or `mouse_area` wrapping text, emitting `LinkClicked(url)`
    - `<table>` → nested `row()` / `column()` (this is where it gets hard)
    - `<ul>`, `<ol>` → `column` with bullet/number prefixes
    - `<blockquote>` → `container` with left border and indentation
    - `<pre>`, `<code>` → monospace `text()` with background container
  - Cache downloaded images separately from text rendering (`HashMap<String, image::Handle>` from cedilla's `MarkdownPreview` struct)

  **What this handles well**: Plain text emails, simple HTML (text + images + links), forwarded message chains (nested blockquotes), most transactional emails (receipts, confirmations). This likely covers 60-80% of email by volume.

  **What this can't handle**: CSS-heavy marketing emails (complex grid layouts, web fonts, media queries), emails with heavy inline CSS (background images, gradients, custom positioning), emails that rely on `<table>` for visual layout (which is most marketing HTML — tables are used for column layout, not data). The table-for-layout problem is the single biggest gap.

  **Proposed approach**: Implement the DOM-to-widget pipeline for the common case. Add a complexity heuristic (count CSS properties, nesting depth, table usage) and fall back to litehtml/CEF for emails that exceed the threshold. This gives native iced rendering for most emails (with proper scrolling, selection, and theming) and full fidelity for complex ones.

  **Interaction with other items**: The Component trait means the reading pane owns this rendering pipeline. Generational load tracking applies to image loading within rendered emails. The token-to-Catalog bridge ensures rendered email widgets inherit the app's theme.

  **Reference**: cedilla/frostmark (`research/cedilla/`), specifically the HTML-to-widget visitor pattern. Image caching from `MarkdownPreview` (HashMap<String, image::Handle>). raffi's `ansi_to_spans()` at `research/raffi/src/ui/wayland/ansi.rs` demonstrates the general approach of parsing formatting codes into iced `span` objects (different input format but same architecture).

---

- [ ] **Config shadow pattern for settings/edit flows**

  Any UI that edits persistent state should clone the real state into an `editing_*` shadow on open. This prevents partial saves, enables live preview, and provides trivial change detection.

  **The problem**: Without shadowing, editing a complex form (account settings with 8+ fields, calendar event with recurrence rules, contact with multiple addresses) either (a) writes each field change immediately to the database, risking inconsistent state if the user abandons the edit, or (b) requires manual dirty tracking for each field to know whether to prompt "save changes?". Both approaches are error-prone.

  **The pattern** (from bloom `research/bloom/src/app.rs` lines 38, 196, 402):
  ```rust
  struct SettingsPanel {
      config: AppConfig,                       // the committed state
      editing_config: Option<AppConfig>,        // the shadow (Some when editing)
  }

  // On settings open:
  self.editing_config = Some(self.config.clone());

  // All edits go to the shadow:
  if let Some(ref mut editing) = self.editing_config {
      editing.theme = new_theme;  // user sees the change live
  }

  // On save:
  self.config = self.editing_config.take().unwrap();
  persist_to_disk(&self.config);

  // On cancel:
  self.editing_config = None;  // discard, config unchanged

  // Change detection:
  let has_changes = self.editing_config.as_ref() != Some(&self.config);
  ```

  **Where to apply**:
  - **Account settings** (display name, color, CalDAV config, re-auth) — `docs/accounts/problem-statement.md`. The slide-in editor clones account settings on open; Save commits, Back discards.
  - **App preferences** (theme, font, date display, auto-advance direction) — `docs/main-layout/iced-implementation-spec.md`. Live preview means the user sees theme changes immediately while editing, but the original theme is restored on cancel.
  - **Calendar event editor** (read mode → edit mode with Save/Cancel) — `docs/calendar/problem-statement.md`. Clone event data on entering edit mode.
  - **Contact import wizard** (file selection, column mapping, target account — all transient state that only commits on "Import") — `docs/contacts/import-spec.md`.
  - **Pinned search edit-in-place** (editing query without navigating away updates existing; navigating away then searching creates new) — `docs/search/pinned-searches.md`.

  **Exception — contacts**: The contacts spec (`docs/contacts/problem-statement.md`) says fields save immediately with no Save/Cancel. This is the opposite of the shadow pattern. For contacts, each field's `on_input` writes directly to the database. The shadow pattern does NOT apply to the contact editor — only to flows with explicit commit/cancel semantics.

  **Interaction with other items**: The Component trait determines which component owns the shadow. For account settings, the settings Component holds `editing_account: Option<AccountSettings>`. For calendar events, the reading pane or calendar Component holds `editing_event: Option<CalendarEvent>`. The generational load tracking interacts if the underlying data changes while editing (e.g., a sync updates the account's token status while the user is editing display name) — the shadow isolates the user from these background changes until they commit.

  **Reference**: bloom `research/bloom/src/app.rs` lines 38 (`editing_config` field), 196 (clone on settings open), 402 (commit/discard on save/cancel). Also rustcast's TOML config with `#[serde(default)]` at `research/rustcast/src/config.rs` for the serialization pattern.

- [ ] **Make sidebar fixed-width (not resizable)** *(Deferred until later)* — The sidebar should be a fixed width, not draggable. Remove the sidebar resize divider and any sidebar width persistence from `WindowState`. The sidebar width is a constant in `layout.rs`, not a user preference.

- [ ] **Per-pane minimum resize limits** — PaneGrid currently uses a uniform `min_size(120)` for all panes. Should have per-pane minimums (e.g., sidebar can't go below 150px, thread list below 200px, contact sidebar below 180px). Requires clamping ratios in the `PaneResized` handler since PaneGrid only supports a single global minimum. Decide on actual values after visual testing.

- [ ] **`responsive` for adaptive layout** — Wrap PaneGrid in `iced::widget::responsive` to collapse panels at narrow window sizes (e.g., hide contact sidebar below 900px, stack sidebar over thread list below 600px).

- [ ] **Keybinding display and edit UI** — Need to redo the Settings/Shortcuts UI. Take a look at https://nyaa.place/blog/libadwaita-1-8/

- [ ] **License display/multiline static text row** — Need to be able to click links and make text selectable/copyable in license display widgets. Needs its own base row type.

- [ ] **Restore OS-based theme and 1.0 scale** *(Deferred until 1.0 release)* — `SettingsState::default()` currently hardcodes `theme: "Light"` for development convenience. Revert to `theme: "System"` once UI prototyping is done, and persist user preferences to disk.

- [x] **Wire up system font detection (Phase 1 — UI font)** — Done. Synchronous detection before app launch via throwaway tokio runtime. Detected font family stored in `OnceLock`, font constants converted to functions (`font::text()`, `font::text_semibold()`, etc.). Falls back to bundled Inter if detection fails.
- [ ] **Wire up system font detection (Phase 2 — Document font)** — The detected document font (e.g., TisaPro) should be used for email body text and other long-form content. This requires threading a separate font through the thread detail view and message body widgets. Not straightforward right now — email bodies are HTML processed by the sanitizer pipeline, not iced text widgets, so the document font may only apply once we have native body rendering. Add as a font setting in the settings UI too (let users override the detected system fonts).

- [x] **Thread list keyboard navigation** — Done (2026-03-21 multi-agent session). j/k navigation via command palette bindings, Arrow Up/Down via `SelectPrevious`/`SelectNext`, Home/End via `SelectFirst`/`SelectLast`, Enter to open, Escape to deselect. Wired through command palette dispatch. PgUp/PgDn not yet implemented.

- [x] **Scrollbars must shift layout, not overlay** — Done. Added `SCROLLBAR_SPACING` constant to `layout.rs` and applied `.spacing(SCROLLBAR_SPACING)` to all 7 scrollable instances (sidebar, thread list, reading pane, right sidebar, 3 settings scrollables). Uses iced's embedded scrollbar mode.

- [ ] **Thread list pagination (revisit later)** — Currently loads all threads at once (LIMIT 1000). This is fast with the test dataset (1000 threads renders instantly). We attempted batched lazy loading (200 per page, `on_scroll` trigger, spacer for honest scrollbar) but reverted: (1) `on_scroll` fires on every pixel of scroll movement, causing a full `update()`/`view()` cycle per pixel which made scrolling sluggish; (2) the spacer approach for honest scrollbar sizing made the content area huge, worsening the `on_scroll` overhead; (3) without the spacer the scrollbar thumb jumps when batches load (content height changes suddenly). The DB layer already supports `LIMIT`/`OFFSET` (`db.get_threads` has the params, `count_threads` exists). Revisit when thread counts actually cause problems — likely needs iced-level virtual scrolling (only render visible rows) rather than application-level pagination, since the bottleneck is widget count in the scrollable, not query speed.

- [ ] **Undo/redo for all text inputs** — iced's built-in `TextInput` and `TextEditor` do not support Ctrl+Z/Ctrl+Y out of the box. Every text field in the app should support basic undo/redo like users expect from any desktop application.

  **Approach**: Use the `dissimilar` crate to maintain a compact diff-based undo history per input. On each change, diff old vs new text, store the patch in a circular buffer (~50 entries). Ctrl+Z applies the patch in reverse, Ctrl+Y reapplies forward. This is lightweight — patches for single-character edits are a few bytes.

  **Standard text inputs** (straightforward): Search bar, subject line, smart folder query editor, contact notes, calendar event fields, account display name, any single-line or multi-line plain text field. Wrap the undo logic in a reusable struct (`UndoableText { current: String, history: VecDeque<Patch>, position: usize }`) that any input can use.

  **Inputs that need special treatment**:
  - **To/Cc/Bcc recipient fields**: These autocomplete to contact "pills" — the underlying state is a `Vec<Recipient>`, not a plain string. Undo needs to operate on the recipient list (undo adding a pill, undo removing one), not on raw text. The text portion (what the user is currently typing before it resolves to a pill) can use standard text undo, but pill add/remove needs its own operation stack.
  - **Rich text compose editor**: Already has operation-based undo/redo designed into its architecture (`docs/editor/architecture.md`). Does not use `dissimilar` — the structured document model captures edits as reversible `EditOp`s, which is more appropriate than string diffing for formatted text.
  - **Label/tag pill inputs**: Same pill pattern as recipients — undo operates on the tag list, not raw text.

  **Implementation**: Add a `UndoableText` helper to `crates/app/src/ui/` and integrate it into the key handler for each text input (intercept Ctrl+Z/Ctrl+Y before iced processes them). For pill-based inputs, define an `UndoableList<T>` that tracks add/remove operations.

  **Reference**: cedilla `research/cedilla/src/app/core/history.rs` (dissimilar-based undo with circular buffer).

## Review Findings (2026-03-20)

Deferred items from code review. Grouped by feature area.

### Pinned Searches (1ba6249)

- [ ] **Replace `pre_search_threads` with `PreSearchView`** — The spec recommends against the `pre_search_threads` clone approach (calling it a "V1 shortcut") and proposes `PreSearchView` for navigation-target-based restoration. The implementation uses `pre_search_threads` for save and `restore_folder_view()` for dismiss. Both search and pinned searches should converge on `PreSearchView`.

- [ ] **Cache `thread_ids` on `PinnedSearch` struct** — The spec defines `thread_ids: Vec<(String, String)>` on the struct (loaded lazily) so re-clicking the same pinned search doesn't re-query the DB. The implementation always re-queries. Minor — the DB query is fast.

- [ ] **Pinned searches Phase 2 features** — No staleness label, no `SearchBarState` type, no periodic expiry subscription. Phase 2/4 items.

### Pop-out Message View (c9d6a42)

- [x] **Add spec scaffolding fields to `MessageViewState`** — Done (2026-03-21 multi-agent session). All fields present: `cc_addresses`, `rendering_mode`, `raw_source`, `scroll_offset`, position tracking, `error_banner`, `overflow_menu_open`, `remote_content_loaded`.

### Pop-out Compose (d650308)

- [x] **Add `cc_addresses` to `ThreadMessage` and `MessageViewState` for Reply All** — Done (2026-03-21 multi-agent session). `cc_addresses` present in `MessageViewState`, `from_thread_message()` seeds it. Reply All compose opens with proper Cc recipients.

### Contacts Management (033650c)

- [ ] **Decide save pattern for contacts** — TODO.md (below) says "contacts save immediately with no Save/Cancel — shadow pattern does NOT apply." The spec distinguishes local (immediate save) vs synced (explicit Save). Implementation uses explicit Save for all contacts. Needs decision: immediate-save for local contacts, or keep explicit Save everywhere.

- [x] **Add account selector to contact editor** — Done (2026-03-21 multi-agent session). Account selector dropdown on contact creation/edit lists connected accounts + "Local".

- [x] **Add delete confirmation for contacts and groups** — Done (2026-03-21 multi-agent session). Two-step delete: Delete -> Confirm delete / Cancel.

- [x] **Replace N+1 group membership query with JOIN** — Done (2026-03-21 multi-agent session). Now uses single JOIN query with `GROUP_CONCAT`.

### Emoji Picker (b15cd89)

- [ ] **Recent/frequent emoji section and skin tone selection** — TODO.md (below) says the picker needs these. Neither is implemented.

- [ ] **Flags emoji category** — Most emoji pickers include country/flag emoji. Not included in the static table.

## Spec-vs-Code Audit (2026-03-20, updated 2026-03-21)

Gaps found comparing current code against implementation specs. Grouped by feature. Full per-feature audit reports in `docs/<feature>/discrepancies.md`.

### Command Palette

**Specs:** `docs/command-palette/app-integration-spec.md`, `docs/command-palette/problem-statement.md`

- [x] **`NavigateToLabel` command entirely missing** — Done (2026-03-21 multi-agent session). `CommandId::NavigateToLabel` registered in registry, dispatched, and resolved. `get_all_labels_cross_account()` wired via `AppInputResolver`.

- [x] **`provider_kind` always `None` in `CommandContext`** — Done (2026-03-21 multi-agent session). Provider kind now resolved from account data in `build_context()`.

- [x] **`current_view` only detects 2 of 14+ view types** — Done (2026-03-21 multi-agent session). View type detection now covers all universal folders, calendar, search, pinned search, and settings.

- [x] **Thread state flags never populated** — Done (2026-03-21 multi-agent session). `is_pinned` and `is_muted` populated from thread data. `in_trash`, `in_spam`, `is_draft` still require label-based detection (not yet implemented).

- [x] **No pending chord indicator UI** — Done (2026-03-21 multi-agent session). Pending chord indicator badge displayed in bottom-right corner. `PendingChord.started` still `#[allow(dead_code)]` (timeout uses subscription, not elapsed check).

- [x] **Snooze/DateTime parameterized commands skipped** — Done (2026-03-21 multi-agent session). DateTime parameterized commands now show preset time options ("1 hour", "2 hours", "4 hours", "Tomorrow 9am", "Tomorrow 1pm", "Next week").

- [ ] **No scroll-to-selected in palette results** — Arrow keys update `selected_index` but no `scrollable::scroll_to` task is returned. Selected item can scroll off-screen.

- [ ] **Palette not componentized** — Spec defines `PaletteEvent` enum following the Component trait pattern. Implementation puts palette logic directly in `App::handle_palette()`.

- [x] **Inline text style closure in `palette_result_row`** — Done (2026-03-21 multi-agent session). Palette now uses `TextClass` variants throughout.

### Sidebar

**Specs:** `docs/sidebar/implementation-spec.md`, `docs/search/pinned-searches-implementation-spec.md`

- [x] **Spam/All Mail folders never appear** — Done (2026-03-21 multi-agent session). Added to `SIDEBAR_UNIVERSAL_FOLDERS` in backend. Sidebar filters them out in "All Accounts" mode, shows when scoped to single account.

- [ ] **Magic number `28` in `truncate_query`** — `truncate_query(&ps.query, 28)` uses a raw number not from layout constants.

- [ ] **`SidebarEvent::CycleAccount` is dead code** — Sidebar internally converts `CycleAccount` to `AccountSelected` in `update()`, so `CycleAccount` is never emitted as a `SidebarEvent`. The handler arm in `handle_sidebar_event` is unreachable.

- [x] **O(n²) HashMap rebuild in `is_hidden_by_collapsed_ancestor`** — Done (2026-03-21 multi-agent session). HashMap now built once in `render_label_tree` and passed in as `id_to_folder` parameter.

- [x] **Pinned search visual deviations from spec** — Mostly done (2026-03-21 multi-agent session). Relative time format now used ("5 min ago"). Query is now primary, date secondary. Magic number `28` replaced with `PINNED_SEARCH_QUERY_MAX_CHARS`. `ButtonClass::PinnedSearch` retained as intentional divergence (better visual distinction). Chevron styling uses `TextClass::Tertiary`.

### Accounts

**Spec:** `docs/accounts/implementation-spec.md`

- [x] **Discovery is completely faked** — Done (2026-03-21 multi-agent session). Real `ratatoskr_core::discovery::discover()` called with 15s timeout. Branches on `source.is_high_confidence()` for auto-proceed vs protocol selection.

- [x] **Account creation bypasses core CRUD** — Done (2026-03-21 multi-agent session). Now calls `create_account_sync()` from core CRUD. Provider and auth method set from discovery results.

- [x] **Hard-coded provider to `'imap'`** — Done (2026-03-21 multi-agent session). Provider and auth method now set from discovery results via `CreateAccountParams`.

- [x] **No `AccountHealth` enum** — Done (2026-03-21 multi-agent session). `AccountHealth` enum with `Healthy/Warning/Error/Disabled` and `compute_health()` implemented. `ManagedAccount` has `health` field. Note: always returns `Healthy` until `token_expires_at` and `is_active` are plumbed from DB.

- [x] **No account editor in settings** — Done (2026-03-21 multi-agent session). Account cards clickable with chevron and health indicator. `AccountEditor` struct with slide-in overlay, name/color/CalDAV fields, save/delete with confirmation.

- [x] **No duplicate account detection** — Done (2026-03-21 multi-agent session). `account_exists_by_email_sync` called during email submission before discovery.

- [x] **Protocol selection is a stub** — Done (2026-03-21 multi-agent session). Shows discovered protocol options as selectable cards with provider name, detail, source label. Pre-selects top option.

- [ ] **`color_palette_grid` not reusable** — Hardcoded to `AddAccountMessage::SelectColor(i)`. Spec says generic widget in `widgets.rs` with `on_select` callback.

- [ ] **Magic numbers in add_account.rs** — `icon::mail().size(48.0)`, `.padding(2)`, stroke width `2.0`, alpha `0.35`.

- [ ] **No re-authentication flow (Phase 7)** — No `ReauthWizard`, no health indicators, no error recovery.

### Search

**Spec:** `docs/search/app-integration-spec.md`

- [x] **Search execution is a SQL LIKE stub** — Done (2026-03-21 multi-agent session). Now calls unified pipeline (`search_pipeline::search()`) when Tantivy index available, with SQL-only fallback using smart folder parser/SQL builder for structured operators. LIKE remains only as last-resort for pure free-text without an index.

- [x] **`SearchBlur` unfocus not wired** — Done (2026-03-21 multi-agent session). Handler now focuses a dummy widget ID to remove focus from search bar.

- [ ] **Phases 2-4 partially done** — Phase 3 (typeahead) done (2026-03-21 multi-agent session): `CursorContext` operator detection in smart-folder parser, dropdown suggestions for from/to (contacts), label/folder, account, has/is/in, before/after (date presets), keyboard navigation (arrows, Tab, Escape). Phase 2 (smart folder CRUD via palette) and Phase 4 ("Search here" scoped search) not started.

- [x] **Smart folder migration (Slice 6) not started** — Partially done (2026-03-21 multi-agent session). `migrate_legacy_tokens()` translates `__LAST_7_DAYS__` -> `-7`, etc. `resolve_query_tokens` no longer re-exported. Execution path still SQL-only for smart folders (intentional — avoids circular dependency with core).

- [x] **Tantivy-only path drops multi from/to values** — Done (2026-03-21 multi-agent session). `SearchParams.from` and `SearchParams.to` are now `Vec<String>`. `build_tantivy_params()` passes all values.

- [x] **`SearchParams.label` is dead** — Done (2026-03-21 multi-agent session). Field removed entirely. Label filtering handled by SQL builder.

- [x] **`delete_all_pinned_searches` not implemented** — Done (2026-03-21 multi-agent session). Function exists in `crates/app/src/db/pinned_searches.rs`.

- [x] **Duplicate `group_by_thread()`** — Done (2026-03-21 multi-agent session). `search_pipeline.rs` now delegates to the public `ratatoskr_search::group_by_thread()` via `group_by_thread_unified()` wrapper.

### Contacts Autocomplete

**Spec:** `docs/contacts/autocomplete-implementation-spec.md`

- [x] **Ranking uses frequency instead of recency** — Done (2026-03-21 multi-agent session). Now uses `last_contacted_at DESC`, `last_seen_at DESC` for recency-based ranking.

- [ ] **`ContactSearchResult` types in app crate instead of core** — Placed in `token_input.rs`. Spec says `crates/core/src/contacts/search.rs`. Violates crate boundary.

- [ ] **Magic numbers in token input widget** — `0.54` char width heuristic, cursor offsets `2.0`/`4.0`, placeholder alpha `0.4`.

- [x] **`label.len()` byte count for width estimation** — Done (2026-03-21 multi-agent session). Now uses `chars().count()` for correct multi-byte handling.

- [x] **No autocomplete dropdown (Phase 2)** — Done (2026-03-21 multi-agent session). `AutocompleteState` in `ComposeState`, `search_contacts_for_autocomplete` wired, dropdown rendering with highlighted row, mouse click to select, generation counter for stale discard.

- [x] **No paste address parser (Phase 3)** — Done (2026-03-21 multi-agent session). RFC 5322 parser in `token_input_parse.rs` handles `Name <email>`, `"Name" <email>`, bare email formats. Dedup within paste and against existing tokens.

- [x] **No arrow key navigation between tokens** — Done (2026-03-21 multi-agent session). Left/Right arrow navigates between tokens. Left at text position 0 selects last token.

- [x] **No right-click context menu on tokens** — Done (2026-03-21 multi-agent session). `TokenContextMenu(TokenId, Point)` message emitted on right-click.

- [x] **No contact group or GAL search in autocomplete** — Partially done (2026-03-21 multi-agent session). Autocomplete now searches contacts, seen addresses, AND contact groups. GAL caching not implemented.

### Signatures

**Spec:** `docs/signatures/implementation-spec.md`

- [x] **Plain text editor instead of rich text** — Done (2026-03-21 multi-agent session). Signature editor now uses `RichTextEditor` with `EditorState`, formatting toolbar (B/I/U/S, lists, blockquote), HTML round-trip via `from_html()`/`to_html()`.

- [x] **Inline SQL bypasses core CRUD** — Done (2026-03-21 multi-agent session). `handlers/signatures.rs` now delegates to core CRUD functions: `db_insert_signature`, `db_update_signature`, `db_delete_signature`, `db_get_all_signatures`, `db_reorder_signatures` via `DbState::from_arc()` bridge. No more raw SQL.

- [x] **`is_reply_default` toggle doesn't clear old default transactionally** — Done (2026-03-21 multi-agent session). Handler in `handlers/signatures.rs` now clears old defaults transactionally for both `is_default` and `is_reply_default`.

- [x] **No `body_text` auto-generation** — Done (2026-03-21 multi-agent session). Handler calls `html_to_plain_text()` from core on save.

- [x] **No drag reordering of signatures** — Done (2026-03-21 multi-agent session). Grip handles on signature rows, `SignatureDragGripPress`/`ListDragMove`/`ListDragEnd` messages, `SettingsEvent::ReorderSignatures` emits to App which calls `db_reorder_signatures` via core CRUD.

- [x] **No delete confirmation for signatures** — Done (2026-03-21 multi-agent session). Delete shows confirmation prompt in editor overlay.

- [x] **Signatures loaded synchronously on UI thread** — Done (2026-03-21 multi-agent session). Now uses async `Task::perform` via `handlers::signatures::load_signatures_async()`.

### Rich Text Editor

**Spec:** `docs/editor/architecture.md`

- [x] **Architecture doc has stale claims** — Done (2026-03-21 multi-agent session). Architecture doc updated: test count, `html_parse` directory structure, `editor_state.rs` added to crate structure, `draw_list_marker()` wiring note corrected.
- [x] **`_last_click` dead code** — Done (2026-03-21 multi-agent session). `last_click` now tracks click state for word selection (double-click) and block selection (triple-click). `Action::DoubleClick` and `Action::TripleClick` handled by `EditorState::perform()`.
- [ ] **`prepare_move_up/down` unused at runtime** — Public functions in `widget/cursor.rs`, tested, but never called from the widget's `update()`. Simpler adjacent-block fallback used instead.
- [x] **`SetBlockAttrs` operation still missing** — Done (2026-03-21 multi-agent session). `EditOp::SetBlockAttrs` implemented for `indent_level` on `ListItem`. `TextAlignment` enum defined for future use. Self-inverse, tested.

### Main Layout

**Specs:** `docs/main-layout/problem-statement.md`, `docs/main-layout/implementation-spec.md`, `docs/main-layout/iced-implementation-spec.md`

- [x] **App-local DB shim used instead of core's `get_thread_detail()`** — Done (2026-03-21 multi-agent session). App now uses `db::threads::load_thread_detail()` which calls core's `get_thread_detail()`. Provides body text from BodyStore, ownership detection, collapsed summaries, resolved label colors, persisted attachment collapse state.
- [ ] **Calendar and pinned search CRUD bypass core** — Calendar CRUD moved to core (`crates/core/src/db/queries_extra/calendars.rs`). Schema DDL removed from `connection.rs`. **Remaining:** pinned search CRUD still in app crate.
- [ ] **Phase 3 interaction flow entirely deferred** — Keyboard shortcuts (j/k, Enter, Escape), auto-advance after archive/trash, multi-select (Shift/Ctrl+click), inline reply composer, context-dependent shortcut dispatch via `FocusedRegion`.
- [x] **No real message body rendering** — Done (2026-03-21 multi-agent session). HTML rendering via DOM-to-widget pipeline in `html_render.rs`. Complexity heuristic for fallback to plain text. CID images, link clicks, table rendering still pending.
- [ ] **No scroll virtualization** — Thread list renders all cards in `column![]` inside `scrollable`. Fixed `THREAD_CARD_HEIGHT` exists for future virtualization.
- [ ] **Right sidebar still placeholder** — Shows static "Calendar placeholder", "No pinned items" text. Calendar built as separate full-page mode instead.
- [x] **Search context line missing scope indicator** — Done (2026-03-21 multi-agent session). Shows result count on left and "All" scope-widening link on right.

### Pop-Out Message View (expanded)

**Specs:** `docs/pop-out-windows/problem-statement.md`, `docs/pop-out-windows/message-view-implementation-spec.md`

- [x] **Phase 2 (message view) mostly complete but missing fields** — Done (2026-03-21 multi-agent session). All fields now present: `cc_addresses`, `raw_source`, `rendering_mode`, `scroll_offset`, `error_banner`, position tracking (`x`, `y`), `overflow_menu_open`, `remote_content_loaded`.
- [x] **Phases 3-6 not started** — Done (2026-03-21 multi-agent session). Phase 3: `RenderingMode` enum + toggle UI. Phase 4: Overflow menu with Archive/Delete/Print/Save As (stubs). Phase 5: Session restore with `session.json`. Phase 6: Save As (.eml/.txt) to downloads dir (no file picker).
- [x] **Compose window is a UI shell** — Done (2026-03-21 multi-agent session). Full compose workflow: rich text editor (`EditorState` from `crates/rich-text-editor/`), formatting toolbar (B/I/U), signature resolution at compose creation via `assemble_compose_document()`, draft auto-save every 30s via subscription, attachment handling (stub file picker), send path (finalize HTML + save to `local_drafts`), discard confirmation with content detection. Remaining gaps: actual provider send, file picker (`rfd` not a dep), block-type format toggles (list/blockquote), link insertion dialog.
- [x] **Status bar incorrectly appears in pop-out windows** — Done (2026-03-21 multi-agent session). Status bar no longer appears in pop-out windows.
- [ ] **Body/attachment loads bypass core** — `Db::load_message_body()` and `Db::load_message_attachments()` are raw SQL in app crate.

### Status Bar

**Specs:** `docs/status-bar/problem-statement.md`, `docs/status-bar/implementation-spec.md`

- [x] **All three data pipelines unwired** — Done (2026-03-21 multi-agent session). `IcedProgressReporter` + `SyncEvent` types implemented. `Message::SyncProgress` + `handle_sync_event()` routes events to status bar methods. Remaining: connect sync orchestrator to reporter, wire `show_confirmation()` to email action handlers.
- [x] **Idle state collapses to zero height** — Done (2026-03-21 multi-agent session). Now renders fixed-height container with `STATUS_BAR_HEIGHT` (28px) and `ContainerClass::StatusBar` styling in idle state.
- [x] **Settings toggle disconnected** — Done (2026-03-21 multi-agent session). `sync_status_bar` now read by `status_bar_view()` to control visibility.
- [x] **Status bar appears in pop-out windows** — Done (2026-03-21 multi-agent session). See above.
- [x] **`ResolvedContent::Warning` missing `account_id`** — Done (2026-03-21 multi-agent session). `account_id` now embedded in resolved content.

### Sidebar (additional findings)

**Specs:** `docs/sidebar/implementation-spec.md`

- [x] **Pinned search date format diverges** — Done (2026-03-21 multi-agent session). Now uses relative time format via `format_relative_time()`.
- [x] **`SidebarEvent::CycleAccount` unreachable** — Partially done (2026-03-21 multi-agent session). Recursive `self.update()` fixed — handler now directly updates state and emits `AccountSelected`. `CycleAccount` variant retained for API compat but parent arm is dead code (maps to `Task::none()`).
- [x] **`NavigationTarget` enum implemented** — Done (2026-03-21 multi-agent session). `NavigationTarget` enum in `command_dispatch.rs` with Inbox, Starred, Snoozed, Sent, Drafts, Trash, Spam, AllMail, SmartFolder, Label, Search, PinnedSearch variants. `Message::NavigateTo` dispatch wired. Note: `selected_label: Option<String>` still exists as the underlying sidebar state — `NavigationTarget::to_label_id()` bridges the two. Full replacement of `selected_label` deferred.
- [ ] **Mixed drafts list view** — Count path handles local+server drafts, but list path only returns server-synced drafts.

### Cross-Cutting

- [ ] **Core CRUD bypassed in multiple places** — Substantially improved (2026-03-21 multi-agent session). Accounts use `create_account_sync()`. Signatures now use core CRUD functions (`db_insert/update/delete_signature`, `db_get_all_signatures`, `db_reorder_signatures`) via `DbState::from_arc()` bridge. Calendar CRUD moved to core (`crates/core/src/db/queries_extra/calendars.rs`). Contacts/groups CRUD moved to core (`contacts.rs`, `contact_groups.rs`). Schema DDL removed from app's `connection.rs`. **Remaining bypasses:** pop-out body/attachment loads (`Db::load_message_body()`, `Db::load_message_attachments()`), compose draft save (raw SQL to `local_drafts`), `load_raw_source`, pinned search CRUD, palette label queries.

- [ ] **Dead code accumulation** *(verified 2026-03-21, updated 2026-03-21 post-multi-agent session)*:

  **Resolved items (2026-03-21 multi-agent session):**
  - ~~`NavigateToLabel`~~ — now registered, dispatched, and resolved
  - ~~`SidebarEvent::CycleAccount`~~ — fixed recursive pattern, retained for API compat
  - ~~`SidebarMessage::Noop`~~ — removed
  - ~~Spam/All Mail sidebar filter code~~ — now active (backend includes these folders)
  - ~~Core CRUD for accounts~~ — `create_account_sync` now used
  - ~~`ContactSearchResult`, `ContactSearchKind`~~ — removed from `token_input.rs`
  - ~~`RecipientField`~~ — now used by `AutocompleteState` and `TokenContextMenuState`
  - ~~`search_contacts_for_autocomplete`~~ — now called from `handlers/contacts.rs`
  - ~~`AUTOCOMPLETE_MAX_HEIGHT`, `AUTOCOMPLETE_ROW_HEIGHT`~~ — now used by dropdown
  - ~~`recency_score` on `CommandMatch`~~ — now used in empty-query sort
  - ~~`PALETTE_TOP_OFFSET`~~ — now used in palette positioning
  - ~~`registry` parameter in `palette_card()`~~ — removed
  - ~~`NavNext`/`NavPrev`~~ — now use `SelectThread` for real navigation
  - ~~`SearchState.index`, `SearchState.schema`~~ — removed from struct
  - ~~`SearchState::search()`~~ — removed, only `search_with_filters()` remains
  - ~~`SearchParams.label`~~ — removed entirely
  - ~~`resolve_query_tokens`~~ — no longer re-exported (deprecated via inline migration)
  - ~~Status bar methods~~ — now reachable via `handle_sync_event()` and `show_confirmation()`
  - ~~`sync_status_bar` toggle~~ — now read by `status_bar_view()`
  - ~~`_last_click` in editor~~ — now used for double/triple click
  - ~~`SetBlockAttrs`~~ — now implemented

  **Resolved items (2026-03-21 multi-agent session, batch 2):**
  - ~~Core CRUD for signatures~~ — `handlers/signatures.rs` now delegates to `db_insert/update/delete_signature` etc. via `DbState::from_arc()`
  - ~~`NavigationTarget` enum deferred~~ — now implemented in `command_dispatch.rs` with `Message::NavigateTo` dispatch

  **Remaining dead code:**
  - `SidebarEvent::CycleAccount` parent handler — maps to `Task::none()`, can be removed
  - `PendingChord::started` — `#[allow(dead_code)]`, timeout via subscription not elapsed check
  - `prepare_move_up/down` in editor — tested infrastructure, not called from widget
  - `Db::get_thread_messages()` and `Db::get_thread_attachments()` in `connection.rs` — replaced by `load_thread_detail`
  - `group_by_thread()` duplicate — search crate is canonical, pipeline delegates to it

## UI Specs Needed

- [ ] **Design Signatures UI** — Signature management lives in Settings. Needs spec for: creating/editing/deleting signatures, per-account default signature assignment, rich text editing (or HTML), signature insertion behavior in compose (new, reply, forward).

- [ ] **Design Emoji Picker** — Shared widget used in compose, calendar event descriptions, contact notes, and anywhere text input supports emoji. Needs spec for: searchable grid, categories/tabs, recent/frequent section, skin tone selection. Separate doc at `docs/emoji-picker/problem-statement.md`.

## Contacts Surface

- [ ] **Implement full contacts crate** — The current `seen-addresses` crate (643 lines, `crates/seen-addresses/`) only tracks sender addresses seen during sync. A proper contacts implementation needs: CardDAV sync (partially started in `core/src/carddav.rs`), contact search/autocomplete, contact detail views, contact groups/labels, merge/dedup, per-provider contact sync (Google People API, Microsoft Graph contacts, LDAP). When this lands, fold `seen-addresses` into the new contacts crate — it's the same domain and shares the same DB tables.

## Code Quality

- [ ] **Decide whether Graph `raw_size = 0` should stay accepted** — Graph still lacks a clean size field for the current query path. Either keep this as an accepted cosmetic limitation or document a better fallback if one exists.

## Microsoft Graph

- [ ] **Ship a default Microsoft OAuth client ID** — Register a multi-tenant Azure AD app ("Accounts in any organizational directory and personal Microsoft accounts"), set as public client (no client secret), configure `http://localhost` redirect URI, request Mail.ReadWrite/Mail.Send/etc. scopes. Ship the client ID as a constant in `oauth.rs`. Then remove the per-account credential UI (the "Update OAuth App" flow in settings that asks users for client_id/client_secret) — users should never see this. Keep the per-account `oauth_client_id` DB column as an optional override for enterprise users who need to use their own tenant-restricted app.

## JMAP

- [ ] **JMAP for Calendars** — `jmap-client` has no calendar support (upstream Issue #3). Blocked until `jmap-client` adds calendar types. Low priority — CalDAV covers calendar sync for now.

- [ ] **Investigate JSContact / JMAP for Contacts support** — Stalwart fully implements JSContact (RFC 9553) and JMAP for Contacts (RFC 9610) with bidirectional vCard conversion. Check whether our JMAP provider crate can use native JMAP Contacts instead of falling back to CardDAV. Audit current `jmap-client` crate for contacts types and determine what (if anything) needs to be added.

## IMAP

- [ ] **QRESYNC VANISHED parsing (Phase 3)** — Send `ENABLE QRESYNC` via raw command, then `SELECT mailbox (QRESYNC (<uidvalidity> <modseq> [<known-uids>]))`. Parse `VANISHED (EARLIER) <uid-set>` untagged responses. Blocked on async-imap CHANGEDSINCE support (Issue #130).
