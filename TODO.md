# TODO

## Bugs

### HIGH

- [ ] **Auto-updater should check local permissions** — Don't show update prompts if the user lacks write access to the app installation directory (e.g., installed system-wide without admin rights). The update would fail anyway — detect this upfront and either hide the prompt or show a helpful message.

---

## Refactoring — Patterns & Boilerplate

- [ ] **`moveToFolder` only adds label, doesn't remove source** — `src-tauri/src/email_actions/commands.rs`

  `email_action_move_to_folder` inserts the target folder label into `thread_labels` but doesn't remove the old label (e.g., INBOX). The TS code had the same behavior, so it's a pre-existing gap — the provider-side IMAP MOVE or Gmail API call handles the actual folder change on the server, but the local DB state shows both labels until the next sync refreshes thread labels.

  In practice this means a thread moved from Inbox to Archive will briefly appear in both views until the next delta sync (≤60s). Not a functional bug since the server state is correct and the UI will self-correct, but it can cause momentary confusion. The fix would be to DELETE the old label in the same transaction, but that requires knowing which label to remove — for archive it's always INBOX, but for arbitrary folder moves the source isn't passed to the command. Would need to either pass the source label as a parameter or query existing labels in the command.

- [x] **~~Unsafe type assertions~~** — Fixed. `gmailProvider.ts` cast eliminated via `GmailRawMessage` type + function overloads on `getMessage()`. `crypto.ts` cast is a genuine TS lib limitation (Uint8Array vs BufferSource generics) — kept with explanatory comment. `ContextMenuPortal.tsx` casts were removed during the earlier component split refactor.

---

## Gmail→Rust Migration Follow-ups

- [x] **~~`scheduledSendManager` is Gmail-only~~** — Fixed. Now uses `sendEmail()` from `emailActions.ts` (provider-agnostic).

- [x] **~~`unsubscribeManager` mailto send is Gmail-only~~** — Fixed. Now uses `sendEmail()` from `emailActions.ts` via dynamic import.

- [x] **~~`EmailList` draft lookup is Gmail-only~~** — Fixed. Now branches on `account.provider`: Gmail calls `gmail_list_drafts`, IMAP/JMAP use message ID directly as draft ID.

- [x] **~~`MultiSelectBar` permanent delete double-deletes locally~~** — Fixed. Removed redundant `deleteThreadFromDb()` call — `permanentDeleteThread()` already executes the identical `DELETE FROM threads` via Rust.

- [ ] **`getGmailClient()` retained for Calendar only** — `src/services/calendar/googleCalendarProvider.ts`

  The TS `GmailClient` class and `tokenManager.ts` client cache are kept solely because `googleCalendarProvider.ts` uses `GmailClient` for Google Calendar API calls (same OAuth token, different endpoint). Once a Rust Calendar client exists, `getGmailClient()`, `client.ts`, and the legacy client cache in `tokenManager.ts` can be deleted entirely.

---

## Phase 4 (Rust Sync Engine) Follow-ups

### MEDIUM

- [ ] **Body text unavailable for filter body matching** — `src/services/filters/filterEngine.ts:168`

  `dbMessageToParsedMessage()` reads `body_html`/`body_text` from the `messages` table columns, but these are always NULL — message bodies live in the separate `bodies.db` file (zstd-compressed, accessed via `body_store_get`/`body_store_get_batch`). This means any filter rule with `criteria.body` set will never match, because the body fields on the `DbMessage` object are null.

  This affects the `applyFiltersToNewMessageIds()` path used by the Rust sync engine (`syncManager.ts:182`) and the `applyFiltersToMessages()` path used by TS IMAP delta sync (`imapSync.ts:886`). The Gmail delta sync path (`sync.ts:510`) is not affected because it passes `ParsedMessage` objects that already have bodies populated from the API response before they're stored.

  Options: (1) Hydrate bodies from the body store in `applyFiltersToNewMessageIds()` by calling `bodyStoreGetBatch()` for the message IDs — simplest fix, adds one IPC call but only when filters with body criteria exist. (2) Move filter evaluation into the Rust sync pipeline where bodies are available before compression — more efficient but much larger change. (3) Accept the limitation and document that body-matching filters only work on Gmail API accounts — least effort, but surprising to users. Option 1 is probably the right call.

- [x] **~~`DbMessage` / schema column audit~~** — Audited and fixed. Removed phantom `has_attachments` from TS `DbMessage` (column only exists on `threads`, not `messages`). Fixed Rust `DbMessage.date` from `Option<String>` → `i64` and `internal_date` from `Option<String>` → `Option<i64>` to match schema INTEGER types. Fixed `backfillService.ts` SQL query referencing `m.has_attachments` (nonexistent column) → `t.has_attachments`. Remaining minor: TS `DbMessage` types `is_read`/`is_starred` as `number` (matches direct SQL 0/1) but Rust returns `bool` — both paths work correctly with their respective consumers.

- [x] **~~Recovery logic duplicated between TS wrapper and Rust~~** — Moved into `sync_imap_delta` Rust command (`src-tauri/src/sync/commands.rs`). Recovery (thread count check → clear history_id + folder sync states → initial sync) now happens entirely within the single `sync_imap_delta` invoke, eliminating 3 IPC round-trips. The TS fallback path (`syncManager.ts:282-307`) retains its own recovery since it doesn't use the Rust engine.

### LOW

- [ ] **Gmail sync still fully in TS** — `src/services/gmail/syncManager.ts:80-112`

  `syncGmailAccount()` uses the Gmail REST API via `GmailClient` (HTTP, not IMAP), so it doesn't benefit from the Rust IMAP sync engine at all. The function calls `initialSync()` or `deltaSync()` from `sync.ts`, which make HTTP requests to `googleapis.com` via the Tauri HTTP plugin, parse JSON responses, and store to the TS-side DB layer.

  Porting would mean: (1) implementing Gmail REST API calls in Rust with `reqwest` (threads.list, threads.get, history.list, messages.get), (2) handling OAuth token refresh in Rust (currently `GmailClient` auto-refreshes 5min before expiry), (3) reimplementing the batched thread fetch logic, history-based delta sync, and HISTORY_EXPIRED fallback. This is a large effort because the Gmail API has different semantics from IMAP — it returns threads natively (no JWZ threading needed), uses history IDs instead of UIDs, and requires OAuth2 bearer tokens. The current TS implementation works well and the HTTP overhead dominates anyway (not IPC), so the benefit of porting is minimal. Only worth considering if we want a unified Rust sync pipeline for architectural consistency.

- [ ] **No per-operation timeout on Rust IMAP fetches** — `src-tauri/src/sync/imap_initial.rs`

  Connection-level timeouts exist via `async-imap`'s TCP stream configuration, but there's no operation-level timeout on individual FETCH commands. A folder with 50K+ messages could result in a single IMAP FETCH that takes arbitrarily long — the server might be slow, rate-limiting, or the connection could be half-open (TCP keepalive not triggered yet).

  The risk is a sync that hangs indefinitely on a large folder, blocking the sync timer for that account. Since sync runs sequentially per account (`syncAccountInternal` is awaited), a hang blocks all subsequent accounts too. The fix would be wrapping each `imap_fetch_messages` call (or the entire folder sync) in a `tokio::time::timeout()`. Need to choose a reasonable duration — probably 5-10 minutes per folder, since legitimate large-folder fetches can take a while. The `async-imap` session would need to be dropped on timeout to close the connection cleanly. Low priority because it's a rare edge case (most IMAP servers handle large fetches fine), but it's a robustness improvement for pathological cases.

---

## Branding / Assets

- [ ] **Replace logo SVG** — `src/assets/logo.svg` still renders the old "VELO" text as path outlines. Needs a new logo for Ratatoskr.

- [ ] **Replace app icons** — `src-tauri/icons/`, `assets/icon.png`, `src/assets/logo.svg`, and the inline SVG in `splashscreen.html` all contain old Velo branding. Need new Ratatoskr icons for all platforms (macOS .icns, Windows .ico, Linux .png at 32x32, 128x128, 256x256, 512x512) plus the root asset and splash screen.

---

## Phase 3b (Graph Provider) Known Issues

- [x] **~~`fetch_all_folders` pagination bug~~** — Fixed. Moved folder fetching to `src-tauri/src/graph/sync.rs` with consistent `get_absolute()` for OData `@odata.nextLink` pagination. Both `fetch_all_folders()` and `fetch_child_folders()` now use `get_json()` for the initial request and `get_absolute()` for all subsequent pages.

- [x] **~~`addr_to_recipients` mail-parser API unverified~~** — Verified correct. `Address::iter()` in mail-parser 0.11 transparently yields `&Addr` items from both `List` and `Group` variants. The code matches the proven pattern in `imap/parse.rs`.

- [ ] **Category add/remove is racy** — `src-tauri/src/graph/ops.rs`

  `add_category`/`remove_category` do a read-then-write (fetch current categories, modify, PATCH back). Two concurrent actions on the same message could clobber each other. Graph has no atomic "add to array" operation, so this is unavoidable but worth knowing.

- [ ] **No `$batch` optimization for thread actions** — `src-tauri/src/graph/ops.rs`

  Thread-level actions loop through messages one-by-one. The plan doc describes batching up to 20 requests per `/$batch` call. Left as follow-up — per-message approach is correct but slower under the 4-concurrent limit.

- [x] **~~`update_draft` changes the draft ID~~** — Fixed. `draftAutoSave.ts` now captures the returned `ActionResult` from `updateDraftAction()` and calls `setDraftId()` with the new ID when it differs. Handles both plain string (Rust Graph) and `{ draftId }` object (IMAP TS) return shapes.

- [x] **~~Sync is fully stubbed~~** — Fixed. Implemented in `src-tauri/src/graph/sync.rs` (~550 lines) + `src-tauri/src/graph/parse.rs` (~170 lines). Covers: folder sync + label persistence, per-folder paginated message fetch with date filter, message parsing (GraphMessage → DB-ready struct with header extraction, label derivation, ISO date parsing), per-folder delta token bootstrap and incremental delta sync, DB writes (thread/message/label upsert), body store (zstd-compressed), Tantivy search indexing, pending-ops conflict filter, and progress events.

- [x] **~~`microsoft_client_id` settings key has no UI~~** — Fixed. Added `microsoft_client_id` field to Settings page (`SettingsAccountsTab.tsx`), `AddGraphAccount.tsx` OAuth setup wizard, `insertGraphAccount()` DB function, `syncGraphAccount()` in syncManager, Graph client init in `App.tsx` startup, and `"graph"` routing across providerFactory/syncManager/AddAccount.

- [x] **~~No attachment enumeration during sync~~** — Fixed. Added `$expand=attachments($select=id,name,contentType,size,isInline,contentId)` to message fetch URLs, `GraphAttachment` fields on `GraphMessage`, `ParsedGraphAttachment` struct in parse.rs, and `upsert_attachments()` in sync.rs following the JMAP pattern. Graph attachment IDs stored in `gmail_attachment_id` column.

- [ ] **`raw_size` is always 0 for Graph messages** — `src-tauri/src/graph/sync.rs`

  Graph's message API has no first-class size property. The MAPI extended property `PidTagMessageSize` (`0x0E08`) is available via `$expand=singleValueExtendedProperties($filter=id eq 'Integer 0x0E08')`, but this can't be combined with `$select` (Microsoft treats it as an advanced query conflict). Dropping `$select` to get size would fetch full message objects — unacceptable for sync performance under the 4-concurrent limit. Separate per-message calls are equally impractical. Accepted as a cosmetic limitation — only affects storage stats display, not functional.

- [x] **~~`list_folders` always re-syncs from server~~** — Fixed. Added `folder_map_last_sync: RwLock<Option<Instant>>` to `ClientInner`. `list_folders` now returns the cached `FolderMap` if it was synced within the last 60 seconds, avoiding unnecessary API calls.

- [x] **~~Delta sync processes all folders every cycle~~** — Fixed. Added `sync_cycle_counter: AtomicU32` to `ClientInner`. `graph_delta_sync` increments on each run and filters folders by priority tier: INBOX/SENT/DRAFT every cycle, TRASH/SPAM/archive every 5th, user folders every 20th.

- [x] **~~No folder tree re-traversal during delta sync~~** — Fixed. Every 10th sync cycle, `graph_delta_sync` calls `sync_folders()` to discover new/renamed/deleted folders. New folders get delta tokens bootstrapped via `$deltatoken=latest`. Stale tokens for removed folders are cleaned up from the DB.

---

## TypeScript Strictness

- [ ] **39 remaining TS errors** — Mostly from `exactOptionalPropertyTypes` (34 TS2375/TS2379) and other type mismatches (TS2322, TS2345). Decide whether to fix all or relax the option.
