# TODO

## Inline Image Store Eviction

- [ ] **Wire up user-configurable eviction for `inline_images.db`** — The Rust backend has the building blocks (`prune_to_size()`, `delete_unreferenced()`, `stats()`, `clear()`), but eviction is not yet exposed in the UI.

  **What's missing**:
  1. **Settings UI**: No user-facing control for inline image store size. The 128 MB cap is hardcoded in Rust.
  2. **Scheduled eviction**: No periodic sweep to catch edge cases (e.g., if `MAX_INLINE_STORE_BYTES` is lowered in a future update).

## Iced App

- [x] **Investigate iced ecosystem projects** — Done. See `research/iced-ecosystem-survey.md` (14 repos analyzed), `docs/iced-ecosystem-cross-reference.md` (cross-referenced against all 17 specs), and `## Ecosystem Patterns` sections appended to each doc in `docs/`.

## Cross-Cutting Architecture (from ecosystem survey)

These patterns appeared across 6-8+ specs and should be adopted as foundational infrastructure before feature work builds on top of them. Full rationale in `docs/iced-ecosystem-cross-reference.md`.

- [ ] **Generational load tracking**

  Pattern established. Two generation counters implemented in `crates/app/src/main.rs`: `nav_generation` (accounts/labels/threads) and `thread_generation` (messages/attachments). All 5 current async load paths tagged; stale results discarded via guard arms.

  **Remaining sites** (apply the same pattern as these features are built):
  - Search result queries (incremental typing) — `docs/search/implementation-spec.md`, `docs/search/problem-statement.md`
  - Sidebar navigation state (`get_navigation_state()` on scope switch) — `docs/sidebar/problem-statement.md`
  - Pinned search thread metadata loading — `docs/search/pinned-searches.md`
  - Command palette option resolution (`CommandInputResolver::get_options()`) — `docs/command-palette/roadmap.md`
  - Status bar sync progress (per-account, needs `HashMap<AccountId, u64>`) — `docs/status-bar/problem-statement.md`
  - Attachment/body store loading — `docs/main-layout/problem-statement.md`
  - Calendar event loading on date navigation — `docs/calendar/problem-statement.md`

---

- [ ] **Component trait for panel isolation**

  Trait defined in `crates/app/src/component.rs` with `Message`/`Event` associated types, `update()`, `view()`, and `subscription()` (default `Subscription::none()`). Extracted components: sidebar, thread list, reading pane, settings. App dispatches via `Message::Sidebar(...)`, `Message::ThreadList(...)`, `Message::ReadingPane(...)`, `Message::Settings(...)` and routes events. Shared widget functions genericized to work with any message type.

  **Remaining panels to componentize** (as these features are built):
  - **Compose** — emits `Sent(draft_id)`, `DraftSaved(draft_id)`, `Discarded`
  - **Calendar** — emits `EventSelected(event_id)`, `DateNavigated(date)`
  - **Command palette** — emits `CommandExecuted(CommandId, CommandArgs)`, `Dismissed`
  - **Status bar** — emits `RequestReauth(account_id)`, `WarningClicked(account_id)`

---

- [ ] **Token-to-Catalog bridge for theming**

  Style migration complete. 8 class enums defined in `theme.rs`: `ButtonClass`, `ContainerClass`, `TextClass`, `RuleClass`, `TextInputClass`, `SliderClass`, `RadioClass`, `TogglerClass`, `PickListClass`. All ~30 style functions centralized behind enum dispatch with `.style()` methods returning the appropriate fn pointer or closure. ~80 call sites across all view files migrated from inline closures to named classes. Color utilities (`mix()`, `ON_AVATAR`, etc.) preserved.

  **Future enhancements** (optional, evaluate as needed):
  - **Token registry** (from shadcn-rs): `ThemeTokenRegistry` (BTreeMap) separating token definition from consumption. Useful if user-customizable themes beyond the 6-seed system are needed.
  - **Phantom-type variants** (from iced-plus): Compile-time safe button variants like `Button<Primary, Medium, Message>`. May be over-engineering — evaluate if the variant space grows significantly.

---

- [ ] **Vendor iced_drop for drag-and-drop**

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

- [ ] **Subscription orchestration pattern**

  Infrastructure established. `Component` trait has `subscription()` with default `Subscription::none()`. `App::subscription()` batches all component subscriptions alongside app-level ones (mundy appearance, window events, settings animation). All 4 components currently return `Subscription::none()`.

  **Remaining work** (as these features are built):
  - Sync pipeline events (4 providers) — use `subscription::channel` with `tokio::select!` to multiplex
  - Keyboard capture for shortcuts — `subscription::events_with` for raw key interception
  - Timer ticks (status bar cycling at 3s, auto-save at 30s, search debounce at 150ms)
  - File system watches (draft changes, attachment modifications)
  - Provider push notifications (IMAP IDLE, JMAP push, Graph webhooks, Gmail watch)

---

- [ ] **DOM-to-widget pipeline for HTML email rendering**

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

- [ ] **Thread list keyboard navigation** — Arrow Up/Down to move selection, PgUp/PgDn to jump by a page, Home/End to jump to first/last. Should scroll the selected thread into view automatically. Enter to open thread, Escape to deselect. Needs an iced keyboard event subscription in the app, gated on the thread list having focus.

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

- [ ] **Add spec scaffolding fields to `MessageViewState`** — `cc_addresses`, `rendering_mode`, `raw_source`, `scroll_offset`, window position tracking. Acceptable for V1.

### Pop-out Compose (d650308)

- [ ] **Add `cc_addresses` to `ThreadMessage` and `MessageViewState` for Reply All** — `cc_addresses` is not in `ThreadMessage` or `MessageViewState`. Reply All currently opens with no Cc recipients (previously it wrongly duplicated To recipients into Cc). Proper Reply All requires adding `cc_addresses` to both data models and populating from the DB. See TODO comments in `crates/app/src/main.rs:2281` and `:2327`.

### Contacts Management (033650c)

- [ ] **Decide save pattern for contacts** — TODO.md (below) says "contacts save immediately with no Save/Cancel — shadow pattern does NOT apply." The spec distinguishes local (immediate save) vs synced (explicit Save). Implementation uses explicit Save for all contacts. Needs decision: immediate-save for local contacts, or keep explicit Save everywhere.

- [ ] **Add account selector to contact editor** — No account selector dropdown — every contact is implicitly "Local." Spec calls for account association.

- [ ] **Add delete confirmation for contacts and groups** — Spec says "Deletion prompts for confirmation." Both contact and group delete are immediate and irreversible.

- [ ] **Replace N+1 group membership query with JOIN** — `load_contacts_filtered()` calls `load_contact_groups()` per contact. 200 contacts = 201 queries. Minor at current scale, but should be a single JOIN query.

### Emoji Picker (b15cd89)

- [ ] **Recent/frequent emoji section and skin tone selection** — TODO.md (below) says the picker needs these. Neither is implemented.

- [ ] **Flags emoji category** — Most emoji pickers include country/flag emoji. Not included in the static table.

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
