# TODO

## Bugs

### HIGH

- [ ] **Auto-updater should check local permissions** — Don't show update prompts if the user lacks write access to the app installation directory (e.g., installed system-wide without admin rights). The update would fail anyway — detect this upfront and either hide the prompt or show a helpful message.

- [ ] **App not killed when main window is closed** — Closing the main window doesn't terminate the process. Investigate: likely minimize-to-tray or `on_window_event` handler preventing exit. May be related to single-instance plugin or background sync tasks keeping the runtime alive.

- [ ] **Remove "launch at login" feature** — Remove the UI option, the Rust `auto-launch` crate dependency, and any related Tauri plugin/capability config. Not needed.

- [ ] **Remove "reduce motion" setting** — Remove the UI option and any associated CSS/animation logic. Not needed.

- [ ] **"Undo send" delay needs a disable option** — Currently forced on with no way to turn it off. Add a "None" / 0s option to the delay picker.

---

## Security & Data Safety

### LOW

- [ ] **Decryption failure fallback returns plaintext** — `src/services/db/accounts.ts:40-81`

  When decryption fails, code falls back to the raw (potentially plaintext) value with only `console.warn`. Credentials stored before encryption was enabled remain accessible in plaintext indefinitely.

- [ ] **Draft auto-save has no crash-recovery guarantee** — `src/services/composer/draftAutoSave.ts`

  3-second debounce means up to 3s of content lost on crash. Combined with `synchronous=NORMAL`, even locally-persisted drafts in `local_drafts` might not survive power failure.

---

## Cache & Inline Image Store

- [ ] **Cache eviction not implemented** — `remove_cached` and `count_by_hash` in `attachment_cache.rs` exist but nothing calls them. The UI has a cache size setting but no code enforces it. Old cached attachments accumulate forever on disk.

- [ ] **Inline image store has no size limit** — `inline_images.db` grows unbounded. No eviction, no cap. Heavy users with lots of signature images will see this grow indefinitely.

- [ ] **Non-IMAP providers don't get inline images during sync** — IMAP stores inline images proactively at sync time. Gmail/JMAP/Graph only store them reactively on first fetch via `cache_after_fetch` in `provider/commands.rs`. First render of every email with inline images is slow for those providers.

- [ ] **`gmail_attachment_id` column naming** — `find_cache_info` in `attachment_cache.rs` queries `gmail_attachment_id` for all providers. For IMAP, the `part_id` is stored in that column. Works, but the name is misleading.

- [ ] **`CacheInfo.local_path` fetched but never read** — `try_cache_hit` in `provider/commands.rs` uses `content_hash` → `read_cached` (which resolves the path itself), so the stored `local_path` in `CacheInfo` is redundant.

---

## Phase 4 (Rust Sync Engine) Follow-ups

### LOW

- [ ] **Gmail sync still fully in TS** — `src/services/gmail/syncManager.ts:80-112`

  `syncGmailAccount()` uses the Gmail REST API via TS HTTP calls, not the Rust sync engine. Porting is a large effort with minimal benefit since HTTP overhead dominates.

- [ ] **No per-operation timeout on Rust IMAP fetches** — `src-tauri/src/sync/imap_initial.rs`

  No operation-level timeout on individual FETCH commands. A folder with 50K+ messages could hang indefinitely. Fix: wrap in `tokio::time::timeout()`. Low priority — rare edge case.

- [ ] **JMAP initial sync re-queries entire result set every batch** — `src-tauri/src/jmap/sync.rs:108-146`

  O(n²) server calls. Fix: use JMAP `position` + `limit` for server-side pagination, or cache IDs from first query.

---

## Branding / Assets

- [ ] **Replace logo SVG** — `src/assets/logo.svg` still renders the old "VELO" text as path outlines. Needs a new logo for Ratatoskr.

- [ ] **Replace app icons** — `src-tauri/icons/`, `assets/icon.png`, `src/assets/logo.svg`, and the inline SVG in `splashscreen.html` all contain old Velo branding. Need new Ratatoskr icons for all platforms (macOS .icns, Windows .ico, Linux .png at 32x32, 128x128, 256x256, 512x512) plus the root asset and splash screen.

---

## Autodiscovery Follow-ups

- [ ] **App-specific password help links** — Providers like iCloud require app-specific passwords. Add a `help_url` field to `ProtocolOption` in `discovery/types.rs`, populate it for iCloud (`https://support.apple.com/en-us/102654`) and similar providers in the registry, surface it in the TS `WellKnownProviderResult`, and show a hint/link in the account setup UI when present.

---

## Phase 3b (Graph Provider) Known Issues

- [ ] **Category add/remove is racy** — `src-tauri/src/graph/ops.rs`

  `add_category`/`remove_category` do a read-then-write. Two concurrent actions could clobber each other. Graph has no atomic "add to array" operation — unavoidable.

- [ ] **No `$batch` optimization for thread actions** — `src-tauri/src/graph/ops.rs`

  Thread-level actions loop per-message. Batching up to 20 per `/$batch` call would be faster.

- [ ] **`raw_size` is always 0 for Graph messages** — `src-tauri/src/graph/sync.rs`

  Graph API has no first-class size property. `PidTagMessageSize` can't combine with `$select`. Accepted cosmetic limitation.

---

## Unified Provider Commands (`6ed3a59`)

- [ ] **Duplicated TS interfaces for provider results** — `ProviderFolderResult`, `ProviderTestResult`, `ProviderProfile` are defined independently in `gmailProvider.ts`, `jmapProvider.ts`, `imapSmtpProvider.ts`, and `labelStore.ts`. Move to a shared location (e.g., `services/email/types.ts`).

- [ ] **`rename_folder` fallback sends empty name** — `src/stores/labelStore.ts:140` — `newName: updates.name ?? existing?.name ?? ""` sends `""` if neither is available, which would rename a label to an empty string. Should bail early or throw.

- [ ] **`testConnection` error handling regression** — Gmail/JMAP/Graph `test_connection` impls propagate `Err(...)` instead of returning `ProviderTestResult { success: false }`. The old TS `gmailProvider.ts` had a try/catch converting errors to `{ success: false }` — that was removed but not replaced. Errors now surface as unhandled rejections.

- [ ] **Boilerplate `ProviderCtx` construction in `commands.rs`** — Every provider command repeats the same ~15-line block (`get_provider_type` → `get_ops` → build `ProviderCtx`). Extract a helper like `with_provider_ops(account_id, states, |ops, ctx| ...)`.

- [ ] **Graph folder CRUD returns "not supported"** — `src-tauri/src/graph/ops.rs` — `create_folder`, `rename_folder`, `delete_folder` are stubbed. Graph API actually supports folder CRUD via `/me/mailFolders`. Track for future implementation.

---

## Gmail OAuth Rust Migration (`95cbb86`)

### BUG

- [ ] **OAuth port fallback mismatch** — `src-tauri/src/account_commands.rs` hardcodes `redirect_uri` as `http://127.0.0.1:17248`, but `start_oauth_server` falls back to ports 17249–17251 if 17248 is taken. Token exchange will fail with `redirect_uri_mismatch` when a fallback port is used. The server should return the actual bound port.

### Medium

- [ ] **No duplicate account check on Gmail creation** — `account_create_gmail_via_oauth` doesn't check if an account with the same email already exists before inserting. Users can create duplicate entries.

- [ ] **`GoogleUserInfo.picture` required but may be absent** — `src-tauri/src/account_commands.rs` — Google accounts without a profile picture may omit the `picture` field, failing deserialization. Make it `Option<String>`.

- [ ] **`#[serde(rename_all = "camelCase")]` on `GoogleUserInfo` is misleading** — All fields are single-word so the rename is a no-op, but adding `given_name`/`family_name` later would silently break deserialization. Remove the attribute or change to `snake_case`.

### Low

- [ ] **`start_oauth_server` is both a Tauri command and called internally** — Two code paths can trigger OAuth (old TS path and new Rust path). If the old TS path is no longer needed, remove it from the invoke handler.

- [ ] **Dead imports in `tokenManager.ts`** — `getSecureSetting` and `getSetting` are still imported but may be unused after `getClientId`/`getClientSecret` were removed. Verify and clean up.

- [ ] **`provider` field now passed to `addAccount` but wasn't before** — `AddAccount.tsx:55` now passes `provider: account.provider`. Other account creation paths (IMAP, Graph, JMAP) should also set it for consistency.

---

## IMAP OAuth Rust Migration (`d6fe740`)

### BUG

- [ ] **Same OAuth port fallback mismatch as Gmail** — `perform_provider_oauth` hardcodes `http://localhost:{OAUTH_CALLBACK_PORT}` as `redirect_uri`. Same port fallback bug. Additionally uses `localhost` while the Gmail function uses `127.0.0.1` — some providers treat these differently. Should be consistent.

- [ ] **Empty email on account creation** — `fetch_provider_userinfo` and `parse_microsoft_userinfo` use `.unwrap_or_default()` for email. If the provider doesn't return an email, the account is created with `email: ""`. Should be an error instead.

### Medium

- [ ] **Plaintext tokens round-trip through IPC** — `account_authorize_oauth_provider` returns raw `access_token`/`refresh_token` to TS, which passes them back to `account_create_imap_oauth` for encryption. The Gmail flow avoids this by handling everything in a single command. Consider merging or documenting why the split is needed.

- [ ] **No `access_type=offline` for non-Google/non-Microsoft providers** — `perform_provider_oauth` doesn't request offline access for generic OIDC providers. Some may not return a refresh token without it.

- [ ] **Microsoft ID token parsed without signature verification** — `parse_microsoft_userinfo` base64-decodes the JWT payload without verifying the signature. Fine for display info but should have a comment noting it's intentional. Risk if someone later uses it for auth decisions.

### Low

- [ ] **`code_verifier.filter(|_| request.use_pkce)` is redundant** — `code_verifier` is already `None` when `use_pkce` is false. The `.filter()` is dead logic.

- [ ] **`picture` field available but not used for IMAP accounts** — `OAuthProviderAuthorizationResult` returns `picture` from Rust but the TS `OAuthAuthorizationResult` interface doesn't include it. The account is created with `avatarUrl: null` even when a picture URL is available.

- [ ] **`account_create_imap_oauth` uses `GmailState` for encryption key** — The encryption key is app-wide but lives in `GmailState`. Increasingly misleading as non-Gmail code depends on it. Future rename candidate (e.g., `AppCryptoState`).

- [ ] **Account ID generated TS-side for IMAP, Rust-side for Gmail** — Inconsistent ownership of ID generation between the two flows.

---

## Graph OAuth Rust Migration (`e45cb4c`)

### Medium

- [ ] **No rollback if Graph client init or profile fetch fails** — `account_create_graph_via_oauth` inserts the account row first, then calls `GraphClient::from_account` and `get_json("/me")`. If either fails, the account row remains orphaned in the DB with no client in memory. The old TS code had explicit cleanup. Should delete the account on post-insert failure.

- [ ] **Mixed encryption key sources** — `account_create_graph_via_oauth` uses `gmail.encryption_key()` to encrypt tokens but `graph.encryption_key()` to init the client. Presumably the same key, but if they ever diverge, the client can't decrypt its own tokens. Use one consistently.

- [ ] **Microsoft scopes missing `User.Read`** — `MICROSOFT_GRAPH_SCOPES` includes `Mail.*` and `MailboxSettings.*` but not `User.Read`. The `/me` endpoint works via `openid`+`profile`, but some tenant admin policies may require `User.Read` explicitly.

### Low

- [ ] **`setStatus` calls are vestigial in `AddGraphAccount.tsx`** — `setStatus("authenticating")` then `setStatus("testing")` fire back-to-back with no work between them. User never sees "authenticating". Collapse to a single status before the invoke.

- [ ] **Same OAuth port fallback mismatch as Gmail/IMAP** — `account_create_graph_via_oauth` uses `perform_provider_oauth` which has the same hardcoded port + `localhost` vs `127.0.0.1` inconsistency. See `95cbb86` and `d6fe740` entries.

- [ ] **`avatar_url` always empty string instead of `Option`** — `GraphAccountResult` returns `avatar_url: String::new()`. TS converts to `null` via `|| null`. Should be `Option<String>` / `None` to match the TS `Account.avatarUrl: string | null` type.

- [ ] **`GraphAccountResult` duplicates `GmailAccountResult`** — Both structs have identical fields. Could be a single `AccountResult` type.

---

## IMAP OAuth Refresh (`829bf5d`)

### Medium

- [ ] **No token refresh concurrency protection** — `ensure_oauth_access_token` has no mutex or coalescing. Concurrent IMAP operations (e.g., sync + user action) may both detect an expired token and issue parallel refresh requests. The second refresh may invalidate the first's new token. The Gmail client has a `refresh_lock` Mutex for this — IMAP should have one too.

- [ ] **`oauth_token_endpoint` hardcoded to 3 providers** — `src-tauri/src/imap/account_config.rs:61-67` only supports `microsoft`, `microsoft_graph`, and `yahoo`. Custom OAuth IMAP providers (set up via `AddImapAccount`) will fail with "Unsupported OAuth provider". The token URL should be stored in the account record, or the match extended.

- [ ] **Double DB read + potential double token refresh on `send_email`** — `ops.rs` `send_email` calls `load_smtp_config` then `load_imap_config` sequentially. Each does a full DB query and potentially a token refresh. Should have a `load_both_configs` or accept a pre-loaded record.

### Low

- [ ] **`decrypt_if_needed` silently returns ciphertext on failure** — `src-tauri/src/imap/account_config.rs:51-58` — If decryption fails, returns the encrypted blob as the IMAP password, causing a confusing auth failure. Should return `Err` instead. (Same pattern as existing TODO item for TS-side decryption fallback.)

- [ ] **`reqwest::Client::new()` on every token refresh** — `account_config.rs:139` creates a new HTTP client per refresh. Should reuse a shared client.

- [ ] **`load_smtp_config` uses `imap_password` for SMTP auth** — `account_config.rs:213` — SMTP password comes from `record.imap_password`. No `smtp_password` column exists. Pre-existing design (same credentials for both), but the new code carries this assumption forward without comment.

- [ ] **`sync/commands.rs` takes `GmailState` for IMAP sync** — `sync_imap_initial` and `sync_imap_delta` now accept `gmail: State<'_, GmailState>` solely for the encryption key. Continues the misleading naming pattern.

---

## Thin IMAP Provider (`f5e6b3a`)

### BUG (verify)

- [ ] **`provider_add_tag`/`provider_remove_tag` may not exist** — `imapSmtpProvider.ts` calls `provider_add_tag` and `provider_remove_tag`, but previous commits only registered `provider_archive`, `provider_trash`, `provider_star`, etc. If these commands aren't in `lib.rs`, the calls fail at runtime. Verify registration.

### Medium

- [ ] **Thread-level vs message-level semantics change** — All action methods (`archive`, `trash`, `markRead`, `star`, etc.) now pass `threadId` to Rust commands and ignore `_messageIds`. The old code operated on specific messages per-folder. If any caller passes specific message IDs (e.g., marking individual messages as read), the behavior is now different — the entire thread is affected.

- [ ] **`sendMessage` lost non-fatal Sent folder copy handling** — Old code caught Sent folder append failures as non-fatal (message was sent, just not copied to Sent). The Rust `provider_send_email` command may propagate that error, making the whole send appear to fail.

- [ ] **`getLegacyImapConfig` duplicates Rust-side OAuth refresh** — Still calls `ensureFreshToken` TS-side for the 3 remaining legacy methods (`listFolders`, `fetchMessage`, `fetchRawMessage`). This races with Rust-side refresh in `account_config.rs`.

### Low

- [ ] **IMAP folder CRUD calls will always fail** — `createFolder`, `deleteFolder`, `renameFolder` now invoke `provider_create_folder` etc., but the Rust IMAP ops return `Err("not supported")`. Functionally same as the old `throw new Error(...)` but with extra IPC. Not a regression, just unnecessary round-trips.

---

## Finish IMAP Provider Migration (`740cb33`)

### BUG

- [ ] **`listFolders` lost `getSyncableFolders` filtering** — The old TS code filtered out `[Gmail]`, `[Google Mail]`, and `[Nostromo]` virtual container folders. The Rust `ImapOps::list_folders` returns all folders unfiltered. Users with IMAP-accessed Gmail accounts will now see the `[Gmail]` container folder. Move filtering to Rust.

- [ ] **`listFolders` lost `mapFolderToLabel` ID mapping** — The old TS code mapped IMAP folders to canonical label IDs (`\Sent` → `SENT`, `\Trash` → `TRASH`, user folders → `folder-{path}`). Rust uses raw paths as IDs. Any downstream code matching on canonical IDs (sidebar highlight, label store, system label filtering) will break.

### Medium

- [ ] **`thread_id` always empty for fetched messages** — `imap_message_to_provider_message` sets `thread_id: String::new()`. If anything downstream relies on `threadId` for thread view linking, this will break.

- [ ] **Verify `msg.date` unit before `* 1000`** — `imap_message_to_provider_message` does `date: msg.date * 1000` assuming seconds-to-millis. If `msg.date` is already in milliseconds, dates will be wrong. Verify the IMAP `msg.date` unit.

### Low

- [ ] **`gmail_attachment_id` field name in `ProviderParsedAttachment`** — Set to `att.part_id` for IMAP. Name is provider-specific but the struct is provider-agnostic. Matches TS `ParsedAttachment.gmailAttachmentId` for compatibility, but should be renamed when the TS interface is cleaned up.

- [ ] **Snippet fallback truncation not grapheme-safe** — `imap_message_to_provider_message` uses `.chars().take(200).collect()` which can split multi-byte grapheme clusters (combining characters, emoji with ZWJ). Minor cosmetic issue.

- [ ] **`ProviderFolder` struct growing wide** — Now has 10 fields (`id`, `name`, `path`, `folder_type`, `special_use`, `delimiter`, `message_count`, `unread_count`, `color_bg`, `color_fg`). Most providers return `None` for several. Doing double duty as creation result (counts meaningless) and listing result. Could split later.

- [ ] **Resolved: `getLegacyImapConfig` dual OAuth refresh** — The TS-side `ensureFreshToken` and `getLegacyImapConfig` are fully removed in this commit, resolving the race condition noted in `f5e6b3a`.

---

## Simplify Rust-Backed Email Providers (`5891cf6`)

### Low

- [ ] **Base class action methods throw instead of delegating to Rust** — `RustBackedProviderBase` default implementations for `archive`, `trash`, `markRead`, etc. throw "not supported". Gmail/JMAP inherit these throws. Safe only because those providers route actions through `emailActions.ts` / Rust commands directly, not through the provider class. Add a comment documenting this assumption.

- [ ] **Gmail and JMAP still use provider-specific `listFolders`** — `GmailApiProvider` calls `gmail_list_labels`, `JmapProvider` calls `jmap_list_folders`. Only IMAP uses the unified `provider_list_folders`. The unified path exists but 2 of 3 providers bypass it.

- [ ] **No Graph provider class** — Graph throws in `providerFactory.ts`. `RustBackedProviderBase` is a natural fit for a `GraphProvider`. Missed opportunity, not a bug.

- [ ] **`GmailApiProvider.mapFolder` override is identical to base** — Can be removed. Only difference is `specialUse` missing `?? null`, which is functionally irrelevant.

---

## Move Sync Selection and Fallback into Rust (`733fcc2`)

### BUG

- [ ] **`JMAP_NO_STATE` fallback dropped** — Old TS code handled both `JMAP_STATE_EXPIRED` and `JMAP_NO_STATE` as fallback triggers. Rust `should_fallback_to_initial` only checks `JMAP_STATE_EXPIRED`. The JMAP sync engine returns `Err("JMAP_NO_STATE")` when there's no stored email state (`jmap/sync.rs:189`). JMAP accounts that lose their state token will fail to sync instead of gracefully recovering. Fix: add `JMAP_NO_STATE` to the fallback marker, or handle it as a separate case.

- [ ] **IMAP initial sync no longer triggers AI categorization** — Old code passed `affectedThreadIds = storedCount > 0 ? ["_imap_stored"] : []` after IMAP initial sync, gating `categorizeNewThreads`. New code returns `affected_thread_ids: Vec::new()` for all initial syncs. IMAP initial sync will no longer trigger categorization. Gmail wasn't affected (never categorized after initial), but this is a regression for IMAP.

### Medium

- [ ] **IMAP `storedCount` proxy for "things changed" is lost** — `sync_initial` returns `Result<(), String>` — no data about what was stored. The old `ImapSyncResult.storedCount` was used as a proxy for "messages were synced". If anything beyond categorization depends on knowing whether initial sync stored messages, it's now blind.

- [ ] **`has_history` gate has fragile provider semantics** — `history_id` column means different things per provider (Google history ID, JMAP state token, Graph delta link, IMAP synthetic marker). Using `IS NOT NULL` as initial-vs-delta gate works today but breaks if any provider sets it before completing initial sync (e.g., partial sync failure).

### Low

- [ ] **Two separate DB queries where one would suffice** — `provider_sync_auto` runs two sequential `with_conn` calls (one for `history_id`, one for `sync_period_days`). Could be combined into one query.

- [ ] **`sync_days` read redundantly for IMAP delta** — `provider_sync_auto` reads `sync_period_days`, then `ImapOps::sync_delta` reads it again internally. Unused in the delta path of `provider_sync_auto`.

- [ ] **No "falling back to initial" progress event** — When delta sync fails and falls back to initial, the UI may show confusing progress (delta progress → sudden restart). No event signals the fallback to the UI.

- [ ] **Graph progress event payload shape is asymmetric** — `mapProviderSyncProgress` for Graph reads `messagesProcessed`/`totalFolders` while others use `phase`/`current`/`total`. Fragile if Graph sync events change shape.

## Move Sync Reset Preparation into Rust (`6aa98a0`)

- [ ] **`sync_prepare_account_resync` doesn't clean up `bodies.db`** — Deletes threads/messages from main DB but orphans their zstd-compressed bodies in the separate body store. Pre-existing issue now consolidated in one place — add `body_store.delete()` for the account's message IDs before deleting from main tables. *(Medium)*

- [ ] **No transaction wrapping in `sync_prepare_account_resync`** — Four sequential statements (delete threads, delete messages, clear history_id, clear folder_sync_states) without explicit transaction. Partial failure leaves account in inconsistent state. *(Low)*

- [ ] **Redundant `DELETE FROM messages`** — Schema has `ON DELETE CASCADE` from threads→messages, so `DELETE FROM threads WHERE account_id = ?1` already removes all messages. The explicit messages DELETE is a no-op. *(Low)*

- [ ] **Provider-agnostic commands in IMAP-specific module** — `sync_prepare_full_sync` and `sync_prepare_account_resync` are provider-agnostic but live in `sync/commands.rs` alongside IMAP-specific sync commands. Consider moving to `provider/commands.rs`. *(Low)*

## Move Sync Queue and Timer into Rust (`5325f01`)

- [ ] **Busy-wait spin loop in `run_sync_queue`** — When `take_pending_batch()` returns empty but `finish_if_idle()` returns `None` (items enqueued between the two calls), the loop spins with no yield point — two mutex acquisitions per iteration burning CPU. Add `tokio::task::yield_now()` or use `tokio::sync::Notify` instead. *(BUG)*

- [ ] **Double `get_provider_type` DB query per sync** — `run_sync_account` calls `get_provider_type` for status events, then `provider_sync_auto_impl` calls it again internally. Two identical DB round-trips per account per sync cycle. *(Medium)*

- [ ] **No per-account concurrency guard in queue-based sync path** — `run_sync_account` doesn't use `SyncState::try_lock_account`. The queue serializes within itself, but the still-registered `provider_sync_auto` command bypasses the queue entirely and could race. Either remove the direct command or add the per-account lock. *(Medium)*

- [ ] **`sync_start_background` errors silently swallowed** — TS calls `void invoke("sync_start_background", ...)` fire-and-forget. If the command fails, no error reaches the UI. *(Medium)*

- [ ] **Background sync captures fixed account ID list** — `sync_start_background` loops forever with the account list provided at start time. Added/removed accounts won't be picked up until restart. Same as old TS behavior but now less visible in Rust. *(Low)*

- [ ] **Unnecessary `#[allow(clippy::too_many_arguments)]` on `sync_run_accounts`** — Command only has 3 parameters. Likely copy-pasted. *(Low)*

- [ ] **`SyncStatusEvent.status` is stringly typed in Rust** — Uses `String` for "syncing"/"done"/"error" rather than an enum. TS types it as a union but Rust has no compile-time enforcement. *(Low)*

- [ ] **CalDAV accounts processed redundantly in Rust and TS** — Rust's `run_sync_account` does a DB lookup and emits events for CalDAV accounts, then TS's `handleSyncStatusEvent` re-checks `provider === "caldav"` and does the actual calendar sync. The Rust side contributes nothing for CalDAV. *(Low)*

## Move Post-Sync Filters into Rust (`90ae3ce`)

- [ ] **`has_attachments` hardcoded to `false` in `load_filterable_messages`** — Any filter with `has_attachment: true` will never match. Pre-existing issue from TS, but the messages table does have a `has_attachments` column — now is the time to use it. *(Medium)*

- [ ] **Filter actions applied sequentially per thread instead of in parallel** — Old TS used `Promise.allSettled` for concurrent per-thread application. Rust iterates sequentially. Could use `tokio::task::JoinSet` or `futures::join_all` for parallelism. *(Medium)*

- [ ] **`apply_filter_result` early-returns on first provider error within a thread** — If `add_tag` fails for one label, remaining labels and `mark_read`/`star` for that thread are skipped. Should collect errors and continue applying remaining actions. *(Medium)*

- [ ] **Third redundant `get_provider_type` call per sync cycle** — `filters_apply_to_new_message_ids_impl` calls `get_provider_type` again (already called in `run_sync_account` and `provider_sync_auto_impl`). Pass the provider string down instead. *(Low)*

- [ ] **`load_filterable_messages` uses `SELECT *` instead of needed columns** — Fetches full message rows via `row_to_message` but only uses 7 fields. Wastes memory on large batches. *(Low)*

- [ ] **Filter body hydration loads all bodies before evaluation** — When any filter has a body criterion, `body_store.get_batch` is called for all message IDs upfront. Could defer to only messages passing non-body criteria first. Matches old TS behavior. *(Low)*

## Move Smart Label Criteria Matching into Rust (`0283687`)

- [ ] **`load_filterable_messages` duplicated verbatim in two modules** — Identical function in `filters/commands.rs` and `smart_labels/commands.rs`. Extract to a shared helper. Any fix to `has_attachments` or `SELECT *` issues must currently be applied in two places. *(Medium)*

- [ ] **Filters and smart labels both query the same message rows independently** — `run_sync_account` calls `filters_apply` then `smart_labels_apply`, each loading the same messages from DB (+ optional body hydration). Two redundant round-trips per sync. *(Medium)*

- [ ] **Fourth redundant `get_provider_type` call per sync cycle** — `smart_labels_apply_criteria_to_new_message_ids_impl` calls `get_provider_type` independently. Now 4 calls per sync cycle for the same account. Pass the provider string down. *(Medium)*

- [ ] **`evaluate_criteria_matches` returns non-deterministic order** — Results from `HashMap::into_iter()` have arbitrary ordering. Not a correctness issue but makes event payload unpredictable. *(Low)*

- [ ] **`SyncStatusEvent` growing into a kitchen sink** — Now 8 fields, most set to `None` in non-success paths. Consider a nested result type or separate event for post-sync hook results. *(Low)*

- [ ] **TS re-queries all messages for AI matching phase** — `applySmartLabelsToNewMessageIds` calls `getMessagesByIds` to get messages the Rust side already loaded. Could be avoided if Rust passed filterable data through the event or a shared cache. *(Low)*

## Move Notification Eligibility into Rust (`98d71c6`)

- [ ] **Third copy of chunked message loading boilerplate** — `evaluate_notifications` has another copy of `SELECT * FROM messages ... IN (...)` with `row_to_message`. Now 3 identical copies (filters, smart_labels, notifications) all loading the same messages in the same sync cycle. Extract shared helper and pass loaded messages between post-sync hooks. *(Medium)*

- [ ] **`muted_thread_ids` loads ALL muted threads for the account** — `SELECT id FROM threads WHERE account_id = ?1 AND is_muted = 1` could return thousands of IDs when only a handful (from new inbox messages) are relevant. Filter to relevant thread IDs with `IN (...)`. *(Medium)*

- [ ] **Entire `evaluate_notifications` runs inside `with_conn`** — Holds a DB connection for settings queries + VIP lookup + muted threads + message loading + category lookup + filtering. All synchronous SQLite so it's correct, but it's a long-running closure. *(Low)*

- [ ] **`SyncStatusEvent` now has 9 fields** — Continuing growth pattern. `notifications_to_queue` is another `Option<Vec<...>>`. *(Low)*

- [ ] **`from_address` normalization double-allocates** — `.to_lowercase().trim().to_string()` allocates twice. Use `email.trim().to_lowercase()` instead. *(Low)*

## Move AI Categorization Candidate Selection into Rust (`fa75f7b`)

- [ ] **`get_ai_categorization_candidates` runs unconditionally** — Runs the settings check + thread query even when `affected_thread_ids` is empty (initial syncs). Results are discarded by TS gate. Check `result.affected_thread_ids.is_empty()` before calling. *(Low)*

- [ ] **Duplicate SQL query in two places** — `get_ai_categorization_candidates` is identical to `db_get_recent_rule_categorized_thread_ids` in `queries_extra.rs`. Old command still registered. Extract to shared query or reuse. *(Low)*

- [ ] **`SyncStatusEvent` now has 10 fields** — `ai_categorization_candidates` continues the growth pattern. Consider restructuring. *(Low)*

- [ ] **Correlated subquery for latest message per thread** — `AND m.date = (SELECT MAX(m2.date) ...)` runs per-thread. Fine with LIMIT 20 but inherited technical debt. *(Low)*

## Move Smart Label AI Preparation into Rust (`f628539`)

- [ ] **`snippet` populated from `body_text` instead of thread snippet** — `prepare_ai_remainder` uses `message.body_text` (full plain-text body) instead of the thread's `snippet` field (~100 char preview). Changes AI input significantly — more tokens, different content. Should query thread snippet from `threads` table. *(BUG)*

- [ ] **Messages loaded a 4th time in the same sync cycle** — `smart_labels_prepare_ai_remainder_impl` calls `load_filterable_messages` for message IDs already loaded by criteria matching, filters, and notifications. Four independent loads of identical data. *(Medium)*

- [ ] **Criteria matching re-evaluated redundantly in `prepare_ai_remainder`** — Re-runs `message_matches_filter` for all rules even though results are already in `pre_applied_matches`. Wasted CPU. *(Medium)*

- [ ] **`load_enabled_rules_for_ai` overlaps with `load_enabled_criteria_rules`** — Both query same table for same account. Could be a single query returning both `ai_description` and `criteria_json`. *(Low)*

- [ ] **`SyncStatusEvent` now has 12 fields** — Two more added. `ai_smart_label_threads` and `ai_smart_label_rules` could be a single nested struct. *(Low)*

- [ ] **`classifySmartLabelRemainder` doesn't filter pre-applied pairs from AI results** — Unlike `matchSmartLabels`, the new function passes AI results through directly. Already-applied labels get re-applied (idempotent but wasteful). *(Low)*

## Move Smart Label Application into Rust (`11463cf`)

- [ ] **Sequential label application replaces parallel** — Old TS `Promise.allSettled` applied all thread+label pairs concurrently. Rust loops sequentially. Same pattern as filter application in `90ae3ce`. *(Low)*

- [ ] **5th redundant `get_provider_type` + `get_ops` call per sync cycle** — `smart_labels_apply_matches_impl` constructs provider ops independently. Continues the accumulating pattern. *(Low)*

- [ ] **`smart_labels_apply_matches` only callable via IPC** — The `_impl` function isn't called from Rust. Label application after AI classification still crosses the IPC boundary. Could be called directly in Rust once AI classification moves too. *(Low)*

## Move Calendar Follow-Up Decision into Rust (`4aaae5f`)

- [ ] **Rust adds `calendar_provider == "google_api"` case not in old TS** — `should_sync_calendar` returns `true` for `google_api` calendar provider, but old `hasCalendarSupport` didn't have this case. Behavioral change for accounts with `calendar_provider = "google_api"` that aren't `gmail_api`. *(Medium)*

- [ ] **`is_delta` field now dead on the TS side** — Still emitted from Rust, no longer read by TS. Remove from both sides or document if kept for future use. *(Low)*

- [ ] **CalDAV "done" emitted before calendar sync completes** — Old code: sync calendar → emit "done". New code: emit "done" → sync calendar. UI shows completion before calendar data arrives. *(Low)*

- [ ] **`SyncStatusEvent` now has 13 fields** — `should_sync_calendar` continues growth. *(Low)*

## Move Calendar Persistence and AI Category Writes into Rust (`e6f2fc8`)

- [ ] **`calendar_upsert_provider_events` silently uses NULL `calendar_id` when calendar not found** — Calendar lookup is `Optional`, so missing `remote_id` → events inserted with `calendar_id = NULL`. `calendar_apply_sync_result` errors on missing calendar instead. Inconsistent — should both error or both handle gracefully. *(BUG)*

- [ ] **`categorization_apply_ai_results` duplicates `db_set_thread_categories_batch`** — Identical SQL and logic. Old command still registered. Reuse or replace. *(Medium)*

- [ ] **18-column calendar event INSERT/ON CONFLICT duplicated twice in `calendar_commands.rs`** — Same SQL in `calendar_upsert_provider_events` and `calendar_apply_sync_result`. Extract to constant or helper. *(Medium)*

- [ ] **UUID generated for every calendar upsert including conflicts** — `uuid::Uuid::new_v4()` called per row even when ON CONFLICT → UPDATE discards the `id`. Harmless but wasteful. *(Low)*

- [ ] **`calendar_apply_sync_result` may clear existing `ctag` when only `sync_token` is provided** — Sets both `sync_token` and `ctag` unconditionally. `None` for one clears the column. Matches old TS behavior. *(Low)*

## Move Google Calendar Provider into Rust (`5eb75b2`)

- [ ] **Duplicate `Authorization` header in `google_calendar_execute_with_retry`** — Set on line 608 and again on line 616. Reqwest appends rather than replaces, so requests send two `Authorization` headers. Some servers may reject. *(BUG)*

- [ ] **All-day event timestamps use UTC instead of local timezone** — Old TS interpreted `T00:00:00`/`T23:59:59` in local timezone via `new Date()`. New Rust uses `.and_utc()`, shifting timestamps for non-UTC users. Affects event display. *(BUG)*

- [ ] **`Content-Type: application/json` set unconditionally for all requests** — Applied to GET/DELETE (no body). Also redundant for POST/PATCH where `request.json()` sets it. *(Medium)*

- [ ] **429 response returned as `Ok` after exhausting retries** — `google_calendar_execute_with_retry` returns the 429 response after max attempts. Works via indirect error path but should explicitly error. *(Medium)*

- [ ] **`let mut` with dead `let _ =` suppression** — `time_min`/`time_max` don't need `mut`. Remove `mut` instead of suppressing. *(Low)*

- [ ] **`updated` vec always empty in `google_calendar_sync_events`** — Initialized but never populated. Matches old TS but misleading. *(Low)*

- [ ] **`google_calendar_request_with_body` body parameter serves double duty** — Used for both initial request and 401 retry via `body.as_ref()`. Correct but fragile if refactored. *(Low)*

## Move Basic Account Lookups into Rust (`350fc78`)

- [ ] **CalDAV password decryption error now propagated instead of fallback** — Old TS silently fell back to raw value. New Rust propagates error via `?`, failing the operation. Could break CalDAV for accounts with corrupted encrypted passwords. *(Low)*

- [ ] **Inconsistent null handling between new account commands** — `account_get_basic_info` returns `Option` for missing accounts, `account_get_caldav_connection_info` returns an error. *(Low)*

## Move Account Summary Lookups into Rust (`bf32b79`)

- [ ] **`initializeClients` no longer checks token presence before init** — Old code skipped accounts without `access_token`/`refresh_token`. New code attempts `gmail_init_client` for all active gmail_api accounts, producing console errors for partially-configured accounts. *(Low)*

## Move CalDAV Provider into Rust (`b8169c1`)

- [ ] **Hand-rolled XML parser doesn't handle arbitrary namespace prefixes** — `split_xml_responses` only checks `<response` and `<d:response>`, `extract_first_element` only checks prefixes `""`, `"d:"`, `"c:"`, `"cs:"`, `"cal:"`. CalDAV servers may use `<D:response>`, `<ns0:response>`, or other prefixes. Old TS used `tsdav` with a real XML parser. Servers with non-standard prefixes will silently produce empty results. *(BUG)*

- [ ] **`caldav_request_with_headers` sets Content-Type twice for PUT with body** — When `body` is `Some(...)`, it unconditionally sets `Content-Type: application/xml`. But callers like `caldav_create_event` also pass `Content-Type: text/calendar` in the headers slice. reqwest keeps both — server sees duplicate Content-Type headers with different values. *(Medium)*

- [ ] **`parse_ical_datetime` treats floating datetimes as UTC and ignores TZID** — Last branch handles datetimes without `Z` suffix (iCal "floating" / local time) but does `.and_utc().timestamp()`. Also, `DTSTART;TZID=America/New_York:20260311T140000` — the `TZID` parameter is completely ignored since parser only looks at value after `:`. *(Medium)*

- [ ] **`unfold_ical_lines` doesn't handle LF-only continuation lines** — RFC 5545 folds with CRLF+SPACE/TAB but some real-world servers emit LF-only (`\n ` or `\n\t`). The function only handles `\r\n ` and `\r\n\t` variants. Long property values from such servers will appear split. *(Medium)*

- [ ] **New `reqwest::Client::new()` on every CalDAV command** — Each of the 7 commands creates a fresh client with no connection pooling. Old TS cached `DAVClient` in `this.client`. Means a fresh TLS handshake per CalDAV operation. Google Calendar commands reuse a client via `GmailState`; CalDAV should do similar. *(Medium)*

- [ ] **`html_unescape` is incomplete** — Only handles `&lt;`, `&gt;`, `&amp;`. Missing `&quot;`, `&apos;`, and numeric character references (`&#123;`, `&#x7B;`). Calendar display names with quotes or special chars will show raw entities. *(Low)*

- [ ] **`extract_tag_value` returns nested elements as content** — Uses first `>` to last `<` to extract text. If element contains nested elements (e.g., `<displayname><inner>text</inner></displayname>`), returns the inner markup instead of text. Fragility of hand-rolled parser. *(Low)*

- [ ] **`CALDAV_NS` constant gives false sense of namespace correctness** — Used in XML body format strings but the XML parser's `extract_first_element` doesn't reference namespaces at all — it just pattern-matches hardcoded prefix strings. *(Low)*

## Move AI Provider Runtime into Rust (`eee4079`)

- [ ] **New `reqwest::Client::new()` on every AI completion call** — Each `complete_*` function creates a fresh client with no connection pooling. AI is called frequently during post-sync hooks (categorization, smart labels, auto-drafts), meaning repeated TLS handshakes to the same API endpoints. Same pattern as CalDAV. *(Medium)*

- [ ] **`map_http_error` rate limit detection is overly broad** — `body.to_lowercase().contains("rate")` matches any response body mentioning "rate" in any context (e.g., "accuracy rate"). Will misclassify unrelated 4xx/5xx errors as `RATE_LIMITED`. Faithfully ported from TS but now centralized so easier to fix. *(Medium)*

- [ ] **`load_ai_config` makes multiple sequential DB reads** — Provider name, model, and API key each go through separate `with_conn` round-trips. Could fetch all AI-related settings in a single query. *(Low)*

- [ ] **`read_plain_setting` clones the key string twice** — `key_name` and `key_label` are both `.to_string()` of `key`. Only one is needed for the closure move; the error message could use the same variable or a static description. *(Low)*

## Simplify AI Service Calls onto Rust Client (`68c572d`)

- [ ] **Duplicate `callAi` wrapper in two services** — Both `aiService.ts` and `writingStyleService.ts` define identical `callAi(systemPrompt, userContent)` that just calls `completeAi`. Callers could use `completeAi` directly or share a single wrapper. *(Low)*

## Remove Dead TypeScript AI Providers (`17d411e`)

- [x] **`clearProviderClients` remains as an exported no-op** — Resolved in `a7a01f7`: function, export, all call sites, and test removed.

## Trim Account Summaries and AI Settings Cleanup (`a7a01f7`)

- [ ] **Account-to-store mapping duplicated 4 times** — `App.tsx` (twice), `ComposerWindow.tsx`, and `ThreadWindow.tsx` all have identical `dbAccounts.map(a => ({ id, email, displayName, avatarUrl, isActive, provider }))`. Could be a shared helper since the mapping is now trivial. *(Low)*
