# TODO

## Bugs

### HIGH

- [x] ~~**Draft auto-save race condition** — `src/services/composer/draftAutoSave.ts`
  Fixed: `saveDraft()` now reads `activeAccountId` from account store at save time instead of capturing as a closure variable.~~

- [x] ~~**Stale closure in EmailList mapDbThreads** — `src/hooks/useEmailListData.ts:208`
  `mapDbThreads` had empty dependency array `[]`. Added `activeAccountId` to deps.~~

### MEDIUM

- [x] ~~**15× silent settings failures** — `src/stores/uiStore.ts`
  Fixed: `persistSetting()` helper replaces silent catches with error logging.~~

- [x] ~~**Backfill only runs once per app lifetime** — `src/App.tsx`
  Fixed: `backfillDoneRef` now tracks per account ID via `Set<string>`.~~

- [x] ~~**Draft null access** — `src/components/layout/EmailList.tsx` + `src/services/gmail/draftDeletion.ts`
  Fixed: optional chaining for `d.message?.id` and `d.message?.threadId`.~~

- [x] ~~**Sync queue race** — `src/services/gmail/syncManager.ts`
  Fixed: synchronous check + Set-based merging with drain loop.~~

- [ ] **Queue processor loses error context** — `src/services/queue/queueProcessor.ts:46-56`
  Original error details lost on permanent failures; only the classified message is stored.

### LOW

- [ ] **Silent attachment pre-cache failures** — `src/services/attachments/preCacheManager.ts:78-80`
  Empty catch block; no visibility when attachment caching fails systematically.

- [ ] **Migration rollback error swallowed** — `src/services/db/migrations.ts:915`
  `ROLLBACK` failure caught and ignored. Could leave transaction open.

- [ ] **pendingOperations.ts still uses direct SQL for reads** — `src/services/db/pendingOperations.ts`
  Queue writes now go through Rust (`db_enqueue_pending_operation`), but reads (`getPendingOperations`, `compactQueue`, `incrementRetry`, `deleteOperation`, etc.) still use `getDb()` direct SQL. Port to Rust commands for consistency.

---

## Duplicated Business Logic

### CRITICAL

- [x] ~~**Thread label ops implemented in 3 places** — snooze now routes through emailActions~~

- [x] ~~**Snooze bypasses offline action queue** — snooze now goes through `executeEmailAction()` with optimistic UI, local DB update, offline queue, and provider sync (archive)~~

- [x] ~~**Pin/unpin not offline-safe** — pin/unpin/mute/unmute now go through `executeEmailAction()` with optimistic UI and local DB updates. Pin/unpin/unmute are local-only; mute delegates to `provider.archive()`~~

### MEDIUM

- [x] ~~**Date parsing duplicated** — consolidated into `src/utils/date.ts`~~

- [ ] **IMAP messages may skip filter engine**
  `src/services/filters/filterEngine.ts` runs for Gmail sync but appears missing from the IMAP sync flow (`src/services/imap/imapSync.ts`). Verify and add if missing.

- [x] ~~**Multi-select target resolution duplicated** — extracted `resolveContextMenuTargets` and `resolveKeyboardTargets` into `src/utils/multiSelectTargets.ts`~~

---

## Refactoring — Large Files

- [x] ~~**SettingsPage.tsx** (2992→600 lines) — Extracted 10 tab components + SettingsShared.tsx~~

- [x] ~~**imapSync.ts** (1209→865 lines) — Extracted imapSyncConvert.ts, imapSyncFetch.ts, imapSyncStore.ts~~

- [x] ~~**EmailList.tsx** (1045→271 lines) — Extracted useEmailListData hook, EmailListHeader, MultiSelectBar, EmptyStateForContext, BundleRow~~

- [x] ~~**AddImapAccount.tsx** (1005→498 lines) — Extracted 4 wizard step components + shared types~~

- [ ] **ContextMenuPortal.tsx** (796 lines) — Extract per-menu-type components (ThreadContextMenu, MessageContextMenu, SidebarLabelContextMenu). Move quote builders to `utils/emailQuoteBuilders.ts`.

- [ ] **Composer.tsx** (691 lines) — Extract template shortcut engine and editor config setup.

---

## Refactoring — Patterns & Boilerplate

- [x] **Rust IMAP session boilerplate** — `src-tauri/src/commands.rs`
  15 command functions with identical connect → work → logout pattern. Created `with_imap_session!` macro.

- [ ] **`moveToFolder` only adds label, doesn't remove source** — `src-tauri/src/email_actions/commands.rs`
  `email_action_move_to_folder` inserts the target folder label but doesn't remove the old label (e.g., INBOX). The TS code had the same behavior, so it's a pre-existing gap — the provider-side move handles the actual folder change, but the local DB state is incomplete until next sync.

- [x] **Rust timeout error messages** — `src-tauri/src/imap/client.rs`
  Same `format!("...timed out after {}s — check your server...")` repeated 10+ times. Create a timeout error helper or macro. *(Done — `timeout_err()` helper function.)*

- [x] ~~**Zustand settings persistence** — `src/stores/uiStore.ts`
  Fixed: `persistSetting()` helper with error logging.~~

- [x] ~~**ContextMenuPortal batch operations** — `src/components/ui/ContextMenuPortal.tsx`
  Fixed: `batchToggle()` helper for toggle read/star/pin/mute.~~

- [ ] **Unsafe type assertions** — `src/services/email/gmailProvider.ts:140`, `src/utils/crypto.ts:54`, `src/components/ui/ContextMenuPortal.tsx:167-268`
  Multiple `as unknown as` casts and untyped context menu data. Create typed payload interfaces and guards.

---

## Refactoring — State Management

- [ ] **Split uiStore** (17+ properties mixing layout, preferences, and sync state)
  Consider splitting into `uiLayoutStore`, `uiPreferencesStore`, `syncStateStore` to reduce re-render scope.

---

## Phase 4 (Rust Sync Engine) Follow-ups

### HIGH

- [x] ~~**`has_attachments` missing from `DbMessage` interface** — `src/services/db/messages.ts:5`
  Fixed: added `has_attachments: number` to `DbMessage`, updated `dbMessageToParsedMessage()` to use it.~~

- [x] ~~**`getMessagesByIds` hits SQLite parameter limit** — `src/services/db/messages.ts:46`
  Fixed: chunked into batches of 500 to stay under SQLite's 999-parameter limit.~~

- [x] ~~**No notification dispatch in Rust sync path** — `src/services/gmail/syncManager.ts:113`
  Fixed: added full notification pipeline (smart notifications, VIP senders, muted threads, category gating) to `syncImapAccountRust()`, only on delta sync.~~

### MEDIUM

- [ ] **Body text unavailable for filter body matching** — `src/services/filters/filterEngine.ts:168`
  `dbMessageToParsedMessage()` reads `body_html`/`body_text` from the messages table, but these are always NULL (bodies live in `bodies.db`). `criteria.body` filter matching will never match. Options: hydrate from body store, move filter evaluation into Rust pipeline, or accept the limitation.

- [ ] **`is_starred` column audit** — `src/services/db/messages.ts:5`
  `DbMessage` uses `SELECT *` which works if schema and interface stay aligned, but there's no compile-time check. Audit the full column list against the interface to catch any mismatches.

- [ ] **Recovery logic duplicated between TS wrapper and Rust** — `src/services/gmail/syncManager.ts:151-163`
  The "delta found 0 + DB has 0 threads → force full resync" recovery is in TS (`syncImapAccountRust`), requiring 2 extra IPC calls. Could move entirely into Rust commands for fewer round-trips.

- [x] ~~**`store_chunk`/`DbInsertData` should move to `pipeline.rs`** — `src-tauri/src/sync/imap_initial.rs`
  Fixed: moved to `pipeline.rs`, both initial and delta sync now import from there.~~

- [x] ~~**Post-sync hooks use dynamic imports unnecessarily** — `src/services/gmail/syncManager.ts:172-193`
  Fixed: converted to static imports for `applyFiltersToNewMessageIds`, `applySmartLabelsToNewMessageIds`, and `categorizeNewThreads`.~~

### LOW

- [ ] **Gmail sync still fully in TS** — `src/services/gmail/syncManager.ts:69`
  `syncGmailAccount` is unchanged. HTTP-based with OAuth coupling, so lower priority, but Gmail accounts don't benefit from Phase 4. Consider porting long-term.

- [ ] **No per-operation timeout on Rust IMAP fetches** — `src-tauri/src/sync/imap_initial.rs`
  Connection timeouts exist via `async-imap`, but long-running fetches on large folders have no operation-level timeout. A 50K-message folder could hang indefinitely.

- [ ] **`ratatoskr-sync-done` event dispatch not verified** — `src/services/gmail/syncManager.ts`
  The Rust path emits `statusCallback?.(accountId, "done")` but other UI side-channel events (`ratatoskr-sync-done` in `App.tsx`) should be verified during integration testing.

- [x] ~~**Phase 6 docs slightly stale** — `docs/rust-core-architecture.md:599`
  Fixed: updated to include categorization commands (5 total).~~

---

## Branding / Assets

- [ ] **Replace logo SVG** — `src/assets/logo.svg` still renders the old "VELO" text as path outlines. Needs a new logo for Ratatoskr.

- [ ] **Replace app icons** — `src-tauri/icons/` contains the old Velo app icons (icon.png, icon.ico, various sizes). Need new Ratatoskr icons for all platforms (macOS .icns, Windows .ico, Linux .png at 32x32, 128x128, 256x256, 512x512).

---

## TypeScript Strictness

- [ ] **39 remaining TS errors** — Mostly from `exactOptionalPropertyTypes` (34 TS2375/TS2379) and other type mismatches (TS2322, TS2345). Decide whether to fix all or relax the option.
