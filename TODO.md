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

  Add a `load_generation: u64` counter pattern for all async-load-then-display paths. This is the single most impactful pattern from the ecosystem survey — it appeared as critical or high-priority in 8 separate specs.

  **The problem**: When the user navigates rapidly (clicking through threads with j/k, typing in search, switching account scope), multiple async tasks are in flight simultaneously. Without generation tracking, a slow response from request N can arrive after request N+1's response, overwriting the correct current state with stale data. The current thread detail loading uses a thread_id comparison check, but this fails when the user re-selects the same thread while a load is in-flight (IDs match but the result is stale).

  **The pattern**:
  ```rust
  // In App state:
  load_generation: u64,

  // On every new request (SelectThread, search keystroke, scope switch, etc.):
  self.load_generation += 1;
  let gen = self.load_generation;
  Task::perform(async move { (gen, do_async_work().await) }, Message::Loaded)

  // In the Loaded handler:
  Message::Loaded((gen, data)) => {
      if gen != self.load_generation { return Task::none(); } // stale, discard
      // ... apply data to state
  }
  ```

  **Where to apply** (each site needs its own generation counter or a per-domain counter map):
  - Thread detail loading (`SelectThread` → `ThreadMessagesLoaded`) — `docs/main-layout/iced-implementation-spec.md`
  - Search result queries (incremental typing) — `docs/search/implementation-spec.md`, `docs/search/problem-statement.md`
  - Sidebar navigation state (`get_navigation_state()` on scope switch) — `docs/sidebar/problem-statement.md`
  - Pinned search thread metadata loading — `docs/search/pinned-searches.md`
  - Command palette option resolution (`CommandInputResolver::get_options()`) — `docs/command-palette/roadmap.md`
  - Status bar sync progress (per-account, needs a `HashMap<AccountId, u64>` rather than single counter) — `docs/status-bar/problem-statement.md`
  - Attachment/body store loading — `docs/main-layout/problem-statement.md`
  - Calendar event loading on date navigation — `docs/calendar/problem-statement.md`

  **Edge case**: For the status bar, bloom's single-counter pattern needs extension to a per-account generation map, since multiple accounts sync concurrently and each has independent progress. Use `HashMap<String, u64>` keyed by account_id.

  **Interaction with other items**: The Component trait (below) affects where generation counters live. If the thread list is a Component, its generation counter lives in the component's state, not `App`. The subscription orchestration pattern determines how async results are delivered back (via `Task::perform` vs `subscription::channel`).

  **Reference**: bloom `research/bloom/src/app.rs` line 157 (counter field), line 161 (increment + tag in update), result check in the Loaded handler.

---

- [ ] **Component trait for panel isolation**

  Define a `Component` trait in `crates/app/` that each major panel implements. This is the most important architectural decision for the iced app — without it, the `Message` enum will grow to 50-100+ variants as features land, making `update()` an unmaintainable monolith.

  **The problem**: Every panel's interactions (sidebar navigation, thread list selection, reading pane actions, compose editor, calendar, command palette, status bar, settings) currently share a single flat `Message` enum and a single `update()` function. Adding a feature like calendar or compose means adding 10-20 new message variants to the same enum. At scale this becomes a classic "god object" — every change to any panel touches the same function, and the type system can't enforce which messages belong to which panel.

  **The pattern** (from trebuchet `research/trebuchet/src/app.rs` lines 95-132):
  ```rust
  pub trait Component {
      type Message;
      type Event;

      fn update(&mut self, message: Self::Message) -> (Task<Self::Message>, Option<Self::Event>);
      fn view(&self) -> Element<'_, Self::Message>;
      fn subscription(&self) -> Subscription<Self::Message>;
  }
  ```

  Each panel implements `Component` with its own `Message` type. The panel's `update()` handles internal state transitions and optionally emits an `Event` for cross-panel communication. The top-level `App` holds all components and dispatches:

  ```rust
  // Top-level App::update():
  AppMessage::ThreadList(msg) => {
      let (task, event) = self.thread_list.update(msg);
      if let Some(evt) = event {
          match evt {
              ThreadListEvent::ThreadSelected(id) => {
                  // trigger reading pane load, with generation tracking
              }
              ThreadListEvent::BulkAction(action) => { ... }
          }
      }
      task.map(AppMessage::ThreadList)
  }
  ```

  **Panels to componentize**:
  - **Sidebar** — emits `ScopeChanged(AccountScope)`, `NavigatedTo(FolderDestination)`, `LabelSelected(label_id)`
  - **Thread list** — emits `ThreadSelected(thread_id)`, `BulkAction(action)`, `SearchExecuted(query)`
  - **Reading pane** — emits `Reply(thread_id)`, `Archive(thread_id)`, `LabelToggled(thread_id, label_id)`
  - **Compose** — emits `Sent(draft_id)`, `DraftSaved(draft_id)`, `Discarded`
  - **Calendar** — emits `EventSelected(event_id)`, `DateNavigated(date)`
  - **Command palette** — emits `CommandExecuted(CommandId, CommandArgs)`, `Dismissed`
  - **Status bar** — emits `RequestReauth(account_id)`, `WarningClicked(account_id)`
  - **Settings** — emits `SettingsChanged(diff)`, `AccountReauthRequested(account_id)`

  **Cross-panel coupling**: Ratatoskr's panels are not fully independent — thread selection in the sidebar/thread list drives content in the reading pane, keyboard shortcuts in the reading pane trigger actions that update the thread list. The Component trait handles this via the `Event` type: the sidebar emits an event, `App::update()` routes it to the reading pane. This is one level of indirection, not the N-level nesting of a flat message enum.

  **Where referenced**: `docs/calendar/problem-statement.md`, `docs/main-layout/problem-statement.md`, `docs/sidebar/problem-statement.md`, `docs/status-bar/problem-statement.md`, `docs/search/problem-statement.md`, `docs/command-palette/problem-statement.md`, `docs/pop-out-windows/problem-statement.md`, `docs/contacts/problem-statement.md`.

  **Interaction with other items**: Generation counters move into each component's state. The subscription orchestration pattern means each component provides `subscription()` and `App` batches them. The config shadow lives in the settings component or in whichever component owns the edit flow.

  **Reference**: trebuchet `research/trebuchet/src/app.rs` lines 95-132 (trait definition, dispatch loop). Also Lumin's `Module` trait at `research/Lumin/src/module.rs` for an alternative take (trait with `run()` for activation).

---

- [ ] **Token-to-Catalog bridge for theming**

  Create an `AppTheme` newtype wrapping iced's `Theme` that implements all widget `Catalog` traits, bridging the existing seed-based palette into iced's styling system. This eliminates inline style closures and ensures visual consistency across all panels.

  **The problem**: Currently, ~30 style functions in `theme.rs` return inline `Style` structs by reaching into `theme.palette()`. Every widget call site uses `.style(|theme, status| { ... })` closures. This works but has drawbacks: (1) style logic is scattered across closures in view code, not centralized; (2) you can't share a style definition between widgets without passing closures around; (3) iced's `Catalog` system (the idiomatic approach since 0.13) is designed for `.class(MyClass::Primary)` which is more readable and composable.

  **The pattern** (from iced-plus `research/iced-plus/iced_plus_theme/src/theme.rs`):
  ```rust
  pub struct AppTheme(pub Theme);

  impl button::Catalog for AppTheme {
      type Class<'a> = ButtonClass;

      fn style(&self, class: &ButtonClass, status: button::Status) -> button::Style {
          let p = self.0.palette();
          match class {
              ButtonClass::Primary => { /* use p.primary.base, etc. */ }
              ButtonClass::Nav { active } => { /* nav_button style */ }
              ButtonClass::Ghost => { /* ghost button style */ }
              // ... all existing style functions become match arms
          }
      }
  }
  ```

  **What migrates**:
  - All `nav_button`, `thread_card`, `badge`, `popover`, `dropdown_item`, `toolbar_button` etc. style functions become `ButtonClass` / `ContainerClass` / `TextClass` variants
  - The `mix()` helper and `ON_AVATAR` semantic color stay in `theme.rs` as utilities
  - The spacing scale and layout constants in `layout.rs` are unaffected — they're already centralized

  **Token registry** (from shadcn-rs `research/shadcn-rs/crates/iced-shadcn/src/tokens.rs`): shadcn-rs goes further with a `ThemeTokenRegistry` (BTreeMap<String, Color|f32|u64|String>) that separates token definition from consumption. This level of indirection is useful if we want user-customizable themes beyond the 6-seed system, but is optional for the initial implementation. Start with the `AppTheme` Catalog bridge; add the token registry later if needed.

  **Phantom-type variants** (from iced-plus `research/iced-plus/iced_plus_components/src/button.rs`): For compile-time safety, button variants can use phantom types: `Button<Primary, Medium, Message>` where `Primary` and `Medium` are unit types implementing sealed `ButtonVariant` and `ButtonSize` traits. This enables monomorphization and prevents invalid combinations. Worth evaluating but may be over-engineering for our needs — start with a simple enum and escalate if the variant space grows.

  **Where referenced**: Every UI spec. Most directly: `docs/iced-ecosystem-decisions.md`, `docs/main-layout/problem-statement.md`, `docs/sidebar/problem-statement.md`, `docs/status-bar/problem-statement.md`, `docs/calendar/problem-statement.md`.

  **Interaction with other items**: The Component trait means each panel's `view()` receives `&AppTheme` (or it's available via iced's theme system). The Catalog bridge is what makes `.class(ButtonClass::Nav { active: true })` work inside component view functions.

  **Reference**: iced-plus `research/iced-plus/iced_plus_theme/src/theme.rs` (Catalog bridge), shadcn-rs `research/shadcn-rs/crates/iced-shadcn/src/tokens.rs` (token registry), `research/shadcn-rs/crates/iced-shadcn/src/theme.rs` (Palette struct with 35 tokens).

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

  Establish a standard pattern for how the app composes all background event streams into a single `Subscription::batch()`. This is infrastructure that every async feature depends on.

  **The problem**: The app already has subscriptions (OS appearance changes via `mundy`, window resize events). As features land, more subscriptions arrive: sync pipeline events (4 providers), keyboard capture for shortcuts, timer ticks (status bar cycling at 3s, auto-save at 30s, debounce for search at 150ms), file system watches (draft changes, attachment modifications), and provider push notifications (IMAP IDLE, JMAP push, Graph webhooks, Gmail watch). Without a standard pattern, each feature adds its subscription ad-hoc, leading to inconsistent error handling, unclear ownership, and subscription lifecycle bugs (the existing mundy D-Bus freeze is likely a subscription lifecycle issue).

  **The pattern** (from pikeru `research/pikeru/` and rustcast `research/rustcast/src/app/tile/elm.rs` lines 158-237):

  Each subsystem provides a function returning `Subscription<SubsystemMessage>`:
  ```rust
  // Each component/subsystem:
  fn subscription(&self) -> Subscription<SyncMessage> { ... }
  fn subscription(&self) -> Subscription<StatusBarMessage> { ... }

  // Top-level App::subscription():
  fn subscription(&self) -> Subscription<AppMessage> {
      Subscription::batch([
          self.sync_pipeline.subscription().map(AppMessage::Sync),
          self.status_bar.subscription().map(AppMessage::StatusBar),
          self.keyboard.subscription().map(AppMessage::Keyboard),
          self.appearance.subscription().map(AppMessage::Appearance),
          // ... each component's subscription
      ])
  }
  ```

  For subsystems that multiplex multiple async sources (e.g., sync across 4 providers + push notification channels), use `subscription::channel` with `tokio::select!`:
  ```rust
  fn sync_subscription(providers: &[ProviderState]) -> Subscription<SyncMessage> {
      subscription::channel(Id::unique(), 100, |mut sender| async move {
          loop {
              tokio::select! {
                  progress = gmail_rx.recv() => sender.send(SyncMessage::Progress(progress)).await,
                  progress = graph_rx.recv() => sender.send(SyncMessage::Progress(progress)).await,
                  push = push_notification_rx.recv() => sender.send(SyncMessage::PushReceived(push)).await,
                  _ = tokio::time::sleep(POLL_INTERVAL) => sender.send(SyncMessage::PollTick).await,
              }
          }
      })
  }
  ```

  **Where referenced**: `docs/calendar/problem-statement.md`, `docs/main-layout/problem-statement.md`, `docs/search/implementation-spec.md`, `docs/status-bar/problem-statement.md`, `docs/accounts/problem-statement.md`, `docs/pop-out-windows/problem-statement.md`.

  **Interaction with other items**: The Component trait means each component owns its subscription. The generational load tracking interacts with how async results are delivered — `Task::perform` for one-shot loads, `subscription::channel` for streaming updates.

  **Reference**: pikeru (subscription::channel + tokio::select! for concurrent thumbnail loading, file watching, and search), rustcast (`research/rustcast/src/app/tile/elm.rs` lines 158-237 for Subscription::batch).

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

- [ ] **Make sidebar fixed-width (not resizable)** — The sidebar should be a fixed width, not draggable. Remove the sidebar resize divider and any sidebar width persistence from `WindowState`. The sidebar width is a constant in `layout.rs`, not a user preference.

- [ ] **Per-pane minimum resize limits** — PaneGrid currently uses a uniform `min_size(120)` for all panes. Should have per-pane minimums (e.g., sidebar can't go below 150px, thread list below 200px, contact sidebar below 180px). Requires clamping ratios in the `PaneResized` handler since PaneGrid only supports a single global minimum. Decide on actual values after visual testing.

- [ ] **Animated toggler widget** — Port libcosmic's slerp-based toggle animation for smooth sliding pill togglers. Current iced built-in toggler snaps instantly. libcosmic's version (`research/libcosmic/src/widget/toggler.rs`) uses `anim::slerp()` with configurable duration (200ms default), interpolating knob position per-frame via `RedrawRequested`. ~150-200 LOC to port.

- [ ] **`responsive` for adaptive layout** — Wrap PaneGrid in `iced::widget::responsive` to collapse panels at narrow window sizes (e.g., hide contact sidebar below 900px, stack sidebar over thread list below 600px).

- [ ] **Keybinding display and edit UI** — Need to redo the Settings/Shortcuts UI. Take a look at https://nyaa.place/blog/libadwaita-1-8/

- [ ] **UI freezes after ~20 minutes with settings open** — App hangs completely with no stdout/stderr. Prime suspect is the `mundy` subscription (`appearance.rs`) holding a D-Bus connection that may drop or block over time. Bisect by disabling subscriptions one-by-one to isolate.

- [ ] **License display/multiline static text row** — Need to be able to click links and make text selectable/copyable in license display widgets. Needs its own base row type.

- [ ] **Restore OS-based theme and 1.0 scale** — `SettingsState::default()` currently hardcodes `theme: "Light"` for development convenience. Revert to `theme: "System"` once UI prototyping is done, and persist user preferences to disk.

- [ ] **Wire up system font detection** — `crates/system-fonts/` is built and working (queries xdg-desktop-portal on Linux, SystemParametersInfo on Windows) but not wired into the app. Two phases:
  1. **UI font**: On startup, call `SystemFonts::detect().await`. If a UI font is found and it's available on the system, apply it via iced's `font::set_defaults` task. Fall back to bundled Inter if detection fails or font isn't installed. This just confirms/overrides the default app font.
  2. **Document font** (separate, later): The detected document font (e.g., TisaPro) should be used for email body text and other long-form content. This requires threading a separate font through the thread detail view and message body widgets. Not straightforward right now — email bodies are HTML processed by the sanitizer pipeline, not iced text widgets, so the document font may only apply once we have native body rendering. Add as a font setting in the settings UI too (let users override the detected system fonts).

- [ ] **Thread list keyboard navigation** — Arrow Up/Down to move selection, PgUp/PgDn to jump by a page, Home/End to jump to first/last. Should scroll the selected thread into view automatically. Enter to open thread, Escape to deselect. Needs an iced keyboard event subscription in the app, gated on the thread list having focus.

- [ ] **Scrollbars must shift layout, not overlay** — When a scrollbar appears (e.g., content grows beyond the viewport), it must push the content inward rather than overlaying existing UI elements. Overlay scrollbars cause text/buttons to be hidden behind the scrollbar track. Ensure all `scrollable` widgets use a mode that reserves space for the scrollbar or accounts for its width in the layout.

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
