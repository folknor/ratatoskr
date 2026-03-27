# TODO

## Remaining Work

- [ ] **Message box / toast notification system** — Generic modal message box and/or toast notification infrastructure for the app. Needed for: compose draft save failure on close (currently silently aborts the close with no user feedback), action service retry exhaustion warnings, and any future error/confirmation flows. Should support at least: transient toasts (auto-dismiss), persistent error banners, and modal confirmation dialogs.

- [ ] **Better MIME handling** - for example in app/src/pop_out_compose.rs mime_from_extension, hardcoded things like this.

- [ ] **Starred thread card background** — The golden tint on starred thread cards uses a fixed `mix()` ratio (`STARRED_BG_ALPHA`) which may not look right across all themes. Needs a GPU-level blend/shader effect that adapts to the theme's background luminance so the starred highlight reads consistently in both light and dark themes.

- [ ] **Star icon: need filled variant** — Lucide only has outline icons. The star toggle in the reading pane needs a filled star (golden) for the active state and an outline star for inactive. Currently uses Unicode ★ as a stopgap, which causes size mismatch and visual jank. Options: (1) add a second icon font with filled variants, (2) use an SVG/image icon, (3) custom widget that draws a filled star path. The button should also not change background color on toggle — just the icon fill.

- [ ] **Collapse individual expanded messages** — Removed the full-card click-to-collapse overlay because it intercepted all clicks on the message body. Need a dedicated collapse affordance — e.g. clicking the message header row (sender/date area), a small collapse chevron button, or a right-click context menu option.

- [ ] **Contact pills on recipients** — Per `docs/pop-out-windows/problem-statement.md`: recipients in To/Cc fields should appear as plain text but become contact pills on hover, revealing an inline edit button for quick contact editing. Applies to: reading pane message headers, pop-out message view, compose window recipient display. Currently recipients are plain text everywhere with no hover interaction. Needs: (1) a contact pill widget that blends with background at rest and reveals pill styling + edit button on hover, (2) display name resolution from the contact system (name → email fallback chain), (3) wiring to the existing `EditContact` flow that opens the settings contact editor.

- [ ] **Email body background override setting** — Email body areas are always rendered on a white background for fidelity (HTML emails are authored against white). Users should be able to override this to use the theme's background instead, for a fully immersive dark mode experience at the cost of email rendering accuracy. Setting in Preferences with three options: "Always white" (default), "Match theme", "Auto" (white in light themes, theme bg in dark themes).

- [x] **Codebase contracts** — *(2026-03-27)* All 24 implicit contracts fixed. See `docs/architecture.md` for the principles and enforcement mechanisms.

- [ ] **App logo in first-launch modal + about page** — `assets/icon.svg` exists but isn't rendered anywhere. Needs iced `svg` feature enabled to use `iced::widget::svg`. SVG preferred over PNG because the icon should be re-colored to match the active theme (e.g. primary color tint). Requires adding `"svg"` to the iced features list in `crates/app/Cargo.toml`.

- [x] **Action service: NoOp detection** — *(2026-03-25)* `ActionOutcome::NoOp` variant added. DB helpers return affected row counts. `archive` and `star` use the affected-rows approach to skip provider dispatch + undo tokens when state didn't change. Other actions have the infrastructure (`usize` returns) but don't detect NoOp yet — can be upgraded incrementally per action.

- [ ] **Action service: user-facing retry status** *(Deferred)* — When an action fails remotely and gets enqueued for pending-ops retry, the user has no visibility. The thread disappears from inbox (local mutation applied), the status bar says "Archived", but there's no persistent indicator that actions are waiting for retry or have exhausted retries. The infrastructure exists: `db_pending_ops_count()` returns pending count, `db_pending_ops_failed_count()` returns exhausted count, `db_pending_ops_retry_failed()` resets failed ops for manual retry. What's missing is UI: a status bar badge or indicator showing "N actions pending retry", a section in settings listing pending/failed ops with operation details and a "retry now" button, and a notification when retries exhaust ("Archive failed after 10 retries — will resolve on next sync"). Without this, the user has no way to know their actions are silently diverged from the server until sync reconciles (or doesn't, if the sync pipeline doesn't cover that state).

- [ ] **Action service: native provider batching** *(Deferred)* — Currently `batch_execute` reuses one provider per account but still makes one HTTP request per thread (sequential `provider.archive()` calls). Some providers support batching natively: Gmail batch API (up to 100 requests in one HTTP multipart request), Graph `/$batch` endpoint (up to 20 per batch), JMAP `Email/set` can modify multiple emails in one method call, IMAP `STORE` can set flags on multiple UIDs in one command. Native batching would reduce 50 HTTP round-trips to 1-3 for bulk operations. Requires adding batch methods to `ProviderOps` (e.g., `archive_batch(&self, ctx, thread_ids: &[&str]) -> Vec<Result<(), ProviderError>>`), implementing per provider (IMAP would need UID set formatting, Gmail needs multipart boundary encoding, Graph needs JSON batch request assembly, JMAP needs method call batching), and updating `batch.rs` to prefer batch methods when available and fall back to sequential for providers that don't implement them. The per-account sequential approach works fine for now — provider reuse eliminated the construction overhead, and the remaining latency is network-bound.

- [ ] **Hardcoded values** - We need to do a sweep of the codebase for hardcoded values that shouldn't be. These need to be extracted to a common location so that we can keep track of them and decide whether or not to make them configurable.

- [ ] **Crate structure and dependency graph** - So much has been implemented without any real consideration for what kind of code lives where. It might be time to get a grip on things.

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

Phases 1-5 complete (schema, Exchange/IMAP/JMAP sync, local dispatch + provider write-back, sidebar). Remaining:

- [ ] **Label pills in reading pane** — Display tag-type labels as colored pills on expanded message headers. Data now in `thread_labels` via unified sync.
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
- [ ] **Compose identity auto-selection** — When replying from shared mailbox context, auto-set From to shared mailbox address. `send_as_shared_mailbox()` and `send_on_behalf_of()` APIs exist.
- [ ] **Gmail delegation support** — Blocked (API limitation). Send-As aliases work.
- [ ] **Per-mailbox sync depth config** — Currently hardcoded to 30 days. No per-mailbox setting.

### JMAP Sharing — `docs/roadmap/jmap-sharing.md`

All 6 backend phases complete (discovery, sync, rights, subscription, notifications, identity resolution). Remaining app-crate UI integration:

- [ ] **Rights gating on action buttons** — `NavigationFolder.rights` (`MailboxRightsInfo`) is populated from synced `myRights`. App should check `may_delete`, `may_rename`, `may_submit` etc. before showing action buttons. Especially important for shared/read-only mailboxes where the user lacks write permissions.
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

- [ ] **Signature placement in compose** — Insert signature in compose body. New compose: bottom. Reply: between new content and quoted text. Wrap in `<div id="ratatoskr-signature">` for replacement/stripping.

### BIMI — `docs/roadmap/bimi.md`

Backend complete (DNS + SVG + cache). Missing UI wiring.

- [ ] **BIMI avatar display** — Wire `BimiLruCache` to message list sender avatars. Fall back to initials when no BIMI logo cached.

### Auto-Responses — `docs/auto-responses/problem-statement.md`

Not yet implemented. Full read/write API available on Exchange, Gmail, and JMAP.

- [x] **Exchange auto-reply read/write** — *(2026-03-25)* `fetch_graph_auto_response` / `push_graph_auto_response` in `core/auto_responses.rs`. Internal/external messages, scheduling, audience control.
- [x] **Gmail vacation settings read/write** — *(2026-03-25)* `fetch_gmail_auto_response` / `push_gmail_auto_response`. Epoch ms timestamps converted to RFC 3339.
- [x] **JMAP VacationResponse read/write** — *(2026-03-25)* `fetch_jmap_auto_response` / `push_jmap_auto_response` using `jmap-client` fork's VacationResponse support.
- [ ] **Auto-reply settings UI** — Per-account editor in settings. Toggle, date pickers, message editor, audience selector. Internal/external tabs for Exchange only.
- [ ] **Active auto-reply status indicator** — Status bar or sidebar indicator when any account has active auto-replies.

### IMAP CONDSTORE/QRESYNC — `docs/roadmap/imap-condstore-qresync.md`

Phases 1-2 complete. Phase 3 blocked on upstream.

- [ ] **QRESYNC VANISHED parsing** — Blocked on `async-imap` upstream (Issue #130). UID-based deletion detection works as workaround.

## Blocked / External

- [ ] **Ship a default Microsoft OAuth client ID** — Manual Azure AD registration task.
- [ ] **QRESYNC VANISHED parsing** — Blocked on `async-imap` upstream (Issue #130). See above.

## Remaining Enhancements (HTML rendering)

The DOM-to-widget pipeline (`html_render.rs`) handles structural HTML but has significant fidelity gaps. Remaining:
- [ ] **Inline text formatting** — `<strong>`, `<b>`, `<em>`, `<i>`, `<u>`, `<s>`, `<code>` (inline) all ignored. Everything renders as plain text. Needs a `Vec<Span>` model per block or `iced::widget::rich_text`.
- [ ] **Link rendering + click handling** — `<a href>` tags treated as plain text. URLs not extracted. Need `href` extraction, visual link styling, and `LinkClicked(url)` message emission.
- [ ] CID image loading from inline image store (`InlineImageStoreState` exists in stores crate, not wired to renderer)
- [ ] Remote image loading with user consent (`block_remote_images` setting exists but disconnected from `render_html` — function signature needs context parameter)
- [ ] Table rendering (table-for-layout is the hardest — no `<table>`/`<tr>`/`<td>` handling at all)
- [ ] Image caching (`HashMap<String, image::Handle>`) — no `iced::widget::image` usage in app crate

## Performance Findings (review agent, 2026-03-25)

- [x] **Body store zstd under lock** — Zstd compress/decompress runs inside the Mutex lock closure. Should compress/decompress outside the lock and only hold it for the DB read/write. `body_store.rs:108-110, 171-172, 225-226`
- [x] **IMAP per-folder connections** — Flag sync and deletion detection open a separate TLS connection per folder. 50 folders = 50 handshakes. Should reuse a single connection with SELECT. `imap_delta.rs:817-894, 988-1031`
- [x] **Reading pane rebuild on expand/collapse** — All message widgets are rebuilt on any expand/collapse toggle. Should diff and rebuild only the affected message. `reading_pane.rs:565-602`
- [x] **Attachment dedup in view()** — HashMap allocation for attachment dedup runs every `view()` cycle, not memoized. `reading_pane.rs:606-626`
- [x] **get_thread_detail lock span** — Holds the DB lock across 7 sequential queries. Should batch or reduce lock scope. `thread_detail.rs:524-575`
- [x] **Contact autocomplete LIKE %pattern%** — No FTS index; uses LIKE with leading wildcard which can't use indexes. `contacts.rs:43-46`
- [x] **JWZ is_ancestor() quadratic** — `is_ancestor()` is O(depth) per link, O(n²) on deep linear threads. `threading.rs:63-72`
- [x] **Attachment cache eviction lock churn** — Deletes one file per loop iteration with 2+ lock acquisitions each. Should collect files to delete, release the lock, then delete in batch. `attachment_cache.rs:188-262`
- [x] **Navigation tag unread counts** — 3-table LEFT JOIN with `LOWER(TRIM())` in GROUP BY for tag unread counts. `navigation.rs:325-339`

## Chats Optimization Findings (review agent, 2026-03-27)

- [ ] **`maybe_update_chat_state` queries ALL accounts per call** — Runs `SELECT email FROM accounts` on every sync-time call. Should cache user emails per sync batch or pass as parameter. `persistence.rs:maybe_update_chat_state`
- [x] **`set_chat_thread_flags` N+1 queries** — Iterates threads individually with per-thread participant count + user check. Should use a single SQL statement with HAVING clause per the plan. `chat.rs:set_chat_thread_flags`
- [x] **`get_chat_timeline` loads ALL messages then truncates** — Queries all messages across all threads, sorts in Rust, then takes last N. Should use SQL LIMIT + reverse, or paginate per-thread with a merge. `chat.rs:get_chat_timeline`
- [x] **Summary update scans ALL chat threads** — Fixed: latest-message query now scoped to the current thread; only updates contact summary when this thread has a newer message than what's stored, skipping the expensive cross-thread join in the common case. `persistence.rs:maybe_update_chat_state`
- [x] **`contact_photo_cache` join uses LOWER()** — Defeats index on `cpc.email`. Cache table should use NOCASE collation or store normalized emails. `chat.rs:get_chat_contacts`

- [ ] **IMAP re-threading can orphan `thread_participants`** — Delta re-threading is rare, but when it happens old participant rows for the previous thread ID are not cleaned up. Low impact, accumulates slowly.
- [x] **Legacy `queries.rs` helpers missing `is_chat_thread = 0`** — `get_threads`, `get_thread_count`, `get_unread_count` in `queries.rs` don't filter chat threads. No current call sites, but residual regression risk if reused. `queries.rs:155, 803, 840`
- [ ] **`get_threads_for_shared_mailbox` missing `is_chat_thread = 0`** — Shared mailbox thread query doesn't exclude chat threads. Low risk (shared mailboxes unlikely to have chat contacts). `scoped_queries.rs:get_threads_for_shared_mailbox`
- [ ] **Self-to-self threads can misclassify as chat** — If user designates one of their own aliases, a thread between two of the user's own accounts could qualify as "1:1 chat". Edge case.
- [x] **`LOWER()` on NOCASE columns** — Several chat queries use `LOWER()` on columns already declared COLLATE NOCASE, defeating index use. Remove the redundant `LOWER()` calls.
- [ ] **IMAP `is_connection_error` missing patterns** — Missing "broken pipe", "reset by peer", "end of file". Also differs between `imap_delta.rs` and `imap_initial.rs` — should be unified. `imap_delta.rs`
- [ ] **IMAP batch delta check loses tail folders on mid-batch disconnect** — If shared session dies halfway through `delta_check_folders`, remaining folders are silently omitted from results. Before reuse, one broken connection only lost one folder. `imap_delta.rs`
- [ ] **IMAP deletion detection skips folder that triggered reconnect** — On error, reconnects and moves to next folder. The failed folder is unchecked until next 10-minute window. `imap_delta.rs`
- [ ] **`seen_addresses` LIKE %pattern% for 3+ char queries** — FTS5 only covers `contacts` table. `seen_addresses` (potentially 50k+ rows) still does full scan for substring matches. Consider FTS5 on seen_addresses. `contacts.rs`
- [ ] **`make_like_pattern` doesn't escape `%` and `_` wildcards** — User input with LIKE wildcards passes through unescaped. Low risk but technically incorrect. `contacts.rs`
- [ ] **Duplicate `make_like_pattern` in two files** — Identical function in `app/db/contacts.rs` and `core/contacts/search.rs`. Extract to shared module.
- [ ] **`fetch_all_flags` doc says "must NOT be SELECTed"** — Stale with session-reuse model. IMAP re-SELECT is harmless. `commands.rs`
- [ ] **Chat timeline pagination cursor** — Currently timestamp-only (`date < ?`). Should use `(date, message_id)` tuple to avoid skipping/duplicating messages with equal timestamps. `chat.rs:get_chat_timeline`
- [ ] **ChatTimeline should implement Component trait** — Currently has ad-hoc `update()`/`view()` methods. UI.md says components implement the shared Component trait. `chat_timeline.rs`
- [ ] **Missing sidebar divider in chat layout** — Normal mail view has a draggable divider between sidebar and content. Chat view omits it. `main.rs` view branching
- [ ] **Search state persists across chat entry/exit** — If user enters chat from search mode, search state persists. Exiting chat may return to stale search. `handlers/chat.rs:enter_chat_view`
- [ ] **Subject change indicator triggers on Re: variations** — `needs_subject_indicator` compares raw subjects. Should normalize by stripping Re:/Fwd: prefixes. `chat_timeline.rs`
- [ ] **`scroll_id` stored but never attached to scrollable** — Deferred scroll-to-bottom needs a usable hook. `chat_timeline.rs`
- [ ] **Chat view command context classifies as Inbox** — `view_type_from_target` returns `ViewType::Inbox` for `Chat` target. Palette/command availability logic thinks user is in Inbox. `command_dispatch.rs`

## Security Findings (review agent, 2026-03-25)

- [x] **`decrypt_or_raw` silent plaintext fallback** — On decryption failure, silently returns the raw ciphertext as if it were plaintext. Masks key corruption or rotation issues — credentials could pass through as garbage strings with no error signal. `crypto.rs:141`
- [ ] **Microsoft ID token not signature-verified** — JWT payload is base64-decoded and trusted for email/name claims without verifying the signature. Token comes over TLS from Microsoft, but a MITM or compromised endpoint could inject arbitrary identity claims. `oauth.rs:735-771`
- [x] **`data:` URI allowed beyond images in sanitizer** — `data:` scheme is generically permitted. Allows `data:text/html,...` in `<a href>` which can be used for phishing. Should restrict `data:` to `<img src>` only or limit to image MIME types. `html_sanitizer.rs:170`
- [x] **CSS `url()` bypasses remote image blocking** — Remote image blocking only covers `<img src>`. A `<div style="background:url(https://tracker/pixel.gif)">` delivers a tracking pixel uncaught. `html_sanitizer.rs:147`
- [x] **Path traversal in `remove_cached_relative`** — Checks for `attachment_cache/` prefix but doesn't canonicalize the path. Should canonicalize before the prefix check. `attachment_cache.rs:67`
- [x] **Hand-rolled `parse_query_string`** — Custom URL query parsing instead of using `form_urlencoded` crate. Unnecessary attack surface. `oauth.rs:513`

## Review Findings — Yesterday's Commits (review agent, 2026-03-27)

### Auto-responses (8d04916)
- [x] **Graph push sends wrong JSON shape for reply messages** — `internalReplyMessage` wrapped in `{ "message": ... }` object instead of plain string. Will fail or corrupt reply text on push. `auto_responses.rs:200`
- [x] **Graph schedule discards timezone on fetch** — Reads `dateTime` but drops `timeZone`. Pushes back with hardcoded `"UTC"`. Round-trip shifts out-of-office window for non-UTC mailboxes. `auto_responses.rs:155, 205`
- [x] **JMAP enable + set_dates non-atomic** — Two separate `VacationResponse/set` requests. If second fails, vacation is enabled without schedule constraints. Should use a single set request. `auto_responses.rs:384-399`
- [x] **Gmail `restrictToDomain` mapped to `ContactsOnly`** — Lossy round-trip. Domain restriction becomes contacts restriction on push. Need `ExternalAudience::DomainOnly` variant or explicit handling. `auto_responses.rs:261, 290`
- [x] **Exchange dates not normalized to RFC 3339** — Cross-provider push (Exchange → Gmail) silently drops dates because `.NET` datetime format lacks timezone offset. `auto_responses.rs:155, 281`

### IMAP OAUTHBEARER (133fff2)
- [x] **Raw IMAP helpers not updated for `"oauthbearer"` auth method** — `raw_fetch_messages` and `raw_fetch_diagnostic` fall through to LOGIN, sending bearer token as password. `raw.rs:57, 190`
- [x] **RFC 7628 GS2 header may need full email as authzid** — Current format uses bare username. Strict implementations may reject. `connection.rs:71`
- [x] **OAUTHBEARER error acknowledgment should be `\x01`** — Current implementation sends empty vec on second challenge instead of RFC 7628 §3.2.3 single `\x01` byte. `connection.rs:78`

### GAL sync (dd0c6e8)
- [x] **`cache_gal_entries` not transactional** — DELETE + N INSERTs without transaction. Crash between DELETE and final INSERT loses cache. Also N individual fsyncs. `gal.rs:34-62`
- [x] **Empty directory results don't update cache age** — Returns early on empty, `cached_at` never advances, retries every 5 minutes indefinitely. Also stale entries persist when directory access is revoked. `gal.rs:262, 273`

### NoOp detection (69bf316)
- [x] **All-NoOp batch shows success toast + auto-advance** — "Archived" toast and selection advance when nothing changed. Should show no toast or "Already archived". `commands.rs:handle_action_completed`
- [ ] **NoOp conflates "already in state" with "target missing"** — 0 affected rows for both cases. Can't distinguish no-op from invalid target. `archive.rs:12, star.rs:12`
- [x] **Degraded batch path erases NoOp signal** — `action_local()` discards the bool via `.map(|_| ())`, returns `LocalOnly` instead of `NoOp`. Causes spurious pending-ops enqueue and undo tokens. `batch.rs:316, 322`

## Remaining Enhancements (other)

- [ ] **iced_drop for cross-container DnD** — Custom DragState works for list reorder. iced_drop needed for: compose token DnD, label drag-to-file, calendar event dragging, attachment drag zones.
- [ ] **Read receipts (outgoing)** — MDN support. See `docs/roadmap/tracking-blocking.md`.
- [ ] **Inline image store eviction UI** — Settings control for store size (128 MB hardcoded).

- [ ] **Provider push notifications (remaining)** — JMAP WebSocket push is wired. Still missing: IMAP IDLE (persistent connection per folder), Graph/Gmail (poll-based, needs tuning — true push requires cloud infrastructure).
- [ ] **Pop-out HTML rendering** — SimpleHtml/OriginalHtml modes in message view pop-out fall back to plain text. Depends on the DOM-to-widget pipeline (`html_render.rs`) being wired into the pop-out view. Tracked separately in the HTML rendering section above.
- [ ] **Pop-out Print** — OS print dialog integration for message view and compose pop-out windows. Platform-specific, no iced precedent. Needs investigation.
- [ ] **Pop-out default rendering mode from settings** — `MessageViewState` hardcodes `RenderingMode::default()` (SimpleHtml). Should load from a system-wide user preference. Needs a settings field + plumbing to pass it into `from_thread_message()` and `from_session_entry()`.
- [ ] **Signature: draft restoration with signature state** — Draft save does not persist `signature_separator_index` or `active_signature_id`. On draft reopen, signature position in the document is not reconstructed.
- [ ] **Signature: per-account default dropdown in Account Settings** — Account editor overlay has no signature dropdown for selecting the default signature for an account.
- [x] **GAL sync trigger** — *(2026-03-25)* `refresh_gal_for_account()` in core, wired to `SyncTick`. Checks cache age (>24h stale threshold), creates provider client, fetches + caches. Runs alongside delta sync every 5 minutes.
- [ ] **CardDAV contact write-back** — CardDAV client supports PROPFIND/REPORT/GET but not PUT/DELETE. Need vCard generation + PUT method for pushing contact edits to CardDAV servers. See `docs/contacts/problem-statement.md`.

## Cross-Cutting Architecture Patterns

Living reference — follow these patterns as features are built. Keep until 1.0.

- **Generational load tracking** — Applied to: nav, thread, search, palette, pop-out, sync, autocomplete, add-account wizard, calendar events, search typeahead. All verified and wired 2026-03-22. No known gaps.

- **Component trait** — 7 components: Sidebar, ThreadList, ReadingPane, Settings, StatusBar, AddAccountWizard, Palette. All verified 2026-03-22. Non-components use free functions + App handler methods: Compose, Calendar, Pop-out windows. Conversion optional — current pattern works.

- **Token-to-Catalog theming** — Zero inline closure violations. Verified 2026-03-22. Exceptions: rich text editor (builder methods), token input (renderer.fill_quad).

- **Config shadow pattern** — Formal: `PreferencesState`. Implicit (clone-on-open): Account editor, Contact editor, Group editor, Calendar event editor, Signature editor. Missing: contact import wizard (creation wizard — value of formal shadow debatable).

- **DOM-to-widget pipeline** — V1 in `html_render.rs`. Complexity heuristic (table depth >5, style tags >2) falls back to plain text. Used in reading pane only (NOT in pop-out message view). See HTML rendering section above for remaining work — significant fidelity gaps (no inline formatting, no links, no images).
