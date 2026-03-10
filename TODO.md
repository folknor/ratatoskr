# TODO

## Bugs

### HIGH

- [ ] **Auto-updater should check local permissions** — Don't show update prompts if the user lacks write access to the app installation directory (e.g., installed system-wide without admin rights). The update would fail anyway — detect this upfront and either hide the prompt or show a helpful message.

### MEDIUM

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

- [ ] **IMAP messages may skip filter engine**
  `src/services/filters/filterEngine.ts` runs for Gmail sync but appears missing from the IMAP sync flow (`src/services/imap/imapSync.ts`). Verify and add if missing.

---

## Refactoring — Large Files

- [ ] **ContextMenuPortal.tsx** (796 lines) — Extract per-menu-type components (ThreadContextMenu, MessageContextMenu, SidebarLabelContextMenu). Move quote builders to `utils/emailQuoteBuilders.ts`.

- [ ] **Composer.tsx** (691 lines) — Extract template shortcut engine and editor config setup.

---

## Refactoring — Patterns & Boilerplate

- [ ] **`moveToFolder` only adds label, doesn't remove source** — `src-tauri/src/email_actions/commands.rs`
  `email_action_move_to_folder` inserts the target folder label but doesn't remove the old label (e.g., INBOX). The TS code had the same behavior, so it's a pre-existing gap — the provider-side move handles the actual folder change, but the local DB state is incomplete until next sync.

- [ ] **Unsafe type assertions** — `src/services/email/gmailProvider.ts:140`, `src/utils/crypto.ts:54`, `src/components/ui/ContextMenuPortal.tsx:167-268`
  Multiple `as unknown as` casts and untyped context menu data. Create typed payload interfaces and guards.

---

## Refactoring — State Management

- [ ] **Split uiStore** (17+ properties mixing layout, preferences, and sync state)
  Consider splitting into `uiLayoutStore`, `uiPreferencesStore`, `syncStateStore` to reduce re-render scope.

---

## Phase 4 (Rust Sync Engine) Follow-ups

### MEDIUM

- [ ] **Body text unavailable for filter body matching** — `src/services/filters/filterEngine.ts:168`
  `dbMessageToParsedMessage()` reads `body_html`/`body_text` from the messages table, but these are always NULL (bodies live in `bodies.db`). `criteria.body` filter matching will never match. Options: hydrate from body store, move filter evaluation into Rust pipeline, or accept the limitation.

- [ ] **`is_starred` column audit** — `src/services/db/messages.ts:5`
  `DbMessage` uses `SELECT *` which works if schema and interface stay aligned, but there's no compile-time check. Audit the full column list against the interface to catch any mismatches.

- [ ] **Recovery logic duplicated between TS wrapper and Rust** — `src/services/gmail/syncManager.ts:151-163`
  The "delta found 0 + DB has 0 threads → force full resync" recovery is in TS (`syncImapAccountRust`), requiring 2 extra IPC calls. Could move entirely into Rust commands for fewer round-trips.

### LOW

- [ ] **Gmail sync still fully in TS** — `src/services/gmail/syncManager.ts:69`
  `syncGmailAccount` is unchanged. HTTP-based with OAuth coupling, so lower priority, but Gmail accounts don't benefit from Phase 4. Consider porting long-term.

- [ ] **No per-operation timeout on Rust IMAP fetches** — `src-tauri/src/sync/imap_initial.rs`
  Connection timeouts exist via `async-imap`, but long-running fetches on large folders have no operation-level timeout. A 50K-message folder could hang indefinitely.

- [ ] **`ratatoskr-sync-done` event dispatch not verified** — `src/services/gmail/syncManager.ts`
  The Rust path emits `statusCallback?.(accountId, "done")` but other UI side-channel events (`ratatoskr-sync-done` in `App.tsx`) should be verified during integration testing.

---

## Branding / Assets

- [ ] **Replace logo SVG** — `src/assets/logo.svg` still renders the old "VELO" text as path outlines. Needs a new logo for Ratatoskr.

- [ ] **Replace app icons** — `src-tauri/icons/`, `assets/icon.png`, `src/assets/logo.svg`, and the inline SVG in `splashscreen.html` all contain old Velo branding. Need new Ratatoskr icons for all platforms (macOS .icns, Windows .ico, Linux .png at 32x32, 128x128, 256x256, 512x512) plus the root asset and splash screen.

---

## TypeScript Strictness

- [ ] **39 remaining TS errors** — Mostly from `exactOptionalPropertyTypes` (34 TS2375/TS2379) and other type mismatches (TS2322, TS2345). Decide whether to fix all or relax the option.
