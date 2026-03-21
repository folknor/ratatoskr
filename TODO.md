# TODO

## Remaining Work

- [ ] **Pop-out body loading uses snippet fallback** — `message_queries.rs` queries `snippet` from the `messages` table instead of reading from BodyStore (`bodies.db`). Pop-out windows show snippet text, not full message bodies. Should use `BodyStoreState::get()` for proper zstd-decompressed body content.

- [ ] **Scroll virtualization** — Thread list renders all cards in `column![]` inside `scrollable`. Needs iced-level virtual scrolling for large mailboxes.

- [ ] **Scroll-to-selected in palette** — Arrow keys update `selected_index` but `scrollable::scroll_to` doesn't exist in our iced fork. Needs alternative approach.

- [ ] **Compose block-type format toggles** — List and blockquote buttons in formatting toolbar are stubs.

- [ ] **`responsive` for adaptive layout** — Collapse panels at narrow window sizes.

- [ ] **Per-pane minimum resize limits** — Clamp ratios on both drag and window resize.

- [ ] **Keybinding management UI (6f)** — Settings panel for rebinding. See https://nyaa.place/blog/libadwaita-1-8/

- [ ] **`prepare_move_up/down` in editor** — Tested infrastructure, not called from widget. Wire or remove.

- [ ] **Restore OS-based theme and 1.0 scale** *(Deferred until 1.0)* — Revert to `"System"` theme, persist user prefs.

## Blocked / External

- [ ] **Ship a default Microsoft OAuth client ID** — Manual Azure AD registration task.
- [ ] **JMAP for Calendars** — Blocked on `jmap-client` upstream (Issue #3). CalDAV covers this.
- [ ] **QRESYNC VANISHED parsing** — Blocked on `async-imap` upstream (Issue #130).

## Remaining Enhancements (HTML rendering)

The DOM-to-widget pipeline (`html_render.rs`) handles structural HTML. Remaining:
- [ ] CID image loading from inline image store
- [ ] Remote image loading with user consent
- [ ] Clickable links (`LinkClicked(url)` message)
- [ ] Table rendering (table-for-layout is the hardest)
- [ ] Image caching (`HashMap<String, image::Handle>`)

## Remaining Enhancements (other)

- [ ] **iced_drop for cross-container DnD** — Custom DragState works for list reorder. iced_drop needed for: compose token DnD, label drag-to-file, calendar event dragging, attachment drag zones.
- [ ] **Read receipts (outgoing)** — MDN support.
- [ ] **Inline image store eviction UI** — Settings control for store size (128 MB hardcoded).
- [ ] **Compose auto-save subscription** — `iced::time::every(30s)` for compose windows with draft_dirty set. Infrastructure exists (`DRAFT_AUTO_SAVE_INTERVAL`, `has_dirty_compose_drafts`, `auto_save_compose_drafts`) but subscription not wired in `App::subscription()`.
- [ ] **Provider push notifications** — IMAP IDLE, JMAP push, Graph webhooks, Gmail watch.
- [ ] **Connect sync orchestrator to IcedProgressReporter** — Reporter and subscription exist, sync pipeline not yet using it.

## Cross-Cutting Architecture Patterns

Living reference — follow these patterns as features are built. Keep until 1.0.

- **Generational load tracking** — Applied everywhere (nav, thread, search, palette, pop-out, sync, autocomplete). Remaining: calendar event loading on date navigation.

- **Component trait** — 7 components (Sidebar, ThreadList, ReadingPane, Settings, StatusBar, AddAccountWizard, Palette). Remaining: Compose, Calendar, Pop-out windows.

- **Token-to-Catalog theming** — Zero inline closures. Exceptions: rich text editor (builder methods), token input (renderer.fill_quad).

- **Config shadow pattern** — Implemented for app preferences (`PreferencesState`). Account editor and calendar event editor follow the pattern implicitly. Remaining: contact import wizard.

- **DOM-to-widget pipeline** — V1 in `html_render.rs`. Complexity heuristic falls back to plain text. See HTML rendering section above for remaining work.
