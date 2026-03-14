# TODO

## Migration Backlog

### AI Migration

- [ ] **Port AI inference execution to Rust** — Rust already owns provider/runtime/config selection. TypeScript still owns prompt assembly and actual inference calls for summaries, smart replies, transforms, ask-inbox, task extraction, smart-label AI, category inference, and auto-drafts. All of this needs to move to the core crate as part of the iced migration.

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

- [ ] **IMAP NAMESPACE-based public folder access** — For non-Exchange IMAP servers (Dovecot, Cyrus), discover public namespaces via the `NAMESPACE` command and `LIST` folders under the public prefix. Access with standard IMAP `SELECT`/`FETCH`. The `namespace_type` column already exists on the labels table (migration v54).
