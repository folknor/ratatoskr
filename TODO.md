# TODO

## Bugs

### HIGH

- [ ] **Auto-updater should check local permissions** — Don't show update prompts if the user lacks write access to the app installation directory (e.g., installed system-wide without admin rights). The update would fail anyway — detect this upfront and either hide the prompt or show a helpful message.

- [ ] **App not killed when main window is closed** — Closing the main window doesn't terminate the process. Investigate: likely minimize-to-tray or `on_window_event` handler preventing exit. May be related to single-instance plugin or background sync tasks keeping the runtime alive.

- [ ] **Remove "launch at login" feature** — Remove the UI option, the Rust `auto-launch` crate dependency, and any related Tauri plugin/capability config. Not needed.

- [ ] **Remove "reduce motion" setting** — Remove the UI option and any associated CSS/animation logic. Not needed.

- [ ] **"Undo send" delay needs a disable option** — Currently forced on with no way to turn it off. Add a "None" / 0s option to the delay picker.

---

## Security & Data Safety

### LOW

- [ ] **Decryption failure fallback returns plaintext** — `src/services/db/accounts.ts:40-81`

  When decryption fails, code falls back to the raw (potentially plaintext) value with only `console.warn`. Credentials stored before encryption was enabled remain accessible in plaintext indefinitely.

- [ ] **Draft auto-save has no crash-recovery guarantee** — `src/services/composer/draftAutoSave.ts`

  3-second debounce means up to 3s of content lost on crash. Combined with `synchronous=NORMAL`, even locally-persisted drafts in `local_drafts` might not survive power failure.

---

## Cache & Inline Image Store

- [ ] **Cache eviction not implemented** — `remove_cached` and `count_by_hash` in `attachment_cache.rs` exist but nothing calls them. The UI has a cache size setting but no code enforces it. Old cached attachments accumulate forever on disk.

- [ ] **Inline image store has no size limit** — `inline_images.db` grows unbounded. No eviction, no cap. Heavy users with lots of signature images will see this grow indefinitely.

- [ ] **Non-IMAP providers don't get inline images during sync** — IMAP stores inline images proactively at sync time. Gmail/JMAP/Graph only store them reactively on first fetch via `cache_after_fetch` in `provider/commands.rs`. First render of every email with inline images is slow for those providers.

- [ ] **`gmail_attachment_id` column naming** — `find_cache_info` in `attachment_cache.rs` queries `gmail_attachment_id` for all providers. For IMAP, the `part_id` is stored in that column. Works, but the name is misleading.

- [ ] **`CacheInfo.local_path` fetched but never read** — `try_cache_hit` in `provider/commands.rs` uses `content_hash` → `read_cached` (which resolves the path itself), so the stored `local_path` in `CacheInfo` is redundant.

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

## Autodiscovery Follow-ups

- [ ] **App-specific password help links** — Providers like iCloud require app-specific passwords. Add a `help_url` field to `ProtocolOption` in `discovery/types.rs`, populate it for iCloud (`https://support.apple.com/en-us/102654`) and similar providers in the registry, surface it in the TS `WellKnownProviderResult`, and show a hint/link in the account setup UI when present.

---

## Phase 3b (Graph Provider) Known Issues

- [ ] **Category add/remove is racy** — `src-tauri/src/graph/ops.rs`

  `add_category`/`remove_category` do a read-then-write. Two concurrent actions could clobber each other. Graph has no atomic "add to array" operation — unavoidable.

- [ ] **No `$batch` optimization for thread actions** — `src-tauri/src/graph/ops.rs`

  Thread-level actions loop per-message. Batching up to 20 per `/$batch` call would be faster.

- [ ] **`raw_size` is always 0 for Graph messages** — `src-tauri/src/graph/sync.rs`

  Graph API has no first-class size property. `PidTagMessageSize` can't combine with `$select`. Accepted cosmetic limitation.
