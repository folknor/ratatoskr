# The Service - Phase 2 Plan: Action service relocation + read/write state-type split

Companion to `phase-1-plan.md` and `phase-1.5-plan.md`. Implements Phase 2 of `implementation-roadmap.md`, including the Phase 1.5 carry-forward bullets named in that section.

## Context

Phase 1.5 made the Service load-bearing for boot but the Service still does no real work post-handshake. Phase 2 moves the first actual workload across the boundary: action *execution*. The split is deliberate and surgical:

- **UI keeps**: action *resolution* and *planning* (`MailActionIntent` -> `resolve_intent` -> `build_execution_plan`). These read selection state, sidebar scope, and `completion_behavior()` policy - all UI-owned. They produce a fully-resolved `MailOperation` list.
- **Service gets**: `batch_execute(plan)` and the four entanglements that come with it - the `pending_ops` retry queue's periodic drainer, the encryption-key consumer, the in-flight dedup set, and the action-DB writer half.
- **UI keeps**: completion-effects (toast, auto-advance, undo eligibility, optimistic rollback) - driven by per-operation `OperationOutcome` notifications and a final `action.completed`.

Phase 2 also lands the first half of the **type-level read/write split** the problem statement names as the global invariant. `WriteDbState` is unreachable from `app` via the Cargo dependency graph (a new `service-state` crate that `app` does not depend on). The body/inline/search write halves do not lock down until Phase 3 (sync moves) and Phase 6 (the rest), so Phase 2's claim is scoped: *the relocated call sites can no longer construct `WriteDbState`*. The global "no UI write surface compiles" claim arrives at Phase 6, not here.

The phase ships as a single milestone but with a clean commit-level split (per the same shape as Phase 1.5): wire types -> `service-state` crate -> ProviderCtx split -> action service relocation -> respawn-aware optimistic rollback -> pending-ops drainer -> Phase 1.5 carry-forwards. A bisect on a regression should land on the right commit.

This is the second-largest UI-side surgery in the project (Phase 1.5 was the largest). The dispatch path that today is `action_ctx.action_ctx().expect(...) -> Task::perform(batch_execute(...))` becomes a multi-step IPC flow whose latency budget is in milliseconds, not microseconds. The hot path is design-bounded - star-toggle p99 of 5-15 ms is the design target. (A CI benchmark was originally planned to enforce this; dropped during close-out - see `## Architecture` § "Action latency benchmark (DROPPED)".)

## Scope

### In scope

1. **`service-state` crate.** New `crates/service-state/` houses the `WriteDbState` type. `app` does not list it as a dependency; `service` does. Compile-time enforcement of "no UI call site constructs `WriteDbState`" follows from the dependency graph - a UI handler that tries to `use service_state::WriteDbState` fails to resolve. `db::DbState` -> `db::ReadDbState` (UI) + `service_state::WriteDbState` (Service); both wrap the same internal `Arc<Mutex<Connection>>` pool but only the write half exposes write methods. `DbState::conn()` and `from_arc()` (`crates/db/src/db/mod.rs`'s raw-`Connection` escape hatches) collapse: `ReadDbState::conn()` stays for read paths; `WriteDbState::conn()` is `pub(crate)` to the `service-state` crate and re-exported only via `WriteDbState`'s narrow API. Body/inline/search state types **do not split in Phase 2** - their writers stay UI-side until Phase 3 (sync) and Phase 6 (everything else). The `Clone + Arc<Mutex<_>>` settled pattern (`docs/architecture.md` § "Settled Patterns") survives the split.

2. **`ActionWirePlan` wire type with explicit `WireMailOperation` mirror.** Today's `ActionExecutionPlan` carries UI completion metadata (auto-advance hints, toast text, completion-behavior policy) that does not belong on the wire. `MailOperation` (`crates/core/src/actions/operation.rs`) is also not a wire type: it derives only `Debug, Clone, PartialEq, Eq` (no serde), and `ActionOutcome` / `ActionError` (`crates/core/src/actions/outcome.rs`) are not serializable either. Phase 2 defines canonical wire enums in `service-api` rather than bolting serde onto core's domain types. `service-api` stays lightweight (no transitive dep on `rtsk` -> providers/search/etc., per `crates/service-api/Cargo.toml`); explicit `From`/`TryFrom` conversions at the app/service edge catch shape drift between core and the wire.

   ```rust
   // in service-api
   /// 128-bit time-ordered UUIDv7. UI-generated, collision-resistant
   /// across UI restarts (a u64 counter reset across a UI restart can
   /// collide with old journal rows from the previous incarnation).
   /// Equals action_jobs.job_id in the journal.
   pub struct PlanId(pub uuid::Uuid);

   pub struct ActionWirePlan {
       pub plan_id: PlanId,
       pub operations: Vec<ActionWireOperation>,
   }
   pub struct ActionWireOperation {
       pub operation_id: OperationId,    // u32, scoped per-plan
       pub account_id: String,
       pub thread_id: String,
       pub operation: WireMailOperation,
   }
   pub enum WireMailOperation {
       SetRead { is_read: bool },
       SetStarred { is_starred: bool },
       Archive,
       Trash,
       MoveToFolder { dest: WireFolderId, source: Option<WireFolderId> },
       AddTag { tag: WireTagId },
       RemoveTag { tag: WireTagId },
       // ... one variant per MailOperation variant in core.
   }
   ```

   `WireMailOperation` mirrors `core::actions::MailOperation` 1:1 with `serde` derives, **including all fields per variant** (the earlier draft dropped `MoveToFolder.source` - field-loss is a real risk and the static-assertion test below catches it). UI converts `MailOperation` -> `WireMailOperation` before send; Service converts back before dispatch. Wire-side typed IDs (`WireFolderId`, `WireTagId`) wrap strings and don't depend on `common::typed_ids`. `PlanId` is `uuid::Uuid` (UUIDv7 for time-ordered insertion / index locality); `OperationId` is `u32` scoped per-plan. UI generates both: it needs to correlate `OperationOutcome` notifications back to the originating intent's UI metadata, and Service-generated ids would round-trip a request before the correlation map could hold the entry. UI completion metadata stays UI-side in an `in_flight_plans: HashMap<PlanId, PlanCompletionMeta>` keyed by `plan_id`. (Decision in implementation: if mirroring proves too painful and the variant set is stable, fall back to adding `serde` derives on `core::actions::MailOperation` and keeping the wire shape in `core`. Default proposal is the mirror crate split because it survives Phase 6's calendar/RSVP wire types better - calendar mutations will not fit `MailOperation` shape, and a single wire enum across action types lives more comfortably in `service-api`.)

   **Static-assertion + field-preservation test placement.** A pure-`service-api` test cannot reference `core::actions::MailOperation` without `service-api` taking a dep on `core` - that contradicts the "lightweight wire crate" goal. The exhaustive-mirror test therefore lives in a small bridge crate (`crates/service-api-bridge/` or simply inside the `app` crate's tests, where `app` already depends on both `core` and `service-api`). The shape:
   ```rust
   #[test]
   fn wire_mirror_is_exhaustive_and_field_preserving() {
       // Compiler errors if a new MailOperation variant is added without
       // a wire mirror, or if a field changes shape.
       fn _assert_to_wire(op: core::actions::MailOperation) -> WireMailOperation {
           match op {
               MailOperation::SetRead { is_read } =>
                   WireMailOperation::SetRead { is_read },
               MailOperation::MoveToFolder { dest, source } =>
                   WireMailOperation::MoveToFolder {
                       dest: dest.into(),
                       source: source.map(Into::into),
                   },
               // ... every variant, no `_`, no `..`
           }
       }
       // Round-trip property test catches silent field-shape drift
       // (e.g. Option<X> -> X, Vec<X> -> X).
       proptest!(|(op in arbitrary_mail_operation())| {
           let wire: WireMailOperation = op.clone().into();
           let back: MailOperation = wire.try_into().unwrap();
           assert_eq!(op, back);
       });
   }
   ```
   The `MailOperation::From<WireMailOperation>` direction (Service-side) and the `WireMailOperation::From<MailOperation>` direction (UI-side) both live at the app/service boundary in this bridge crate, **not** in `service-api`. The bridge crate is a Phase 2 deliverable.

3. **`action.execute_plan` IPC method (handler enqueues, worker drives).** The request handler validates the plan, durably enqueues it into the action-plan journal (item 18a), signals a Service-owned action worker, and returns `ActionPlanAck { plan_id }`. The handler does NOT call `batch_execute` itself: the dispatch loop sends the JSON-RPC response only after the handler future returns (`crates/service/src/dispatch.rs:406-417`), so an ack-then-stream shape requires the worker to be a separate task. If the handler drove `batch_execute` directly it would (a) hold its in-flight semaphore permit for the entire plan duration (starving other admissions), and (b) emit `MustDeliver` outcomes from inside the request future, which under outbound-pipe backpressure is exactly the dispatch-starvation deadlock the problem statement names (`docs/service/problem-statement.md` § IPC). Handler timeout: 5 s (just enqueue + signal). Per-operation outcomes stream from the worker as `OperationOutcome` notifications; final `action.completed` closes the stream. The worker has its own per-account concurrency (4) and its own semaphore, separate from the dispatch in-flight cap. Bypasses neither cap, but doesn't hold either across plan execution.

4. **`OperationOutcome` and `action.completed` notifications.** Both `MustDeliver` (the cross-respawn drop guarantee from Phase 1.5's generation tag preempts stale outcomes from a dying incarnation hitting the new one's UI). Both implement `WithGeneration` per the contract added in Phase 1.5 commit 30, so the catalog test grows automatically. Per-operation outcomes carry `{ plan_id, operation_id, result: OperationResult }`; the final `action.completed { plan_id, summary }` only fires after the last per-operation transaction has committed and is visible to the read connection. The contract is *commit visibility*, not WAL fsync: the DB runs `PRAGMA synchronous = NORMAL` (`crates/db/src/db/mod.rs:101`), and an fsync-tight contract would conflict with the 16 ms latency target without buying anything for UI read-after-write coherence (worker writes commit, then any UI read against `ReadDbState` sees the committed state - that is all the UI's "got `action.completed`, refresh thread list now" pattern needs).

5. **`action.send` IPC method: Service takes ownership of attachment bytes before ack.** Compose-send relocates. The current `SendRequest` carries raw `Vec<u8>` attachment bytes (`crates/core/src/send.rs:23`), which would blow past the 4 MiB JSON-RPC frame cap; bytes go through a UI-controlled staging directory, but a hash-validated reference is **not enough for durable ack** - if the staging file is deleted, has its permissions changed, has a symlink swapped under it, or hits an FS error mid-SMTP after `SendAck` returned, the journal cannot replay the send and the user has been told "queued for sending" but the work is lost. The contract therefore requires the Service to take ownership of the bytes during the handler, before ack:

   ```rust
   pub struct SendWireRequest {
       pub send_id: SendId,
       pub from_account_id: String,
       pub message: SendWireMessage,                  // headers + body, small
       pub attachments: Vec<SendWireAttachment>,
   }
   pub struct SendWireAttachment {
       pub source: SendAttachmentSource,
       pub size: u64,
       pub mime: String,
       pub filename: String,
   }
   pub enum SendAttachmentSource {
       /// UI-staging path under <app_data>/staging/<send_id>/. UI writes bytes
       /// there before issuing action.send. Service moves/copies into its own
       /// vault during the handler, BEFORE returning SendAck.
       StagingFile {
           /// Relative path under staging/<send_id>/. NOT an arbitrary path -
           /// the handler rejects '..', absolute paths, and symlinks.
           relative_path: String,
           content_hash: [u8; 32],
       },
       /// Already in the pack store (forward / reply with no edits).
       /// Service refcounts the existing entry; no copy needed.
       PackStore { content_hash: [u8; 32] },
   }
   ```

   **Handler sequence (durable transfer of ownership):**
   1. Validate every `relative_path`: reject `..` segments, absolute paths, symlinks (check via `lstat`, not `stat`), paths escaping `staging/<send_id>/`.
   2. For each `StagingFile`: re-read the bytes once, verify `content_hash`, atomically rename / hardlink into the Service-owned vault `<app_data>/send_vault/<job_id>/<index>.bin` (same filesystem -> rename; cross-FS -> copy + fsync + verify hash again). Hash mismatch -> reject with `RemoteFailure { retryable: false }` BEFORE journaling. Permission / IO error -> reject with a clear error before journaling.
   3. For each `PackStore` ref: bump its refcount (Phase 2 has no pack-store eviction yet, but the contract holds: the Service owns a stable reference to the bytes regardless of what the pack-store GC does later).
   4. Journal the send into `action_jobs` with `kind = 'send'` and `payload = JournaledSend { ..., attachments: [{ vault_path, content_hash, size, ... }] }`. Vault paths replace `relative_path`s in the journaled payload - the staging directory is no longer load-bearing.
   5. Return `SendAck { send_id, journaled: true }`.
   6. Worker drives the SMTP submit, reads from the vault, emits progress + final `SendCompleted`.

   **Lifecycle:** UI writes attachment bytes to `<app_data>/staging/<send_id>/<index>.bin` before dispatch. The Service moves/copies + verifies + journals + acks. After ack, the staging directory is the UI's responsibility to remove (UI deletes it on `SendCompleted`, or at next boot for orphans; it carries no Service-side correctness weight). The send vault unlinks on terminal `action_jobs.status` (`completed` or `failed`).

   **Boot recovery for the vault:** unlink any `<app_data>/send_vault/<job_id>/` whose `job_id` is not present in `action_jobs`, OR whose `action_jobs.status IN ('completed', 'failed')`. Mirrors the journal's lease-recovery pass.

   The 60 s SMTP timeout from the original timeout table moves onto the worker, not the request handler (handler is validate + transfer + journal + ack, same shape as `action.execute_plan` but with the byte-transfer step). UI-side `crates/app/src/handlers/pop_out/compose_send.rs::dispatch_send` becomes the staging write + IPC call; the staging directory is *no longer* a durable substrate, just a transfer buffer.

6. **`action.undo` IPC method.** Undo is structurally a compensating-action plan dispatched via the same path; the UI builds the reverse `ActionWirePlan` from the stashed `MailUndoPayload` (existing in `action_resolve.rs`) and submits it. No new wire shape - reuses `action.execute_plan`. Phase 2 closes the loop by ensuring the stashed payload survives a Service respawn (the in_flight_plans map is UI-side and persistent across respawn; payloads are re-issuable).

7. **`action.snooze_resurface_due` IPC method.** The snooze resurfacing tick (`handlers/commands.rs` `SyncTick` path) becomes a Service-internal periodic task. Phase 2 transition: UI fires a `pending_ops.kick`-style notification on `SyncTick`; the Service's snooze-runner inspects the DB, dispatches resurface operations as a normal `action.execute_plan`. The `SyncTick` policy stays UI-side (depends on focus, online state, etc.) - the trigger is UI-driven, the work is Service-side.

8. **Pending-ops periodic drainer relocates.** The `process_pending_ops` periodic (`crates/core/src/actions/pending.rs`) moves Service-side. UI sends `pending_ops.kick` notification on `Message::SyncTick`. Default proposal: UI-driven kick (preserves the existing tick policy that gates on focus + online state) over a Service-side periodic. The Service-side drainer reuses the boot-time `recover_on_boot_db_only` path Phase 1.5 already extracted. **`pending_operations` is orthogonal to the new `action_jobs` journal (item 18a)**: `pending_operations` is the per-op transient-retry queue (it stores single ops, no `job_id` / ordinal / outcome state, and replaces rows by `(account_id, resource_id, operation_type)` per `crates/db/src/db/pending_ops.rs:61`); `action_jobs` + `action_job_ops` are the per-job execution journal that backs the durable-ack contract. A plan op that fails with a retryable `RemoteFailure` marks its journal row `failed` with a "queued for retry" outcome and enqueues the single op into `pending_operations` for the periodic to retry. Action-worker recovery drains the journal at boot; pending-ops periodic drains transient retries on tick. The two paths complement each other and neither subsumes the other.

9. **`ProviderCtx` shape adjustment: object-safe DB capabilities, not `with_transaction` over a trait object.** Today's `ProviderCtx { account_id, db, body_store, inline_images, search, progress }` (per `crates/common/src/types.rs`) is one struct passed to every `ProviderOps` method, including action methods that only need `db` + the encryption key. Putting a concrete `service_state::WriteDbState` into a `common`-defined `ProviderCtx` would pull `service-state` into `common`'s public API and break the dependency-graph enforcement scope item 1 promises (`app` transitively depends on `common`, so `common::ActionProviderCtx { db_write: &WriteDbState }` makes `WriteDbState` reachable from `app`).

   An earlier draft proposed `with_transaction<F, T>` on a `dyn ActionDbWrite`. That doesn't work: a generic closure-returning method is not object-safe, and even if it were, `rusqlite::Transaction<'_>` borrows `&mut Connection` and that lifetime cannot be carried across async provider awaits. The actual shape is **narrow object-safe capability methods on the trait, with transaction orchestration kept on the Service side, not exposed through `common`**:

   - **In `common`**: define narrow object-safe traits with no generics and no transaction-lifetime exposure:
     ```rust
     pub trait ActionDbWrite: Send + Sync {
         fn set_thread_read(&self, account_id: &str, thread_id: &str, read: bool) -> Result<(), DbError>;
         fn set_thread_starred(&self, account_id: &str, thread_id: &str, starred: bool) -> Result<(), DbError>;
         fn add_thread_label(&self, account_id: &str, thread_id: &str, label: &TagId) -> Result<(), DbError>;
         fn remove_thread_label(&self, account_id: &str, thread_id: &str, label: &TagId) -> Result<(), DbError>;
         fn delete_thread(&self, account_id: &str, thread_id: &str) -> Result<(), DbError>;
         fn move_thread(&self, account_id: &str, thread_id: &str, dest: &FolderId) -> Result<(), DbError>;
         // ... one method per shared-table mutation an action method legitimately needs.
     }
     pub trait ActionKeyAccess: Send + Sync {
         fn encrypt_for_storage(&self, plaintext: &[u8]) -> Result<Vec<u8>, KeyError>;
         fn decrypt_from_storage(&self, ciphertext: &[u8]) -> Result<Vec<u8>, KeyError>;
     }
     pub struct ActionProviderCtx<'a> {
         pub account_id: &'a str,
         pub db_write: &'a dyn ActionDbWrite,
         pub key: &'a dyn ActionKeyAccess,
         pub progress: &'a dyn ProgressReporter,
     }
     ```
   - **In `service-state`**: implement these traits on `WriteDbState` and the encryption-key holder. Each method opens its own transaction internally (via the `with_journal_write_conn`-style helpers in `db`), runs the SQL, commits or rolls back. The transaction lifetime never escapes into `common`, so the trait stays object-safe and async-await-compatible. Construction stays Cargo-graph-unreachable from `app`.
   - **Multi-statement atomicity**, when an action method needs to write multiple rows atomically, is handled by adding a higher-level method to `ActionDbWrite` that does the multi-statement work internally. The trait grows when a real action needs it; we don't try to make the trait into a database-abstraction layer. (Today's action methods are all single-statement DB writes followed by provider dispatch; this shape covers Phase 2.)
   - **`SyncProviderCtx<'a>` is scaffolded in Phase 2 but its signatures do NOT switch in Phase 2.** Sync methods today need real DB / body / inline / search handles (`crates/common/src/ops.rs:19`, `crates/gmail/src/ops.rs:36`); switching them to `body_write: ()` placeholders would not compile against live sync. Phase 2 leaves sync method signatures untouched. The `SyncProviderCtx` type lives in `common` as an unused-but-defined scaffold; Phase 3 introduces the trait split for sync alongside the actual writer relocation.

   The trait split touches every action method in `crates/common/src/ops.rs` and its impls in the four provider crates, plus the `ActionContext` plumbing inside the Service. Mechanical but broad. Lands as one focused commit so the action-relocation commits don't drown in `git blame`. (Decision in implementation: if the action methods turn out to need multi-statement atomicity beyond what trivial-method-additions can cover, fall back to making `ActionProviderCtx` generic over a `D: ActionDbWrite` rather than `dyn`, and keep the transaction orchestration in the Service action code that owns the `WriteDbState`. Default proposal is the object-safe trait because it keeps `ActionContext` shape simple and lets the action service pass a single `&dyn ActionDbWrite` through provider calls.)

10. **`ProgressReporter` trait.** Already `Send + Sync` and serializable (`crates/db/src/progress.rs:32` - `emit_json(event_name: &str, json: serde_json::Value)`). Service-side `IpcProgressReporter` posts to the notification queue with `Coalesce { key: ProgressEvent(account_id) }` per emission. UI's `IcedProgressReporter` keeps consuming the same shape - no trait redesign needed. The "trait method signatures must become serializable" risk the roadmap flagged was based on the older `ProgressReporter` shape; the current trait is already serializable so this collapses to "construct one new impl".

11. **UI -> Service notification framing support, OR fall back to acked requests.** The Phase 1 framing in `crates/service-api/src/framing.rs:184,214` rejects messages without a JSON-RPC `id` - the wire only supports requests UI -> Service today. Phase 2 needs `pending_ops.kick` and `mark_chat_read` to be fire-and-forget so a UI tick doesn't await a Service round-trip; the prior draft called these "Drop-class" while still adding them to `RequestParams`, which is contradictory.

    **Phase 2 picks: extend the framing with a real client-to-service notification path.** Add a `ClientNotification` envelope (`{"jsonrpc":"2.0","method":"...","params":{...}}` with no `id`) that the Service dispatches into a per-method handler with `Drop`-class admission semantics (no admission semaphore, drop oldest under queue pressure). This matches the existing Service-to-UI notification taxonomy for symmetry. `pending_ops.kick` and (a future) `chat.viewing_changed` use this path. `action.mark_chat_read` is **not** a notification - it's a journaled job (item 18c) that needs durability, so it stays a regular acked request.

    Notification handlers run on a separate task pool (cap 4) so a slow notification handler can't starve the request dispatcher. Wire-shape lockdown: the existing inbound parser routes by `id IS NULL` -> notification path, otherwise -> request path. Existing oversize-frame protection still applies. Symmetry with the Service-to-UI notification class taxonomy: client notifications declare their class on the `service-api` side; only `Drop` is needed for Phase 2.

12. **`ActionContext` reconstruction Service-side.** `crates/core/src/actions/context.rs::ActionContext` becomes Service-internal. Constructed once at Phase 2 boot from the `BootContext` (Phase 1.5 already holds `db_conn` + `encryption_key` + `recovery_warnings` waiting for Phase 2 to consume them - this resolves the `apply_standard_pragmas per-connection waste / BootContext.db_conn unused` carry-forward from the roadmap). The `in_flight: Arc<Mutex<HashSet<String>>>` dedup set lives Service-side; the UI also gets a client-side throttle in `service_client.rs` keyed by `(account_id, thread_id)` to avoid issuing two IPC roundtrips on a fast double-click. Both layers serve - the UI throttle reduces IPC pressure; the Service set is the canonical correctness gate.

13. **Generation counters bump pre-dispatch.** UI bumps `nav_generation` and `thread_generation` *before* sending the plan over IPC - not after `action.completed` arrives. The IPC delay creates a window where stale `ThreadsLoaded` / `NavigationLoaded` results can otherwise land between dispatch and ack and overwrite optimistic UI updates. This applies to `dispatch_plan` (the canonical path), the compose-send path, the undo path, and the snooze-resurfacing-tick path. Per `docs/architecture.md` § "Generation counters for async safety", these are `GenerationCounter<T>` instances; the bumps use `let _ = counter.next()` for invalidation-only side effects.

14. **Optimistic UI rollback path: tri-state (Pending / Acked / AckUnknown), reconciled via `action.job_status`.** "Did the UI observe the ack?" is **not** the same question as "did the Service journal the plan?". The Service can crash *after* commit while the ack is still in the OS pipe buffer or before the UI reader dispatches it; the request can also time out client-side while SQLite was waiting on its 15 s busy timeout (`crates/db/src/db/mod.rs:101`) and committed shortly after. Either case lands the UI in a state where rollback is unsafe (the journal has the row) and not-rolling-back is also unsafe (the UI thinks the plan is in flight forever). The fix is a tri-state and a reconciliation IPC method.

    `in_flight_plans[plan_id].state` is one of:
    - `Pending`: the IPC future has not resolved. Optimistic state is applied. No rollback path triggers yet.
    - `Acked`: `ActionPlanAck { journaled: true }` was observed by the UI. Plan is durable; `ServiceCrashed` does NOT trigger rollback; outcomes will arrive via journal replay.
    - `AckUnknown`: the IPC future resolved with `ClientError::ServiceCrashed` OR `ClientError::Timeout`, with no observed ack. The UI does not know whether the Service journaled. **Do NOT roll back yet, do NOT confirm the action yet.** Hold the optimistic state, mark the plan `AckUnknown`. After the next `boot.ready` (respawn handshake), the UI calls `action.job_status(plan_id)` (item 18d) for every `AckUnknown` plan and reconciles based on the response.

    Trigger paths for the rollback path:
    - `OperationOutcome` with `result: Failure(_)`: per-operation rollback fires when the failure notification arrives (independent of the plan's tri-state).
    - `ClientError::ServiceCrashed` / `Timeout` while in `Pending` -> transition to `AckUnknown`. Do nothing else until reconciliation.
    - `ClientError::ServiceCrashed` while in `Acked` -> stay in `Acked`; the respawned Service replays outcomes; the per-plan `applied_outcomes` set dedupes.
    - `action.job_status(plan_id)` returns `NotFound` -> transition to `RollBack`; clear optimistic state; the action was never durably journaled.
    - `action.job_status(plan_id)` returns `Journaled { status }` -> transition to `Acked`; let outcome replay drive completion.
    - `ClientError::SchemaVersionChanged` / `BootFailureReason::*` (terminal failures): emit `Terminal` and `iced::exit()`s - no rollback needed because the process is exiting.

    The applied-set (`in_flight_plans[plan_id].applied_outcomes: HashSet<OperationId>`) is consulted before each `OperationOutcome` is applied; idempotent application means re-emitted outcomes the UI already processed are no-ops. Distinguishing the tri-state from the prior pre-ack/acked binary is critical: the earlier framing would either roll back too aggressively (treating timeout as "definitely lost") or not aggressively enough (treating ack-pipe-buffered crashes as "definitely durable"). Both are wrong; `AckUnknown` plus the `action.job_status` query is the correct shape.

15. **`action.completed` after commit visibility.** The Service emits `action.completed` only after the per-plan transaction has committed and the result is observable from a fresh read connection (the `ReadDbState` connection pool the UI uses). Documented as an IPC contract on the method, not just behavior - Phase 2 + tests pin it. The contract is *commit visibility*, not WAL fsync: the DB runs `PRAGMA synchronous = NORMAL` and per-commit fsync is not guaranteed; the useful contract for UI refresh coherence is "any read-pool connection can see the write," which a `COMMIT` against the WAL satisfies. The fsync-tight version of the contract would conflict with the 16 ms latency target and would not buy any UI-visible coherence on top.

16. **Action latency benchmark. DROPPED.** A p99-budget CI smoke test would flap on shared runners more than it'd catch real regressions, and the budget itself becomes a load-bearing number with no clear owner. The "does it feel fast" check stays manual (matrix items 1-2 below: real-keyboard star-toggle on a seeded mailbox; bulk-archive of 200 threads mid-scroll). If a latency regression surfaces we'll measure it ad hoc rather than maintain a permanent CI gate.

17. **Per-operation idempotency contract.** Two paths re-issue operations: (a) the action-plan journal (item 18a) replays a plan op when the Service is respawned mid-execution; (b) the `pending_operations` periodic re-issues a single op after a transient retryable provider failure. Either path can land a duplicate `WireMailOperation` against an already-applied target. `WireMailOperation` (and the `MailOperation` it mirrors) must be idempotent at the wire level - re-archiving an already-archived thread is a no-op, not an error; re-marking a read thread as read is a no-op. Already true today (the action-service's local DB mutations check current state before applying); Phase 2 lifts the implicit contract into a doc-comment on `MailOperation`, `WireMailOperation`, and `RequestParams::ActionExecutePlan`. UI-side, the `applied_outcomes: HashSet<OperationId>` per plan provides the matching idempotency layer for replayed `OperationOutcome` notifications.

18. **Error-shape decisions.** `ActionError` variants need a per-variant "preserve across the boundary" decision. Default proposal: collapse provider-specific errors into `RemoteFailure { provider_message: String, http_status: Option<u16>, retryable: bool }`; preserve action-pipeline errors verbatim (`ThreadNotFound`, `AccountUnknown`, `OperationConflict { … }`). The retryable flag drives whether `pending_ops` re-enqueues. Locked into `service-api` so adding a new variant requires a wire decision.

18a. **Durable action-job journal (sibling-job model: `action_jobs` + `action_job_ops`).** The ack contract (item 3) requires the job to be durable before the handler returns. An earlier draft proposed `action_plans` + `action_plan_ops` with `MailOperation::Send` carried in the same op table; that doesn't work. Send has no `thread_id`, has attachments + a message payload + per-send progress, and a job-level `quiet` flag (item 18c) has no place in a plain ops table. Phase 2 ships a sibling-job model: a `kind`-discriminated `action_jobs` table for job identity / status / payload, plus a child `action_job_ops` table only for mail-thread-shaped jobs. Send and mark-chat-read live in `action_jobs` only; multi-op mail plans live in both.

   ```sql
   CREATE TABLE action_jobs (
       -- 128-bit UUIDv7 (time-ordered + collision-resistant across UI restarts;
       -- see PlanId discussion in scope item 2). Stored as 16-byte BLOB.
       job_id BLOB PRIMARY KEY,
       kind TEXT NOT NULL,
           -- 'mail_plan' | 'send' | 'mark_chat_read' | ... (extend per future jobs)
       account_id INTEGER NOT NULL,
       status TEXT NOT NULL,
           -- 'queued' | 'leased' | 'executing' | 'completed' | 'partial' | 'failed'
       quiet INTEGER NOT NULL DEFAULT 0
           CHECK (quiet IN (0, 1)),
       payload BLOB NOT NULL,
           -- Job-kind-specific serialized payload (see below).
       summary BLOB,
           -- Serialized PlanSummary / SendSummary / etc., populated on terminal status.
       lease_owner BLOB,
           -- Worker instance UUID currently leasing this job, NULL when idle.
       lease_expires_at INTEGER,
           -- UNIX millis. NULL when idle. Worker renews on long jobs; recovery
           -- reclaims expired leases.
       created_at INTEGER NOT NULL,
       updated_at INTEGER NOT NULL,
       CHECK (kind IN ('mail_plan', 'send', 'mark_chat_read')),
       CHECK (status IN ('queued', 'leased', 'executing', 'completed', 'partial', 'failed')),
       FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
   );

   CREATE TABLE action_job_ops (
       job_id BLOB NOT NULL,
       operation_id INTEGER NOT NULL,
       ordinal INTEGER NOT NULL,
       thread_id TEXT NOT NULL,
       operation BLOB NOT NULL,
           -- serialized WireMailOperation
       status TEXT NOT NULL,
           -- 'pending' | 'leased' | 'executing' | 'done' | 'failed' | 'conflict'
       outcome BLOB,
           -- serialized OperationResult; presence is the durable "result available"
           -- bit (replaces the earlier emitted_to_wire flag). Recovery replays any
           -- op with outcome IS NOT NULL on a non-quiet job whose parent job is not
           -- yet completed.
       lease_owner BLOB,
       lease_expires_at INTEGER,
       PRIMARY KEY (job_id, operation_id),
       UNIQUE (job_id, ordinal),
       CHECK (status IN ('pending', 'leased', 'executing', 'done', 'failed', 'conflict')),
       FOREIGN KEY (job_id) REFERENCES action_jobs(job_id) ON DELETE CASCADE
   );

   -- Account-fair scheduling + boot recovery scans. The first index lets the
   -- worker pick "next ready op per account, oldest job first"; the second
   -- speeds up the recovery pass that resets stale leases.
   CREATE INDEX action_job_ops_ready
       ON action_job_ops(job_id, ordinal)
       WHERE status = 'pending';
   CREATE INDEX action_jobs_status_account
       ON action_jobs(status, account_id, created_at);
   CREATE INDEX action_jobs_lease_expiry
       ON action_jobs(lease_expires_at)
       WHERE lease_expires_at IS NOT NULL;
   ```

   **Payload shapes by `kind`** (deserialized via tagged enum, key = `kind`):
   - `mail_plan`: empty payload (the per-op state lives in `action_job_ops`).
   - `send`: serialized `JournaledSend { account_id, send_id, message: SendWireMessage, attachments: Vec<JournaledAttachment> }` where `JournaledAttachment { content_hash, size, mime, filename, vault_path }` references Service-owned bytes (see item 5 for the staging-to-vault transfer that makes Send durable).
   - `mark_chat_read`: serialized `JournaledChatRead { account_id, chat_email, resolved_thread_ids: Vec<String> }`. Resolved thread set is captured at handler time so worker behavior is deterministic across respawns.

   **Account FK** points to `accounts(id)` (not `accounts(account_id)`); the table key is the integer `id` per `crates/db/src/db/schema/01_core.sql:3`. **`status` columns are `CHECK`-constrained** so a typoed status cannot silently strand a row outside the partial index. **`UNIQUE(job_id, ordinal)`** prevents duplicate ordinals from a buggy handler.

   **Lease-based scheduling.** Workers claim ready ops with `UPDATE action_job_ops SET status='leased', lease_owner=?, lease_expires_at=? WHERE status='pending' AND ... RETURNING ...` (SQLite supports `RETURNING` since 3.35). Leases expire (default 60 s; renewed by the worker for long-running ops). Boot recovery resets `status` from `leased` / `executing` back to `pending` for any row whose `lease_expires_at` is in the past or whose `lease_owner` is not the current Service incarnation. This pattern survives multi-respawn races without losing or duplicating work; `Notify` becomes a "please rescan" wakeup, not the queue itself.

   **Replay semantics (replaces `emitted_to_wire`).** On UI reconnection (post-respawn), the Service replays every `action_job_ops` row whose `outcome IS NOT NULL` for any non-`quiet` job whose parent `action_jobs.status` is not `completed`/`failed`. The UI's per-job `applied_outcomes: HashSet<OperationId>` dedupes; the durable bit on the Service side is the presence of `outcome`, not a separately-set flag. Avoids the earlier "outcome persisted but emitted_to_wire bit not set" gap. For `quiet` jobs, Service replays only the final `action.completed`-shape notification.

   **`pending_operations` is unchanged** (per item 8). It remains the per-op transient-retry queue for retryable RemoteFailures; `action_jobs` is the per-job execution journal. The journal write happens inside the request handler (before `ActionPlanAck` / `SendAck` / `MarkChatReadAck` return); the worker reads from the journal asynchronously, leases ready work, runs it, persists outcomes. This is the boundary that makes the ack durable.

18b. **Action-time search-index writes: temporary inconsistency.** Some action paths today write to the Tantivy index (notably delete). The Tantivy writer doesn't move Service-side until Phase 3 (it rides with sync's per-account writer cohabitation). Per the roadmap (`implementation-roadmap.md` § Phase 2), the Phase 2 plan must pick: **(a)** Phase 2 actions skip the index update, with Phase 3's minimal cross-store invariant pass dropping orphaned Tantivy docs whose message ids no longer exist. **(b)** Phase 2 actions defer the index update via a wire round-trip back to a UI-side writer. **(c)** Move the Tantivy writer earlier than Phase 3.

   **Phase 2 picks option (a).** Reasons: (b) inverts the Service-is-only-writer invariant the entire split exists to enforce; (c) tangles Phase 2 with the Phase 3 sync surgery and its boot-handshake "Tantivy index initialized" extension. (a) creates a temporary inconsistency window where a deleted thread's Tantivy entry survives until next Phase 3 boot. The Phase 3 minimal invariant pass already handles this (`docs/service/problem-statement.md` § Cross-store crash consistency).

   Phase 2 documents the temporary inconsistency on the `MailOperation::Trash` / delete handlers, ships a regression test that verifies the orphan is dropped on the next Phase 3 boot, and explicitly *removes* the search-write call sites when actions relocate Service-side (today's `SearchState` exposes write methods at `crates/search/src/lib.rs:288` and `:325` that are reachable from action paths). Phase 2 does NOT split `SearchState` - the writer half lockdown rides with sync in Phase 3. If a UI handler that currently writes the search index gets missed in the relocation grep, that's a Phase 2 regression and the test catches it.

18c. **Chat read-on-view relocates as a quiet job.** Per the roadmap's write-surface inventory (`docs/service/problem-statement.md`), `mark_chat_read_local_sync` is a Phase 2 surface. Today the UI's `crates/app/src/handlers/chat.rs:65` and `:71` call into the DB write transaction at `crates/db/src/db/queries_extra/chat.rs:218`. Phase 2 introduces `action.mark_chat_read { account_id, chat_email }` as a regular acked IPC request (NOT a fire-and-forget notification - it needs durability so the read-state mutation survives a Service crash mid-handler). The handler resolves the affected threads, journals an `action_jobs` row with `kind = 'mark_chat_read'`, `quiet = 1`, captures the resolved thread set in the journaled payload, and returns `MarkChatReadAck { job_id, journaled: true }`. Worker applies the read-state mutation; emits only `action.completed` (no per-operation `OperationOutcome` notifications, suppressed by the `quiet` flag per item 18a). The UI handler in `chat.rs` becomes `client.mark_chat_read(...).await`. This surface is named in the Phase 2 promotion criteria so a plan that ships without it is not "done."

18d. **`action.job_status(plan_id)` reconciliation IPC method.** The tri-state in item 14 needs a way to resolve `AckUnknown`. After every `boot.ready` (initial spawn + every respawn handshake), the UI iterates `in_flight_plans` for entries in `AckUnknown` state and calls `action.job_status(plan_id)` for each:
   ```rust
   pub enum JobStatusResponse {
       /// No journal row exists with this plan_id. The Service crashed
       /// before commit, OR the IPC was dropped before the handler ran.
       /// UI's optimistic state is wrong; roll back.
       NotFound,
       /// Journal row exists. UI's optimistic state is correct; the
       /// worker will replay outcomes, the per-plan applied_outcomes
       /// dedupes any duplicates the UI already saw.
       Journaled { status: JobStatus, summary: Option<PlanSummary> },
   }
   pub enum JobStatus {
       Queued,
       Leased,
       Executing,
       Completed,
       Partial,
       Failed,
   }
   ```
   The query is a small SELECT against `action_jobs WHERE job_id = ?`, fast even on a large mailbox. Timeout: 5 s (uses the same handler-only path as other journal queries; bypasses neither admission cap). Reconciliation MUST run before the UI dispatches any new actions for accounts that had `AckUnknown` plans, otherwise a new action could be optimistically applied on top of unresolved state. The UI tracks "reconciliation pending for account X" and gates `dispatch_plan` for that account until each `AckUnknown` resolves to either `Acked` (do nothing) or `RollBack` (clear optimistic state, mark plan dropped). Phase 2 ships the reconciliation flow as part of the post-boot handshake; Phase 1.5's `BootReady` -> `Ready` transition is extended with one async task per `AckUnknown` plan.

19. **Phase 1.5 carry-forwards close out**. Each items lands as part of Phase 2 and is named so the roadmap's carry-forward bullets can be deleted. See "Phase 1.5 carry-forward closeout" subsection at the bottom of `In scope`.

### In scope (Phase 1.5 carry-forward closeout)

These bullets come from `implementation-roadmap.md` Phase 2 § "Phase 1.5 carry-forward (close out as part of Phase 2)". Original framing was "each is in scope for Phase 2 implementation"; close-out reality has each item either CLOSED, PARTIAL, or DEFERRED to a later phase. The roadmap is the source of truth for the current home of each item; the bullets below are kept for design-rationale reference.

19a. **`ReadyApp::from_boot_ready` heavy synchronous init. DEFERRED to Phase 8.** Today `crates/app/src/app.rs::from_boot_ready` opens the DB, loads stores, parses bootstrap snapshots, restores pop-out windows synchronously. Original plan: relocate the body / inline / search store init to async tasks dispatched from a `BootingApp::update` arm so the splash stays responsive while they finish. Less load-bearing than expected once `ActionContext` moved Service-side; pure UI surgery, no Phase 2 blocker. Slotted into Phase 8 alongside the boot-path crash-recovery polish.

19b. **`apply_standard_pragmas` per-connection waste / `BootContext.db_conn` consumed. PARTIAL.** The action worker now consumes `db_conn` from `BootSharedState`; the encryption key is also reachable. The two-connection waste (Service worker + UI reader) remains; the third UI-side write connection collapses with the global write-half lockdown in Phase 6.

19c. **`SchemaVersionChanged` end-to-end test (`--test-fake-schema=N`). DEFERRED to Phase 8.** Original plan: a test-helper flag analogous to the existing `--test-fake-version`; a real-subprocess test that flips the value across SIGKILL and asserts `ClientError::SchemaVersionChanged` arrives via Terminal. Phase 2 introduced real schema-version sensitivity (the action worker depends on the schema being what the UI thinks it is), but the test didn't land in the same milestone. Slotted into Phase 8 alongside the rest of the crash-recovery / respawn test polish.

19d. **`from_boot_ready` re-loads encryption key. DEFERRED to Phase 6a.** The hard requirement from Phase 1.5 commit 30 carried into Phase 2 unchanged: plumb `BootContext::encryption_key` through the IPC boundary so the UI consumes the Service's already-validated key instead of re-reading the file. Two design options: (a) handle-based - Service holds raw bytes; UI calls `internal.encrypt_for_storage { plaintext } -> ciphertext`; (b) trusted-bytes-once - Service exports the bytes via a one-shot IPC method, UI keeps in memory. Default proposal: handle-based. The TOCTOU window the arch review flagged remains until this lands. Slotted into Phase 6a alongside the rest of the credential / account / preference write-surface relocations.

19e. **Pre-dispatch generation-counter bumps. CLOSED in Phase 2.** Same as `In scope` item 13. Closed as part of the `dispatch_plan` rewrite (task 10).

19f. **`parent_death` crate boundary. CLOSED in Phase 2.** Extracted to `crates/process-lifetime/` (task 1). Both `service` and `app` depend on it; the App -> Service dependency for `ProcessGuard` is gone.

19g. **`boot_progress::emit` per-phase regression test / class-aware helper. DEFERRED to Phase 8.** Original proposal: class-aware helper picking `try_send` for `Coalesce`/`Drop`, `send().await` for `MustDeliver`. First implementation attempt introduced a hang in the `service_subprocess` test cohort and was reverted; today's helper still uses `try_send` only, structurally incompatible with `MustDeliver` semantics. Slotted into Phase 8 alongside the entangled flaky-test root-cause (the same writer-task drain-ordering issue is the most likely cause of both).

19h. **`BootSharedState` flood resilience for `boot.ready`. CLOSED in Phase 2.** `boot_ready_inflight: AtomicBool` plus a cache-result-first check on the handler (task 22). Subsequent callers fail fast with `Backpressure` or read the cached result.

### Out of scope

- **Sync.** Sync still runs in the UI process. It moves in Phase 3.
- **Push.** Same.
- **Body/inline/search write-half lockdown.** Their writers stay UI-side until Phase 3 (sync) and Phase 6 (rest). Phase 2 only locks down `WriteDbState`.
- **Streaming progress for arbitrarily-long actions.** The notification model supports it; the *cadence* contract (coalesce per account, emit at most every N ms or every K messages) is a Phase 3 concern when sync drives the volume.
- **Cancellation of in-flight `action.execute_plan`** (e.g. user closes a window mid-bulk-archive). Plan runs to completion in Phase 2; explicit `action.cancel_plan` is a Phase 8 follow-up if the need surfaces.
- **Calendar mutations.** Series-vs-occurrence + RSVP semantics may not fit a flat `MailOperation` list. Phase 6.
- **OAuth flow relocation.** Phase 6.
- **Re-tuning the per-account concurrency limit.** Stays at 4.
- **Crashloop detection refinement, exponential backoff, status indicator.** Phase 8.
- **Marker-file gating for cross-store invariant pass.** Phase 8.

## Architecture

### `service-state` crate boundary

```
crates/
  db/                    -- ReadDbState (UI), shared SQL, schema
  service-state/         -- WriteDbState (Service-only)
                            depends on: db (for the underlying connection pool),
                                        crypto-key
                            depended on by: service, NOT app
  service/               -- consumes service-state
  app/                   -- does NOT list service-state in its Cargo.toml
```

`db::DbState` becomes `db::ReadDbState`. The internal `Arc<Mutex<Connection>>` pool moves to a private `db::ConnectionPool` type that both `ReadDbState` and `WriteDbState` wrap. `WriteDbState` is constructed in `crates/service-state/src/lib.rs::WriteDbState::new(pool: ConnectionPool, key: SecretKey) -> Self`, with the constructor `pub` only within the crate's own module surface and reachable from `service` via `use service_state::WriteDbState`. Any `app` call site that tries this `use` fails at link time.

The `db::DbState::conn()` and `db::DbState::from_arc()` raw-`Connection` escapes (`crates/db/src/db/mod.rs`) collapse:
- `ReadDbState::conn()` stays public for read paths.
- `WriteDbState::conn()` lives in `service-state` and is `pub(crate)` to that crate; consumers go through narrow `with_write_conn(|c: &Connection| ...) -> Result<R>` helpers.
- `from_arc` disappears - construction is one path (the boot sequence), and that path lives Service-side.

This is the "bare types are not enough" / "crate boundary, not visibility" pattern from `problem-statement.md` § "Type-level enforcement". Phase 2's work makes the `WriteDbState` type unreachable from the `app` crate at the dependency-graph level - not from a `pub(crate)` access check that future contributors can poke at.

### `process-lifetime` crate boundary

`crates/process-lifetime/` houses the `ProcessGuard` (UI-side) and `exit_if_parent_missing` (Service-side) currently in `crates/service/src/parent_death/`. Both `service` and `app` depend on it. The App -> Service dependency that today carries `parent_death::ProcessGuard` collapses; the Service crate becomes purely "the subprocess worker" without the UI reaching into it.

This is a small commit (mechanical move + import updates) but unblocks the conceptual cleanliness of the workspace. Lands as part of the `service-state` carve-up in the same PR so the workspace surgery is one event.

### Action service Service-side shape

```
service/src/handlers/action.rs          -- new: dispatch entry points
service/src/actions/                    -- new: relocated from core/src/actions
  mod.rs
  context.rs                            -- ActionContext (Service-internal)
  batch.rs                              -- batch_execute (unchanged behavior)
  pending.rs                            -- periodic drainer
  ...                                   -- per-action files (archive, trash, ...)
service/src/progress.rs                 -- new: IpcProgressReporter
```

`crates/core/src/actions/` retains the *resolution* and *planning* surface (`MailActionIntent`, `resolve_intent`, `build_execution_plan`, `MailOperation`, `MailUndoPayload`, `CompletionBehavior`, `ToggleField`) - everything UI-side needs to construct the `ActionWirePlan`. The execution surface (`batch.rs`, `context.rs`, per-action files like `archive.rs`/`trash.rs`/etc., `pending.rs`) physically relocates to `crates/service/src/actions/`. The action functions (`archive_thread`, `trash_thread`, `add_label`, etc.) stay shape-compatible - their signatures change from `(ctx: &ActionContext, ...)` to `(ctx: &ActionContext, ...)` with the new Service-internal `ActionContext`. No call-site changes inside the action files; the move is mostly mechanical.

The `pub(crate)` enforcement on the 7 thread-action DB helpers (`set_thread_read`, `set_thread_starred`, `set_thread_pinned`, `set_thread_muted`, `delete_thread`, `add_thread_label`, `remove_thread_label`) per `docs/architecture.md` § "Action service as mutation gate" extends naturally - they now live behind the `service-state` crate boundary as well as the `pub(crate)` gate.

### Wire types

`service-api/src/action.rs`:

```rust
pub struct PlanId(pub u64);
pub struct OperationId(pub u64);

pub struct ActionWirePlan {
    pub plan_id: PlanId,
    pub operations: Vec<ActionWireOperation>,
}

pub struct ActionWireOperation {
    pub operation_id: OperationId,
    pub account_id: String,
    pub thread_id: String,
    pub operation: WireMailOperation,
}

pub enum WireMailOperation {
    SetRead { is_read: bool },
    SetStarred { is_starred: bool },
    Archive,
    Trash,
    MoveToFolder { folder: WireFolderId },
    AddTag { tag: WireTagId },
    RemoveTag { tag: WireTagId },
    // ... mirrors core::actions::MailOperation 1:1.
}

pub struct ActionPlanAck {
    pub plan_id: PlanId,
    /// Plan has been written to action_jobs + action_job_ops (item 18a).
    /// UI's in_flight_plans[plan_id].state transitions to Acked on this ack
    /// (per item 14's tri-state); from this point a ServiceCrashed does NOT
    /// trigger optimistic rollback. Loss of this ack on the wire transitions
    /// the UI to AckUnknown, reconciled via action.job_status (item 18d).
    pub journaled: bool,
}
```

`service-api/src/notification.rs` extensions:

```rust
pub enum Notification {
    BootProgress(BootProgress),
    OperationOutcome(OperationOutcome),    // MustDeliver, WithGeneration
    ActionCompleted(ActionCompleted),      // MustDeliver, WithGeneration
    SyncProgress(SyncProgress),            // Coalesce { key: SyncProgress(account_id) }
    // (Phase 3+ extends)
}

pub struct OperationOutcome {
    pub plan_id: PlanId,
    pub operation_id: OperationId,
    pub result: OperationResult,
    pub service_generation: u32,
}

pub struct ActionCompleted {
    pub plan_id: PlanId,
    pub summary: PlanSummary,
    pub service_generation: u32,
}

pub enum OperationResult {
    Success,
    LocalOnly,                            // local DB write succeeded; provider deferred to pending_ops
    RemoteFailure(RemoteFailure),
    ConflictRejected { detail: String },
}

pub struct PlanSummary {
    pub total: u32,
    pub local_only: u32,
    pub remote_succeeded: u32,
    pub remote_failed: u32,
    pub conflicts: u32,
}
```

Both `OperationOutcome` and `ActionCompleted` implement `WithGeneration` per Phase 1.5 commit 30's contract. The `production_notification_catalog()` test grows two entries; the catalog round-trip test catches "I forgot to tag my new variant" automatically.

### Service-side dispatch flow

The handler-vs-worker split is critical: the dispatch loop sends a JSON-RPC response only after the request handler future returns, so the handler must NOT drive `batch_execute`. Handler does enqueue + signal; worker does lease + execution + outcome emission + completion. `Notify` is a wakeup signal, not a work queue - the journal is the source of truth.

```
1. UI: build ActionWirePlan from ActionExecutionPlan.
        Convert MailOperation -> WireMailOperation.
        Generate plan_id = UUIDv7.
        Stash UI metadata in in_flight_plans[plan_id] =
          PlanCompletionMeta {
            state: Pending,                    // tri-state per item 14
            applied_outcomes: HashSet<OperationId>,
            ...,
          }
2. UI: bump nav_generation, thread_generation (pre-dispatch invalidation).
3. UI: apply optimistic updates (existing logic).
4. UI: client_throttle.try_acquire((account_id, thread_id))  [client-side dedup]
5. UI: service_client.execute_plan(plan).await -> ActionPlanAck | Err(...)
        On ActionPlanAck:
          UI transitions in_flight_plans[plan_id].state = Acked.
          ServiceCrashed from now on does NOT roll the plan back -
          the journal will replay it on respawn.
        On Err(ServiceCrashed | Timeout):
          UI transitions state = AckUnknown. Hold optimistic state.
          On next boot.ready, call action.job_status(plan_id) (item 18d)
          to reconcile to either Acked or RollBack.
        On Err(other): treat as failure; roll back per item 14.
6. Service handler: in a single transaction
        - INSERT action_jobs(job_id=plan_id, kind='mail_plan', account_id,
            status='queued', quiet=0, payload=<empty>, lease_owner=NULL,
            lease_expires_at=NULL, created_at=now, updated_at=now)
        - INSERT one action_job_ops row per operation (status='pending')
        Notify the worker pool ("please rescan") and return ActionPlanAck.
7. Service worker (account-fair scheduler):
        On wakeup, repeatedly:
          - Lease the next ready op:
              UPDATE action_job_ops
              SET status='leased', lease_owner=worker_uuid,
                  lease_expires_at=now+60s
              WHERE (job_id, ordinal) = (
                  SELECT ops.job_id, ops.ordinal
                  FROM action_job_ops ops
                  JOIN action_jobs jobs USING (job_id)
                  WHERE ops.status = 'pending'
                    AND <account_fair_scheduling_predicate>
                  ORDER BY jobs.created_at, ops.ordinal
                  LIMIT 1
              )
              RETURNING ...;
          - If no rows: stop draining, await Notify or lease-expiry timer.
          - Else: run via batch_execute, persist outcome+status in one tx
            (status -> done/failed/conflict, outcome -> serialized result),
            emit OperationOutcome (MustDeliver), and on retryable
            RemoteFailure also enqueue into pending_operations for the
            transient-retry periodic.
        Account-fair scheduling: at most 4 leased ops per account at a
        time (separate per-account semaphore on the worker side); the
        SELECT ORDER BY also filters out accounts that are at their
        leased-op cap. Notify wakes ALL workers; each worker drains
        what it can.
8. Service worker: when the last op in a job transitions out of
        pending/leased/executing, set action_jobs.status = 'completed'
        | 'partial' | 'failed' and persist summary in the same tx.
        Emit action.completed AFTER the COMMIT is visible to the read
        connection (commit visibility, not WAL fsync).
9. UI: each OperationOutcome arrives ->
        Message::OperationOutcomeReceived(...) ->
          if (plan_id, operation_id) already in applied_outcomes:
              drop (idempotent)
          else:
              apply completion-effects per stashed metadata
              (toast, undo eligibility, optimistic rollback on Failure)
              applied_outcomes.insert((plan_id, operation_id))
10. UI: ActionCompleted arrives ->
         Message::ActionCompletedReceived(plan_id) ->
         remove in_flight_plans[plan_id]; release client_throttle.
```

Crash handling at each step:
- (5) crash before ack observed: UI sees `ClientError::ServiceCrashed` or `Timeout`; transitions state = `AckUnknown`. On next `boot.ready`, calls `action.job_status(plan_id)`. `NotFound` -> roll back; `Journaled` -> stay in `Acked` and let replay drive completion.
- (5) crash after ack observed: plan IS journaled, UI is in `Acked` state. The respawned Service drains the journal (lease recovery resets stale leases) and replays outcomes; idempotent application dedupes anything the UI already saw.
- During (7) execution crash: per-operation outcomes that landed before the crash are already applied UI-side (recorded in `applied_outcomes`). Lease recovery resets `leased`/`executing` rows whose `lease_expires_at` is in the past or whose `lease_owner` is not the current Service incarnation back to `pending`. The worker re-runs them; idempotent application dedupes.
- During (8) crash before commit: `action_jobs.status` stays at its pre-commit value; the new Service's worker re-evaluates whether the job is complete based on `action_job_ops` row states and writes the terminal status. UI never sees `action.completed` for this plan from the dying incarnation; the new one emits it.

Idempotent application is the chosen contract over per-outcome acks: re-emitting an OperationOutcome the UI has already processed is a no-op because the optimistic-state mutation is already settled and the applied-set dedupes. No `action.outcome_acked` chatter on the wire. **The "outcomes arrive in strict ordinal order" promise is dropped** - account-fair leasing means a fast op for account A and a slow op for account B can interleave; the test that asserted strict order is replaced with one that asserts every op for a given plan eventually arrives exactly once and the final `action.completed` arrives last.

### Generation counters across the IPC boundary

UI bumps pre-dispatch:
```rust
fn dispatch_plan(plan: ActionExecutionPlan) -> Task<Message> {
    let plan_id = next_plan_id();
    self.in_flight_plans.insert(plan_id, plan_meta_from(&plan));

    // Pre-dispatch invalidation - PRE-IPC, not post-completion.
    // Without these bumps, a stale ThreadsLoaded landing between the
    // dispatch and the OperationOutcome would overwrite the optimistic
    // update.
    let _ = self.nav_generation.next();
    let _ = self.thread_generation.next();

    apply_optimistic_updates(&plan);
    let wire = ActionWirePlan::from_resolved(plan, plan_id);
    Task::perform(
        async move {
            client.execute_plan(wire).await
        },
        |ack| Message::ActionDispatched(ack),
    )
}
```

The cross-process `service_generation` tag (Phase 1.5) and the within-process `GenerationCounter<T>` (per `docs/architecture.md` § "Generation counters for async safety") solve different problems:
- `service_generation`: which Service incarnation emitted this notification?
- `GenerationCounter<T>`: which UI dispatch round produced this load result?

Both apply to action notifications. The `service_generation` filter lives in `notification_should_dispatch` (Phase 1.5's gate). The within-process generation check happens at the completion-effects handler when correlating plan_id back to optimistic state.

### Action latency benchmark (DROPPED)

A p99-budget CI smoke test was originally planned at `crates/service/tests/action_latency_smoke.rs` with a 16 ms star-toggle target (one frame at 60 fps). Dropped during Phase 2 close-out: a budget-enforcing test reliably flaps on shared CI runners more than it catches real regressions, and the budget itself becomes a load-bearing number with no clear owner. Latency stays a manual-matrix concern (items 1-2 in `## Verification`); ad hoc measurement is the response if a regression surfaces.

## Detailed task list

In recommended commit order. Each item is one focused commit unless noted.

1. **`process-lifetime` crate.** Mechanical move of `crates/service/src/parent_death/` -> `crates/process-lifetime/`. Update `service` and `app` Cargo.toml dependencies. No behavior change. Zero risk if the move is correct; integration tests catch any miswire.

2. **`service-state` crate (scaffold).** New crate with `WriteDbState` type that is identical-in-shape to today's `DbState`. App still uses `DbState`; nothing locks down yet. This is the type-level scaffold so the next commit can lift call sites.

3. **`db::DbState` -> `db::ReadDbState` + private `db::ConnectionPool`.** `DbState` becomes `ReadDbState` (rename). The internal `Arc<Mutex<Connection>>` pool extracts to a private `ConnectionPool` type. `ReadDbState` exposes `conn()` for read paths only; write methods on the old `DbState` API split out and become `pub(crate)` to a new `db::write_helpers` module that `service-state::WriteDbState` consumes. This commit is the type-rename surgery; cargo check fails at every UI write call site, which the next two commits fix.

4. **`service-api`: action wire types.** `PlanId`, `OperationId`, `ActionWirePlan`, `ActionWireOperation`, `WireMailOperation` (1:1 mirror of `core::actions::MailOperation` with serde derives), `WireFolderId` / `WireTagId`, `ActionPlanAck { plan_id, journaled }`, `OperationResult`, `OperationOutcome`, `ActionCompleted`, `PlanSummary`, `RemoteFailure`. Type-only commit; serde round-trip tests; `RequestParams::ActionExecutePlan.timeout() == 5s` (handler is just enqueue + signal; SMTP/long work happens in the worker); `WithGeneration` impl on `OperationOutcome` and `ActionCompleted`; `Notification::class()` arms return `MustDeliver`; `production_notification_catalog()` extended; static-assertion test that every `MailOperation` variant has a corresponding `WireMailOperation` variant (catches drift when adding a new action variant in core).

5. **`service`: `IpcProgressReporter`.** New `crates/service/src/progress.rs::IpcProgressReporter` impl of `db::ProgressReporter` that posts `Notification::SyncProgress(...)` via the existing `boot_progress::emit_classified` helper (extended to take a `Notification` directly).

6. **`service`: relocate action service.** Move `crates/core/src/actions/` -> `crates/service/src/actions/` keeping the *resolution* and *planning* types in core. The `MailActionIntent` / `resolve_intent` / `build_execution_plan` / `MailOperation` / `MailUndoPayload` / `CompletionBehavior` / `ToggleField` surface stays in `crates/core/src/actions_resolve/` (or stays at `crates/core/src/actions/resolve/`, design choice). The execution surface (`batch.rs`, `context.rs`, per-action files, `pending.rs`) moves to `crates/service/src/actions/`. `ActionContext` becomes Service-internal and consumes `BootContext` from Phase 1.5. `batch_execute` signature change: `&ActionContext` stays; the surrounding wiring moves.

7. **`common`: `ProviderCtx` split via traits.** Define `ActionDbWrite` and `ActionKeyAccess` traits in `crates/common/src/ops.rs`. `ActionProviderCtx<'a> { account_id: &'a str, db_write: &'a dyn ActionDbWrite, key: &'a dyn ActionKeyAccess, progress: &'a dyn ProgressReporter }` - all trait objects, no `service-state` types. `service-state::WriteDbState` and the encryption-key holder implement these traits. `SyncProviderCtx<'a>` is added as a type-only scaffold in `common` for Phase 3, but **sync method signatures stay on the existing `ProviderCtx` shape in Phase 2** - `body_write: ()` placeholders would not compile against live sync, which still needs real handles. Phase 3 introduces the sync trait split alongside the actual writer relocation. Phase 2's mechanical work covers every action method on `ProviderOps`, its impls in the four provider crates, and the `ActionContext` plumbing inside the Service.

8. **`db`: action-job journal tables (`action_jobs` + `action_job_ops`).** New schema bump in `crates/db/src/db/migrations.rs` plus a new SQL file under `crates/db/src/db/schema/`. Tables per item 18a in scope (sibling-job model with `kind`-discriminated `action_jobs` and `kind = 'mail_plan'`-only child rows in `action_job_ops`). Account FK references `accounts(id)` (verify against `crates/db/src/db/schema/01_core.sql:3`); `status` columns are CHECK-constrained; UNIQUE (`job_id`, `ordinal`) on the ops table. Indexes: `(job_id, ordinal) WHERE status='pending'` for worker scheduling, `(status, account_id, created_at)` for boot recovery, `(lease_expires_at) WHERE NOT NULL` for lease-expiry sweeps. Add narrow write helpers (`with_journal_write_conn(|c| ...)`, `lease_next_ready_op(...)`, `release_expired_leases(...)`, `mark_op_complete(...)`, etc.) under `pub(crate)` to a new `db::action_journal` module that `service-state::WriteDbState` re-exposes through `ActionDbWrite`. Boot-time recovery resets `leased`/`executing` rows whose `lease_owner` is not the current Service incarnation OR whose `lease_expires_at` is in the past back to `pending`; wires into `recover_on_boot_db_only` so it runs Service-side at the same point Phase 1.5 already drains `pending_operations`.

9. **`service`: `action.execute_plan` handler + worker.** Two pieces, one commit. Handler in `crates/service/src/handlers/action.rs::handle_execute_plan` validates the plan, in a single transaction inserts the `action_jobs` row (`kind='mail_plan'`) and one `action_job_ops` row per operation, signals the worker pool via a shared `tokio::sync::Notify` (Notify is the wakeup, not the queue), returns `ActionPlanAck { plan_id, journaled: true }`. Worker pool in `crates/service/src/actions/worker.rs` runs in its own task pool (cap 4 per account, separate per-account semaphore); on wakeup, repeatedly leases the next ready op via `UPDATE ... RETURNING` (DB is the source of truth), runs via `batch_execute`, persists outcome+status in a single transaction, emits `OperationOutcome`. On plan completion (no more pending/leased/executing ops for the job), updates `action_jobs.status` and emits `action.completed` only after the commit is visible to the read connection. Replay semantics: presence of `outcome IS NOT NULL` is the durable bit (no separate `emitted_to_wire` flag); on UI reconnection the Service replays every op with an outcome whose parent job is not yet terminal. Handler timeout: 5 s. Worker has no IPC timeout (it runs to completion or until respawn).

10. **`app`: dispatch_plan rewrite.** `crates/app/src/handlers/commands.rs::dispatch_plan` becomes `client.execute_plan(wire_plan).await`. Convert `MailOperation` -> `WireMailOperation` at the boundary. Pre-dispatch generation bumps (`nav_generation.next()`, `thread_generation.next()`). Optimistic updates stay UI-side. UI metadata stash: `in_flight_plans: HashMap<PlanId, PlanCompletionMeta { acked, applied_outcomes, ... }>` on `ReadyApp`. New messages: `ActionDispatched(ActionPlanAck)`, `OperationOutcomeReceived(OperationOutcome)`, `ActionCompletedReceived(ActionCompleted)`. Removes the direct `Task::perform(batch_execute(...))` call.

11. **`app`: tri-state in-flight tracking + post-respawn reconciliation via `action.job_status`.** `PlanCompletionMeta.state: PlanState` is one of `Pending` / `Acked` / `AckUnknown`. On `ClientError::ServiceCrashed | Timeout` while in `Pending`, transition to `AckUnknown` (do NOT roll back yet). On `Acked`, do not roll back at all - the journal will replay. On `boot.ready` post-respawn, the UI dispatches one async task per `AckUnknown` plan that calls `action.job_status(plan_id)` (item 18d) and reconciles to `Acked` (`Journaled`) or `RollBack` (`NotFound`). Plan dispatch for the affected account is gated until reconciliation completes for all pending `AckUnknown` plans on that account, so a new action cannot land on top of unresolved optimistic state. Tests: kill Service before ack (pre-ack -> AckUnknown -> reconcile -> NotFound -> rollback); kill Service after ack (Acked -> stays Acked -> replay applies cleanly); kill Service in the ack-pipe-buffered race (AckUnknown -> reconcile -> Journaled -> stay Acked).

11a. **`service` + `app`: `action.job_status` IPC method (scope item 18d).** New small SELECT-only handler on the Service: `handle_job_status(plan_id) -> JobStatusResponse { NotFound | Journaled { status, summary } }`. 5 s timeout. Bypasses neither admission cap; the SELECT is fast even on a million-row journal because of the primary-key index. UI: a `client.job_status(plan_id)` call wired into the post-`boot.ready` reconciliation flow.

11b. **`service-api`: client-to-service notification framing (scope item 11).** Extend `framing.rs` to accept `id`-less inbound JSON-RPC envelopes as client notifications. New `ClientNotification` enum in `service-api` with `Drop`-class admission. Phase 2 uses for `pending_ops.kick`; future phases extend. Notification handlers run on a separate task pool (cap 4) so they cannot starve the request dispatcher. Tests: ack-less message round-trip; oversize notification rejected; admission cap enforced; existing inbound parse path still rejects malformed envelopes.

12. **`app`: client-side action throttle.** New `client_throttle: HashMap<(AccountId, ThreadId), Instant>` on `ReadyApp` (or inside `service_client.rs`). 200 ms debounce. Prevents two IPC roundtrips on a fast double-click; complements the Service's `ActionContext::in_flight` set.

13. **`service`: `action.send` handler with bytes-ownership transfer before ack.** **LANDED** (split into six commits per the implementation, not the single-commit shape sketched here).
    - **Wire types** (`crates/service-api/src/action.rs`): `SendWireRequest` { send_id, from_account_id, message, attachments }, `SendWireMessage`, `SendWireAttachment`, `SendAttachmentSource::StagingFile { relative_path, content_hash }`, `SendAck`. The `PackStore { content_hash }` variant is **deferred to Phase 6** - pack-store eviction doesn't land until Phase 6, so a refcount bump in Phase 2 has no eviction to keep alive against.
    - **`send_vault` module** (`crates/service/src/send_vault.rs`): path validation (rejects `..`, absolute, NUL, rooted-or-prefixed), `verify_and_transfer` (lstat-checked symlink rejection + SHA-256 verify + atomic rename, same-FS only), `cleanup_vault_dir` for terminal unlink, `cleanup_orphan_vaults` for boot recovery.
    - **`action.send` handler** (`crates/service/src/handlers/action_send.rs`): mirrors `handle_mark_chat_read` shape. spawn_blocking validates + transfers all attachments + builds `JournaledSend` payload; second spawn_blocking journals as `insert_quiet_job(kind='send')`. Pre-journal failure cleans up partial vault state. Returns `SendAck { send_id, journaled: true }`. Handler timeout 30 s (covers SHA-256 verify of gigabyte-class attachments).
    - **Worker `drain_send_jobs`** (`crates/service/src/actions/worker.rs`): leases via `lease_next_quiet_job_via_blocking("send")`, deserializes `JournaledSend`, reads vault bytes, reconstructs `SendRequest`, calls existing `super::send::send_email` (unchanged from its previous direct-from-UI invocation), finalizes job + unlinks vault dir + emits `ActionCompleted`. Worker-layer errors finalize as Failed + cleanup vault using job_id-as-send_id mapping.
    - **Boot orphan cleanup** (`crates/service/src/boot.rs::reconcile_send_vault`): `db::action_journal::live_send_job_ids` returns the set of non-terminal `kind='send'` jobs; `cleanup_orphan_vaults` removes everything else from `<app_data>/send_vault/`. Folded into the recovery block alongside `pending-ops recovery`. Failures surface as `recovery_warnings`.
    - **App rewire** (`crates/app/src/handlers/pop_out/compose_send.rs::dispatch_send`): writes bytes to `<app_data>/staging/<send_id>/<index>.bin`, computes SHA-256 per attachment, builds `SendWireRequest`, calls `client.send_email`. Inserts `send_id -> window_id` into `in_flight_sends: HashMap<PlanId, iced::window::Id>`; `handle_notification_action_completed` short-circuits on `in_flight_sends` lookup to fire `Message::SendCompleted` against the right window. Staging directory unlinks in the result closure regardless of ack outcome. **`action_ctx` is no longer required for send** (calendar + contacts still use it - those relocate in Phase 6).

    No `SendCompleted` notification - the `ActionCompleted { plan_id: send_id, summary { remote_succeeded: 1 } }` shape is sufficient for Phase 2 (no SMTP progress notifications). If Phase 6 adds upload-progress, a dedicated `SendProgress` notification can layer on top.

14. **`service`: `action.undo` flow.** UI builds the reverse `ActionWirePlan` from the stashed `MailUndoPayload`; submits via `action.execute_plan`. No new wire shape. Test: dispatch action, observe completion, undo, observe inverse outcome.

15. **`service`: `action.mark_chat_read` handler (scope item 18c).** Acked IPC method `action.mark_chat_read { account_id, chat_email }`. Service resolves affected threads (captured in the journaled payload for deterministic replay), journals an `action_jobs` row with `kind = 'mark_chat_read'`, `quiet = 1`, returns `MarkChatReadAck { job_id, journaled: true }`. Worker emits only `action.completed` (no per-operation `OperationOutcome` notifications, suppressed by `quiet` per item 18a). UI's `crates/app/src/handlers/chat.rs:65` and `:71` become `client.mark_chat_read(...).await`. Removes the direct DB write at `crates/db/src/db/queries_extra/chat.rs:218` from UI reach.

16. **`service`: remove action-time search-index writes (scope item 18b).** Action paths today reach into `SearchState` write methods (`crates/search/src/lib.rs:288`, `:325`) - notably the delete path. After actions relocate Service-side, those call sites are removed (Phase 2 picks option (a): action skips index update; Phase 3's invariant pass cleans up orphans). Phase 2 does NOT split `SearchState` - that lockdown rides with sync in Phase 3. Test: delete a thread; verify the Tantivy entry survives in Phase 2; verify Phase 3's invariant pass would drop it (assertion against the orphan-detection function shape, not its full execution).

17. **`service`: snooze resurfacing relocation.** `crates/service/src/snooze_runner.rs` triggered by UI `pending_ops.kick` notifications. Service queries snooze table, dispatches resurface operations as a normal `action.execute_plan`. UI's `SyncTick` path becomes a `client.kick_pending_ops()` call.

18. **`service`: pending_ops periodic drainer relocation.** `crates/service/src/actions/pending.rs::process_pending_ops` runs Service-side, triggered by `pending_ops.kick`. UI removes the periodic dispatch.

19. **`service` + `app`: encryption key handle (Phase 1.5 carry-forward 19d). DEFERRED to Phase 6a.** Sketched here for future reference: handle-based access (Service holds `SecretKey` in `BootContext`; UI's `from_boot_ready` no longer calls `rtsk::load_encryption_key`; internal IPC method `internal.encrypt_for_storage { plaintext, label } -> ciphertext` covers credential persistence). Slotted into Phase 6a alongside the rest of the credential / account / preference write-surface relocations - see `implementation-roadmap.md` Phase 6a "In scope" entry.

20. **`service` + `app`: `--test-fake-schema=N` end-to-end test (Phase 1.5 carry-forward 19c). DEFERRED to Phase 8.** Sketched here: test-helper flag analogous to `--test-fake-version`; real-subprocess test asserts `ClientError::SchemaVersionChanged` arrives via Terminal across SIGKILL. Slotted into Phase 8 alongside the rest of the crash-recovery / respawn test polish.

21. **`app`: `from_boot_ready` async store init (Phase 1.5 carry-forward 19a). DEFERRED to Phase 8.** Sketched here: body / inline / search store init relocates to async tasks dispatched from `BootingApp::update`; the `Booting -> Ready` transition fires earlier (right after `BootReady`); async store-init tasks fire `Message::ReadyStoreReady(...)` events that finalize the `ReadyApp` field set incrementally. Slotted into Phase 8 because the crash-recovery polish already touches the boot path.

22. **`service`: `BootSharedState` flood resilience (Phase 1.5 carry-forward 19h).** `AtomicBool` "already in flight" on the `boot.ready` handler. Subsequent callers either join the existing waiter or fail fast.

23. **`service-api`: class-aware `boot_progress::emit` helper (Phase 1.5 carry-forward 19g). DEFERRED to Phase 8.** First attempt (try_send for Coalesce/Drop, awaited send for MustDeliver) introduced a hang in the `service_subprocess` test cohort and was reverted; today's helper still uses `try_send` only. Slotted into Phase 8 alongside the entangled flaky-test root-cause - see `implementation-roadmap.md` Phase 8 "In scope" entry.

24. **`service`: action latency benchmark. DROPPED.** See `## Architecture` § "Action latency benchmark (DROPPED)" for rationale. Latency is a manual-matrix concern; no CI gate.

25. **`service` + `app`: error-shape decisions.** `RemoteFailure` wire type lockdown; per-`ActionError`-variant decision recorded in `service-api`'s error module doc-comment.

26. **`docs/service`: update problem-statement.md and implementation-roadmap.md.** Reflect Phase 2's settled items: action service relocation, `service-state` + `process-lifetime` crate boundaries, `ActionWirePlan` + `WireMailOperation`, action-plan journal, per-plan correlation, generation-counter pre-dispatch contract, ack-state-scoped optimistic rollback, action latency benchmark, encryption-key handle, attachment refs for send, search-write inconsistency policy. Delete the Phase 1.5 carry-forward bullets that are now closed. Bundle with the implementation commit that ships per CLAUDE.md.

## File-by-file changes

**New crates:**
- `crates/process-lifetime/` - `ProcessGuard` + `exit_if_parent_missing` extracted from service.
- `crates/service-state/` - `WriteDbState` constructor reachable only from `service`.

**New files:**
- `crates/service-api/src/action.rs` - wire types: `ActionWirePlan`, `WireMailOperation` (1:1 mirror of core's `MailOperation`), `WireFolderId` / `WireTagId`, `ActionPlanAck { plan_id, journaled }`, `OperationResult`, `OperationOutcome`, `ActionCompleted`, `PlanSummary`, `RemoteFailure`, `SendWireRequest`, `SendWireAttachment`, `SendAttachmentSource`.
- `crates/service/src/handlers/action.rs` - dispatch handlers: `handle_execute_plan` (handler, just enqueue+signal), `handle_send` (handler), `handle_mark_chat_read` (handler).
- `crates/service/src/actions/` - relocated execution surface from `crates/core/src/actions/` (batch, context, per-action files, pending periodic).
- `crates/service/src/actions/worker.rs` - the action worker that drives `batch_execute` against journal rows; emits `OperationOutcome` and final `action.completed`.
- `crates/service/src/snooze_runner.rs` - timer-driven resurfacing.
- `crates/service/src/progress.rs` - `IpcProgressReporter`.
- `crates/app/src/in_flight_plans.rs` - UI-side correlation map: `PlanCompletionMeta { acked, applied_outcomes, ... }`.
- `crates/db/src/db/schema/NN_actions.sql` (or extension to existing schema file) - `action_jobs` + `action_job_ops` tables (sibling-job model per scope item 18a).
- `crates/db/src/db/action_journal.rs` - narrow `pub(crate)` write helpers for the journal tables.

**Modified files:**
- `Cargo.toml` (workspace) - register `process-lifetime`, `service-state` crates.
- `crates/db/src/db/mod.rs` - `DbState` -> `ReadDbState` rename; private `ConnectionPool`; raw-`Connection` escapes deleted.
- `crates/db/src/db/migrations.rs` - schema bump for the action-plan journal tables.
- `crates/db/src/lib.rs` - re-exports.
- `crates/common/src/ops.rs` - new `ActionDbWrite`, `ActionKeyAccess` traits; `ActionProviderCtx<'a>` shape with trait objects; `SyncProviderCtx<'a>` scaffolded but unused; `ProviderOps` action methods take `ActionProviderCtx` (sync methods unchanged in Phase 2).
- `crates/{gmail,jmap,graph,imap}/src/ops.rs` - update action method impls to take `ActionProviderCtx`; sync impls unchanged in Phase 2.
- `crates/core/src/actions/` - keep resolution + planning surface; remove batch + context + per-action execution; remove direct `SearchState` write call sites (they no longer compile through the moved action code).
- `crates/core/src/actions/operation.rs` - kept; no serde derives added (the wire mirror lives in `service-api` per scope item 2).
- `crates/service-api/src/request.rs` - `ActionExecutePlan { plan }` (5 s timeout), `ActionSend { request: SendWireRequest }` (5 s handler timeout; SMTP is on the worker), `ActionMarkChatRead { account_id, chat_email }`, `PendingOpsKick`.
- `crates/service-api/src/notification.rs` - `OperationOutcome`, `ActionCompleted`, `SendCompleted`, `SyncProgress` variants; `WithGeneration` impls; catalog extension.
- `crates/service-api/Cargo.toml` - dep on `core` if `WireMailOperation` lives in `service-api` and uses any core typed-ID types (default: no - wire types stay self-contained).
- `crates/service-state/src/lib.rs` - `WriteDbState`; impls of `ActionDbWrite` and `ActionKeyAccess`.
- `crates/service/src/lib.rs` - boot-time action service init from `BootContext` (consumes `db_conn` + `encryption_key`); spawn worker task; orphan-staging-dir cleanup at boot.
- `crates/service/src/handlers/mod.rs` - register action handlers.
- `crates/service/src/parent_death/mod.rs` - DELETE (moved to `process-lifetime`).
- `crates/app/src/app.rs` - drop `App.action_ctx` field + `action_ctx()` method; add `in_flight_plans: HashMap<PlanId, PlanCompletionMeta>`; UI never re-reads the encryption key (consumes Service handle per carry-forward 19d).
- `crates/app/src/handlers/commands.rs` - `dispatch_plan` rewrite; pre-dispatch generation bumps; new message arms; ack-state-scoped rollback.
- `crates/app/src/handlers/chat.rs` - `mark_chat_read_local_sync` call sites become `client.mark_chat_read(...)`.
- `crates/app/src/handlers/pop_out/compose_send.rs` - `dispatch_send` writes attachments to `<app_data>/staging/<send_id>/`, then issues `client.send(SendWireRequest)`.
- `crates/app/src/handlers/pop_out/compose_draft.rs` - draft save/delete via IPC.
- `crates/app/src/action_resolve.rs` - splits into "build wire plan" (with `MailOperation -> WireMailOperation` conversion) + "stash UI metadata keyed by plan_id".
- `crates/app/src/message.rs` - `ActionDispatched`, `OperationOutcomeReceived`, `ActionCompletedReceived`, `SendCompletedReceived` variants; updated Booting whitelist.
- `crates/app/src/update.rs` - dispatch the new messages.
- `crates/app/src/service_client.rs` - new `execute_plan`, `send`, `mark_chat_read`, `kick_pending_ops` methods; respawn-aware ack-state-scoped rollback wiring.
- `crates/search/src/lib.rs` - no API change; the writer methods stay (Phase 3 locks them down). Action-side call sites that currently call them are removed when actions relocate.

**Deleted files:**
- `crates/service/src/parent_death/` - moved to `process-lifetime`.

## Test plan

### Unit tests

- `service-api`: serde round-trip for `ActionWirePlan`, `WireMailOperation`, `OperationOutcome`, `ActionCompleted`, `RemoteFailure`, `SendWireRequest`. `RequestParams::ActionExecutePlan.timeout()` returns 5 s; `RequestParams::ActionSend.timeout()` returns 5 s (worker handles the long path). Catalog test (existing, from Phase 1.5 commit 30) automatically covers the new `MustDeliver` variants. Static-assertion test that every `core::actions::MailOperation` variant has a corresponding `WireMailOperation` variant (a `match` on `MailOperation` covering all variants -> compile-time enforcement that the wire mirror is exhaustive).
- `service-state`: `WriteDbState::new` constructs cleanly; `with_write_conn(|c| ...)` returns the result; raw `Connection` is not exposed; `ActionDbWrite` and `ActionKeyAccess` impls match the trait contract.
- `service`: `ActionContext` constructs from `BootContext` (consumes `db_conn` + `encryption_key`). Worker leases ready `action_job_ops` via `UPDATE ... RETURNING` (DB is the source of truth, not `Notify`); account-fair scheduling caps leased ops per account at 4. `IpcProgressReporter::emit_json` enqueues a `SyncProgress` notification with the right `Coalesce` key. Boot recovery resets `leased`/`executing` rows whose `lease_owner` is not the current Service or whose `lease_expires_at` is in the past back to `pending`, for both `pending_operations` and `action_job_ops`. Boot recovery unlinks `<app_data>/send_vault/<job_id>/` directories whose job is missing or terminal.
- `db`: `action_jobs` + `action_job_ops` schema migration applies cleanly; FK to `accounts(id)` cascades on account delete; CHECK constraints on `status`, `kind`, `quiet` reject bad values; UNIQUE(`job_id`, `ordinal`); the `(job_id, ordinal) WHERE status='pending'` partial index covers the lease query (EXPLAIN check).
- `app`: `dispatch_plan` bumps both `nav_generation` and `thread_generation` BEFORE the IPC call. `in_flight_plans` correlation: an `OperationOutcome` for an unknown `plan_id` is logged at debug and dropped. `client_throttle` debounces a fast double-click. `applied_outcomes` dedupes a duplicate `OperationOutcome`. Tri-state rollback: `ServiceCrashed` while `Pending` -> `AckUnknown`; `ServiceCrashed` while `Acked` -> stay `Acked`. Reconciliation via `action.job_status` after `boot.ready` resolves `AckUnknown` to `Acked` (Journaled) or rollback (NotFound).
- Compile-check: an `app/src/...` source file that tries `use service_state::WriteDbState` fails to compile. Add a UI-side test that imports `service_state` and asserts on the missing-crate-dependency build error (or document that the regression test is "the workspace doesn't build" if a cargo dependency cycle is added by accident).

### Integration tests (in-process)

- `tests/dispatch_in_process.rs::execute_plan_acks_immediately_then_streams_outcomes` - submit a 3-operation plan; assert `ActionPlanAck { journaled: true }` returns within 5 ms (handler is just enqueue + signal); assert 3 `OperationOutcome` notifications arrive in order from the worker; assert `action.completed` arrives last.
- `tests/dispatch_in_process.rs::action_completed_after_commit_visibility` - submit a plan; immediately on `action.completed` arrival, open a fresh read connection against `ReadDbState`; assert the read sees committed state. Does not depend on fsync; uses `PRAGMA synchronous = NORMAL`.
- `tests/dispatch_in_process.rs::operation_outcome_carries_generation_tag` - drive the dispatch; verify the notification's `service_generation` matches the active reader's captured generation.
- `tests/dispatch_in_process.rs::stale_outcomes_dropped_after_respawn` - hand-craft an `OperationOutcome` with a stale generation; verify it never reaches the completion-effects handler (Phase 1.5's generation gate must apply to the new variants too).
- `tests/dispatch_in_process.rs::journal_replays_after_respawn` - submit plan, ack arrives, kill Service after one `OperationOutcome` lands but before the rest; respawn; assert the new Service drains `action_plan_ops` (skipping the already-`done` op) and emits the remaining outcomes; assert the UI's `applied_outcomes` dedupes any re-emitted op.
- `tests/dispatch_in_process.rs::pre_ack_crash_rolls_back` - submit plan, kill Service before `ActionPlanAck` arrives; assert `ClientError::ServiceCrashed`; assert UI rolled back the optimistic state; assert the journal has no row for that plan_id.
- `tests/dispatch_in_process.rs::post_ack_crash_does_not_roll_back` - submit plan, await ack, kill Service, respawn; assert UI did NOT roll back optimistic state; assert outcomes eventually arrive from the journal-driven replay.
- `tests/dispatch_in_process.rs::handler_does_not_drive_batch_execute` - submit a plan whose execution would block (synthetic worker barrier); assert the dispatch loop continues to answer `health.ping` while the plan is queued (proves the handler returned without holding its in-flight permit).
- `tests/dispatch_in_process.rs::send_wire_attachment_validation` - submit a `SendWireRequest` with a `StagingFile` whose `content_hash` doesn't match the bytes; assert `OperationOutcome::RemoteFailure { retryable: false }`.
- `tests/dispatch_in_process.rs::send_wire_oversize_payload_handler_path` - submit a `SendWireRequest` whose JSON envelope (excluding bytes, which aren't on the wire) stays under 4 MiB even for a 50 MB attachment; assert framing accepts it. Negative test: a malformed handler that tries to embed bytes is rejected at the wire-type compile check (no `Vec<u8>` field in `SendWireRequest`).
- `tests/dispatch_in_process.rs::mark_chat_read_emits_only_action_completed` - submit `action.mark_chat_read`; assert the worker emits one `action.completed` and zero `OperationOutcome` notifications (quiet plan).
- `tests/dispatch_in_process.rs::action_skips_search_index_write` - delete a thread; assert `MailOperation::Trash` ran; assert the Tantivy index still has the doc (Phase 2 temporary inconsistency); document this as expected behavior cleaned up by Phase 3's invariant pass.

### Real-subprocess smoke tests

- `crates/app/tests/service_subprocess.rs::star_toggle_round_trips_through_ipc` - spawn against a seeded data dir; submit a star-toggle plan; assert the thread row updates within 1 s; observe `OperationOutcome` + `ActionCompleted` on the wire.
- `crates/app/tests/service_subprocess.rs::bulk_archive_200_threads_under_budget` - submit a 200-operation plan; assert all outcomes arrive within 5 s.
- `crates/app/tests/service_subprocess.rs::pre_ack_crash_rolls_back_subprocess` - submit plan, SIGKILL the subprocess before ack; respawn; assert UI rolled back optimistic state.
- `crates/app/tests/service_subprocess.rs::post_ack_crash_replays_subprocess` - submit plan, await ack, SIGKILL after one outcome lands; respawn; assert the journal replay completes the remaining ops AND the UI did not roll back.
- `crates/app/tests/service_subprocess.rs::compose_send_50mb_attachment` - construct a `SendWireRequest` with a 50 MB staging-file attachment; verify the JSON envelope on the wire is small (< 100 KB); verify the SMTP submit succeeds Service-side; verify staging file is unlinked on `SendCompleted`.
- `crates/app/tests/service_subprocess.rs::undo_round_trips_as_compensating_plan` - dispatch action, undo, assert the inverse plan runs and outcomes match.
- `crates/app/tests/service_subprocess.rs::test_fake_schema_propagates_via_terminal` - per Phase 1.5 carry-forward 19c.

### Manual matrix updates

- Real-keyboard star-toggle latency sanity on a 50 GB seeded mailbox - checks "does it *feel* fast" through the wire round-trip. (Replaces the dropped CI latency benchmark.)
- Bulk-archive of 200 threads while the UI is mid-scroll. Verify rendering stays responsive throughout (Service is doing the work; the UI thread should be idle).
- Compose with a 50 MB attachment. SMTP submit takes 30+ s; verify the UI's send button stays in "sending" state with progress notifications, then transitions cleanly on `SendCompleted`.
- Service crash mid-action (`kill <service-pid>` while a bulk-archive is in flight). Verify optimistic rollback fires, the respawn happens within Phase 1.5's bounds, and the pending operations eventually drain.

## Open questions

Resolve in implementation:

1. **Encryption-key handle vs trusted-bytes-once.** Default proposal: handle-based (`internal.encrypt_for_storage` IPC method). Confirm during task 19 once the credential-persistence call sites are surveyed; if the IPC overhead is unacceptable on the hot credential paths, fall back to a one-shot `internal.export_key` call that the UI persists in-memory only.

2. **OperationOutcome ack notifications vs idempotent application.** Settled: idempotent application (UI handler is no-op on a duplicate `(plan_id, operation_id)` via `applied_outcomes`). Confirm during task 10 (`dispatch_plan` rewrite) that re-applying every `OperationOutcome` variant truly is idempotent; if any path has a non-idempotent side effect (toast spam, audio cue, etc.), revisit.

3. **Service-side periodic vs UI-driven kick for pending_ops.** Default proposal: UI-driven kick on `Message::SyncTick` (preserves existing tick policy that gates on focus/online state). Confirm during task 18 - if the UI-driven kick produces noticeable retry-latency drift, add a Service-side fallback periodic with a long interval (5 minutes) as a safety net.

4. **Where does `ActionExecutionPlan` -> `ActionWirePlan` conversion live?** Two options: (a) `From` impl on `ActionWirePlan` in `service-api` (no UI metadata leakage; clean dependency direction) or (b) `to_wire_plan(&self) -> ActionWirePlan` method on `ActionExecutionPlan` in core. Default proposal: (a) - service-api owns the wire shape and its conversion.

5. **Send variant on the action-plan journal.** `MailOperation::Send` does not exist today - send is a separate `SendRequest` shape (`crates/core/src/send.rs`). Phase 2 needs the journal to carry sends so the durable-ack contract holds for compose-send (item 13). Two options: (a) add `MailOperation::Send { send_id }` and let the action-plan journal carry sends generically; (b) keep send as a sibling enum that the journal serializes to a different blob column. Default proposal: (a) - one journal substrate, one worker loop, one set of replay semantics. Decide during task 13.

6. **Per-account concurrency in batch_execute Service-side.** Today's batch_execute groups by account and dispatches per-account in parallel (limit 4). Service-side, the natural place is the worker. Decision: keep the limit at 4 unchanged in Phase 2; revisit if manual-matrix latency runs reveal contention.

7. **In-flight dedup set sharing.** The Service holds `ActionContext::in_flight: Arc<Mutex<HashSet<String>>>`. UI's client throttle is a separate map. Both layers serve - the question is whether the UI throttle entries should expire on `OperationOutcome` arrival vs `ActionCompleted` arrival vs a fixed timeout. Default: expire on `ActionCompleted`. Confirm during task 12 against the click-pattern manual test.

## Verification (end-to-end)

1. Fresh data dir + seeded DB. Star-toggle on 50 threads from the UI. Latency feels instant; benchmark agrees.
2. Bulk-archive of 200 threads. UI stays responsive; Service is at 100% CPU on its own thread; outcomes stream in. `ActionCompleted` arrives within budget.
3. Kill the Service mid-bulk-archive (after ack, mid-execution). UI does NOT roll back optimistic state; UI logs the respawn; respawned Service drains the action-plan journal. The 200 outcomes eventually all land; `applied_outcomes` dedupes any re-emitted ops.
4. Kill the Service before any plan is acked (race against the spawn). UI sees `ClientError::ServiceCrashed` for the in-flight dispatch; UI rolls back optimistic state; nothing in the journal for that plan.
5. Compose with a 50 MB attachment. Send. UI writes attachment bytes to `<app_data>/staging/<send_id>/`, issues `client.send(SendWireRequest)` (small JSON envelope on the wire). UI shows "sending" with progress notifications during SMTP upload; transitions to "sent" on `SendCompleted`. Staging directory unlinks.
6. Trigger an action that the provider rejects (e.g. archive a thread that the server already deleted). `OperationOutcome::RemoteFailure { retryable: false }` arrives; optimistic update rolls back; toast surfaces the failure.
7. Two fast star-toggle clicks (within the 200 ms window). Only one IPC request fires; client throttle absorbs the second.
8. Snooze a thread to resurface in 30 s. Wait. Verify the resurface fires Service-side; the thread re-enters Inbox; the UI receives `OperationOutcome` for the resurface op.
9. Open a chat thread (mark-chat-read trigger). Verify a single `action.completed` arrives with no per-operation outcomes; verify the chat's read state in the DB matches.
10. Delete a thread. Verify the thread is gone from the UI; verify the Tantivy entry SURVIVES (Phase 2 temporary inconsistency, will be cleaned up by Phase 3 invariant pass). Document this in the verification log so reviewers don't flag it as a bug.
11. `brokkr check` clean.
12. `cargo test -p app` includes the ack-state-scoped rollback tests and the journal-replay tests; all pass.
13. Compile-time check: a sed-driven test that injects `use service_state::WriteDbState` into a UI source file and asserts the workspace fails to build.

## Promotion criteria

This phase is done when:

- All items in `In scope` (excluding the explicitly-deferred carry-forwards 19a / 19c / 19d / 19g, which now have homes in Phase 6a / Phase 8) are implemented and wired - actions execute Service-side, the UI no longer constructs `ActionContext`, all five non-`MailActionIntent` paths (undo, send, snooze resurfacing, draft delete, mark-chat-read) flow through IPC.
- The action-job journal (`action_jobs` + `action_job_ops`, sibling-job model) ships with its schema migration, lease-based recovery path, and orphan-vault cleanup.
- The `action.execute_plan` handler returns `ActionPlanAck` without driving `batch_execute` itself; the worker is a separate task.
- `WireMailOperation` mirrors `core::actions::MailOperation` 1:1; static-assertion test enforces the mirror.
- `action.send` accepts attachments only via refs (`StagingFile` / `PackStore`); raw bytes do not appear on the wire.
- `action.mark_chat_read` exists and the UI's `chat.rs` handlers no longer write to the DB directly.
- Action paths no longer write to the Tantivy index (Phase 2 temporary inconsistency policy is the agreed surface).
- All `Exit criteria` from the implementation-roadmap.md Phase 2 section are satisfied.
- The `service-state` crate boundary holds: a UI source file that tries `use service_state::WriteDbState` fails to build.
- The `process-lifetime` crate exists; `service` and `app` both depend on it; the App -> Service dependency for `parent_death` is gone.
- Of Phase 1.5's three Phase-2 carry-forwards: `BootContext.db_conn` consumed (PARTIAL - action worker reads it; UI two-connection waste collapses in Phase 6); `from_boot_ready` heavy init and `SchemaVersionChanged` e2e test deferred to Phase 8.
- Of Phase 1.5's five Phase-2 carry-forwards from commit 30: pre-dispatch generation bumps + `parent_death` boundary + `BootSharedState` flood resilience all CLOSED in Phase 2; `from_boot_ready` key reload deferred to Phase 6a; `boot_progress::emit` class-aware helper deferred to Phase 8.
- Reviewer signoff on this plan + the delivered code.

The next phase (Phase 3 - JMAP sync relocation, Tantivy/body/inline writer relocation) gets its own equivalent plan document at the time it's tackled. Phase 3 lights up the `SyncProviderCtx` shape Phase 2 scaffolds, locks down the body/inline/search write halves behind the same `service-state` crate boundary, introduces the minimal cross-store invariant pass (which also cleans up Phase 2's deliberate Tantivy orphans), and adds the `SearchState` read/write split.
