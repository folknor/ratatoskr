# TODO

## Security & Data Safety

- [ ] **Decryption failure fallback returns plaintext** — `src/services/db/accounts.ts:40-81` — When decryption fails, code falls back to the raw (potentially plaintext) value with only `console.warn`. Credentials stored before encryption was enabled remain accessible in plaintext indefinitely. *(LOW)*

- [ ] **`decrypt_if_needed` silently returns ciphertext on failure** — `src-tauri/src/imap/account_config.rs:51-58` — If decryption fails, returns the encrypted blob as the IMAP password, causing a confusing auth failure. Should return `Err` instead. Same pattern as TS-side decryption fallback above. *(LOW)*

- [ ] **Draft auto-save has no crash-recovery guarantee** — `src/services/composer/draftAutoSave.ts` — 3-second debounce means up to 3s of content lost on crash. Combined with `synchronous=NORMAL`, even locally-persisted drafts might not survive power failure. *(LOW)*

---

## OAuth & Account Creation

---

## Provider Operations



- [ ] **Snippet fallback truncation not grapheme-safe** — `imap_message_to_provider_message` uses `.chars().take(200).collect()` which can split multi-byte grapheme clusters. Minor cosmetic issue. *(LOW)*

---

## Sync Engine



---

## Post-Sync Hooks

> **Systemic issue**: Rust sync now owns filters, smart labels, calendar follow-up, notification evaluation, and AI categorization preparation/application. The remaining Rust/TS boundary is mainly desktop notification display/action handling via the notification service.

---

## AI Service

- [ ] **Duplicate `callAi` wrapper in two services** — Both `aiService.ts` and `writingStyleService.ts` define identical `callAi(systemPrompt, userContent)` wrappers. Callers could use `completeAi` directly or share a single wrapper. *(LOW)*

---

## Cache & Inline Images

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
