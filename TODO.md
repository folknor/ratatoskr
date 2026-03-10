# TODO

## Bugs

### HIGH

- [ ] **Auto-updater should check local permissions** — Don't show update prompts if the user lacks write access to the app installation directory (e.g., installed system-wide without admin rights). The update would fail anyway — detect this upfront and either hide the prompt or show a helpful message.

---

## Refactoring — Patterns & Boilerplate

- [ ] **`moveToFolder` only adds label, doesn't remove source** — `src-tauri/src/email_actions/commands.rs`

  `email_action_move_to_folder` inserts the target folder label into `thread_labels` but doesn't remove the old label (e.g., INBOX). The TS code had the same behavior, so it's a pre-existing gap — the provider-side IMAP MOVE or Gmail API call handles the actual folder change on the server, but the local DB state shows both labels until the next sync refreshes thread labels.

  In practice this means a thread moved from Inbox to Archive will briefly appear in both views until the next delta sync (≤60s). Not a functional bug since the server state is correct and the UI will self-correct, but it can cause momentary confusion. The fix would be to DELETE the old label in the same transaction, but that requires knowing which label to remove — for archive it's always INBOX, but for arbitrary folder moves the source isn't passed to the command. Would need to either pass the source label as a parameter or query existing labels in the command.

- [x] **~~Unsafe type assertions~~** — Fixed. `gmailProvider.ts` cast eliminated via `GmailRawMessage` type + function overloads on `getMessage()`. `crypto.ts` cast is a genuine TS lib limitation (Uint8Array vs BufferSource generics) — kept with explanatory comment. `ContextMenuPortal.tsx` casts were removed during the earlier component split refactor.

---

## Phase 4 (Rust Sync Engine) Follow-ups

### MEDIUM

- [ ] **Body text unavailable for filter body matching** — `src/services/filters/filterEngine.ts:168`

  `dbMessageToParsedMessage()` reads `body_html`/`body_text` from the `messages` table columns, but these are always NULL — message bodies live in the separate `bodies.db` file (zstd-compressed, accessed via `body_store_get`/`body_store_get_batch`). This means any filter rule with `criteria.body` set will never match, because the body fields on the `DbMessage` object are null.

  This affects the `applyFiltersToNewMessageIds()` path used by the Rust sync engine (`syncManager.ts:182`) and the `applyFiltersToMessages()` path used by TS IMAP delta sync (`imapSync.ts:886`). The Gmail delta sync path (`sync.ts:510`) is not affected because it passes `ParsedMessage` objects that already have bodies populated from the API response before they're stored.

  Options: (1) Hydrate bodies from the body store in `applyFiltersToNewMessageIds()` by calling `bodyStoreGetBatch()` for the message IDs — simplest fix, adds one IPC call but only when filters with body criteria exist. (2) Move filter evaluation into the Rust sync pipeline where bodies are available before compression — more efficient but much larger change. (3) Accept the limitation and document that body-matching filters only work on Gmail API accounts — least effort, but surprising to users. Option 1 is probably the right call.

- [ ] **`DbMessage` / schema column audit** — `src/services/db/messages.ts:5`

  `DbMessage` interface uses `SELECT *` queries, which works as long as the TS interface and SQLite schema stay perfectly aligned, but there's no compile-time or runtime check. If a migration adds/renames/removes a column and the interface isn't updated to match, the mismatch will silently produce undefined values or ignore data. This has already bitten us once with `has_attachments` being missing from the interface.

  The audit should compare every column in the `messages` CREATE TABLE statement (in `migrations.ts`) against the `DbMessage` interface fields, checking types (`number` vs `string` vs `null`), and verifying that Rust's `db_get_messages_for_thread` return type matches too. Could also consider switching from `SELECT *` to explicit column lists to make mismatches a hard error, though that's more verbose. Low-effort task, just needs someone to sit down and diff the schema against the types.

- [x] **~~Recovery logic duplicated between TS wrapper and Rust~~** — Moved into `sync_imap_delta` Rust command (`src-tauri/src/sync/commands.rs`). Recovery (thread count check → clear history_id + folder sync states → initial sync) now happens entirely within the single `sync_imap_delta` invoke, eliminating 3 IPC round-trips. The TS fallback path (`syncManager.ts:282-307`) retains its own recovery since it doesn't use the Rust engine.

### LOW

- [ ] **Gmail sync still fully in TS** — `src/services/gmail/syncManager.ts:80-112`

  `syncGmailAccount()` uses the Gmail REST API via `GmailClient` (HTTP, not IMAP), so it doesn't benefit from the Rust IMAP sync engine at all. The function calls `initialSync()` or `deltaSync()` from `sync.ts`, which make HTTP requests to `googleapis.com` via the Tauri HTTP plugin, parse JSON responses, and store to the TS-side DB layer.

  Porting would mean: (1) implementing Gmail REST API calls in Rust with `reqwest` (threads.list, threads.get, history.list, messages.get), (2) handling OAuth token refresh in Rust (currently `GmailClient` auto-refreshes 5min before expiry), (3) reimplementing the batched thread fetch logic, history-based delta sync, and HISTORY_EXPIRED fallback. This is a large effort because the Gmail API has different semantics from IMAP — it returns threads natively (no JWZ threading needed), uses history IDs instead of UIDs, and requires OAuth2 bearer tokens. The current TS implementation works well and the HTTP overhead dominates anyway (not IPC), so the benefit of porting is minimal. Only worth considering if we want a unified Rust sync pipeline for architectural consistency.

- [ ] **No per-operation timeout on Rust IMAP fetches** — `src-tauri/src/sync/imap_initial.rs`

  Connection-level timeouts exist via `async-imap`'s TCP stream configuration, but there's no operation-level timeout on individual FETCH commands. A folder with 50K+ messages could result in a single IMAP FETCH that takes arbitrarily long — the server might be slow, rate-limiting, or the connection could be half-open (TCP keepalive not triggered yet).

  The risk is a sync that hangs indefinitely on a large folder, blocking the sync timer for that account. Since sync runs sequentially per account (`syncAccountInternal` is awaited), a hang blocks all subsequent accounts too. The fix would be wrapping each `imap_fetch_messages` call (or the entire folder sync) in a `tokio::time::timeout()`. Need to choose a reasonable duration — probably 5-10 minutes per folder, since legitimate large-folder fetches can take a while. The `async-imap` session would need to be dropped on timeout to close the connection cleanly. Low priority because it's a rare edge case (most IMAP servers handle large fetches fine), but it's a robustness improvement for pathological cases.

---

## Branding / Assets

- [ ] **Replace logo SVG** — `src/assets/logo.svg` still renders the old "VELO" text as path outlines. Needs a new logo for Ratatoskr.

- [ ] **Replace app icons** — `src-tauri/icons/`, `assets/icon.png`, `src/assets/logo.svg`, and the inline SVG in `splashscreen.html` all contain old Velo branding. Need new Ratatoskr icons for all platforms (macOS .icns, Windows .ico, Linux .png at 32x32, 128x128, 256x256, 512x512) plus the root asset and splash screen.

---

## TypeScript Strictness

- [ ] **39 remaining TS errors** — Mostly from `exactOptionalPropertyTypes` (34 TS2375/TS2379) and other type mismatches (TS2322, TS2345). Decide whether to fix all or relax the option.
