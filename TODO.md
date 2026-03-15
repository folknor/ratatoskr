# TODO

## Migration Backlog

### AI Migration

- [x] **Port AI inference execution to Rust** — Done: `core/src/ai/` module with prompts, types, `AiCompleter` trait, 11 orchestration functions (summarize, smart replies, ask inbox, categorize, smart labels, extract task, transform, compose, reply, writing style, auto-draft), defensive response parsers, DB caching integration. App crate provides the `AiCompleter` implementation with HTTP calls.

### Regression Coverage

- [ ] **Expand regression coverage around migrated sync/bootstrap behavior** — Add focused tests for sync status events, background sync start/stop, post-sync hook triggering, and account bootstrap paths that now rely on Rust-backed summary DTOs.

- [ ] **Replace the magic microtask loop in `flushListenerSetup`** — The current 8-iteration `await Promise.resolve()` loop is brittle and hides ordering assumptions in sync listener tests.

## Inline Image Store Eviction

- [ ] **Wire up user-configurable eviction for `inline_images.db`** — The Rust backend has the building blocks (`prune_to_size()`, `delete_unreferenced()`, `stats()`, `clear()`), but the TS/UI side has no settings or scheduled eviction plumbing.

  **What's missing**:
  1. **Settings UI**: No user-facing control for inline image store size. The 128 MB cap is hardcoded in Rust.
  2. **Scheduled eviction**: No periodic sweep to catch edge cases (e.g., if `MAX_INLINE_STORE_BYTES` is lowered in a future update).

## Iced Rewrite

- [ ] **Investigate iced ecosystem projects** — Review these repos for patterns, widget implementations, and architecture ideas:
  - https://github.com/hecrj/iced_fontello — Icon font integration for iced
  - https://github.com/hecrj/iced_palace — Hecrj's iced showcase/playground
  - https://github.com/pop-os/cosmic-edit — COSMIC text editor (large real-world iced app)
  - https://github.com/pop-os/iced/blob/master/widget/src/markdown.rs — COSMIC fork's markdown widget

- [ ] **Persist window state across restarts** — Remember window size, position, maximized state, and which monitor the window was on. Restore on next launch. Likely serialize to a small JSON/TOML file in the app data dir.

- [ ] **Per-pane minimum resize limits** — PaneGrid currently uses a uniform `min_size(120)` for all panes. Should have per-pane minimums (e.g., sidebar can't go below 150px, thread list below 200px, contact sidebar below 180px). Requires clamping ratios in the `PaneResized` handler since PaneGrid only supports a single global minimum. Decide on actual values after visual testing.

- [ ] **Animated toggler widget** — Port libcosmic's slerp-based toggle animation for smooth sliding pill togglers. Current iced built-in toggler snaps instantly. libcosmic's version (`research/libcosmic/src/widget/toggler.rs`) uses `anim::slerp()` with configurable duration (200ms default), interpolating knob position per-frame via `RedrawRequested`. ~150-200 LOC to port.

- [ ] **`responsive` for adaptive layout** — Wrap PaneGrid in `iced::widget::responsive` to collapse panels at narrow window sizes (e.g., hide contact sidebar below 900px, stack sidebar over thread list below 600px).

## Non-Migration Cleanup

### Branding

- [ ] **Replace logo SVG** — `src/assets/logo.svg` still renders the old "VELO" text as path outlines. Needs a new logo for Ratatoskr.

- [ ] **Replace app icons** — `src-tauri/icons/`, `assets/icon.png`, `src/assets/logo.svg`, and the inline SVG in `splashscreen.html` still contain old Velo branding. Need new Ratatoskr icons for all platforms.

### Code Quality

- [ ] **Decide whether Graph `raw_size = 0` should stay accepted** — Graph still lacks a clean size field for the current query path. Either keep this as an accepted cosmetic limitation or document a better fallback if one exists.

### Microsoft Graph

- [ ] **Ship a default Microsoft OAuth client ID** — Register a multi-tenant Azure AD app ("Accounts in any organizational directory and personal Microsoft accounts"), set as public client (no client secret), configure `http://localhost` redirect URI, request Mail.ReadWrite/Mail.Send/etc. scopes. Ship the client ID as a constant in `oauth.rs`. Then remove the per-account credential UI (the "Update OAuth App" flow in settings that asks users for client_id/client_secret) — users should never see this. Keep the per-account `oauth_client_id` DB column as an optional override for enterprise users who need to use their own tenant-restricted app.

### JMAP

- [ ] **JMAP for Calendars** — `jmap-client` has no calendar support (upstream Issue #3). Blocked until `jmap-client` adds calendar types. Low priority — CalDAV covers calendar sync for now.

## Roadmap — Backend-Only Work

Items below are derived from `docs/roadmap/` and scoped to Rust backend work only (no UI/frontend). See the individual roadmap docs for full context.

### IMAP CONDSTORE/QRESYNC (Tier 1)

- [ ] **QRESYNC VANISHED parsing (Phase 3)** — Send `ENABLE QRESYNC` via raw command, then `SELECT mailbox (QRESYNC (<uidvalidity> <modseq> [<known-uids>]))`. Parse `VANISHED (EARLIER) <uid-set>` untagged responses. Blocked on async-imap CHANGEDSINCE support (Issue #130).

### Public Folders (Tier 1)

- [x] **IMAP NAMESPACE-based public folder access** — Done: `imap/public_folders.rs` discovers shared folders via NAMESPACE, checks permissions via MYRIGHTS, syncs messages via SELECT/FETCH into `public_folder_items`.
