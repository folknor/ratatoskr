# TODO

## Bugs

### HIGH

- [x] ~~**Draft auto-save race condition** — `src/services/composer/draftAutoSave.ts`
  Fixed: `saveDraft()` now reads `activeAccountId` from account store at save time instead of capturing as a closure variable.~~

- [x] ~~**Stale closure in EmailList mapDbThreads** — `src/hooks/useEmailListData.ts:208`
  `mapDbThreads` had empty dependency array `[]`. Added `activeAccountId` to deps.~~

### MEDIUM

- [ ] **15× silent settings failures** — `src/stores/uiStore.ts:94-168`
  All `setSetting().catch(() => {})` silently swallow errors. User preferences lost on DB error with no indication. Add logging or a central persist helper.

- [ ] **Backfill only runs once per app lifetime** — `src/App.tsx:471`
  `backfillDoneRef` is a permanent boolean flag. After re-auth or re-sync, uncategorized threads are never backfilled. Track per-account instead.

- [ ] **Draft null access** — `src/components/layout/EmailList.tsx:181`
  `d.message.id` accessed without checking if `d.message` exists. Runtime error if draft has no message.

- [ ] **Sync queue race** — `src/services/gmail/syncManager.ts:289-317`
  `pendingAccountIds` can lose IDs under rapid concurrent triggers. Needs proper async locking.

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

- [ ] **Date parsing duplicated**
  - `src/services/search/searchParser.ts:28-39` — `parseDateToTimestamp()`
  - `src/services/imap/imapSync.ts:104-120` — `formatImapDate()` / `computeSinceDate()`
  Move to a shared `src/utils/date.ts`.

- [ ] **IMAP messages may skip filter engine**
  `src/services/filters/filterEngine.ts` runs for Gmail sync but appears missing from the IMAP sync flow (`src/services/imap/imapSync.ts`). Verify and add if missing.

- [ ] **Multi-select target resolution duplicated**
  `src/components/ui/ContextMenuPortal.tsx:264-268` and `src/components/layout/EmailList.tsx` both compute target thread IDs from selection. Extract to a shared utility.

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

- [ ] **Rust timeout error messages** — `src-tauri/src/imap/client.rs`
  Same `format!("...timed out after {}s — check your server...")` repeated 10+ times. Create a timeout error helper or macro.

- [ ] **Zustand settings persistence** — `src/stores/uiStore.ts`
  15× identical `setSetting("key", value).catch(() => {})`. Create a `persistSetting()` helper with logging.

- [ ] **ContextMenuPortal batch operations** — `src/components/ui/ContextMenuPortal.tsx:344-422`
  `handleToggleRead`, `handleToggleStar`, `handleTogglePin` all follow the same for-loop-find-toggle pattern. Create a `ThreadBatchOperation` helper.

- [ ] **Unsafe type assertions** — `src/services/email/gmailProvider.ts:140`, `src/utils/crypto.ts:54`, `src/components/ui/ContextMenuPortal.tsx:167-268`
  Multiple `as unknown as` casts and untyped context menu data. Create typed payload interfaces and guards.

---

## Refactoring — State Management

- [ ] **Split uiStore** (17+ properties mixing layout, preferences, and sync state)
  Consider splitting into `uiLayoutStore`, `uiPreferencesStore`, `syncStateStore` to reduce re-render scope.

---

## TypeScript Strictness

- [ ] **39 remaining TS errors** — Mostly from `exactOptionalPropertyTypes` (34 TS2375/TS2379) and other type mismatches (TS2322, TS2345). Decide whether to fix all or relax the option.
