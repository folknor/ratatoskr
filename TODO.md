# TODO

## Iced Migration Prep

> The project is moving from Tauri (Rust+TS) to a pure Rust stack using iced for UI.
> These tasks prepare the Rust codebase for that transition.

### Phase 1: Extract Portable Core ✅

- [x] **Introduce `ProgressReporter` trait** — Replaced all `app.emit()` calls with trait-based `&dyn ProgressReporter`. `TauriProgressReporter` wraps `AppHandle::emit()` at command boundaries. Core trait lives in `ratatoskr-core::progress`.

- [x] **Decouple `attachment_cache.rs`** — Changed from `&AppHandle` to `&Path` (app_data_dir). Made `DbState`, `BodyStoreState`, `InlineImageStoreState`, `SearchState`, `AppCryptoState` all `Clone`.

- [x] **Extract `ratatoskr-core` crate** — 21.6k lines of framework-agnostic logic: all 4 providers (gmail, jmap, graph, imap), sync engine, threading, filters, smart labels, categorization, discovery, email actions, SMTP, DB core, body/inline-image/search stores, attachment cache, `ProgressReporter` trait. App crate (16.5k lines) retains Tauri command wrappers and `TauriProgressReporter`. App `mod.rs` files re-export from core via `pub use ratatoskr_core::{module}::*;`.

### Phase 1.5: Remaining Decoupling

- [x] **`AppState` aggregate (plan Step 5)** — Added `AppState`/`ProviderStates`, made provider states and `SyncQueueState` cloneable, and rewired `sync/commands.rs` background/manual sync paths to pass cloned state instead of calling `app.state::<T>()` inside spawned tasks.

- [x] **`ProviderRegistry` trait (plan Step 6)** — Added `provider/registry.rs`, implemented it for `ProviderStates`, and switched the sync auto path in `provider/commands.rs` to resolve providers through the registry.

- [x] **Finish registry refactor outside sync path** — `filters/commands.rs`, `smart_labels/commands.rs`, and the remaining non-sync provider commands now resolve providers through `ProviderRegistry` / `AppState`, and `provider/router.rs::get_ops()` has been removed.

- [x] **Split `db/queries.rs` commands from logic** — Pure DB logic for `db/queries.rs`, `db/queries_extra.rs`, and `db/pending_ops.rs` now lives in `ratatoskr-core::db::{queries,queries_extra,pending_ops}` with thin Tauri wrappers retained in the app crate.

- [x] **Clean up post-refactor Rust warnings** — Removed the tracked warnings from `src-tauri/src/sync/mod.rs`, `src-tauri/src/progress.rs`, `src-tauri/src/provider/registry.rs`, and put `AppState::app_data_dir` into use in attachment fetch flow.

### Phase 2: Decouple Tauri-Specific Concerns

- [ ] **Abstract OAuth flow** — The browser/listener authorization-code flow now lives in `src-tauri/src/oauth.rs`, but `account_commands.rs` still owns provider-specific token/user-info handling. Finish this with a standalone `OAuthProvider` trait or equivalent provider abstraction.

- [x] **Split `lib.rs` (715 lines)** — Extracted app-state initialization into `src-tauri/src/app_setup.rs`, tray handling into `src-tauri/src/tray.rs`, and window helpers into `src-tauri/src/window.rs`. `lib.rs` now delegates setup/window/tray responsibilities to those modules.

- [x] **Abstract window/tray management** — Platform-specific tray setup now lives in `src-tauri/src/tray.rs`, and window show/hide/focus helpers now live in `src-tauri/src/window.rs`, reducing `lib.rs` to wiring.

---

## Rust Code Consolidation

> Provider implementations (Gmail, JMAP, Graph, IMAP) have significant duplication.
> Estimated ~800-1200 lines removable.

### High Priority

- [ ] **Shared address parsing** — `gmail/parse.rs:118-139`, `imap/parse.rs:335-370`, `graph/parse.rs:210-226`, `jmap/parse.rs:187-203` all implement the same "Name \<email\>" parsing and formatting. Consolidate into `provider/email_parsing.rs`.

- [ ] **Shared folder role mapping** — `jmap/mailbox_mapper.rs:4-77`, `graph/folder_mapper.rs:7-124`, `imap/ops.rs:22-42` all map well-known folder roles (inbox, sent, trash, junk, drafts, archive) to canonical names. Create a shared `SYSTEM_FOLDER_ROLES` constant.

- [ ] **Shared label/flag extraction** — `jmap/mailbox_mapper.rs:51-77` and `graph/folder_mapper.rs:78-107` have ~95% identical logic for extracting labels from folder membership + keywords/flags. Unify with provider-specific adapters.

- [ ] **Shared attachment deduplication** — `gmail/parse.rs:172-201` (`dedup_by_attachment_id`) and `imap/parse.rs:286-318` (`dedup_attachments_by_hash`) do nearly identical dedup-and-merge logic. Extract to `provider/attachment_dedup.rs` with a generic key trait.

- [ ] **Shared sync progress emission** — `gmail/sync.rs`, `jmap/sync.rs`, `graph/sync.rs` each have `emit_progress()` with identical structure (`account_id`, `phase`, `current`, `total`). Consolidate into `sync/progress.rs`. (Overlaps with `ProgressReporter` trait above.)

- [ ] **Shared sync state persistence** — `gmail/sync.rs` (`update_account_history_id`), `jmap/sync.rs` (`save_sync_state`), `graph/sync.rs` (`save_delta_token`) all save/restore sync cursors with nearly identical transaction patterns. Create generic `sync/state.rs` with `save_sync_state()` / `load_sync_state()`.

- [ ] **Shared message persistence pipeline** — All 3 sync modules follow the same pattern: filter pending ops → upsert messages → update thread aggregates → set thread labels → write to body store → index for search. Consolidate into `sync/persistence.rs`. (~300 lines duplication.)

### Medium Priority

- [ ] **Shared header extraction** — `gmail/parse.rs:111-116`, `graph/parse.rs:198-208`, `imap/parse.rs:320-332` all extract headers by name (case-insensitive) from a header collection. Shared helper.

- [ ] **Shared thread message lookup** — `graph/ops.rs:482-501`, `imap/ops.rs:75-109`, `jmap/commands.rs:280-295` all query message IDs for a thread. Move the SQL query to a shared DB helper.

- [ ] **Consolidate base64 utilities** — `gmail/parse.rs:262-269` (URL_SAFE_NO_PAD), `attachment_cache.rs:71-82` (STANDARD), `graph/parse.rs:228-236` (STANDARD) — different wrappers with different error handling. Consolidate into `provider/encoding.rs`.

- [ ] **Shared pending ops filtering** — `jmap/sync.rs:245,317` and `graph/sync.rs:317` both call `filter_pending_ops()` defined separately in each sync module. Move to `db/` or `email_actions/`.

---

## Rust Code Structure

- [ ] **Split `db/queries_extra.rs` (5128 lines)** — Largest file in the codebase. Split by domain (calendar queries, contact queries, draft queries, etc.).

- [ ] **Split `calendar_commands.rs` (2083 lines)** — Could be split by provider or concern (CalDAV sync, event parsing, recurrence handling).

- [ ] **Split `account_commands.rs` (~600 lines)** — The generic OAuth browser/listener flow moved into `src-tauri/src/oauth.rs`, but provider-specific OAuth/token/user-info code is still mixed with account CRUD and provider initialization.

- [ ] **Audit `.unwrap()` in `calendar_commands.rs`** — Clippy denies `unwrap_used` project-wide but there may be an instance that slipped through. Verify and convert to `?` or `.unwrap_or()`.

---

## Security & Data Safety

- [ ] **Decryption failure fallback returns plaintext** — `src/services/db/accounts.ts:40-81` — When decryption fails, code falls back to the raw (potentially plaintext) value with only `console.warn`. Credentials stored before encryption was enabled remain accessible in plaintext indefinitely. *(LOW)*

- [ ] **`decrypt_if_needed` silently returns ciphertext on failure** — `src-tauri/src/imap/account_config.rs:51-58` — If decryption fails, returns the encrypted blob as the IMAP password, causing a confusing auth failure. Should return `Err` instead. *(LOW)*

- [ ] **Draft auto-save has no crash-recovery guarantee** — `src/services/composer/draftAutoSave.ts` — 3-second debounce means up to 3s of content lost on crash. Combined with `synchronous=NORMAL`, even locally-persisted drafts might not survive power failure. *(LOW)*

---

## Provider Operations

- [ ] **Snippet fallback truncation not grapheme-safe** — `imap_message_to_provider_message` uses `.chars().take(200).collect()` which can split multi-byte grapheme clusters. Minor cosmetic issue. *(LOW)*

---

## Post-Sync Hooks

> **Systemic issue**: Rust sync now owns filters, smart labels, calendar follow-up, notification evaluation, and AI categorization preparation/application. The remaining Rust/TS boundary is mainly desktop notification display/action handling via the notification service.

---

## AI Service

- [ ] **Duplicate `callAi` wrapper in two services** — Both `aiService.ts` and `writingStyleService.ts` define identical `callAi(systemPrompt, userContent)` wrappers. Callers could use `completeAi` directly or share a single wrapper. *(LOW)*

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
