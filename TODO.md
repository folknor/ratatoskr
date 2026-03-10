# TODO

## Bugs

### HIGH

- [ ] **Auto-updater should check local permissions** — Don't show update prompts if the user lacks write access to the app installation directory (e.g., installed system-wide without admin rights). The update would fail anyway — detect this upfront and either hide the prompt or show a helpful message.

### LOW

- [ ] **IMAP `fetchAttachment` returns base64 `data.length` as `size`** — `src/services/email/imapSmtpProvider.ts:267-269`

  Base64 inflates size by ~33%. The `EmailProvider` interface says `size` is in bytes, but the returned value is the base64 string length.

  Fix: Return `Math.floor(data.length * 3 / 4)` or decode and measure.

---

## Security & Data Safety

### HIGH

- [ ] **`withSerializedExecution` has no real SQL transaction** — `src/services/db/connection.ts:51-78`

  Serializes operations via a JS promise queue but explicitly does NOT use `BEGIN`/`COMMIT`/`ROLLBACK` (comment on line 70-73 explains tauri-plugin-sql pool constraint). If the app crashes mid-"transaction", partial writes persist. For example, during IMAP sync, a crash after `upsertThread` but before `upsertMessage` leaves an empty thread. `setThreadLabels` (DELETE-then-INSERT pattern) can lose all labels on a crash between the two statements.

  Fix: Use `SAVEPOINT`/`RELEASE` if the pool issue is specifically with nested transactions. Or move critical multi-step writes to Rust-side `DbState::with_conn` where real transactions are available.

### MEDIUM

- [ ] **AI API keys may be stored in plaintext** — `src/services/db/settings.ts`

  `getSecureSetting`/`setSecureSetting` exist, but if any code path uses `setSetting` instead of `setSecureSetting` for API keys, they are stored unencrypted.

  Fix: Audit all settings writes for credential-like keys to ensure they use `setSecureSetting`.

- [ ] **`sql:allow-execute` grants arbitrary SQL from frontend** — `src-tauri/capabilities/default.json:17`

  The frontend can execute arbitrary SQL (INSERT, UPDATE, DELETE, DROP). Any XSS could do `__TAURI__.invoke('plugin:sql|execute', {query: 'DROP TABLE accounts'})`. Inherent to the architecture.

  Fix: Migrate remaining critical DB operations to Rust Tauri commands (partially done with `db_*` commands), eventually remove `sql:allow-execute`.

### LOW

- [ ] **Decryption failure fallback returns plaintext** — `src/services/db/accounts.ts:40-81`

  When decryption fails, code falls back to the raw (potentially plaintext) value with only `console.warn`. Credentials stored before encryption was enabled remain accessible in plaintext indefinitely.

- [ ] **`synchronous=NORMAL` with WAL mode** — `src/services/db/connection.ts:10`, `src-tauri/src/db/mod.rs:50`

  Committed transactions can be lost on power failure (DB won't corrupt, but data lost). Acceptable for server-synced email, but locally-composed drafts, tasks, and settings could be lost.

- [ ] **Draft auto-save has no crash-recovery guarantee** — `src/services/composer/draftAutoSave.ts`

  3-second debounce means up to 3s of content lost on crash. Combined with `synchronous=NORMAL`, even locally-persisted drafts in `local_drafts` might not survive power failure.

---

## Refactoring — Patterns & Boilerplate

- [ ] **`moveToFolder` only adds label, doesn't remove source** — `src-tauri/src/email_actions/commands.rs`

  `email_action_move_to_folder` inserts the target folder label into `thread_labels` but doesn't remove the old label (e.g., INBOX). The TS code had the same behavior, so it's a pre-existing gap — the provider-side IMAP MOVE or Gmail API call handles the actual folder change on the server, but the local DB state shows both labels until the next sync refreshes thread labels.

  In practice this means a thread moved from Inbox to Archive will briefly appear in both views until the next delta sync (≤60s). Not a functional bug since the server state is correct and the UI will self-correct, but it can cause momentary confusion. The fix would be to DELETE the old label in the same transaction, but that requires knowing which label to remove — for archive it's always INBOX, but for arbitrary folder moves the source isn't passed to the command. Would need to either pass the source label as a parameter or query existing labels in the command.

---

## Consolidation — Dead Code & Duplication

- [ ] **JMAP `commands.rs` action commands duplicate `ops.rs` trait implementations** — `src-tauri/src/jmap/commands.rs:228-555`

  ~330 lines of `jmap_archive`, `jmap_trash`, `jmap_mark_read`, etc. are exact duplicates of the `JmapOps` trait methods. Since `emailActions.ts` now routes through unified `provider_*` commands, these per-provider commands are dead from the action dispatch path. The `jmap_*` lifecycle/sync/folder/attachment commands should remain.

  Fix: Remove ~330 lines from `commands.rs` and registrations from `lib.rs`. **Risk: MEDIUM** — verify no TS code calls `jmap_archive` etc. directly.

- [ ] **Wizard step indicator duplicated across account components** — `AddImapAccount.tsx:363-391`, `AddJmapAccount.tsx:226-254`

  Nearly identical `renderStepIndicator()` functions with same CSS, layout, and active/completed state logic.

  Fix: Extract `<StepIndicator steps={...} currentStep={...} />`. Saves ~60 lines. **Risk: LOW.**

- [ ] **`GmailClient` TS class (461 lines) + legacy `tokenManager.ts` kept for one caller** — `src/services/gmail/client.ts`, `tokenManager.ts`

  `GmailClient` is `@deprecated`. Only `googleCalendarProvider.ts` uses it. `tokenManager.ts` creates legacy TS clients for every Gmail account on startup solely for this.

  Fix: Migrate calendar provider to use Rust HTTP or direct `fetch` with token from DB. Removes ~600 lines. **Risk: MEDIUM.**

- [ ] **Duplicate email action dispatchers** — `src/services/emailActions.ts:278-428`

  `executeViaProviderRust` and `executeViaImapProvider` are nearly identical switch statements. When IMAP actions go through Rust `provider_*` commands, `executeViaImapProvider` becomes dead code.

  Fix: Remove ~50 lines once IMAP is fully ported. **Risk: MEDIUM.**

- [ ] **TS IMAP sync fallback path (1,259 lines)** — `imapSync.ts`, `imapSyncConvert.ts`, `imapSyncFetch.ts`, `imapSyncStore.ts`

  Only used when `use_rust_sync` setting is `"false"`. Default is `true` (Rust sync). Legacy fallback for the old TS IMAP sync pipeline.

  Fix: Remove once Rust IMAP sync is proven stable. **Risk: HIGH** — gate on release milestone.

- [ ] **`emailActions.test.ts` references old command names** — Tests assert `gmail_modify_thread`, `gmail_delete_thread`, `gmail_send_email`, `gmail_create_draft` etc., but code now calls `provider_*` commands. Tests may be broken or giving false confidence.

---

## Gmail→Rust Migration Follow-ups

- [ ] **`getGmailClient()` retained for Calendar only** — `src/services/calendar/googleCalendarProvider.ts`

  The TS `GmailClient` class and `tokenManager.ts` client cache are kept solely because `googleCalendarProvider.ts` uses `GmailClient` for Google Calendar API calls (same OAuth token, different endpoint). Once a Rust Calendar client exists, `getGmailClient()`, `client.ts`, and the legacy client cache in `tokenManager.ts` can be deleted entirely.

---

## Phase 4 (Rust Sync Engine) Follow-ups

### MEDIUM

- [ ] **Body text unavailable for filter body matching** — `src/services/filters/filterEngine.ts:168`

  `dbMessageToParsedMessage()` reads `body_html`/`body_text` from the `messages` table columns, but these are always NULL — message bodies live in the separate `bodies.db` file (zstd-compressed, accessed via `body_store_get`/`body_store_get_batch`). This means any filter rule with `criteria.body` set will never match, because the body fields on the `DbMessage` object are null.

  This affects the `applyFiltersToNewMessageIds()` path used by the Rust sync engine (`syncManager.ts:182`) and the `applyFiltersToMessages()` path used by TS IMAP delta sync (`imapSync.ts:886`). The Gmail delta sync path (`sync.ts:510`) is not affected because it passes `ParsedMessage` objects that already have bodies populated from the API response before they're stored.

  Options: (1) Hydrate bodies from the body store in `applyFiltersToNewMessageIds()` by calling `bodyStoreGetBatch()` for the message IDs — simplest fix, adds one IPC call but only when filters with body criteria exist. (2) Move filter evaluation into the Rust sync pipeline where bodies are available before compression — more efficient but much larger change. (3) Accept the limitation and document that body-matching filters only work on Gmail API accounts — least effort, but surprising to users. Option 1 is probably the right call.

### LOW

- [ ] **Gmail sync still fully in TS** — `src/services/gmail/syncManager.ts:80-112`

  `syncGmailAccount()` uses the Gmail REST API via `GmailClient` (HTTP, not IMAP), so it doesn't benefit from the Rust IMAP sync engine at all. The function calls `initialSync()` or `deltaSync()` from `sync.ts`, which make HTTP requests to `googleapis.com` via the Tauri HTTP plugin, parse JSON responses, and store to the TS-side DB layer.

  Porting would mean: (1) implementing Gmail REST API calls in Rust with `reqwest` (threads.list, threads.get, history.list, messages.get), (2) handling OAuth token refresh in Rust (currently `GmailClient` auto-refreshes 5min before expiry), (3) reimplementing the batched thread fetch logic, history-based delta sync, and HISTORY_EXPIRED fallback. This is a large effort because the Gmail API has different semantics from IMAP — it returns threads natively (no JWZ threading needed), uses history IDs instead of UIDs, and requires OAuth2 bearer tokens. The current TS implementation works well and the HTTP overhead dominates anyway (not IPC), so the benefit of porting is minimal. Only worth considering if we want a unified Rust sync pipeline for architectural consistency.

- [ ] **No per-operation timeout on Rust IMAP fetches** — `src-tauri/src/sync/imap_initial.rs`

  Connection-level timeouts exist via `async-imap`'s TCP stream configuration, but there's no operation-level timeout on individual FETCH commands. A folder with 50K+ messages could result in a single IMAP FETCH that takes arbitrarily long — the server might be slow, rate-limiting, or the connection could be half-open (TCP keepalive not triggered yet).

  The risk is a sync that hangs indefinitely on a large folder, blocking the sync timer for that account. Since sync runs sequentially per account (`syncAccountInternal` is awaited), a hang blocks all subsequent accounts too. The fix would be wrapping each `imap_fetch_messages` call (or the entire folder sync) in a `tokio::time::timeout()`. Need to choose a reasonable duration — probably 5-10 minutes per folder, since legitimate large-folder fetches can take a while. The `async-imap` session would need to be dropped on timeout to close the connection cleanly. Low priority because it's a rare edge case (most IMAP servers handle large fetches fine), but it's a robustness improvement for pathological cases.

- [ ] **JMAP initial sync re-queries entire result set every batch** — `src-tauri/src/jmap/sync.rs:108-146`

  The loop re-executes `email_query` on every iteration, getting the full result set, then does `.skip(position).take(BATCH_SIZE)`. For 10,000 emails with `BATCH_SIZE=50`, this makes 200 queries each returning 10,000 IDs. If the result set changes between queries, position-based skip can miss messages or process duplicates.

  Fix: Use JMAP query's `position` + `limit` parameters for server-side pagination, or cache IDs from the first query and batch-fetch from the cached list.

---

## Branding / Assets

- [ ] **Replace logo SVG** — `src/assets/logo.svg` still renders the old "VELO" text as path outlines. Needs a new logo for Ratatoskr.

- [ ] **Replace app icons** — `src-tauri/icons/`, `assets/icon.png`, `src/assets/logo.svg`, and the inline SVG in `splashscreen.html` all contain old Velo branding. Need new Ratatoskr icons for all platforms (macOS .icns, Windows .ico, Linux .png at 32x32, 128x128, 256x256, 512x512) plus the root asset and splash screen.

---

## Phase 3b (Graph Provider) Known Issues

- [ ] **Category add/remove is racy** — `src-tauri/src/graph/ops.rs`

  `add_category`/`remove_category` do a read-then-write (fetch current categories, modify, PATCH back). Two concurrent actions on the same message could clobber each other. Graph has no atomic "add to array" operation, so this is unavoidable but worth knowing.

- [ ] **No `$batch` optimization for thread actions** — `src-tauri/src/graph/ops.rs`

  Thread-level actions loop through messages one-by-one. The plan doc describes batching up to 20 requests per `/$batch` call. Left as follow-up — per-message approach is correct but slower under the 4-concurrent limit.

- [ ] **`raw_size` is always 0 for Graph messages** — `src-tauri/src/graph/sync.rs`

  Graph's message API has no first-class size property. The MAPI extended property `PidTagMessageSize` (`0x0E08`) is available via `$expand=singleValueExtendedProperties($filter=id eq 'Integer 0x0E08')`, but this can't be combined with `$select` (Microsoft treats it as an advanced query conflict). Dropping `$select` to get size would fetch full message objects — unacceptable for sync performance under the 4-concurrent limit. Separate per-message calls are equally impractical. Accepted as a cosmetic limitation — only affects storage stats display, not functional.

---

## TypeScript Strictness

- [ ] **39 remaining TS errors** — Mostly from `exactOptionalPropertyTypes` (34 TS2375/TS2379) and other type mismatches (TS2322, TS2345). Decide whether to fix all or relax the option.
