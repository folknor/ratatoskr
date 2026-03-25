# Architecture

Living reference for ratatoskr's architectural principles, boundaries, and settled patterns. Follow these when building new features or reviewing changes.

## Guiding Principle

**Make the right thing the only thing.** Correctness should not depend on every developer remembering a multi-step protocol. A single entry point, a type that enforces the invariant, or a compiler error when the protocol is violated — these are how contracts become real.

When evaluating a design: if adding a new call site can silently break existing behavior, the API is wrong.

## Crate Boundaries

**`ratatoskr-core`** is the facade. All business logic lives here — accounts, OAuth, discovery, email actions, DB queries, cloud attachments. The app crate calls core functions directly.

**Provider crates** (`gmail`, `jmap`, `graph`, `imap`) each implement the `ProviderOps` trait (`core/src/provider/ops.rs`). No provider-specific logic should leak into app or core beyond the trait surface.

**`ratatoskr-stores`** owns all content outside the main SQLite database: zstd-compressed body store (`bodies.db`), inline image store, attachment file cache. Never assume message content is in the main DB.

**`ratatoskr-provider-utils`** holds shared provider helpers: encryption (AES-256-GCM), email parsing, HTML sanitization.

**`app`** is the iced UI. Elm architecture (boot/update/view). It should contain presentation logic only — no direct DB writes, no provider calls, no business rules.

## Architectural Boundaries

### Action service as mutation gate

Every email state mutation (archive, delete, star, move, label, etc.) must flow through `core::actions::*`. This is the single path that coordinates local DB mutation + provider dispatch + pending-ops + undo tokens + in-flight guards.

**Enforcement:** The 7 thread-action DB helpers (`set_thread_read`, `set_thread_starred`, `set_thread_pinned`, `set_thread_muted`, `delete_thread`, `add_thread_label`, `remove_thread_label`) are `pub(crate)` — the app crate cannot call them directly. New email-action DB helpers should follow the same pattern.

### Provider trait as abstraction layer

The four providers are unified behind `ProviderOps`. All provider-specific behavior is behind this trait — callers should never branch on provider type.

**Current weakness:** Provider APIs are stringly typed. `move_to_folder(&str)`, `add_tag(&str)` take raw strings — callers must know whether they're passing a folder ID or tag ID. Wrong string kind compiles fine and may silently do the wrong thing.

**Target state:** Typed IDs (`FolderId`, `TagId`) and capability markers instead of raw `&str`.

### Scope as a single source of truth

The active scope (which account, shared mailbox, or public folder the user is looking at) must be consistent across sidebar, navigation context, and all DB queries.

**Current weakness:** Scope state is split across `selected_account`, `selected_shared_mailbox`, `navigation_target`, `selected_label`. The `current_scope()` function only reads `selected_account` — shared mailbox and public folder selection is ignored.

**Target state:** A single `Scope` enum (`All`, `Account(id)`, `SharedMailbox(id)`, `PublicFolder(id)`) consumed by all query and context builders.

### Generation counters for async safety

Async loads (nav, threads, search, etc.) must not overwrite fresher state. Each load site captures a generation counter before dispatch and checks it on completion — stale results are discarded.

**Current weakness:** The pattern is convention-based across 5+ counters (`nav_generation`, `thread_generation`, `search_generation`, etc.). Nothing prevents a new load site from forgetting to bump or check.

**Target state:** Typed loader helpers that allocate and validate generations automatically.

## Adding a New Email Action

Adding a new action currently requires 8 coordinated edits:

1. `EmailAction` variant
2. `CompletedAction` variant (with `removes_from_view`, `success_label`)
3. `BatchAction` variant
4. `to_batch_action` mapping
5. `handle_action_completed` arm
6. `UndoToken` variant
7. Undo dispatch arm
8. `handle_email_action` arm

Missing any one silently degrades (no undo, wrong toast, etc.) because wildcard arms return `None`/empty instead of failing.

**Target state:** A single action descriptor (table or derive macro) that generates classification, batch mapping, rollback policy, and undo mapping from one definition.

## Database Integrity

Tables with `account_id` should CASCADE on account deletion. ~10 tables added in later migrations have `account_id TEXT NOT NULL` without the FK constraint, leaving orphan rows on account deletion. The `delete_account_orchestrate()` function mitigates the worst effects by cleaning external stores explicitly, but the missing CASCADEs still leave main-DB orphans in tables like `cloud_attachments`, `folder_sync_state`, `shared_mailbox_sync_state`.

## Settled Patterns

These are verified, adopted project-wide, and should be followed for all new work.

**Generational load tracking** — Applied to: nav, thread, search, palette, pop-out, sync, autocomplete, add-account wizard, calendar events, search typeahead.

**Component trait** — 7 components: Sidebar, ThreadList, ReadingPane, Settings, StatusBar, AddAccountWizard, Palette. Non-components (Compose, Calendar, Pop-out windows) use free functions + App handler methods.

**Token-to-Catalog theming** — All styling goes through the theme catalog. No inline color closures. Exceptions: rich text editor (builder methods), token input (renderer.fill_quad).

**Config shadow pattern** — Formal: `PreferencesState`. Implicit (clone-on-open): Account editor, Contact editor, Group editor, Calendar event editor, Signature editor. Editors work on a shadow copy and commit on save.

**`ProgressReporter` trait** — All event emission from core goes through `&dyn ProgressReporter`. The app provides its own implementation.

**State types are `Clone`** — `DbState`, `BodyStoreState`, `InlineImageStoreState`, `SearchState`, `AppCryptoState` all wrap `Arc<Mutex<_>>` and implement `Clone`.
