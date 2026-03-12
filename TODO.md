# TODO

## Migration Backlog

### AI Boundary

- [ ] **Decide whether AI inference execution should move to Rust** — Rust already owns provider/runtime/config selection, but TypeScript still owns prompt assembly and actual inference calls for summaries, smart replies, transforms, ask-inbox, task extraction, smart-label AI, category inference, and auto-drafts. This needs an explicit boundary decision, not ad-hoc drift.

- [ ] **Deduplicate the shared `callAi` wrapper** — `aiService.ts` and `writingStyleService.ts` still define the same `callAi(systemPrompt, userContent)` helper. If inference remains in TypeScript, this should collapse to one shared wrapper or direct `completeAi` use.

### Post-Sync Boundary

> Rust sync now owns filters, smart labels, calendar follow-up, notification evaluation, and AI categorization preparation/application.
> The remaining Rust/TS boundary is mainly desktop notification display plus actual AI inference calls still triggered from the frontend.

- [ ] **Trim `syncManager.ts` down to a deliberate UI boundary** — Keep only event subscription, UI progress shaping, and notification display in TypeScript. Any remaining policy logic should move to Rust or be removed.

### Settings and Account Compatibility Sweeps

- [x] **Stop decrypting every setting in `read_setting_map`** — Settings snapshots now decrypt only the small secure-key subset they actually need, while UI/non-sensitive bootstrap reads stay plain.

- [x] **Stop bundling API keys with non-sensitive settings snapshots** — Non-sensitive settings bootstrap data and secrets now come from separate snapshot commands, so only the settings page requests API keys/client secrets.

- [ ] **Sweep remaining full account/settings compatibility reads** — Continue replacing one-off `getAccount()` / `getSetting()` reads and legacy full-row helpers with narrow Rust DTOs in active paths such as `src/services/db/accounts.ts`, `src/services/db/settings.ts`, and `src/services/gmail/tokenManager.ts`.

### Regression Coverage

- [ ] **Expand regression coverage around migrated sync/bootstrap behavior** — Add focused tests for sync status events, background sync start/stop, post-sync hook triggering, and account bootstrap paths that now rely on Rust-backed summary DTOs.

- [ ] **Replace the magic microtask loop in `flushListenerSetup`** — The current 8-iteration `await Promise.resolve()` loop is brittle and hides ordering assumptions in sync listener tests.

## Non-Migration Cleanup

### Branding

- [ ] **Replace logo SVG** — `src/assets/logo.svg` still renders the old "VELO" text as path outlines. Needs a new logo for Ratatoskr.

- [ ] **Replace app icons** — `src-tauri/icons/`, `assets/icon.png`, `src/assets/logo.svg`, and the inline SVG in `splashscreen.html` still contain old Velo branding. Need new Ratatoskr icons for all platforms.

### Code Quality

- [ ] **Category add/remove is racy** — `src-tauri/src/graph/ops.rs` does read-then-write for categories. Two concurrent actions can clobber each other. Graph has no atomic array-update primitive, so this likely needs client-side locking if we want to address it.

- [ ] **Add Graph `$batch` optimization for thread actions** — Thread-level Graph actions still loop per message. Batching up to 20 operations per `/$batch` call would reduce request count.

- [ ] **Decide whether Graph `raw_size = 0` should stay accepted** — Graph still lacks a clean size field for the current query path. Either keep this as an accepted cosmetic limitation or document a better fallback if one exists.

- [ ] **Deduplicate account-to-store mapping in the React entry points** — `App.tsx`, `ComposerWindow.tsx`, and `ThreadWindow.tsx` still duplicate the same `dbAccounts.map(...)` shaping logic.
