# TODO

## Remaining Work

- [x] ~~**Avatar in main window account selector**~~ — Fixed: the selector now renders full account avatars in both the trigger and dropdown, using the account color as the avatar background when available.

- [x] ~~**Investigate dev-seed labels**~~ — Fixed: sidebar had two sections both titled "LABELS" sharing the same toggle. Provider folders now render as "FOLDERS" with own toggle; tags render as "LABELS" with own toggle.

- [ ] **Expand dev-seed** - The dev-seed script needs to create pinned searches, Smart Folders, contact groups, VIP contacts. Also verify that attachments are actual files. Needs to create actual signatures for contacts, both HTML and simple ones. Needs to create more emails with links and other non-text content. Needs to create fake shared accounts and mailboxes.

- [ ] **dev-seed calendars** - Obvious.

- [ ] **Divide up sidebar** - The top part with the Compose button, account selector, and calendar button should not scroll along with the rest of the content, but be stickied to the top like the Settings button is stickied to the bottom.

- [x] ~~**Mail list mouse clicks**~~ — Fixed: clicking or double-clicking the already-open thread no longer reloads the reading pane and cause flashing.

- [ ] **Settings row help icon** - Add support for a help (?) icon with a tooltip for settings rows. Should anchor to the right side of the label, hugging the label. First candidate: Message Dates.

- [ ] **Settings/People** - The contacts and group lists here need to conform much closer to the spec at docs/contacts/problem-statement.md. We're quite a ways off.

- [ ] **Settings/Notifications** - VIP Senders should move to contact editing, and this should be a toggle button here.

- [ ] **Compose window help text** - The help text in the compose windows to/cc/bcc fields ("Add recipients...") is not vertically centered in the input field.

- [ ] **Settings slide-in/over panel** - Clicking anywhere on the background of this panel currently closes it for some reason. Only the (1) back button, (2) selecting a different Settings section, or (3) closing the Settings should dismiss the slide-in. Possibly Esc should hotkey to it as well. Currently Esc closes the settings completely, but perhaps it should close a slide-in, if open, first.

- [ ] **Settings/People: Contacts list** - Group/account pills need to lay out horizontally first, then vertically.

- [ ] **Settings/Accounts: Edit Account** - This section needs rework.

- [ ] **Compose window input fields** - The to/cc/bcc and the Subject fields have different styling.

- [ ] **Compose window account dropdown + cc/bcc buttons** - These need similar styling to other such controls with proper hover effects. The chevron icon in the dropdown should also be unified across the codebase, we use different chevron icons all over the place. Actually, the buttons at the bottom as well: Discard/attach/send, they need uniform app styling. Send should probably use the same styling as the main windows Compose button.

- [ ] **Compose window "pop ups"** - There's a popup when you Discard, and to Insert Link. These are not actually modals at the moment; they render at the bottom of the compose window.

- [ ] **Compose window labels** - The From/To/CC/Bcc/Subject labels should be right-aligned, so that they float near their relative inputs.

- [ ] **Attachment saving** - Should remember last folder. Ideally last folder per thread ID.

- [ ] **Reading pane** - There's too much vertical spacing between the top part and the first reply/all/forward action line. Needs a tighter fit; vertical space doesn't come cheap on a laptop.

- [ ] **Collapse individual expanded messages** — Chevron-down button in expanded message header should be a chevron-up to collapse. Also, the button needs a new place to live. Probably a very long, thin button that stretches across the entire horizontal space at the top of the message frame. This needs to be unified with the Attachments panel collapsing, which is currently taking up too much vertical space; also too much padding above the Attachments section.

- [ ] **Attachment "Save All" button** - Needs to have same styling as other in-section buttons in the reading pane, and should not be part of the same interaction block as the collapse/expand header.

- [ ] **Attachments in the reading pane** - They're not interactive? What's supposed to happen when they are clicked? See spec. Same thing in the pop out message window: not interactive there either.

- [ ] **Email body background override setting** — This needs to apply to the pop out window as well, and we need an inset rounded+bordered area in the pop-out viewing window just like in the reading pane.

- [ ] **Compose window close** - Closing the compose window doesn't currently ask the user whether they want to discard the draft. Should wire that in same as the Discard button.

- [ ] **Settings window dropdown rows** - The inlined dropdowns inside settings rows currently have their own background color hover effect, but this is not necessary because the entire settings row has a background hover effect.

- [ ] **Settings window dropdown closing** - The dropdown opens when the settings row is clicked, which is nice - but clicking the settings row again doesn't close it. It closes + reopens it with 1 click.

- [ ] **Settings window row hover** - Currently the hover effect for the settings row doesn't use the same border radius as the bottom/top settings rows, which means hovering those looks a bit weird.

- [ ] **Settings/Composing: Signatures** - This section needs work.

- [ ] **App logo in first-launch modal** — SVG rendered via iced svg feature, embedded with include_bytes, but it's not showing.

- [ ] **Standardized popup/dropdown/modal** - Currently setting dropdowns, various modal dialogs (the Settings slide-in, Add Account modal, etc) use various methods to dim/control/disable/dismiss. We need standardized controls for all this. For example the Add Account modal currently dims the background (rest of the window), but it doesn't prevent interaction with any controls - even controls that are actually directly below it can still be interacted with. We need the same treatment as the Settings slide-in that does in fact disable things behind it.

- [ ] **Label pills in reading pane** — Pills should not show on each message, only at the top. Labels are per-thread, not per-message, at least in the UI.

- [ ] **Link click handling (email content)** — Should open in system browser. Nothing happens.

- [ ] **Pop out message viewer body rendering** - The current pills for selecting Plain/Simple/Original/Source need to move. The spec currently doesn't say clearly where they should go. This needs to be resolved first.

- [ ] **Pop out message viewer body rendering toggle buttons** - The current pills for selecting Plain/Simple/Original/Source have zero effect, and the "Source" button just shows a generic "error" about message bodies being in a separate database. Even with dev-seed.

- [ ] **Pop out message viewer dropdown menu** - It seems to be constrained to the width of the window, which doesn't work because it's all the way on the right side. It needs room to show its contents. Either it needs to be able to render outside the window, or it needs to grow left.

- [ ] **Pop out message viewer paddings/margin** - This needs to be unified. Currently for example the date/time stamp on the right side of the subject hugs the right window edge much closer than the dropdown action button above it. And also I'm not sure the subject and datetime are baseline aligned - it seems the subject floats a bit higher up. Could be wrong about that, haven't measured pixels.

- [ ] **Message box / toast notification system** — Generic modal message box and/or toast notification infrastructure for the app. Needed for: compose draft save failure on close (currently silently aborts the close with no user feedback), action service retry exhaustion warnings, and any future error/confirmation flows. Should support at least: transient toasts (auto-dismiss), persistent error banners, and modal confirmation dialogs.

- [ ] **Starred thread card background** — The golden tint on starred thread cards uses a fixed `mix()` ratio (`STARRED_BG_ALPHA`) which may not look right across all themes. Needs a GPU-level blend/shader effect that adapts to the theme's background luminance so the starred highlight reads consistently in both light and dark themes.

- [ ] **Star icon: need filled variant** — Lucide only has outline icons. The star toggle in the reading pane needs a filled star (golden) for the active state and an outline star for inactive. Currently uses Unicode ★ as a stopgap, which causes size mismatch and visual jank. Options: (1) add a second icon font with filled variants, (2) use an SVG/image icon, (3) custom widget that draws a filled star path. The button should also not change background color on toggle — just the icon fill.

- [ ] **Autocomplete: cross-field drag-and-drop** — Drag detection works but drop cancels. Context menu "Move to" is the workaround. Needs ghost token rendering and target field hit-testing.
- [ ] **Autocomplete: email validation before tokenization** — Enter/Tab/comma/semicolon tokenize any non-empty text. Should validate plausible email format before creating a token.
- [ ] **Autocomplete: context menu Cut/Copy/Paste** — Token context menu has Delete, Expand group, Move-to-field only. Missing clipboard operations.
- [ ] **Autocomplete: bulk-paste "Save as group"** — Banner renders but save action is not wired.
- [ ] **Autocomplete: richer dropdown rendering** — Currently plain text "Name <email>". Spec calls for two-column layout (name + email), group icon, member count display.
- [ ] **Autocomplete: group token "(N)" suffix** — `member_count` stored on Token but chip label is just the group name.
- [ ] **Autocomplete: search debounce** — Search dispatches immediately on every keystroke. Spec calls for 10-20ms debounce to coalesce rapid typing.
- [ ] **Autocomplete: paste dedup** — `dedup_parsed()` exists but is never called. Also no dedup against existing tokens in the field.
- [ ] **Autocomplete: reuse beyond compose** — Widget only used in compose. Calendar attendee picker and group editor could reuse it.
- [ ] **Contact pills on recipients** — Per `docs/pop-out-windows/problem-statement.md`: recipients in To/Cc fields should appear as plain text but become contact pills on hover, revealing an inline edit button for quick contact editing. Applies to: reading pane message headers, pop-out message view, compose window recipient display. Currently recipients are plain text everywhere with no hover interaction. Needs: (1) a contact pill widget that blends with background at rest and reveals pill styling + edit button on hover, (2) display name resolution from the contact system (name → email fallback chain), (3) wiring to the existing `EditContact` flow that opens the settings contact editor.

- [ ] **Action service: user-facing retry status** *(Deferred — blocked on toast system)* — Backend complete: `db_pending_ops_count()`, `db_pending_ops_failed_count()`, `db_pending_ops_retry_failed()` all exist. Zero UI wiring. Needs the toast/notification system (first TODO item) before this can surface "N actions pending retry" badges or "Archive failed after 10 retries" persistent notifications. Without this, users have no visibility into silently diverged state.

- [ ] **Action service: native provider batching** *(Deferred — low ROI until bulk ops are common)* — `batch_execute` dispatches per-thread `MailOperation` sequentially within each account. Provider reuse per account already eliminated client construction overhead — remaining cost is network latency (one round-trip per thread). Native batching (Gmail batch API, Graph `/$batch`, JMAP `Email/set`, IMAP multi-UID STORE) would reduce 50 round-trips to 1-3 for bulk operations. `PartialEq` on `MailOperation` enables grouping identical operations; the executor contract already specifies regrouping semantics. Implementation deferred until bulk operations on 50+ threads become a real user workflow.

- [x] ~~**Typed IDs: CommandArgs fields**~~ — Extracted `FolderId`/`TagId` to `crates/types/` micro-crate. `cmdk` now uses typed IDs directly; dispatch passes them through without manual wrapping.
- [x] ~~**Typed IDs: sidebar.selected_label**~~ — Replaced with `selection: SidebarSelection` enum (`Inbox | Folder(SystemFolder) | Bundle(Bundle) | FeatureView(FeatureView) | SmartFolder | ProviderFolder(FolderId) | Tag(TagId)`). `NavigationTarget` collapsed to `Sidebar(SidebarSelection) | Search | PinnedSearch | Chat`. Deleted `view_type_from_label()` and `view_type_from_target()`. Per-use-case helpers instead of generic bridge.
- [x] ~~**Palette NavigateToLabel untyped at boundary**~~ — Split `CommandArgs::NavigateToLabel` into `NavigateToFolder(FolderId)` / `NavigateToTag(TagId)`. Encoded `label_kind` in palette option ID as `"account_id:f|t:label_id"`.
- [x] ~~**Potential tag duplication in navigation state**~~ — Confirmed: `build_account_labels()` was returning tags alongside containers when single-account scoped, duplicating `build_all_account_tags()`. Fixed by filtering `label_kind != "tag"` in `build_account_labels()` — tags now come exclusively from the cross-account builder.
- [x] ~~**`App.navigation_target` is vestigial**~~ — Replaced with `active_chat: Option<String>`. `reset_view_state()` no longer takes a parameter. `NavigationTarget` enum retained for `Message::NavigateTo` dispatch only.
- [ ] **First-launch modal not dismissible** — In zero-accounts state, cancel doesn't close the wizard. Spec says it should dismiss over an unusable empty app. Intentional safety measure or bug — decide and document.
- [x] ~~**Default scope is first account, not All Accounts**~~ — `handle_accounts_loaded()` now sets `ViewScope::AllAccounts` instead of scoping to the first account.
- [ ] **App-specific-password help not clickable** — Discovery types carry `help_url` but UI shows plain text "Check {domain} for setup instructions" — no clickable link to provider app-password pages.
- [ ] **Deleted-account compose/pop-out cleanup** — Account deletion doesn't close compose windows or message-view pop-outs for the deleted account, and doesn't block sending from a deleted identity.
- [ ] **Sync-task cancellation on account deletion** — Delete flow removes DB data but doesn't cancel in-flight sync tasks. Stale sync completions could write to deleted account state.
- [x] ~~**Search scope respects ViewScope**~~ — SQL fallback and free-text LIKE search now pass `current_account_scope()` through. Tantivy path still ignores scope (needs post-filtering or re-indexing — left as TODO comment in code).

- [ ] **Scroll virtualization** — Thread list renders all cards in `column![]` inside `scrollable`. Needs iced-level virtual scrolling for large mailboxes.

- [ ] **Scroll-to-selected in palette** — Arrow keys update `selected_index` but `scrollable::scroll_to` doesn't exist in our iced fork. Needs alternative approach.

- [ ] **`responsive` for adaptive layout** — Collapse panels at narrow window sizes.

- [ ] **Keybinding management UI (Slice 6f)** — Settings panel for viewing, searching, and rebinding shortcuts. Backend ready (override persistence, conflict detection, set/unbind/reset APIs). See `docs/cmdk/app-integration-spec.md` § Slice 6f.

- [ ] **Restore OS-based theme and 1.0 scale** *(Deferred until 1.0)* — Revert to `"System"` theme, persist user prefs.

- [ ] **Bundle SQLite for release builds** *(Deferred until 1.0)* — Re-enable `rusqlite/bundled` feature for release builds so the binary ships a known SQLite version with FTS5 guaranteed. Dev builds use system libsqlite3 for faster compiles.

- [ ] **Reconsider sidebar layout** *(Deferred until right before 1.0)* — Currently the spec says: (1) sidebar should not show any Labels section when "All Accounts" is selected, (2) when a single account is selected, only labels belonging to that account should be shown, and (3) that for providers that have a "folder" concept, the users folders should show in the Labels section. We might need to re-think all 3.

## Roadmap Features — Remaining Work

Features with backend complete but UI or integration work remaining. Each references its roadmap spec.

### Labels Unification — `docs/labels-unification/problem-statement.md`

Phases 1-6 complete (backend unified). **10 discrepancies remain** — see `docs/labels-unification/discrepancies.md`. Critical: command palette rejects non-Gmail label operations, palette queries use legacy type filtering. Also:

- [ ] **Label picker overlay** — Triggered from reading pane or command palette. Lists all available tag-type labels with colors for apply/remove.

### Search — `docs/search/problem-statement.md`

Backend pipeline exists (parser, SQL builder, Tantivy, unified router). **29 discrepancies remain** — see `docs/search/discrepancies.md`. Critical: combined path applies free text in SQL before Tantivy ranking, Tantivy-only results show wrong message metadata, date boundaries inconsistent across engines. Also typeahead, pinned search lifecycle, and smart folder management gaps.

### Calendar — `docs/calendar/problem-statement.md`

Views, editor, pop-out, sidebar all partially implemented. **39 discrepancies remain** — see `docs/calendar/discrepancies.md`. Critical: new event creation broken (no calendar selector), calendar sync never triggered from app, timezone handling treats everything as UTC, two competing CalDAV implementations. Also drag interactions, RSVP actions, reminder system, meeting invite detection.

### Generic OAuth — `docs/generic-oauth/problem-statement.md`

Core OIDC discovery + OAUTHBEARER implemented. **6 discrepancies remain** — see `docs/generic-oauth/discrepancies.md`. Critical: re-auth broken for generic/OIDC providers (registry lookup fails for non-built-in provider IDs). Also no manual issuer URL flow, no client ID entry, JMAP OAuth unsupported.

### Chats — `docs/chats/problem-statement.md`

Backend plumbing complete (schema, sync, core APIs, timeline view). Feature unreachable by users. **7 discrepancies remain** — see `docs/chats/discrepancies.md`. Critical: no sidebar entry point, no body text rendering, no mark-read, no inline compose.

### Tracking Blocking — `docs/roadmap/tracking-blocking.md`

Sanitization pipeline, MDN detection, tracking pixel detection, URL cleaning all done. Remaining:

- [ ] **Read receipt prompt UI** — `read_receipt_policy` table and `mdn.rs` policy resolution exist. Need UI prompt when opening a message with `mdn_requested=true`: "Send read receipt?" with per-sender/per-account policy options (ask/always/never).
- [ ] **Read receipt policy management in Settings** — Settings panel for configuring default MDN policy per account and per-sender overrides.

### Cloud Attachments — `docs/roadmap/cloud-attachments.md`

OneDrive and Google Drive upload both implemented. Remaining:

- [ ] **Compose UI for cloud attachment flow** — Size threshold detection in compose, prompt to upload to cloud, upload progress indicator, insert link into message body. Orchestration logic exists in `core/cloud_attachments.rs`.
- [ ] **Offline upload queue** — Queue uploads when offline, retry when connectivity returns.

### Public Folders — `docs/roadmap/public-folders.md`

EWS SOAP client, autodiscover routing, offline sync, IMAP NAMESPACE public folders, DB schema all done. Sidebar pins done (2026-03-22). Remaining:

- [ ] **Public folder browser** — Lazy-load tree widget for browsing the hierarchy and pinning folders. Uses existing `browse_public_folders()` API.
- [ ] **Reply/post wiring** — Connect compose to `CreateItem` EWS operation for replies and posts to public folders.

### Shared Mailboxes — `docs/roadmap/shared-mailboxes.md`

Exchange Graph sync + Autodiscover + sidebar integration done. Remaining:

- [ ] **Gmail delegation support** — Blocked (API limitation). Send-As aliases work.
- [ ] **Per-mailbox sync depth config** — Currently hardcoded to 30 days. No per-mailbox setting.

### JMAP Sharing — `docs/roadmap/jmap-sharing.md`

All 6 backend phases complete (discovery, sync, rights, subscription, notifications, identity resolution). Remaining app-crate UI integration:

- [ ] **Subscription toggle in sidebar** — `NavigationFolder.is_subscribed` is populated from JMAP `isSubscribed`. App needs a UI toggle (context menu or button) on shared account labels that calls `JmapOps::subscribe_mailbox()` / `unsubscribe_mailbox()`. These accept an optional `jmap_account_id` for shared accounts.
- [ ] **Compose identity auto-selection from shared mailbox** — `shared_mailbox_sync_state.email_address` is resolved via JMAP Principals (Phase 6). When replying from a shared mailbox context, compose should query `sync_state::get_shared_mailbox_email()` and auto-set From. Also check `may_submit` from the mailbox rights before offering the identity.

### Labels — `docs/labels-unification/problem-statement.md`

- [ ] **Label picker UI** — Overlay for applying/removing tag-type labels from messages. Triggered from reading pane or command palette. Lists all available labels with colors. Provider dispatch via `add_tag()`/`remove_tag()`.

### Mentions — `docs/roadmap/mentions.md`

- [ ] **Compose @-autocomplete** — Detect `@` in compose editor, show floating contact picker, insert `@Display Name` text, auto-add to To/CC if not already a recipient. Works identically across all providers (cosmetic markup only).

### Scheduled Send — `docs/roadmap/scheduled-send.md`

Backend complete (server delegation + overdue handling). Missing UI.

- [ ] **Schedule picker UI** — Date/time picker in compose toolbar. Delegates to Exchange (deferred delivery) or JMAP (FUTURERELEASE) server-side, falls back to local timer for Gmail/IMAP.
- [ ] **"Scheduled" virtual folder** — Virtual folder view showing all pending scheduled messages across accounts with edit/reschedule/cancel.

### Signatures — `docs/roadmap/signatures.md`

Backend complete (Gmail + JMAP sync). Exchange fetch permanently blocked (no public API, Microsoft confirmed no plans).

### BIMI — `docs/roadmap/bimi.md`

Backend complete (DNS + SVG + cache).

- [ ] **BIMI cache** — Is this actually working? I dont think we cache for example if we get no response. We should cache that as well, and not re-ping every time. Caches should persist across sessions as well so we dont re-ping BIMI for every email every time we start the app.

### Auto-Responses — `docs/auto-responses/problem-statement.md`

Read/write API complete on all 3 providers. Remaining:

- [ ] **Auto-reply settings UI** — Per-account editor in settings. Toggle, date pickers, message editor, audience selector. Internal/external tabs for Exchange only. Provider HTML must be sanitized before rendering (stored unsanitized in DB).

### IMAP CONDSTORE/QRESYNC — `docs/roadmap/imap-condstore-qresync.md`

Phases 1-2 complete. Phase 3 blocked on upstream.

- [ ] **QRESYNC VANISHED parsing** — Blocked on `async-imap` upstream (Issue #130). UID-based deletion detection works as workaround.

## Blocked / External

- [ ] **Ship a default Microsoft OAuth client ID** — Manual Azure AD registration task.
- [ ] **QRESYNC VANISHED parsing** — Blocked on `async-imap` upstream (Issue #130). See above.

## Remaining Enhancements (HTML rendering)

The DOM-to-widget pipeline (`html_render.rs`) handles structural HTML but has significant fidelity gaps. Remaining:
- [ ] **Inline text formatting** — `<strong>`, `<b>`, `<em>`, `<i>`, `<u>`, `<s>`, `<code>` (inline) all ignored. Everything renders as plain text. Needs a `Vec<Span>` model per block or `iced::widget::rich_text`.
- [ ] Remote image loading with user consent (`block_remote_images` setting exists but disconnected from `render_html` — function signature needs context parameter)
- [ ] Table rendering (table-for-layout is the hardest — no `<table>`/`<tr>`/`<td>` handling at all)
- [ ] Image caching (`HashMap<String, image::Handle>`) — no `iced::widget::image` usage in app crate

## Security / Bug Findings (unfixed)

- [ ] **`contact_photo_cache` join duplicates chat sidebar entries** — Keyed by `(email, account_id)`, so contacts with photos from multiple accounts produce duplicate rows.
- [ ] **Microsoft ID token not signature-verified** — JWT payload is base64-decoded and trusted for email/name claims without verifying the signature. Token comes over TLS from Microsoft, but a MITM or compromised endpoint could inject arbitrary identity claims.

## Remaining Enhancements (other)

- [ ] **iced_drop for cross-container DnD** — Custom DragState works for list reorder. iced_drop needed for: compose token DnD, label drag-to-file, calendar event dragging, attachment drag zones.
- [ ] **Read receipts (outgoing)** — MDN support. See `docs/roadmap/tracking-blocking.md`.
- [ ] **Inline image store eviction UI** — Settings control for store size (128 MB hardcoded).

- [ ] **Provider push notifications (remaining)** — JMAP WebSocket push is wired. Still missing: IMAP IDLE (persistent connection per folder), Graph/Gmail (poll-based, needs tuning — true push requires cloud infrastructure).
- [ ] **Pop-out HTML rendering** — SimpleHtml/OriginalHtml modes in message view pop-out fall back to plain text. Depends on the DOM-to-widget pipeline (`html_render.rs`) being wired into the pop-out view. Tracked separately in the HTML rendering section above.
- [ ] **Pop-out Print** — OS print dialog integration for message view and compose pop-out windows. Platform-specific, no iced precedent. Needs investigation.
- [ ] **Signature: per-account default dropdown in Account Settings** — Account editor overlay has no signature dropdown for selecting the default signature for an account.
- [ ] **CardDAV contact write-back** — CardDAV client supports PROPFIND/REPORT/GET but not PUT/DELETE. Need vCard generation + PUT method for pushing contact edits to CardDAV servers. See `docs/contacts/problem-statement.md`.

## Needs Visual Review

Completed features that need to be visually verified in the running app.

- **Compose block-type format toggles** — Blockquote button wired in toolbar. Fixed apply_set_block_type for blockquote-to-paragraph conversion.
- **Compose identity auto-selection (shared mailboxes)** — Auto-selects shared mailbox email when replying from SharedMailbox scope.
- **Rights gating on action buttons (JMAP sharing)** — Mailbox rights flow through CommandContext. Actions disabled when rights deny.
- **Signature placement in compose** — Auto-resolved on compose open. New compose: bottom. Reply: between content and quoted text.
- **BIMI avatar display** — Wired BimiLruCache to thread list sender avatars with circular image, initials fallback.
- **Active auto-reply status indicator** — Status bar shows "Out of Office auto-reply is active" when any account has enabled auto-replies.
- **CID image loading from inline image store** — Wired through thread detail → HTML renderer.

## Cross-Cutting Architecture Patterns

Living reference — follow these patterns as features are built. Keep until 1.0.

- **Generational load tracking** — 9 branded `GenerationCounter<T>` instances across App and component levels. See `docs/architecture.md`.

- **Component trait** — 8 components: Sidebar, ThreadList, ReadingPane, Settings, StatusBar, AddAccountWizard, Palette, ChatTimeline. Non-components use free functions + App handler methods: Compose, Calendar, Pop-out windows.

- **Token-to-Catalog theming** — Zero inline closure violations. Exceptions: rich text editor (builder methods), token input (renderer.fill_quad).

- **Config shadow pattern** — Formal: `PreferencesState`. Implicit (clone-on-open): Account editor, Contact editor, Group editor, Calendar event editor, Signature editor. Editors work on a shadow copy and commit on save.

- **DOM-to-widget pipeline** — V1 in `html_render.rs`. Supports links, CID images, block structure. Complexity heuristic (table depth >5, style tags >2) falls back to plain text. Used in reading pane only (NOT in pop-out message view). Remaining: inline formatting, remote images, tables.
