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

**Enforcement:** `FolderId` and `TagId` newtypes in `crates/provider-utils/src/typed_ids.rs`. The `ProviderOps` trait uses `&FolderId` for `move_to_folder`, `rename_folder`, `delete_folder` and `&TagId` for `add_tag`, `remove_tag`. Passing a folder ID where a tag ID is expected is a compile error. The action layer constructs typed IDs at the provider call boundary.

### Scope as a single source of truth

The active scope (which account, shared mailbox, or public folder the user is looking at) must be consistent across sidebar, navigation context, and all DB queries.

**Enforcement:** `ViewScope` enum (`AllAccounts`, `Account`, `SharedMailbox`, `PublicFolder`) in `crates/core/src/scope.rs`. The sidebar stores `selected_scope: ViewScope` as the single source of truth. `fire_navigation_load()` and `load_threads_for_current_view()` dispatch on the enum — shared mailboxes and public folders use dedicated query paths, personal accounts use `AccountScope`-based queries. Shared mailbox threads are distinguished by `threads.shared_mailbox_id`; personal queries filter `shared_mailbox_id IS NULL`. Public folder items come from the separate `public_folder_items` table. Actions are gated for public folder scope.

### Generation counters for async safety

Async loads (nav, threads, search, etc.) must not overwrite fresher state. Each load site captures a generation counter before dispatch and checks it on completion — stale results are discarded.

**Enforcement:** `GenerationCounter` and `GenerationToken` types in `crates/core/src/generation.rs`. The counter's `next()` method is the only way to get a token (forces the bump), and `is_current()` is the only way to check (forces the comparison). Message variants carry `GenerationToken` instead of raw `u64`, so the type system prevents mixing up counter values. The 4 App-level counters (`nav_generation`, `thread_generation`, `search_generation`, `pop_out_generation`) all use this type.

## Adding a New Email Action

Adding a new action requires 8 coordinated edits: `EmailAction` variant, `CompletedAction` variant (with `removes_from_view`, `success_label`), `BatchAction` variant, `to_batch_action` mapping, `handle_action_completed` arm, `UndoToken` variant, undo dispatch arm, `handle_email_action` arm.

**Enforcement:** All match arms on `CompletedAction` are exhaustive — no wildcards. Adding a new variant produces compiler errors at every dispatch site (`to_batch_action`, `to_toggle_batch`, `rollback_toggles`, `produce_undo_tokens`, `success_label`, `removes_from_view`). The 8-edit protocol still exists, but you can't silently miss a step.

## Database Integrity

All tables with `account_id` CASCADE on account deletion. Migration 77 recreated the 16 tables that were missing the constraint. `delete_account_orchestrate()` handles external store cleanup (body store, inline images, attachment cache, search index).

## Settled Patterns

These are verified, adopted project-wide, and should be followed for all new work.

**Generational load tracking** — Applied to: nav, thread, search, palette, pop-out, sync, autocomplete, add-account wizard, calendar events, search typeahead.

**Component trait** — 7 components: Sidebar, ThreadList, ReadingPane, Settings, StatusBar, AddAccountWizard, Palette. Non-components (Compose, Calendar, Pop-out windows) use free functions + App handler methods.

**Token-to-Catalog theming** — All styling goes through the theme catalog. No inline color closures. Exceptions: rich text editor (builder methods), token input (renderer.fill_quad).

**Config shadow pattern** — Formal: `PreferencesState`. Implicit (clone-on-open): Account editor, Contact editor, Group editor, Calendar event editor, Signature editor. Editors work on a shadow copy and commit on save.

**`ProgressReporter` trait** — All event emission from core goes through `&dyn ProgressReporter`. The app provides its own implementation.

**State types are `Clone`** — `DbState`, `BodyStoreState`, `InlineImageStoreState`, `SearchState`, `AppCryptoState` all wrap `Arc<Mutex<_>>` and implement `Clone`.
