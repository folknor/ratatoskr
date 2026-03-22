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

### Labels Unification — `docs/labels-unification/problem-statement.md`

Phases 1-5 complete (schema, Exchange/IMAP/JMAP sync, local dispatch, sidebar). Remaining:

- [ ] **Label pills in reading pane** — Display tag-type labels as colored pills on expanded message headers. Data now in `thread_labels` via unified sync.
- [ ] **Label picker overlay** — Triggered from reading pane or command palette. Lists all available tag-type labels with colors for apply/remove.
- [ ] **Provider write-back for label operations** — Local apply/remove works (Phase 4). Actual provider API calls (Exchange category set, IMAP STORE +FLAGS, JMAP keyword set) awaits provider client access from app layer.
- [ ] **Phase 6: Deprecate old tables** — Drop `categories` and `message_categories` tables once all sync paths are verified on the unified system.
- [ ] **IMAP PERMANENTFLAGS graceful degradation** — Verify keyword write-back survives server restrictions. UI should indicate when an IMAP account doesn't support custom keywords.

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

EWS SOAP client, autodiscover routing, offline sync, IMAP NAMESPACE public folders, DB schema all done. Sidebar pins done (2026-03-22). Remaining:

- [x] **Sidebar pin rendering** — "PUBLIC FOLDERS" section with folder icon, unread count, loaded at boot from `public_folder_pins`. *(2026-03-22)*
- [ ] **Thread loading on selection** — App handler for `PublicFolderSelected` event to load threads from `public_folder_items` into thread list.
- [ ] **Public folder browser** — Lazy-load tree widget for browsing the hierarchy and pinning folders. Uses existing `browse_public_folders()` API.
- [ ] **Reply/post wiring** — Connect compose to `CreateItem` EWS operation for replies and posts to public folders.

### Shared Mailboxes — `docs/roadmap/shared-mailboxes.md`

Exchange Graph sync + Autodiscover + sidebar integration done. Remaining:

- [x] **Sidebar scope dropdown** — Shared mailboxes auto-populate from Autodiscover results, rendered with users icon. *(2026-03-22)*
- [ ] **Thread loading on selection** — App handler for `SharedMailboxSelected` event to load navigation and threads for the selected shared mailbox.
- [ ] **Compose identity auto-selection** — When replying from shared mailbox context, auto-set From to shared mailbox address. `send_as_shared_mailbox()` and `send_on_behalf_of()` APIs exist.
- [ ] **Gmail delegation support** — Blocked (API limitation). Send-As aliases work.
- [ ] **Per-mailbox sync depth config** — Currently hardcoded to 30 days. No per-mailbox setting.

### JMAP Sharing — `docs/roadmap/jmap-sharing.md`

All 6 backend phases complete (discovery, sync, rights, subscription, notifications, identity resolution). Remaining app-crate UI integration:

- [ ] **Rights gating on action buttons** — `NavigationFolder.rights` (`MailboxRightsInfo`) is populated from synced `myRights`. App should check `may_delete`, `may_rename`, `may_submit` etc. before showing action buttons. Especially important for shared/read-only mailboxes where the user lacks write permissions.
- [ ] **Subscription toggle in sidebar** — `NavigationFolder.is_subscribed` is populated from JMAP `isSubscribed`. App needs a UI toggle (context menu or button) on shared account labels that calls `JmapOps::subscribe_mailbox()` / `unsubscribe_mailbox()`. These accept an optional `jmap_account_id` for shared accounts.
- [ ] **Compose identity auto-selection from shared mailbox** — `shared_mailbox_sync_state.email_address` is resolved via JMAP Principals (Phase 6). When replying from a shared mailbox context, compose should query `sync_state::get_shared_mailbox_email()` and auto-set From. Also check `may_submit` from the mailbox rights before offering the identity.

### IMAP CONDSTORE/QRESYNC — `docs/roadmap/imap-condstore-qresync.md`

Phases 1-2 complete. Phase 3 blocked on upstream.

- [ ] **QRESYNC VANISHED parsing** — Blocked on `async-imap` upstream (Issue #130). UID-based deletion detection works as workaround.

## Blocked / External

- [ ] **Ship a default Microsoft OAuth client ID** — Manual Azure AD registration task.
- [ ] **JMAP for Calendars** — Blocked on `jmap-client` upstream (Issue #3). CalDAV covers this.
- [ ] **QRESYNC VANISHED parsing** — Blocked on `async-imap` upstream (Issue #130). See above.

## Remaining Enhancements (HTML rendering)

The DOM-to-widget pipeline (`html_render.rs`) handles structural HTML but has significant fidelity gaps. Remaining:
- [ ] **Inline text formatting** — `<strong>`, `<b>`, `<em>`, `<i>`, `<u>`, `<s>`, `<code>` (inline) all ignored. Everything renders as plain text. Needs a `Vec<Span>` model per block or `iced::widget::rich_text`.
- [ ] **Link rendering + click handling** — `<a href>` tags treated as plain text. URLs not extracted. Need `href` extraction, visual link styling, and `LinkClicked(url)` message emission.
- [ ] **`<br>` handling** — Currently splits into separate paragraphs (extra vertical spacing). Should insert a line break within the current paragraph.
- [ ] **HTML entity decoding** — Only 8 named entities decoded. Missing: numeric entities (`&#123;`, `&#x7B;`), common named entities (`&mdash;`, `&ndash;`, `&hellip;`, `&copy;`, etc.).
- [ ] CID image loading from inline image store (`InlineImageStoreState` exists in stores crate, not wired to renderer)
- [ ] Remote image loading with user consent (`block_remote_images` setting exists but disconnected from `render_html` — function signature needs context parameter)
- [ ] Table rendering (table-for-layout is the hardest — no `<table>`/`<tr>`/`<td>` handling at all)
- [ ] Image caching (`HashMap<String, image::Handle>`) — no `iced::widget::image` usage in app crate

## Remaining Enhancements (other)

- [ ] **iced_drop for cross-container DnD** — Custom DragState works for list reorder. iced_drop needed for: compose token DnD, label drag-to-file, calendar event dragging, attachment drag zones.
- [ ] **Read receipts (outgoing)** — MDN support. See `docs/roadmap/tracking-blocking.md`.
- [ ] **Inline image store eviction UI** — Settings control for store size (128 MB hardcoded).

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

- **Generational load tracking** — Applied to: nav, thread, search, palette, pop-out, sync, autocomplete, add-account wizard. Verified 2026-03-22. Gaps:
  - Calendar event loading on date navigation (confirmed missing — race condition on rapid date changes)
  - Search typeahead dynamic queries (`dispatch_typeahead_query` has no generation counter — stale suggestions possible on fast typing)

- **Component trait** — 7 components: Sidebar, ThreadList, ReadingPane, Settings, StatusBar, AddAccountWizard, Palette. All verified 2026-03-22. Non-components use free functions + App handler methods: Compose, Calendar, Pop-out windows. Conversion optional — current pattern works.

- **Token-to-Catalog theming** — Zero inline closure violations. Verified 2026-03-22. Exceptions: rich text editor (builder methods), token input (renderer.fill_quad).

- **Config shadow pattern** — Formal: `PreferencesState`. Implicit (clone-on-open): Account editor, Contact editor, Group editor, Calendar event editor, Signature editor. Missing: contact import wizard (creation wizard — value of formal shadow debatable).

- **DOM-to-widget pipeline** — V1 in `html_render.rs`. Complexity heuristic (table depth >5, style tags >2) falls back to plain text. Used in reading pane only (NOT in pop-out message view). See HTML rendering section above for remaining work — significant fidelity gaps (no inline formatting, no links, no images).
