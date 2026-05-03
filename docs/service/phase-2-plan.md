# The Service - Phase 2 Plan: Action service relocation + read/write state-type split

Companion to `phase-1-plan.md` and `phase-1.5-plan.md`. Implements Phase 2 of `implementation-roadmap.md`, including the Phase 1.5 carry-forward bullets named in that section.

## Context

Phase 1.5 made the Service load-bearing for boot but the Service still does no real work post-handshake. Phase 2 moves the first actual workload across the boundary: action *execution*. The split is deliberate and surgical:

- **UI keeps**: action *resolution* and *planning* (`MailActionIntent` -> `resolve_intent` -> `build_execution_plan`). These read selection state, sidebar scope, and `completion_behavior()` policy - all UI-owned. They produce a fully-resolved `MailOperation` list.
- **Service gets**: `batch_execute(plan)` and the four entanglements that come with it - the `pending_ops` retry queue's periodic drainer, the encryption-key consumer, the in-flight dedup set, and the action-DB writer half.
- **UI keeps**: completion-effects (toast, auto-advance, undo eligibility, optimistic rollback) - driven by per-operation `OperationOutcome` notifications and a final `action.completed`.

Phase 2 also lands the first half of the **type-level read/write split** the problem statement names as the global invariant. `WriteDbState` is unreachable from `app` via the Cargo dependency graph (a new `service-state` crate that `app` does not depend on). The body/inline/search write halves do not lock down until Phase 3 (sync moves) and Phase 6 (the rest), so Phase 2's claim is scoped: *the relocated call sites can no longer construct `WriteDbState`*. The global "no UI write surface compiles" claim arrives at Phase 6, not here.

The phase ships as a single milestone but with a clean commit-level split (per the same shape as Phase 1.5): wire types -> `service-state` crate -> ProviderCtx split -> action service relocation -> respawn-aware optimistic rollback -> pending-ops drainer -> Phase 1.5 carry-forwards. A bisect on a regression should land on the right commit.

This is the second-largest UI-side surgery in the project (Phase 1.5 was the largest). The dispatch path that today is `action_ctx.action_ctx().expect(...) -> Task::perform(batch_execute(...))` becomes a multi-step IPC flow whose latency budget is in milliseconds, not microseconds. The hot path is design-bounded - star-toggle p99 of 5-15 ms is the explicit target, and a benchmark lands in this phase so regressions are observable before the next phase ships.

## Scope

### In scope

1. **`service-state` crate.** New `crates/service-state/` houses the `WriteDbState` type. `app` does not list it as a dependency; `service` does. Compile-time enforcement of "no UI call site constructs `WriteDbState`" follows from the dependency graph - a UI handler that tries to `use service_state::WriteDbState` fails to resolve. `db::DbState` -> `db::ReadDbState` (UI) + `service_state::WriteDbState` (Service); both wrap the same internal `Arc<Mutex<Connection>>` pool but only the write half exposes write methods. `DbState::conn()` and `from_arc()` (`crates/db/src/db/mod.rs`'s raw-`Connection` escape hatches) collapse: `ReadDbState::conn()` stays for read paths; `WriteDbState::conn()` is `pub(crate)` to the `service-state` crate and re-exported only via `WriteDbState`'s narrow API. Body/inline/search state types **do not split in Phase 2** - their writers stay UI-side until Phase 3 (sync) and Phase 6 (everything else). The `Clone + Arc<Mutex<_>>` settled pattern (`docs/architecture.md` § "Settled Patterns") survives the split.

2. **`ActionWirePlan` wire type.** Today's `ActionExecutionPlan` carries UI completion metadata (auto-advance hints, toast text, completion-behavior policy) that does not belong on the wire. Define `ActionWirePlan` in `service-api`:
   ```rust
   pub struct ActionWirePlan {
       pub plan_id: PlanId,
       pub operations: Vec<ActionWireOperation>,
   }
   pub struct ActionWireOperation {
       pub operation_id: OperationId,
       pub account_id: String,
       pub thread_id: String,
       pub operation: MailOperation,
   }
   ```
   `MailOperation` (`crates/core/src/actions/operation.rs`) is already a clean enum of typed-ID-bearing variants and serializes with no shape change. `PlanId` and `OperationId` are wire newtypes (`u64`) generated UI-side - the UI needs to correlate `OperationOutcome` notifications back to the originating intent's UI metadata, and Service-generated ids would round-trip a request before the correlation map could hold the entry. UI completion metadata stays UI-side in an `in_flight_plans: HashMap<PlanId, PlanCompletionMeta>` keyed by `plan_id`.

3. **`action.execute_plan` IPC method.** The single entry point for action execution. Returns `ActionPlanAck { plan_id }` synchronously when the Service has accepted and persisted the plan to `pending_ops`; per-operation outcomes stream as `OperationOutcome` notifications; final `action.completed` closes the stream. Timeout: 60 s (bulk operations against slow remotes). Bypasses neither the per-handler semaphore nor the admission cap - actions are bounded work.

4. **`OperationOutcome` and `action.completed` notifications.** Both `MustDeliver` (the cross-respawn drop guarantee from Phase 1.5's generation tag preempts stale outcomes from a dying incarnation hitting the new one's UI). Both implement `WithGeneration` per the contract added in Phase 1.5 commit 30, so the catalog test grows automatically. Per-operation outcomes carry `{ plan_id, operation_id, result: OperationResult }`; the final `action.completed { plan_id, summary }` only fires after WAL fsync of the last per-operation transaction (the read-after-write contract documented on the IPC method, not just an implementation detail).

5. **`action.send` IPC method.** Compose-send relocates. SMTP submit + DB updates via the action context happen Service-side. Long IPC timeout (60 s today; the timeout-table entry already accounts for 50 MB attachments). Returns `SendAck { send_id }` synchronously, with progress notifications during attachment upload and a final `action.completed`-shaped `SendCompleted` notification. The current UI-side `crates/app/src/handlers/pop_out/compose_send.rs::dispatch_send` becomes an IPC call.

6. **`action.undo` IPC method.** Undo is structurally a compensating-action plan dispatched via the same path; the UI builds the reverse `ActionWirePlan` from the stashed `MailUndoPayload` (existing in `action_resolve.rs`) and submits it. No new wire shape - reuses `action.execute_plan`. Phase 2 closes the loop by ensuring the stashed payload survives a Service respawn (the in_flight_plans map is UI-side and persistent across respawn; payloads are re-issuable).

7. **`action.snooze_resurface_due` IPC method.** The snooze resurfacing tick (`handlers/commands.rs` `SyncTick` path) becomes a Service-internal periodic task. Phase 2 transition: UI fires a `pending_ops.kick`-style notification on `SyncTick`; the Service's snooze-runner inspects the DB, dispatches resurface operations as a normal `action.execute_plan`. The `SyncTick` policy stays UI-side (depends on focus, online state, etc.) - the trigger is UI-driven, the work is Service-side.

8. **Pending-ops periodic drainer relocates.** The `process_pending_ops` periodic (`crates/core/src/actions/pending.rs`) moves Service-side. UI sends `pending_ops.kick` notification on `Message::SyncTick`. Default proposal: UI-driven kick (preserves the existing tick policy that gates on focus + online state) over a Service-side periodic. The Service-side drainer reuses the boot-time `recover_on_boot_db_only` path Phase 1.5 already extracted.

9. **`ProviderCtx` shape adjustment.** Today's `ProviderCtx { account_id, db, body_store, inline_images, search, progress }` (per `crates/common/src/types.rs`) is one struct passed to every `ProviderOps` method, including action methods that only need `db` + the encryption key. Split:
   - `ActionProviderCtx<'a> { account_id: &'a str, db_write: &'a WriteDbState, key: &'a SecretKey, progress: &'a dyn ProgressReporter }` - what `archive`, `trash`, `mark_read`, `star`, `move_to_folder`, `add_tag`, `remove_tag`, etc. need.
   - `SyncProviderCtx<'a> { account_id, db_write, body_write, inline_write, search_write, progress }` - the eventual Phase 3 shape. Scaffolded in Phase 2 (the type exists, with `body_write`/`inline_write`/`search_write` typed as `()` placeholder slots) so the trait signatures Phase 3 introduces are already wire-compatible.

   This touches every `ProviderOps` method in `crates/common/src/ops.rs` and every implementation in the four provider crates - mechanical but broad. The mechanical work lands as one focused commit so the action-relocation commits don't drown in `git blame`.

10. **`ProgressReporter` trait.** Already `Send + Sync` and serializable (`crates/db/src/progress.rs:32` - `emit_json(event_name: &str, json: serde_json::Value)`). Service-side `IpcProgressReporter` posts to the notification queue with `Coalesce { key: ProgressEvent(account_id) }` per emission. UI's `IcedProgressReporter` keeps consuming the same shape - no trait redesign needed. The "trait method signatures must become serializable" risk the roadmap flagged was based on the older `ProgressReporter` shape; the current trait is already serializable so this collapses to "construct one new impl".

11. **`pending_ops.kick` notification (UI -> Service).** Wire shape: zero-payload notification (`Drop`-class - if the Service is busy, the next tick will kick again). Triggers Service-internal `process_pending_ops` immediately rather than waiting for the next periodic.

12. **`ActionContext` reconstruction Service-side.** `crates/core/src/actions/context.rs::ActionContext` becomes Service-internal. Constructed once at Phase 2 boot from the `BootContext` (Phase 1.5 already holds `db_conn` + `encryption_key` + `recovery_warnings` waiting for Phase 2 to consume them - this resolves the `apply_standard_pragmas per-connection waste / BootContext.db_conn unused` carry-forward from the roadmap). The `in_flight: Arc<Mutex<HashSet<String>>>` dedup set lives Service-side; the UI also gets a client-side throttle in `service_client.rs` keyed by `(account_id, thread_id)` to avoid issuing two IPC roundtrips on a fast double-click. Both layers serve - the UI throttle reduces IPC pressure; the Service set is the canonical correctness gate.

13. **Generation counters bump pre-dispatch.** UI bumps `nav_generation` and `thread_generation` *before* sending the plan over IPC - not after `action.completed` arrives. The IPC delay creates a window where stale `ThreadsLoaded` / `NavigationLoaded` results can otherwise land between dispatch and ack and overwrite optimistic UI updates. This applies to `dispatch_plan` (the canonical path), the compose-send path, the undo path, and the snooze-resurfacing-tick path. Per `docs/architecture.md` § "Generation counters for async safety", these are `GenerationCounter<T>` instances; the bumps use `let _ = counter.next()` for invalidation-only side effects.

14. **Optimistic UI rollback path.** Today's optimistic updates roll back from the same call stack that issued the action. Phase 2 introduces two trigger paths for rollback:
    - `OperationOutcome` with `result: Failure(_)`: the per-operation rollback fires when the failure notification arrives.
    - `ClientError::ServiceCrashed`: when the Service dies mid-action, **every** plan in `in_flight_plans` rolls back. The respawned Service has no knowledge of the dying plan; the UI-side completion handler is the authoritative source for "did this happen?". Without this, an optimistic update for an action that crashed the Service stays permanent until the next sync.
    - `ClientError::SchemaVersionChanged` / `BootFailureReason::*` (terminal failures): these emit `Terminal` and `iced::exit()`s - no rollback needed because the process is exiting.

15. **`action.completed` after WAL fsync.** The Service emits `action.completed` only after the per-plan transaction has been WAL-fsync'd. This is documented as an IPC contract on the method, not just behavior - Phase 2 + tests pin it. Without the contract, the UI's natural pattern of "got `action.completed`, refresh thread list now" can race the disk and return pre-commit data on a slow disk.

16. **Action latency benchmark.** New `crates/app/benches/action_latency.rs` (or `crates/service/tests/action_latency_smoke.rs` if Criterion is overkill). Targets: star-toggle submit-to-`action.completed` p99 < 16 ms under healthy local Service load; bulk-archive-of-200-threads measured separately. Runs in CI so a regression in any phase 2-7 commit is visible. Excludes the IPC encode/decode round-trip when run in-process; includes it when run as a real-subprocess test.

17. **Per-operation idempotency contract.** The `pending_ops` retry queue can re-issue an operation after a crash. `MailOperation` variants must be idempotent at the wire level - re-archiving an already-archived thread is a no-op, not an error. Already true today (the action-service's local DB mutations check current state before applying); Phase 2 lifts the implicit contract into a doc-comment on `MailOperation` and on `RequestParams::ActionExecutePlan`.

18. **Error-shape decisions.** `ActionError` variants need a per-variant "preserve across the boundary" decision. Default proposal: collapse provider-specific errors into `RemoteFailure { provider_message: String, http_status: Option<u16>, retryable: bool }`; preserve action-pipeline errors verbatim (`ThreadNotFound`, `AccountUnknown`, `OperationConflict { … }`). The retryable flag drives whether `pending_ops` re-enqueues. Locked into `service-api` so adding a new variant requires a wire decision.

19. **Phase 1.5 carry-forwards close out**. Each items lands as part of Phase 2 and is named so the roadmap's carry-forward bullets can be deleted. See "Phase 1.5 carry-forward closeout" subsection at the bottom of `In scope`.

### In scope (Phase 1.5 carry-forward closeout)

These bullets come from `implementation-roadmap.md` Phase 2 § "Phase 1.5 carry-forward (close out as part of Phase 2)". Each is in scope for Phase 2 implementation.

19a. **`ReadyApp::from_boot_ready` heavy synchronous init.** Today (Phase 1.5) `crates/app/src/app.rs::from_boot_ready` opens the DB, loads stores, parses bootstrap snapshots, restores pop-out windows synchronously. Phase 2 reworks `ActionContext` Service-side - which removes the ActionContext-side init from the critical path - then relocates the body/inline/search store init to async tasks dispatched from a `BootingApp::update` arm. The splash stays responsive while these finish.

19b. **`apply_standard_pragmas` per-connection waste / `BootContext.db_conn` consumed.** Phase 1.5 holds an idle `Connection` in `BootContext` waiting for Phase 2's `ActionContext` to consume. Phase 2 picks it up at the action-service-boot step (item 12 in `In scope`) so the connection is not duplicated and not leaked. If Phase 2 consumes the connection cleanly, the carry-forward is satisfied; if not, the fallback is to drop the field from `BootContext` and have Phase 2 reopen.

19c. **`SchemaVersionChanged` end-to-end test (`--test-fake-schema=N`).** Add a `--test-fake-schema=N` test-helper flag analogous to the existing `--test-fake-version`. Real-subprocess test flips the value across SIGKILL and asserts `ClientError::SchemaVersionChanged` arrives via Terminal. Phase 2 introduces real schema-version sensitivity (the action service depends on the schema being what the UI thinks it is), so the test naturally lands here.

19d. **`from_boot_ready` re-loads encryption key.** The hard requirement from Phase 1.5 commit 30. Phase 2 plumbs `BootContext::encryption_key` through the IPC boundary so the UI consumes the Service's already-validated key instead of re-reading the key file. Specifically: an internal `internal.crypto_handle` IPC method (or a new boot.ready response field) returns a Service-managed handle the UI can use to encrypt/decrypt without holding the raw bytes. **Decision in implementation**: handle-based access (Service holds bytes; UI calls `internal.encrypt { plaintext } -> ciphertext`) vs. trusted-bytes-once (Service hands the bytes once on a one-shot IPC method, UI keeps in memory). Default proposal: handle-based - extra IPC round-trips per encrypt are tolerable for credentials and are not a hot path; the security benefit (Service is the only process holding raw key bytes) is real.

19e. **Pre-dispatch generation-counter bumps.** Same as `In scope` item 13. Phase 1.5 noted the gap; Phase 2 closes it as part of the `dispatch_plan` rewrite.

19f. **`parent_death` crate boundary.** Extract `parent_death` from `service` into a new `process-lifetime` micro-crate. Both `service` and `app` depend on it. The dependency graph stops looking inverted (today `app -> service`). Land as part of the `service-state` crate split so the workspace surgery happens in one commit.

19g. **`boot_progress::emit` per-phase regression test.** Today the contract ("`OUTBOUND_QUEUE_CAP=1024` must remain >> Phase-1.5 boot frame count") is doc-only. Phase 2 introduces several new `MustDeliver` notifications (`OperationOutcome`, `action.completed`, send progress, etc.) - each new emitter must ship with a regression test bounding total emit count for its phase, OR the helper must become class-aware (`try_send` for `Coalesce`/`Drop`, `send().await` for `MustDeliver` per the established backpressure model). Default proposal: class-aware helper. The contract becomes "use the helper, contract enforcement is structural."

19h. **`BootSharedState` flood resilience for `boot.ready`.** The `boot.ready` handler parks each request on the shared `Notify` and consumes a `JoinSet` slot until boot completes. Phase 2 touches the boot.ready handler shape (adds the encryption-key-handle response field per 19d), so this is the right moment to add an `AtomicBool` "already in flight" so subsequent callers either join the existing waiter or fail fast.

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
    pub operation: MailOperation,
}

pub struct ActionPlanAck {
    pub plan_id: PlanId,
    pub persisted: bool,
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

```
1. UI: build ActionWirePlan from ActionExecutionPlan.
        Stash UI metadata in in_flight_plans[plan_id] = PlanCompletionMeta { ... }
2. UI: bump nav_generation, thread_generation (pre-dispatch invalidation).
3. UI: apply optimistic updates (existing logic).
4. UI: client_throttle.try_acquire((account_id, thread_id))  [client-side dedup]
5. UI: service_client.execute_plan(plan).await -> ActionPlanAck
        Service has persisted the plan to pending_ops. UI now knows the
        Service will retry on its own if the connection drops.
6. Service: handler dispatches to batch_execute(plan).
   Service: per-operation execution emits OperationOutcome notifications.
   Service: final action.completed notification AFTER WAL fsync of last txn.
7. UI: each OperationOutcome arrives, dispatches via update.rs ::
        Message::OperationOutcomeReceived(...) ->
        completion-effects per the stashed metadata
        (toast, undo eligibility, optimistic rollback on Failure).
8. UI: ActionCompleted arrives ->
        Message::ActionCompletedReceived(plan_id) ->
        remove in_flight_plans[plan_id]; release client_throttle.
```

Crash handling at each step:
- (5) crash: `ClientError::ServiceCrashed`. Roll back optimistic updates for the plan. Pending_ops persistence has not happened; the action is lost.
- Between (5) ack and (6) start: the Service has persisted; pending_ops will retry on respawn. UI sees the post-respawn outcomes via the same notification path.
- During (6) execution: per-operation outcomes that landed before the crash are dispatched; pending operations get retried by the new Service. UI's `in_flight_plans` correlates by `plan_id` across respawn so the eventual outcomes apply correctly.

The post-respawn handler must replay any `OperationOutcome`s the Service marks as "previously emitted but not yet acked". Phase 2's wire shape includes a per-`OperationOutcome` `acked: bool` flag (UI sends `action.outcome_acked { plan_id, operation_id }` after applying); the Service replays unacked ones on respawn. **Decision in implementation**: ack notifications add chatter; alternative is "UI is responsible for idempotent application" (re-applying an OperationOutcome is a no-op because the optimistic update is already settled). Default proposal: idempotent application, no acks. Cleaner wire, identical UX.

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

### Action latency benchmark

`crates/service/tests/action_latency_smoke.rs` (in-process):

```rust
#[tokio::test(flavor = "multi_thread")]
async fn star_toggle_p99_under_local_service() {
    let (client, _server) = spawn_in_process_test_harness().await;
    let mut samples = Vec::with_capacity(200);
    for _ in 0..200 {
        let start = Instant::now();
        let plan = build_star_toggle_plan(thread_id);
        let _ack = client.execute_plan(plan).await.expect("dispatch");
        wait_for_action_completed(&mut subscription).await;
        samples.push(start.elapsed());
    }
    let p99 = percentile(&samples, 99.0);
    assert!(
        p99 < Duration::from_millis(16),
        "star-toggle p99 {p99:?} exceeded 16 ms budget",
    );
}
```

The 16 ms target is one frame at 60 fps - star-toggle that takes longer is user-visible jitter. Bulk-archive of 200 threads gets a separate benchmark with a more generous target (1.5 s for the full plan including per-operation wire round-trips). Both run in CI; regressions in any commit between Phase 2 and Phase 7 are visible before the next phase ships.

## Detailed task list

In recommended commit order. Each item is one focused commit unless noted.

1. **`process-lifetime` crate.** Mechanical move of `crates/service/src/parent_death/` -> `crates/process-lifetime/`. Update `service` and `app` Cargo.toml dependencies. No behavior change. Zero risk if the move is correct; integration tests catch any miswire.

2. **`service-state` crate (scaffold).** New crate with `WriteDbState` type that is identical-in-shape to today's `DbState`. App still uses `DbState`; nothing locks down yet. This is the type-level scaffold so the next commit can lift call sites.

3. **`db::DbState` -> `db::ReadDbState` + private `db::ConnectionPool`.** `DbState` becomes `ReadDbState` (rename). The internal `Arc<Mutex<Connection>>` pool extracts to a private `ConnectionPool` type. `ReadDbState` exposes `conn()` for read paths only; write methods on the old `DbState` API split out and become `pub(crate)` to a new `db::write_helpers` module that `service-state::WriteDbState` consumes. This commit is the type-rename surgery; cargo check fails at every UI write call site, which the next two commits fix.

4. **`service-api`: action wire types.** `PlanId`, `OperationId`, `ActionWirePlan`, `ActionWireOperation`, `ActionPlanAck`, `OperationResult`, `OperationOutcome`, `ActionCompleted`, `PlanSummary`, `RemoteFailure`. Type-only commit; serde round-trip tests; `RequestParams::ActionExecutePlan.timeout() == 60s` test; `WithGeneration` impl on `OperationOutcome` and `ActionCompleted`; `Notification::class()` arms return `MustDeliver`; `production_notification_catalog()` extended.

5. **`service`: `IpcProgressReporter`.** New `crates/service/src/progress.rs::IpcProgressReporter` impl of `db::ProgressReporter` that posts `Notification::SyncProgress(...)` via the existing `boot_progress::emit_classified` helper (extended to take a `Notification` directly).

6. **`service`: relocate action service.** Move `crates/core/src/actions/` -> `crates/service/src/actions/` keeping the *resolution* and *planning* types in core. The `MailActionIntent` / `resolve_intent` / `build_execution_plan` / `MailOperation` / `MailUndoPayload` / `CompletionBehavior` / `ToggleField` surface stays in `crates/core/src/actions_resolve/` (or stays at `crates/core/src/actions/resolve/`, design choice). The execution surface (`batch.rs`, `context.rs`, per-action files, `pending.rs`) moves to `crates/service/src/actions/`. `ActionContext` becomes Service-internal and consumes `BootContext` from Phase 1.5. `batch_execute` signature change: `&ActionContext` stays; the surrounding wiring moves.

7. **`common`: `ProviderCtx` split.** `ProviderCtx<'a>` -> `ActionProviderCtx<'a> { account_id, db_write, key, progress }` + `SyncProviderCtx<'a> { account_id, db_write, body_write: (), inline_write: (), search_write: (), progress }`. `body_write`/`inline_write`/`search_write` are typed as `()` in Phase 2 (Phase 3 fills them when sync moves). Action methods on `ProviderOps` change to take `ActionProviderCtx`; sync methods take `SyncProviderCtx`. Mechanical edits to every `ProviderOps` impl in the four provider crates + every action method body.

8. **`service`: `action.execute_plan` handler.** New `crates/service/src/handlers/action.rs::handle_execute_plan(ack)`. Persists plan to `pending_ops`, returns `ActionPlanAck { plan_id, persisted: true }`, then drives `batch_execute` with per-operation `OperationOutcome` emission. Final `action.completed` after WAL fsync. Bypasses neither the per-handler semaphore nor the admission cap.

9. **`app`: dispatch_plan rewrite.** `crates/app/src/handlers/commands.rs::dispatch_plan` becomes `client.execute_plan(wire_plan).await`. Pre-dispatch generation bumps (`nav_generation.next()`, `thread_generation.next()`). Optimistic updates stay UI-side. UI metadata stash: `in_flight_plans: HashMap<PlanId, PlanCompletionMeta>` on `ReadyApp`. New messages: `ActionDispatched(ActionPlanAck)`, `OperationOutcomeReceived(OperationOutcome)`, `ActionCompletedReceived(ActionCompleted)`. Removes the direct `Task::perform(batch_execute(...))` call.

10. **`app`: respawn-aware optimistic rollback.** When `ClientError::ServiceCrashed` arrives for any in-flight plan dispatch, OR the App receives a `ServiceNotification(ServiceClient::ServiceCrashed)`-shaped event, iterate `in_flight_plans` and roll back every entry's optimistic state. Test: kill Service mid-dispatch, verify state matches pre-optimistic-update.

11. **`app`: client-side action throttle.** New `client_throttle: HashMap<(AccountId, ThreadId), Instant>` on `ReadyApp` (or inside `service_client.rs`). 200 ms debounce. Prevents two IPC roundtrips on a fast double-click; complements the Service's `ActionContext::in_flight` set.

12. **`service`: `action.send` handler.** Compose-send relocates. `crates/service/src/handlers/action.rs::handle_send(SendRequest)`. UI: `crates/app/src/handlers/pop_out/compose_send.rs::dispatch_send` becomes `client.send(request).await`.

13. **`service`: `action.undo` flow.** UI builds the reverse `ActionWirePlan` from the stashed `MailUndoPayload`; submits via `action.execute_plan`. No new wire shape. Test: dispatch action, observe completion, undo, observe inverse outcome.

14. **`service`: snooze resurfacing relocation.** `crates/service/src/snooze_runner.rs` triggered by UI `pending_ops.kick` notifications. Service queries snooze table, dispatches resurface operations as a normal `action.execute_plan`. UI's `SyncTick` path becomes a `client.kick_pending_ops()` call.

15. **`service`: pending_ops periodic drainer relocation.** `crates/service/src/actions/pending.rs::process_pending_ops` runs Service-side, triggered by `pending_ops.kick`. UI removes the periodic dispatch.

16. **`service` + `app`: encryption key handle (Phase 1.5 carry-forward 19d).** Default proposal: handle-based access. Service holds `SecretKey` in `BootContext`; UI's `from_boot_ready` no longer calls `rtsk::load_encryption_key`. Internal IPC method: `internal.encrypt_for_storage { plaintext, label }` -> `ciphertext`. UI uses this for credential persistence; the action service uses the local `SecretKey` directly (it lives Service-side already).

17. **`service` + `app`: `--test-fake-schema=N` end-to-end test (Phase 1.5 carry-forward 19c).** Test-helper flag analogous to `--test-fake-version`. Real-subprocess test asserts `ClientError::SchemaVersionChanged` arrives via Terminal across SIGKILL.

18. **`app`: `from_boot_ready` async store init (Phase 1.5 carry-forward 19a).** Body/inline/search store init relocates to async tasks dispatched from `BootingApp::update`. The `Booting -> Ready` transition moves earlier in the timeline (right after `BootReady`); the async store-init tasks fire `Message::ReadyStoreReady(...)` events that finalize the `ReadyApp` field set incrementally.

19. **`service`: `BootSharedState` flood resilience (Phase 1.5 carry-forward 19h).** `AtomicBool` "already in flight" on the `boot.ready` handler. Subsequent callers either join the existing waiter or fail fast.

20. **`service-api`: class-aware `boot_progress::emit` helper (Phase 1.5 carry-forward 19g).** Helper picks `try_send` for `Coalesce`/`Drop`, `send().await` for `MustDeliver`. Phase 2's new emitters use the helper exclusively.

21. **`service`: action latency benchmark.** `crates/service/tests/action_latency_smoke.rs` per the Architecture section. Runs in CI.

22. **`service` + `app`: error-shape decisions.** `RemoteFailure` wire type lockdown; per-`ActionError`-variant decision recorded in `service-api`'s error module doc-comment.

23. **`docs/service`: update problem-statement.md and implementation-roadmap.md.** Reflect Phase 2's settled items: action service relocation, `service-state` + `process-lifetime` crate boundaries, `ActionWirePlan`, per-plan correlation, generation-counter pre-dispatch contract, action latency benchmark, encryption-key handle. Delete the Phase 1.5 carry-forward bullets that are now closed. Bundle with the implementation commit that ships per CLAUDE.md.

## File-by-file changes

**New crates:**
- `crates/process-lifetime/` - `ProcessGuard` + `exit_if_parent_missing` extracted from service.
- `crates/service-state/` - `WriteDbState` constructor reachable only from `service`.

**New files:**
- `crates/service-api/src/action.rs` - wire types.
- `crates/service/src/handlers/action.rs` - dispatch handlers.
- `crates/service/src/actions/` - relocated execution surface from `crates/core/src/actions/`.
- `crates/service/src/snooze_runner.rs` - timer-driven resurfacing.
- `crates/service/src/progress.rs` - `IpcProgressReporter`.
- `crates/service/tests/action_latency_smoke.rs` - benchmark.
- `crates/app/src/in_flight_plans.rs` - UI-side correlation map.

**Modified files:**
- `Cargo.toml` (workspace) - register new crates.
- `crates/db/src/db/mod.rs` - `DbState` -> `ReadDbState` rename; private `ConnectionPool`; raw-`Connection` escapes deleted.
- `crates/db/src/lib.rs` - re-exports.
- `crates/common/src/types.rs` - `ProviderCtx` split.
- `crates/common/src/ops.rs` - `ProviderOps` method signatures take `ActionProviderCtx` for actions, `SyncProviderCtx` for sync.
- `crates/{gmail,jmap,graph,imap}/src/ops.rs` - update impls to match the new `ProviderCtx` signatures.
- `crates/core/src/actions/` - keep resolution + planning surface; remove batch + context + per-action execution.
- `crates/service-api/src/request.rs` - `ActionExecutePlan { plan }`, `ActionSend { request }` with their timeouts.
- `crates/service-api/src/notification.rs` - `OperationOutcome`, `ActionCompleted`, `SyncProgress` variants; `WithGeneration` impls; catalog extension.
- `crates/service/src/lib.rs` - boot-time action service init from `BootContext`.
- `crates/service/src/handlers/mod.rs` - register action handlers.
- `crates/service/src/parent_death/mod.rs` - DELETE (moved to `process-lifetime`).
- `crates/app/src/app.rs` - drop `App.action_ctx` field + `action_ctx()` method; add `in_flight_plans: HashMap<PlanId, PlanCompletionMeta>`.
- `crates/app/src/handlers/commands.rs` - `dispatch_plan` rewrite; pre-dispatch generation bumps; new message arms.
- `crates/app/src/handlers/pop_out/compose_send.rs` - `dispatch_send` becomes IPC.
- `crates/app/src/handlers/pop_out/compose_draft.rs` - draft save/delete via IPC.
- `crates/app/src/action_resolve.rs` - splits into "build wire plan" + "stash UI metadata keyed by plan_id".
- `crates/app/src/message.rs` - `ActionDispatched`, `OperationOutcomeReceived`, `ActionCompletedReceived` variants; updated Booting whitelist.
- `crates/app/src/update.rs` - dispatch the new messages.
- `crates/app/src/service_client.rs` - new `execute_plan`, `send`, `kick_pending_ops` methods; respawn-aware optimistic rollback wiring.

**Deleted files:**
- `crates/service/src/parent_death/` - moved to `process-lifetime`.

## Test plan

### Unit tests

- `service-api`: serde round-trip for `ActionWirePlan`, `OperationOutcome`, `ActionCompleted`, `RemoteFailure`. `RequestParams::ActionExecutePlan.timeout()` returns 60s. Catalog test (existing, from Phase 1.5 commit 30) automatically covers the two new `MustDeliver` variants - if the contributor forgets to implement `WithGeneration` on the payload, the test fails.
- `service-state`: `WriteDbState::new` constructs cleanly; `with_write_conn(|c| ...)` returns the result; raw `Connection` is not exposed.
- `service`: `ActionContext` constructs from `BootContext` (consumes `db_conn` + `encryption_key` per scope item 12). `IpcProgressReporter::emit_json` enqueues a `SyncProgress` notification with the right `Coalesce` key.
- `app`: `dispatch_plan` bumps both `nav_generation` and `thread_generation` BEFORE the IPC call. `in_flight_plans` correlation: an `OperationOutcome` for an unknown `plan_id` is logged at debug and dropped. `client_throttle` debounces a fast double-click.
- Compile-check: an `app/src/...` source file that tries `use service_state::WriteDbState` fails to compile. Add a UI-side test that imports `service_state` and asserts on the missing-crate-dependency build error (or document that the regression test is "the workspace doesn't build" if a cargo dependency cycle is added by accident).

### Integration tests (in-process)

- `tests/dispatch_in_process.rs::execute_plan_returns_ack_then_streams_outcomes` - submit a 3-operation plan; assert ack returns; assert 3 `OperationOutcome` notifications arrive in order; assert `action.completed` arrives last.
- `tests/dispatch_in_process.rs::action_completed_after_wal_fsync` - mock the SQLite WAL fsync hook; assert `action.completed` only fires after the hook returns.
- `tests/dispatch_in_process.rs::operation_outcome_carries_generation_tag` - drive the dispatch; verify the notification's `service_generation` matches the active reader's captured generation.
- `tests/dispatch_in_process.rs::stale_outcomes_dropped_after_respawn` - hand-craft an `OperationOutcome` with a stale generation; verify it never reaches the completion-effects handler (Phase 1.5's generation gate must apply to the new variants too).
- `tests/dispatch_in_process.rs::pending_ops_replays_after_respawn` - submit plan, kill Service after `ack` but before any `OperationOutcome`; respawn; assert the new Service drains the pending entry and emits outcomes.

### Real-subprocess smoke tests

- `crates/app/tests/service_subprocess.rs::star_toggle_round_trips_through_ipc` - spawn against a seeded data dir; submit a star-toggle plan; assert the thread row updates within 1 s; observe `OperationOutcome` + `ActionCompleted` on the wire.
- `crates/app/tests/service_subprocess.rs::bulk_archive_200_threads_under_budget` - submit a 200-operation plan; assert all outcomes arrive within 5 s.
- `crates/app/tests/service_subprocess.rs::optimistic_rollback_on_service_crash_mid_action` - submit plan, SIGKILL Service after ack but before completion; respawn; assert the UI rolled back optimistic updates AND the new Service eventually replays the operation via pending_ops.
- `crates/app/tests/service_subprocess.rs::compose_send_via_ipc` - construct a `SendRequest`; assert the SMTP submit path runs Service-side; observe `SendCompleted`.
- `crates/app/tests/service_subprocess.rs::undo_round_trips_as_compensating_plan` - dispatch action, undo, assert the inverse plan runs and outcomes match.
- `crates/app/tests/service_subprocess.rs::test_fake_schema_propagates_via_terminal` - per Phase 1.5 carry-forward 19c.

### Action latency benchmark

- `crates/service/tests/action_latency_smoke.rs` per the Architecture section. Targets:
  - Star-toggle p99 < 16 ms (in-process) / < 40 ms (real subprocess).
  - Bulk-archive of 200 threads p99 < 1.5 s (real subprocess).

### Manual matrix updates

- Real-keyboard star-toggle latency sanity. The benchmark covers the wire round-trip; this checks "does it *feel* fast" on a 50 GB seeded mailbox.
- Bulk-archive of 200 threads while the UI is mid-scroll. Verify rendering stays responsive throughout (Service is doing the work; the UI thread should be idle).
- Compose with a 50 MB attachment. SMTP submit takes 30+ s; verify the UI's send button stays in "sending" state with progress notifications, then transitions cleanly on `SendCompleted`.
- Service crash mid-action (`kill <service-pid>` while a bulk-archive is in flight). Verify optimistic rollback fires, the respawn happens within Phase 1.5's bounds, and the pending operations eventually drain.

## Open questions

Resolve in implementation:

1. **Encryption-key handle vs trusted-bytes-once.** Default proposal: handle-based (`internal.encrypt_for_storage` IPC method). Confirm during item 16 once the credential-persistence call sites are surveyed; if the IPC overhead is unacceptable on the hot credential paths, fall back to a one-shot `internal.export_key` call that the UI persists in-memory only.

2. **OperationOutcome ack notifications vs idempotent application.** Default proposal: idempotent application (UI handler is no-op on a duplicate plan_id, operation_id). Confirm during item 9 that re-applying every `OperationOutcome` variant truly is idempotent; if any path has a non-idempotent side effect (toast spam? audio cue?), revisit.

3. **Service-side periodic vs UI-driven kick for pending_ops.** Default proposal: UI-driven kick on `Message::SyncTick` (preserves existing tick policy that gates on focus/online state). Confirm during item 15 - if the UI-driven kick produces noticeable retry-latency drift, add a Service-side fallback periodic with a long interval (5 minutes) as a safety net.

4. **Where does `ActionExecutionPlan` -> `ActionWirePlan` conversion live?** Two options: (a) `From` impl on `ActionWirePlan` in `service-api` (no UI metadata leakage; clean dependency direction) or (b) `to_wire_plan(&self) -> ActionWirePlan` method on `ActionExecutionPlan` in core. Default proposal: (a) - service-api owns the wire shape and its conversion.

5. **Per-account concurrency in batch_execute Service-side.** Today's batch_execute groups by account and dispatches per-account in parallel (limit 4). Service-side, the natural place is the action handler. Decision: keep the limit at 4 unchanged in Phase 2; revisit if the action latency benchmark reveals contention.

6. **In-flight dedup set sharing.** The Service holds `ActionContext::in_flight: Arc<Mutex<HashSet<String>>>`. UI's client throttle is a separate map. Both layers serve - the question is whether the UI throttle entries should expire on `OperationOutcome` arrival vs `ActionCompleted` arrival vs a fixed timeout. Default: expire on `ActionCompleted`. Confirm during item 11 against the click-pattern manual test.

## Verification (end-to-end)

1. Fresh data dir + seeded DB. Star-toggle on 50 threads from the UI. Latency feels instant; benchmark agrees.
2. Bulk-archive of 200 threads. UI stays responsive; Service is at 100% CPU on its own thread; outcomes stream in. `ActionCompleted` arrives within budget.
3. Kill the Service mid-bulk-archive. UI logs the respawn; optimistic updates roll back; respawned Service drains pending_ops. The 200 outcomes eventually all land.
4. Compose with a 50 MB attachment. Send. UI shows "sending" with progress notifications during the SMTP upload; transitions to "sent" on `SendCompleted`.
5. Trigger an action that the provider rejects (e.g. archive a thread that the server already deleted). `OperationOutcome::RemoteFailure` arrives; optimistic update rolls back; toast surfaces the failure.
6. Two fast star-toggle clicks (within the 200 ms window). Only one IPC request fires; client throttle absorbs the second.
7. Snooze a thread to resurface in 30 s. Wait. Verify the resurface fires Service-side; the thread re-enters Inbox; the UI receives `OperationOutcome` for the resurface op.
8. `brokkr check` clean.
9. `cargo test -p app` includes `optimistic_rollback_on_service_crash_mid_action` and `pending_ops_replays_after_respawn`; both pass.
10. Compile-time check: a sed-driven test that injects `use service_state::WriteDbState` into a UI source file and asserts the workspace fails to build.

## Promotion criteria

This phase is done when:

- All items in `In scope` (including the `Phase 1.5 carry-forward closeout` block) are implemented and wired - actions execute Service-side, the UI no longer constructs `ActionContext`, all four non-`MailActionIntent` paths (undo, send, snooze resurfacing, draft delete) flow through IPC.
- All `Exit criteria` from the implementation-roadmap.md Phase 2 section are satisfied.
- Action latency benchmark in CI; star-toggle p99 < 16 ms in-process / < 40 ms real-subprocess.
- The `service-state` crate boundary holds: a UI source file that tries `use service_state::WriteDbState` fails to build.
- The `process-lifetime` crate exists; `service` and `app` both depend on it; the App -> Service dependency for `parent_death` is gone.
- Phase 1.5's three Phase-2 carry-forwards (`from_boot_ready` heavy init, `BootContext.db_conn` consumed, `SchemaVersionChanged` end-to-end test) are closed.
- Phase 1.5's five Phase-2 carry-forwards from commit 30 (`from_boot_ready` key reload, pre-dispatch generation bumps, `parent_death` boundary, `boot_progress::emit` regression-test contract, `BootSharedState` flood resilience) are closed.
- Reviewer signoff on this plan + the delivered code.

The next phase (Phase 3 - JMAP sync relocation, Tantivy/body/inline writer relocation) gets its own equivalent plan document at the time it's tackled. Phase 3 lights up the `SyncProviderCtx` shape Phase 2 scaffolds, locks down the body/inline/search write halves behind the same `service-state` crate boundary, and introduces the minimal cross-store invariant pass.
