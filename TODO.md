# TODO

## Remaining Work

- [ ] **Pop-out body loading uses snippet fallback** — `message_queries.rs` queries `snippet` from the `messages` table instead of reading from BodyStore (`bodies.db`). Pop-out windows show snippet text, not full message bodies. Should use `BodyStoreState::get()` for proper zstd-decompressed body content.

- [ ] **Scroll virtualization** — Thread list renders all cards in `column![]` inside `scrollable`. Needs iced-level virtual scrolling for large mailboxes.

- [ ] **Scroll-to-selected in palette** — Arrow keys update `selected_index` but `scrollable::scroll_to` doesn't exist in our iced fork. Needs alternative approach.

- [ ] **Compose block-type format toggles** — List and blockquote buttons in formatting toolbar are stubs.

- [ ] **`responsive` for adaptive layout** — Collapse panels at narrow window sizes.

- [ ] **Per-pane minimum resize limits** — Clamp ratios on both drag and window resize.

- [ ] **Keybinding management UI (Slice 6f)** — Settings panel for viewing, searching, and rebinding shortcuts. Backend ready (override persistence, conflict detection, set/unbind/reset APIs). See `docs/command-palette/app-integration-spec.md` § Slice 6f.

- [ ] **`prepare_move_up/down` in editor** — Tested infrastructure, not called from widget. Wire or remove.

- [ ] **Restore OS-based theme and 1.0 scale** *(Deferred until 1.0)* — Revert to `"System"` theme, persist user prefs.

## Roadmap Features — Remaining Work

Features with backend complete but UI or integration work remaining. Each references its roadmap spec.

### Categories — `docs/roadmap/categories.md`

Backend complete (all 4 providers sync, 25-preset color model, ProviderOps apply/remove). Remaining:

- [ ] **Category picker UI** — Color palette grid widget exists (`widgets.rs::color_palette_grid`). Need a picker overlay that lists account categories with colors, triggered from reading pane or command palette.
- [ ] **Category badges on messages/threads** — Display category names with colors on thread list cards and expanded message headers. Data available via `message_categories` join table.
- [ ] **Apply/Remove Category commands** — `CommandId::EmailAddLabel` and `EmailRemoveLabel` exist but route to label operations. Need parallel `ApplyCategory`/`RemoveCategory` commands, or reuse the label commands with a category-aware resolver.
- [ ] **IMAP keyword write-back** — `apply_category`/`remove_category` on IMAP provider use `set_keyword_if_supported()` but need verification that write-back survives PERMANENTFLAGS restrictions gracefully.

### Tracking Blocking — `docs/roadmap/tracking-blocking.md`

Sanitization pipeline, MDN detection, tracking pixel detection, URL cleaning all done. Remaining:

- [ ] **Read receipt prompt UI** — `read_receipt_policy` table and `mdn.rs` policy resolution exist. Need UI prompt when opening a message with `mdn_requested=true`: "Send read receipt?" with per-sender/per-account policy options (ask/always/never).
- [ ] **Read receipt policy management in Settings** — Settings panel for configuring default MDN policy per account and per-sender overrides.
- [x] **Remote image strip in sanitizer** — `sanitize_html_body_with_image_policy()` strips remote `<img src="http(s)://...">` unless sender is allowlisted. Preserves cid:/data: URIs. *(2026-03-22)*
- [x] **Link tracking visual indicators** — `is_known_tracker()` exported from tracking_pixels, `has_tracking_params()` added to url_cleaning. UI renderer can annotate links. *(2026-03-22)*
- [x] **AMP HTML blocking** — `strip_amp_elements()` removes 14 amp-* elements and amp4email attribute. Integrated into `sanitize_html_body_with_image_policy()`. *(2026-03-22)*

### Cloud Attachments — `docs/roadmap/cloud-attachments.md`

OneDrive and Google Drive upload both implemented. Remaining:

- [x] **Incoming cloud link detection** — `detect_cloud_links()` already exists in `core/cloud_attachments.rs` with full pattern matching for OneDrive, GDrive, Dropbox, Box. 12 tests. Needs wiring to sync pipeline via `insert_incoming_cloud_links()`. *(verified 2026-03-22)*
- [ ] **Compose UI for cloud attachment flow** — Size threshold detection in compose, prompt to upload to cloud, upload progress indicator, insert link into message body. Orchestration logic exists in `core/cloud_attachments.rs`.
- [ ] **Offline upload queue** — Queue uploads when offline, retry when connectivity returns.
- [x] **JMAP/IMAP graceful degradation** — `supports_cloud_upload()` + `large_attachment_warning()` + `LARGE_ATTACHMENT_THRESHOLD` (25 MB) in `core/cloud_attachments.rs`. UI needs to call these during compose attach flow. *(2026-03-22)*
- [x] **`cloud_attachments` DB table** — Already exists (migration 39). 14 columns, 2 indexes, full CRUD in `core/cloud_attachments.rs`. *(verified 2026-03-22)*

### Public Folders — `docs/roadmap/public-folders.md`

EWS SOAP client, autodiscover routing, offline sync, IMAP NAMESPACE public folders, DB schema all done. Remaining:

- [ ] **UI integration** — Public folder browser in sidebar (collapsible tree), message display from public folder threads, compose reply-to-public-folder. Exchange-only feature.

### Shared Mailboxes — `docs/roadmap/shared-mailboxes.md`

Exchange Graph read/write/sync isolation and IMAP ACL/NAMESPACE done. Remaining:

- [ ] **Gmail delegation support** — Gmail delegation API for reading/sending from delegated accounts. Requires `gmail.settings.sharing` scope.
- [ ] **JMAP Sharing support** — JMAP `ShareNotification` and `Principal` types for shared mailbox access. Blocked on server support maturity.
- [ ] **Shared mailbox UI** — Sidebar section or scope dropdown entries for shared/delegated mailboxes. Account-switcher integration.

### IMAP CONDSTORE/QRESYNC — `docs/roadmap/imap-condstore-qresync.md`

Phases 1-2 complete. Phase 3 blocked on upstream.

- [ ] **QRESYNC VANISHED parsing** — Blocked on `async-imap` upstream (Issue #130). UID-based deletion detection works as workaround.

## Blocked / External

- [ ] **Ship a default Microsoft OAuth client ID** — Manual Azure AD registration task.
- [ ] **JMAP for Calendars** — Blocked on `jmap-client` upstream (Issue #3). CalDAV covers this.
- [ ] **QRESYNC VANISHED parsing** — Blocked on `async-imap` upstream (Issue #130). See above.

## Remaining Enhancements (HTML rendering)

The DOM-to-widget pipeline (`html_render.rs`) handles structural HTML. Remaining:
- [ ] CID image loading from inline image store
- [ ] Remote image loading with user consent (integrates with tracking-blocking allowlist)
- [ ] Clickable links (`LinkClicked(url)` message)
- [ ] Table rendering (table-for-layout is the hardest)
- [ ] Image caching (`HashMap<String, image::Handle>`)

## Remaining Enhancements (other)

- [ ] **iced_drop for cross-container DnD** — Custom DragState works for list reorder. iced_drop needed for: compose token DnD, label drag-to-file, calendar event dragging, attachment drag zones.
- [ ] **Read receipts (outgoing)** — MDN support. See `docs/roadmap/tracking-blocking.md`.
- [ ] **Inline image store eviction UI** — Settings control for store size (128 MB hardcoded).
- [ ] **Compose auto-save subscription** — `iced::time::every(30s)` for compose windows with draft_dirty set. Infrastructure exists (`DRAFT_AUTO_SAVE_INTERVAL`, `has_dirty_compose_drafts`, `auto_save_compose_drafts`) but subscription not wired in `App::subscription()`.
- [ ] **Provider push notifications** — IMAP IDLE, JMAP push, Graph webhooks, Gmail watch.
- [ ] **Connect sync orchestrator to IcedProgressReporter** — Reporter and subscription exist, sync pipeline not yet using it. Once connected, also wire `begin_sync_generation`/`prune_stale_sync` for stale progress cleanup.
- [ ] **Token expiry → status bar warning** — `WarningKind::TokenExpiry` type, UI, and click-to-reauth handler all exist. Missing: auth error detection path that calls `status_bar.set_warning()` with `TokenExpiry` when OAuth refresh fails or tokens expire.
- [ ] **Pop-out HTML rendering** — SimpleHtml/OriginalHtml modes in message view pop-out fall back to plain text. Depends on the DOM-to-widget pipeline (`html_render.rs`) being wired into the pop-out view. Tracked separately in the HTML rendering section above.
- [ ] **Pop-out Print** — OS print dialog integration for message view and compose pop-out windows. Platform-specific, no iced precedent. Needs investigation.
- [ ] **Pop-out default rendering mode from settings** — `MessageViewState` hardcodes `RenderingMode::default()` (SimpleHtml). Should load from a system-wide user preference. Needs a settings field + plumbing to pass it into `from_thread_message()` and `from_session_entry()`.
- [ ] **Signature: draft restoration with signature state** — Draft save does not persist `signature_separator_index` or `active_signature_id`. On draft reopen, signature position in the document is not reconstructed.
- [ ] **Signature: per-account default dropdown in Account Settings** — Account editor overlay has no signature dropdown for selecting the default signature for an account.
- [ ] **Signature: edit detection flag** — No dirty/edited tracking in `SignatureEditorState` for confirming unsaved changes on close.
- [ ] **GAL directory API calls** — `gal_cache` table and autocomplete integration exist. Missing: actual Graph `/users` and Google Directory API calls to populate the cache. Awaits sync orchestrator providing provider client access. See `docs/contacts/problem-statement.md` § GAL Caching.
- [ ] **CardDAV contact write-back** — CardDAV client supports PROPFIND/REPORT/GET but not PUT/DELETE. Need vCard generation + PUT method for pushing contact edits to CardDAV servers. See `docs/contacts/problem-statement.md`.
- [ ] **Provider write-back HTTP calls** — `dispatch_provider_write_back()` scaffolded for Google/Graph (body builders + server info lookups exist). JMAP `ContactCard/set` fully implemented. Missing: actual HTTP dispatch for Google (`PATCH /v1/{resourceName}:updateContact`) and Graph (`PATCH /me/contacts/{id}`). Awaits provider client access from handlers.

## Cross-Cutting Architecture Patterns

Living reference — follow these patterns as features are built. Keep until 1.0.

- **Generational load tracking** — Applied everywhere (nav, thread, search, palette, pop-out, sync, autocomplete). Remaining: calendar event loading on date navigation.

- **Component trait** — 7 components (Sidebar, ThreadList, ReadingPane, Settings, StatusBar, AddAccountWizard, Palette). Remaining: Compose, Calendar, Pop-out windows.

- **Token-to-Catalog theming** — Zero inline closures. Exceptions: rich text editor (builder methods), token input (renderer.fill_quad).

- **Config shadow pattern** — Implemented for app preferences (`PreferencesState`). Account editor and calendar event editor follow the pattern implicitly. Remaining: contact import wizard.

- **DOM-to-widget pipeline** — V1 in `html_render.rs`. Complexity heuristic falls back to plain text. See HTML rendering section above for remaining work.
