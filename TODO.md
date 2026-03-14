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
