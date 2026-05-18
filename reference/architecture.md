# Architecture

Living reference for ratatoskr's architectural principles, boundaries, and settled patterns. Follow these when building new features or reviewing changes.

## Guiding Principle

**Make the right thing the only thing.** Correctness should not depend on every developer remembering a multi-step protocol. A single entry point, a type that enforces the invariant, or a compiler error when the protocol is violated - these are how contracts become real.

When evaluating a design: if adding a new call site can silently break existing behavior, the API is wrong.

## Crate Boundaries

**`rtsk`** is the facade. Business logic and domain orchestration live here - accounts, OAuth, discovery, email actions, calendar workflow, search routing, cloud-attachment orchestration. The app crate calls core functions directly.

**`db`** owns the main application SQLite schema and all shared-table SQL. Query shape, write shape, conflict resolution, and transaction-scoped shared-table persistence belong here. If multiple crates need to write the same table, `db` owns that write API.

**Provider crates** (`gmail`, `jmap`, `graph`, `imap`) each implement the `ProviderOps` trait (`common/src/ops.rs`). No provider-specific logic should leak into app or core beyond the trait surface. Service-side sync implementations live in `provider-sync`, which owns `SyncProviderCtx`, `ProviderSyncOps`, and the per-provider orphan impls that carry writer-half state.

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

### Cargo-level boundary rules

A subset of the boundaries above is mechanically enforced by `brokkr check` via `[[dependency_rule]]` entries in `brokkr.toml`. The phase reads `cargo metadata --no-deps` and fails on direct forbidden edges before clippy or tests run.

The dependency graph runs roughly leafward-to-rootward as `types` / `crypto-key` -> `common` -> `db` -> `store`, `search`, `seen` -> `sync` -> providers (`gmail`, `jmap`, `graph`, `imap`) -> `provider-sync`, `cal`, `rtsk` (core) -> `service-state` -> `service` -> `app`. The rules forbid upward edges and sideways edges that would let two peers form a tighter coupling than the layer above mediates.

UI / writer split:
- `app` may not depend on `rusqlite`, `db`, `service-state`, or any provider crate. UI talks to the writer side only through `service-api` IPC.
- `service` may not depend on `app`. Writer side never reaches into the UI process crate.
- `rtsk` (core) may not depend on `rusqlite` (shared-table SQL belongs to `db`) or on any of `service`, `service-state`, `cal`, `app`.

Layer rules:
- `common` may not depend on `sync`, providers, `provider-sync`, `rtsk`, `service`, `service-state`, `cal`, or `app`. It stays a shared-helper leaf.
- `sync` may not depend on providers, `provider-sync`, `cal`, `service`, `service-state`, or `app`. Provider crates depend on `sync`, never the reverse.
- `cal` may not depend on `app`.

Provider isolation:
- Each provider crate (`gmail`, `jmap`, `graph`, `imap`) may not depend on the other three, nor on `provider-sync`, `rtsk`, `cal`, `service`, `service-state`, or `app`. Provider crates are leaves of the protocol-specific subtree.

Rules are direct-edge only. Transitive boundaries (e.g. "no shared-table SQL anywhere outside `db`", or "`app` cannot transitively reach `service-state` at all") are not expressible at the Cargo level. The transitive lockdown for `app` -> `cal` and `app` -> `service-state` lives in `crates/service-state/tests/lockdown.rs`. Shared-table SQL scoping inside provider/sync crates remains a code-review concern - provider and sync crates legitimately need `rusqlite` for protocol-state tables carved out in *Current Exceptions* below (`jmap_sync_state`, `graph_*_delta_tokens`, `folder_sync_state`, etc.).

### Action service as mutation gate
<!-- coverage: architecture.action_service_as_mutation_gate enforcement=rust-test -->

Every email state mutation (archive, delete, star, move, label, send, snooze, mark-chat-read, etc.) and every calendar event mutation (create, update, delete) must flow through the action service. As of Phase 2 the email *execution* surface lives in `service::actions::*` (the relocated home; `core::actions` keeps a shim that re-exports the public API). UI handlers no longer call the execution functions directly - they build an `ActionExecutionPlan`, convert to `ActionWirePlan`, and dispatch via `client.execute_plan(...)`. The Service journals the plan, signals the worker, and per-operation `OperationOutcome` + final `ActionCompleted` notifications stream back over IPC. Calendar mutations flow through the sibling `cal_action.execute_plan` IPC into `service::cal_actions::batch_execute`, sharing the `action_jobs` journal via the `kind` discriminator and the per-op lease + recovery contract; see "Calendar action pipeline" below for the per-kind dispatch shape.

**Enforcement:**
- **Crate boundary.** `WriteDbState` is constructed in the `service-state` crate; `app` does not depend on `service-state`, so a UI source file that tries `use service_state::WriteDbState` fails to resolve. Phase 6a-part-2 deleted `Db::with_write_conn`, `Db::with_write_conn_sync`, and `Db::write_db_state` from the app crate; Phase 6d-A deleted the residual `Db::phase_6c_pending_write_state` accessor alongside the `app.action_ctx` field that consumed it (the contacts pipeline relocated to the `contacts.contact_save_with_writeback` + `contacts.contact_delete` IPC pair). The lockdown integration tests (`crates/service-state/tests/lockdown.rs`) assert that `app` has no direct `service-state` dependency, cannot reach `cal`, and cannot transitively reach `service-state` through any path-dep chain.

**Prefixed-label escape valve.** When the action service receives a `kw:`, `cat:`, or `importance:` label_id that doesn't yet have a `labels` row, `add_label_local` synthesises a tag-kind row on the fly via `ensure_prefixed_tag_label` (`crates/service/src/actions/label.rs`) instead of failing with `not_found`. The prefix is the type signal, so the action pipeline can apply a label observed only in `thread_labels` without a prior sidebar enumeration round-trip. This catches the window between a provider observing a new keyword/category on an incoming message and the next master-list sync, and tolerates per-message ad-hoc Exchange categories that never appear in `masterCategories`.

### Service process model

Ratatoskr runs as two cooperating processes. The UI process owns iced
rendering, input, UI state, and read-side queries. The Service is a
child worker process that owns durable writes and long-running work:
action execution, sync, push receivers, the retry queue, body/inline/
attachment store writes, attachment text extraction, and the Tantivy
writer. The Service is not a daemon; closing the app shuts it down.

The boundary is JSON-RPC 2.0 over newline-delimited stdio. Wire types
live in `service-api`, and frame writing goes through the shared
compact `write_message` helper so no pretty-printed or multi-line JSON
can corrupt framing. Large blobs never cross JSON; IPC returns stable
locations such as content hashes, and the UI reads bytes directly from
the on-disk store.

Service boot is two phase. The UI first sends `health.ping` with
`PROTOCOL_VERSION`; a mismatch is a fatal boot error. The Service then
runs the slow boot sequence: single-instance lock, key load, database
open, schema migrations, pending-op recovery, draft WAL drain, and
startup invariant recovery. `boot.ready` returns only after those
steps complete. Terminal boot exits use `BootExitCode` values and are
not respawned; post-ready crashes are respawn candidates.

Each Service incarnation is tagged by a UI-side generation counter.
The reader task stamps notifications with the live generation as it
enqueues them, and UI dispatch drops notifications whose generation no
longer matches. This prevents late frames from a dying Service from
mutating the new incarnation's UI state.

Shutdown is explicit. The UI sends the shutdown request, the Service
stops accepting new work, drains subsystems in the fixed order
`Push -> Calendar -> Sync -> Prefetch -> Extract -> Rebuild -> search writer`,
writes the clean-shutdown sentinel, responds, and exits. `kill_on_drop`
is disabled on the child handle so escalation stays under the explicit
SIGTERM / SIGKILL or Windows terminate policy.

Parent-death binding is platform-specific. Linux uses
`PR_SET_PDEATHSIG` plus a startup `getppid() == 1` recheck to close
the fork-to-registration race. Windows uses a Job Object with
`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`; the UI holds the job handle for
the child lifetime, so Service grandchildren inherit the same lifetime
binding. macOS is not a supported target yet; the retained design is
`kqueue` parent-exit registration plus a post-registration parent
recheck.

The Service owns stdout exclusively for JSON-RPC frames. At startup it
duplicates the real stdio handles for the IPC reader and writer, then
redirects the process standard stdin/stdout to the platform null
device. Logging goes to Service log files, never stdout. Sensitive
values are not loggable: request params, response payloads, auth
codes, bearer tokens, message bodies, search queries, draft content,
encryption-key bytes, attachment bytes, and extracted text must be
redacted or summarized.

Notifications use three delivery classes:
- `MustDeliver`: awaited send and ordered delivery; used for state
  changes such as action completions, sync completions, and committed
  index notifications.
- `Coalesce { key }`: latest-wins per key; used for progress and view
  reload hints.
- `Drop`: advisory events that may be discarded under pressure.

The UI has one ordered notification channel so cross-class order is
preserved. Inbound frames are capped while reading, handler concurrency
is bounded by a semaphore, outbound writes are isolated behind the
writer task, and request timeouts are declared at the API definition
site.

Service tests that need deterministic IO-boundary behavior - boot,
dispatch, drain, crash, framing - go through the Lua-driven harness
under `crates/app/tests/service-harness/` and `sync-harness/`, not
libtest. The runtime lives in `app`'s harness module, embeds the
`dellingr` Lua VM, and is orchestrated from outside by brokkr
(`brokkr service-test`, `service-suite`, `sync-bench`). Tests are
`.lua` scripts; failure preserves a self-contained artefact directory
with frames, events, step trace, `/proc` snapshot, and a copy of the
data dir. See `reference/glossary/harness.md`.

### Cross-store crash consistency

The Service writes multiple durable stores: main SQLite, `bodies.db`,
inline-image state, attachment cache files, and Tantivy. A clean
shutdown writes the sentinel after every writer has drained. Boot
removes the sentinel after acquiring writers; if the sentinel was
missing at startup, the Service runs the cross-store invariant pass
before `boot.ready`.

The invariant pass is idempotent. It reconciles message/body/inline/
attachment/search references, clears stale provider cursors for dirty
accounts where needed, removes orphan store rows, and records timing
stats. Cursor rows in the main database bound the amount scanned after
an unclean shutdown, so a large mailbox does not require a full-store
walk every time recovery runs.

### Calendar action pipeline

Calendar event mutations (create, update, delete) flow through a sibling pipeline added in Phase 6c: `CalendarOperation -> cal_action.execute_plan IPC -> service::cal_actions::batch_execute -> CalendarOperationOutcome notifications -> CalendarActionCompleted -> handle_calendar_action_completed`. The two pipelines share the `action_jobs` journal via the `kind` CHECK constraint (Phase 6c-1 widened it to include `'calendar_plan'`) and the per-op lease + recovery contract; the worker (Phase 6c-7) reads `action_jobs.kind` on each lease and dispatches to the right per-kind pipeline. Calendar uses a sibling `CalendarActionContext` (writer-half + encryption key) rather than the email-shared `ActionContext`; `cal::actions::*` write surface is reachable only Service-side. The UI awaits `CalendarActionCompleted` via `ServiceClient::pending_calendar_actions` (Phase 6c-9), which uses the same Pending-or-latched-Completed shape as `pending_calendars` for the late-subscriber race. Calendar plans are 1:1 today (one user intent = one operation); Phase 6d revisits if RSVP / series-vs-occurrence semantics introduce N-op plans.

### Text extraction pipeline

Cached attachments get text-extracted (per-mime: PDF via `pdf-extract`, OOXML via `zip` + `quick-xml`, plain via `encoding_rs`) and indexed into Tantivy alongside the parent message body, so search queries match attachment contents and the result row carries a "matched in *<filename>*" annotation. The pipeline is Service-only: `text_extract::extract` is a pure function (`crates/service/src/text_extract/{mod,pdf,ooxml,plain}.rs`) called from `ExtractRuntime` (`crates/service/src/extract.rs`) under `tokio::task::spawn_blocking + tokio::time::timeout(30s)`. Extracted text persists to the `attachment_extracted_text` table keyed by `content_hash` so the indexed text survives attachment-cache eviction; `attachments.text_indexed_at` records per-attachment status.

Triggers: (a) `attachment.fetch` enqueues on cache-miss after `PackStore::put` and on cache-hit when `extraction_status` is null or retry-eligible; (b) a one-shot `extract.backfill_kick` fires from `Message::ServiceBootReady` to catch up after a Service crash mid-extraction; (c) an hourly `iced::time::every` ticker re-fires the same kick as a safety net. The handler runs `find_unindexed_cached_attachments` and feeds the runtime; idempotency comes from three layers (SELECT returns 0 rows after backlog drains, runtime's `in_flight_hashes` rejects duplicates, worker's status-aware skip handles already-extracted rows). On successful extraction the worker emits `WriterCommand::Index` to the search-writer task with the full `Vec<AttachmentDocFragment>` populated from DB at extraction time - sync's DB-write-before-Index ordering means the writer always sees canonical state, so no writer-side staleness guard is needed.

The Tantivy doc shape uses **per-attachment `add_text` calls with a 32-token boundary pad** (`rtskbnd` repeated) inserted before all-but-the-first attachment text value, in addition to Tantivy's default `POSITION_GAP=1`. The pad is belt-and-suspenders against `slop>=2` phrase queries straddling attachment boundaries; `slop=0` queries are already blocked by the default position gap. `SearchResult.match_kind: MatchKind` (`Body` / `Subject` / `From` / `Attachment { attachment_id, filename, mime, snippet }`) plus `also_matched: Vec<MatchKind>` (secondary fields above 50% of top score, score-descending) drives the "matched in *<filename>*" annotation. Per-attachment attribution runs a `SnippetGenerator` against each attachment's text segment in a single batched DB SELECT joining `attachments` + `attachment_extracted_text`; tiebreak is term frequency, then filename alphabetical.

Schema-version mismatch (when `INDEX_SCHEMA_VERSION` bumps) takes the PreserveExisting rebuild path. `BootPhase::OpeningSearchIndex` reads the active index slot's `.version` before spawning the writer. On mismatch it marks `BootSharedState.pending_schema_rebuild`; the post-ready dispatcher calls `handle_rebuild` with `RebuildPolicy::PreserveExisting`. That rebuild opens a staging index directory, mirrors concurrent writes into the staging writer, replays all canonical messages into staging, writes the staging `.version`, then atomically updates `search_index.active` to point future readers and boots at the rebuilt slot. The old reader keeps serving from the previous slot until `IndexRebuildCompleted` reaches the UI and `SearchReadState::init` rebinds. Sentinel-write ordering is preserved: a mid-rebuild crash leaves the OLD active slot and OLD `.version` in place, so the next boot re-fires. The user-triggered palette command "Rebuild Search Index" (`CommandId::AppRebuildSearchIndex`) still routes through `handle_rebuild` with `RebuildPolicy::Wipe`, which intentionally clears and repopulates the active slot in place.

The `ExtractRuntime` lifecycle mirrors `CalendarRuntime`: a `CancellationToken` + stored worker `JoinHandle` + `Arc<Mutex<HashSet<content_hash>>>` enqueue dedupe. The drain order is `Push -> Calendar -> Sync -> Prefetch -> Extract -> Rebuild (single-task) -> search-writer -> sentinel`. The `Prefetch` slot is `PrefetchRuntime` (attachments roadmap Phase 4, `crates/service/src/prefetch.rs`); it follows `Sync` because sync's post-completion sweep produces prefetch work, and precedes `Extract` because prefetch's writes feed extraction. `PrefetchRuntime` mirrors `ExtractRuntime`'s shutdown shape (`CancellationToken` + `JoinHandle` + capped FIFO dedupe + per-item `JoinSet`) and uses a biased select to drain its Sync queue before its Backfill queue. The `search_write` slot on `BootSharedState` is single-use - `take_search_write` is called from inside `spawn_post_ready_extract_startup` on success and defensively from `run_shutdown_drain` before the search-writer await, so no `SearchWriteHandle` clone leaks past the writer-task EOF observation (the bug that bricked `boot_ready_blocks_until_sequence_completes` during the 7-4d revival).
- **Thread-action DB helpers** (`set_thread_read`, `set_thread_starred`, `set_thread_pinned`, `set_thread_muted`, `delete_thread`, `add_thread_label`, `remove_thread_label`) are `pub(crate)` - the app crate cannot call them directly.
- **`ActionProviderCtx`** (`crates/common/src/types.rs`) carries only `account_id` / `&ReadDbState` / `&dyn ProgressReporter` - no `&SearchState` field, so action methods cannot write to the Tantivy index. A regression test in `common::types::tests` enforces the exhaustive-destructure shape.
- **Tri-state in-flight tracking** (`crates/app/src/app.rs::PlanState`). UI plans live as `Pending` / `Acked` / `AckUnknown`; `ServiceCrashed` while `Pending` defers rollback to post-respawn `action.job_status` reconciliation, while `ServiceCrashed` while `Acked` does nothing because the journal will replay.

### Provider trait as abstraction layer
<!-- coverage: architecture.provider_trait_as_abstraction_layer enforcement=compiler -->

The four providers are unified behind `ProviderOps`. All provider-specific behavior is behind this trait - callers should never branch on provider type.

**Enforcement:** `FolderId` and `LabelId` newtypes in `crates/common/src/typed_ids.rs`. The `ProviderOps` trait uses `&FolderId` for `move_to_folder`, `rename_folder`, `delete_folder` and `&LabelId` for `add_label`, `remove_label`. Passing a folder ID where a label ID is expected is a compile error. Typed IDs flow from `MailActionIntent` through `MailOperation` through `batch_execute` to the provider - no raw string boundaries except JSON deserialization in `pending.rs` and `CommandArgs` in the palette crate.

For persistence, the provider boundary is:
- providers fetch and translate protocol payloads
- `db` owns shared-table writes
- provider-local sync tables are explicit exceptions, not accidental ownership of shared schema

### Scope as a single source of truth

The active scope (which account, shared mailbox, or public folder the user is looking at) must be consistent across sidebar, navigation context, and all DB queries.

**Enforcement:** `ViewScope` enum (`AllAccounts`, `Account`, `SharedMailbox`, `PublicFolder`) in `crates/core/src/scope.rs`. The sidebar stores `selected_scope: ViewScope` as the single source of truth. `fire_navigation_load()` and `load_threads_for_current_view()` dispatch on the enum - shared mailboxes and public folders use dedicated query paths, personal accounts use `AccountScope`-based queries. Shared mailbox threads are distinguished by `threads.shared_mailbox_id`; personal queries filter `shared_mailbox_id IS NULL`. Public folder items come from the separate `public_folder_items` table. Actions are gated for public folder scope.

### Generation counters for async safety
<!-- coverage: architecture.generation_counters_for_async_safety enforcement=compiler -->

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
<!-- coverage: architecture.folder_vs_label_semantics_are_explicit enforcement=lua-harness -->

Ratatoskr has exactly two persisted sidebar concepts:
- folders: provider containers stored in `folders`, with thread membership in `thread_folders`.
- label groups: user-visible grouped labels stored in `label_groups`, with pending local thread intent in `pending_thread_label_intents`.

Raw provider labels are a storage concept, not a sidebar concept. They live in `labels`, with provider-observable thread membership in `thread_labels`. Provider-native concepts must be normalized into either folders or raw labels before persistence. System folders use canonical Ratatoskr IDs (`INBOX`, `SENT`, `TRASH`, etc.), not provider-native IDs.

**Enforcement:** provider folder sync paths write `folders` through `insert_folders_batch`; provider label sync paths write tag-only `labels` through label upsert helpers. Sidebar/navigation code never infers folder vs label from provider strings: universal folders read `thread_folders` or thread-state booleans, and the LABELS section reads explicit `label_groups`.

**Pre-create rows for any folder_id that lands in `thread_folders` and any label_id that lands in `thread_labels`.** Folder membership must reference `folders`; raw label membership must reference `labels`. Graph upserts `cat:{name}` rows alongside category-backed `thread_labels` writes, IMAP upserts `kw:{keyword}` rows in `replace_message_keywords`, and JMAP upserts `kw:{keyword}` rows in keyword sync. Synthesised Graph importance rows (`importance:high`, `importance:low`) are upserted in the same Graph persistence path. Master-list pulls still provide slower-cadence colour metadata, but the message-persist upsert is the immediate FK guarantee.

## Adding a New Email Action
<!-- coverage: architecture.adding_a_new_email_action enforcement=compiler -->

The action pipeline flows: `MailActionIntent -> resolve_intent -> build_execution_plan -> ActionWirePlan -> action.execute_plan IPC -> service::actions::batch_execute -> OperationOutcome notifications -> handle_action_completed`. Adding a new action requires:

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

### Composite operations

A composite operation is a single planner-level `MailOperation` that fans out internally to N per-resource provider dispatches inside its service-side action function. The fan-out is invisible to the planner: one `OperationOutcome` returns, one toast fires, one undo payload covers the whole apply/remove pair. `ApplyLabelGroup` / `RemoveLabelGroup` (`crates/service/src/actions/label_group.rs`) are the worked example - one composite op per (thread, group), N member-label dispatches inside.

Composites change three things relative to a regular action and getting any one of them wrong reproduces the failure shape that landed in the labels-unification refactor:

1. **Retry preflight is load-bearing.** When the pending-ops drainer re-runs a composite retry, the service-side function MUST re-read the overlay-aware rendered group state before dispatching any member writes. If the user has reversed intent in the meantime - removed the group after an Apply enqueued, re-applied after a Remove enqueued - the queued member writes will resurrect or re-clear a pill against current intent for the entire retry-queue TTL. The preflight skips the queued member dispatches and resolves the retry as `Success`.

2. **Per-member dispatches must not enqueue per-member retries.** The composite's preflight contract only fires when the *composite* op type lands in `pending_operations`. If the inner member dispatches call the standard `enqueue_if_retryable` and enqueue raw `addLabel` / `removeLabel` rows, the drainer will re-run those per-member ops directly, with no preflight, and the contract above is bypassed. Either thread a "suppress per-member enqueue" flag through `ActionContext` so the composite enqueues a single composite-typed row covering the failed members, or factor out a `_no_enqueue` helper that the composite calls and that the per-member entry points wrap.

3. **`MailUndoPayload` pairs composites as wholes.** `ApplyLabelGroup` undoes via `RemoveLabelGroup` and vice versa; member-level undo is not exposed. The composite reads current member state at undo dispatch time - so member-set drift between original apply and undo means the undo touches the *current* member set, not the historical one. This is tolerable when members are user-authored and changes are local; document the same property for any new composite.

The composite reports a single `OperationOutcome` to the planner. Per-member provider failures are reconciled by the composite's own retry path with the preflight described above, never as separate planner notifications. Local writes from the composite's local step stay committed regardless of provider outcomes, matching the per-label-action contract in `crates/service/src/actions/label.rs`.

## Database Integrity

All tables with `account_id` CASCADE on account deletion. Migration 77 recreated the 16 tables that were missing the constraint. `delete_account_orchestrate()` handles external store cleanup (body store, inline images, attachment cache, search index).

## Settled Patterns

These are verified, adopted project-wide, and should be followed for all new work.

**Generational load tracking** - 9 branded `GenerationCounter<T>` instances across App and component levels. See "Generation counters for async safety" above.

**Component trait** - 8 components: Sidebar, ThreadList, ReadingPane, Settings, StatusBar, AddAccountWizard, Palette, ChatTimeline. Non-components (Compose, Calendar, Pop-out windows) use free functions + App handler methods.

**Token-to-Catalog theming** - All styling goes through the theme catalog. No inline color closures. Exceptions: rich text editor (builder methods), token input (renderer.fill_quad).

**Config shadow pattern** - Formal: `PreferencesState`. Implicit (clone-on-open): Account editor, Contact editor, Group editor, Calendar event editor, Signature editor. Editors work on a shadow copy and commit on save.

**`ProgressReporter` trait** - All event emission from core goes through `&dyn ProgressReporter`. The app provides its own implementation; the Service-side relocated action service uses `service::progress::IpcProgressReporter` which serializes events into `Notification::SyncProgress` frames over IPC.

**State types are `Clone`** - `ReadDbState`, `BodyStoreState`, `InlineImageStoreState`, `SearchState`, `AppCryptoState` all wrap shared handles and implement `Clone`. The main DB split is now explicit: `db_read::ReadDbState` owns read-only access, while `service_state::WriteDbState` wraps `db::WriterPool` and is Service-only. Writer access exposes typed capabilities only: `with_write` / `with_write_mapped` / `with_write_sync` hand closures a `db::WriteConn`, and read-through-writer paths use `with_read` / `with_read_sync` with `db::ReadConn`. The old untyped writer shims (`with_conn*`) and raw transaction escape hatch are gone. Phase 6a-part-2 lands the global mutation lockdown: every UI write surface that previously took a write-conn handle now routes through a Service IPC; the body / inline-image / search write halves moved Service-side at Phase 3, the `cal::actions` ActionContext was relocated in Phase 6c, and the contacts pipeline (the last UI-side write surface) relocated in Phase 6d-A.

**Service kick handlers** - The recurring background work that the UI used to drive directly (GAL refresh, calendar resync, pending-ops drain, pinned-search staleness sweep, attachment-cache eviction) now ships as Service-side `Drop`-class notification handlers gated on a per-task staleness window: `gal.kick`, `calendar.kick`, `pending_ops.kick`, `pinned_search.kick`, `attachment.eviction_kick`. The UI fan-outs the cadence on its 5-min `Message::SyncTick`; missed kicks self-heal on the next tick because the work is idempotent. The handlers live in `crates/service/src/handlers/{gal,calendar,pending_ops_kick,pinned_search,attachment}.rs` and share the same shape: pull the per-task staleness threshold, gate on it, run the underlying sync helper. Adding a new periodic surface follows this pattern - notification class `Drop`, idempotent body, staleness gate inside the handler.

**Service-side write surfaces** - Phase 6a / 6a-part-2 / 6b / 6c / 6d-A relocated every UI write surface to a per-method IPC. The wire types live in `crates/service-api/src/{settings,thread_ui_state,calendar,signature,pinned_search,smart_folder,contacts,account,internal,oauth,attachment,extract}.rs`; the handlers live in `crates/service/src/handlers/`. Each surface follows the same per-method shape: typed `Params` and `Ack` structs round-trip through serde; the `RequestParams` enum carries the variant; method-name + timeout + round-trip tests pin the wire envelope. The bulk wire envelopes are `account.delete` (folds cancel-and-await for sync/push/calendar runners + `delete_account_orchestrate` + external-store cleanup into a 60 s IPC) and `oauth.exchange_code` (token-endpoint + userinfo round-trips, optional re-auth persist; bypasses admission so heavy traffic does not delay an OAuth flow). The encryption-key handle (`internal.read_bootstrap_snapshots`, `internal.encrypt_for_storage`, `internal.decrypt_for_storage`) closes Phase 2 carry-forward 19d for the bootstrap-snapshot path. Phase 6d-A's contacts relocation (`contacts.contact_save_with_writeback`, `contacts.contact_delete`) deleted the last UI-side `rtsk::load_encryption_key` call - the Service holds the validated key bytes for the lifetime of the process and the UI never opens `ratatoskr.key`. Phase 7 added the `attachment_extracted_text` table and the Tantivy-writer side as Service-only writers - extraction runs Service-side via `ExtractRuntime` and the search-writer task is the only path into the index. Compose drafts are the only UI write that survives the lockdown; they go through `crates/app/src/draft_wal.rs` (sync append to `<data_dir>/drafts.wal`) and the Service drains the WAL at `BootPhase::DrainingDraftWal` on next boot - sub-millisecond shutdown durability without an IPC race against `iced::exit()`.

**Service marker files** (`crates/service/src/markers/`). Multi-step recovery for crash-safe operations. Each marker is a versioned `MarkerFile<T>` carrying a step-completed list (or status enum) under `<app_data>/<dir_name>/<key>.json`. Atomic-rename writes survive a partial-write crash; idempotent unlinks let drain code paths re-run on boot. Today two consumers: `sync_markers` (Phase 4 in-flight sync state) and `account_delete_markers` (Phase 6b in-flight account-deletion step list). The shared helper landed at Phase 6b; future multi-step recovery surfaces hook in via `MarkerFile::new("<dir_name>")` and a serde-friendly payload struct.

**Pack-aware attachment reads** (attachments roadmap Phase 3, landed). `attachment.fetch` runs through `crates/service/src/attachment_materialize.rs::materialize_blob`: it reads bytes from `PackStore::get`, writes them to `<app_data>/attachment_fetch_tmp/<content_hash>-<request_id>` (`.part` then atomic rename), and returns the path in the ack. The UI opens the tmp file positionally; the open fd is the pin (`unlink` is fd-safe on Linux so a concurrent reap survives an in-flight read). `attachment.tmp_cleanup_kick` fires on the same 5-min cadence as the other UI ticks and unlinks tmp entries older than 10 minutes. Eviction during read is race-free: tombstoning a blob in `attachment_blobs` (Phase 8's responsibility) and eventual GC of its pack frame are independent of any in-flight UI read against a separate tmp file. The `attachment.eviction_kick` notification still exists as a wire-compatible no-op; Phase 8 reuses it for date-windowed tombstoning. No lease IDs.

**In-flight task handles for per-entity background work** - When the app dispatches a long-running per-entity Task (currently: per-account delta sync, in `App.sync_handles: HashMap<String, iced::task::Handle>`), it wraps the dispatch with `Task::abortable()`, stashes the handle keyed by the entity id, and (1) skips re-dispatch when an entry already exists, (2) removes the entry on the completion message, (3) calls `handle.abort()` when the underlying entity is deleted. The completion handler also drops results for entities that no longer exist - defense in depth against stale messages. Caveat: `Handle::abort` cancels at the next yield point, so writes already in-flight are not undone - external-store cleanup must run after the abort, and tightly racy writes still need per-entity generation checks at the write site.

**DOM-to-widget pipeline** - V1 in `html_render.rs`. Supports links, CID images, inline formatting (bold/italic/underline/strike/code via `iced::widget::rich_text`), block structure. Complexity heuristic (table depth >5, style tags >2) falls back to plain text. Used in both the reading pane and the pop-out message view (Simple HTML / Original HTML modes). Remaining gaps: remote image loading with consent, table rendering, image caching - tracked in TODO.md.

**Per-message membership store with derived thread aggregate** - Three per-message membership tables in `crates/db/src/db/schema/02_mail.sql` carry the ground truth that partial-delta providers actually deliver, with the thread-level aggregates derived from their union:

- `message_keywords` keyed `(account_id, message_id, keyword, label_id)` - IMAP/JMAP keyword rows. Recomputed into `thread_labels` `kw:%` rows by `recompute_thread_keyword_labels` (`crates/provider-sync/src/keyword_membership.rs`).
- `message_folders` keyed `(account_id, message_id, folder_id)` - Graph/JMAP per-message folder membership. Recomputed into `thread_folders` by `recompute_thread_folders_from_messages` (`crates/db/src/db/queries_extra/message_membership.rs`). IMAP needs no row here because `messages.imap_folder` already carries per-message ground truth.
- `message_labels` keyed `(account_id, message_id, label_id)` - Graph non-keyword label rows (categories, importance). Recomputed into `thread_labels` by `recompute_thread_labels_from_messages`.

All three are FK-cascaded by message. The pattern exists because partial-delta providers (Graph, JMAP) only describe what a message *is* in now, not what it is no longer in. Without per-message ground truth, a cross-client move that subtracts the source folder/label from every message of a thread would leave the stale thread-aggregate row in place - the delta is observationally equal to "no change for the messages we didn't fetch." With the per-message tables, partial-delta writers do a per-message replace (safe because they have the full current state of *that one message*), then recompute the thread aggregate from the union. Hook points for message delete / JWZ rethread call `delete_message_membership_rows` followed by the recompute, the same shape the keyword path already uses.

**Partial-delta membership merge vs full-thread replace** - Thread folder and raw-label writes follow one of two provider-local paths depending on whether the provider gives full-thread or partial-thread snapshots, both routed through `crates/provider-sync/src/thread_membership.rs`. Gmail storage and the IMAP thread-store pass (`crates/provider-sync/src/imap/thread_store.rs`) call `replace_thread_membership_from_full_coverage` - destructive replace is safe because the caller has every message of the thread. Graph and JMAP sync modules call `replace_message_membership_and_recompute` (or the folder-only variant for JMAP, whose labels are keyword-shaped and route through the keyword recompute), which atomically replaces the per-message rows for one message and recomputes the thread aggregate from the union. Neither helper exposes a separable "write rows" step from the recompute, so a contributor cannot forget the second half. `db::queries_extra::thread_persistence` and `message_membership` expose only raw row insert/delete primitives; provider-semantic orchestration lives in `provider-sync` where the provider's coverage shape is known. Same-client moves continue to mutate `thread_folders` locally before dispatching to the provider; same-client label actions write `pending_thread_label_intents` and let provider truth catch up.

**Send-intent threading** - When a Reply or Forward is sent from Ratatoskr, the user's intent and the source message's local DB ID flow through every layer of the send pipeline so the appropriate provider can write back the replied/forwarded primitive. The `SendIntent` enum (`New` / `Reply` / `Forward`) is defined in `crates/common/src/types.rs` and `crates/service-api/src/action.rs` (wire copy). It threads UI compose state -> `service_api::SendWireMessage` -> action journal (`crates/service/src/handlers/action_send.rs::JournaledMessage`) -> `service::send::SendRequest` -> the action service's `send_email`. The action service then performs two writes in order: `mark_send_intent_local` (`crates/service/src/send.rs`) flips `messages.is_replied` or `is_forwarded` on the source message immediately - this is the authoritative local mark - and the provider's `mark_send_intent` (`ProviderOps` trait method, `crates/common/src/ops.rs`) issues a best-effort write-back: IMAP `STORE +FLAGS \Answered` or `$Forwarded`; JMAP `EmailSet` keyword `$answered` or `$forwarded`; Graph PATCH on `singleValueExtendedProperties` for `PR_LAST_VERB_EXECUTED`. Provider failures are logged at warn and the local state remains the source of truth. Gmail has no `mark_send_intent` because the parser derives the bit from `SENT` membership + reply headers on the next sync. Adding a new send-intent surface (e.g. mark-as-template) extends `SendIntent`, the wire copy, and one match arm in each provider's `mark_send_intent`.

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
