# TODO

As a general rule, TODO.md items are **removed** when completed.

## Remaining Work

- [ ] **Settings/Notifications** - VIP Senders should move to contact editing, and this should be a toggle button here.

- [ ] **Settings/Accounts: Edit Account** - This section needs rework.

- [ ] **Password input UX** - `input_row_secure` currently masks every character to a dot the moment it's typed. Open questions: (1) should there be an "eye" toggle that reveals the value while held / pressed? (2) should the most recently typed character render as plaintext for ~1 second before turning into a dot, the way iOS / Android do? (3) should reveal-on-hover ever apply, or strictly explicit gesture? Affects `input_row_secure` in `row_widgets.rs` and every CalDAV / IMAP / SMTP password field that uses it.

- [ ] **Attachment saving** - Should remember last folder. Ideally last folder per thread ID.

- [ ] **Collapse individual expanded messages** - The button needs a new place to live - probably a very long, thin button that stretches across the entire horizontal space at the top of the message frame. This needs to be unified with the Attachments panel collapsing, which is currently taking up too much vertical space; also too much padding above the Attachments section.

- [ ] **Settings/Composing: Signatures** - This section needs work.

- [ ] **Standardized popup/dropdown/modal** - Structural primitives are done (`modal_overlay`, `AnchoredOverlay`; see `docs/ui/overlay-standardization-plan.md`). The modal blocker now absorbs left/right/middle clicks, double clicks, and scroll so widgets behind the dimmed area no longer respond. The Add Account modal and confirm/form dialogs all share `ContainerClass::DialogCard` for visual consistency. Remaining gaps:
  - **Focus trapping** is still unsupported by iced (tracked separately below).
  - **Settings dropdowns** (the in-tab `select` widgets) close on outside click via their own `AnchoredOverlay::on_dismiss`, but that's per-widget rather than a unified contract; verify all `select` instances dismiss consistently.

- [ ] **Focus trapping for modals and sheets** - iced does not natively support focus trapping. Modal and Sheet surfaces should trap Tab/Shift-Tab focus within their content, but currently focus can escape to widgets behind the blocker. If iced adds focus trapping support, `modal_overlay()` (see `docs/ui/overlay-standardization-plan.md`) is the single place to wire it in. Until then, this is a known contract gap.

- [ ] **Calendar event detail popover → AnchoredOverlay** - `calendar::popover_stack()` is the only anchored surface still using a hand-rolled `stack![]` instead of the `AnchoredOverlay` primitive. Target behavior: anchor near the clicked event pill using `anchor_point`. Requires capturing click coordinates in `CalendarPopover::EventDetail` (not currently stored). See `docs/ui/overlay-standardization-plan.md` deferred work.

- [ ] **Settings help tooltip → Ratatoskr Tooltip primitive** - The settings help surface uses `AnchoredOverlay` but is semantically a tooltip (hover-triggered, non-blocking, informational). The legacy pinned/sticky behavior has been removed. Should migrate to a Ratatoskr Tooltip primitive once one exists. Independent of the overlay standardization effort.

- [ ] **Escape key audit for overlay surfaces** - Calendar Escape now routes through `CalendarMessage::ClosePopover` / `CloseModal` instead of bluntly resetting the workflow, so Escape from the editor's ConfirmDiscard returns to the editor with the draft intact rather than nuking everything. Settings sheet's discard-changes confirm dialog also cancels on Escape. Still owed: a mechanical verification sweep over every Modal/Sheet surface (compose pop-out save-as-group dialog Escape, palette Escape inside a sub-state, add-account modal Escape, etc.) once everything has had some shakedown time.

- [ ] **Calendar move semantics for existing events** - The calendar picker is disabled for `EditingEvent` because moving an event between calendars requires provider-specific support (some providers need delete+create). When provider calendar-move APIs are implemented, re-enable the picker for existing events and update `account_id` ownership logic in the `CalendarSelected` handler accordingly.

- [ ] **Link hover URL disclosure (email content)** - Links in email bodies need status-bar disclosure.

- [ ] **Link context menu (email content)** - Right-clicking a link in an email body should offer actions like Copy Link and related link operations.

- [ ] **Pop-out viewer attachment Open / Save / Save All are stubs** - The compact attachment cards in the message-view pop-out render Save / Open buttons on hover and a Save All button in the panel header, but `handlers/pop_out/message_view.rs` handles all three with `log::info!("not yet implemented")`. The buttons are clickable; nothing happens. Open should hand off to the OS default handler; Save / Save All should use `rfd::AsyncFileDialog` (file / folder picker) and write attachment bytes from the attachment file cache. Should share the last-folder-per-thread memory from the "Attachment saving" item above once that lands. See `docs/pop-out-windows/discrepancies.md` Medium #10.

- [ ] **Starred thread card background** - The golden tint on starred thread cards uses a fixed `mix()` ratio (`STARRED_BG_ALPHA`) which may not look right across all themes. Needs a GPU-level blend/shader effect that adapts to the theme's background luminance so the starred highlight reads consistently in both light and dark themes.

- [ ] **Star icon: need filled variant** *(Deferred - blocked on sluggrs SVG icon rendering)* - Lucide only has outline icons (confirmed: `star` U+E176, `star-half` U+E20B, no filled variant in the bundled font). Currently uses Unicode `*` as a stopgap, which causes size mismatch and visual jank. Will be resolved by switching to real SVG vector icon rendering (recently implemented in sluggrs, our text renderer) - filled and outline star SVGs can both ship and the toggle just swaps the asset. The button should also not change background color on toggle - just the icon fill.

- [ ] **Autocomplete: cross-field drag-and-drop** - Drag detection works but drop cancels. Context menu "Move to" is the workaround. Needs ghost token rendering and target field hit-testing.

- [ ] **Autocomplete: reuse beyond compose** - Widget only used in compose. Calendar attendee picker and group editor could potentially reuse it.

- [ ] **Contact pills on recipients** - Per `docs/pop-out-windows/problem-statement.md`: recipients in To/Cc fields (in all parts of the app: pop-out view, compose, reading pane thread view, and chat view) should appear as plain text but become contact pills on hover, revealing an inline edit button for quick contact editing. Applies to: reading pane message headers, pop-out message view, compose window recipient display. Currently recipients are plain text everywhere (except pop-out compose window) with no hover interaction. Needs: (1) a contact pill widget that blends with background at rest and reveals pill styling + edit button on hover, (2) display name resolution from the contact system (name → email fallback chain), (3) wiring to the existing `EditContact` flow that opens the settings contact editor. See `docs/pop-out-windows/discrepancies.md` High #4.

- [ ] **Action service: user-facing retry status** *(Deferred - blocked on toast system)* - Backend complete: `db_pending_ops_count()`, `db_pending_ops_failed_count()`, `db_pending_ops_retry_failed()` all exist. Zero UI wiring. Needs a toast/notification syste before this can surface "N actions pending retry" badges or "Archive failed after 10 retries" persistent notifications. Without this, users have no visibility into silently diverged state.

- [ ] **Action service: native provider batching** *(Deferred - low ROI until bulk ops are common)* - `batch_execute` dispatches per-thread `MailOperation` sequentially within each account. Provider reuse per account already eliminated client construction overhead - remaining cost is network latency (one round-trip per thread). Native batching (Gmail batch API, Graph `/$batch`, JMAP `Email/set`, IMAP multi-UID STORE) would reduce 50 round-trips to 1-3 for bulk operations. `PartialEq` on `MailOperation` enables grouping identical operations; the executor contract already specifies regrouping semantics. Implementation deferred until bulk operations on 50+ threads become a real user workflow.

- [ ] **Raw message source store** - The Source view in the pop-out message viewer currently synthesizes a pseudo-`.eml` from parsed headers + body store content (best effort, not faithful to the original MIME framing). For real on-the-wire raw source we'd need a new `raw_source_store` (zstd-compressed blob store, parallel to `body_store` / inline image store, keyed by `(account_id, message_id)`) populated during sync. Each provider needs a separate fetch path: Gmail `format=raw`, JMAP blob endpoint, Graph `/messages/{id}/$value`, IMAP `BODY[]` (currently parsed-on-the-fly and discarded). Without it, DKIM/ARC verification, the original Received chain, original Content-Transfer-Encoding, MIME boundary strings, header order/casing, and address comments all stay lost - reassembly from the parsed columns can't reproduce any of those byte-exactly. Storage cost is real at the project's "150+ GB cached mailbox" target, so the rollout should consider scope (only newer messages? evict on archive? per-account opt-in?) before turning capture on by default. See `docs/pop-out-windows/discrepancies.md` Medium #7.

- [ ] **Scroll virtualization** - Thread list renders all cards in `column![]` inside `scrollable`. Needs iced-level virtual scrolling for large mailboxes. This is a much more difficult problem than it appears. Do not take this work lightly.

- [ ] **Scroll-to-selected in palette** - Arrow keys update `selected_index` but `scrollable::scroll_to` doesn't exist in our iced fork. Needs alternative approach.

- [ ] **`responsive` for adaptive layout** - Collapse panels at narrow window sizes.

- [ ] **Keybinding management UI (Slice 6f)** - Settings panel for viewing, searching, and rebinding shortcuts. Backend ready (override persistence, conflict detection, set/unbind/reset APIs). See `docs/cmdk/app-integration-spec.md` § Slice 6f.

- [ ] **Restore OS-based theme and 1.0 scale** *(Deferred until 1.0)* - Revert to `"System"` theme, persist user prefs.

- [ ] **Bundle SQLite for release builds** *(Deferred until 1.0)* - Re-enable `rusqlite/bundled` feature for release builds so the binary ships a known SQLite version with FTS5 guaranteed. Dev builds use system libsqlite3 for faster compiles.

- [ ] **Reconsider sidebar layout** *(Deferred until right before 1.0)* - Currently the spec says: (1) sidebar should not show any Labels section when "All Accounts" is selected, (2) when a single account is selected, only labels belonging to that account should be shown, and (3) that for providers that have a "folder" concept, the users folders should show in the Labels section. We might need to re-think all 3.

## Roadmap Features - Remaining Work

Features with backend complete but UI or integration work remaining. Each references its roadmap spec.

### Labels Unification - `docs/labels-unification/problem-statement.md`

**10 discrepancies remain** - see `docs/labels-unification/discrepancies.md`. Critical: command palette rejects non-Gmail label operations, palette queries use legacy type filtering. Also:

- [ ] **Label picker overlay** - Triggered from reading pane or command palette. Lists all available tag-type labels with colors for apply/remove.

### Search - `docs/search/problem-statement.md`

Backend pipeline exists (parser, SQL builder, Tantivy, unified router). **29 discrepancies remain** - see `docs/search/discrepancies.md`. Critical: combined path applies free text in SQL before Tantivy ranking, Tantivy-only results show wrong message metadata, date boundaries inconsistent across engines. Also typeahead, pinned search lifecycle, and smart folder management gaps.

- [ ] **Promote pinned search to Smart Folder** - Sidebar pinned searches need an action that converts a pinned search into a Smart Folder.

### Calendar - `docs/calendar/problem-statement.md`

Views, editor, pop-out, sidebar all partially implemented. See `docs/calendar/discrepancies.md` for the live list. Backend now covers TZID/VTIMEZONE resolution (CalDAV) and Windows timezone names (Graph), CalDAV is consolidated on `rtsk::caldav` (calcard parser, ctag/etag incremental sync), `canEdit` flows from Graph/Google access roles to a `calendars.can_edit` column, and meeting-invite detection populates `messages.has_meeting_invite` / `meeting_invite_method` at insert time. RRULE expansion now handles BYDAY/BYMONTHDAY/BYMONTH on top of the FREQ/INTERVAL/COUNT/UNTIL baseline. Still open: drag interactions, RSVP actions, runtime reminder timer, meeting-invite UI affordances, permission gating on action buttons.

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
- [ ] Remote image loading with user consent (`block_remote_images` setting exists but disconnected from `render_html` - function signature needs context parameter)
- [ ] Table rendering (table-for-layout is the hardest - no `<table>`/`<tr>`/`<td>` handling at all)
- [ ] Image caching (`HashMap<String, image::Handle>`) - no `iced::widget::image` usage in app crate

## Security / Bug Findings (unfixed)

- [ ] **Microsoft ID token not signature-verified** - JWT payload is base64-decoded and trusted for email/name claims without verifying the signature. Token comes over TLS from Microsoft, but a MITM or compromised endpoint could inject arbitrary identity claims.

- [ ] **Account secrets stored plaintext in `accounts` table** *(unverified - surfaced by codex security review 2026-05-01)* - `CreateAccountParams` builds raw OAuth tokens and IMAP/SMTP passwords (`crates/app/src/ui/add_account/identity.rs:46`), then `create_account_sync` inserts them directly (`crates/db/src/db/queries_extra/accounts_crud.rs:73`). Reauth has the same issue for passwords (`crates/app/src/ui/add_account/state.rs:480`) and OAuth tokens (`crates/app/src/ui/add_account/oauth.rs:137`); CalDAV password updates also write raw values (`crates/app/src/ui/settings/update/accounts.rs:126`). The bug is masked because provider clients tolerate plaintext via `decrypt_or_raw` / `decrypt_if_needed` (`crates/common/src/crypto.rs:141`). Fix: encrypt at the DB boundary; assert in regression tests that stored values never equal the raw token/password.

- [ ] **Mail content stores not encrypted at rest** *(unverified - surfaced by codex security review 2026-05-01)* - `BodyStore` keeps compressed HTML/text bodies in `bodies.db` unencrypted (`crates/stores/src/body_store.rs:117`), inline images are raw SQLite blobs (`crates/stores/src/inline_image_store.rs:96`), and attachment cache files are raw bytes (`crates/stores/src/attachment_cache.rs:52`). Compression is not a security boundary. Either envelope-encrypt with AES-256-GCM, or document explicitly that content at rest relies on OS / full-disk encryption.


## Remaining Enhancements (other)

- [ ] **iced_drop for cross-container DnD** - Custom DragState works for list reorder. iced_drop needed for: compose token DnD, label drag-to-file, calendar event dragging, attachment drag zones (the compose-window two-zone overlay - see `docs/pop-out-windows/discrepancies.md` High #1).
- [ ] **Read receipts (outgoing)** - MDN support. See `docs/roadmap/tracking-blocking.md`.
- [ ] **Inline image store eviction UI** - Settings control for store size (128 MB hardcoded).

- [ ] **Provider push notifications (remaining)** - JMAP WebSocket push is wired. Still missing: IMAP IDLE (persistent connection per folder), Graph/Gmail (poll-based, needs tuning - true push requires cloud infrastructure).
- [ ] **Pop-out Print** - OS print dialog integration for message view and compose pop-out windows. Platform-specific, no iced precedent. Needs investigation. See `docs/pop-out-windows/discrepancies.md` Medium #9 (and High #3 for the missing compose-header Print button).
- [ ] **Signature: per-account default dropdown in Account Settings** - Account editor overlay has no signature dropdown for selecting the default signature for an account.
- [ ] **Modal dialog content unification (GNOME HIG / libadwaita)** - The `alert_dialog` / `form_dialog` primitives in `ui/dialog.rs` now lock down GNOME HIG / `AdwAlertDialog` semantics (window-like card via `ContainerClass::DialogCard`, `TEXT_HEADING` title, `TEXT_MD` secondary body, right-aligned button row, libadwaita action appearances via `ButtonClass::Suggested` / `ButtonClass::Destructive`). Migrated: compose discard / link / save-as-group, calendar delete-event / discard-changes. Remaining work:
  - **Add-account modal** (`main.rs::view_with_add_account_modal`) is a multi-step flow, not a simple alert - keep its own card but reuse `ContainerClass::DialogCard` and the action-row layout pattern.
  - **First-launch onboarding** (`main.rs::view_first_launch_modal`) is a full-screen surface, not a stacked modal; leave as-is per `docs/ui/overlay-standardization-plan.md`.
  - **Inline confirmation rows** in settings (delete-account in `accounts.rs`, delete-signature in `signatures.rs`, delete-group in `groups.rs`, delete-contact in `contacts.rs`) live inside the settings *Sheet*, not a Modal stack. Different pattern; out of scope for `alert_dialog`. Should still get a unified inline-confirm helper, but distinct from the dialog primitive.

- [ ] **CardDAV contact write-back** - CardDAV client supports PROPFIND/REPORT/GET but not PUT/DELETE. Need vCard generation + PUT method for pushing contact edits to CardDAV servers. See `docs/contacts/problem-statement.md`.

- [ ] **Rich text editor (rte) post-review gaps** - Surfaced during the 12-finding correctness review. None are regressions; all are interactions between the recent fixes and the existing flat `DocPosition` model.
  - `is_atomic_block()` is defined as `!is_inline_block()`, so it includes `BlockQuote` alongside `Image` and `HorizontalRule`. Backspace at the start of a paragraph immediately following a `BlockQuote` now removes the entire quoted reply (not a no-op, not a merge). Acceptable but aggressive in the compose pop-out where BlockQuotes hold reply content - if user feedback bites, split atomic-vs-container behaviour in `resolve_delete_backward` / `resolve_delete_forward` (`crates/rte/src/rules.rs`).
  - `link_at_content_point` (`crates/rte/src/widget/mod.rs`) returns `None` when `entry.paragraph()` is `None`, which is the case for container blocks (`BlockQuote`, list groups). Single-clicking a link inside a quoted reply still falls through to caret placement instead of emitting `Action::LinkClicked`. Matches the existing "container content isn't `DocPosition`-addressable" limitation - revisit when/if container content becomes addressable.
  - Caret rendering inside an atomic block: `draw_cursor` (`crates/rte/src/widget/mod.rs`) falls into the no-paragraph branch and draws at `para_origin_x` for both offset 0 and offset 1, so arrowing across an `Image` or `HorizontalRule` produces no visible cursor movement even though the offset advances. Functionally fine (Backspace/Delete on the post-atom offset still removes the atom); purely a visual fidelity gap.
  - `paste_plain_text` (`crates/rte/src/widget/editor_state.rs`) splits on `\n` after CRLF normalization, so a trailing newline (e.g. `"alpha\n"`) produces an extra empty paragraph at the end. Likely intended (preserves explicit blank-line intent), but worth confirming against real-world paste sources before treating as final.

- [ ] **`html_render` post-review gaps** *(Bridge fixes only - litehtml-rs at `/home/folk/Programs/litehtml-rs` is the eventual replacement)* - Surfaced during the 11-finding review of `crates/app/src/ui/html_render.rs`. None are regressions; each is a known limitation of the targeted fixes that landed for the bridge period.
  - **Inline image frame width.** `render_cid_image` uses `width(Length::Fill) + ContentFit::ScaleDown`. Large images correctly scale down to body width, but small images now reserve the full body width with empty space around the rendered pixels. iced's `image` widget doesn't expose `max_width`; a real "shrink to natural, cap at container width" needs a `responsive` wrapper or a natural-dimension query that picks `Length::Fixed(min(natural_w, available_w))`. Verify visually before treating as final.
  - **Heading style fidelity.** `Block::Heading(String, u8)` only stores plain text, so `<h1>Hello <em>world</em></h1>` collapses to "Hello world" rendered semibold - the `<em>` italic run is lost. Promoting to `Block::Heading(Vec<InlineSpan>, u8)` would restore fidelity but ripples through all heading rendering call sites; not worth doing now if litehtml-rs is close.
  - **Inline styles inside `<pre>`.** Style flag bumps are gated by `!self.in_pre` so `<pre>plain<b>bold</b>plain</pre>` flattens to `Preformatted("plainboldplain")` - bold is lost. Correct semantics for pre-as-plain-text but wrong for source-with-syntax-highlighting. Same path-of-least-resistance trade-off until litehtml-rs.
  - **Trailing-text-after-nested-list ordinal renumbering.** `<ol><li>outer<ul>...</ul>after</li></ol>` parses as `1. outer / • inner / 2. after`. The "2." is a side-effect of the flat block model emitting the trailing inline content as its own outer-list item. Same flat-model compromise as the rte parser - users may or may not notice.

## Test Harness

Architecture and design rationale stay in `docs/glossary/harness.md`. The milestone roadmap is retired - remaining work is captured here.

### Tests unlocked by saehrimnir 45bf850..28017e7

These depend on installing a saehrimnir binary at or after `28017e7`
and mirroring any needed upstream fixtures into
`crates/app/tests/sync-fixtures/`.

- [ ] **Graph shared-mailbox `/users/{id}` mail sync** - Drive
  Graph sync through `GraphClient::for_shared_mailbox` against a
  secondary account in `multi-account-small`. Assert shared mailbox
  folders, messages, attachment metadata, and delta tokens are stored
  under `shared_mailbox_id` instead of the personal account scope.
- [ ] **Graph `/users/{id}` calendar scoping** - Extend the
  shared-mailbox sync harness to cover Graph calendar reads through
  `/v1.0/users/{id}/...`. Assert per-account calendars, events, and
  delta links stay isolated.
- [ ] **Graph contacts shared-mailbox path hardening** - Try contact
  sync against `/v1.0/users/{id}/contactFolders/...`. This should
  drive `contact_sync` to use `GraphClient::api_path_prefix()` instead
  of hardcoded `/me` paths, then assert contact folders, contacts, and
  delta links stay scoped to the shared mailbox.
- [ ] **Graph master category label sync** - Use
  `graph-categories-small.toml` to exercise
  `graph_label_sync()`. Assert `cat:<displayName>` tag labels are
  inserted with the expected account id, `label_kind = 'tag'`, sort
  order, and Exchange preset colors from the Graph category palette.
- [ ] **Graph category shared-mailbox path hardening** - Combine
  master categories with a multi-account fixture. This should drive
  `graph_label_sync()` to use `GraphClient::api_path_prefix()` instead
  of hardcoded `/me`, then assert category labels from one mailbox
  never appear in another mailbox's label set or sidebar scope.
- [ ] **Exchange group sync compatibility smoke** - Try
  `sync_exchange_groups()` against the new Graph group fixture. Track
  whether saehrimnir needs aliases for Ratatoskr's exact paths:
  `/me/memberOf/microsoft.graph.group` and
  `/groups/{id}/transitiveMembers/microsoft.graph.user`. Once those
  paths are covered, assert groups and member email rows land in
  contact groups.
- [ ] **Google OAuth token account binding: Gmail** - Mint two mock
  OAuth tokens with different `account_id` form fields, seed two
  Gmail accounts, and sync them against one multi-account fixture.
  Assert Gmail labels, threads, messages, and attachments are scoped
  by the bearer token's account, not by the fixture primary.
- [ ] **Gmail SendAs signature import** - Mirror the new
  `[[send_as]]` rows from `multi-account-small` and run Gmail
  signature sync. Assert each `sendAsEmail` becomes the right local
  signature / alias metadata for its account, including HTML body,
  display name, primary/default flags, and no cross-account leakage.
- [ ] **Gmail SendAs signature writeback** - Edit a Gmail-backed
  signature through the Service/settings path and assert saehrimnir
  receives `PATCH /gmail/v1/users/me/settings/sendAs/{email}` with
  the sparse signature fields. Re-read the mock SendAs row and assert
  the signature changed while read-only `isPrimary` stayed unchanged.
- [ ] **Gmail SendAs fault injection** - Use Lua
  `on("gmail", "send_as", fn)` to force list/get/patch failures.
  Assert signature import reports a provider failure without corrupting
  local signatures, and writeback leaves the expected pending or retry
  state.
- [ ] **Gmail SendAs token account binding** - Extend the Gmail
  multi-account OAuth tests to cover SendAs identities. Mint tokens for
  primary and secondary accounts, sync signatures for both accounts,
  and assert each account only imports or patches its token-bound
  identity.
- [ ] **Google OAuth token account binding: Calendar and People** -
  Repeat the token-scoping shape for Google Calendar and People API.
  Assert `calendarList`, events, contacts, and otherContacts all
  scope to the token-bound account and that missing / default tokens
  keep the old primary-account behavior.
- [ ] **CalDAV multi-account principal scoping** - Use
  `multi-account-small` with two CalDAV accounts whose usernames map
  to `account-primary` and `account-secondary`. Sync both and assert
  `/principals/{user}/` and `/calendars/{user}/...` only expose that
  account's calendars and events; primary must never import
  secondary's calendar rows or vice versa.
- [ ] **CalDAV secondary-principal write isolation** - Create, update,
  and delete an event through the secondary CalDAV account. Assert the
  event is reachable through `/calendars/account-secondary/...`,
  404s under the primary principal, and Ratatoskr stores/removes it
  only under the secondary account.
- [ ] **CalDAV `MKCALENDAR` create-calendar action** - Once the
  Ratatoskr create-calendar path is exposed in the harness, create a
  calendar against a CalDAV account and assert saehrimnir records
  `MKCALENDAR`, preserves display name / calendar color, and the next
  sync imports the new calendar. Include duplicate-id and unknown
  principal failure cases.
- [ ] **Cross-protocol calendar creation visibility** - After a
  CalDAV `MKCALENDAR`, sync JMAP Calendar and Graph Calendar against
  the same mock fixture. Assert JMAP `Calendar/changes` reports the
  created calendar and Graph `/me/calendars` lists it, proving
  saehrimnir's shared `calendar_created` transition is visible across
  protocol surfaces.
- [ ] **IMAP LOGIN multi-account binding** - Seed two IMAP accounts
  with different usernames matching fixture account names. Sync both
  through the same mock process and assert LIST / STATUS / SELECT /
  FETCH only expose the authenticated account's mailboxes and
  messages.
- [ ] **IMAP XOAUTH2 / OAUTHBEARER account binding** - Mint mock
  OAuth tokens for different accounts and use them in IMAP auth.
  Assert the connection binds to the token account, and add a
  fallback case where an unknown token or user stays on the primary
  account.
- [ ] **SMTP AUTH account attribution** - Send via SMTP for two
  accounts and assert `/test/smtp/submissions` records the resolved
  `account_id`, auth mechanism, recipients, and parsed MIME summary
  for each submission.
- [ ] **SMTP AUTH failure callback** - Use Lua
  `on("smtp", "AUTH", fn)` to force an auth failure. Assert the send
  action reports the right provider failure, does not record a
  successful submission, and leaves any retry / pending-op state in
  the expected shape.
- [ ] **Expand recurrence read matrix** - Initial
  `calendar-recurrence-small.toml` smoke coverage now exists for
  Graph, Google Calendar, JMAP Calendar, and CalDAV. Broaden it to
  include daily, yearly, BYMONTH, EXDATE, timezone handling, and
  expanded calendar-window row assertions.
- [ ] **Recurrence write matrix** - Create and update recurring
  events through the Service calendar action path for Graph, Google
  Calendar, JMAP Calendar, and CalDAV. Assert the request log carries
  provider-native recurrence payloads and a follow-up sync imports
  the same recurrence metadata back into local state.
- [ ] **Cross-protocol recurring-event mutation deltas** - Mutate a
  recurring event through one mock protocol, then sync another
  protocol backed by the same fixture state. Cover at least Graph
  after CalDAV, Google Calendar after Graph, and JMAP Calendar after
  Google Calendar so the shared change-log recurrence path is pinned.

### Environment-blocked (Windows)

The Linux equivalents already automate. The harness scripts are platform-agnostic; the gate is the test environment (cross-platform CI runner, dev box, or paid test service). If any of these become permanent automation, add Windows-capable Lua or libtest coverage and keep the Linux-only SIGTERM script separate.

- [ ] **M6.1 Windows parent-death (Job Object)** - Verify the Service exits when its parent is killed via the Windows Job Object machinery.
- [ ] **M6.2 Windows clean-shutdown handshake** - Verify SIGTERM-equivalent / `WM_CLOSE` triggers shutdown drain and the `clean_shutdown` sentinel.
- [ ] **M6.3 Windows stdio-corruption defense** - Verify `println!` from a handler doesn't corrupt JSON-RPC framing on Windows.

### M9 follow-ups (optional)

- [ ] **Per-host baselines for `jmap_steady_state_delta`** - The checked-in baseline map (`brokkr.toml`) is currently single-host (`plantasjen` only). Other contributors or CI hosts that should run the gate need to record their own baseline with `brokkr sync-bench crates/app/tests/sync-harness/jmap-steady-state-delta.lua --gate jmap_steady_state_delta --as-baseline --bench 10` and append the printed line under `[ratatoskr.gate.jmap_steady_state_delta.baseline]`.
- [ ] **More checked-in gates** - Once a stable benchmark script matters to CI or release decisions, add a `[ratatoskr.gate.<name>]` block to `brokkr.toml` and record per-host baselines. Good candidates: JMAP scripted incremental, IMAP steady-state, Graph calendar remote-delta, CalDAV calendar remote-delta.

### Brokkr polish

- [ ] **`brokkr service-list --json`** - Machine-readable script discovery for failure-triage tooling and editor integrations. Deferred (no current consumer).

### Capability backlog (land when a test needs it)

The original M1 foundation sketch named these as target surface; the M2-M8 cohort all landed without needing them. Each becomes work when a future test names coverage it unblocks.

- [ ] **Generic `harness.wait_for { predicate, child, backstop }`** - Lua-facing wait combinator that races arbitrary predicates against child-exit observation. Today's scripts use typed `ServiceClient` requests, event-stream receives, async request handles, and per-call timeouts.
- [ ] **`NotificationQueue` Lua userdata** - `queue:recv(timeout)` / `queue:drain_for(duration)` returning `Notification` userdata with `service_generation`, `method`, and a `serde_json::Value`-backed `params` view for filtering on payload details.
- [ ] **Sentinel-file watch** - `harness.wait_for_sentinel { path, backstop }` for data-dir-relative paths and `{ absolute, backstop }` for explicit absolute paths. No leading-slash auto-detection, no glob support.
- [ ] **Parent-death helper bindings** - `harness.spawn_parent_death_helper(service_binary, data_dir) -> { service_pid, helper_handle }`. The `parent_death_helper` binary already exists; the binding does not. Required for `linux_parent_sigkill_terminates_service_within_two_seconds`-style coverage.
- [ ] **Generic `harness.wait_exit(client, backstop) -> ExitStatus`** - With `code()`, `signal()`, `wall_time_ms()` accessors.
- [ ] **Resource-budget summary** - `harness.resource_summary(client) -> { rss_kb, io_bytes, ... }` reusing brokkr's existing sidecar profiler.
- [ ] **Parsed `frames.jsonl` payloads** - The frame writer currently records redacted raw frames + length + SHA-256 with `parsed: null`. Structural parsed redaction (per-`RequestParams` field allowlist) is future hardening before any credentialed script lands.

### Lua-helper cleanup

- [ ] **Hoist extract/search script helpers** - Don't add another extract/search script that copy-pastes backfill, attachment polling, search polling, or attachment lookup helpers. First hoist them into shared harness helpers or a supported Lua include path.

## Refactor Backlog

Flagged inline as `TODO(refactor)` with `#[allow(clippy::too_many_arguments)]` or `#[allow(clippy::type_complexity)]` so clippy stays clean. Nothing here is blocking - each is a localized API cleanup that would replace a long arg list or nested-Option tuple with a named struct.

**Triage first: dead code in the params-struct backlog.** These are `pub` and re-exported via `pub use ...::*` in `queries_extra.rs`, but have **zero workspace callers**. Decide delete-vs-refactor before doing the params-struct rewrite - polishing dead code has no payoff.
- [ ] `db_insert_scheduled_email` (14 args) - `crates/db/src/db/queries_extra/compose.rs`
- [ ] `db_upsert_attachment` (10 args) - `crates/db/src/db/queries_extra/labels_attachments.rs`
- [ ] `db_upsert_alias` (10 args) - `crates/db/src/db/queries_extra/compose.rs`
- [ ] `db_upsert_label_coalesce` (9 args) - `crates/db/src/db/queries_extra/labels_attachments.rs`
- [ ] `db_update_template` (8 args) - `crates/db/src/db/queries_extra/compose.rs`

**Replace long arg lists with a params struct:**
- [ ] `gmail::ops::send_reaction` (9 args) - `crates/gmail/src/ops.rs:454` -> `ReactionMessage` (headers + threading fields)
- [ ] `imap_delta_sync` (8 args) - `crates/imap/src/imap_delta.rs:41` -> bundle stores/state into a `SyncCtx` struct
- [ ] `compose::new_reply` (8 args) - `crates/app/src/pop_out/compose.rs:563` -> `ReplyContext`
- [ ] `compose::build_recipient_row_inner` (8 args) - `crates/app/src/pop_out/compose.rs:1915` -> recipient row params struct (autocomplete + selection state)
- [ ] `calendar_month::mini_month` (9 args) - `crates/app/src/ui/calendar_month.rs:346` -> navigation params struct
- [ ] `settings::row_widgets::slider_row` (8 args) - `crates/app/src/ui/settings/row_widgets.rs:486` -> `SliderRow` builder
- [ ] `undoable_text_input::handle_update` (9 args) - `crates/app/src/ui/undoable_text_input.rs:291` -> `UpdateCtx` struct

**Replace nested-Option tuples with named structs:**
- [ ] `merge_contact_pair_sync` builds a 6-tuple of `Option<String>` for the merge row - `crates/db/src/db/queries_extra/contacts/dedup.rs:75`. Local-only - immediately destructured into named locals; struct adds boilerplate without clarity gain. Skip unless we want zero `type_complexity` allows.
- [ ] compressed-body batches `(String, Option<Vec<u8>>, Option<Vec<u8>>)` (two call sites) - `crates/stores/src/body_store.rs:152, 241` -> `CompressedBody` struct. Two unrelated sites that share a shape but no logic; struct improves readability of the in-flight Vec but doesn't dedup anything.

## Cross-Cutting Architecture Patterns

See `docs/architecture.md` § "Settled Patterns" for the living reference.
