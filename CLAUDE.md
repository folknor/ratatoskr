# Ratatoskr

**Subagents must always be launched in the foreground** (never use `run_in_background: true`) so the user can approve tool requests.

Pure Rust desktop email client. Cargo workspace (19 crates). Key crates:

- **`ratatoskr-core`** (`crates/core/`) — Top-level facade: re-exports all subsystem crates, plus owns accounts, oauth, discovery, email actions, DB queries, cloud attachments.
- **`app`** (`crates/app/`) — iced UI app. Elm architecture (boot/update/view). All UI conventions are in `UI.md` at the repo root — **read UI.md before any UI work.**
- **`squeeze`** (`crates/squeeze/`) — Attachment compression (CLI + library). Images (mozjpeg-rs + oxipng), PDFs (lopdf), OOXML/ODF.
- **`ratatoskr-stores`** (`crates/stores/`) — Content stores: email body store (zstd-compressed), inline image store, attachment file cache.
- **`ratatoskr-sync`** (`crates/sync/`) — Sync pipeline, threading (JWZ), categorization, filters, smart labels.
- **`ratatoskr-provider-utils`** (`crates/provider-utils/`) — Shared provider helpers, encryption (AES-256-GCM), email parsing, HTML sanitization.
- **`ratatoskr-label-colors`** (`crates/label-colors/`) — Label color resolution + Exchange category color presets.
- **Providers**: `gmail`, `jmap`, `graph`, `imap` — each in `crates/{name}/`.

## Commands

- `cargo check --workspace` — check all crates
- `cargo check -p ratatoskr-core` — check core only
- `cargo check -p app` — check app only
- `cargo run -p app` — run the iced app (requires a seeded DB, see `crates/app/seed-db.py`)
- `cargo check -p squeeze` — check squeeze only
- `cargo test -p squeeze` — run squeeze tests

## Crate Architecture

**`ProgressReporter` trait** (`ratatoskr_core::progress`) — All event emission goes through `&dyn ProgressReporter`. The iced app will provide its own implementation.

**State types are `Clone`** — `DbState`, `BodyStoreState`, `InlineImageStoreState`, `SearchState`, `AppCryptoState` all wrap `Arc<Mutex<Connection>>` or similar and implement `Clone`. Both `DbState` and `BodyStoreState` expose `pub fn conn(&self) -> Arc<Mutex<Connection>>` for synchronous access.

**Scoped queries** (`core/src/db/queries_extra/scoped_queries.rs`) — Cross-account query infrastructure. `ViewScope` enum (`AllAccounts`/`Account`/`SharedMailbox`/`PublicFolder`) in `core/src/scope.rs` is the sidebar's single source of truth. Personal-account queries use `AccountScope` internally and filter `shared_mailbox_id IS NULL`. Shared mailbox and public folder scopes route to dedicated query functions. Predicate-based virtual folder queries for Starred/Snoozed use boolean flags on `threads`, not label joins. Draft counts include `local_drafts` table.

**Navigation state** (`core/src/db/queries_extra/navigation.rs`) — `get_navigation_state()` returns the full sidebar state in one call: universal folders (Inbox, Starred, Snoozed, Sent, Drafts, Trash) with unread counts, smart folders, and per-account labels when scoped. Smart folder and per-label unread counts are scaffolded (return 0).

**Thread detail** (`core/src/db/queries_extra/thread_detail.rs`) — `get_thread_detail()` returns messages (with ownership detection, collapsed summaries, body text from body store), labels (with resolved colors), attachments (with message context), and attachment collapse state for a single thread.

**Label colors** (`core/src/label_colors.rs`) — `resolve_label_color()` returns synced colors for Gmail labels, deterministic hash-based fallback from the 25-preset palette for all other providers.

**Smart folder engine** (`core/src/smart_folder/`) — Query parser, date token resolver (`__LAST_7_DAYS__` etc.), and SQL builder. Supports `AccountScope` for cross-account queries.

**Command palette** (`core/src/command_palette/`) — Command registry, fuzzy search (nucleo-matcher), context-sensitive command availability. `CommandContext` includes `focused_region: Option<FocusedRegion>` for panel-aware shortcut dispatch.

## Gotchas that will break your code

**Multiple content stores** (`crates/stores/`): Message bodies live outside the main `messages` table in `bodies.db` (zstd-compressed), and inline multipart images have their own attachment database. Use `BodyStoreState` / `InlineImageStoreState` rather than assuming message content is in the main SQLite database. The attachment file cache is also in this crate.

**Four email providers**: `gmail_api`, `jmap`, `graph` (Microsoft), `imap`. All unified behind the `ProviderOps` trait (`provider-utils/src/ops.rs`). Folder-accepting methods use `&FolderId`, tag-accepting methods use `&TagId` (`provider-utils/src/typed_ids.rs`). Typed IDs flow from `MailActionIntent` through `MailOperation` to the provider — no raw string boundaries in the action pipeline.

**Action pipeline**: `MailActionIntent → resolve_intent() → build_execution_plan() → batch_execute() → handle_action_completed()`. All 12 action types flow through one path. `MailOperation` (core) is the canonical execution type. `CompletionBehavior` (app) drives toast, auto-advance, and undo via exhaustive match. See `docs/architecture.md` § "Adding a New Email Action" for the checklist.

**Generation counters use branded tokens**: `GenerationCounter<T>` / `GenerationToken<T>` in `core/src/generation.rs`. `next()` is the only way to get a token (bumps and returns). `#[must_use]` on `next()` — use `let _ = counter.next()` for invalidation-only bumps. Phantom type brands prevent cross-counter comparison. See `docs/architecture.md` for the full pattern.

**Core crate boundary**: Business logic belongs in `ratatoskr-core`. The app crate calls core functions directly (no command wrappers needed — the Tauri app shell has been removed). When adding new core functionality, add it to `crates/core/src/`.

**iced is depended on in 3 places**: `crates/app/Cargo.toml` (full iced umbrella), `crates/rich-text-editor/Cargo.toml` (iced umbrella, optional behind `widget` feature), and `crates/iced-drop/Cargo.toml` (iced_core + iced_widget + iced_runtime individually). All three must point to the same iced source. When switching between the git URL and local path, update all three.

## `jmap-client` crate gotchas

These are non-obvious behaviors of the `jmap-client` crate that will matter if the code is modified:

- **Getting all mailboxes**: `mailbox_get(id, props)` fetches ONE mailbox. To get all, use the builder: `MailboxGet::new(&account_id)` with no IDs set, submitted via `request.call(get)`. See `sync/mailbox.rs:fetch_all_mailboxes_for()`.
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

## Multi-Agent Orchestration

**Do NOT use worktree isolation for parallel agents.** Worktrees create merge conflicts that silently drop agent work. Instead, launch agents in the same tree with strict file ownership — zero overlap.

**Why no worktrees:** Worktrees let agents work on diverged snapshots. When merging back, `git checkout --ours/--theirs` drops code, conflict markers get missed, and features end up "existing but not wired" — types/functions created but never connected to message dispatch, views, or call sites. This happened repeatedly in a 114-commit session and was only caught by a rigorous 3-pass audit.

**Agent coordination rules:**
- Each agent gets exclusive ownership of specific files. No two agents touch the same file.
- `main.rs` is shared — agents may ONLY add Message enum variants and one-line dispatch arms. All handler logic goes in `handlers/*.rs`.
- Agents must read their handler file FIRST (it already has extracted methods). Do not replace existing code with placeholders.
- Agents must NOT run `cargo check/build/test`. The orchestrator validates between agents.
- Include `UI.md` and `CLAUDE.md` in every agent's required reading.

**Verification standard — "implemented" means wired:**
- A feature is NOT implemented unless the user can reach it through current Message dispatch → handler → view wiring.
- Types that exist but are never constructed, methods that exist but are never called, message variants with no dispatch arm — these are dead code, not implementations.
- After agents complete, verify wiring by checking: (1) Message variant exists, (2) dispatch arm in update() calls handler, (3) handler performs the work, (4) view renders the result or side effect is observable.

**Audit protocol:**
- Do not trust agent claims of completion. Verify existence + wiring + behavior.
- Use the 3-pass audit structure: domain-specific verification → cross-cutting reconciliation → editorial normalization.
- Discrepancies docs should contain only current gaps, not historical records. Remove resolved items entirely.

**Common agent mistakes:**
- Using `gen` as a variable name (reserved keyword in edition 2024)
- Using `iced::mouse::click::Kind` instead of `iced::advanced::mouse::click::Kind` (the `iced::mouse` re-export doesn't include the `click` submodule)
- Creating types/functions without wiring them to message dispatch (the #1 failure mode)
- Rewriting entire files instead of making targeted edits
- Claiming features are "done" when only the types exist but call sites are missing

## Code Review (`review`)

`review` is a CLI tool that fans out code review requests to persistent AI sessions. Each session is a long-lived Claude or Codex conversation that has already been onboarded with project context for a specific review lens. Configuration lives in `.review.toml` at the repo root.

Four archetypes are configured: `security`, `bugs`, `perf`, `arch`. The `sweep` group fans out to all four in parallel.

Instructions are piped via stdin. The agents fetch code themselves — just tell them what to look at:

```bash
echo "check the new sync logic" | review bugs
echo "review this change" | review arch
echo "full review" | review sweep
echo "look at stores" | review perf
```

Without `--anchor`, stdin goes directly to the session — the sessions are already onboarded with project context and their review focus. Use `--anchor` for the first review in a session or to re-anchor a stale session. When using `--anchor`, reinforce the session's identity in your piped instructions:

- **security**: "Remember: you're our security auditor for ratatoskr."
- **bugs**: "Remember: you're our QA engineer embedded on ratatoskr."
- **perf**: "Remember: you're our performance engineer reviewing ratatoskr."
- **arch**: "Remember: you're our software architect reviewing ratatoskr."

**Never run reviews in parallel to the same archetype.** Each archetype maps to a single persistent session — concurrent sends will race messages into the same conversation and corrupt context. Run sequentially, or only parallelize across different archetypes (e.g. `review bugs` and `review security` can run concurrently, but not two `review bugs` calls).

## Commit rules

- Don't commit pure markdown changes on their own. Bundle them with the code change they relate to, or skip them. Unless the markdown update is substantive.
- Has Cargo.lock changed? Commit it.

## Encryption

AES-256-GCM (`core/src/provider/crypto.rs`). Key file: `ratatoskr.key` (or legacy `velo.key`) in app data dir. Format: `base64(iv):base64(ct+tag)`. Falls back to zero-key if missing.
