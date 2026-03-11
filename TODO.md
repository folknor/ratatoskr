# TODO

## Security & Data Safety

- [ ] **Decryption failure fallback returns plaintext** — `src/services/db/accounts.ts:40-81` — When decryption fails, code falls back to the raw (potentially plaintext) value with only `console.warn`. Credentials stored before encryption was enabled remain accessible in plaintext indefinitely. *(LOW)*

- [ ] **`decrypt_if_needed` silently returns ciphertext on failure** — `src-tauri/src/imap/account_config.rs:51-58` — If decryption fails, returns the encrypted blob as the IMAP password, causing a confusing auth failure. Should return `Err` instead. Same pattern as TS-side decryption fallback above. *(LOW)*

- [ ] **Draft auto-save has no crash-recovery guarantee** — `src/services/composer/draftAutoSave.ts` — 3-second debounce means up to 3s of content lost on crash. Combined with `synchronous=NORMAL`, even locally-persisted drafts might not survive power failure. *(LOW)*

---

## OAuth & Account Creation

- [ ] **Plaintext tokens round-trip through IPC** — `account_authorize_oauth_provider` returns raw `access_token`/`refresh_token` to TS, which passes them back to `account_create_imap_oauth` for encryption. The Gmail flow avoids this by handling everything in a single Rust command. Consider merging or documenting why the split is needed. *(MED)*

- [ ] **`GmailState` used as encryption key source for non-Gmail code** — `account_create_imap_oauth` and `sync/commands.rs` (`sync_imap_initial`, `sync_imap_delta`) still depend on `GmailState` solely for the encryption key. The key is app-wide. Rename to `AppCryptoState` or similar. *(LOW)*

- [ ] **Account ID generated TS-side for IMAP, Rust-side for Gmail** — Inconsistent ownership of ID generation between the two flows. *(LOW)*

---

## Provider Operations

- [ ] **Thread-level vs message-level semantics change** — All action methods (`archive`, `trash`, `markRead`, `star`, etc.) now pass `threadId` to Rust commands and ignore `_messageIds`. If any caller passes specific message IDs (e.g., marking individual messages as read), the entire thread is affected instead. *(MED)*

- [ ] **Boilerplate `ProviderCtx` construction in `commands.rs`** — Every provider command repeats the same ~15-line block (`get_provider_type` → `get_ops` → build `ProviderCtx`). Extract a helper like `with_provider_ops(account_id, states, |ops, ctx| ...)`. *(LOW)*

- [ ] **Graph folder CRUD returns "not supported"** — `create_folder`, `rename_folder`, `delete_folder` are stubbed in `src-tauri/src/graph/ops.rs`. Graph API actually supports folder CRUD via `/me/mailFolders`. *(LOW)*

- [ ] **No Graph provider class** — Graph throws in `providerFactory.ts`. `RustBackedProviderBase` is a natural fit for a `GraphProvider`. *(LOW)*

- [ ] **`gmail_attachment_id` field name in `ProviderParsedAttachment`** — Set to `att.part_id` for IMAP. Name is provider-specific but the struct is provider-agnostic. Should be renamed when the TS interface is cleaned up. *(LOW)*

- [ ] **Snippet fallback truncation not grapheme-safe** — `imap_message_to_provider_message` uses `.chars().take(200).collect()` which can split multi-byte grapheme clusters. Minor cosmetic issue. *(LOW)*

- [ ] **`ProviderFolder` struct growing wide** — Now has 10 fields. Most providers return `None` for several. Doing double duty as creation result and listing result. Could split later. *(LOW)*

---

## Sync Engine

- [ ] **Double `get_provider_type` DB query per sync in queue** — `run_sync_account` calls `get_provider_type` for status events, then `provider_sync_auto_impl` calls it again internally. *(MED)*

- [ ] **No per-account concurrency guard in queue-based sync path** — `run_sync_account` doesn't use `SyncState::try_lock_account`. The queue serializes within itself, but the still-registered `provider_sync_auto` command bypasses the queue entirely and could race. Either remove the direct command or add the per-account lock. *(MED)*

- [ ] **`sync_prepare_account_resync` doesn't clean up `bodies.db`** — Deletes threads/messages from main DB but orphans their zstd-compressed bodies in the separate body store. Add `body_store.delete()` for the account's message IDs before deleting from main tables. *(MED)*

- [ ] **`has_history` gate has fragile provider semantics** — `history_id` column means different things per provider (Google history ID, JMAP state token, Graph delta link, IMAP synthetic marker). Using `IS NOT NULL` as initial-vs-delta gate breaks if any provider sets it before completing initial sync. *(MED)*

- [ ] **IMAP `storedCount` proxy for "things changed" is lost** — `sync_initial` returns `Result<(), String>` — no data about what was stored. If anything beyond categorization depends on knowing whether initial sync stored messages, it's now blind. *(MED)*

- [ ] **Background sync captures fixed account ID list** — `sync_start_background` loops forever with the account list provided at start time. Added/removed accounts won't be picked up until restart. *(LOW)*

- [ ] **No transaction wrapping in `sync_prepare_account_resync`** — Four sequential statements (delete threads, delete messages, clear history_id, clear folder_sync_states) without explicit transaction. Partial failure leaves account in inconsistent state. *(LOW)*

- [ ] **Redundant `DELETE FROM messages` in resync** — Schema has `ON DELETE CASCADE` from threads→messages, so `DELETE FROM threads WHERE account_id = ?1` already removes all messages. The explicit messages DELETE is a no-op. *(LOW)*

- [ ] **Provider-agnostic commands in IMAP-specific module** — `sync_prepare_full_sync` and `sync_prepare_account_resync` are provider-agnostic but live in `sync/commands.rs`. Consider moving to `provider/commands.rs`. *(LOW)*

- [ ] **Two separate DB queries where one would suffice** — `provider_sync_auto` runs two sequential `with_conn` calls (one for `history_id`, one for `sync_period_days`). Could be combined. *(LOW)*

- [ ] **`sync_days` read redundantly for IMAP delta** — `provider_sync_auto` reads `sync_period_days`, then `ImapOps::sync_delta` reads it again internally. *(LOW)*

- [ ] **No "falling back to initial" progress event** — When delta sync fails and falls back to initial, the UI may show confusing progress. No event signals the fallback. *(LOW)*

- [ ] **Graph progress event payload shape is asymmetric** — `mapProviderSyncProgress` for Graph reads `messagesProcessed`/`totalFolders` while others use `phase`/`current`/`total`. Fragile if Graph sync events change shape. *(LOW)*

- [ ] **`SyncStatusEvent.status` is stringly typed in Rust** — Uses `String` for "syncing"/"done"/"error" rather than an enum. *(LOW)*

- [ ] **CalDAV accounts processed redundantly in Rust and TS** — Rust's `run_sync_account` does a DB lookup and emits events for CalDAV accounts, then TS's `handleSyncStatusEvent` re-checks `provider === "caldav"` and does the actual calendar sync. The Rust side contributes nothing for CalDAV. *(LOW)*

- [ ] **Gmail sync still fully in TS** — `src/services/gmail/syncManager.ts:80-112` — `syncGmailAccount()` uses Gmail REST API via TS HTTP calls, not the Rust sync engine. Porting is a large effort with minimal benefit since HTTP overhead dominates. *(LOW)*

- [ ] **No per-operation timeout on Rust IMAP fetches** — `src-tauri/src/sync/imap_initial.rs` — No operation-level timeout on individual FETCH commands. A folder with 50K+ messages could hang indefinitely. Fix: wrap in `tokio::time::timeout()`. *(LOW)*

- [ ] **JMAP initial sync re-queries entire result set every batch** — `src-tauri/src/jmap/sync.rs:108-146` — O(n²) server calls. Fix: use JMAP `position` + `limit` for server-side pagination, or cache IDs from first query. *(LOW)*

---

## Post-Sync Hooks

> **Systemic issue**: `load_filterable_messages` is duplicated verbatim in `filters/commands.rs`, `smart_labels/commands.rs`, and `notifications/` (3 copies). Filters, smart labels, and notifications each independently load the same message rows from DB in the same sync cycle (up to 4× redundant loads). `get_provider_type` is called independently by each hook, accumulating to 5 calls per sync cycle for the same account. These should be extracted into shared helpers and the loaded data passed between hooks.

- [ ] **Consolidate `load_filterable_messages` duplicates** — 3 identical copies across filters, smart labels, and notifications modules. Any fix to `has_attachments` or `SELECT *` issues must currently be applied in multiple places. *(MED)*

- [ ] **Redundant message loading across post-sync hooks** — Filters, smart labels (criteria), smart labels (AI prep), and notifications each call `load_filterable_messages` independently for the same message IDs in the same sync cycle. Pass loaded messages between hooks or use a shared cache. *(MED)*

- [ ] **Redundant `get_provider_type` calls per sync cycle** — Now 5 independent calls: `run_sync_account`, `provider_sync_auto_impl`, `filters_apply`, `smart_labels_apply_criteria`, and `smart_labels_apply_matches`. Pass the provider string down instead. *(MED)*

- [ ] **`has_attachments` hardcoded to `false` in `load_filterable_messages`** — Any filter with `has_attachment: true` will never match. The `messages` table has a `has_attachments` column — use it. *(MED)*

- [ ] **Filter and smart label actions applied sequentially instead of in parallel** — Old TS used `Promise.allSettled` for concurrent per-thread application. Rust iterates sequentially. Could use `tokio::task::JoinSet`. *(MED)*

- [ ] **Criteria matching re-evaluated redundantly in `prepare_ai_remainder`** — Re-runs `message_matches_filter` for all rules even though results are already in `pre_applied_matches`. *(MED)*

- [ ] **`load_filterable_messages` uses `SELECT *` instead of needed columns** — Fetches full message rows but only uses 7 fields. Wastes memory on large batches. *(LOW)*

- [ ] **Filter body hydration loads all bodies before evaluation** — When any filter has a body criterion, `body_store.get_batch` is called for all message IDs upfront. Could defer to only messages passing non-body criteria first. *(LOW)*

- [ ] **`evaluate_criteria_matches` returns non-deterministic order** — Results from `HashMap::into_iter()` have arbitrary ordering. Not a correctness issue but makes event payload unpredictable. *(LOW)*

- [ ] **`SyncStatusEvent` has grown to 13 fields** — Accumulated across post-sync hook additions. Most fields are `Option<...>` set to `None` in non-success paths. Consider a nested result type or separate event types for post-sync hook results. *(LOW)*

- [ ] **Entire `evaluate_notifications` runs inside `with_conn`** — Holds a DB connection for settings queries + VIP lookup + muted threads + message loading + category lookup + filtering. Long-running closure over synchronous SQLite. *(LOW)*

- [ ] **Correlated subquery for latest message per thread** — `AND m.date = (SELECT MAX(m2.date) ...)` runs per-thread. Fine with LIMIT 20 but inherited technical debt. *(LOW)*

- [ ] **`load_enabled_rules_for_ai` overlaps with `load_enabled_criteria_rules`** — Both query same table for same account. Could be a single query. *(LOW)*

- [ ] **`classifySmartLabelRemainder` doesn't filter pre-applied pairs from AI results** — Already-applied labels get re-applied (idempotent but wasteful). *(LOW)*

- [ ] **`smart_labels_apply_matches` only callable via IPC** — Label application after AI classification still crosses the IPC boundary. Could be called directly in Rust once AI classification moves too. *(LOW)*

- [ ] **TS re-queries all messages for AI matching phase** — `applySmartLabelsToNewMessageIds` calls `getMessagesByIds` to get messages the Rust side already loaded. *(LOW)*

- [ ] **CalDAV "done" emitted before calendar sync completes** — Old code: sync calendar → emit "done". New code: emit "done" → sync calendar. UI shows completion before calendar data arrives. *(LOW)*

---

## Calendar

- [ ] **App-specific password help links** — Providers like iCloud require app-specific passwords. Add a `help_url` field to `ProtocolOption` in `discovery/types.rs`, populate it for iCloud and similar providers, surface it in the account setup UI when present. *(LOW)*

- [ ] **`html_unescape` is incomplete** — Only handles `&lt;`, `&gt;`, `&amp;`. Missing `&quot;`, `&apos;`, and numeric character references. Calendar display names with quotes or special chars will show raw entities. *(LOW)*

- [ ] **`extract_tag_value` returns nested elements as content** — Uses first `>` to last `<` to extract text. If element contains nested elements, returns the inner markup instead of text. *(LOW)*

- [ ] **`CALDAV_NS` constant gives false sense of namespace correctness** — Used in XML body format strings but the XML parser's `extract_first_element` doesn't reference namespaces at all. *(LOW)*

- [ ] **UUID generated for every calendar upsert including conflicts** — `uuid::Uuid::new_v4()` called per row even when ON CONFLICT → UPDATE discards the `id`. Harmless but wasteful. *(LOW)*

- [ ] **`calendar_apply_sync_result` may clear existing `ctag` when only `sync_token` is provided** — Sets both `sync_token` and `ctag` unconditionally. `None` for one clears the column. *(LOW)*

- [ ] **`updated` vec always empty in `google_calendar_sync_events`** — Initialized but never populated. *(LOW)*

- [ ] **`google_calendar_request_with_body` body parameter serves double duty** — Used for both initial request and 401 retry via `body.as_ref()`. Correct but fragile if refactored. *(LOW)*

---

## AI Service

- [ ] **`reqwest::Client::new()` on every AI completion call** — Each `complete_*` function creates a fresh client. AI is called frequently during post-sync hooks, meaning repeated TLS handshakes to the same API endpoints. *(MED)*

- [ ] **`map_http_error` rate limit detection is overly broad** — `body.to_lowercase().contains("rate")` matches any response body mentioning "rate" in any context. Will misclassify unrelated errors as `RATE_LIMITED`. *(MED)*

- [ ] **`load_ai_config` makes multiple sequential DB reads** — Provider name, model, and API key each go through separate `with_conn` round-trips. Could fetch all AI-related settings in a single query. *(LOW)*

- [ ] **Duplicate `callAi` wrapper in two services** — Both `aiService.ts` and `writingStyleService.ts` define identical `callAi(systemPrompt, userContent)` wrappers. Callers could use `completeAi` directly or share a single wrapper. *(LOW)*

---

## Cache & Inline Images

- [ ] **Cache eviction not implemented** — `remove_cached` and `count_by_hash` in `attachment_cache.rs` exist but nothing calls them. The UI has a cache size setting but no code enforces it. Old cached attachments accumulate forever on disk.

- [ ] **Inline image store has no size limit** — `inline_images.db` grows unbounded. No eviction, no cap. Heavy users with lots of signature images will see this grow indefinitely.

- [ ] **Non-IMAP providers don't get inline images during sync** — IMAP stores inline images proactively at sync time. Gmail/JMAP/Graph only store them reactively on first fetch via `cache_after_fetch`. First render of every email with inline images is slow for those providers.

- [ ] **`gmail_attachment_id` column naming** — `find_cache_info` in `attachment_cache.rs` queries `gmail_attachment_id` for all providers. For IMAP, the `part_id` is stored in that column. Works, but the name is misleading.

---

## Settings

- [ ] **`read_setting_map` decrypts all settings unconditionally** — Every value goes through `decode_setting_value`/`is_encrypted`. Most settings aren't encrypted (only API keys). Wasteful when reused by the UI bootstrap snapshot which has no encrypted fields. *(LOW)*

- [ ] **API keys bundled with non-sensitive settings in one snapshot** — All 4 API keys returned alongside UI settings like `notifications_enabled`. Callers other than `SettingsPage` would receive API keys unnecessarily. *(LOW)*

---

## Branding

- [ ] **Replace logo SVG** — `src/assets/logo.svg` still renders the old "VELO" text as path outlines. Needs a new logo for Ratatoskr.

- [ ] **Replace app icons** — `src-tauri/icons/`, `assets/icon.png`, `src/assets/logo.svg`, and the inline SVG in `splashscreen.html` all contain old Velo branding. Need new Ratatoskr icons for all platforms (macOS .icns, Windows .ico, Linux .png at 32x32, 128x128, 256x256, 512x512).

---

## Code Quality

- [ ] **Category add/remove is racy** — `src-tauri/src/graph/ops.rs` — `add_category`/`remove_category` do a read-then-write. Two concurrent actions could clobber each other. Graph has no atomic "add to array" operation — unavoidable without client-side locking. *(LOW)*

- [ ] **No `$batch` optimization for Graph thread actions** — Thread-level actions loop per-message. Batching up to 20 per `/$batch` call would be faster. *(LOW)*

- [ ] **`raw_size` is always 0 for Graph messages** — Graph API has no first-class size property. `PidTagMessageSize` can't combine with `$select`. Accepted cosmetic limitation. *(LOW)*

- [ ] **Account-to-store mapping duplicated 4 times** — `App.tsx` (twice), `ComposerWindow.tsx`, and `ThreadWindow.tsx` all have identical `dbAccounts.map(...)`. Could be a shared helper. *(LOW)*

---

## Testing

- [ ] **`flushListenerSetup` uses magic 8-iteration microtick loop** — `for (let index = 0; index < 8; index += 1) { await Promise.resolve(); }` is brittle and unexplained. If `ensureSyncListeners` gains more async steps, tests will silently break. *(LOW)*

- [ ] **No test for the CalDAV provider special path** — `handleSyncStatusEvent` has a branch `if (event.provider === "caldav")` that emits "done" and runs calendar sync without post-sync hooks. Tests only cover `gmail_api`. *(LOW)*

- [ ] **No test for `null` sync token → `undefined` conversion** — `syncCalendarForAccount` passes `cal.sync_token ?? undefined` to `provider.syncEvents`. The `null` → `undefined` coercion path is untested. *(LOW)*
