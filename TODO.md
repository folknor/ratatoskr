# TODO

## Migration Backlog

### AI Boundary

- [ ] **Decide whether AI inference execution should move to Rust** — Rust already owns provider/runtime/config selection, but TypeScript still owns prompt assembly and actual inference calls for summaries, smart replies, transforms, ask-inbox, task extraction, smart-label AI, category inference, and auto-drafts. This needs an explicit boundary decision, not ad-hoc drift.

- [x] **Deduplicate the shared `callAi` wrapper** — `aiService.ts` and `writingStyleService.ts` still define the same `callAi(systemPrompt, userContent)` helper. If inference remains in TypeScript, this should collapse to one shared wrapper or direct `completeAi` use.

### Regression Coverage

- [ ] **Expand regression coverage around migrated sync/bootstrap behavior** — Add focused tests for sync status events, background sync start/stop, post-sync hook triggering, and account bootstrap paths that now rely on Rust-backed summary DTOs.

- [ ] **Replace the magic microtask loop in `flushListenerSetup`** — The current 8-iteration `await Promise.resolve()` loop is brittle and hides ordering assumptions in sync listener tests.

## Inline Image Store Eviction

- [ ] **Wire up user-configurable eviction for `inline_images.db`** — The file-based attachment cache (`attachment_cache/`) has full user-facing eviction: a configurable max size in settings, `evictOldestCached()` in `cacheManager.ts` that respects content-hash dedup, and `clearAllCache()`. The inline image store has none of this plumbing on the TS/UI side, even though the Rust backend already has the building blocks (`prune_to_size()`, `delete_unreferenced()`, `stats()`, `clear()`).

  **What exists today (Rust side)**:
  - `prune_to_size(max_bytes)` — evicts oldest rows by `created_at ASC` until total size fits under the cap. Already called automatically after every `put()` and `put_batch()` with a hardcoded 128 MB ceiling (`MAX_INLINE_STORE_BYTES`).
  - `delete_unreferenced(db, hashes)` — cross-references `attachments` table to find orphaned blobs (no remaining `is_inline = 1` rows with that `content_hash`), then deletes them.
  - `stats()` — returns `{ image_count, total_bytes }`.
  - `clear()` — deletes all rows.
  - `inline_image_stats` and `inline_image_clear` Tauri commands already exposed.

  **What's missing**:
  1. **Settings UI**: No user-facing control for inline image store size. The 128 MB cap is hardcoded in Rust. The settings page should expose this alongside the existing attachment cache size slider, or at minimum show the current usage via `inline_image_stats`.
  2. ~~**Orphan cleanup on account/message deletion**~~: Done — `db_delete_account` and `provider_prepare_account_resync` now collect inline content hashes before deletion, then call `delete_unreferenced()` to clean orphaned blobs.
  3. **Scheduled eviction**: The file cache runs eviction after every `provider_fetch_attachment` cache-on-miss (via `enforce_cache_limit`). The inline store's `prune_to_size` runs after `put`/`put_batch` which covers sync-time inserts, but there's no periodic sweep to catch edge cases (e.g., if `MAX_INLINE_STORE_BYTES` is lowered in a future update). Consider adding a periodic call in `preCacheManager.ts` or a dedicated background task.
  4. ~~**Tauri command for configurable limit**~~: Done — `inline_image_prune` command accepts a custom `max_bytes` limit and triggers immediate eviction. The 128 MB default constant remains for automatic post-insert pruning; the command enables UI-driven limit changes.

## Iced Rewrite

- [ ] **Investigate iced ecosystem projects** — Review these repos for patterns, widget implementations, and architecture ideas:
  - https://github.com/hecrj/iced_fontello — Icon font integration for iced
  - https://github.com/hecrj/iced_palace — Hecrj's iced showcase/playground
  - https://github.com/pop-os/cosmic-edit — COSMIC text editor (large real-world iced app)
  - https://github.com/pop-os/iced/blob/master/widget/src/markdown.rs — COSMIC fork's markdown widget: two-phase architecture using `pulldown_cmark` to parse into an `Item` enum, then a `Viewer` trait to render items as iced widgets (`rich_text` for text/headings, `container` for code blocks, `row`+`column` for lists, `table` for tables, syntax highlighting via `highlighter` feature). Supports incremental parsing and span caching. Relevant for rendering HTML email bodies.

## Non-Migration Cleanup

### Branding

- [ ] **Replace logo SVG** — `src/assets/logo.svg` still renders the old "VELO" text as path outlines. Needs a new logo for Ratatoskr.

- [ ] **Replace app icons** — `src-tauri/icons/`, `assets/icon.png`, `src/assets/logo.svg`, and the inline SVG in `splashscreen.html` still contain old Velo branding. Need new Ratatoskr icons for all platforms.

### Code Quality

- [x] **Category add/remove is racy** — Fixed with a per-account `category_lock` mutex on `GraphClient` that serializes the read-modify-write. Also batched the GET and PATCH phases via `/$batch` to reduce round-trips.

- [x] **Add Graph `$batch` optimization for thread actions** — `move_messages`, `patch_messages`, and `permanent_delete` now batch up to 20 operations per `POST /$batch` call. Category add/remove still uses per-message GET-then-PATCH (read-modify-write pattern not batchable).

- [ ] **Decide whether Graph `raw_size = 0` should stay accepted** — Graph still lacks a clean size field for the current query path. Either keep this as an accepted cosmetic limitation or document a better fallback if one exists.

- [x] **Deduplicate account-to-store mapping in the React entry points** — The shared account-store shaping now lives in `src/services/accounts/basicInfo.ts::mapAccountBasicInfos()`, and `App.tsx`, `ComposerWindow.tsx`, and `ThreadWindow.tsx` all use that helper.

### Per-Account OAuth Credentials

- [x] **Add per-account credential editing UI for existing accounts** — Gmail and Graph accounts now expose an explicit per-account “Update OAuth App” reauth path in settings. The UI reads that account’s stored credentials, lets the user inspect or change them, and reauthorizes without any cross-account prefill/default behavior.

- [x] **Clean up orphaned global credential settings rows** — Migration v29 backfilled per-account `oauth_client_id`/`oauth_client_secret` from the global `settings` table, but the original `google_client_id`, `google_client_secret`, and `microsoft_client_id` rows remain in the `settings` table as dead data. Add a follow-up migration or cleanup step to delete these rows once the per-account migration has been live long enough.

### Microsoft Graph

- [ ] **Decide on Azure AD app registration model** — Currently users must provide their own `microsoft_client_id` during account setup. No default client ID is shipped. The open question is whether to register and ship a default Entra ID app registration (simpler onboarding, but requires maintaining an Azure AD app, handling consent prompts, and staying within Microsoft's rate limits across all users) or keep requiring user-provided credentials (friction for non-technical users, but zero shared infrastructure). Credentials are now stored per-account — this is a product/policy decision, not a code gap.

- [x] **Implement large attachment upload sessions (>3MB)** — Graph API rejects inline base64 attachments over 3MB. Larger files require a multi-step resumable upload session: `POST /me/messages/{id}/attachments/createUploadSession` → chunked `PUT` to the returned upload URL. Currently `graph/ops.rs` uses simple `POST /me/messages/{id}/attachments` which will fail silently or error for large files. Need to detect size threshold, create upload session, chunk the file (recommended 5-10MB chunks per Microsoft docs), and handle resume on failure. Affects `send_message` (create-draft-then-send pattern in `ops.rs` lines ~211-249) and `save_draft`.

- [ ] **Add Graph webhook subscriptions for real-time sync** — Currently Graph sync is purely poll-based via delta queries with priority-based folder scheduling (`sync.rs`). Microsoft Graph supports change notifications via webhooks (`POST /subscriptions` with `changeType: "created,updated,deleted"` on `/me/messages`). This would enable near-instant inbox updates instead of waiting for the next sync cycle. Requires: a notification endpoint (likely a Tauri localhost server or push notification relay), subscription lifecycle management (create, renew before 3-day expiry, handle validation tokens), and delta sync as fallback when subscriptions lapse. Low priority while polling works acceptably.

- [x] **Wire up Focused Inbox data from Graph** — `inferenceClassification` is now deserialized into `GraphMessage` and surfaced as a "FOCUSED" pseudo-label in `thread_labels` (same pattern as UNREAD/STARRED). Persisted via the existing label pipeline — no schema migration needed.

### JMAP

- [ ] **Add Bearer/OAuth authentication for JMAP** — Currently `jmap/client.rs` uses `Credentials::basic()` only, ignoring the `auth_method` column in the accounts table. This means JMAP providers that require OAuth2 (e.g., Fastmail's OAuth flow) can't be used. Implementation needs: (a) read `auth_method` from DB in `read_jmap_credentials()` and branch on `"oauth2"`/`"bearer"` to use `Credentials::bearer(token)`, (b) handle token refresh — `jmap-client` binds credentials at construction, so either rebuild `JmapClientInner` on refresh or patch the crate for a credential callback, (c) per-provider OAuth endpoint config (Fastmail has its own auth/token URLs and scopes, distinct from Microsoft/Google), (d) account setup UI flow for OAuth JMAP providers. The `auth_method` column and IMAP/Graph OAuth infra already exist as reference patterns.

- [ ] **Add JMAP push notifications via WebSocket** — Currently JMAP sync is purely poll-based via `email_changes()`/`mailbox_changes()` state strings in `sync.rs`. The JMAP spec (RFC 8620 §7) defines push via EventSource or WebSocket, and `jmap-client` 0.4 supports WebSocket push. This would enable near-instant sync instead of waiting for the next poll cycle. Requires: WebSocket connection lifecycle management, state change event parsing, and triggering delta sync on push events. Delta polling remains as fallback. Low priority while polling works.

- [ ] **Add JMAP Sieve filter management** — `jmap-client` supports full Sieve CRUD (RFC 6785 over JMAP). This would allow users to manage server-side mail filters (filing rules, vacation replies, forwarding) directly from the UI. Currently no Sieve code exists. Requires: Sieve script CRUD commands, a filter rule editor UI, and testing against Fastmail/Stalwart/Cyrus Sieve implementations. Nice-to-have feature, not blocking.

- [x] **Fetch `List-Unsubscribe` header in JMAP sync** — The `messages` table has `list_unsubscribe` and `list_unsubscribe_post` columns, but JMAP sync sets them to NULL. JMAP can fetch arbitrary headers via `header:List-Unsubscribe:asText` and `header:List-Unsubscribe-Post:asText` properties in `Email/get`. Add these to the property list in `parse.rs`'s `email_get_properties()`, parse the values, and persist them. This enables one-click unsubscribe UI for JMAP accounts (IMAP and Gmail already populate these columns).

- [x] **Batch `Email/set` for JMAP thread actions** — All thread-level actions in `jmap/ops.rs` (archive, trash, move, mark read, star, spam) loop through email IDs and call `email_set_mailbox()`/`email_set_keyword()` per-email sequentially — one API round-trip per email in the thread. Should build a single `Email/set` request with patches for all email IDs using jmap-client's request builder, reducing N API calls to 1. The per-email convenience methods (`email_set_mailbox`, `email_set_keyword`) don't support batching; need to drop to the lower-level `set_email()` builder with explicit patch operations.

- [x] **Batch JMAP send into a single request** — `send_email` now runs `upload()` and `Identity/get` concurrently, then batches `Email/import` + `EmailSubmission/set` (with `onSuccessUpdateEmail` to clear `$draft`) into a single JMAP request. Reduced from 5 sequential round-trips to 2 steps.

- [x] **Add `is_known_jmap_provider()` quick-check utility** — `registry::is_known_jmap_provider(domain)` checks the hardcoded registry for JMAP support (no network calls). Exposed as a Tauri command for UI hints during account setup.

- [ ] **JMAP for Calendars** — `jmap-client` has no calendar support (upstream Issue #3). JMAP for Calendars (RFC 8984) would unify calendar sync for providers like Fastmail that support it. Blocked until `jmap-client` adds calendar types, or we build raw JMAP calendar requests. Low priority — CalDAV covers calendar sync for now.

## Roadmap — Backend-Only Work

Items below are derived from `docs/roadmap/` and scoped to Rust backend work only (no UI/frontend). See the individual roadmap docs for full context.

### Categories (Tier 1)

- [x] **Gmail label color sync** — Already implemented: `GmailLabelColor` struct, `GmailLabel.color` field, and `sync_labels_to_categories()` with `ON CONFLICT DO UPDATE` for color persistence.

- [ ] **Gmail label-as-category classification heuristic** — Not all Gmail labels are "categories" — some are folder-like. Distinguish them using `messageListVisibility` (show vs hide), nesting (`/` in name = folder hierarchy), and `labelListVisibility`. Labels that behave as tags get synced to `categories`; folder-like ones stay as mailbox folders.

- [x] **JMAP keyword-to-category mapping** — Implemented: `parse_jmap_email` extracts non-`$` keywords into `keyword_categories`, `sync_keyword_categories()` upserts them into `categories` table with `kw_` prefix and links to threads via `thread_categories`.

- [x] **IMAP PERMANENTFLAGS detection for category writeback** — Implemented: `supports_custom_keywords: bool` on `ImapFolderStatus`, populated from `Flag::MayCreate` in SELECT responses across all 4 construction sites. Raw TCP fallback also handles it.

- [ ] **`ProviderOps` methods for category mutation** — Add `apply_category` / `remove_category` trait methods with provider-specific implementations: Graph uses `PATCH /me/messages/{id}` with full `categories` array replacement; Gmail uses `messages.modify` with `addLabelIds`/`removeLabelIds`; JMAP uses `Email/set` keyword patches; IMAP uses `UID STORE +FLAGS`/`-FLAGS`. Default impl does local-only DB write. Wire into the offline action queue.

- [ ] **Populate `message_categories` during sync** — When messages are fetched/synced, parse their category associations (Graph `categories[]` array, Gmail label IDs matched against category-classified labels, JMAP non-system keywords) and populate the `message_categories` join table. Currently sync writes messages but doesn't link them to categories.

- [ ] **Unified color model with Exchange presets as canonical palette** — Implement a const array mapping Exchange's 25 preset names to hex values. When syncing Gmail label colors, map to the nearest Exchange preset by color distance. Store both `color_preset` (for Exchange round-tripping) and `color_bg`/`color_fg` (for rendering) in the `categories` table. This gives a consistent color picker vocabulary across providers.

### Contacts (Tier 1)

- [ ] **Google People API sync (Phase 5)** — Implement `people.connections.list` with `personFields` mask (names, emailAddresses, phoneNumbers, organizations, photos), `pageToken` pagination, and `syncToken` storage (tokens expire after 7 days — fall back to full resync). Map Google contact fields to the existing `contacts` table schema. Requires `contacts.readonly` OAuth scope added to the Google auth flow.

- [ ] **Google `otherContacts` ingestion** — Fetch `GET /v1/otherContacts` with separate `contacts.other.readonly` scope. These are auto-collected contacts Google tracks (similar to our `seen_addresses`). Insert as lower-priority autocomplete candidates — they rank above locally-observed addresses but below explicit contacts.

- [ ] **Exchange group/distribution list resolution (Phase 6)** — Call `GET /groups/{id}/transitiveMembers/microsoft.graph.user` to resolve M365 Groups and distribution lists into individual recipients. The Graph API handles recursion (nested groups) server-side. Track partial resolution when hidden membership count > resolved count, so the UI can indicate "12 members + others".

- [ ] **Contact photo fetching and caching (Phase 7)** — Exchange: `GET /me/contacts/{id}/photo/$value` returns JPEG, cache keyed by `changeKey` for invalidation. Google: public photo URLs from People API with `?sz=` size parameter. Store photos on disk in `{app_data}/contact_photos/` with an eviction policy. Photos are optional enrichment — autocomplete and display work without them.

- [ ] **CardDAV sync (Phase 8)** — Add `libdav` + `calcard` crate dependencies. Implement etag-based contact sync: PROPFIND for etags, GET changed vCards, PUT local changes. Targets Fastmail, Stalwart, and other CardDAV-capable servers. Parses vCard 3.0/4.0 responses via `calcard` into the existing `contacts` schema.

### Tracking Blocking (Tier 1)

- [ ] **`sanitize_html_body()` pipeline in core** — Implement the three-stage HTML sanitization function: (1) `css-inline` to inline `<style>` blocks into element attributes, (2) `lol_html` for streaming removal of `<link>`, `<script>`, `<iframe>`, remote `<img>` sources, `@import` rules, and `meta refresh` tags, (3) `ammonia` for whitelist-based DOM sanitization as a final safety net. This is a framework-agnostic core function called before any rendering backend (React webview or future iced HTML renderer). Three new crate dependencies.

- [x] **AMP email content blocking** — Implemented: shared `is_amp_content_type()` utility, filtering in JMAP `extract_body_value()`, defense-in-depth guards in Gmail and IMAP parsers. Graph unaffected (pre-rendered body).

- [ ] **`$MDNSent` keyword management across providers** — Before sending a read receipt (MDN), check if the `$MDNSent` keyword is already set on the message to avoid duplicate receipts. After sending, set the keyword. Provider-specific: JMAP uses lowercase `$mdnsent`; IMAP uses `$MDNSent` (check `PERMANENTFLAGS` first to see if the server accepts custom keywords); Graph uses the `isReadReceiptRequested` property. Prevents the "send receipt every time you open the message" problem.

- [x] **MDN message generation (RFC 8098)** — Implemented in `mdn.rs`: `build_mdn_message()` builds `multipart/report` with human-readable + `message/disposition-notification` parts. `resolve_read_receipt_policy()` implements most-specific-wins lookup (sender → domain → account → global default). 8 unit tests.

- [ ] **Tracking domain list + URL parameter stripping** — Ship a resource file of known tracking redirect domains (Mailchimp, SendGrid, etc.). Integrate a URL cleaning pass into the sanitization pipeline to strip `utm_*`, `mc_eid`, `fbclid`, and similar tracking parameters from link URLs. Run during HTML processing so cleaned URLs are what gets persisted/rendered.

- [ ] **1×1 tracking pixel detection** — When the user allows remote images for a sender, inspect fetched images for tracking pixel signatures: check `Content-Length` < threshold or decode dimensions and flag 1×1 transparent images. Surface this as metadata on the message so the UI can optionally indicate "this sender uses tracking pixels" without blocking the images.

### Cloud Attachments (Tier 1)

- [x] **`cloud_attachments` DB table** — Migration v39: table with all columns, indexes on `(message_id)` and partial on `(upload_status)` excluding 'sent'.

- [ ] **OneDrive resumable upload via Graph API** — Two-phase upload: `POST /me/drive/items/root:/Ratatoskr Attachments/{filename}:/createUploadSession` to get an upload URL, then sequential `PUT` requests with `Content-Range` headers. Chunks must be multiples of 320 KiB (Microsoft requirement). Handle resume via `GET` to `uploadUrl` which returns `nextExpectedRanges`. Uses raw `reqwest` — no OneDrive SDK crate needed since the Graph client already exists.

- [ ] **OneDrive sharing link creation** — After upload completes, `POST /me/drive/items/{item-id}/createLink` with `{ "type": "view", "scope": "organization" }`. Parse response for `link.webUrl`. For personal accounts, use `"scope": "anonymous"` instead. The sharing URL is what gets inserted into the email body.

- [ ] **Exchange `referenceAttachment` for cloud links** — When sending an email with a cloud attachment on an Exchange account, `POST /beta/me/messages/{id}/attachments` with `@odata.type: "#microsoft.graph.referenceAttachment"`, `sourceUrl`, `providerType: "oneDriveBusiness"`, and `permission: "view"`. This makes cloud attachments render as proper attachment chips in Outlook recipients' UI instead of bare URLs.

- [ ] **Google Drive resumable upload** — Initiate with `POST /upload/drive/v3/files?uploadType=resumable`, then `PUT` chunks to the resumable URI. Requires adding `drive.file` OAuth scope to the Google auth flow (minimal scope — only grants access to files the app creates, not the user's entire Drive).

- [ ] **Google Drive permission creation** — After upload, `POST /drive/v3/files/{fileId}/permissions` with `{ "role": "reader", "type": "anyone" }` (or `"type": "domain"` for org-restricted sharing). Returns the sharing URL for email body insertion.

- [x] **Incoming cloud link detection via `RegexSet`** — Implemented in `cloud_attachments.rs`: `LazyLock<RegexSet>` with 8 patterns, `detect_cloud_links()` extracts hrefs and matches, `insert_incoming_cloud_links()` persists to DB. `CloudProvider` enum. 10 unit tests.

- [ ] **Cloud link metadata enrichment** — For detected incoming cloud links, fetch file metadata: OneDrive uses `GET /shares/{base64-encoded-url}/driveItem`; Google Drive uses `GET /files/{id}?fields=name,size,mimeType,iconLink` (requires extracting the file ID from the sharing URL). Cache name, size, and MIME type in `cloud_attachments` so the UI can show "Budget.xlsx (2.3 MB)" instead of a bare URL.

- [ ] **Offline upload queue state machine** — Manage outgoing cloud attachment lifecycle: `pending → uploading → uploaded → linked → sent` (plus `failed` with retry). On app restart, reset any `uploading` rows to `pending`, check if upload session URLs are still valid (sessions expire after ~7 days), and resume or restart. Wire into the existing offline action queue pattern.

### IMAP CONDSTORE/QRESYNC (Tier 1)

- [x] **Wire up `modseq` writes in `upsert_folder_sync_state()`** — Fixed: `imap_initial.rs` was the only call site passing `None` — now passes `highest_modseq` from folder status. Delta sync in `imap_delta.rs` was already correct. `FolderSyncState.modseq` was already properly named.

- [ ] **Deletion detection without QRESYNC** — For servers that support CONDSTORE but not QRESYNC (most servers), add periodic `UID SEARCH ALL` to get the full UID set, then diff against locally cached UIDs to find server-side deletions. Run at lower frequency (every 5–10 minutes) since deletion detection is less latency-sensitive than flag changes. Currently deletions are only caught during full syncs.

- [ ] **QRESYNC VANISHED parsing (Phase 3)** — Send `ENABLE QRESYNC` via raw command, then `SELECT mailbox (QRESYNC (<uidvalidity> <modseq> [<known-uids>]))`. Parse `VANISHED (EARLIER) <uid-set>` untagged responses to get deleted UIDs in a single round-trip instead of the UID SEARCH diff approach. Requires a custom response parser since `imap-proto` doesn't handle VANISHED responses. Blocked on async-imap CHANGEDSINCE support (Issue #130).

- [ ] **HIGHESTMODSEQ reset defense** — If the server's HIGHESTMODSEQ is lower than our cached value but UIDVALIDITY is unchanged, treat it as a mod-sequence counter reset (can happen on server migration or mailbox repair) and trigger a full resync of that folder. Without this, the client would skip changes that happened during the reset window.

- [ ] **iCloud QRESYNC workaround** — iCloud advertises QRESYNC in CAPABILITY but has a broken implementation. After sending `ENABLE QRESYNC`, verify the server actually responds with an `ENABLED` response. If not, fall back to CONDSTORE-only mode for that session. Prevents hard failures on iCloud accounts.

- [ ] **UID-based fallback for non-CONDSTORE servers** — For servers without CONDSTORE support (Exchange IMAP, Courier, hMailServer), add periodic `UID FETCH 1:* (FLAGS)` to diff flag changes and `UID SEARCH ALL` for deletion detection. These servers currently have no incremental flag sync path — every sync re-fetches all flags, which is wasteful for large mailboxes.

### Shared Mailboxes (Tier 1)

- [ ] **Request `*.Shared` OAuth scopes for Graph** — Add `Mail.Read.Shared`, `Mail.ReadWrite.Shared`, `Mail.Send.Shared` to the Graph OAuth scope set. Without these, accessing `/users/{shared-mailbox}/messages` returns 403 even if the user has delegate access in Exchange. The scopes are admin-consentable and don't require additional Azure AD app configuration beyond listing them.

- [ ] **Shared mailbox read/write via Graph API** — Change the API path from `/me/...` to `/users/{shared-mailbox-id}/...` for all message operations (list, get, update, delete, send). Same API surface, different path prefix. Delta sync tokens are independent per mailbox, so each shared mailbox needs its own sync state entries. The `GraphClient` needs a `mailbox_id: Option<String>` field to switch path prefixes.

- [ ] **Autodiscover XML parsing for shared mailbox discovery** — Call `https://outlook.office365.com/autodiscover/autodiscover.xml` with the user's OAuth token. Parse `AlternativeMailbox` elements from the SOAP response using `quick-xml` or `roxmltree` (~100–200 lines). Returns the list of auto-mapped shared mailboxes the user has access to, avoiding manual mailbox address entry.

- [ ] **Send As vs Send on Behalf implementation** — Two distinct send modes for shared mailboxes: Send As sets `from` to the shared mailbox address (message appears to come from the shared mailbox); Send on Behalf sets both `from` (shared mailbox) and `sender` (delegate's address, shows "sent on behalf of"). Exchange enforces permissions server-side — the API call just sets the right headers.

- [x] **`send_identities` table** — Migration v42: table with `(account_id, email, display_name, mailbox_id, send_mode, save_to_personal_sent, is_primary)`, UNIQUE on `(account_id, email)`, index on `(account_id)`.

- [ ] **Per-mailbox sync context isolation** — Model each shared/delegated mailbox as an independent sync context with its own delta tokens, retry state, and error tracking. Each shared mailbox gets its own entries in sync state tables. Prevents one shared mailbox's sync failures from blocking another's.

- [ ] **Auto-From selection logic** — Implement priority-based From address selection in core: (1) reply from shared mailbox context → use that mailbox's address, (2) match To/Cc of original against known identities → reply from matched identity, (3) compose from shared mailbox context → use that mailbox, (4) fall back to account primary. Pure matching logic against the `send_identities` table.

- [ ] **IMAP ACL + NAMESPACE discovery** — Implement `MYRIGHTS <mailbox>` and `NAMESPACE` commands via `Session::run_command_and_check_ok()` with custom response parsing. Parse the three-part NAMESPACE response (personal, other users, shared) and use prefixes to `LIST` folders under shared namespaces. This enables shared mailbox access on Dovecot/Cyrus IMAP servers without Exchange.

### Public Folders (Tier 1)

- [ ] **Minimal EWS SOAP client** — Build a focused Exchange Web Services client using `quick-xml` + `reqwest`: SOAP envelope boilerplate (~30 lines), support for `FindFolder`, `GetFolder`, `FindItem`, `GetItem`, `CreateItem` operations. Estimated ~1500–2500 lines total. Reuse existing Graph OAuth tokens with the additional `EWS.AccessAsUser.All` scope. No Graph API equivalent exists for public folders — Microsoft has no plans to add one.

- [ ] **Autodiscover for public folder routing** — Public folder access requires two autodiscover lookups: a SOAP call to get `PublicFolderInformation` (the hierarchy mailbox), and a POX autodiscover for the content mailbox routing. Set `X-AnchorMailbox` and `X-PublicFolderMailbox` headers on each request. Different folders may route to different content mailboxes.

- [ ] **`PR_REPLICA_LIST` decoding for content mailbox routing** — Fetch extended property `0x6698` (Binary) from the FindFolder response to determine which content mailbox hosts a specific folder's items. Decode the base64 blob to extract the content mailbox GUID, construct `{GUID}@{domain}` as an SMTP address, then autodiscover that address to get the correct EWS endpoint. Track per-folder content mailbox mappings.

- [ ] **Public folder DB schema** — Create tables: `public_folders` (hierarchy cache with folder_id, parent_id, display_name, folder_class, unread_count, effective_rights), `public_folder_items` (synced items), `public_folder_pins` (user-favorited folders + sync settings like depth and frequency), `public_folder_sync_state` (per-folder sync cursors using timestamps, since public folders don't support delta tokens).

- [ ] **Offline sync for pinned public folders** — Polling-based sync: `FindItem` with `DateTimeReceived >= last_sync_timestamp` for new items, match by `ItemId + ChangeKey` for updates, and periodic full `ItemId`-only `FindItem` + local diff for deletion detection. No change notifications or delta tokens available for public folders, so polling is the only option.

- [ ] **IMAP NAMESPACE-based public folder access** — For non-Exchange IMAP servers (Dovecot, Cyrus), discover public namespaces via the `NAMESPACE` command and `LIST` folders under the public prefix. Access with standard IMAP `SELECT`/`FETCH`. Add a `namespace_type` column to the existing folder table to distinguish personal, other-users, and shared folders.

### Mentions (Tier 2)

- [x] **`mentions` table and `is_mentioned` column** — Migration v40: `mentions` table with indexes, `is_mentioned` column on `messages` with partial index.

- [ ] **Exchange `mentionsPreview` sync via Graph beta** — Switch to the Graph beta endpoint for message sync to include `mentionsPreview` in `$select`. Extract `mentionsPreview.isMentioned` and populate `messages.is_mentioned` during delta sync. The beta endpoint is identical to v1.0 for all other fields — only `mentions` requires beta. This is a lightweight sync-time flag, not the full mention details.

- [ ] **Lazy-load full mention details on message open** — When opening a message where `is_mentioned = 1`, fetch `GET /beta/me/messages/{id}?$expand=mentions` to get the full mentions array (who mentioned whom, with names and addresses). Upsert into the `mentions` table. Avoids fetching mention details for every message during sync — only loads them on demand.

- [ ] **Send mentions in Graph API calls** — When sending via `POST /beta/me/sendMail` on an Exchange account, include the `mentions` array with `mentioned.name` and `mentioned.address` for each @-mention. The compose layer passes mention metadata; the backend serializes it into the Graph beta API request body.

- [ ] **HTML body mention correlation** — Parse message HTML body for `<a href="mailto:...">` tags and match email addresses against `mentions` table entries. Return structured mention annotations alongside the body content so the rendering layer can style mentioned names differently (bold, highlighted) without the renderer needing to know about the mentions data model.

### Signatures (Tier 2)

- [x] **Signature sync DB schema** — Migration v41: six columns added (`server_id`, `body_text`, `is_reply_default`, `source`, `last_synced_at`, `server_html_hash`) with unique index on `(account_id, server_id)`.

- [x] **Gmail `sendAs` signature fetch** — Implemented: `sync_signatures()` / `persist_signatures()` in `gmail/sync.rs`. Fetches via `list_send_as()`, upserts with SHA-256 hash for conflict detection, hooked into both initial and delta sync paths.

- [ ] **Gmail bidirectional signature sync** — On local signature edit, push to Gmail via `PUT /gmail/v1/users/me/settings/sendAs/{sendAsEmail}`. Conflict resolution uses hash comparison (`server_html_hash`): if server changed and local didn't, update local; if local changed and server didn't, push; if both changed, prefer server and surface a conflict notification.

- [x] **JMAP Identity signature sync** — Implemented in `jmap/signatures.rs`: `sync_jmap_identity_signatures()` fetches all identities and upserts signatures with SHA-256 hashing. `push_signature_to_jmap()` pushes local edits back via `Identity/set`.

- [ ] **Signature inline image handling on import** — Parse fetched signature HTML for `<img src="cid:...">` tags and base64 data URIs. Resolve CID images from the MIME structure, decode base64 data URIs, store in the inline image store, and rewrite references to local paths. Without this, synced signatures with logos or headshots show broken images.

### Scheduled Send (Tier 2)

- [x] **Scheduled send DB schema for server delegation** — Migration v43: seven columns added (`delegation`, `remote_message_id`, `remote_status`, `timezone`, `from_email`, `error_message`, `retry_count`).

- [x] **Exchange deferred delivery via `PidTagDeferredSendTime`** — Implemented: `schedule_send`, `cancel_scheduled_send`, `reschedule_send` methods on `GraphOps`. Uses `SingleValueExtendedProperty` type with `SystemTime 0x3FEF`. Creates draft with deferred time then sends; cancellation via DELETE, reschedule via PATCH.

- [ ] **JMAP FUTURERELEASE via EmailSubmission** — Create `EmailSubmission` with `HOLDUNTIL` or `HOLDFOR` parameters in `envelope.mailFrom.parameters` per RFC 4865. Check server capability `maxDelayedSend` from `urn:ietf:params:jmap:submission` to enforce limits. Cancellation: `EmailSubmission/set` update `undoStatus` to `"canceled"`. May require patching `jmap-client` if `Address.parameters` is not exposed in the submission builder.

- [ ] **Provider-aware delegation routing** — When scheduling a send, determine delegation type based on account provider: Exchange → server delegation via deferred delivery, JMAP → check FUTURERELEASE capability → server or local, Gmail/IMAP → always local timer. Update `scheduled_emails.delegation` accordingly. The compose layer just says "send at time X"; the backend decides how.

- [ ] **Hybrid scheduler overdue handling** — On app startup, check for overdue scheduled emails. If overdue by <24h, send immediately. If overdue by >24h, mark as `needs_review` rather than auto-sending (the user may not want a week-old scheduled email going out after a vacation). Status flow: `pending → delegated → sent` for server-delegated, `pending → sending → sent` for local.

### Reactions (Tier 2)

- [x] **`message_reactions` DB table** — Migration v37: table with UNIQUE constraint on `(message_id, account_id, reactor_email, reaction_type)`, index on `(message_id, account_id)`.

- [x] **Gmail reaction MIME parsing** — Implemented: `extract_reaction_emoji()` parses MIME parts, `insert_reactions()` resolves targets via `In-Reply-To`, `is_reaction` column (migration v44) on messages, thread aggregates exclude reaction messages from counts/snippets.

- [ ] **Exchange reaction extended property reading** — Fetch `ReactionsCount` and `OwnerReactionType` via `singleValueExtendedProperties` with GUID `{41F28F13-83F4-4114-A584-EEDB5A6B0BFF}`. Store the owner's reaction and aggregate count in `message_reactions`. The full `ReactionsSummary` binary blob format is undocumented — defer parsing it until Microsoft documents it or someone reverse-engineers the format.

- [ ] **Gmail reaction sending** — Compose and send a MIME message with a `text/vnd.google.email-reaction+json` part containing `{"emoji":"...","version":1}`, proper `In-Reply-To` and `References` headers pointing to the target message, and the expected multipart structure Gmail uses. Sent via the existing Gmail API send path.

- [ ] **Reaction delta sync handling** — Exchange reactions do NOT update `lastModifiedDateTime` or `changeKey` on the message, so delta queries miss reaction changes entirely. Need periodic re-fetch of `ReactionsCount` for recently viewed messages. Gmail reactions appear as new messages in `history.list` and are detected via MIME type during normal incremental sync — no special handling needed beyond the MIME parser above.

### BIMI (Tier 3)

- [x] **`bimi_cache` DB table and filesystem cache** — Migration v38: table with `domain` PK, `has_bimi`, `logo_uri`, `authority_uri`, `fetched_at`, `expires_at`. Filesystem cache at `{cache_dir}/bimi/{sha256}.png`.

- [x] **DNS BIMI record lookup via `hickory-resolver`** — Implemented in `bimi.rs`: `lookup_bimi_dns()` queries `default._bimi.{domain}` TXT records with organizational domain fallback.

- [x] **`Authentication-Results` header parsing for DMARC verification** — Implemented in `bimi.rs`: `dmarc_passed()` checks for `dmarc=pass` in header value.

- [x] **`BIMI-Indicator` header shortcut** — Implemented in `bimi.rs`: `decode_bimi_indicator()` decodes base64 SVG from header.

- [x] **SVG fetch, validation, and rasterization pipeline** — Implemented in `bimi.rs`: `fetch_and_validate_svg()` (32KB limit, Tiny PS check, external URI rejection) + `rasterize_svg_to_png()` via `resvg` to 128×128 PNG. Full `lookup_bimi()` orchestrator function with DB caching (7-day positive, 24h negative TTL). 10 unit tests.

- [ ] **BIMI cache warming** — Background task: collect unique sender domains from visible/recent messages, batch concurrent DNS lookups (limit ~20 concurrent), fetch/rasterize/cache logos for new domains. Run on sync completion and when scrolling through message lists. In-memory LRU (N=500 domains) for active rendering to avoid filesystem reads on every message display.

### IMAP SPECIAL-USE Polish (Tier 3)

- [x] **Add `\Important` attribute detection** — Added `NameAttribute::Extension` match arm in `detect_special_use()` in `imap/parse.rs`.

- [x] **Expand heuristic folder name aliases for non-English servers** — Added DE/ES/IT/PT aliases for Drafts, Sent, and Trash in `provider/folder_roles.rs`.

- [x] **Add "Bulk Mail" to spam folder aliases** — Added to SPAM role's `imap_name_aliases` in `provider/folder_roles.rs`.
