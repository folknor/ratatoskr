# TODO

## Bugs

### HIGH

- [ ] **Auto-updater should check local permissions** — Don't show update prompts if the user lacks write access to the app installation directory (e.g., installed system-wide without admin rights). The update would fail anyway — detect this upfront and either hide the prompt or show a helpful message.

---

## Security & Data Safety

### HIGH

- [ ] **`withSerializedExecution` has no real SQL transaction** — `src/services/db/connection.ts:51-78`

  Serializes operations via a JS promise queue but explicitly does NOT use `BEGIN`/`COMMIT`/`ROLLBACK` (comment on line 70-73 explains tauri-plugin-sql pool constraint). If the app crashes mid-"transaction", partial writes persist. For example, during IMAP sync, a crash after `upsertThread` but before `upsertMessage` leaves an empty thread. `setThreadLabels` (DELETE-then-INSERT pattern) can lose all labels on a crash between the two statements.

  Fix: Use `SAVEPOINT`/`RELEASE` if the pool issue is specifically with nested transactions. Or move critical multi-step writes to Rust-side `DbState::with_conn` where real transactions are available.

### MEDIUM

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

## Phase 3b (Graph Provider) Known Issues

- [ ] **Category add/remove is racy** — `src-tauri/src/graph/ops.rs`

  `add_category`/`remove_category` do a read-then-write. Two concurrent actions could clobber each other. Graph has no atomic "add to array" operation — unavoidable.

- [ ] **No `$batch` optimization for thread actions** — `src-tauri/src/graph/ops.rs`

  Thread-level actions loop per-message. Batching up to 20 per `/$batch` call would be faster.

- [ ] **`raw_size` is always 0 for Graph messages** — `src-tauri/src/graph/sync.rs`

  Graph API has no first-class size property. `PidTagMessageSize` can't combine with `$select`. Accepted cosmetic limitation.

---

## TypeScript Strictness

- [ ] **39 remaining TS errors** — Mostly from `exactOptionalPropertyTypes` (34 TS2375/TS2379) and other type mismatches (TS2322, TS2345). Decide whether to fix all or relax the option.
