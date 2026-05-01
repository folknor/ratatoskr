# TODO

## Remaining Work

- [ ] **Expand dev-seed** - The dev-seed script needs to create Smart Folders, contact groups, VIP contacts. Also attachments should be actual files. Needs to create actual signatures for contacts, both HTML and simple ones. Needs to create more emails with links and other non-text content. Needs to create fake shared accounts and mailboxes.

- [ ] **dev-seed calendars** - Obvious.

- [ ] **Settings/People** - The contacts and group lists here need to conform much closer to the spec at docs/contacts/problem-statement.md. We're quite a ways off.

- [ ] **Slide-in editor: confirm-discard on unsaved changes** - The contact and group editor slide-ins can currently close without warning whenever the slide-in "wants" to dismiss: clicking the Back button, pressing Escape, clicking a different settings tab in the left nav, or any other future close path. If the editor is dirty (new contact/group with content, or an existing one with edits not yet saved/auto-saved), we should pop a "Discard unsaved changes?" confirmation first. Needs a single chokepoint so every dismiss path runs the same check rather than per-call-site retrofits. Same pattern likely belongs on the account editor and signature editor sheets too.

- [ ] **Settings/Notifications** - VIP Senders should move to contact editing, and this should be a toggle button here.

- [ ] **Compose window help text** - The help text in the compose windows to/cc/bcc fields ("Add recipients...") is not vertically centered in the input field. Note: `token_input.rs::draw_text_area` already draws the placeholder with `align_y: Vertical::Center` inside a `TOKEN_HEIGHT` box and `PAD_TOKEN_INPUT` is symmetric (top=4, bottom=4), so on paper it should be centered. The misalignment must come from elsewhere - possibly the field's overall layout height vs. the `TOKEN_HEIGHT` slot, or font metrics asymmetry. Needs investigation with rendered measurements.

- [ ] **Settings/Accounts: Edit Account** - This section needs rework.

- [ ] **Codebase-wide chevron unification** - The chevron icon should be unified across the codebase; we use different chevron icons in different places (dropdowns, accordions, popovers, etc.). Audit all chevron uses and standardize on one icon set + sizing scale.

- [ ] **Attachment saving** - Should remember last folder. Ideally last folder per thread ID.

- [ ] **Collapse individual expanded messages** - Chevron now points up (fixed: added `icon::chevron_up()` at U+E070, swapped in `widgets::expanded_message_card`). Remaining: the button needs a new place to live - probably a very long, thin button that stretches across the entire horizontal space at the top of the message frame. This needs to be unified with the Attachments panel collapsing, which is currently taking up too much vertical space; also too much padding above the Attachments section.

- [ ] **Settings row hover (group editor members)** - The group editor's `group_member_section` (`crates/app/src/ui/settings/tabs.rs`) builds its section manually instead of via `section()`, so its rows still use uniform `RADIUS_SM` hover corners and don't pick up the position-aware styling that the rest of the settings rows now use. Convert it to use `section_untitled` with `RowBuilder` items, or have the helper accept and propagate `RowPosition`.

- [ ] **Settings/Composing: Signatures** - This section needs work.

- [ ] **Standardized popup/dropdown/modal** - Currently setting dropdowns, various modal dialogs (the Settings slide-in, Add Account modal, etc) use various methods to dim/control/disable/dismiss. We need standardized controls for all this. For example the Add Account modal currently dims the background (rest of the window), but it doesn't prevent interaction with any controls - even controls that are actually directly below it can still be interacted with. We need the same treatment as the Settings slide-in that does in fact disable things behind it. See `docs/ui/overlay-standardization-plan.md` for the implementation plan.

- [ ] **Cursor bleed-through on blocking overlays** - When a Sheet or Modal is active, hovering over the blocker area may still show pointer/hand cursors from widgets in the base layer underneath. The `mouse_area` blocker sets `.interaction(mouse::Interaction::default())` but iced's `stack!` may composite `mouse_interaction` from all layers. May be pre-existing. Investigate whether iced's stack respects the topmost layer's cursor or falls through.

- [ ] **Focus trapping for modals and sheets** - iced does not natively support focus trapping. Modal and Sheet surfaces should trap Tab/Shift-Tab focus within their content, but currently focus can escape to widgets behind the blocker. If iced adds focus trapping support, `modal_overlay()` (see `docs/ui/overlay-standardization-plan.md`) is the single place to wire it in. Until then, this is a known contract gap.

- [ ] **Calendar event detail popover → AnchoredOverlay** - `calendar::popover_stack()` is the only anchored surface still using a hand-rolled `stack![]` instead of the `AnchoredOverlay` primitive. Target behavior: anchor near the clicked event pill using `anchor_point`. Requires capturing click coordinates in `CalendarPopover::EventDetail` (not currently stored). See `docs/ui/overlay-standardization-plan.md` deferred work.

- [ ] **Settings help tooltip → Ratatoskr Tooltip primitive** - The settings help surface uses `AnchoredOverlay` but is semantically a tooltip (hover-triggered, non-blocking, informational). The legacy pinned/sticky behavior has been removed. Should migrate to a Ratatoskr Tooltip primitive once one exists. Independent of the overlay standardization effort.

- [ ] **Escape key audit for overlay surfaces** - Verify every Modal surface dismisses on Escape. Verify no Sheet surface dismisses on Escape. Verify calendar modals (event detail, editor, delete confirm, discard confirm) all handle Escape correctly. Includes nested-modal case: Escape from ConfirmDiscard should return to the editor (preserving the draft), not close everything. Requires routing Escape through the calendar handler for workflow-aware dispatch rather than the current blunt `is_some() → None` in main.rs. Mechanical verification pass, best done after `modal_overlay()` has been in use for a bit.

- [ ] **Calendar move semantics for existing events** - The calendar picker is disabled for `EditingEvent` because moving an event between calendars requires provider-specific support (some providers need delete+create). When provider calendar-move APIs are implemented, re-enable the picker for existing events and update `account_id` ownership logic in the `CalendarSelected` handler accordingly.

- [ ] **Link hover URL disclosure (email content)** - Links in email bodies need either a tooltip that shows the destination URL or status-bar disclosure. Decision still pending.

- [ ] **Link context menu (email content)** - Right-clicking a link in an email body should offer actions like Copy Link and related link operations.

- [ ] **Pop out message viewer body rendering** - The current pills for selecting Plain/Simple/Original/Source need to move. The spec currently doesn't say clearly where they should go. This needs to be resolved first.

- [ ] **Pop out message viewer body rendering toggle buttons** - Plain / Simple HTML / Original HTML still all render the plain-text body identically because `html_render.rs` isn't wired into the pop-out yet (tracked under "Pop-out HTML rendering" below). Source mode synthesizes a usable pseudo-`.eml` from headers + body, but the original on-the-wire MIME framing is lost - faithful Source needs the "Raw message source store" entry below.

- [ ] **Message box / toast notification system** - Generic modal message box and/or toast notification infrastructure for the app. Needed for: compose draft save failure on close (currently silently aborts the close with no user feedback), action service retry exhaustion warnings, and any future error/confirmation flows. Should support at least: transient toasts (auto-dismiss), persistent error banners, and modal confirmation dialogs.

- [ ] **Starred thread card background** - The golden tint on starred thread cards uses a fixed `mix()` ratio (`STARRED_BG_ALPHA`) which may not look right across all themes. Needs a GPU-level blend/shader effect that adapts to the theme's background luminance so the starred highlight reads consistently in both light and dark themes.

- [ ] **Star icon: need filled variant** *(Deferred - blocked on sluggrs SVG icon rendering)* - Lucide only has outline icons (confirmed: `star` U+E176, `star-half` U+E20B, no filled variant in the bundled font). Currently uses Unicode `*` as a stopgap, which causes size mismatch and visual jank. Will be resolved by switching to real SVG vector icon rendering (recently implemented in sluggrs, our text renderer) - filled and outline star SVGs can both ship and the toggle just swaps the asset. The button should also not change background color on toggle - just the icon fill.

- [ ] **Autocomplete: cross-field drag-and-drop** - Drag detection works but drop cancels. Context menu "Move to" is the workaround. Needs ghost token rendering and target field hit-testing.
- [ ] **Autocomplete: reuse beyond compose** - Widget only used in compose. Calendar attendee picker and group editor could reuse it.
- [ ] **Contact pills on recipients** - Per `docs/pop-out-windows/problem-statement.md`: recipients in To/Cc fields should appear as plain text but become contact pills on hover, revealing an inline edit button for quick contact editing. Applies to: reading pane message headers, pop-out message view, compose window recipient display. Currently recipients are plain text everywhere with no hover interaction. Needs: (1) a contact pill widget that blends with background at rest and reveals pill styling + edit button on hover, (2) display name resolution from the contact system (name → email fallback chain), (3) wiring to the existing `EditContact` flow that opens the settings contact editor.

- [ ] **Action service: user-facing retry status** *(Deferred - blocked on toast system)* - Backend complete: `db_pending_ops_count()`, `db_pending_ops_failed_count()`, `db_pending_ops_retry_failed()` all exist. Zero UI wiring. Needs the toast/notification system (first TODO item) before this can surface "N actions pending retry" badges or "Archive failed after 10 retries" persistent notifications. Without this, users have no visibility into silently diverged state.

- [ ] **Action service: native provider batching** *(Deferred - low ROI until bulk ops are common)* - `batch_execute` dispatches per-thread `MailOperation` sequentially within each account. Provider reuse per account already eliminated client construction overhead - remaining cost is network latency (one round-trip per thread). Native batching (Gmail batch API, Graph `/$batch`, JMAP `Email/set`, IMAP multi-UID STORE) would reduce 50 round-trips to 1-3 for bulk operations. `PartialEq` on `MailOperation` enables grouping identical operations; the executor contract already specifies regrouping semantics. Implementation deferred until bulk operations on 50+ threads become a real user workflow.

- [ ] **Raw message source store** - The Source view in the pop-out message viewer currently synthesizes a pseudo-`.eml` from parsed headers + body store content (best effort, not faithful to the original MIME framing). For real on-the-wire raw source we'd need a new `raw_source_store` (zstd-compressed blob store, parallel to `body_store` / inline image store, keyed by `(account_id, message_id)`) populated during sync. Each provider needs a separate fetch path: Gmail `format=raw`, JMAP blob endpoint, Graph `/messages/{id}/$value`, IMAP `BODY[]` (currently parsed-on-the-fly and discarded). Without it, DKIM/ARC verification, the original Received chain, original Content-Transfer-Encoding, MIME boundary strings, header order/casing, and address comments all stay lost - reassembly from the parsed columns can't reproduce any of those byte-exactly. Storage cost is real at the project's "150+ GB cached mailbox" target, so the rollout should consider scope (only newer messages? evict on archive? per-account opt-in?) before turning capture on by default.

- [ ] **Sync-task cancellation on account deletion** - Delete flow removes DB data but doesn't cancel in-flight sync tasks. Stale sync completions could write to deleted account state.

- [ ] **Scroll virtualization** - Thread list renders all cards in `column![]` inside `scrollable`. Needs iced-level virtual scrolling for large mailboxes.

- [ ] **Scroll-to-selected in palette** - Arrow keys update `selected_index` but `scrollable::scroll_to` doesn't exist in our iced fork. Needs alternative approach.

- [ ] **`responsive` for adaptive layout** - Collapse panels at narrow window sizes.

- [ ] **Keybinding management UI (Slice 6f)** - Settings panel for viewing, searching, and rebinding shortcuts. Backend ready (override persistence, conflict detection, set/unbind/reset APIs). See `docs/cmdk/app-integration-spec.md` § Slice 6f.

- [ ] **Restore OS-based theme and 1.0 scale** *(Deferred until 1.0)* - Revert to `"System"` theme, persist user prefs.

- [ ] **Bundle SQLite for release builds** *(Deferred until 1.0)* - Re-enable `rusqlite/bundled` feature for release builds so the binary ships a known SQLite version with FTS5 guaranteed. Dev builds use system libsqlite3 for faster compiles.


- [ ] **Reconsider sidebar layout** *(Deferred until right before 1.0)* - Currently the spec says: (1) sidebar should not show any Labels section when "All Accounts" is selected, (2) when a single account is selected, only labels belonging to that account should be shown, and (3) that for providers that have a "folder" concept, the users folders should show in the Labels section. We might need to re-think all 3.

## Roadmap Features - Remaining Work

Features with backend complete but UI or integration work remaining. Each references its roadmap spec.

### Labels Unification - `docs/labels-unification/problem-statement.md`

Phases 1-6 complete (backend unified). **10 discrepancies remain** - see `docs/labels-unification/discrepancies.md`. Critical: command palette rejects non-Gmail label operations, palette queries use legacy type filtering. Also:

- [ ] **Label picker overlay** - Triggered from reading pane or command palette. Lists all available tag-type labels with colors for apply/remove.

### Search - `docs/search/problem-statement.md`

Backend pipeline exists (parser, SQL builder, Tantivy, unified router). **29 discrepancies remain** - see `docs/search/discrepancies.md`. Critical: combined path applies free text in SQL before Tantivy ranking, Tantivy-only results show wrong message metadata, date boundaries inconsistent across engines. Also typeahead, pinned search lifecycle, and smart folder management gaps.

- [ ] **Promote pinned search to Smart Folder** - Sidebar pinned searches need an action that converts a pinned search into a Smart Folder.

### Calendar - `docs/calendar/problem-statement.md`

Views, editor, pop-out, sidebar all partially implemented. **39 discrepancies remain** - see `docs/calendar/discrepancies.md`. Critical: new event creation broken (no calendar selector), calendar sync never triggered from app, timezone handling treats everything as UTC, two competing CalDAV implementations. Also drag interactions, RSVP actions, reminder system, meeting invite detection.

**Calendar UI issues (observed 2026-04-04):**

Event popover (quick-glance card):
- [ ] Position is wrong - currently right-aligned in the calendar view, should anchor near the clicked event pill
- [ ] Styling needs work (visual polish pass)
- [ ] Clicking a different event pill while the popover is open just closes the popover instead of closing and immediately opening the new event's popover. Root cause: `popover_stack` (`crates/app/src/ui/calendar.rs`) renders a full-viewport `mouse_area` backdrop with `on_press(ClosePopover)` on top of the calendar base, which swallows the click before it reaches the underlying event pill. Will be resolved by the deferred AnchoredOverlay migration (see "Calendar event detail popover -> AnchoredOverlay" above) - anchoring the popover near the pill removes the need for a click-blocking backdrop.

Event detail modal:
- [ ] Needs significant visual and layout work

Event editor modal:
- [ ] Does not adhere to the editor spec at all - needs a full implementation pass
- [ ] Discarding changes doesn't work (but doesn't save changes either, so no data loss)

Month view:
- [ ] Event pill overflow still not filling actual available space - current fix uses CALENDAR_CELL_MIN_HEIGHT, so cells only pack events to the minimum height; when the window is taller, cells grow but still cap at the same event count. Needs a layout-aware widget that measures actual rendered cell height.

Week view:
- [ ] All-day events are not laid out properly at the top of the day columns

### Generic OAuth - `docs/generic-oauth/problem-statement.md`

Core OIDC discovery + OAUTHBEARER implemented. **6 discrepancies remain** - see `docs/generic-oauth/discrepancies.md`. Critical: re-auth broken for generic/OIDC providers (registry lookup fails for non-built-in provider IDs). Also no manual issuer URL flow, no client ID entry, JMAP OAuth unsupported.

### Chats - `docs/chats/problem-statement.md`

Backend plumbing complete (schema, sync, core APIs, timeline view). Feature unreachable by users. **7 discrepancies remain** - see `docs/chats/discrepancies.md`. Critical: no sidebar entry point, no body text rendering, no mark-read, no inline compose.

- [ ] **Per-bubble user-account indicator** - Spec (`docs/chats/problem-statement.md` § "What about multi-account contacts?", L201-205) calls for "a subtle account indicator (the account's color dot or abbreviation)" on each chat bubble so the user can tell which of *their own* accounts a given message belongs to when a contact spans multiple accounts (e.g. work + personal). Currently unimplemented - bubbles render with no account marker. Likely a small colored dot using `account.account_color` near the bubble corner, or a short abbreviation tag - low-visual-weight, since most chats are single-account in practice.

- [ ] **Conversation party name/identity in chat view** - The spec is silent on showing the contact's name *within* the chat view itself; the only on-screen identity cue today is the sidebar pill (which can scroll out of frame). This is a spec gap, not a deferred feature. We probably want a slim header bar above the timeline with the contact's name + avatar (and email under it) so the active chat is identifiable at-a-glance. Resolve the spec gap before implementing - decide whether it's a sticky header, a bubble-level sender label, or a toolbar-style row, then update `docs/chats/problem-statement.md` § "A view mode, not a message type".

### Tracking Blocking - `docs/roadmap/tracking-blocking.md`

Sanitization pipeline, MDN detection, tracking pixel detection, URL cleaning all done. Remaining:

- [ ] **Read receipt prompt UI** - `read_receipt_policy` table and `mdn.rs` policy resolution exist. Need UI prompt when opening a message with `mdn_requested=true`: "Send read receipt?" with per-sender/per-account policy options (ask/always/never).
- [ ] **Read receipt policy management in Settings** - Settings panel for configuring default MDN policy per account and per-sender overrides.

### Cloud Attachments - `docs/roadmap/cloud-attachments.md`

OneDrive and Google Drive upload both implemented. Remaining:

- [ ] **Compose UI for cloud attachment flow** - Size threshold detection in compose, prompt to upload to cloud, upload progress indicator, insert link into message body. Orchestration logic exists in `core/cloud_attachments.rs`.
- [ ] **Offline upload queue** - Queue uploads when offline, retry when connectivity returns.

### Public Folders - `docs/roadmap/public-folders.md`

EWS SOAP client, autodiscover routing, offline sync, IMAP NAMESPACE public folders, DB schema all done. Sidebar pins done (2026-03-22). Remaining:

- [ ] **Public folder browser** - Lazy-load tree widget for browsing the hierarchy and pinning folders. Uses existing `browse_public_folders()` API.
- [ ] **Reply/post wiring** - Connect compose to `CreateItem` EWS operation for replies and posts to public folders.

### Shared Mailboxes - `docs/roadmap/shared-mailboxes.md`

Exchange Graph sync + Autodiscover + sidebar integration done. Remaining:

- [ ] **Gmail delegation support** - Blocked (API limitation). Send-As aliases work.
- [ ] **Per-mailbox sync depth config** - Currently hardcoded to 30 days. No per-mailbox setting.

### JMAP Sharing - `docs/roadmap/jmap-sharing.md`

All 6 backend phases complete (discovery, sync, rights, subscription, notifications, identity resolution). Remaining app-crate UI integration:

- [ ] **Subscription toggle in sidebar** - `NavigationFolder.is_subscribed` is populated from JMAP `isSubscribed`. App needs a UI toggle (context menu or button) on shared account labels that calls `JmapOps::subscribe_mailbox()` / `unsubscribe_mailbox()`. These accept an optional `jmap_account_id` for shared accounts.

### Labels - `docs/labels-unification/problem-statement.md`

- [ ] **Label picker UI** - Overlay for applying/removing tag-type labels from messages. Triggered from reading pane or command palette. Lists all available labels with colors. Provider dispatch via `add_tag()`/`remove_tag()`.

### Mentions - `docs/roadmap/mentions.md`

- [ ] **Compose @-autocomplete** - Detect `@` in compose editor, show floating contact picker, insert `@Display Name` text, auto-add to To/CC if not already a recipient. Works identically across all providers (cosmetic markup only).

### Scheduled Send - `docs/roadmap/scheduled-send.md`

Backend complete (server delegation + overdue handling). Missing UI.

- [ ] **Schedule picker UI** - Date/time picker in compose toolbar. Delegates to Exchange (deferred delivery) or JMAP (FUTURERELEASE) server-side, falls back to local timer for Gmail/IMAP.
- [ ] **"Scheduled" virtual folder** - Virtual folder view showing all pending scheduled messages across accounts with edit/reschedule/cancel.

### Signatures - `docs/roadmap/signatures.md`

Backend complete (Gmail + JMAP sync). Exchange fetch permanently blocked (no public API, Microsoft confirmed no plans).

### Auto-Responses - `docs/auto-responses/problem-statement.md`

Read/write API complete on all 3 providers. Remaining:

- [ ] **Auto-reply settings UI** - Per-account editor in settings. Toggle, date pickers, message editor, audience selector. Internal/external tabs for Exchange only. Provider HTML must be sanitized before rendering (stored unsanitized in DB).

### IMAP CONDSTORE/QRESYNC - `docs/roadmap/imap-condstore-qresync.md`

Phases 1-2 complete. Phase 3 blocked on upstream.

- [ ] **QRESYNC VANISHED parsing** - Blocked on `async-imap` upstream (Issue #130). UID-based deletion detection works as workaround.

## Blocked / External

- [ ] **Ship a default Microsoft OAuth client ID** - Manual Azure AD registration task.
- [ ] **QRESYNC VANISHED parsing** - Blocked on `async-imap` upstream (Issue #130). See above.

## Remaining Enhancements (HTML rendering)

The DOM-to-widget pipeline (`html_render.rs`) handles structural HTML but has significant fidelity gaps. Remaining:
- [ ] **Inline text formatting** - `<strong>`, `<b>`, `<em>`, `<i>`, `<u>`, `<s>`, `<code>` (inline) all ignored. Everything renders as plain text. Needs a `Vec<Span>` model per block or `iced::widget::rich_text`.
- [ ] Remote image loading with user consent (`block_remote_images` setting exists but disconnected from `render_html` - function signature needs context parameter)
- [ ] Table rendering (table-for-layout is the hardest - no `<table>`/`<tr>`/`<td>` handling at all)
- [ ] Image caching (`HashMap<String, image::Handle>`) - no `iced::widget::image` usage in app crate

## Security / Bug Findings (unfixed)

- [ ] **Microsoft ID token not signature-verified** - JWT payload is base64-decoded and trusted for email/name claims without verifying the signature. Token comes over TLS from Microsoft, but a MITM or compromised endpoint could inject arbitrary identity claims.

## Remaining Enhancements (other)

- [ ] **iced_drop for cross-container DnD** - Custom DragState works for list reorder. iced_drop needed for: compose token DnD, label drag-to-file, calendar event dragging, attachment drag zones.
- [ ] **Read receipts (outgoing)** - MDN support. See `docs/roadmap/tracking-blocking.md`.
- [ ] **Inline image store eviction UI** - Settings control for store size (128 MB hardcoded).

- [ ] **Provider push notifications (remaining)** - JMAP WebSocket push is wired. Still missing: IMAP IDLE (persistent connection per folder), Graph/Gmail (poll-based, needs tuning - true push requires cloud infrastructure).
- [ ] **Pop-out HTML rendering** - SimpleHtml/OriginalHtml modes in message view pop-out fall back to plain text. Depends on the DOM-to-widget pipeline (`html_render.rs`) being wired into the pop-out view. Tracked separately in the HTML rendering section above.
- [ ] **Pop-out Print** - OS print dialog integration for message view and compose pop-out windows. Platform-specific, no iced precedent. Needs investigation.
- [ ] **Signature: per-account default dropdown in Account Settings** - Account editor overlay has no signature dropdown for selecting the default signature for an account.
- [ ] **Modal dialog content unification (GNOME HIG / libadwaita)** - The `alert_dialog` / `form_dialog` primitives in `ui/dialog.rs` now lock down GNOME HIG / `AdwAlertDialog` semantics (window-like card via `ContainerClass::DialogCard`, `TEXT_HEADING` title, `TEXT_MD` secondary body, right-aligned button row, libadwaita action appearances via `ButtonClass::Suggested` / `ButtonClass::Destructive`). Migrated: compose discard / link / save-as-group, calendar delete-event / discard-changes. Remaining work:
  - **Add-account modal** (`main.rs::view_with_add_account_modal`) is a multi-step flow, not a simple alert - keep its own card but reuse `ContainerClass::DialogCard` and the action-row layout pattern.
  - **First-launch onboarding** (`main.rs::view_first_launch_modal`) is a full-screen surface, not a stacked modal; leave as-is per `docs/ui/overlay-standardization-plan.md`.
  - **Inline confirmation rows** in settings (delete-account in `accounts.rs`, delete-signature in `signatures.rs`, delete-group in `groups.rs`, delete-contact in `contacts.rs`) live inside the settings *Sheet*, not a Modal stack. Different pattern; out of scope for `alert_dialog`. Should still get a unified inline-confirm helper, but distinct from the dialog primitive.

- [ ] **Compose: surface "Add at least one recipient" properly** - Sending with no recipients sets `state.status = "Add at least one recipient"` (`pop_out/compose.rs::Send`), which renders as a small status line at the bottom of the form. Should be a real validation surface - inline error near the To field, a toast, or a focus-and-shake on the empty field. Same path also covers the placeholder "Send not yet wired" message and any future send-failure feedback; depends on the toast/notification system in the main TODO list.

- [ ] **CardDAV contact write-back** - CardDAV client supports PROPFIND/REPORT/GET but not PUT/DELETE. Need vCard generation + PUT method for pushing contact edits to CardDAV servers. See `docs/contacts/problem-statement.md`.

## Refactor Backlog

Flagged inline as `TODO(refactor)` with `#[allow(clippy::too_many_arguments)]` or `#[allow(clippy::type_complexity)]` so clippy stays clean. Nothing here is blocking - each is a localized API cleanup that would replace a long arg list or nested-Option tuple with a named struct.

**Replace long arg lists with a params struct:**
- [ ] `db_save_local_draft` (15 args) - `crates/db/src/db/queries_extra/compose.rs:505` -> `SaveLocalDraftParams`
- [ ] `db_insert_scheduled_email` (14 args) - `crates/db/src/db/queries_extra/compose.rs:705` -> `ScheduledEmailParams`
- [ ] `db_upsert_contact_full` (10 args) - `crates/db/src/db/queries_extra/contacts.rs:121` -> `UpsertContactParams`
- [ ] `db_upsert_attachment` (10 args) - `crates/db/src/db/queries_extra/labels_attachments.rs:66` -> `UpsertAttachmentParams`
- [ ] `db_upsert_alias` (10 args) - `crates/db/src/db/queries_extra/compose.rs:402` -> `UpsertAliasParams`
- [ ] `db_upsert_label_coalesce` (9 args) - `crates/db/src/db/queries_extra/labels_attachments.rs:5` -> `UpsertLabelParams`
- [ ] `db_update_template` (8 args) - `crates/db/src/db/queries_extra/compose.rs:46` -> `UpdateTemplateParams`
- [ ] `upsert_auto_response_sync` (8 args) - `crates/db/src/db/queries_extra/auto_responses.rs:49` -> `UpsertAutoResponseParams`
- [ ] `gmail::ops::send_reaction` (9 args) - `crates/gmail/src/ops.rs:454` -> `ReactionMessage` (headers + threading fields)
- [ ] `imap_delta_sync` (8 args) - `crates/imap/src/imap_delta.rs:41` -> bundle stores/state into a `SyncCtx` struct
- [ ] `compose::new_reply` (8 args) - `crates/app/src/pop_out/compose.rs:563` -> `ReplyContext`
- [ ] `compose::build_recipient_row_inner` (8 args) - `crates/app/src/pop_out/compose.rs:1915` -> recipient row params struct (autocomplete + selection state)
- [ ] `calendar_month::mini_month` (9 args) - `crates/app/src/ui/calendar_month.rs:346` -> navigation params struct
- [ ] `settings::row_widgets::slider_row` (8 args) - `crates/app/src/ui/settings/row_widgets.rs:486` -> `SliderRow` builder
- [ ] `undoable_text_input::handle_update` (9 args) - `crates/app/src/ui/undoable_text_input.rs:291` -> `UpdateCtx` struct

**Replace nested-Option tuples with named structs:**
- [ ] `get_contact_meta_by_id_sync` returns `Option<(Option<String>, Option<String>, Option<String>)>` - `crates/db/src/db/queries_extra/action_helpers.rs:42` -> `ContactMeta` struct
- [ ] `merge_contact_pair_sync` builds a 6-tuple of `Option<String>` for the merge row - `crates/db/src/db/queries_extra/contacts.rs:949` -> `MergeContactRow` struct
- [ ] address-row 4-tuples of `Option<String>` (two call sites) - `crates/db/src/db/queries_extra/thread_persistence.rs:447, 665` -> `AddressRow` struct
- [ ] compressed-body batches `(String, Option<Vec<u8>>, Option<Vec<u8>>)` (two call sites) - `crates/stores/src/body_store.rs:152, 241` -> `CompressedBody` struct

## Needs Visual Review

Completed features that need to be visually verified in the running app.

- **Compose identity auto-selection (shared mailboxes)** - Auto-selects shared mailbox email when replying from SharedMailbox scope.
- **Rights gating on action buttons (JMAP sharing)** - Mailbox rights flow through CommandContext. Actions disabled when rights deny.
- **Signature placement in compose** - Auto-resolved on compose open. New compose: bottom. Reply: between content and quoted text.
- **BIMI avatar display** - Wired BimiLruCache to thread list sender avatars with circular image, initials fallback.
- **Active auto-reply status indicator** - Status bar shows "Out of Office auto-reply is active" when any account has enabled auto-replies.
- **CID image loading from inline image store** - Wired through thread detail → HTML renderer.

## Cross-Cutting Architecture Patterns

Living reference - follow these patterns as features are built. Keep until 1.0.

- **Generational load tracking** - 9 branded `GenerationCounter<T>` instances across App and component levels. See `docs/architecture.md`.

- **Component trait** - 8 components: Sidebar, ThreadList, ReadingPane, Settings, StatusBar, AddAccountWizard, Palette, ChatTimeline. Non-components use free functions + App handler methods: Compose, Calendar, Pop-out windows.

- **Token-to-Catalog theming** - Zero inline closure violations. Exceptions: rich text editor (builder methods), token input (renderer.fill_quad).

- **Config shadow pattern** - Formal: `PreferencesState`. Implicit (clone-on-open): Account editor, Contact editor, Group editor, Calendar event editor, Signature editor. Editors work on a shadow copy and commit on save.

- **DOM-to-widget pipeline** - V1 in `html_render.rs`. Supports links, CID images, block structure. Complexity heuristic (table depth >5, style tags >2) falls back to plain text. Used in reading pane only (NOT in pop-out message view). Remaining: inline formatting, remote images, tables.
