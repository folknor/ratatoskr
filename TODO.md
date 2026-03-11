# TODO

## Bugs (High Priority)


- [x] **App not killed when main window is closed** — Fixed: `on_window_event` now calls `app_handle().exit(0)` instead of hiding to tray.

- [x] **Remove "launch at login" feature** — Removed UI toggle, `tauri-plugin-autostart` dep, Cargo entry, and capability permissions.

- [x] **Remove "reduce motion" setting** — Removed UI toggle, store state, App.tsx effect, Rust bootstrap field, and i18n keys.

- [x] **"Undo send" delay needs a disable option** — Added "None (send immediately)" option; Composer skips undo UI when delay is 0.

- [x] **`JMAP_NO_STATE` fallback dropped** — Added `|| err == "JMAP_NO_STATE"` to the delta-sync fallback guard in `provider_sync_auto_impl`.

- [x] **IMAP initial sync no longer triggers AI categorization** — IMAP initial sync now returns `["_initial_sync_completed"]` as `affected_thread_ids` to trigger post-sync hooks.

- [x] **`snippet` populated from `body_text` instead of thread snippet** — `prepare_ai_remainder` now queries thread snippets from `threads` table and uses them instead of `body_text`.

- [x] **`calendar_upsert_provider_events` silently uses NULL `calendar_id` when calendar not found** — Now returns an error if the calendar is not found, consistent with `calendar_apply_sync_result`.

- [x] **Duplicate `Authorization` header in `google_calendar_execute_with_retry`** — Removed the duplicate `.header("Authorization", ...)` call before `.send()`.

- [x] **All-day event timestamps use UTC instead of local timezone** — Now uses `.and_local_timezone(chrono::Local).single()` to match old TS `new Date()` behaviour.

- [x] **Hand-rolled CalDAV XML parser doesn't handle arbitrary namespace prefixes** — `split_xml_responses` and `extract_first_element` now extract namespace prefix→URI mappings from the XML document itself (`xml_ns_prefixes_for`), handling `<D:response>`, `<ns0:response>`, and any other prefixes dynamically. *(BUG)*

- [x] **`listFolders` lost `getSyncableFolders` filtering** — `list_folders` in `imap/client.rs` now skips `\Noselect` folders (including `[Gmail]`, `[Google Mail]` container folders).

- [x] **`listFolders` lost `mapFolderToLabel` ID mapping** — Added `canonical_folder_id()` in `imap/ops.rs` that maps special-use flags to well-known IDs (`SENT`, `TRASH`, etc.) and user folders to `folder-{path}`.

- [x] **Busy-wait spin loop in `run_sync_queue`** — Added `tokio::task::yield_now().await` before `continue` to yield the scheduler instead of spinning.

---

## Security & Data Safety

- [ ] **Decryption failure fallback returns plaintext** — `src/services/db/accounts.ts:40-81` — When decryption fails, code falls back to the raw (potentially plaintext) value with only `console.warn`. Credentials stored before encryption was enabled remain accessible in plaintext indefinitely. *(LOW)*

- [ ] **`decrypt_if_needed` silently returns ciphertext on failure** — `src-tauri/src/imap/account_config.rs:51-58` — If decryption fails, returns the encrypted blob as the IMAP password, causing a confusing auth failure. Should return `Err` instead. Same pattern as TS-side decryption fallback above. *(LOW)*

- [ ] **Draft auto-save has no crash-recovery guarantee** — `src/services/composer/draftAutoSave.ts` — 3-second debounce means up to 3s of content lost on crash. Combined with `synchronous=NORMAL`, even locally-persisted drafts might not survive power failure. *(LOW)*

---

## OAuth & Account Creation

- [x] **OAuth port fallback mismatch** — Fixed: `bind_oauth_listener` binds first and returns the actual port; both `perform_google_oauth` and `perform_provider_oauth` build `redirect_uri` from the actual bound port. All flows now use `127.0.0.1` consistently. *(BUG)*

- [x] **Empty email on account creation** — Fixed: `fetch_provider_userinfo` and `parse_microsoft_userinfo` now return `Err` if email is empty or missing; the redundant empty-email check in `account_create_graph_via_oauth` was removed. *(BUG)*

- [x] **No rollback if Graph client init or profile fetch fails** — Fixed: client init and profile fetch wrapped in an async block; on failure the inserted account row is deleted before returning the error. *(MED)*

- [x] **No token refresh concurrency protection for IMAP** — Fixed: added `IMAP_REFRESH_LOCKS` static (per-account `tokio::sync::Mutex`) in `imap/account_config.rs`; `ensure_oauth_access_token` acquires the lock before refreshing and re-reads from DB after acquiring (double-check pattern). *(MED)*

- [x] **No duplicate account check on Gmail creation** — Fixed: `account_create_gmail_via_oauth` queries for an existing account with the same email and provider before inserting. *(MED)*

- [x] **`oauth_token_endpoint` hardcoded to 3 providers** — Fixed: added migration 26 (`oauth_token_url` column), `CreateImapOAuthAccountRequest` now carries `oauth_token_url`, the stored URL takes precedence in `oauth_token_endpoint`, and `AddImapAccount.tsx` passes the provider's `tokenUrl` at account creation time. *(MED)*

- [ ] **Plaintext tokens round-trip through IPC** — `account_authorize_oauth_provider` returns raw `access_token`/`refresh_token` to TS, which passes them back to `account_create_imap_oauth` for encryption. The Gmail flow avoids this by handling everything in a single Rust command. Consider merging or documenting why the split is needed. *(MED)*

- [ ] **Mixed encryption key sources in Graph OAuth** — `account_create_graph_via_oauth` uses `gmail.encryption_key()` to encrypt tokens but `graph.encryption_key()` to init the client. Presumably the same key, but if they ever diverge, the client can't decrypt its own tokens. Use one consistently. *(MED)*

- [ ] **No `access_type=offline` for non-Google/non-Microsoft providers** — `perform_provider_oauth` doesn't request offline access for generic OIDC providers. Some may not return a refresh token without it. *(MED)*

- [ ] **Microsoft scopes missing `User.Read`** — `MICROSOFT_GRAPH_SCOPES` includes `Mail.*` and `MailboxSettings.*` but not `User.Read`. Some tenant admin policies may require it explicitly. *(MED)*

- [ ] **Double DB read + potential double token refresh on `send_email`** — `ops.rs` `send_email` calls `load_smtp_config` then `load_imap_config` sequentially. Each does a full DB query and potentially a token refresh. Should have a `load_both_configs` or accept a pre-loaded record. *(MED)*

- [ ] **`GoogleUserInfo.picture` required but may be absent** — `src-tauri/src/account_commands.rs` — Google accounts without a profile picture may omit the `picture` field, failing deserialization. Make it `Option<String>`. *(MED)*

- [ ] **`GmailState` used as encryption key source for non-Gmail code** — `account_create_imap_oauth`, `sync/commands.rs` (`sync_imap_initial`, `sync_imap_delta`), and `account_create_graph_via_oauth` (partially) all depend on `GmailState` solely for the encryption key. The key is app-wide. Rename to `AppCryptoState` or similar. *(LOW)*

- [ ] **Microsoft ID token parsed without signature verification** — `parse_microsoft_userinfo` base64-decodes the JWT payload without verifying the signature. Fine for display info but add a comment noting it's intentional, to prevent someone later using it for auth decisions. *(LOW)*

- [ ] **`code_verifier.filter(|_| request.use_pkce)` is redundant** — `code_verifier` is already `None` when `use_pkce` is false. The `.filter()` is dead logic. *(LOW)*

- [ ] **`picture` field available but not used for IMAP accounts** — `OAuthProviderAuthorizationResult` returns `picture` from Rust but the TS `OAuthAuthorizationResult` interface doesn't include it. Account created with `avatarUrl: null` even when a picture URL is available. *(LOW)*

- [ ] **Account ID generated TS-side for IMAP, Rust-side for Gmail** — Inconsistent ownership of ID generation between the two flows. *(LOW)*

- [ ] **`#[serde(rename_all = "camelCase")]` on `GoogleUserInfo` is misleading** — All fields are single-word so the rename is a no-op, but adding `given_name`/`family_name` later would silently break deserialization. Remove the attribute. *(LOW)*

- [ ] **`GraphAccountResult` duplicates `GmailAccountResult`** — Both structs have identical fields. Could be a single `AccountResult` type. *(LOW)*

- [ ] **`avatar_url` always empty string instead of `Option`** — `GraphAccountResult` returns `avatar_url: String::new()`. Should be `Option<String>` / `None` to match the TS `Account.avatarUrl: string | null` type. *(LOW)*

- [ ] **`setStatus` calls are vestigial in `AddGraphAccount.tsx`** — `setStatus("authenticating")` then `setStatus("testing")` fire back-to-back with no work between them. Collapse to a single status. *(LOW)*

- [ ] **`start_oauth_server` is both a Tauri command and called internally** — Two code paths can trigger OAuth. If the old TS path is no longer needed, remove it from the invoke handler. *(LOW)*

- [ ] **Dead imports in `tokenManager.ts`** — `getSecureSetting` and `getSetting` may be unused after `getClientId`/`getClientSecret` were removed. Verify and clean up. *(LOW)*

- [ ] **`provider` field inconsistently passed to `addAccount`** — `AddAccount.tsx:55` now passes `provider: account.provider`. Other account creation paths (IMAP, Graph, JMAP) should also set it. *(LOW)*

- [ ] **`reqwest::Client::new()` on every token refresh** — `account_config.rs` creates a new HTTP client per refresh. Should reuse a shared client. *(LOW)*

- [ ] **`load_smtp_config` uses `imap_password` for SMTP auth** — `account_config.rs:213` — SMTP password comes from `record.imap_password`. No `smtp_password` column exists. Pre-existing design (same credentials for both), but the new code carries this assumption forward without comment. *(LOW)*

- [ ] **`initializeClients` no longer checks token presence before init** — Old code skipped accounts without `access_token`/`refresh_token`. New code attempts `gmail_init_client` for all active gmail_api accounts, producing console errors for partially-configured accounts. *(LOW)*

- [ ] **CalDAV password decryption error now propagated instead of fallback** — Old TS silently fell back to raw value. New Rust propagates error via `?`, failing the operation. Could break CalDAV for accounts with corrupted encrypted passwords. *(LOW)*

- [ ] **Inconsistent null handling between new account commands** — `account_get_basic_info` returns `Option` for missing accounts, `account_get_caldav_connection_info` returns an error. *(LOW)*

---

## Provider Operations

- [x] **`testConnection` error handling regression** — Fixed: `provider_test_connection` now catches `Err` from `ops.test_connection()` and returns `Ok(ProviderTestResult { success: false, message })` instead of propagating as an unhandled TS rejection. *(MED)*

- [ ] **Thread-level vs message-level semantics change** — All action methods (`archive`, `trash`, `markRead`, `star`, etc.) now pass `threadId` to Rust commands and ignore `_messageIds`. If any caller passes specific message IDs (e.g., marking individual messages as read), the entire thread is affected instead. *(MED)*

- [x] **`sendMessage` lost non-fatal Sent folder copy handling** — Already handled: IMAP `send_email` uses `if let Err(e) = async { ... }.await { log::error!(...) }` — Sent-copy failure is non-fatal and logged. *(MED)*

- [x] **`thread_id` always empty for fetched IMAP messages** — Fixed: `fetch_message` queries the DB for the stored `thread_id` after fetching body; falls back to empty string if message isn't indexed yet. *(MED)*

- [x] **Verify `msg.date` unit before `* 1000`** — Confirmed: `ImapMessage.date` is in seconds (comment in `sync/convert.rs`). `* 1000` in `imap_message_to_provider_message` is correct. Not a bug. *(MED)*

- [ ] **Duplicated TS interfaces for provider results** — `ProviderFolderResult`, `ProviderTestResult`, `ProviderProfile` are defined independently in `gmailProvider.ts`, `jmapProvider.ts`, `imapSmtpProvider.ts`, and `labelStore.ts`. Move to a shared location (e.g., `services/email/types.ts`). *(LOW)*

- [ ] **`rename_folder` fallback sends empty name** — `src/stores/labelStore.ts:140` — `newName: updates.name ?? existing?.name ?? ""` sends `""` if neither is available. Should bail early or throw. *(LOW)*

- [ ] **Boilerplate `ProviderCtx` construction in `commands.rs`** — Every provider command repeats the same ~15-line block (`get_provider_type` → `get_ops` → build `ProviderCtx`). Extract a helper like `with_provider_ops(account_id, states, |ops, ctx| ...)`. *(LOW)*

- [ ] **Graph folder CRUD returns "not supported"** — `create_folder`, `rename_folder`, `delete_folder` are stubbed in `src-tauri/src/graph/ops.rs`. Graph API actually supports folder CRUD via `/me/mailFolders`. *(LOW)*

- [ ] **IMAP folder CRUD calls will always fail** — `createFolder`, `deleteFolder`, `renameFolder` invoke Rust IMAP ops that return `Err("not supported")`. Not a regression from old `throw new Error(...)`, but unnecessary IPC round-trips. *(LOW)*

- [ ] **Base class action methods throw instead of delegating to Rust** — `RustBackedProviderBase` default implementations for `archive`, `trash`, `markRead`, etc. throw "not supported". Safe only because those providers route actions through `emailActions.ts` directly. Add a comment documenting this assumption. *(LOW)*

- [ ] **Gmail and JMAP still use provider-specific `listFolders`** — `GmailApiProvider` calls `gmail_list_labels`, `JmapProvider` calls `jmap_list_folders`. Only IMAP uses the unified `provider_list_folders`. *(LOW)*

- [ ] **No Graph provider class** — Graph throws in `providerFactory.ts`. `RustBackedProviderBase` is a natural fit for a `GraphProvider`. *(LOW)*

- [ ] **`GmailApiProvider.mapFolder` override is identical to base** — Can be removed. *(LOW)*

- [ ] **`gmail_attachment_id` field name in `ProviderParsedAttachment`** — Set to `att.part_id` for IMAP. Name is provider-specific but the struct is provider-agnostic. Should be renamed when the TS interface is cleaned up. *(LOW)*

- [ ] **Snippet fallback truncation not grapheme-safe** — `imap_message_to_provider_message` uses `.chars().take(200).collect()` which can split multi-byte grapheme clusters. Minor cosmetic issue. *(LOW)*

- [ ] **`ProviderFolder` struct growing wide** — Now has 10 fields. Most providers return `None` for several. Doing double duty as creation result and listing result. Could split later. *(LOW)*

---

## Sync Engine

- [ ] **Double `get_provider_type` DB query per sync in queue** — `run_sync_account` calls `get_provider_type` for status events, then `provider_sync_auto_impl` calls it again internally. *(MED)*

- [ ] **No per-account concurrency guard in queue-based sync path** — `run_sync_account` doesn't use `SyncState::try_lock_account`. The queue serializes within itself, but the still-registered `provider_sync_auto` command bypasses the queue entirely and could race. Either remove the direct command or add the per-account lock. *(MED)*

- [ ] **`sync_start_background` errors silently swallowed** — TS calls `void invoke("sync_start_background", ...)` fire-and-forget. If the command fails, no error reaches the UI. *(MED)*

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

- [ ] **Unnecessary `#[allow(clippy::too_many_arguments)]` on `sync_run_accounts`** — Command only has 3 parameters. Likely copy-pasted. *(LOW)*

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

- [x] **`apply_filter_result` early-returns on first provider error within a thread** — If `add_tag` fails for one label, remaining labels and `mark_read`/`star` for that thread are skipped. Should collect errors and continue. *(MED)*

- [ ] **Filter and smart label actions applied sequentially instead of in parallel** — Old TS used `Promise.allSettled` for concurrent per-thread application. Rust iterates sequentially. Could use `tokio::task::JoinSet`. *(MED)*

- [x] **`muted_thread_ids` loads ALL muted threads for the account** — `SELECT id FROM threads WHERE account_id = ?1 AND is_muted = 1` could return thousands of IDs when only a handful (from new inbox messages) are relevant. Filter to relevant thread IDs with `IN (...)`. *(MED)*

- [ ] **Criteria matching re-evaluated redundantly in `prepare_ai_remainder`** — Re-runs `message_matches_filter` for all rules even though results are already in `pre_applied_matches`. *(MED)*

- [ ] **`load_filterable_messages` uses `SELECT *` instead of needed columns** — Fetches full message rows but only uses 7 fields. Wastes memory on large batches. *(LOW)*

- [ ] **Filter body hydration loads all bodies before evaluation** — When any filter has a body criterion, `body_store.get_batch` is called for all message IDs upfront. Could defer to only messages passing non-body criteria first. *(LOW)*

- [ ] **`evaluate_criteria_matches` returns non-deterministic order** — Results from `HashMap::into_iter()` have arbitrary ordering. Not a correctness issue but makes event payload unpredictable. *(LOW)*

- [ ] **`SyncStatusEvent` has grown to 13 fields** — Accumulated across post-sync hook additions. Most fields are `Option<...>` set to `None` in non-success paths. Consider a nested result type or separate event types for post-sync hook results. *(LOW)*

- [ ] **Entire `evaluate_notifications` runs inside `with_conn`** — Holds a DB connection for settings queries + VIP lookup + muted threads + message loading + category lookup + filtering. Long-running closure over synchronous SQLite. *(LOW)*

- [ ] **`from_address` normalization double-allocates** — `.to_lowercase().trim().to_string()` allocates twice. Use `email.trim().to_lowercase()` instead. *(LOW)*

- [ ] **`get_ai_categorization_candidates` runs unconditionally** — Runs settings check + thread query even when `affected_thread_ids` is empty (initial syncs). Results are discarded by TS gate. Check `result.affected_thread_ids.is_empty()` before calling. *(LOW)*

- [ ] **Duplicate SQL query for AI categorization candidates** — `get_ai_categorization_candidates` is identical to `db_get_recent_rule_categorized_thread_ids` in `queries_extra.rs`. Old command still registered. Extract to shared query or reuse. *(LOW)*

- [ ] **Correlated subquery for latest message per thread** — `AND m.date = (SELECT MAX(m2.date) ...)` runs per-thread. Fine with LIMIT 20 but inherited technical debt. *(LOW)*

- [ ] **`load_enabled_rules_for_ai` overlaps with `load_enabled_criteria_rules`** — Both query same table for same account. Could be a single query. *(LOW)*

- [ ] **`classifySmartLabelRemainder` doesn't filter pre-applied pairs from AI results** — Already-applied labels get re-applied (idempotent but wasteful). *(LOW)*

- [ ] **`smart_labels_apply_matches` only callable via IPC** — Label application after AI classification still crosses the IPC boundary. Could be called directly in Rust once AI classification moves too. *(LOW)*

- [ ] **TS re-queries all messages for AI matching phase** — `applySmartLabelsToNewMessageIds` calls `getMessagesByIds` to get messages the Rust side already loaded. *(LOW)*

- [ ] **`is_delta` field dead on the TS side** — Still emitted from Rust, no longer read by TS. Remove from both sides or document if kept for future use. *(LOW)*

- [ ] **CalDAV "done" emitted before calendar sync completes** — Old code: sync calendar → emit "done". New code: emit "done" → sync calendar. UI shows completion before calendar data arrives. *(LOW)*

- [x] **`categorization_apply_ai_results` duplicates `db_set_thread_categories_batch`** — Identical SQL and logic. Old command still registered. *(MED)*

---

## Calendar

- [ ] **`parse_ical_datetime` treats floating datetimes as UTC and ignores TZID** — Last branch handles datetimes without `Z` suffix but does `.and_utc().timestamp()`. Also, `DTSTART;TZID=America/New_York:20260311T140000` — the `TZID` parameter is completely ignored. *(MED)*

- [ ] **`unfold_ical_lines` doesn't handle LF-only continuation lines** — RFC 5545 folds with CRLF+SPACE/TAB but some real-world servers emit LF-only (`\n ` or `\n\t`). Long property values from such servers will appear split. *(MED)*

- [ ] **`caldav_request_with_headers` sets Content-Type twice** — When `body` is `Some(...)`, it unconditionally sets `Content-Type: application/xml`. Callers like `caldav_create_event` also pass `Content-Type: text/calendar` in the headers slice. Server sees duplicate Content-Type headers with different values. *(MED)*

- [ ] **New `reqwest::Client::new()` on every CalDAV command** — Each of the 7 commands creates a fresh client with no connection pooling. Old TS cached `DAVClient` in `this.client`. Means a fresh TLS handshake per CalDAV operation. Google Calendar commands reuse a client via `GmailState`; CalDAV should do similar. *(MED)*

- [ ] **`Content-Type: application/json` set unconditionally for all Google Calendar requests** — Applied to GET/DELETE (no body). Also redundant for POST/PATCH where `request.json()` sets it. *(MED)*

- [ ] **429 response returned as `Ok` after exhausting retries** — `google_calendar_execute_with_retry` returns the 429 response after max attempts. Should explicitly error. *(MED)*

- [ ] **18-column calendar event INSERT/ON CONFLICT duplicated** — Same SQL in `calendar_upsert_provider_events` and `calendar_apply_sync_result`. Extract to constant or helper. *(MED)*

- [ ] **`Rust adds `calendar_provider == "google_api"` case not in old TS** — `should_sync_calendar` returns `true` for `google_api` calendar provider, but old `hasCalendarSupport` didn't have this case. Behavioral change for accounts with `calendar_provider = "google_api"` that aren't `gmail_api`. *(MED)*

- [ ] **App-specific password help links** — Providers like iCloud require app-specific passwords. Add a `help_url` field to `ProtocolOption` in `discovery/types.rs`, populate it for iCloud and similar providers, surface it in the account setup UI when present. *(LOW)*

- [ ] **`html_unescape` is incomplete** — Only handles `&lt;`, `&gt;`, `&amp;`. Missing `&quot;`, `&apos;`, and numeric character references. Calendar display names with quotes or special chars will show raw entities. *(LOW)*

- [ ] **`extract_tag_value` returns nested elements as content** — Uses first `>` to last `<` to extract text. If element contains nested elements, returns the inner markup instead of text. *(LOW)*

- [ ] **`CALDAV_NS` constant gives false sense of namespace correctness** — Used in XML body format strings but the XML parser's `extract_first_element` doesn't reference namespaces at all. *(LOW)*

- [ ] **UUID generated for every calendar upsert including conflicts** — `uuid::Uuid::new_v4()` called per row even when ON CONFLICT → UPDATE discards the `id`. Harmless but wasteful. *(LOW)*

- [ ] **`calendar_apply_sync_result` may clear existing `ctag` when only `sync_token` is provided** — Sets both `sync_token` and `ctag` unconditionally. `None` for one clears the column. *(LOW)*

- [ ] **`let mut` with dead `let _ =` suppression in Google Calendar** — `time_min`/`time_max` don't need `mut`. Remove `mut` instead of suppressing. *(LOW)*

- [ ] **`updated` vec always empty in `google_calendar_sync_events`** — Initialized but never populated. *(LOW)*

- [ ] **`google_calendar_request_with_body` body parameter serves double duty** — Used for both initial request and 401 retry via `body.as_ref()`. Correct but fragile if refactored. *(LOW)*

---

## AI Service

- [ ] **`reqwest::Client::new()` on every AI completion call** — Each `complete_*` function creates a fresh client. AI is called frequently during post-sync hooks, meaning repeated TLS handshakes to the same API endpoints. *(MED)*

- [ ] **`map_http_error` rate limit detection is overly broad** — `body.to_lowercase().contains("rate")` matches any response body mentioning "rate" in any context. Will misclassify unrelated errors as `RATE_LIMITED`. *(MED)*

- [ ] **`load_ai_config` makes multiple sequential DB reads** — Provider name, model, and API key each go through separate `with_conn` round-trips. Could fetch all AI-related settings in a single query. *(LOW)*

- [ ] **Duplicate `callAi` wrapper in two services** — Both `aiService.ts` and `writingStyleService.ts` define identical `callAi(systemPrompt, userContent)` wrappers. Callers could use `completeAi` directly or share a single wrapper. *(LOW)*

- [ ] **`read_plain_setting` clones the key string twice** — `key_name` and `key_label` are both `.to_string()` of `key`. Only one is needed. *(LOW)*

---

## Cache & Inline Images

- [ ] **Cache eviction not implemented** — `remove_cached` and `count_by_hash` in `attachment_cache.rs` exist but nothing calls them. The UI has a cache size setting but no code enforces it. Old cached attachments accumulate forever on disk.

- [ ] **Inline image store has no size limit** — `inline_images.db` grows unbounded. No eviction, no cap. Heavy users with lots of signature images will see this grow indefinitely.

- [ ] **Non-IMAP providers don't get inline images during sync** — IMAP stores inline images proactively at sync time. Gmail/JMAP/Graph only store them reactively on first fetch via `cache_after_fetch`. First render of every email with inline images is slow for those providers.

- [ ] **`gmail_attachment_id` column naming** — `find_cache_info` in `attachment_cache.rs` queries `gmail_attachment_id` for all providers. For IMAP, the `part_id` is stored in that column. Works, but the name is misleading.

- [ ] **`CacheInfo.local_path` fetched but never read** — `try_cache_hit` in `provider/commands.rs` uses `content_hash` → `read_cached` (which resolves the path itself), so the stored `local_path` in `CacheInfo` is redundant.

---

## Settings

- [ ] **`read_setting_map` decrypts all settings unconditionally** — Every value goes through `decode_setting_value`/`is_encrypted`. Most settings aren't encrypted (only API keys). Wasteful when reused by the UI bootstrap snapshot which has no encrypted fields. *(LOW)*

- [ ] **API keys bundled with non-sensitive settings in one snapshot** — All 4 API keys returned alongside UI settings like `notifications_enabled`. Callers other than `SettingsPage` would receive API keys unnecessarily. *(LOW)*

- [ ] **Two boolean-parsing conventions without explanation** — Some fields use `.is_some_and(|v| v == "true")` (opt-in: missing = `false`), others use `get_bool(key, true)` with `value != "false"` (opt-out: missing = `true`). Both match old TS semantics but look inconsistent. A comment distinguishing opt-in vs opt-out defaults would help. *(LOW)*

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
