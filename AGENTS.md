# Ratatoskr

Rust desktop enterprise email client:

1. Exchange + Calendar - no free client at scale supports both.
2. Extreme volume - users process hundreds of emails/day; cached mailboxes hit 150+ GB uncapped.
3. Deep local search - 5+ years of history, searchable instantly.

Performance, storage efficiency, search speed, and deduplication (e.g. inline image dedup in the attachment store) are hard requirements.

Cargo workspace. Key crates:

- `rtsk` (`crates/core/`) - Top-level facade: re-exports all subsystem crates, plus owns accounts, oauth, discovery, email actions, DB queries, cloud attachments.
- `app` (`crates/app/`) - iced UI app. Elm architecture (boot/update/view). All UI conventions are in `UI.md` at the repo root - read UI.md before any UI work.
- `squeeze` (`crates/squeeze/`) - Attachment compression (CLI + library). Images (mozjpeg-rs + oxipng), PDFs (lopdf), OOXML/ODF.
- `store` (`crates/stores/`) - Content stores: email body store (compressed), inline image store, attachment file cache.
- `sync` (`crates/sync/`) - Sync pipeline, threading (JWZ), bundling (AI inbox classification), filters, smart labels.
- `provider` (`crates/common/`) - Shared provider helpers, encryption (AES-256-GCM), email parsing, HTML sanitization.
- `label-colors` (`crates/label-colors/`) - Label color resolution + Exchange preset color palette.
- `types` (`crates/types/`) - Lightweight shared types (`FolderId`, `TagId`, `SidebarSelection`). Minimal deps (serde only).
- `dev-seed` (`crates/dev-seed/`) - Deterministic test database generator. See dev-seed section below.
- Providers: `gmail`, `jmap`, `graph`, `imap` - each in `crates/{name}/`.

## Required reading

Read the doc before starting work in its area. Subagents launched for these tasks must include the relevant doc in their required-reading list.

- Any UI work - `UI.md` at the repo root.
- Architectural decisions, crate boundaries, new email actions, generation counters, scope wiring, calendar workflow layering, provider trait additions - `docs/architecture.md`.
- Anything touching (email provider) folders, labels, the `labels` table, `thread_labels`, `label_kind`, system folder IDs (`INBOX`, `TRASH`, `SPAM`, `SENT`, `DRAFT`, `archive`, `STARRED`), or provider folder/label sync - `docs/glossary/folders-labels.md`.
- Adding or refactoring tooltips, dropdowns, context menus, popovers, modals, sheets, or any new overlay-like surface - `docs/glossary/overlay-surfaces.md`.
- Service test harness, sync-harness scripts, harness Lua bindings, `app --test-harness`, `dellingr` VM, `brokkr service-test`/`service-suite`/`sync-bench`, gate baselines, or anything touching `crates/app/tests/service-harness/` or `crates/app/tests/sync-harness/` - `docs/glossary/harness.md`.

## Rules

### General rules

- Don't use gremlins! Em-dash, en-dash, strange quotes, whatever - they're all verboten.
- Don't remind the user of the rules. They wrote them, so they know them.
- The user can exempt you from any rule at any time.

### Bash rules

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

Use `brokkr` (not `cargo`) for check/test. By default output is filtered to changed files and capped at 20 diagnostics per phase.

- `brokkr check` - gremlins + clippy + all tests (changed-files scope)
- `brokkr check --all` - show every diagnostic, no cap, no scope filter
- `brokkr check -p <crate>` - scope to one package (e.g. `-p rtsk`, `-p app`, `-p squeeze`)
- `brokkr check -- --test <file>` - forward args to `cargo test` (args after the second `--` go to the test binary)
- `brokkr test -p <crate> <NAME>` - release-mode focused single-test runner. Always passes `--release --include-ignored --nocapture --test-threads=1`. `<NAME>` is a case-sensitive substring filter (matches both unit and integration tests). Streams the test's own stdout/stderr live and prints a `[test] PASS/FAIL` footer with wall time. Defaults to `--all-features`; runs a second sweep if `[check].consumer_features` is set in `brokkr.toml`. Gated off for litehtml/sluggrs (use `brokkr visual` there).
  - `-p, --package <PKG>` - cargo package. Required in this workspace - no default package, and overrides `[test] default_package` in `brokkr.toml` if set.
  - `-N, --repeat <N>` - run the test N times per sweep (flaky-test hunting).
  - `-j, --jobs <N>` - parallel cargo compile jobs.
  - `--raw` - bypass output filtering, print everything cargo emits.
  - `--debug` - build and run the test in dev profile instead of release. Use this for subprocess-lifecycle / IPC / boot-path tests where release-LTO compile time (3-4 min for the full workspace) dominates wall time and the optimization level doesn't change the behavior under test. `BROKKR_TEST_BIN_DIR` points at `<target>/debug` accordingly.
  - Example: `brokkr test -p common truncates_without_splitting` or `brokkr test -p calendar extract_tag_value_flattens_nested_text -N 5` or `brokkr test -p app terminal_failure_at_initial_boot_does_not_respawn --debug`.
- `cargo run -p app` - run the iced app

A healthy `brokkr check` finishes well under 4 minutes. If it does not, something is wrong - kill it and investigate.

Never run `cargo|brokkr fmt`. Formatting is the user's call - leave whitespace, line breaks, and import ordering as written.

## Harness

Lua Service harness scripts live under `crates/app/tests/service-harness/`.
Sync harness scripts live under `crates/app/tests/sync-harness/`.

- `brokkr service-test <SCRIPT>` - run one Service harness script.
- `brokkr service-test <DIR> -N <N>` - run a cohort directory; `-N`
  means cohort cycles.
- `brokkr service-suite [--filter X]` - run the discovered Service
  harness suite, optionally filtered.
- `brokkr service-list` - list scripts and parsed frontmatter.

`brokkr.toml` has two ratatoskr sections:

- `[ratatoskr.harness]` selects the check sweep and app binary that
  `brokkr service-test` drives (`test-helpers` build of `app`).
- `[ratatoskr]` wires sync-harness mock servers: installed
  `saehrimnir` binary, fixture dir, endpoint env var names, and
  `sync_script_dir`.

`saehrimnir` is the external mock-provider server used by sync harness
scripts. Brokkr starts it, injects
`RATATOSKR_TEST_{JMAP,IMAP,SMTP,GRAPH,GMAIL}_ENDPOINT`, and scripts
exercise ratatoskr's real provider sync against those endpoints.

## Dev-Seed

`crates/dev-seed/` generates a deterministic test database from scratch. Config lives in `dev-seed.toml` at the repo root. When the app is built with `--features dev-seed` (it always is during development), it wipes the entire dev data directory and re-seeds on every launch - there is no persistence between runs. Schema comes from `crates/db/src/db/migrations.rs` (a single v100 migration).

## Crate Architecture

`ProgressReporter` trait (`rtsk::progress`) - All event emission goes through `&dyn ProgressReporter`. The iced app will provide its own implementation.

State types are `Clone` - `DbState`, `BodyStoreState`, `InlineImageStoreState`, `SearchState`, `AppCryptoState` all wrap `Arc<Mutex<Connection>>` or similar and implement `Clone`. Both `DbState` and `BodyStoreState` expose `pub fn conn(&self) -> Arc<Mutex<Connection>>` for synchronous access.

Scoped queries (`core/src/db/queries_extra/scoped_queries.rs`) - Cross-account query infrastructure. `ViewScope` enum (`AllAccounts`/`Account`/`SharedMailbox`/`PublicFolder`) in `core/src/scope.rs` is the sidebar's single source of truth. Personal-account queries use `AccountScope` internally and filter `shared_mailbox_id IS NULL`. Shared mailbox and public folder scopes route to dedicated query functions. Predicate-based virtual folder queries for Starred/Snoozed use boolean flags on `threads`, not label joins. Draft counts include `local_drafts` table.

Navigation state (`core/src/db/queries_extra/navigation.rs`) - `get_navigation_state()` returns the full sidebar state in one call: universal folders (Inbox, Starred, Snoozed, Sent, Drafts, Trash) with unread counts, smart folders, and per-account labels when scoped. Smart folder and per-label unread counts are scaffolded (return 0).

Thread detail (`core/src/db/queries_extra/thread_detail.rs`) - `get_thread_detail()` returns messages (with ownership detection, collapsed summaries, body text from body store), labels (with resolved colors), attachments (with message context), and attachment collapse state for a single thread.

## Gotchas that will break your code

Never run squeeze against `fixtures/5.pdf`. It's a 220MB PDF that pegs all CPU cores and freezes the user's machine. When testing squeeze on the PDF fixtures, exclude 5.pdf explicitly - use 2.pdf, 3.pdf, 9.pdf, or 14.pdf instead.

Multiple content stores (`crates/stores/`): Message bodies live outside the main `messages` table in `bodies.db` (compressed), and inline multipart images have their own attachment database. Use `BodyStoreState` / `InlineImageStoreState` rather than assuming message content is in the main SQLite database. The attachment file cache is also in this crate.

Four email providers: `gmail_api`, `jmap`, `graph` (Microsoft), `imap`. All unified behind the `ProviderOps` trait (`common/src/ops.rs`). Folder-accepting methods use `&FolderId`, tag-accepting methods use `&TagId` (`common/src/typed_ids.rs`). Typed IDs flow from `MailActionIntent` through `MailOperation` to the provider - no raw string boundaries in the action pipeline.

Action pipeline: `MailActionIntent → resolve_intent() → build_execution_plan() → batch_execute() → handle_action_completed()`. All 12 action types flow through one path. `MailOperation` (core) is the canonical execution type. `CompletionBehavior` (app) drives toast, auto-advance, and undo via exhaustive match. See `docs/architecture.md` § "Adding a New Email Action" for the checklist.

Generation counters use branded tokens: `GenerationCounter<T>` / `GenerationToken<T>` in `core/src/generation.rs`. `next()` is the only way to get a token (bumps and returns). `#[must_use]` on `next()` - use `let _ = counter.next()` for invalidation-only bumps. Phantom type brands prevent cross-counter comparison. See `docs/architecture.md` for the full pattern.

Core crate boundary: Business logic belongs in `rtsk`. The app crate calls core functions directly (no command wrappers needed - the Tauri app shell has been removed). When adding new core functionality, add it to `crates/core/src/`.

iced is depended on in 3 places: `crates/app/Cargo.toml` (full iced umbrella), `crates/rte/Cargo.toml` (iced umbrella, optional behind `widget` feature), and `crates/iced-drop/Cargo.toml` (iced_core + iced_widget + iced_runtime individually). All three must point to the same iced source. When switching between the git URL and local path, update all three.

## `jmap-client` crate gotchas

These are non-obvious behaviors of the `jmap-client` crate that will matter if the code is modified:

- Getting all mailboxes: `mailbox_get(id, props)` fetches ONE mailbox. To get all, use the builder: `MailboxGet::new(&account_id)` with no IDs set, submitted via `request.call(get)`. See `sync/mailbox.rs:fetch_all_mailboxes_for()`.
- `mb.role()` returns `Role` directly (not `Option<Role>`). Compare with `Role::None` to check if unset.
- `mb.total_emails()` returns `usize` directly, not `Option<usize>`.
- `take_id()` / `take_list()` require `let mut` on the response object.
- Filter type inference: Rust can't infer the generic for `Some(filter.into())` in `email_query()`. Bind to an explicit type: `let filter: core::query::Filter<email::query::Filter> = ...;`
- `download(blob_id)` takes only the blob ID - NOT `(account_id, blob_id, name)`.
- `email_submission_create(email_id, identity_id)` needs an identity ID, not account ID. Fetch identities via builder pattern.
- `changes.created()/updated()/destroyed()` return `&[String]`, not `&[&str]`. Use `.map(String::as_str)` not `.copied()`.
- `fetch_text_body_values(true)` is accessed via `get_req.arguments().fetch_text_body_values(true)`, not directly on the get request.
- `mailbox_changes(since_state, 0)` - max_changes of 0 is invalid per JMAP spec. Use 500.

## Code Review (`review`)

`review` is a CLI tool that fans out code review requests to anchored AI sessions. Each archetype has a stored prime prompt (in `.review.toml` under `[_prime].<archetype>`) that defines the review lens. Configuration lives in `.review.toml` at the repo root.

Four archetypes: `security`, `bugs`, `perf`, `arch`.

Multiple archetypes go in one invocation as a comma list (e.g. `review bugs,arch,perf --oneshot`).

Use `--oneshot`. Prompt cache is ~1 hour, so starting a fresh session for each unrelated review is cheaper than resuming.

```bash
echo "review the new sync code" | review bugs --oneshot
# session: 019de...
# <findings>
```

Use `--session` for continuity within a thread.

## Encryption

AES-256-GCM (`crates/common/src/crypto.rs` for the cipher; key load lives in the dep-free `crates/crypto-key/` crate shared between `common` and `service`). Key file: `ratatoskr.key` (or legacy `velo.key`) in app data dir. Format: base64-encoded 32 bytes. Encrypted-value wire format: `base64(iv):base64(ct+tag)`.

Boot path: Service loads + validates the key during `BootPhase::LoadingKey`. A missing or unreadable key file is a fatal Service exit (`BootExitCode::KeyLoadFailure = 73`); there is no zero-key fallback. The `crypto-key` crate enforces TOCTOU-safe permission repair (`O_NOFOLLOW` + `fchmod` via the open fd), file-owner UID validation on Unix, and unconditional rejection of an all-zero key (which would silently downgrade AES-256-GCM to a known-public key); dev-seed writes a non-zero deterministic pattern so dev workflows pass that gate cleanly. Loaded keys are returned in a `SecretKey` wrapper that zeroizes its buffer on drop.
