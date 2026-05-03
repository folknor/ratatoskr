# Ratatoskr

**Subagents must always be launched in the foreground** (never use `run_in_background: true`) so the user can approve tool requests.

Pure Rust desktop email client targeting enterprise users currently locked into Outlook/Microsoft 365. Three non-negotiable constraints shape the project:

1. **Exchange + Calendar** - no free client at scale supports both.
2. **Extreme volume** - users process hundreds of emails/day; cached mailboxes hit 150+ GB uncapped.
3. **Deep local search** - 5+ years of history, searchable instantly.

Performance, storage efficiency (zstd compression, efficient DB), search speed, and deduplication (e.g. inline image dedup in the attachment store) are hard requirements, not nice-to-haves.

Cargo workspace (19 crates). Key crates:

- **`rtsk`** (`crates/core/`) - Top-level facade: re-exports all subsystem crates, plus owns accounts, oauth, discovery, email actions, DB queries, cloud attachments.
- **`app`** (`crates/app/`) - iced UI app. Elm architecture (boot/update/view). All UI conventions are in `UI.md` at the repo root - **read UI.md before any UI work.**
- **`squeeze`** (`crates/squeeze/`) - Attachment compression (CLI + library). Images (mozjpeg-rs + oxipng), PDFs (lopdf), OOXML/ODF.
- **`store`** (`crates/stores/`) - Content stores: email body store (compressed), inline image store, attachment file cache.
- **`sync`** (`crates/sync/`) - Sync pipeline, threading (JWZ), bundling (AI inbox classification), filters, smart labels.
- **`provider`** (`crates/common/`) - Shared provider helpers, encryption (AES-256-GCM), email parsing, HTML sanitization.
- **`label-colors`** (`crates/label-colors/`) - Label color resolution + Exchange preset color palette.
- **`types`** (`crates/types/`) - Lightweight shared types (`FolderId`, `TagId`, `SidebarSelection`). Minimal deps (serde only).
- **`dev-seed`** (`crates/dev-seed/`) - Deterministic test database generator. See dev-seed section below.
- **Providers**: `gmail`, `jmap`, `graph`, `imap` - each in `crates/{name}/`.

## Required reading

Read the doc before starting work in its area. Subagents launched for these tasks must include the relevant doc in their required-reading list.

- **Any UI work** - `UI.md` at the repo root.
- **Architectural decisions, crate boundaries, new email actions, generation counters, scope wiring, calendar workflow layering, provider trait additions** - `docs/architecture.md`.
- **Anything touching folders, labels, the `labels` table, `thread_labels`, `label_kind`, system folder IDs (`INBOX`, `TRASH`, `SPAM`, `SENT`, `DRAFT`, `archive`, `STARRED`), or provider folder/label sync** - `docs/glossary/folders-labels.md`.
- **Adding or refactoring tooltips, dropdowns, context menus, popovers, modals, sheets, or any new overlay-like surface** - `docs/glossary/overlay-surfaces.md`.

## Rules

### General rules

- Don't use gremlins! Em-dash, en-dash, strange quotes, whatever - they're all verboten.
- Don't remind the user of CLAUDE.md rules. They wrote them, so they know them.

### Memory rules

Do not use your Memory functionality. Do not read, write, or update memories. Do not suggest saving things to memory. Durable context belongs in CLAUDE.md or the relevant docs, not in per-session memory files - this project is developed across several hosts and users, and memory does not transfer between them; CLAUDE.md does.

### Bash rules

- Never use `sed`, `find`, `awk`, `head`, `tail`, or complex bash commands.
- Never chain commands with `&&`.
- Never chain commands with `;`.
- Never chain/pipe commands with `|`. Exception: piping into `review` is allowed (writing scratch prompt files is wasteful).
- Never capture stdout into env vars (`UUID=$(...)`).
- Never read or write from `/tmp`. All data lives in the project.
- Never run raw `cargo`, `curl`, `pkill`. Use `brokkr`.

### git commit rules

- Never commit markdown changes alone. Bundle them with upcoming code commits.
- When committing other changes: always tag along markdown files if dirty.
- Write substantive engineering-focused commit messages.
- Has `Cargo.lock` changed? Commit it.
- Never `git push` unless the user explicitly asks. Stop after the commit.

## Commands

Use `brokkr` (not `cargo`) for check/test. It runs a gremlins scan (banned Unicode), then clippy, then tests - clippy denies warnings project-wide, so a clippy failure short-circuits before tests run. By default output is filtered to changed files and capped at 20 diagnostics per phase.

- `brokkr check` - gremlins + clippy + all tests (changed-files scope)
- `brokkr check --all` - show every diagnostic, no cap, no scope filter
- `brokkr check --fix-gremlins` - rewrite banned Unicode in tracked files (em/en dash -> `-`, smart quotes -> straight, NBSP -> space, zero-width/bidi deleted) before checking
- `brokkr check -p <crate>` - scope to one package (e.g. `-p rtsk`, `-p app`, `-p squeeze`)
- `brokkr check -- --test <file>` - forward args to `cargo test` (args after the second `--` go to the test binary)
- `brokkr test -p <crate> <NAME>` - release-mode focused single-test runner. Always passes `--release --include-ignored --nocapture --test-threads=1`. `<NAME>` is a case-sensitive substring filter (matches both unit and integration tests). Streams the test's own stdout/stderr live and prints a `[test] PASS/FAIL` footer with wall time. Defaults to `--all-features`; runs a second sweep if `[check].consumer_features` is set in `brokkr.toml`. Gated off for litehtml/sluggrs (use `brokkr visual` there).
  - `-p, --package <PKG>` - cargo package. Required in this workspace - no default package, and overrides `[test] default_package` in `brokkr.toml` if set.
  - `-N, --repeat <N>` - run the test N times per sweep (flaky-test hunting).
  - `-j, --jobs <N>` - parallel cargo compile jobs.
  - `--raw` - bypass output filtering, print everything cargo emits.
  - `--debug` - build and run the test in dev profile instead of release. Use this for subprocess-lifecycle / IPC / boot-path tests where release-LTO compile time (3-4 min for the full workspace) dominates wall time and the optimization level doesn't change the behavior under test. `BROKKR_TEST_BIN_DIR` points at `<target>/debug` accordingly.
  - Example: `brokkr test -p common truncates_without_splitting` or `brokkr test -p calendar extract_tag_value_flattens_nested_text -N 5` or `brokkr test -p app terminal_failure_at_initial_boot_does_not_respawn --debug`.
- `cargo run -p app` - run the iced app (requires a seeded DB, see `crates/app/seed-db.py`)

Fall back to raw `cargo check`/`cargo test` only when you need to bypass clippy gating for a targeted run.

**Never run `cargo fmt`.** Formatting is the user's call - leave whitespace, line breaks, and import ordering as written.

## Dev-Seed

`crates/dev-seed/` generates a deterministic test database from scratch. Config lives in `dev-seed.toml` at the repo root (thread count, account count, locale, RNG seed). When the app is built with `--features dev-seed`, it **wipes the entire dev data directory and re-seeds on every launch** - there is no persistence between runs. Schema comes from `crates/db/src/db/migrations.rs` (a single v100 migration). Dev-seed does not use DB migrations for schema changes - just update the CREATE TABLE in migrations.rs and re-run.

## Crate Architecture

**`ProgressReporter` trait** (`rtsk::progress`) - All event emission goes through `&dyn ProgressReporter`. The iced app will provide its own implementation.

**State types are `Clone`** - `DbState`, `BodyStoreState`, `InlineImageStoreState`, `SearchState`, `AppCryptoState` all wrap `Arc<Mutex<Connection>>` or similar and implement `Clone`. Both `DbState` and `BodyStoreState` expose `pub fn conn(&self) -> Arc<Mutex<Connection>>` for synchronous access.

**Scoped queries** (`core/src/db/queries_extra/scoped_queries.rs`) - Cross-account query infrastructure. `ViewScope` enum (`AllAccounts`/`Account`/`SharedMailbox`/`PublicFolder`) in `core/src/scope.rs` is the sidebar's single source of truth. Personal-account queries use `AccountScope` internally and filter `shared_mailbox_id IS NULL`. Shared mailbox and public folder scopes route to dedicated query functions. Predicate-based virtual folder queries for Starred/Snoozed use boolean flags on `threads`, not label joins. Draft counts include `local_drafts` table.

**Navigation state** (`core/src/db/queries_extra/navigation.rs`) - `get_navigation_state()` returns the full sidebar state in one call: universal folders (Inbox, Starred, Snoozed, Sent, Drafts, Trash) with unread counts, smart folders, and per-account labels when scoped. Smart folder and per-label unread counts are scaffolded (return 0).

**Thread detail** (`core/src/db/queries_extra/thread_detail.rs`) - `get_thread_detail()` returns messages (with ownership detection, collapsed summaries, body text from body store), labels (with resolved colors), attachments (with message context), and attachment collapse state for a single thread.

## Gotchas that will break your code

**Never run squeeze against `fixtures/5.pdf`.** It's a 220MB PDF that pegs all CPU cores and freezes the user's machine. When testing squeeze on the PDF fixtures, exclude 5.pdf explicitly - use 2.pdf, 3.pdf, 9.pdf, or 14.pdf instead.

**Multiple content stores** (`crates/stores/`): Message bodies live outside the main `messages` table in `bodies.db` (compressed), and inline multipart images have their own attachment database. Use `BodyStoreState` / `InlineImageStoreState` rather than assuming message content is in the main SQLite database. The attachment file cache is also in this crate.

**Four email providers**: `gmail_api`, `jmap`, `graph` (Microsoft), `imap`. All unified behind the `ProviderOps` trait (`common/src/ops.rs`). Folder-accepting methods use `&FolderId`, tag-accepting methods use `&TagId` (`common/src/typed_ids.rs`). Typed IDs flow from `MailActionIntent` through `MailOperation` to the provider - no raw string boundaries in the action pipeline.

**Action pipeline**: `MailActionIntent → resolve_intent() → build_execution_plan() → batch_execute() → handle_action_completed()`. All 12 action types flow through one path. `MailOperation` (core) is the canonical execution type. `CompletionBehavior` (app) drives toast, auto-advance, and undo via exhaustive match. See `docs/architecture.md` § "Adding a New Email Action" for the checklist.

**Generation counters use branded tokens**: `GenerationCounter<T>` / `GenerationToken<T>` in `core/src/generation.rs`. `next()` is the only way to get a token (bumps and returns). `#[must_use]` on `next()` - use `let _ = counter.next()` for invalidation-only bumps. Phantom type brands prevent cross-counter comparison. See `docs/architecture.md` for the full pattern.

**Core crate boundary**: Business logic belongs in `rtsk`. The app crate calls core functions directly (no command wrappers needed - the Tauri app shell has been removed). When adding new core functionality, add it to `crates/core/src/`.

**iced is depended on in 3 places**: `crates/app/Cargo.toml` (full iced umbrella), `crates/rte/Cargo.toml` (iced umbrella, optional behind `widget` feature), and `crates/iced-drop/Cargo.toml` (iced_core + iced_widget + iced_runtime individually). All three must point to the same iced source. When switching between the git URL and local path, update all three.

## `jmap-client` crate gotchas

These are non-obvious behaviors of the `jmap-client` crate that will matter if the code is modified:

- **Getting all mailboxes**: `mailbox_get(id, props)` fetches ONE mailbox. To get all, use the builder: `MailboxGet::new(&account_id)` with no IDs set, submitted via `request.call(get)`. See `sync/mailbox.rs:fetch_all_mailboxes_for()`.
- **`mb.role()`** returns `Role` directly (not `Option<Role>`). Compare with `Role::None` to check if unset.
- **`mb.total_emails()`** returns `usize` directly, not `Option<usize>`.
- **`take_id()` / `take_list()`** require `let mut` on the response object.
- **Filter type inference**: Rust can't infer the generic for `Some(filter.into())` in `email_query()`. Bind to an explicit type: `let filter: core::query::Filter<email::query::Filter> = ...;`
- **`download(blob_id)`** takes only the blob ID - NOT `(account_id, blob_id, name)`.
- **`email_submission_create(email_id, identity_id)`** needs an identity ID, not account ID. Fetch identities via builder pattern.
- **`changes.created()/updated()/destroyed()`** return `&[String]`, not `&[&str]`. Use `.map(String::as_str)` not `.copied()`.
- **`fetch_text_body_values(true)`** is accessed via `get_req.arguments().fetch_text_body_values(true)`, not directly on the get request.
- **`mailbox_changes(since_state, 0)`** - max_changes of 0 is invalid per JMAP spec. Use 500.

## Lint rules

**Rust (edition 2024, strict clippy)**:
- `unwrap_used`: denied - use `?` or handle errors
- `await_holding_lock`: denied
- `too_many_arguments`: 7 max
- `too_many_lines`: 100 max
- `cognitive_complexity`: denied at threshold

## Multi-Agent Orchestration

**Do NOT use worktree isolation for parallel agents.** Worktrees create merge conflicts that silently drop agent work. Instead, launch agents in the same tree with strict file ownership - zero overlap.

**Why no worktrees:** Worktrees let agents work on diverged snapshots. When merging back, `git checkout --ours/--theirs` drops code, conflict markers get missed, and features end up "existing but not wired" - types/functions created but never connected to message dispatch, views, or call sites. This happened repeatedly in a 114-commit session and was only caught by a rigorous 3-pass audit.

**Agent coordination rules:**
- Each agent gets exclusive ownership of specific files. No two agents touch the same file.
- `main.rs` is shared - agents may ONLY add Message enum variants and one-line dispatch arms. All handler logic goes in `handlers/*.rs`.
- Agents must read their handler file FIRST (it already has extracted methods). Do not replace existing code with placeholders.
- Agents must NOT run `cargo check/build/test`. The orchestrator validates between agents.
- Include `UI.md` and `CLAUDE.md` in every agent's required reading.

**Verification standard - "implemented" means wired:**
- A feature is NOT implemented unless the user can reach it through current Message dispatch → handler → view wiring.
- Types that exist but are never constructed, methods that exist but are never called, message variants with no dispatch arm - these are dead code, not implementations.
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

`review` is a CLI tool that fans out code review requests to anchored AI sessions. Each archetype has a stored prime prompt (in `.review.toml` under `[_prime].<archetype>`) that defines the review lens. Configuration lives in `.review.toml` at the repo root.

Four archetypes: `security`, `bugs`, `perf`, `arch`. The `sweep` group fans out to all four in parallel.

**Multiple archetypes go in one invocation as a comma list** (e.g. `review bugs,arch,perf --oneshot`), not as separate parallel `review` calls. The CLI staggers requests internally to stay under upstream HTTP rate limits; firing several `review` processes at once defeats that and trips the limiter.

**Default to `--oneshot`.** Anthropic's prompt cache is ~1 hour, so starting a fresh session for each unrelated review is cheaper than resuming a long-lived one. `--oneshot` starts a fresh session, prepends the stored prime prompt, runs the query, and prints the session ID to stdout.

**Follow-ups:** within the cache window, resume the same session with `--session <id>` (using the ID printed by the previous `--oneshot`). The cache stays warm; only the new query and reply are billed. `--session` requires `--provider` and is mutually exclusive with `--oneshot`.

```bash
echo "review the new sync code" | review bugs --oneshot
# session: 019de...
# <findings>

echo "follow up on the second finding" | review bugs --provider claude --session 019de...
# <answer>
```

Don't reach for a second `--oneshot` to follow up - that creates a different fresh session with no memory of the first. Use `--session` for continuity within a thread, `--oneshot` for new threads.

To update the prime prompt for an archetype, pipe new content to `review prime <archetype> --provider <p>`. The prompt is stored once per archetype and shared across providers; once stored, prime any other provider with no stdin to reuse it.

## Encryption

AES-256-GCM (`crates/common/src/crypto.rs` for the cipher; key load lives in the dep-free `crates/crypto-key/` crate shared between `common` and `service`). Key file: `ratatoskr.key` (or legacy `velo.key`) in app data dir. Format: base64-encoded 32 bytes. Encrypted-value wire format: `base64(iv):base64(ct+tag)`.

Boot path: Service loads + validates the key during `BootPhase::LoadingKey`. A missing or unreadable key file is a fatal Service exit (`BootExitCode::KeyLoadFailure = 73`); there is no zero-key fallback. The `crypto-key` crate enforces TOCTOU-safe permission repair (`O_NOFOLLOW` + `fchmod` via the open fd), file-owner UID validation on Unix, and a release-build hard-fail on the all-zero dev-seed key so a stray dev key cannot silently downgrade AES-256-GCM in production. Loaded keys are returned in a `SecretKey` wrapper that zeroizes its buffer on drop.
