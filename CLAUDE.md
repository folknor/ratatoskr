# Ratatoskr

**Subagents must always be launched in the foreground** (never use `run_in_background: true`) so the user can approve tool requests.

Pure Rust desktop email client. Cargo workspace with three crates:

- **`ratatoskr-core`** (`crates/core/`, ~22k lines) — Framework-agnostic business logic: providers, sync, threading, filters, search, DB, encryption, AI, categories, smart folders, etc.
- **`app`** (`crates/app/`) — iced UI app. Elm architecture (boot/update/view). All UI conventions are in `UI.md` at the repo root — **read UI.md before any UI work.**
- **`squeeze`** (`crates/squeeze/`) — Attachment compression crate (CLI + library). Compresses images (mozjpeg-rs + oxipng), PDFs (lopdf), and OOXML/ODF documents (ZIP archive image compression).

## Commands

- `cargo check --workspace` — check all three crates
- `cargo check -p ratatoskr-core` — check core only
- `cargo check -p app` — check app only
- `cargo run -p app` — run the iced app (requires a seeded DB, see `crates/app/seed-db.py`)
- `cargo check -p squeeze` — check squeeze only
- `cargo test -p squeeze` — run squeeze tests

## Crate Architecture

**`ProgressReporter` trait** (`ratatoskr_core::progress`) — All event emission goes through `&dyn ProgressReporter`. The iced app will provide its own implementation.

**State types are `Clone`** — `DbState`, `BodyStoreState`, `InlineImageStoreState`, `SearchState`, `AppCryptoState` all wrap `Arc<Mutex<Connection>>` or similar and implement `Clone`. Both `DbState` and `BodyStoreState` expose `pub fn conn(&self) -> Arc<Mutex<Connection>>` for synchronous access.

**Scoped queries** (`core/src/db/queries_extra/scoped_queries.rs`) — Cross-account query infrastructure. `AccountScope` enum (`Single`/`Multiple`/`All`) controls which accounts a query spans. Predicate-based virtual folder queries for Starred/Snoozed use boolean flags on `threads`, not label joins. Draft counts include `local_drafts` table.

**Navigation state** (`core/src/db/queries_extra/navigation.rs`) — `get_navigation_state()` returns the full sidebar state in one call: universal folders (Inbox, Starred, Snoozed, Sent, Drafts, Trash) with unread counts, smart folders, and per-account labels when scoped. Smart folder and per-label unread counts are scaffolded (return 0).

**Thread detail** (`core/src/db/queries_extra/thread_detail.rs`) — `get_thread_detail()` returns messages (with ownership detection, collapsed summaries, body text from body store), labels (with resolved colors), attachments (with message context), and attachment collapse state for a single thread.

**Label colors** (`core/src/label_colors.rs`) — `resolve_label_color()` returns synced colors for Gmail labels, deterministic hash-based fallback from the 25-preset palette for all other providers.

**Smart folder engine** (`core/src/smart_folder/`) — Query parser, date token resolver (`__LAST_7_DAYS__` etc.), and SQL builder. Supports `AccountScope` for cross-account queries.

**Command palette** (`core/src/command_palette/`) — Command registry, fuzzy search (nucleo-matcher), context-sensitive command availability. `CommandContext` includes `focused_region: Option<FocusedRegion>` for panel-aware shortcut dispatch.

## Gotchas that will break your code

**Multiple content stores**: Message bodies live outside the main `messages` table in `bodies.db` (zstd-compressed), and inline multipart images have their own attachment database. Use `BodyStoreState` / `InlineImageStoreState` rather than assuming message content is in the main SQLite database.

**Four email providers**: `gmail_api`, `jmap`, `graph` (Microsoft), `imap`. All unified behind the `ProviderOps` trait (`core/src/provider/ops.rs`).

**Core crate boundary**: Business logic belongs in `ratatoskr-core`. The app crate calls core functions directly (no command wrappers needed — the Tauri app shell has been removed). When adding new core functionality, add it to `crates/core/src/`.

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

## Lint rules

**Rust (edition 2024, strict clippy)**:
- `unwrap_used`: denied — use `?` or handle errors
- `await_holding_lock`: denied
- `too_many_arguments`: 7 max
- `too_many_lines`: 100 max
- `cognitive_complexity`: denied at threshold

## Encryption

AES-256-GCM (`core/src/provider/crypto.rs`). Key file: `ratatoskr.key` (or legacy `velo.key`) in app data dir. Format: `base64(iv):base64(ct+tag)`. Falls back to zero-key if missing.
