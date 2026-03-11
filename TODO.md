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

- [ ] **Graph folder CRUD returns "not supported"** — `create_folder`, `rename_folder`, `delete_folder` are stubbed in `src-tauri/src/graph/ops.rs`. Graph API actually supports folder CRUD via `/me/mailFolders`. *(LOW)*

- [ ] **Snippet fallback truncation not grapheme-safe** — `imap_message_to_provider_message` uses `.chars().take(200).collect()` which can split multi-byte grapheme clusters. Minor cosmetic issue. *(LOW)*

---

## Sync Engine

- [ ] **`has_history` gate has fragile provider semantics** — `history_id` column means different things per provider (Google history ID, JMAP state token, Graph delta link, IMAP synthetic marker). Using `IS NOT NULL` as initial-vs-delta gate breaks if any provider sets it before completing initial sync. *(MED)*

- [ ] **IMAP `storedCount` proxy for "things changed" is lost** — `sync_initial` returns `Result<(), String>` — no data about what was stored. If anything beyond categorization depends on knowing whether initial sync stored messages, it's now blind. *(MED)*

- [ ] **Provider-agnostic commands in IMAP-specific module** — `sync_prepare_full_sync` and `sync_prepare_account_resync` are provider-agnostic but live in `sync/commands.rs`. Consider moving to `provider/commands.rs`. *(LOW)*

- [ ] **No "falling back to initial" progress event** — When delta sync fails and falls back to initial, the UI may show confusing progress. No event signals the fallback. *(LOW)*

- [ ] **Gmail sync still fully in TS** — `src/services/gmail/syncManager.ts:80-112` — `syncGmailAccount()` uses Gmail REST API via TS HTTP calls, not the Rust sync engine. Porting is a large effort with minimal benefit since HTTP overhead dominates. *(LOW)*

- [ ] **No per-operation timeout on Rust IMAP fetches** — `src-tauri/src/sync/imap_initial.rs` — No operation-level timeout on individual FETCH commands. A folder with 50K+ messages could hang indefinitely. Fix: wrap in `tokio::time::timeout()`. *(LOW)*

- [ ] **JMAP initial sync re-queries entire result set every batch** — `src-tauri/src/jmap/sync.rs:108-146` — O(n²) server calls. Fix: use JMAP `position` + `limit` for server-side pagination, or cache IDs from first query. *(LOW)*

---

## Post-Sync Hooks

> **Systemic issue**: Rust sync now shares one post-sync message load across filters, criteria smart labels, AI prep, and notifications, but later TS-side AI matching still re-queries message data and post-sync actions still duplicate some provider/setup work. The remaining debt is mostly across the Rust/TS boundary.

- [ ] **Filter and smart label actions applied sequentially instead of in parallel** — Old TS used `Promise.allSettled` for concurrent per-thread application. Rust iterates sequentially. Could use `tokio::task::JoinSet`. *(MED)*

- [ ] **Filter body hydration loads all bodies before evaluation** — When any filter has a body criterion, `body_store.get_batch` is called for all message IDs upfront. Could defer to only messages passing non-body criteria first. *(LOW)*

- [ ] **Correlated subquery for latest message per thread** — `AND m.date = (SELECT MAX(m2.date) ...)` runs per-thread. Fine with LIMIT 20 but inherited technical debt. *(LOW)*

- [ ] **`smart_labels_apply_matches` only callable via IPC** — Label application after AI classification still crosses the IPC boundary. Could be called directly in Rust once AI classification moves too. *(LOW)*

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
