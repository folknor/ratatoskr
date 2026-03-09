# TODO

## Bugs

### HIGH

- [ ] **Draft auto-save race condition** — `src/services/composer/draftAutoSave.ts:15-75`
  `currentAccountId` captured as closure variable. If user switches accounts during the 3s debounce, draft saves to wrong account. Fix: use a ref or read account at save time.

- [ ] **Stale closure in EmailList mapDbThreads** — `src/components/layout/EmailList.tsx:356`
  `mapDbThreads` has empty dependency array `[]` but uses `activeAccountId`. Thread metadata may be fetched for wrong account.

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

---

## Duplicated Business Logic

### CRITICAL

- [ ] **Thread label ops implemented in 3 places**
  - `src/services/emailActions.ts:162-225` — direct `DELETE/INSERT FROM thread_labels`
  - `src/services/snooze/snoozeManager.ts:32-67` — direct label manipulation for snooze
  - `src/services/db/threads.ts:122-140` — `setThreadLabels()` function
  Consolidate to `setThreadLabels()` in `db/threads.ts` as the canonical implementation.

- [ ] **Snooze bypasses offline action queue**
  `src/services/snooze/snoozeManager.ts` does direct DB label manipulation instead of going through `emailActions`. Won't work offline, won't sync to provider. Refactor to use `emailActions.removeThreadLabel()` / `addThreadLabel()`.

- [ ] **Pin/unpin not offline-safe**
  `src/services/db/threads.ts:209-229` and `src/services/quickSteps/executor.ts:68-84` call direct DB updates, bypassing the offline queue system that archive/trash/star use. Add pin/unpin wrappers in `emailActions.ts`.

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

- [ ] **SettingsPage.tsx** (2992 lines) — Extract each tab into its own component (GeneralSettingsTab, AISettingsTab, ComposingSettingsTab, etc.). 65+ useState hooks could become a single settings state object.

- [ ] **imapSync.ts** (1209 lines) — Split into phases: folder discovery, message fetch (with circuit breaker), JWZ threading, DB storage.

- [ ] **EmailList.tsx** (1045 lines) — Separate list orchestration, pagination/virtualization, and search/filter logic.

- [ ] **AddImapAccount.tsx** (1005 lines) — Extract each wizard step (basic, IMAP config, SMTP config, connection test) into its own component. Move OAuth discovery logic to a helper.

- [ ] **ContextMenuPortal.tsx** (796 lines) — Extract per-menu-type components (ThreadContextMenu, MessageContextMenu, SidebarLabelContextMenu). Move quote builders to `utils/emailQuoteBuilders.ts`.

- [ ] **Composer.tsx** (691 lines) — Extract template shortcut engine and editor config setup.

---

## Refactoring — Patterns & Boilerplate

- [ ] **Rust IMAP session boilerplate** — `src-tauri/src/commands.rs`
  8 command functions with identical connect → work → logout pattern. Create a `with_imap_session()` helper.

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

- [ ] **52 remaining TS errors** — Mostly from `exactOptionalPropertyTypes` (34 TS2375/TS2379) and other type mismatches (TS2322, TS2345). Decide whether to fix all or relax the option.
