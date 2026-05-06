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

Every email state mutation (archive, delete, star, move, label, send, snooze, mark-chat-read, etc.) must flow through the action service. As of Phase 2 the *execution* surface lives in `service::actions::*` (the relocated home; `core::actions` keeps a shim that re-exports the public API). UI handlers no longer call the execution functions directly - they build an `ActionExecutionPlan`, convert to `ActionWirePlan`, and dispatch via `client.execute_plan(...)`. The Service journals the plan, signals the worker, and per-operation `OperationOutcome` + final `ActionCompleted` notifications stream back over IPC.

**Enforcement:**
- **Crate boundary** (`docs/service/problem-statement.md` § "Type-level enforcement"). `WriteDbState` is constructed in the `service-state` crate; `app` does not depend on `service-state`, so a UI source file that tries `use service_state::WriteDbState` fails to resolve. Phase 6a-part-2 deletes `Db::with_write_conn`, `Db::with_write_conn_sync`, and `Db::write_db_state` from the app crate; the single residual write-conn accessor is `Db::phase_6c_pending_write_state`, used only to construct the contacts handlers' `ActionContext` at `app.rs::from_boot_ready`. The contacts pipeline migration is tracked for Phase 6d/8. Phase 6b's lockdown integration test (`crates/service-state/tests/lockdown.rs`) asserts that `crates/app/Cargo.toml` does not list `service-state` as a direct dependency; Phase 6c-11 layered the transitive lockdown that asserts `app` cannot reach `cal` through any path-dep chain (the regression class Phase 6c closed by relocating `cal::actions` Service-side via `CalendarActionContext` + `cal_action.execute_plan` IPC).

### Calendar action pipeline

Calendar event mutations (create, update, delete) flow through a sibling pipeline added in Phase 6c: `CalendarOperation -> cal_action.execute_plan IPC -> service::cal_actions::batch_execute -> CalendarOperationOutcome notifications -> CalendarActionCompleted -> handle_calendar_action_completed`. The two pipelines share the `action_jobs` journal via the `kind` CHECK constraint (Phase 6c-1 widened it to include `'calendar_plan'`) and the per-op lease + recovery contract; the worker (Phase 6c-7) reads `action_jobs.kind` on each lease and dispatches to the right per-kind pipeline. Calendar uses a sibling `CalendarActionContext` (writer-half + encryption key) rather than the email-shared `ActionContext`; `cal::actions::*` write surface is reachable only Service-side. The UI awaits `CalendarActionCompleted` via `ServiceClient::pending_calendar_actions` (Phase 6c-9), which uses the same Pending-or-latched-Completed shape as `pending_calendars` for the late-subscriber race. Calendar plans are 1:1 today (one user intent = one operation); Phase 6d revisits if RSVP / series-vs-occurrence semantics introduce N-op plans.
- **Thread-action DB helpers** (`set_thread_read`, `set_thread_starred`, `set_thread_pinned`, `set_thread_muted`, `delete_thread`, `add_thread_label`, `remove_thread_label`) are `pub(crate)` - the app crate cannot call them directly.
- **`ActionProviderCtx`** (`crates/common/src/types.rs`) carries only `account_id` / `&ReadDbState` / `&dyn ProgressReporter` - no `&SearchState` field, so action methods cannot write to the Tantivy index. A regression test in `common::types::tests` enforces the exhaustive-destructure shape.
- **Tri-state in-flight tracking** (`crates/app/src/app.rs::PlanState`). UI plans live as `Pending` / `Acked` / `AckUnknown`; `ServiceCrashed` while `Pending` defers rollback to post-respawn `action.job_status` reconciliation, while `ServiceCrashed` while `Acked` does nothing because the journal will replay.

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

The action pipeline flows: `MailActionIntent → resolve_intent → build_execution_plan → ActionWirePlan → action.execute_plan IPC → service::actions::batch_execute → OperationOutcome notifications → handle_action_completed`. Adding a new action requires:

1. **Variant in `MailActionIntent`** (`crates/app/src/action_resolve.rs`) - the user intent.
2. **Variant in `MailOperation`** (`crates/service/src/actions/operation.rs`) - the canonical execution type.
3. **Variant in `WireMailOperation`** (`crates/service-api/src/action.rs`) - the serializable wire mirror, 1:1 with `MailOperation`.
4. **Arm in `to_wire_op` / `wire_to_mail`** (`crates/app/src/action_wire.rs` and `crates/service/src/actions/wire_conversion.rs`) - exhaustive matches catch a missing mirror at compile time.
5. **Arm in `resolve_intent()`** - collapses intent + UI context into operation + compensation.
6. **Arm in `completion_behavior()`** - defines view effect, post-success effect, undo behavior, toast label. Compiler-enforced exhaustive match.
7. **Service-side action function** (e.g., `crates/service/src/actions/my_action.rs`) - local DB mutation + provider dispatch.
8. **Arms in `batch.rs` routing** (`dispatch_with_provider`, `op_local`, `enqueue_params`, `op_name`) - route `MailOperation` to the action function.
9. **`MailUndoPayload` variant + compensation arm** (`action_resolve.rs`, `commands.rs::undo_payload_to_ops`) - if reversible. Phase 2's undo path goes through `dispatch_plan_with_undo`, which dispatches the inverse plan via the standard `action.execute_plan` IPC.

**Enforcement:** `MailOperation` is an exhaustive enum, mirrored 1:1 by `WireMailOperation`. Adding a variant produces compiler errors in `completion_behavior()`, `dispatch_with_provider`, `op_local`, `enqueue_params`, `op_name`, `to_wire_op`, `wire_to_mail`, and `build_standard_undo_payloads`. No wildcards - a missing site is a compile error.

Toggle actions (boolean state flips) need only the `MailOperation` variant and a `ToggleField` entry - `build_execution_plan` handles per-thread resolution, optimistic UI, and rollback generically.

## Database Integrity

All tables with `account_id` CASCADE on account deletion. Migration 77 recreated the 16 tables that were missing the constraint. `delete_account_orchestrate()` handles external store cleanup (body store, inline images, attachment cache, search index).

## Settled Patterns

These are verified, adopted project-wide, and should be followed for all new work.

**Generational load tracking** - 9 branded `GenerationCounter<T>` instances across App and component levels. See "Generation counters for async safety" above.

**Component trait** - 8 components: Sidebar, ThreadList, ReadingPane, Settings, StatusBar, AddAccountWizard, Palette, ChatTimeline. Non-components (Compose, Calendar, Pop-out windows) use free functions + App handler methods.

**Token-to-Catalog theming** - All styling goes through the theme catalog. No inline color closures. Exceptions: rich text editor (builder methods), token input (renderer.fill_quad).

**Config shadow pattern** - Formal: `PreferencesState`. Implicit (clone-on-open): Account editor, Contact editor, Group editor, Calendar event editor, Signature editor. Editors work on a shadow copy and commit on save.

**`ProgressReporter` trait** - All event emission from core goes through `&dyn ProgressReporter`. The app provides its own implementation; the Service-side relocated action service uses `service::progress::IpcProgressReporter` which serializes events into `Notification::SyncProgress` frames over IPC.

**State types are `Clone`** - `ReadDbState`, `BodyStoreState`, `InlineImageStoreState`, `SearchState`, `AppCryptoState` all wrap `Arc<Mutex<_>>` and implement `Clone`. Phase 2 split the read/write halves of `DbState` into `db::ReadDbState` (UI side, exposes read methods) and `service_state::WriteDbState` (Service-only, constructed inside the `service-state` crate that the `app` crate does not depend on - see "Action service as mutation gate" above for the crate-boundary enforcement). Phase 6a-part-2 lands the global mutation lockdown: every UI write surface that previously took a write-conn handle now routes through a Service IPC; the body / inline-image / search write halves moved Service-side at Phase 3, and the `cal::actions` ActionContext at `app.rs::from_boot_ready` is the single remaining UI-side holdout, removed in Phase 6c.

**Service kick handlers** - The recurring background work that the UI used to drive directly (GAL refresh, calendar resync, pending-ops drain, pinned-search staleness sweep, attachment-cache eviction) now ships as Service-side `Drop`-class notification handlers gated on a per-task staleness window: `gal.kick`, `calendar.kick`, `pending_ops.kick`, `pinned_search.kick`, `attachment.eviction_kick`. The UI fan-outs the cadence on its 5-min `Message::SyncTick`; missed kicks self-heal on the next tick because the work is idempotent. The handlers live in `crates/service/src/handlers/{gal,calendar,pending_ops_kick,pinned_search,attachment}.rs` and share the same shape: pull the per-task staleness threshold, gate on it, run the underlying sync helper. Adding a new periodic surface follows this pattern - notification class `Drop`, idempotent body, staleness gate inside the handler.

**Service-side write surfaces** - Phase 6a / 6a-part-2 / 6b relocated every UI write surface to a per-method IPC. The wire types live in `crates/service-api/src/{settings,thread_ui_state,calendar,signature,pinned_search,smart_folder,contacts,account,internal,oauth,attachment}.rs`; the handlers live in `crates/service/src/handlers/`. Each surface follows the same per-method shape: typed `Params` and `Ack` structs round-trip through serde; the `RequestParams` enum carries the variant; method-name + timeout + round-trip tests pin the wire envelope. The bulk wire envelopes are `account.delete` (folds cancel-and-await for sync/push/calendar runners + `delete_account_orchestrate` + external-store cleanup into a 60 s IPC) and `oauth.exchange_code` (token-endpoint + userinfo round-trips, optional re-auth persist; bypasses admission so heavy traffic does not delay an OAuth flow). The encryption-key handle (`internal.read_bootstrap_snapshots`, `internal.encrypt_for_storage`, `internal.decrypt_for_storage`) closes Phase 2 carry-forward 19d for the bootstrap-snapshot path; the action_ctx still carries an encryption_key, removed alongside the ActionContext in Phase 6c. Compose drafts are the only UI write that survives the lockdown; they go through `crates/app/src/draft_wal.rs` (sync append to `<data_dir>/drafts.wal`) and the Service drains the WAL at `BootPhase::DrainingDraftWal` on next boot - sub-millisecond shutdown durability without an IPC race against `iced::exit()`.

**Service marker files** (`crates/service/src/markers/`). Multi-step recovery for crash-safe operations. Each marker is a versioned `MarkerFile<T>` carrying a step-completed list (or status enum) under `<app_data>/<dir_name>/<key>.json`. Atomic-rename writes survive a partial-write crash; idempotent unlinks let drain code paths re-run on boot. Today two consumers: `sync_markers` (Phase 4 in-flight sync state) and `account_delete_markers` (Phase 6b in-flight account-deletion step list). The shared helper landed at Phase 6b; future multi-step recovery surfaces hook in via `MarkerFile::new("<dir_name>")` and a serde-friendly payload struct.

**Forward reference: pack-aware attachment reads (Phase 1a).** `attachment.fetch` and `attachment.eviction_kick` today operate against the flat hash-keyed file cache (`attachment_cache/<content_hash>`). The flat cache's "open fd is the pin against eviction" works because Linux `unlink` is fd-safe - a UI process holding a cache file open survives a concurrent eviction sweep. When the attachments roadmap's Phase 1a lands a pack store (`attachment_pack.rs` + `pack_index` + frame-aware reads), the pin model breaks: eviction can rewrite the *file* and the UI's cached offset becomes meaningless. The revision pass at that point adds lease IDs (`PackStore::get_with_lease`), an active-lease counter on `pack_index`, an `attachment.gc_kick` notification for unreachable-frame collection, and a repack handler. The wire shape of `attachment.fetch` grows a `lease_id` field; the eviction-kick handler grows lease-aware bookkeeping. Until Phase 1a, the flat-cache assumptions hold.

**In-flight task handles for per-entity background work** - When the app dispatches a long-running per-entity Task (currently: per-account delta sync, in `App.sync_handles: HashMap<String, iced::task::Handle>`), it wraps the dispatch with `Task::abortable()`, stashes the handle keyed by the entity id, and (1) skips re-dispatch when an entry already exists, (2) removes the entry on the completion message, (3) calls `handle.abort()` when the underlying entity is deleted. The completion handler also drops results for entities that no longer exist - defense in depth against stale messages. Caveat: `Handle::abort` cancels at the next yield point, so writes already in-flight are not undone - external-store cleanup must run after the abort, and tightly racy writes still need per-entity generation checks at the write site.

**DOM-to-widget pipeline** - V1 in `html_render.rs`. Supports links, CID images, inline formatting (bold/italic/underline/strike/code via `iced::widget::rich_text`), block structure. Complexity heuristic (table depth >5, style tags >2) falls back to plain text. Used in both the reading pane and the pop-out message view (Simple HTML / Original HTML modes). Remaining gaps: remote image loading with consent, table rendering, image caching - tracked in TODO.md.

## Current Exceptions

These are intentional unresolved areas, not reasons to bypass the boundaries above.

- **`action_ctx` for contacts handlers** at `crates/app/src/app.rs::from_boot_ready` is the residual UI-side write surface. The accessor (`Db::phase_6c_pending_write_state`) backs `service::actions::contacts::{save_contact, delete_contact}` for provider write-back of contacts. Phase 6c relocated `cal::actions` Service-side (via `CalendarActionContext` and `cal_action.execute_plan` IPC) but the contacts pipeline kept its UI-side path; that's a Phase 6d/8 follow-up. The path uses `ReadDbState` (db crate), not `WriteDbState` (service-state), so the Phase 6b/6c-11 lockdown holds: `app -> service-state` is closed at the Cargo-graph level, and the only write path the UI can still drive locally is the contacts one.
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
