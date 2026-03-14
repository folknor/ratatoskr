# Ratatoskr

**Subagents must always be launched in the foreground** (never use `run_in_background: true`) so the user can approve tool requests.

Tauri v2 desktop email client migrating to pure Rust (iced UI). Cargo workspace with two crates:

- **`ratatoskr-core`** (`src-tauri/core/`, ~22k lines) — Framework-agnostic business logic: providers, sync, threading, filters, search, DB, etc.
- **`ratatoskr`** (`src-tauri/src/`, ~16.5k lines) — Tauri app shell: `#[tauri::command]` wrappers, `TauriProgressReporter`, window/tray management.
- **Frontend** — React 19 + TypeScript (~73k lines), to be replaced by iced.
- **`squeeze`** (`squeeze/`) — Standalone attachment compression crate (CLI + library). NOT a workspace member — compiles independently to avoid adding to main build time. Compresses images (mozjpeg-rs + oxipng), PDFs (lopdf image recompression + `save_modern` structural compression), and OOXML/ODF documents (ZIP archive image compression). Designed for later integration via `squeeze = { path = "squeeze", optional = true }` with a feature flag. Run `cargo check` / `cargo test` from inside `squeeze/`.

## Commands

- `pnpm exec biome check --write` — lint+format (Biome, not ESLint/Prettier)
- `bunx tauri dev` — run dev (uses bun for tauri CLI, pnpm for deps)
- `vitest run` — tests; `vitest run path/to/file` for single file
- `cargo check --workspace` — check both Rust crates
- `cargo check -p ratatoskr-core` — check core crate only

## Crate Architecture

App module `mod.rs` files re-export from core: `pub use ratatoskr_core::{module}::*;` + `pub mod commands;`. The `commands.rs` files stay in the app crate with Tauri-specific imports.

**`ProgressReporter` trait** (`ratatoskr_core::progress`) — All event emission goes through `&dyn ProgressReporter`. App crate provides `TauriProgressReporter` (wraps `AppHandle::emit()`). Future iced frontend will provide its own implementation.

**State types are `Clone`** — `DbState`, `BodyStoreState`, `InlineImageStoreState`, `SearchState`, `AppCryptoState` all wrap `Arc<Mutex<Connection>>` or similar and implement `Clone`.

**Scoped queries** (`core/src/db/queries_extra/scoped_queries.rs`) — Cross-account query infrastructure. `AccountScope` enum (`Single`/`Multiple`/`All`) controls which accounts a query spans. Predicate-based virtual folder queries for Starred/Snoozed use boolean flags on `threads`, not label joins. Draft counts include `local_drafts` table.

**Navigation state** (`core/src/db/queries_extra/navigation.rs`) — `get_navigation_state()` returns the full sidebar state in one call: universal folders (Inbox, Starred, Snoozed, Sent, Drafts, Trash) with unread counts, smart folders, and per-account labels when scoped. Smart folder and per-label unread counts are scaffolded (return 0).

**Smart folder engine** (`core/src/smart_folder/`) — Rust port of the TypeScript smart folder query pipeline: query parser, date token resolver (`__LAST_7_DAYS__` etc.), and SQL builder. Supports `AccountScope` for cross-account queries. The TypeScript version in `src/services/search/` still exists and is used by the React frontend.

## Gotchas that will break your code

**Multiple content stores**: Message bodies live outside the main `messages` table in `bodies.db` (zstd-compressed), and inline multipart images have their own attachment database. Use the Rust-side data access layer rather than assuming message content is in the main SQLite database.

**Four email providers in Rust**: `gmail_api`, `jmap`, `graph` (Microsoft), `imap`. All unified behind the `ProviderOps` trait (`core/src/provider/ops.rs`). Provider-agnostic commands in `src/provider/commands.rs`.

**Offline actions**: All email mutations go through `emailActions.ts` (optimistic UI + local DB + offline queue). Never call provider APIs directly from components.

**`src/core/`** (TS) is the facade layer — UI imports from `core/`, not from `services/db/` directly. `rustDb.ts` (38KB) wraps all Rust DB invoke calls.

**Core vs app crate boundary**: Business logic belongs in `ratatoskr-core`. Anything importing `tauri::*` stays in the app crate. When adding new core functionality, add it to `core/src/` and re-export from the app's `mod.rs`.

## `jmap-client` crate gotchas

These are non-obvious behaviors of the `jmap-client` crate that will matter if the code is modified:

- **Getting all mailboxes**: `mailbox_get(id, props)` fetches ONE mailbox. To get all, use the builder pattern: `request.get_mailbox()` with no ID set.
- **`mb.role()`** returns `Role` directly (not `Option<Role>`). Compare with `Role::None` to check if unset.
- **`mb.total_emails()`** returns `usize` directly, not `Option<usize>`.
- **`take_id()` / `take_list()`** require `let mut` on the response object.
- **Filter type inference**: Rust can't infer the generic for `Some(filter.into())` in `email_query()`. Bind to an explicit type: `let filter: core::query::Filter<email::query::Filter> = ...;`
- **`download(blob_id)`** takes only the blob ID — NOT `(account_id, blob_id, name)`.
- **`email_submission_create(email_id, identity_id)`** needs an identity ID, not account ID. Fetch identities via builder pattern.
- **`changes.created()/updated()/destroyed()`** return `&[String]`, not `&[&str]`. Use `.map(String::as_str)` not `.copied()`.
- **`fetch_text_body_values(true)`** is accessed via `get_req.arguments().fetch_text_body_values(true)`, not directly on the get request.
- **`mailbox_changes(since_state, 0)`** — max_changes of 0 is invalid per JMAP spec. Use 500.

## Lint rules that will surprise you

**Rust (edition 2024, strict clippy)**:
- `unwrap_used`: denied — use `?` or handle errors
- `await_holding_lock`: denied

**TypeScript**:
- `verbatimModuleSyntax: true` — must use `import type` for type-only imports
- `erasableSyntaxOnly: true`
- `exactOptionalPropertyTypes: true` — `undefined` vs missing property matters
- `noUncheckedIndexedAccess: true`
- Target: ES2024, bundler module resolution
- Path alias: `@/*` → `src/*`

**Biome**:
- `useExplicitType`: all functions need explicit return types (off in tests)
- `noForEach`: use `for...of`
- `noExplicitAny`, `noNonNullAssertion`, `noShadow`: all errors
- `noFloatingPromises`, `noMisusedPromises`: enforced
- `noNamespaceImport`: no `import * as` (off in tests)
- `noBarrelFile`, `noReExportAll`, `noImportCycles`: all errors

## Tauri-specific

- **Capabilities**: New plugins need permissions in `src-tauri/capabilities/default.json`. Allowed windows: `main`, `splashscreen`, `thread-*`, `compose-*`
- **SQL plugin preload**: Must be array `["sqlite:ratatoskr.db"]`, not object
- **Progress events**: Use `TauriProgressReporter::from_ref(&app_handle)` at command boundaries, pass `&reporter` (as `&dyn ProgressReporter`) to core functions. Never call `app.emit()` directly in core logic.
- **Single instance**: Must be first plugin registered
- **Minimize-to-tray**: Use `.on_window_event()` on Builder, not `window.on_window_event()`
- **Linux tray**: Uses `tray-item` crate (KSNI), not Tauri's built-in tray. `set_tray_tooltip` is a no-op on Linux
- **Window decorations**: macOS uses `titleBarStyle: "Overlay"` from config; Windows/Linux remove decorations programmatically in Rust

## Testing

Vitest + jsdom. `globals: true` (no imports for describe/it/expect). Tests colocated with source. Mocks in `src/test/mocks/`. Tests excluded from tsconfig compilation.

## Encryption

AES-256-GCM (`provider/crypto.rs`). Key file: `ratatoskr.key` (or legacy `velo.key`) in app data dir. Format: `base64(iv):base64(ct+tag)`. Falls back to zero-key if missing.

## Multi-window

Three window types via URL params in `main.tsx`: main app (no params), thread pop-out (`?thread=...&account=...`), composer pop-out (`?compose`). TanStack Router with hash history, lazy-loaded pages.
