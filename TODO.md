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

- [x] **Abstract OAuth flow** — OAuth browser/listener handling, provider definitions, token exchange, and provider-specific user-info fetches now live in `src-tauri/src/oauth.rs`, with a standalone `OAuthIdentityProvider` abstraction. `account_commands.rs` now just invokes the flow and persists account state.

- [x] **Split `lib.rs` (715 lines)** — Extracted app-state initialization into `src-tauri/src/app_setup.rs`, tray handling into `src-tauri/src/tray.rs`, and window helpers into `src-tauri/src/window.rs`. `lib.rs` now delegates setup/window/tray responsibilities to those modules.

- [x] **Abstract window/tray management** — Platform-specific tray setup now lives in `src-tauri/src/tray.rs`, and window show/hide/focus helpers now live in `src-tauri/src/window.rs`, reducing `lib.rs` to wiring.

---

## Rust Code Consolidation

> Provider implementations (Gmail, JMAP, Graph, IMAP) have significant duplication.
> Estimated ~800-1200 lines removable.

### High Priority

- [x] **Shared address parsing** — The duplicated "Name <email>" parsing/formatting helpers from `ratatoskr-core::{gmail,imap,graph,jmap}::parse` now live in `ratatoskr-core::provider::email_parsing`.

- [x] **Shared folder role mapping** — The duplicated system folder role mappings for JMAP, Graph, and IMAP now live in `ratatoskr-core::provider::folder_roles` as a shared `SYSTEM_FOLDER_ROLES` table with provider-specific lookup helpers.

- [x] **Shared label/flag extraction** — The shared label assembly logic for JMAP and Graph now lives in `ratatoskr-core::provider::label_flags`, with provider-specific mailbox/folder resolution left in the adapters.

- [x] **Shared attachment deduplication** — The shared dedup/merge mechanics for Gmail and IMAP attachments now live in `ratatoskr-core::provider::attachment_dedup`, with provider-specific key selection left at the call sites.

- [x] **Shared sync progress emission** — Shared sync progress emission now lives in `ratatoskr-core::sync::progress`, with Gmail, JMAP, and Graph sync routing their event payloads through the shared helper.

- [x] **Shared sync state persistence** — Shared sync cursor persistence now lives in `ratatoskr-core::sync::state`, covering Gmail history IDs, JMAP sync state, and Graph delta tokens.

- [x] **Shared message persistence pipeline** — The shared thread aggregate, thread-label replacement, body-store writes, inline-image writes, and search-index writes now live in `ratatoskr-core::sync::persistence`, with provider-specific DB upserts and JMAP inline-fetch handling left in the adapters.

### Medium Priority

- [x] **Shared header extraction** — Shared case-insensitive header lookup now lives in `ratatoskr-core::provider::headers`; Gmail and Graph parsing use it directly. The old IMAP reference here was stale after the core split.

- [x] **Shared thread message lookup** — Shared message/thread lookup helpers now live in `ratatoskr-core::db::lookups`, and the old `jmap/commands.rs` reference in this item was stale after the core/app split.

- [x] **Consolidate base64 utilities** — Shared base64/base64url helpers now live in `ratatoskr-core::provider::encoding`, and the relevant provider/cache/mailer call sites now route through it.

- [x] **Shared pending ops filtering** — Shared pending-operation filtering now lives in `ratatoskr-core::sync::pending`, and Gmail/JMAP/Graph sync now delegate to it.

---

## Rust Code Structure

- [x] **Split `db/queries_extra.rs`** — Core DB extra queries now live under `src-tauri/core/src/db/queries_extra/` as domain modules with a small facade in `queries_extra.rs`.

- [ ] **Split `calendar_commands.rs` (2083 lines)** — Could be split by provider or concern (CalDAV sync, event parsing, recurrence handling).

- [ ] **Split `account_commands.rs` (~600 lines)** — OAuth flow internals moved to `src-tauri/src/oauth.rs`, but account CRUD and provider initialization still live together in one large command module.

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
