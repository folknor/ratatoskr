# TODO

## Remaining Work

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

- [ ] **Typed IDs: CommandArgs fields** — `CommandArgs::MoveToFolder { folder_id: String }`, `AddLabel { label_id: String }`, `RemoveLabel { label_id: String }` in `command-palette` are still raw strings. Wrapping happens at the `command_dispatch.rs` boundary (3 lines in `dispatch_parameterized`). The palette crate can't depend on `provider-utils` (pulls in reqwest, rusqlite, stores, search). Fix would require either extracting `FolderId`/`TagId` to a micro-crate or duplicating the newtypes in the palette crate.
- [ ] **Typed IDs: sidebar.selected_label** — `sidebar.selected_label: Option<String>` is wrapped to `FolderId` at call sites. 32 references across the app. The field is semantically ambiguous — it holds folder IDs for navigation (e.g., "INBOX", "TRASH") AND label IDs for tag highlighting. Typing it as `FolderId` would be wrong in label contexts; typing it as a union would add complexity. The root issue is that the sidebar conflates "which container am I viewing" with "which label is highlighted."
- [ ] **First-launch modal not dismissible** — In zero-accounts state, cancel doesn't close the wizard. Spec says it should dismiss over an unusable empty app. Intentional safety measure or bug — decide and document.
- [ ] **Default scope is first account, not All Accounts** — After account load, app selects first account instead of unified All Accounts inbox.
- [ ] **App-specific-password help not clickable** — Discovery types carry `help_url` but UI shows plain text "Check {domain} for setup instructions" — no clickable link to provider app-password pages.
- [ ] **Deleted-account compose/pop-out cleanup** — Account deletion doesn't close compose windows or message-view pop-outs for the deleted account, and doesn't block sending from a deleted identity.
- [ ] **Sync-task cancellation on account deletion** — Delete flow removes DB data but doesn't cancel in-flight sync tasks. Stale sync completions could write to deleted account state.
- [ ] **Search scope respects ViewScope** — `execute_search_sql_fallback` hardcodes `AccountScope::All`. When viewing a shared mailbox or single account, search should be scoped accordingly. Tantivy search path also ignores scope. Follow-up from Contract #10.

- [ ] **Crate structure and dependency graph** - So much has been implemented without any real consideration for what kind of code lives where. It might be time to get a grip on things.

- [ ] **Scroll virtualization** — Thread list renders all cards in `column![]` inside `scrollable`. Needs iced-level virtual scrolling for large mailboxes.

- [ ] **Scroll-to-selected in palette** — Arrow keys update `selected_index` but `scrollable::scroll_to` doesn't exist in our iced fork. Needs alternative approach.

- [ ] **`responsive` for adaptive layout** — Collapse panels at narrow window sizes.

- [ ] **Keybinding management UI (Slice 6f)** — Settings panel for viewing, searching, and rebinding shortcuts. Backend ready (override persistence, conflict detection, set/unbind/reset APIs). See `docs/command-palette/app-integration-spec.md` § Slice 6f.

- [ ] **Restore OS-based theme and 1.0 scale** *(Deferred until 1.0)* — Revert to `"System"` theme, persist user prefs.

## Roadmap Features — Remaining Work

Features with backend complete but UI or integration work remaining. Each references its roadmap spec.

### Labels Unification — `docs/labels-unification/problem-statement.md`

Phases 1-5 complete (schema, Exchange/IMAP/JMAP sync, local dispatch + provider write-back, sidebar). Remaining:

- [ ] **Label picker overlay** — Triggered from reading pane or command palette. Lists all available tag-type labels with colors for apply/remove.

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

- [ ] **Thread loading on selection** — App handler for `PublicFolderSelected` event to load threads from `public_folder_items` into thread list.
- [ ] **Public folder browser** — Lazy-load tree widget for browsing the hierarchy and pinning folders. Uses existing `browse_public_folders()` API.
- [ ] **Reply/post wiring** — Connect compose to `CreateItem` EWS operation for replies and posts to public folders.

### Shared Mailboxes — `docs/roadmap/shared-mailboxes.md`

Exchange Graph sync + Autodiscover + sidebar integration done. Remaining:

- [ ] **Thread loading on selection** — App handler for `SharedMailboxSelected` event to load navigation and threads for the selected shared mailbox.
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

## Bug Hunt Findings (review agent, 2026-03-27)

- [ ] **Scope dropdown missing public folder entries** — Dropdown has All Accounts + accounts + shared mailboxes but no public folders.

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

- **Collapse individual expanded messages** — Chevron-down button in expanded message header, chevron-right on collapsed rows.
- **Email body background override setting** — Three-option setting in Preferences (Always White, Match Theme, Auto). Auto checks theme luminance.
- **App logo in first-launch modal** — SVG rendered via iced svg feature, embedded with include_bytes.
- **Compose block-type format toggles** — Blockquote button wired in toolbar. Fixed apply_set_block_type for blockquote-to-paragraph conversion.
- **Per-pane minimum resize limits** — Sidebar 220, thread list 250, reading pane 300. Divider drag and window resize both clamped.
- **Label pills in reading pane** — Tag-type labels as colored pills on expanded message headers.
- **Compose identity auto-selection (shared mailboxes)** — Auto-selects shared mailbox email when replying from SharedMailbox scope.
- **Rights gating on action buttons (JMAP sharing)** — Mailbox rights flow through CommandContext. Actions disabled when rights deny.
- **Signature placement in compose** — Auto-resolved on compose open. New compose: bottom. Reply: between content and quoted text.
- **BIMI avatar display** — Wired BimiLruCache to thread list sender avatars with circular image, initials fallback.
- **Active auto-reply status indicator** — Status bar shows "Out of Office auto-reply is active" when any account has enabled auto-replies.
- **Link rendering + click handling (HTML)** — Accent-colored clickable links, opens in system browser.
- **CID image loading from inline image store** — Wired through thread detail → HTML renderer.

## Cross-Cutting Architecture Patterns

Living reference — follow these patterns as features are built. Keep until 1.0.

- **Generational load tracking** — 9 branded `GenerationCounter<T>` instances across App and component levels. See `docs/architecture.md`.

- **Component trait** — 8 components: Sidebar, ThreadList, ReadingPane, Settings, StatusBar, AddAccountWizard, Palette, ChatTimeline. Non-components use free functions + App handler methods: Compose, Calendar, Pop-out windows.

- **Token-to-Catalog theming** — Zero inline closure violations. Exceptions: rich text editor (builder methods), token input (renderer.fill_quad).

- **Config shadow pattern** — Formal: `PreferencesState`. Implicit (clone-on-open): Account editor, Contact editor, Group editor, Calendar event editor, Signature editor. Editors work on a shadow copy and commit on save.

- **DOM-to-widget pipeline** — V1 in `html_render.rs`. Supports links, CID images, block structure. Complexity heuristic (table depth >5, style tags >2) falls back to plain text. Used in reading pane only (NOT in pop-out message view). Remaining: inline formatting, remote images, tables.
