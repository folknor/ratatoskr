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

There are no ratatoskr-owned provider crates. Every mail protocol (Gmail, JMAP, Graph, IMAP) is implemented in the external `bifrost` workspace and reached exclusively through the resident `SyncEngine` from `crates/service/src/bifrost/`; `app` and `rtsk` never depend on a protocol client directly.

Bifrost is a SERVICE-side dependency, and that boundary is machine-enforced by the `app-no-bifrost` / `core-no-bifrost` rules in `brokkr.toml`, not just convention: the app depends on `rtsk` plus `service-api` wire types only, so a bifrost type reaching `rtsk` would be pulled into the UI build. The service-to-app IPC contract is the firewall - `AccountError` never crosses it. `action-types` and `cal` carry `bifrost-types` alone, deliberately, for the `CalendarAccountOpener` seam. Bifrost and the `saehrimnir` mock are sibling repos we own and change ourselves; the procedure for doing so is `docs/side-quests.md`.

## Required reading

Read the doc before starting work in its area. Subagents launched for these tasks must include the relevant doc in their required-reading list.

- Any UI work - `UI.md` at the repo root.
- Architectural decisions, crate boundaries, new email actions, generation counters, scope wiring, calendar workflow layering, bifrost `Account`/`SyncEngine` surface additions - `reference/architecture.md`.
- Anything touching (email provider) folders, labels, the `labels` table, `thread_labels`, `label_kind`, system folder IDs (`INBOX`, `TRASH`, `SPAM`, `SENT`, `DRAFT`, `archive`, `STARRED`), or provider folder/label sync - `reference/glossary/folders-labels.md`.
- Adding or refactoring tooltips, dropdowns, context menus, popovers, modals, sheets, or any new overlay-like surface - `reference/glossary/overlay-surfaces.md`.
- Service test harness, sync-harness scripts, harness Lua bindings, `app --test-harness`, `dellingr` VM, `brokkr service-test`/`service-suite`/`sync`, gate baselines, or anything touching `crates/app/tests/service-harness/` or `crates/app/tests/sync-harness/` - `reference/glossary/harness.md`.
- Any change to `bifrost` or `saehrimnir` (the sibling dependency repos), or any ratatoskr work blocked on one - `docs/side-quests.md`. Read it BEFORE editing anything under `./research/`, `../bifrost`, or `../sæhrimnir`.

## Rules

### General rules

- Don't use gremlins! Em-dash, en-dash, strange quotes, whatever - they're all verboten.
- Don't remind the user of the rules. They wrote them, so they know them.
- The user can exempt you from any rule at any time.

### Behavioral gates

- A green `brokkr check` is necessary but NOT sufficient for anything touching sync, actions, calendar, or contacts. It proves the code compiles and unit tests pass, not that real provider sync still behaves. Changes in those areas name and run the relevant `brokkr service-test` / sync-harness scripts, plus `brokkr sync --bench` where performance is in scope. A change satisfiable by a compile-only replacement is under-gated.
- No parallel hand-rolled dependency may survive alongside a bifrost equivalent. `scripts/b15-audit.sh` mechanically walks every crate's manifest and module tree for dependencies with a bifrost equivalent, modules claiming provider transport duty, and dead `RATATOSKR_TEST_*` consumers. Re-run it after any deletion or dependency work; every flag is either deleted or retained with a stated rationale.

### Bash rules

- Never read or write from `/tmp`. All data lives in the project.
- Never run raw `cargo`, `curl`, `pkill`. Use `brokkr`.

## Commands

Use `brokkr` (not `cargo`) for check/test. By default output is filtered to changed files and capped at 20 diagnostics per phase.

- `brokkr check` - gremlins + clippy + all tests (changed-files scope)
- `brokkr check --all` - show every diagnostic, no cap, no scope filter
- `brokkr check -p <crate>` - scope to one package (e.g. `-p app`). You generally do not want to run this; a single `brokkr check` is faster than 2-3 `-p` runs, and brokkr intelligently filters which warnings and errors to show you
- `brokkr check -- --test <file>` - forward args to `cargo test` (args after the second `--` go to the test binary)
- `brokkr test -p <crate> <NAME>` - release-mode focused single-test runner. Always passes `--release --include-ignored --nocapture --test-threads=1`. `<NAME>` is a case-sensitive substring filter (matches both unit and integration tests). Streams the test's own stdout/stderr live and prints a `[test] PASS/FAIL` footer with wall time. Defaults to `--all-features`; runs a second sweep if `[check].consumer_features` is set in `brokkr.toml`. Gated off for litehtml/sluggrs (use `brokkr visual` there).
  - `-p, --package <PKG>` - cargo package. Required in this workspace - no default package, and overrides `[test] default_package` in `brokkr.toml` if set.
  - `-N, --repeat <N>` - run the test N times per sweep (flaky-test hunting).
  - `-j, --jobs <N>` - parallel cargo compile jobs.
  - `--raw` - bypass output filtering, print everything cargo emits.
  - `--debug` - build and run the test in dev profile instead of release. Use this for subprocess-lifecycle / IPC / boot-path tests where release-LTO compile time (3-4 min for the full workspace) dominates wall time and the optimization level doesn't change the behavior under test. `BROKKR_TEST_BIN_DIR` points at `<target>/debug` accordingly.
  - Example: `brokkr test -p common truncates_without_splitting` or `brokkr test -p calendar extract_tag_value_flattens_nested_text -N 5` or `brokkr test -p app terminal_failure_at_initial_boot_does_not_respawn --debug`.
- `cargo run -p app` - run the iced app

### brokkr baselines and gate.db

- brokkr writes TWO databases with DIFFERENT dirty-tree policies. `.brokkr/results.db`
  stores nothing on a dirty tree; `.brokkr/ratatoskr/gate.db` records EVERY gated run by
  design, so a failure stays inspectable. `--force` therefore does write `gate.db`, and
  always has - the row is tagged dirty. A dirty-tagged baseline is real and usable; it is
  not lost work and does not need re-recording for the numbers to exist.
- brokkr NEVER writes `brokkr.toml`. `--as-baseline` prints a UUID and a TOML line for you
  to paste. Editing that file is yours, so paste by REPLACING the existing host key in the
  block - never by anchoring an insert on the `[...baseline]` header. A pre-existing
  `plantasjen = "..."` often sits below a comment; a header-anchored insert sails past it
  and produces a duplicate key, and one duplicate makes the whole file unparseable, which
  breaks every gate-config-reading brokkr command, not just the gate you touched.

## Harness

Lua Service harness scripts live under `crates/app/tests/service-harness/`.
Sync harness scripts live under `crates/app/tests/sync-harness/`.

- `brokkr service-test <SCRIPT>` - run one Service harness script.
- `brokkr service-test <DIR> -N <N>` - run a cohort directory; `-N`
  means cohort cycles.
- `brokkr service-suite [--filter X]` - run the discovered Service
  harness suite, optionally filtered.
- `brokkr service-list` - list scripts and parsed frontmatter.
- `brokkr sync` - list discovered sync-harness scripts.
- `brokkr sync <SCRIPT>` - run one, PASS/FAIL.
- `brokkr sync --all [--filter X] [--include-ignored]` - run every
  discovered script.
- `brokkr sync <SCRIPT> --bench [N]` - measure one (N defaults to 3).
- `brokkr sync --gate all --bench [N]` - sweep every configured gate.

`brokkr.toml` has two ratatoskr sections:

- `[ratatoskr.harness]` describes the orchestration build: `package`
  to build (defaults to also being the spawned `binary`) and `debug`
  to pick the dev profile. Self-contained; not referenced by bare
  `brokkr check`.
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

Scoped queries (`core/src/db/queries_extra/scoped_queries.rs`) - Cross-account query infrastructure. `ViewScope` enum (`AllAccounts`/`Account`/`SharedMailbox`/`PublicFolder`) in `core/src/scope.rs` is the sidebar's single source of truth. Personal-account queries use `AccountScope` internally and filter `t.namespace_kind IS NULL`. Shared mailbox scopes route through the dedicated `get_threads_for_shared_mailbox{,_starred,_snoozed,_label_group}` functions; public folder scopes route through `get_threads_for_namespace`, keyed on `(namespace_kind, namespace_id)`. Predicate-based virtual folder queries for Starred/Snoozed use boolean flags on `threads`, not label joins. Draft counts include `local_drafts` table.

Navigation state (`core/src/db/queries_extra/navigation.rs`) - `get_navigation_state()` returns the full sidebar state in one call: universal folders (Inbox, Starred, Snoozed, Sent, Drafts, Trash) with unread counts, smart folders (real unread counts via `count_smart_folder_unread` per folder - N+1 today, see `docs/search/implementation-spec.md` § Known semantic issues), and per-account labels when scoped. Per-label unread counts are scaffolded (return 0).

Thread detail (`core/src/db/queries_extra/thread_detail.rs`) - `get_thread_detail()` returns messages (with ownership detection, collapsed summaries, body text from body store), labels (with resolved colors), attachments (with message context), and attachment collapse state for a single thread.

## Gotchas that will break your code

Never run squeeze against `fixtures/5.pdf`. It's a 220MB PDF that pegs all CPU cores and freezes the user's machine. When testing squeeze on the PDF fixtures, exclude 5.pdf explicitly - use 2.pdf, 3.pdf, 9.pdf, or 14.pdf instead.

Multiple content stores (`crates/stores/`): Message bodies live outside the main `messages` table in `bodies.db` (compressed), and inline multipart images have their own attachment database. Use `BodyStoreState` / `InlineImageStoreState` rather than assuming message content is in the main SQLite database. The attachment file cache is also in this crate.

Mail providers are reached only through the resident bifrost `SyncEngine` and
its protocol crates. Typed IDs flow from `MailActionIntent` through
`MailOperation` to bifrost operations; legacy provider crates and `ProviderOps`
are gone.

Action pipeline: `MailActionIntent → resolve_intent() → build_execution_plan() → batch_execute() → handle_action_completed()`. All 12 action types flow through one path. `MailOperation` (core) is the canonical execution type. `CompletionBehavior` (app) drives toast, auto-advance, and undo via exhaustive match. See `reference/architecture.md` § "Adding a New Email Action" for the checklist.

Generation counters use branded tokens: `GenerationCounter<T>` / `GenerationToken<T>` in `core/src/generation.rs`. `next()` is the only way to get a token (bumps and returns). `#[must_use]` on `next()` - use `let _ = counter.next()` for invalidation-only bumps. Phantom type brands prevent cross-counter comparison. See `reference/architecture.md` for the full pattern.

Core crate boundary: Business logic belongs in `rtsk`. The app crate calls core functions directly (no command wrappers needed - the Tauri app shell has been removed). When adding new core functionality, add it to `crates/core/src/`.

iced is depended on in 3 places: `crates/app/Cargo.toml` (full iced umbrella), `crates/rte/Cargo.toml` (iced umbrella, optional behind `widget` feature), and `crates/iced-drop/Cargo.toml` (iced_core + iced_widget + iced_runtime individually). All three must point to the same iced source. When switching between the git URL and local path, update all three.

## Encryption

AES-256-GCM (`crates/common/src/crypto.rs` for the cipher; key load lives in the dep-free `crates/crypto-key/` crate shared between `common` and `service`). Key file: `ratatoskr.key` (or legacy `velo.key`) in app data dir. Format: base64-encoded 32 bytes. Encrypted-value wire format: `base64(iv):base64(ct+tag)`.

Boot path: Service loads + validates the key during `BootPhase::LoadingKey`. A missing or unreadable key file is a fatal Service exit (`BootExitCode::KeyLoadFailure = 73`); there is no zero-key fallback. The `crypto-key` crate enforces TOCTOU-safe permission repair (`O_NOFOLLOW` + `fchmod` via the open fd), file-owner UID validation on Unix, and unconditional rejection of an all-zero key (which would silently downgrade AES-256-GCM to a known-public key); dev-seed writes a non-zero deterministic pattern so dev workflows pass that gate cleanly. Loaded keys are returned in a `SecretKey` wrapper that zeroizes its buffer on drop.
