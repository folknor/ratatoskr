# Architecture

Living reference for ratatoskr's architectural principles, boundaries, and settled patterns. Follow these when building new features or reviewing changes.

## Guiding Principle

**Make the right thing the only thing.** Correctness should not depend on every developer remembering a multi-step protocol. A single entry point, a type that enforces the invariant, or a compiler error when the protocol is violated - these are how contracts become real.

When evaluating a design: if adding a new call site can silently break existing behavior, the API is wrong.

## Crate Boundaries

**`rtsk`** is the facade. Business logic and domain orchestration live here - accounts, OAuth, discovery, email actions, calendar workflow, search routing, cloud-attachment orchestration. The app crate calls core functions directly.

**`db`** owns the main application SQLite schema and all shared-table SQL. Query shape, write shape, conflict resolution, and transaction-scoped shared-table persistence belong here. If multiple crates need to write the same table, `db` owns that write API.

**Provider crates** (`gmail`, `jmap`, `graph`, `imap`) each implement the `ProviderOps` trait (`common/src/ops.rs`). No provider-specific logic should leak into app or core beyond the trait surface.

For shared-table persistence, providers normalize protocol payloads into `db` write structs and call `db` APIs. They do not independently own SQL for shared tables like `messages`, `attachments`, `labels`, `contacts`, `threads`, or calendar tables.

**`store`** owns all content outside the main SQLite database: compressed body store (`bodies.db`), inline image store, attachment file cache. Never assume message content is in the main DB.

**`provider`** holds shared provider helpers: encryption (AES-256-GCM), email parsing, HTML sanitization.

**`app`** is the iced UI. Elm architecture (boot/update/view). It contains presentation logic only - no direct SQLite ownership, no provider calls, no business rules.

## Architectural Boundaries

### Shared-table SQL belongs to `db`

Shared application tables are not owned by `app`, `core`, `sync`, or provider crates. They are owned by `db`.

That includes:
- message/thread persistence
- attachment persistence
- label and folder persistence
- shared contact persistence
- shared calendar persistence

**Enforcement:** `app` no longer depends on `rusqlite`. `core` no longer depends on `rusqlite`. Provider and sync crates now route shared-table writes through `db` APIs instead of embedding their own SQL for those tables.

### Action service as mutation gate

Every email state mutation (archive, delete, star, move, label, etc.) must flow through `core::actions::*`. This is the single path that coordinates local DB mutation + provider dispatch + pending-ops + undo tokens + in-flight guards.

**Enforcement:** The 7 thread-action DB helpers (`set_thread_read`, `set_thread_starred`, `set_thread_pinned`, `set_thread_muted`, `delete_thread`, `add_thread_label`, `remove_thread_label`) are `pub(crate)` - the app crate cannot call them directly. New email-action DB helpers should follow the same pattern.

### Provider trait as abstraction layer

The four providers are unified behind `ProviderOps`. All provider-specific behavior is behind this trait - callers should never branch on provider type.

**Enforcement:** `FolderId` and `TagId` newtypes in `crates/common/src/typed_ids.rs`. The `ProviderOps` trait uses `&FolderId` for `move_to_folder`, `rename_folder`, `delete_folder` and `&TagId` for `add_tag`, `remove_tag`. Passing a folder ID where a tag ID is expected is a compile error. Typed IDs flow from `MailActionIntent` through `MailOperation` through `batch_execute` to the provider - no raw string boundaries except JSON deserialization in `pending.rs` and `CommandArgs` in the palette crate.

For persistence, the provider boundary is:
- providers fetch and translate protocol payloads
- `db` owns shared-table writes
- provider-local sync tables are explicit exceptions, not accidental ownership of shared schema

### Scope as a single source of truth

The active scope (which account, shared mailbox, or public folder the user is looking at) must be consistent across sidebar, navigation context, and all DB queries.

**Enforcement:** `ViewScope` enum (`AllAccounts`, `Account`, `SharedMailbox`, `PublicFolder`) in `crates/core/src/scope.rs`. The sidebar stores `selected_scope: ViewScope` as the single source of truth. `fire_navigation_load()` and `load_threads_for_current_view()` dispatch on the enum - shared mailboxes and public folders use dedicated query paths, personal accounts use `AccountScope`-based queries. Shared mailbox threads are distinguished by `threads.shared_mailbox_id`; personal queries filter `shared_mailbox_id IS NULL`. Public folder items come from the separate `public_folder_items` table. Actions are gated for public folder scope.

### Generation counters for async safety

Async loads (nav, threads, search, etc.) must not overwrite fresher state. Each load site captures a generation counter before dispatch and checks it on completion - stale results are discarded.

**Enforcement:** `GenerationCounter<T>` and `GenerationToken<T>` types in `crates/core/src/generation.rs`. Phantom type brands prevent cross-counter token comparison at compile time. `next()` is the only way to get a token (`#[must_use]` - use `let _ =` for invalidation-only bumps). `is_current()` is the only way to check freshness. All 9 counters are migrated: App-level (`Nav`, `ThreadDetail`, `Search`, `PopOut`) and component-level (`Calendar`, `PaletteOptions`, `Typeahead`, `AddAccount`, `Autocomplete`).

### Calendar workflow state owns meaning

Calendar state is split into four layers:
- view/navigation state
- workflow state
- editor session state
- surface state

Workflow state is authoritative for lifecycle meaning and identity. The editor session is the single source of truth for editable event state. Surface state (`CalendarPopover`, `CalendarModal`) is derived from workflow state and is never used to recover workflow semantics.

**Enforcement:** handlers update workflow first and then synchronize surfaces. Editable event data is read from the editor session, not from `active_modal`.

### Folder vs label semantics are explicit

Ratatoskr has exactly two persisted sidebar concepts:
- folders: `label_kind = "container"`
- labels: `label_kind = "tag"`

The `labels` table stores both. Provider-native concepts must be normalized into these two semantics before persistence. System folders use canonical Ratatoskr IDs (`INBOX`, `SENT`, `TRASH`, etc.), not provider-native IDs.

**Enforcement:** provider label/folder sync paths map their payloads into shared `db` label writes with explicit `label_kind`. Sidebar/navigation code branches on `label_kind` rather than provider-specific heuristics.

## Adding a New Email Action

The action pipeline flows: `MailActionIntent → resolve_intent → build_execution_plan → batch_execute → handle_action_completed`. Adding a new action requires:

1. **Variant in `MailActionIntent`** (`action_resolve.rs`) - the user intent
2. **Variant in `MailOperation`** (`core/actions/operation.rs`) - the core execution type
3. **Arm in `resolve_intent()`** - collapses intent + UI context into operation + compensation
4. **Arm in `completion_behavior()`** - defines view effect, post-success effect, undo behavior, toast label. Compiler-enforced exhaustive match.
5. **Core action function** (e.g., `core/actions/my_action.rs`) - local DB mutation + provider dispatch
6. **Arms in `batch.rs` routing** (`dispatch_with_provider`, `op_local`, `enqueue_params`, `op_name`) - route `MailOperation` to the action function
7. **`MailUndoPayload` variant + compensation arm** (`action_resolve.rs`, `commands.rs`) - if reversible

**Enforcement:** `MailOperation` is an exhaustive enum. Adding a variant produces compiler errors in `completion_behavior()`, `dispatch_with_provider`, `op_local`, `enqueue_params`, `op_name`, and `build_standard_undo_payloads`. No wildcards - you cannot silently miss a dispatch site.

Toggle actions (boolean state flips) need only the `MailOperation` variant and a `ToggleField` entry - `build_execution_plan` handles per-thread resolution, optimistic UI, and rollback generically.

## Database Integrity

All tables with `account_id` CASCADE on account deletion. Migration 77 recreated the 16 tables that were missing the constraint. `delete_account_orchestrate()` handles external store cleanup (body store, inline images, attachment cache, search index).

## Settled Patterns

These are verified, adopted project-wide, and should be followed for all new work.

**Generational load tracking** - 9 branded `GenerationCounter<T>` instances across App and component levels. See "Generation counters for async safety" above.

**Component trait** - 7 components: Sidebar, ThreadList, ReadingPane, Settings, StatusBar, AddAccountWizard, Palette. Non-components (Compose, Calendar, Pop-out windows) use free functions + App handler methods.

**Token-to-Catalog theming** - All styling goes through the theme catalog. No inline color closures. Exceptions: rich text editor (builder methods), token input (renderer.fill_quad).

**Config shadow pattern** - Formal: `PreferencesState`. Implicit (clone-on-open): Account editor, Contact editor, Group editor, Calendar event editor, Signature editor. Editors work on a shadow copy and commit on save.

**`ProgressReporter` trait** - All event emission from core goes through `&dyn ProgressReporter`. The app provides its own implementation.

**State types are `Clone`** - `DbState`, `BodyStoreState`, `InlineImageStoreState`, `SearchState`, `AppCryptoState` all wrap `Arc<Mutex<_>>` and implement `Clone`.

## Current Exceptions

These are intentional unresolved areas, not reasons to bypass the boundaries above.

- **Signatures** are not yet a settled architecture surface. Gmail and JMAP signature sync/write behavior exists, but the product/spec is not finalized enough to treat that path as a completed shared persistence contract.
- **Provider-local sync/state tables** may still live in provider or sync crates. That is acceptable only for provider-owned or protocol-owned state, not for shared application tables. The ownership is now explicit:
  - **Provider-owned mapping/state tables** stay with the provider logic that interprets them:
    - Gmail: `google_contact_map`, `google_other_contact_map`
    - Graph: `graph_contact_map`, `graph_subscriptions`
    - JMAP: `jmap_push_state`
  - **Sync-owned protocol coordination tables** stay with sync/protocol helpers until there is a clear benefit to moving them behind narrow `db` APIs:
    - `jmap_sync_state`
    - `graph_folder_delta_tokens`
    - `graph_contact_delta_tokens`
    - `graph_shared_mailbox_delta_tokens`
    - `shared_mailbox_sync_state`
    - `folder_sync_state`
    - `public_folder_sync_state`
  - These tables are exceptions because they track provider protocol state, cursors, subscriptions, or mapping identity. They do not make the provider or sync crates owners of shared application tables like `messages`, `labels`, `contacts`, or calendar rows.
