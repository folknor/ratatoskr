# TODO

## Usability Blockers

These prevent the app from functioning as an email client.

- [ ] **Provider send** — Compose saves to `local_drafts` with `sync_status = 'finalized'` but never actually sends via IMAP/SMTP/Gmail API/Graph. Need to wire `ProviderOps::send()` to pick up finalized drafts and transmit them. This is the single biggest gap for usability.

- [ ] **Calendar provider sync (Layer 5)** — Google Calendar (API), Graph (CalDAV), and generic CalDAV sync. Events are local-only right now. Core calendar CRUD is ready (`crates/core/src/db/queries_extra/calendars.rs`). Need sync pipeline integration similar to email sync.

- [ ] **Re-authentication flow (Phase 7)** — When OAuth tokens expire, there's no way to re-auth. Status bar has `AccountWarning`/`WarningKind::TokenExpiry` types and `RequestReauth` event, but no `ReauthWizard` to handle it. The compose agent noted that `AccountHealth::compute_health()` always returns `Healthy` until `token_expires_at` and `is_active` are plumbed from DB.

- [ ] **Connect sync orchestrator to `IcedProgressReporter`** — The reporter type, `SyncEvent` enum, channel factory, and `Message::SyncProgress` handler all exist. What's missing is the sync pipeline actually using the reporter to emit events. Without this, the status bar never shows sync progress or connection warnings.

## UX Polish

Important for a good experience, not blocking core functionality.

- [ ] **Right sidebar** — Still shows static "Calendar placeholder", "No pinned items" text. Should have mini-calendar with today's agenda and pinned/starred items. Calendar was built as a separate full-page mode instead.

- [ ] **Multi-select + auto-advance (Phase 3 interaction)** — Shift+click range select, Ctrl+click toggle select. Auto-advance to next thread after archive/trash. Inline reply composer. Context-dependent shortcut dispatch via `FocusedRegion`. Thread list keyboard nav (j/k/Enter/Escape) is done, but these interaction items remain.

- [ ] **Scroll virtualization** — Thread list renders all cards in `column![]` inside `scrollable`. Fixed `THREAD_CARD_HEIGHT` exists for future virtualization. Currently fast with 1000 threads but won't scale. Needs iced-level virtual scrolling (only render visible rows) rather than application-level pagination — see detailed analysis in previous attempts.

- [ ] **Undo/redo for all text inputs** — iced's built-in `TextInput` and `TextEditor` do not support Ctrl+Z/Ctrl+Y. Every text field should support basic undo/redo. The rich text editor already has full undo/redo via `EditOp` stack.

  **Approach**: Use `dissimilar` crate for compact diff-based undo history per input. Wrap in `UndoableText { current: String, history: VecDeque<Patch>, position: usize }`. For pill-based inputs (To/Cc/Bcc, labels), define `UndoableList<T>` tracking add/remove operations.

  **Reference**: cedilla `research/cedilla/src/app/core/history.rs` (dissimilar-based undo with circular buffer).

- [ ] **Config shadow pattern for settings/edit flows** — Any UI that edits persistent state should clone into an `editing_*` shadow on open. Prevents partial saves, enables live preview, provides trivial change detection.

  **Where to apply**: Account settings, app preferences, calendar event editor, contact import wizard, pinned search edit-in-place. **Exception**: contacts spec says fields save immediately (no Save/Cancel).

  **Reference**: bloom `research/bloom/src/app.rs` lines 38, 196, 402.

- [ ] **`responsive` for adaptive layout** — Wrap layout in `iced::widget::responsive` to collapse panels at narrow window sizes (e.g., hide right sidebar below 900px, stack sidebar over thread list below 600px).

- [ ] **Per-pane minimum resize limits** — Custom divider currently uses a single `SIDEBAR_MIN_WIDTH` / `THREAD_LIST_MIN_WIDTH`. Should have per-pane minimums with ratio clamping on both `DividerDragMove` and `WindowResized`.

- [ ] **Make sidebar fixed-width (not resizable)** *(Deferred until later)* — Remove sidebar resize divider and width persistence from `WindowState`. Sidebar width is a constant, not a user preference.

- [ ] **No scroll-to-selected in palette results** — Arrow keys update `selected_index` but `scrollable::scroll_to` doesn't exist in our iced fork. Selected item can scroll off-screen. Needs alternative approach (widget operation or state manipulation).

- [ ] **Mixed drafts list view** — Count path handles local+server drafts, but list path only returns server-synced drafts. Needs design decision on union type vs promotion approach.

- [ ] **Replace `pre_search_threads` with `PreSearchView`** — Spec recommends navigation-target-based restoration instead of thread vector cloning.

- [ ] **Compose: remaining gaps** — File picker (needs `rfd` dependency), block-type format toggles (list/blockquote in toolbar), link insertion dialog, provider send (see Usability Blockers above).

## Infrastructure

Cross-cutting work that enables or improves multiple features.

- [ ] **Wire iced_drop to features** — Crate vendored at `crates/iced-drop/` but not wired to any UI. Needed for: thread reordering, label drag-to-file, account reordering in settings, compose token DnD (To↔Cc↔Bcc), group editor member DnD, calendar event dragging, attachment drag zones. Calendar needs augmentation for continuous position mapping (pixel offset → time).

- [ ] **Remaining core CRUD bypasses** — Pop-out body/attachment loads (`load_message_body`, `load_message_attachments`), compose draft save (raw SQL to `local_drafts`), `load_raw_source`, pinned search CRUD, palette label queries. Calendar, contacts, signatures, and accounts are resolved.

- [ ] **Keybinding persistence + management UI (6e-6f)** — `BindingTable` supports overrides in memory but they're not saved/loaded. No settings panel for rebinding. Take a look at https://nyaa.place/blog/libadwaita-1-8/

- [ ] **Wire up system font detection (Phase 2 — Document font)** — Detected document font should be used for email body text. Requires threading a separate font through thread detail view and message body widgets. Add as a font setting in settings UI.

- [ ] **Cache `thread_ids` on `PinnedSearch` struct** — Spec defines `thread_ids: Vec<(String, String)>` loaded lazily. Implementation always re-queries. Minor — DB query is fast.

- [ ] **Pinned searches Phase 2 features** — Staleness label, `SearchBarState` type, periodic expiry subscription.

- [ ] **Search Phases 2 + 4** — Phase 2: smart folder CRUD via command palette (save search as smart folder). Phase 4: "Search here" scoped search from sidebar right-click context menu.

- [ ] **`color_palette_grid` not reusable** — Hardcoded to `AddAccountMessage::SelectColor(i)`. Should be generic widget in `widgets.rs` with `on_select` callback.

- [ ] **License display/multiline static text row** — Need to click links and make text selectable/copyable in license display widgets. Needs its own base row type.

## Tier 4 — Additive Features

New features that add capability without blocking core email functionality.

- [ ] **Contact import crate** — CSV/XLSX/vCard import with encoding detection, column mapping, preview, and import wizard UI. Spec at `docs/contacts/import-spec.md`. Fold `seen-addresses` crate into the new contacts crate.

- [ ] **Full contacts crate** — CardDAV sync (partially started in `core/src/carddav.rs`), contact detail views, merge/dedup, per-provider contact sync (Google People API, Microsoft Graph contacts, LDAP).

- [ ] **Emoji picker** — Searchable grid with categories/tabs, recent/frequent section, skin tone selection. Separate doc at `docs/emoji-picker/problem-statement.md`. Missing: recent/frequent section, skin tone selection, flags emoji category.

- [ ] **Read receipts (outgoing)** — MDN support. No spec needed.

## Blocked / External

Dependent on upstream changes or manual external tasks.

- [ ] **Ship a default Microsoft OAuth client ID** — Register a multi-tenant Azure AD app, set as public client, configure `http://localhost` redirect URI. Ship client ID as constant in `oauth.rs`. Remove per-account credential UI. Keep `oauth_client_id` DB column as enterprise override. *Manual registration task.*

- [ ] **JMAP for Calendars** — `jmap-client` has no calendar support (upstream Issue #3). Blocked until upstream adds calendar types. Low priority — CalDAV covers calendar sync.

- [ ] **Investigate JSContact / JMAP for Contacts** — Stalwart implements JSContact (RFC 9553) and JMAP for Contacts (RFC 9610). Check whether JMAP provider crate can use native JMAP Contacts instead of CardDAV.

- [ ] **QRESYNC VANISHED parsing (Phase 3)** — Blocked on async-imap CHANGEDSINCE support (Issue #130).

## Code Quality / Minor

- [ ] **Decide whether Graph `raw_size = 0` should stay accepted** — Graph lacks a clean size field. Accept as cosmetic limitation or find better fallback.

- [ ] **Magic numbers** — `add_account.rs` (`icon::mail().size(48.0)`, `.padding(2)`, stroke width `2.0`, alpha `0.35`), token input (`0.54` char width heuristic, cursor offsets `2.0`/`4.0`, placeholder alpha `0.4`), sidebar `truncate_query` magic `28`.

- [ ] **`ContactSearchResult` types in app crate instead of core** — Should be in `crates/core/src/contacts/search.rs` per spec.

- [ ] **Palette not componentized** — Spec defines `PaletteEvent` enum but palette logic is inline in `App::handle_palette()`.

- [ ] **`SidebarEvent::CycleAccount` parent handler is dead code** — Maps to `Task::none()`, can be removed.

- [ ] **`prepare_move_up/down` unused at runtime** — Tested infrastructure in editor, not called from widget. Keep as infrastructure or remove.

- [ ] **Decide save pattern for contacts** — Spec distinguishes local (immediate save) vs synced (explicit Save). Implementation uses explicit Save for all. Needs decision.

- [ ] **Restore OS-based theme and 1.0 scale** *(Deferred until 1.0 release)* — Revert `SettingsState::default()` from `"Light"` to `"System"`. Persist user preferences to disk.

## Inline Image Store Eviction

- [ ] **Wire up user-configurable eviction for `inline_images.db`** — Backend has `prune_to_size()`, `delete_unreferenced()`, `stats()`, `clear()`. Missing: settings UI for store size (128 MB cap is hardcoded), scheduled eviction sweep.

## Cross-Cutting Architecture Patterns

These are living reference documentation — patterns to follow as features are built. They stay until 1.0.

---

- [ ] **Generational load tracking**

  Pattern established and well-applied. Counters: `nav_generation`, `thread_generation`, `search_generation` in App. `option_load_generation` on PaletteState. `pop_out_generation` for pop-out windows. `sync_generations` map on StatusBar. `search_generation` on AutocompleteState.

  **Remaining sites** (apply as built):
  - Calendar event loading on date navigation
  - Attachment/body store loading (if converted to async)

---

- [ ] **Component trait for panel isolation**

  Six components: Sidebar, ThreadList, ReadingPane, Settings, StatusBar, AddAccountWizard.

  **Remaining panels to componentize:**
  - Compose — uses free functions, not Component trait
  - Calendar — state on App directly
  - Command palette — tight coupling to registry/resolver, may be intentional
  - Pop-out windows — free functions, inconsistent with main window

---

- [ ] **Token-to-Catalog bridge for theming**

  Zero inline style closures across all UI files. 8+ class enums in `theme.rs`.

  **Known exceptions** (architecturally correct):
  - Rich text editor passes colors via builder methods (standalone widget)
  - Token input draws via `renderer.fill_quad()` (custom `advanced::Widget`)

---

- [ ] **Subscription orchestration pattern**

  Well-established. Active: keyboard listener, chord timeout, search debounce, status bar cycling, settings animation, compose auto-save (30s). `IcedProgressReporter` built but not connected to sync.

  **Remaining:**
  - Connect sync orchestrator to reporter
  - File system watches (draft changes, attachment modifications)
  - Provider push notifications (IMAP IDLE, JMAP push, Graph webhooks, Gmail watch)
  - GAL polling refresh (hourly, for contacts)

---

- [ ] **DOM-to-widget pipeline for HTML email rendering**

  V1 implemented in `crates/app/src/ui/html_render.rs`. Handles paragraphs, headings, lists, blockquotes, pre, hr, image alt text. Complexity heuristic falls back to plain text for CSS-heavy emails.

  **Remaining:** CID image loading from inline image store, remote image loading with user consent, clickable links (`LinkClicked(url)` message), table rendering (table-for-layout is the hardest problem), image caching (`HashMap<String, image::Handle>`).

  **Fallback strategy:** For emails exceeding the complexity heuristic, consider litehtml (C++ lightweight HTML renderer) or CEF (full Chromium). The DOM-to-widget pipeline covers ~60-80% of email by volume.

---

- [ ] **iced_drop drag-and-drop**

  Vendored at `crates/iced-drop/`. Provides `Droppable<Message>` widget wrapper with drop zone detection via `Operation` trait. 623 lines, adapted to our iced fork.

  **Where needed:** Thread reordering, label drag-to-file, account reordering, compose token DnD, group editor member DnD, calendar event dragging, attachment drag zones.

  **Calendar gap:** iced_drop handles discrete drop zones. Calendar time grid needs continuous position mapping (pixel offset → time).

---

## Completed (2026-03-21)

<details>
<summary>66 items completed across 19 agents in 2 rounds</summary>

### Editor
- [x] Architecture doc stale claims fixed (test count, html_parse directory, editor_state.rs, draw_list_marker)
- [x] `_last_click` → `last_click` with double/triple click detection
- [x] `SetBlockAttrs` operation implemented (indent_level, TextAlignment enum)

### Sidebar
- [x] Spam/All Mail folders wired (added to SIDEBAR_UNIVERSAL_FOLDERS)
- [x] O(n²) HashMap rebuild fixed (build once, pass in)
- [x] `SidebarMessage::Noop` removed
- [x] CycleAccount recursive pattern fixed
- [x] Pinned search relative dates, query-primary layout, chevron styling

### Status Bar
- [x] IcedProgressReporter + SyncEvent + create_sync_progress_channel()
- [x] Idle state fixed height (STATUS_BAR_HEIGHT container)
- [x] Settings toggle wired (sync_status_bar read by status_bar_view)
- [x] Status bar removed from pop-out windows
- [x] ResolvedContent::Warning.account_id added
- [x] Generational tracking (sync_generations map)

### Command Palette
- [x] NavigateToLabel registered, dispatched, resolved
- [x] provider_kind resolved from account data
- [x] current_view detection for all 14+ view types
- [x] Pending chord indicator badge
- [x] Snooze/DateTime preset options
- [x] recency_score wired into empty-query sort
- [x] Inline text style closures replaced with TextClass variants
- [x] registry parameter removed from palette_card

### Search
- [x] Unified pipeline wired (Tantivy → SQL fallback → LIKE)
- [x] Multi-value from/to OR semantics
- [x] SearchParams.label removed
- [x] SearchState dead fields removed
- [x] group_by_thread deduplicated
- [x] Smart folder token migration
- [x] delete_all_pinned_searches
- [x] SearchBlur unfocus wired
- [x] Search typeahead (Phase 3) — CursorContext, dropdown, keyboard nav

### Signatures
- [x] DbSignature extended (7 columns)
- [x] html_to_plain_text implemented
- [x] Core CRUD wired (db_insert/update/delete_signature via DbState::from_arc)
- [x] Rich text editor in signature editor
- [x] Formatting toolbar (B/I/U/S, lists, blockquote)
- [x] Drag reorder with grip handles
- [x] Delete confirmation
- [x] Async loading via Task::perform
- [x] active_signature_id in ComposeDocumentAssembly
- [x] finalize_compose_html / finalize_compose_plain_text

### Accounts
- [x] Real discovery wired (ratatoskr_core::discovery::discover)
- [x] Core CRUD for creation (create_account_sync)
- [x] Protocol selection UI (interactive cards)
- [x] OAuth flow (OAuthComplete, RetryOAuth, authorize_with_provider)
- [x] Credential validation (Validating step, IMAP connection test)
- [x] Account editor in settings (slide-in overlay)
- [x] AccountHealth enum + health dots
- [x] Account deletion with confirmation
- [x] Duplicate account detection

### Contacts
- [x] Autocomplete dropdown + AutocompleteState
- [x] RFC 5322 paste parser (token_input_parse.rs)
- [x] Arrow key navigation between tokens
- [x] Right-click context menu
- [x] Group/GAL search in autocomplete
- [x] Recency ranking (last_contacted_at DESC)
- [x] N+1 query → JOIN (GROUP_CONCAT)
- [x] Delete confirmation for contacts and groups
- [x] Account selector on contact editor
- [x] Group token visual distinction

### Main Layout
- [x] Core's get_thread_detail() wired (body store, ownership, label colors, attachment persistence)
- [x] HTML email rendering pipeline (DOM-to-widget in html_render.rs)
- [x] Thread list keyboard navigation (j/k/Enter/Escape/Home/End)
- [x] Search scope "All" indicator
- [x] Per-message Reply/ReplyAll/Forward actions

### Pop-Out Windows
- [x] RenderingMode enum + toggle UI
- [x] Overflow menu (Archive/Delete/Print/Save As)
- [x] Session restore (session.json)
- [x] Save As (.eml/.txt)
- [x] cc_addresses on ThreadMessage + MessageViewState
- [x] Error banner for failed body loads
- [x] Per-window generation tracking (pop_out_generation)
- [x] Discard confirmation with content detection

### Compose
- [x] Rich text editor (EditorState from rich-text-editor crate)
- [x] Formatting toolbar (B/I/U/S)
- [x] Signature resolution at compose creation
- [x] Draft auto-save (30s subscription)
- [x] Send path (finalize HTML + save to local_drafts)
- [x] Attachment tracking (stub file picker)

### Cross-Cutting
- [x] NavigationTarget enum (19 variants, dispatch wired)
- [x] Thread state flags (is_pinned, is_muted, in_trash, in_spam, is_draft)
- [x] Vendor iced_drop (adapted to iced fork)
- [x] Calendar CRUD → core
- [x] Contacts/groups CRUD → core
- [x] Schema DDL removed from connection.rs → core migrations
- [x] Pop-out body loads → BodyStore
- [x] Scrollbars shift layout (SCROLLBAR_SPACING)
- [x] System font detection Phase 1

</details>
