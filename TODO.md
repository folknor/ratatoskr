# TODO

## Remaining Work

- [ ] **Message box / toast notification system** — Generic modal message box and/or toast notification infrastructure for the app. Needed for: compose draft save failure on close (currently silently aborts the close with no user feedback), action service retry exhaustion warnings, and any future error/confirmation flows. Should support at least: transient toasts (auto-dismiss), persistent error banners, and modal confirmation dialogs.

- [ ] **Starred thread card background** — The golden tint on starred thread cards uses a fixed `mix()` ratio (`STARRED_BG_ALPHA`) which may not look right across all themes. Needs a GPU-level blend/shader effect that adapts to the theme's background luminance so the starred highlight reads consistently in both light and dark themes.

- [ ] **Star icon: need filled variant** — Lucide only has outline icons. The star toggle in the reading pane needs a filled star (golden) for the active state and an outline star for inactive. Currently uses Unicode ★ as a stopgap, which causes size mismatch and visual jank. Options: (1) add a second icon font with filled variants, (2) use an SVG/image icon, (3) custom widget that draws a filled star path. The button should also not change background color on toggle — just the icon fill.

- [x] **Collapse individual expanded messages** — *(needs visual review)* Chevron-down button in expanded message header, chevron-right on collapsed rows.

- [ ] **Contact pills on recipients** — Per `docs/pop-out-windows/problem-statement.md`: recipients in To/Cc fields should appear as plain text but become contact pills on hover, revealing an inline edit button for quick contact editing. Applies to: reading pane message headers, pop-out message view, compose window recipient display. Currently recipients are plain text everywhere with no hover interaction. Needs: (1) a contact pill widget that blends with background at rest and reveals pill styling + edit button on hover, (2) display name resolution from the contact system (name → email fallback chain), (3) wiring to the existing `EditContact` flow that opens the settings contact editor.

- [x] **Email body background override setting** — *(needs visual review)* Three-option setting in Preferences (Always White, Match Theme, Auto). Auto checks theme luminance.

- [x] **App logo in first-launch modal** — *(needs visual review)* SVG rendered via iced svg feature, embedded with include_bytes.

- [ ] **Action service: user-facing retry status** *(Deferred)* — When an action fails remotely and gets enqueued for pending-ops retry, the user has no visibility. The thread disappears from inbox (local mutation applied), the status bar says "Archived", but there's no persistent indicator that actions are waiting for retry or have exhausted retries. The infrastructure exists: `db_pending_ops_count()` returns pending count, `db_pending_ops_failed_count()` returns exhausted count, `db_pending_ops_retry_failed()` resets failed ops for manual retry. What's missing is UI: a status bar badge or indicator showing "N actions pending retry", a section in settings listing pending/failed ops with operation details and a "retry now" button, and a notification when retries exhaust ("Archive failed after 10 retries — will resolve on next sync"). Without this, the user has no way to know their actions are silently diverged from the server until sync reconciles (or doesn't, if the sync pipeline doesn't cover that state).

- [ ] **Action service: native provider batching** *(Deferred)* — Currently `batch_execute` reuses one provider per account but still makes one HTTP request per thread (sequential `provider.archive()` calls). Some providers support batching natively: Gmail batch API (up to 100 requests in one HTTP multipart request), Graph `/$batch` endpoint (up to 20 per batch), JMAP `Email/set` can modify multiple emails in one method call, IMAP `STORE` can set flags on multiple UIDs in one command. Native batching would reduce 50 HTTP round-trips to 1-3 for bulk operations. Requires adding batch methods to `ProviderOps` (e.g., `archive_batch(&self, ctx, thread_ids: &[&str]) -> Vec<Result<(), ProviderError>>`), implementing per provider (IMAP would need UID set formatting, Gmail needs multipart boundary encoding, Graph needs JSON batch request assembly, JMAP needs method call batching), and updating `batch.rs` to prefer batch methods when available and fall back to sequential for providers that don't implement them. The per-account sequential approach works fine for now — provider reuse eliminated the construction overhead, and the remaining latency is network-bound.

- [ ] **Crate structure and dependency graph** - So much has been implemented without any real consideration for what kind of code lives where. It might be time to get a grip on things.

- [ ] **Scroll virtualization** — Thread list renders all cards in `column![]` inside `scrollable`. Needs iced-level virtual scrolling for large mailboxes.

- [ ] **Scroll-to-selected in palette** — Arrow keys update `selected_index` but `scrollable::scroll_to` doesn't exist in our iced fork. Needs alternative approach.

- [x] **Compose block-type format toggles** — *(needs visual review)* Blockquote button wired in toolbar. Fixed apply_set_block_type for blockquote-to-paragraph conversion.

- [ ] **`responsive` for adaptive layout** — Collapse panels at narrow window sizes.

- [x] **Per-pane minimum resize limits** — *(needs visual review)* Sidebar 220, thread list 250, reading pane 300. Divider drag and window resize both clamped.

- [ ] **Keybinding management UI (Slice 6f)** — Settings panel for viewing, searching, and rebinding shortcuts. Backend ready (override persistence, conflict detection, set/unbind/reset APIs). See `docs/command-palette/app-integration-spec.md` § Slice 6f.

- [ ] **Restore OS-based theme and 1.0 scale** *(Deferred until 1.0)* — Revert to `"System"` theme, persist user prefs.

## Roadmap Features — Remaining Work

Features with backend complete but UI or integration work remaining. Each references its roadmap spec.

### Labels Unification — `docs/labels-unification/problem-statement.md`

Phases 1-5 complete (schema, Exchange/IMAP/JMAP sync, local dispatch + provider write-back, sidebar). Remaining:

- [x] **Label pills in reading pane** — *(needs visual review)* Tag-type labels as colored pills on expanded message headers.
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
- [x] **Compose identity auto-selection** — *(needs visual review)* Auto-selects shared mailbox email when replying from SharedMailbox scope.
- [ ] **Gmail delegation support** — Blocked (API limitation). Send-As aliases work.
- [ ] **Per-mailbox sync depth config** — Currently hardcoded to 30 days. No per-mailbox setting.

### JMAP Sharing — `docs/roadmap/jmap-sharing.md`

All 6 backend phases complete (discovery, sync, rights, subscription, notifications, identity resolution). Remaining app-crate UI integration:

- [x] **Rights gating on action buttons** — *(needs visual review)* Mailbox rights flow through CommandContext. Actions disabled when rights deny.
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

- [x] **Signature placement in compose** — *(needs visual review)* Auto-resolved on compose open. New compose: bottom. Reply: between content and quoted text.

### BIMI — `docs/roadmap/bimi.md`

Backend complete (DNS + SVG + cache).

- [x] **BIMI avatar display** — *(needs visual review)* Wired BimiLruCache to thread list sender avatars with circular image, initials fallback.

### Auto-Responses — `docs/auto-responses/problem-statement.md`

Read/write API complete on all 3 providers. Remaining:

- [ ] **Auto-reply settings UI** — Per-account editor in settings. Toggle, date pickers, message editor, audience selector. Internal/external tabs for Exchange only.
- [x] **Active auto-reply status indicator** — *(needs visual review)* Status bar shows "Out of Office auto-reply is active" when any account has enabled auto-replies.

### IMAP CONDSTORE/QRESYNC — `docs/roadmap/imap-condstore-qresync.md`

Phases 1-2 complete. Phase 3 blocked on upstream.

- [ ] **QRESYNC VANISHED parsing** — Blocked on `async-imap` upstream (Issue #130). UID-based deletion detection works as workaround.

## Blocked / External

- [ ] **Ship a default Microsoft OAuth client ID** — Manual Azure AD registration task.
- [ ] **QRESYNC VANISHED parsing** — Blocked on `async-imap` upstream (Issue #130). See above.

## Remaining Enhancements (HTML rendering)

The DOM-to-widget pipeline (`html_render.rs`) handles structural HTML but has significant fidelity gaps. Remaining:
- [ ] **Inline text formatting** — `<strong>`, `<b>`, `<em>`, `<i>`, `<u>`, `<s>`, `<code>` (inline) all ignored. Everything renders as plain text. Needs a `Vec<Span>` model per block or `iced::widget::rich_text`.
- [x] **Link rendering + click handling** — *(needs visual review)* Accent-colored clickable links, opens in system browser.
- [x] CID image loading from inline image store — *(needs visual review)* Wired through thread detail → HTML renderer.
- [ ] Remote image loading with user consent (`block_remote_images` setting exists but disconnected from `render_html` — function signature needs context parameter)
- [ ] Table rendering (table-for-layout is the hardest — no `<table>`/`<tr>`/`<td>` handling at all)
- [ ] Image caching (`HashMap<String, image::Handle>`) — no `iced::widget::image` usage in app crate

## Bug Hunt Findings (review agent, 2026-03-27)

- [ ] **`from_address` nullable crash in chat timeline** — `row.get::<_, String>("from_address")` on NULL silently drops the message. Mailer-daemon/DSN messages can have no From. `chat.rs:368`
- [ ] **N+1 per-user-email queries in `maybe_update_chat_state`** — One SQL query per user email per thread per sync. Replace with single `IN` query. `persistence.rs:312-317`
- [ ] **`user_emails()` excludes send-as aliases** — Only primary account emails. Send-as aliases break chat ownership detection and 1:1 thread classification. Should include `send_identities` table. `handlers/chat.rs:91`
- [ ] **Chat summary stale when thread stops qualifying** — Early-return paths clear `is_chat_thread` but don't recompute `chat_contacts` summary. Thread deletion also skips summary update. `persistence.rs:278,291,434`
- [ ] **`enter_chat_view` doesn't clear pinned-search context** — `sidebar.active_pinned_search` persists, overriding `NavigationTarget::Chat` in `current_view_and_label()`. `handlers/chat.rs:11`
- [ ] **Reading pane star sync keyed by thread_id only** — `update_star()` compares only `t.id`, not `(account_id, thread_id)`. Cross-account thread ID collision can mutate wrong thread. `reading_pane.rs:249`
- [ ] **Chat subject normalization panics on non-ASCII** — `normalize_subject()` slices original string using byte lengths from lowercased copy. Unicode lowercase can change byte length → invalid slice boundary. `chat_timeline.rs:221`
- [ ] **Scope dropdown missing public folder entries** — Dropdown has All Accounts + accounts + shared mailboxes but no public folders. `sidebar.rs:413`

## Security Audit Findings (review agent, 2026-03-27)

- [ ] **Link click command injection on Windows** — Email HTML links are passed directly to `cmd /c start` with no scheme allowlist. A malicious `href` can reach a shell-adjacent sink. On all platforms, arbitrary schemes (`file:`, custom handlers) are opened from untrusted content. Fix: allowlist `http://`, `https://`, `mailto:` only. `reading_pane.rs:730, html_render.rs:217`
- [ ] **ammonia still allows `data:` scheme globally** — Stage 2 (lol_html) restricts data: URIs, but ammonia (stage 3) still permits the `data` scheme. If stage 2 fails, `data:text/html` in `<a href>` passes through. Remove `"data"` from ammonia's `url_schemes`. `html_sanitizer.rs:207`
- [ ] **`data:image/svg+xml` allowed in sanitizer** — SVG is active content, not a passive bitmap. Allowing it through the sanitizer is risky if output ever reaches a richer renderer. `html_sanitizer.rs:18`
- [ ] **OAuth error responses reflect raw provider bodies** — `oauth_exchange_token`, `oauth_refresh_token`, userinfo fetch include `response.text()` verbatim in errors. Could leak secrets or expose untrusted remote content in logs/UI. `oauth.rs:572,615,673`
- [ ] **`contact_photo_cache` join duplicates chat sidebar entries** — Keyed by `(email, account_id)`, so contacts with photos from multiple accounts produce duplicate rows. `chat.rs:141`
- [ ] **Auto-response HTML stored unsanitized** — Future XSS risk when the auto-reply settings UI renders it. Sanitize before display. `auto_responses.rs`
- [ ] **Encryption key not zeroized on drop** — `[u8; 32]` remains in freed memory. Consider `zeroize::Zeroize`. `crypto.rs:8`

## Security Findings (review agent, 2026-03-25)

- [ ] **Microsoft ID token not signature-verified** — JWT payload is base64-decoded and trusted for email/name claims without verifying the signature. Token comes over TLS from Microsoft, but a MITM or compromised endpoint could inject arbitrary identity claims. `oauth.rs:735-771`

## Remaining Enhancements (other)

- [ ] **iced_drop for cross-container DnD** — Custom DragState works for list reorder. iced_drop needed for: compose token DnD, label drag-to-file, calendar event dragging, attachment drag zones.
- [ ] **Read receipts (outgoing)** — MDN support. See `docs/roadmap/tracking-blocking.md`.
- [ ] **Inline image store eviction UI** — Settings control for store size (128 MB hardcoded).

- [ ] **Provider push notifications (remaining)** — JMAP WebSocket push is wired. Still missing: IMAP IDLE (persistent connection per folder), Graph/Gmail (poll-based, needs tuning — true push requires cloud infrastructure).
- [ ] **Pop-out HTML rendering** — SimpleHtml/OriginalHtml modes in message view pop-out fall back to plain text. Depends on the DOM-to-widget pipeline (`html_render.rs`) being wired into the pop-out view. Tracked separately in the HTML rendering section above.
- [ ] **Pop-out Print** — OS print dialog integration for message view and compose pop-out windows. Platform-specific, no iced precedent. Needs investigation.
- [ ] **Signature: per-account default dropdown in Account Settings** — Account editor overlay has no signature dropdown for selecting the default signature for an account.
- [ ] **CardDAV contact write-back** — CardDAV client supports PROPFIND/REPORT/GET but not PUT/DELETE. Need vCard generation + PUT method for pushing contact edits to CardDAV servers. See `docs/contacts/problem-statement.md`.

## Cross-Cutting Architecture Patterns

Living reference — follow these patterns as features are built. Keep until 1.0.

- **Generational load tracking** — 9 branded `GenerationCounter<T>` instances across App and component levels. See `docs/architecture.md`.

- **Component trait** — 8 components: Sidebar, ThreadList, ReadingPane, Settings, StatusBar, AddAccountWizard, Palette, ChatTimeline. Non-components use free functions + App handler methods: Compose, Calendar, Pop-out windows.

- **Token-to-Catalog theming** — Zero inline closure violations. Exceptions: rich text editor (builder methods), token input (renderer.fill_quad).

- **Config shadow pattern** — Formal: `PreferencesState`. Implicit (clone-on-open): Account editor, Contact editor, Group editor, Calendar event editor, Signature editor. Editors work on a shadow copy and commit on save.

- **DOM-to-widget pipeline** — V1 in `html_render.rs`. Supports links, CID images, block structure. Complexity heuristic (table depth >5, style tags >2) falls back to plain text. Used in reading pane only (NOT in pop-out message view). Remaining: inline formatting, remote images, tables.
