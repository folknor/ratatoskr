# The Service - Implementation Roadmap

Companion to `problem-statement.md`. Each phase below is intended as a **separate `EnterPlanMode` session** that produces a focused implementation plan, lands as one or a small handful of commits, and unblocks the next phase. Nothing here is a complete plan in itself - the goal is to chart the order of attack and keep us from accidentally building things in the wrong sequence.

This document is a sketch. Phase scope, interfaces, and risks will firm up when each phase enters its own planning session.

**Cross-document dependency.** The attachment work (`docs/attachments/`) consumes this work. Specifically: attachments Phase 2 needs the Service hosting sync (Phase 3 here); attachments Phase 3 needs the `attachment.fetch` IPC method (lands as part of Phase 6 here); attachments Phase 7 (text extraction + indexing) is essentially Phase 7 here.

## How to read this

- **Goal** - one-sentence outcome.
- **Entry criteria** - what must already exist for this phase to start cleanly.
- **In scope / Out of scope** - hard boundaries for the phase.
- **Touchpoints** - files / modules likely to change. Indicative, not exhaustive.
- **Exit criteria** - observable evidence the phase is complete.
- **Risks / open questions** - unknowns to resolve during the planning session.

---

## Phase 1 - Process boundary scaffolding

**Goal.** A second process exists. The UI spawns it at start, exchanges `health.ping` over JSON-RPC stdio, kills it cleanly on shutdown, and detects + handles its disappearance. No real work moves across the boundary yet; this phase is the empty scaffold every later phase plugs into.

**Entry criteria.**
- Problem statement approved.
- Decision pinned on single-binary-multi-mode (one `ratatoskr` binary, `--service` flag selects mode) vs separate binaries. Default proposal: single binary with mode flag.

**In scope.**
- New crate `crates/service-api/` defining `Request`, `Response`, `Notification` enums + framing helpers shared between UI and Service. Phase 1 surface: just `health.ping` and `Shutdown`. `PROTOCOL_VERSION` constant; first ping asserts UI's constant matches Service's response or boot fails. `health.ping` envelope shape is **frozen** for v1 - any future Service binary must still be able to parse and respond to a v1 ping (so version-mismatch is visible even when the rest of the protocol changes).
- New crate `crates/service/` with `run_service()` async entry point + `run_service_with_io()` generic over `AsyncRead`/`AsyncWrite` for testability.
- `crates/app/src/main.rs` dispatches based on `--service` flag.
- New module `crates/app/src/service_client.rs`: spawns subprocess, pipes stdio, manages request/response correlation, dedicated stdout-writer task with bounded queue.
- Stdio corruption defense: at the top of `run_service()`, dup the original stdin/stdout to saved FDs and replace `STDOUT_FILENO`/`STDIN_FILENO` with `/dev/null`. Route the writer/reader tasks through the saved FDs. Otherwise transitive `println!` / default tracing-subscriber stdout / interactive panic handler reads desync the framing irrecoverably.
- Per-method timeout policy declared at API definition site (not call site). Phase 1 table: `health.ping` 5 s, `Shutdown` 30 s. Later phases extend.
- Notification class taxonomy in `service-api`: `enum NotificationClass { Coalesce, Drop, MustDeliver }` per-method. Phase 1 has no notifications, but the type lands so Phase 2's first notifications classify cleanly.
- Bounded notification channel with per-class semantics: `Coalesce`/`Drop` share a 1024-cap channel with enqueue-side coalescing/drop; `MustDeliver` gets a smaller (cap 64) channel with backpressure-on-producer. Reader task uses `try_send` for notifications - never blocks on a full channel, so a slow UI consumer cannot stall responses.
- Bounded in-flight requests on the Service side: max 64 concurrent handlers via semaphore, further requests wait rather than ballooning Service memory.
- Inbound frame cap (4 MiB) enforced *during* read using a bounded line decoder (`tokio_util::codec::LinesCodec::new_with_max_length` or equivalent `read_until` against a `Take`-wrapped reader). A 1 GiB no-newline payload must not OOM the Service before the cap fires.
- `kill_on_drop` is **disabled** on the spawned `tokio::process::Child` handle; shutdown is ordered explicitly (request -> SIGTERM after 30 s -> SIGKILL after another 5 s -> drop). The default `kill_on_drop(true)` would race the SIGTERM-then-SIGKILL escalation.
- `ServiceClient::Drop` ordering specified: cancel reader/writer/heartbeat task handles, await with deadline, close stdin (Service sees EOF), wait briefly for child exit, only then SIGKILL. Drop drains the pending map and rejects every outstanding sender with `ClientError::ServiceCrashed`.
- Notification dispatch into iced: subscription recipe (mpsc receiver wrapped per the existing `JmapPushReceiver` pattern). Phase 1 emits no notifications, but the recipe lands so Phase 2 plugs in cleanly.
- SIGTERM handler in the Service triggers the same shutdown drain as the request-driven path. Out-of-band `kill <service-pid>` therefore flushes Tantivy + writes the clean-shutdown sentinel rather than just exiting.
- Panic safety: every handler wrapped in `catch_unwind`; panics return `ServiceError::Panic`, dispatch loop continues. Process-level panic hook writes to the Service log file before the default behavior runs (otherwise panics in non-handler tasks vanish in production windowed UI).
- File-based logging: Service writes to `<app_data>/logs/service.<pid>.log` with simple size-based rolling (~10 MB cap, keep 3). PID in the filename avoids the multi-writer race during respawn. A `service.log` symlink in the same directory points at the current Service. stderr stays for `cargo run` debugging.
- **Sensitive-value logging policy** (defined in `problem-statement.md` § IPC): method names + IDs are loggable, params/results are not, OAuth auth codes never reach the log. Wire types use `RedactedString`/`RedactedBytes` wrappers.
- Heartbeat: 30 s interval. Logs missed beats only - no respawn here (lands in Phase 1.5). Heartbeat handler bypasses the per-request semaphore so heavy load can't starve it.
- Parent-death detection (v1: Linux + Windows; macOS deferred, design retained in `problem-statement.md`).
  - Linux: `pre_exec` + `prctl(PR_SET_PDEATHSIG, SIGTERM)` + post-prctl `getppid() == 1` check at startup.
  - Windows: parent creates a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, assigns the Service to it before spawning. No PID lookup, no PID-reuse race.
- Clean shutdown: `Shutdown` is a **request** (not a notification); UI awaits the response with a 30 s timeout, then SIGTERM, then SIGKILL after another 5 s. Service writes the clean-shutdown sentinel as the last step before responding.

**Out of scope.**
- Any actual functionality moving across the boundary.
- Respawn-on-crash (lands in Phase 1.5).
- Schema migrations + encryption-key relocation (lands in Phase 1.5).
- Tray icon, autostart, daemon promotion.
- Schema versioning of the JSON-RPC protocol beyond freezing `health.ping` (pin format-version-1 in v1; UI/Service shipped as a coupled pair).

**Touchpoints.**
- New crates: `crates/service-api/`, `crates/service/`.
- `crates/app/Cargo.toml` - dep on the two new crates.
- `crates/app/src/main.rs` - mode dispatch.
- `crates/app/src/app.rs` - boot launches `ServiceClient`, stores it on `App`.
- `crates/app/src/service_client.rs` - new module.
- Workspace `Cargo.toml` - register the two new crates.

**Exit criteria.**
- `cargo run -p app` spawns the subprocess; `ps` shows two processes.
- UI logs "Service ready (pid=...)" on start.
- Quitting the UI cleanly exits the Service via the request/ack handshake (no orphan in `ps`); clean-shutdown sentinel written.
- SIGKILLing the UI exits the Service within seconds on v1 platforms (Linux: PR_SET_PDEATHSIG + getppid recheck; Windows: Job Object kill-on-close).
- `<app_data>/logs/service.<pid>.log` exists and contains the boot + heartbeat lines, with no payload contents.
- In-process integration tests cover: happy-path ping, EOF-during-pending-request, malformed JSON, concurrent ping fan-out (id correlation), version mismatch, spawn failure, panicking handler returns `ServiceError::Panic` and Service stays up, oversize frame rejected without OOM.
- Real-subprocess smoke tests cover: spawn + ping, spawn + shutdown (clean ack), spawn + drop (no-orphan verification), Linux parent-death (SIGKILL UI, Service exits within 2 s).
- Manual matrix in [`manual-test-matrix.md`](manual-test-matrix.md) run on Windows: Job Object parent-death, clean shutdown handshake, and `SetStdHandle(NUL)` stdio corruption defense all observable on a real Windows host.

**Risks / open questions.**
- The stdio framing helper is more code than it looks: parse errors, frame-size rejection during read, EOF, partial reads, write timeouts, and panic catching all need explicit handling.
- `ResponseResult` is *not* a unified untagged enum. `pending` map holds `oneshot::Sender<Result<serde_json::Value, ServiceError>>`; the typed `request<R, P>()` wrapper deserializes the value into `R` after correlating by id. Avoids the silent-misroute trap of untagged-with-many-variants.
- `ServiceClient::Drop` ordering: cancel tasks -> close stdin -> wait briefly -> SIGKILL if still alive -> drain pending map (reject as `ClientError::ServiceCrashed`).

---

## Phase 1.5 - The Service becomes load-bearing for boot

**Goal.** The Service owns boot-side ownership of schema migrations + encryption key + pending-ops recovery, and is resilient enough that a crash during Phases 2-7 doesn't take the app down. The framing "minimal respawn" undersold the scope - this phase is the shift from "Service does nothing useful" to "UI cannot proceed past splash without a healthy Service." The respawn machinery is the survival kit that lets that coupling not regress UI boot when the Service flakes.

The phase lands as one milestone but with a clean commit-level split: respawn machinery, schema migration + key relocation, and global state-handle refactor are separate failure domains. Bisecting a regression should land on the right commit.

**Entry criteria.**
- Phase 1 landed.

**In scope.**
- **Respawn loop with terminal-failure classification.** If the Service exits unexpectedly: `ServiceClient` awaits the dying child's `wait()` (so the file lock is genuinely released) with a 5 s watchdog escalating to `start_kill`, then classifies the exit. Terminal classifications (`KeyLoadFailure`, `MigrationFailure`, `AnotherInstanceRunning`, `UnexpectedExit`) skip respawn and surface a fatal error to the UI; runtime crashes respawn after a 1 s sleep. Pending requests at crash time fail with `ClientError::ServiceCrashed`. The shared `NotificationQueue` survives respawns; each Service incarnation gets a `service_generation: u32` tag so stale notifications from the dying Service are dropped on dispatch. No exponential backoff, no crashloop detection, no UI status indicator yet (Phase 8).
- **Schema migrations relocate to Service boot.** UI's `ReadDbState` construction depends on a Service handshake signaling "schema OK, ready." `ReadWriteDb::init`'s velo->ratatoskr rename migration moves with the rest.
- **Encryption-key load relocates to Service boot.** Must happen before any token-bearing IPC. Missing or unreadable key file is a **fatal Service exit**, not the silent zero-key fallback the single-process code falls back to today - the auto-respawn machinery would otherwise widen the window where data gets written under the zero key.
- **Pending-ops boot recovery (`pending_ops::recover_on_boot`)** runs Service-side and completes before the boot handshake signals readiness. Stranded "executing" rows reset to "pending" before the UI thinks the Service is ready. (The *periodic* `pending_ops` drainer relocates with the action service in Phase 2; Phase 1.5 only owns the boot-time recovery.)
- **`db_mark_queued_drafts_failed_sync` relocates** from `App::boot` to Service boot. Today it races the iced runtime startup; on the Service side it slots into the boot sequence cleanly.
- **Boot handshake via dedicated `boot.ready` method, not extended `health.ping`.** Phase 1.5 introduces a separate `boot.ready` IPC method with a 10-minute timeout, leaving `health.ping` at 5 s for heartbeat use. The Service answers `boot.ready` only after migrations + key load + pending-ops recovery + queued-drafts sweep + thread-participants backfill complete. `boot.progress` notifications fire during the long boot, with per-phase coalesce keys so each `BootPhase` collapses independently and the ordered phase sequence reaches the UI. The chicken-and-egg (notifications need a consumer; the consumer is App; App is awaiting the boot handshake) resolves via **two-phase spawn**: `ServiceClient::spawn` returns a `mpsc::Receiver<SpawnEvent>`; `ChildSpawned` fires after spawn + version-check ping (App stores client + subscribes), `BootReady` fires after `boot.ready` returns (App transitions to Ready). See `phase-1.5-plan.md` for the detailed shape, including the service-generation tag for stale-notification dispatch across respawns.
- **Single-instance lock.** Service takes an OS-level file lock on `<app_data>/ratatoskr.lock` at boot via `fs2` (chosen over `fd-lock` for ecosystem reach). If locked, exit with `BootExitCode::AnotherInstanceRunning` (code 71); UI surfaces "Ratatoskr is already running." `AnotherInstanceRunning` is *not* a JSON-RPC `ServiceError` - the Service exits before stdio is established as a JSON-RPC channel. The `BootExitCode` enum (codes 70-73, picked outside the clap=2 / panic=101 / shell-signal ranges) maps via `BootClassification` so the UI distinguishes terminal boot failures (`KeyLoadFailure`, `MigrationFailure`, `AnotherInstanceRunning`) from runtime crashes; terminal failures **do not respawn**, runtime crashes do.
- **Global state-handle refactor.** Today `crates/app/src/main.rs` opens `Db` synchronously and stores it in `OnceLock<Arc<Db>>`; many sync `crate::DB.get().expect(...)` call sites assume the handle is populated before any code runs. After Phase 1.5, init defers until the Service handshake. Either the OnceLock becomes lazy (populated from `App::boot`'s post-handshake task), or App stores `Arc<Db>` as `Option<...>` (or moves to a state-machine `App = Booting | Ready { db, ... }`). The current `App` field is non-`Optional` (`crates/app/src/app.rs`), so this is a real refactor - the plan must pick a structure and audit `view()`, subscriptions, and every direct `crate::DB.get()` call site.
- **Spawn-failure policy.** Spawn failure at boot is **fatal** (UI refuses to boot). Consistent across phases - avoids a "this used to boot without the Service" regression in later phases.
- **Boot handshake "ready" semantics** (extended progressively across phases): "schema migrated + key loaded + pending-ops recovered." Phase 3 will extend with "Tantivy index initialized." Phase 6 with "all writers initialized." Each addition is a one-line addition to the handshake response shape.
- **JSON id-space across respawn.** `ServiceClient.next_id` continues across respawn while the new Service starts at id=1 in its own perspective. Harmless in v1 because there are no Server -> UI requests, but document it - if a future phase adds Server -> UI requests, the id space needs to be partitioned to avoid collision.

**Phase 1.5 ships its own plan doc** (sibling of `phase-1-plan.md`) before implementation. It is the largest UI-side surgery in the project and the under-specified items above need to be settled - particularly the `boot.progress` chicken-and-egg resolution, the `App` state-machine shape, and the `BootExitCode` mapping.

**Out of scope.**
- Backoff, crashloop detection, status indicator (Phase 8).
- In-flight request replay (Phase 8).

**Touchpoints.**
- `crates/app/src/service_client.rs` - two-phase spawn (`SpawnEvent::ChildSpawned` / `BootReady` / `Terminal`), respawn loop with `RunningState` + `RespawnConfig`, pending-request cleanup on respawn, dispatch-side stale-notification drop via the per-incarnation `service_generation` tag, `BootFailureReason` + `surface_terminal_failure` for the UI's terminal-failure surface. The 10-minute boot timeout lives on the dedicated `boot.ready` method, not on `health.ping` (which keeps its 5 s timeout for heartbeat use).
- `crates/service/src/lib.rs` (boot path) - run schema migrations Service-side; load encryption key (fatal on missing); `pending_ops::recover_on_boot`; single-instance lock; emit `boot.progress` notifications with per-`BootPhaseKind` coalesce so each phase compacts independently while the ordered phase sequence reaches the UI.
- `crates/app/src/main.rs` and `crates/app/src/app.rs` - `App` becomes a `Booting | Ready` state machine; the UI waits for `Message::ServiceBootReady` before constructing `ReadDbState` / loading accounts / populating sidebar. Boot-time `db_mark_queued_drafts_failed_sync` and per-account thread-participants backfill removed from UI side. `crate::DB: OnceLock<Arc<Db>>` is deleted; `crate::APP_DATA_DIR` is retained (it has multiple non-`App::boot` callers and does not depend on the Service handshake).

**Exit criteria.**
- `kill <service-pid>`; UI logs the crash, spawns a new Service, app continues to function. Stale notifications from the dying reader are dropped at dispatch via the generation tag.
- Pending requests at crash time fail with `ClientError::ServiceCrashed`, not a hang.
- Schema migration on a 50 GB DB succeeds within `boot.ready`'s 10-minute budget; UI splash renders per-phase migration progress including ordered `Migrating { current, total }` updates.
- Two `cargo run -p app` instances against the same data dir: second instance exits with `BootExitCode::AnotherInstanceRunning` (code 71); the UI surfaces "Ratatoskr is already running" rather than treating it as a crash. **No respawn loop in the second instance.**
- Missing `ratatoskr.key`: Service exits with `BootExitCode::KeyLoadFailure` (code 73); UI surfaces a fatal error and `iced::exit()`s. **No respawn loop** - the terminal-failure policy (scope item 15 of `phase-1.5-plan.md`) explicitly does not respawn on deterministic boot codes.
- `cargo test -p app` includes the respawn integration test (`respawn_after_sigkill_succeeds`) and the terminal-no-respawn integration test (`terminal_failure_at_initial_boot_does_not_respawn`).

**Risks / open questions.**
- The `OnceLock<Arc<Db>>` refactor is the largest bit of UI-side surgery in this phase. Many sync call sites assume DB is ready at any point in the program. Plan needs to enumerate which sites move to async and which can stay sync (most should be able to access the handle once `App::boot`'s post-handshake task populates it).
- This phase introduces the first "UI defers until Service ready" coupling. The common case (no pending migration) should not regress UI startup visibly - the boot handshake should complete in ms when the schema is already current.

---

## Phase 2 - Move the Action service into the Service

**Goal.** Action *execution* moves into the Service. Resolution + planning + completion-effects stay UI-side. UI sends a resolved plan; Service executes; outcome streams back as notifications. The read/write type split lands here in scoped form (only `ActionContext` consumes it); the global UI-side write lockdown is **not** complete until Phase 6.

**Entry criteria.**
- Phase 1 + 1.5 landed.
- A clear inventory of every Action service entry point in the current code, including the non-`MailActionIntent` paths: undo (`handlers/commands.rs`), compose send (`handlers/pop_out/compose_send.rs`), snooze resurfacing tick (`handlers/commands.rs` SyncTick path), draft delete/marking. The planning session enumerates these.

**In scope.**
- **Type-level write/read split (introduces the types; not the global lockdown).**
  - `DbState` -> `ReadDbState` + `WriteDbState`. Read half for the UI; write half lives behind the `service-state` crate boundary so UI cannot reach it.
  - Same shape for `BodyStoreState`, `InlineImageStoreState`, `SearchState` - but the UI continues to construct *read* halves; the *write* halves don't move out of UI reach yet because the writers (sync persistence) are still UI-side until Phase 3. Only the `Db` writer split is enforced in Phase 2.
  - Phase 6 closes the loop: every remaining UI-side write surface relocates and the write halves of all four state types become unreachable from the `app` crate.
  - This "the Phase 2 commit makes UI-construction of `WriteDbState` fail to compile" claim must be scoped to *the call sites this phase relocates*. Phase 6 owns the full lockdown. (Earlier draft asserted the global compile-time error in Phase 2 - that was provably false until the remaining UI writers moved.)
- **Action service: execution-only relocates.**
  - UI keeps: `MailActionIntent`, `resolve_intent`, `build_execution_plan`. These read selection state, sidebar scope, completion-behavior policy - all UI-owned. They produce a list of `MailOperation`.
  - Service gets: `batch_execute(plan: Vec<MailOperation>) -> outcomes`. UI no longer calls `core::actions::*` directly.
  - UI keeps: completion-effects (toast, auto-advance, undo eligibility, optimistic thread-list updates) - driven by `action.completed` notifications.
- **Non-`MailActionIntent` action paths also relocate.** Undo (compensating-action dispatch), compose send (SMTP submit + DB updates via `ActionContext`), snooze resurfacing (timer-driven mutation), and draft delete/marking all go through the IPC. Each is a Service-side handler with its own IPC method or rides on top of `action.execute_plan` if shape-compatible.
- **Wire plan is a serializable subset of `ActionExecutionPlan`.** Today's `ActionExecutionPlan` carries UI completion metadata (auto-advance hints, toast text, etc.) that does not belong on the wire. Define `ActionWirePlan` in `service-api`: `Vec<{ operation_id, account_id, thread_id, MailOperation }>`. The completion metadata stays UI-side, keyed by `operation_id` so the UI can correlate `OperationOutcome` notifications back to the originating intent.
- **Per-plan correlation id.** Notifications have no JSON-RPC `id` field. Each plan gets a Service-generated `plan_id`; per-operation `OperationOutcome` notifications carry `{ plan_id, operation_id, result }`; final `action.completed { plan_id, summary }` closes the stream.
- `service-api` new methods: `action.execute_plan { plan }` -> per-operation `OperationOutcome` notifications + final `action.completed`. Also `action.send`, `action.undo`, `action.snooze_resurface_due` (or whatever shape the planning session settles on).
- **Pending-ops worker relocates.** The retry queue (`db_pending_ops_*` drainer) runs inside the Service since it dispatches actions and the action execution layer is now Service-side. Boot recovery (`recover_on_boot`) already moved in Phase 1.5; the periodic drainer moves here.
- **`process_pending_ops` periodic.** Today triggered UI-side from `Message::SyncTick`. Choices: Service runs its own periodic, or UI sends a `pending_ops.kick` notification on tick. The latter keeps tick policy UI-side (depends on focus, online state); preferred.
- **`ProviderCtx` shape adjustment** is a top-level deliverable, not a planning sub-bullet. Today `ProviderCtx { db, body_store, inline_images, search, progress, ... }` is passed to every provider method, including action methods that only touch `ctx.db`. The Service-side `ActionContext` cannot hand the action methods a write-half SearchState (the writer doesn't exist yet until Phase 3). Decision in the Phase 2 plan: split `ProviderCtx` into action-side (`ActionProviderCtx { db_write, key, ... }`) and sync-side (`SyncProviderCtx { db_write, body_write, inline_write, search_write, progress, ... }`) variants. Touches every `ProviderOps` method signature in `crates/common/src/ops.rs` and every implementation in the four provider crates - mechanical but broad.
- **`ProgressReporter` trait redesign** is a top-level deliverable. The Service-side impl serializes events into IPC notifications; UI's existing `IcedProgressReporter` keeps consuming them. Trait method signatures must become serializable (no `&Connection`, no `Arc<Mutex<...>>` in method args). Survey current trait + impls in `crates/core/src/progress.rs` and call sites in sync paths; the redesign is required before any handler that emits progress can be relocated. Per-operation outcomes from `batch_execute` are `MustDeliver`; progress events are `Coalesce`.
- **Search-index write during actions.** Some actions (delete) currently write to Tantivy. The Tantivy writer doesn't move Service-side until Phase 3. Phase 2 plan must pick: (a) Phase 2 actions skip the index update and Phase 3's consistency pass cleans up; (b) Phase 2 actions defer the index update via a wire round-trip back to a UI-side writer (gross); (c) move the Tantivy writer earlier than Phase 3 (out of phase). Default proposal: (a) - the Phase 3 minimal cross-store invariant pass already drops orphaned Tantivy docs whose message ids no longer exist, so a deleted message's index entry gets cleaned up at the next Phase 3 boot regardless. Document the temporary inconsistency window in the plan.
- **Action latency target (design only, no CI gate).** Star-toggle p99 of 5-15 ms is the design target (16 ms = one frame at 60 fps). A CI smoke test was originally planned and dropped during Phase 2 close-out (a p99-budget gate flaps on shared runners more than it catches real regressions). Latency stays a manual-matrix concern; the per-phase commentary explicitly drops the benchmark line.
- **Optimistic UI rollback path** triggers from both `action.completed` (failure) and `ClientError::ServiceCrashed` (Service died mid-action). Otherwise an optimistic UI update for an action that crashed the Service stays permanent.
- **Generation counters bump pre-dispatch** on plan submission, not post-completion. Otherwise the IPC delay creates a stale-load window.
- **Read-after-write coherence:** Service emits `action.completed` only after WAL fsync. Documented as a contract on the IPC method, not just an implementation detail.

**Out of scope.**
- Sync. Sync still happens in the UI process - it'll move in Phase 3.
- Push. Same.
- Streaming progress for long-running actions (e.g. bulk archive of 500 threads). The notification model supports this naturally; the *transport* must support it (cap 1024 should be sufficient with `Coalesce`/`MustDeliver` separation).
- Cancellation of in-flight `action.execute_plan` (e.g. user closes a window mid-bulk-archive). Plan runs to completion in Phase 2; explicit `action.cancel_plan` is a follow-up if needed.
- Calendar mutations (their `MailOperation`-shaped flat list is unclear because of series-vs-occurrence + RSVP semantics). Phase 6.

**Touchpoints.**
- New crate `crates/service-state/` (or equivalent) - houses `WriteDbState`, the `app` crate does not depend on it.
- `crates/db/src/db/...` - introduce the read/write state-type split. Drop `Db::write_db_state()` from the app-visible API; drop `DbState::conn()` / `from_arc()` raw-Connection escapes.
- `crates/stores/src/...` - introduce read/write halves for body / inline-image / search states (read halves consumed in Phase 2; write halves enforced unreachable in Phase 3 with sync, Phase 6 with the rest).
- `crates/common/src/types.rs` (`ProviderCtx`) - `ProviderCtx` shape adjustment per scope above.
- `crates/{jmap,gmail,graph,imap}/src/ops.rs` - method signatures adjust per the `ProviderCtx` decision.
- `crates/service-api/` - `action.execute_plan` + `action.send` + `action.undo` + `action.snooze_resurface_due` methods; `OperationOutcome` / `action.completed` notifications; `ActionWirePlan` type.
- `crates/service/src/handlers/action.rs` - new.
- `crates/service/src/pending_ops.rs` - new (retry queue periodic drainer).
- `crates/service/src/progress.rs` - new (`IpcProgressReporter`).
- `crates/app/src/handlers/commands.rs` - dispatch goes through the service client; planning stays here. Including undo, snooze tick, send paths.
- `crates/app/src/handlers/pop_out/compose_send.rs` - dispatch via IPC.
- `crates/app/src/action_resolve.rs` - splits into "build wire plan" + "stash UI metadata keyed by operation_id."
- `crates/core/src/actions/context.rs` - decouple `ActionContext` from `App` state references; reconstructed Service-side from `WriteDbState`, encryption key, write halves of stores.
- `crates/app/src/app.rs` - remove `App.action_ctx` field and `App.action_ctx()` method.

**Exit criteria.**
- All user-triggered actions (archive, delete, label, snooze, send, undo, etc.) build the plan UI-side and execute Service-side.
- `service-state` crate is scaffolded so a UI source file that tries `use service_state::WriteDbState` fails to build at the dependency-graph level. (`WriteDbState` itself is not yet wired into Service-side write paths - the action worker still uses `ReadDbState::conn()`. The narrower invariant Phase 2 enforces is `ActionProviderCtx` excluding body / inline / search store handles, regression-tested via exhaustive destructure. Full lockdown lands in Phase 6 alongside the global write-surface relocation.)
- `MutationLog` entries continue to land correctly (logged from the Service side).
- Undo continues to work via the IPC path.
- Pending-ops queue continues to drain (now Service-side).
- Cross-account plans split UI-side at dispatch (one sub-plan per account; the Service handler rejects multi-account plans because `action_jobs.account_id` is single-valued).
- Per-account leasing fairness (the plan's "cap 4 per account, separate semaphore") is **not** shipped: the worker leases one op at a time, sequentially. UI-side split keeps the practical penalty bounded; re-introducing parallelism is a follow-up if bulk-action latency becomes a hot path.

**Risks / open questions.**
- The interaction with the UI's `nav_generation` / `thread_generation` counters: bump pre-dispatch on plan submission, not post-completion (resolved above).
- Error type serialization across the boundary - decide which `ActionError` variants survive intact and which collapse into a generic `RemoteError`.
- The `ActionContext` decoupling from `App` state may force some other extractions in `core::actions::context`. Scope to keep on the radar but not block the phase.
- Compose send timing: SMTP can take 30+ seconds with a 50 MB attachment. Send method gets a generous IPC timeout; UI surfaces in-flight state via progress notifications.
- The `in_flight: Arc<Mutex<HashSet<String>>>` action-dedup map currently lives in `ActionContext`. Service-side dedup is correct; UI may also need a client-side throttle to avoid sending two IPC roundtrips on a fast double-click. Decide in planning.

**Phase 1.5 carry-forward (Phase 2 status).**
- **CLOSED in Phase 2 (final): compose-send relocation.** `action.send` IPC ships the bytes-ownership transfer: UI stages each attachment under `<app_data>/staging/<send_id>/<index>.bin`, sends `SendWireRequest` with `StagingFile { relative_path, content_hash }` references, Service handler verifies SHA-256 + atomically renames into `<app_data>/send_vault/<send_id>/` + journals as `kind = 'send'` quiet job, returns `SendAck`. Worker `drain_send_jobs` reads vault bytes, calls existing `send_email` SMTP path, finalizes the job, unlinks the vault directory. Boot recovery (`reconcile_send_vault`) sweeps orphan vault dirs whose `job_id` is not in the journal or whose status is terminal. Closes the last ✗ in Phase 2's write-surface inventory.
- **CLOSED in Phase 2: pre-dispatch generation-counter bumps.** `dispatch_plan` now bumps both `nav_generation` and `thread_generation` BEFORE the IPC `action.execute_plan` call. Closes the stale-`ThreadsLoaded` window the bugs review flagged.
- **CLOSED in Phase 2: `parent_death` crate boundary.** Extracted to `crates/process-lifetime/`. Both `service` and `app` depend on it; the App → Service dependency for `ProcessGuard` is gone.
- **CLOSED in Phase 2: `BootSharedState` flood resilience.** `boot_ready_inflight: AtomicBool` plus a cache-result-first check on the handler. Subsequent callers fail fast with `Backpressure` or read the cached result.
- **PARTIAL in Phase 2: `apply_standard_pragmas` / `BootContext.db_conn` consumed.** The action worker now consumes `db_conn` from `BootSharedState`; the encryption key is also reachable. The two-connection waste (Service worker + UI reader) remains - the third UI-side write connection collapses with the global write-half lockdown in Phase 6.
- **DEFERRED to Phase 8: `boot_progress::emit` per-phase regression test / class-aware helper.** First implementation attempt (try_send for Coalesce/Drop, awaited send for MustDeliver) introduced a hang in the `service_subprocess` test cohort and was reverted. Now a first-class Phase 8 item alongside the flaky-test root-cause it's entangled with (the same writer-task drain-ordering issue is the most likely cause of both). Coalesce-class try_send remains correct for `boot.progress` / `sync.progress` regardless.
- **DEFERRED to Phase 8: `ReadyApp::from_boot_ready` heavy synchronous init.** Body / inline / search store init still runs synchronously inside `from_boot_ready`. Less load-bearing now that `ActionContext` is Service-side, but the splash-blocking moment remains on slow disks. Pure UI surgery (relocate to async tasks dispatched from `BootingApp::update`; `Message::ReadyStoreReady` arms finalize the `ReadyApp` field set incrementally). Slotted into Phase 8 because the crash-recovery polish already touches the boot path; can move earlier if a UI engineer is in the area.
- **DEFERRED to Phase 8: `SchemaVersionChanged` e2e test (`--test-fake-schema=N`).** No real-subprocess test flips the schema across a respawn yet. Phase 2 introduced real schema-version sensitivity (the action worker depends on the schema being what the UI thinks it is), so the test still belongs here in spirit; it just didn't land in the same milestone. Slotted alongside the rest of the Phase 8 crash-recovery / respawn test polish.
- **DEFERRED to Phase 6: `from_boot_ready` re-loads the encryption key.** UI still re-reads `ratatoskr.key` post-handshake instead of consuming the Service's already-validated handle. The TOCTOU window the arch review flagged remains. Slotted into Phase 6a alongside the rest of the credential / account / preference write-surface relocations; the fix is an `internal.encrypt_for_storage` IPC method (or a one-shot key-export), both shapes sketched in `phase-2-plan.md` § 19d.
- **DROPPED: action latency benchmark in CI.** A p99 budget enforced by a CI smoke test reliably flaps on shared runners more than it catches real regressions, and the budget itself becomes a load-bearing number with no clear owner. Manual matrix items 1-2 in `phase-2-plan.md` (real-keyboard star-toggle on a seeded mailbox; bulk-archive of 200 threads while mid-scroll) cover the "does it feel fast" check. If a regression surfaces we'll measure it ad hoc rather than running a permanent CI gate.

---

## Phase 3 - Move sync into the Service (JMAP first), including Tantivy/body/inline writer relocation

**Goal.** JMAP delta sync runs inside the Service, including all of its write-side interactions (DB, body store, inline image store, Tantivy writer). UI gets sync progress + completion via notifications. Tantivy reader stays UI-side, driven by `index.committed` notifications. **Minimal cross-store invariant pass lands here**, not deferred to Phase 8 - this is the moment the Service becomes a four-store writer.

**Entry criteria.**
- Phase 1 + 1.5 + 2 landed.
- The Action service migration validated the IPC pattern under realistic load.

**In scope.**
- **Tantivy writer + body store writer + inline-image store writer relocate.** Sync today indexes via `SearchState`, persists bodies via `BodyStoreState`, persists inline images via `InlineImageStoreState`. All three are written *only* by sync paths (`store_message_bodies`, `store_inline_images` in `sync/persistence.rs`, plus Tantivy add-doc calls). Sync moving Service-side means all three writers come with it - they're entangled. This phase enforces that the write halves of all three are unreachable from the `app` crate (ride on top of the `service-state` crate boundary established in Phase 2).
- `service-api` new methods: `sync.start_account { account_id }`, `sync.cancel_account { account_id }`. New notifications: `sync.progress` (`Coalesce`), `sync.completed` (`MustDeliver`), `index.committed` (`MustDeliver`).
- Service owns sync dispatch: `sync_delta_for_account` runs Service-side using Service-owned `WriteDbState` / write halves of body store / inline image store / Tantivy writer.
- UI's `dispatch_sync_delta` -> `Task::perform(...)` becomes `service_client.start_sync(account_id)` returning a future that resolves on `sync.completed`.
- **Cancellation semantics change.** Today `iced::task::Handle::abort()` interrupts at any await boundary. Tomorrow cancellation is IPC: UI sends `sync.cancel_account`, Service flips a `CancellationToken`, sync code checks it. **Different semantics:** abort interrupts arbitrarily; token-check only at explicit checkpoints. A 60 s sync iteration that doesn't check the token will run to completion after cancel. This phase places explicit cancellation checkpoints in the per-folder loop and the per-message-batch loop of every sync path. Documented test: cancel-mid-sync returns within 5 s.
- **Tantivy writer is single-instance per index; concurrent per-account syncs contend.** Today, the iced runtime implicitly serializes writer access; Service-side, multiple concurrent sync handlers compete for one writer. Wrap as `Arc<Mutex<IndexWriter>>` (or batch via a writer task with a queue). Plan choice in the planning session, but the choice is not deferrable.
- **Tantivy index initialization is part of the boot handshake.** First-run UI on a fresh data dir requires the `search_index/` directory to exist before `IndexReader::open()` succeeds. The Service initializes the index (writer creates the directory + initial segment) before signaling boot-handshake readiness. Otherwise a fresh-install boot races the reader open against the Service init.
- UI search reader subscribes to `index.committed` notifications and calls `reader.reload()` on each. **Debounced UI-side** (~200 ms or "next idle frame"); a heavy sync emits dozens of `index.committed`/sec and reload-per-event has visible cost.
- **Minimal cross-store invariant pass.** On Service boot, if the clean-shutdown sentinel is missing, run the per-store recovery scan before signaling boot-handshake readiness:
  - For every Tantivy doc: assert the message id still exists in `messages`. Drop orphans.
  - For every body-store entry: assert the message id still exists. Drop orphans.
  - For every inline-image-store entry: assert the message id still exists. Drop orphans.
  Naive full-table scans, no marker-file optimization yet (that's Phase 8). Bounded by message count; idempotent. Logged with stats. Adds the kill-mid-sync integration test that verifies the pass triggers and recovers correctly.
- **`App.sync_handles`** (the `iced::task::Handle` map from the recent sync-cancellation work) replaced by Service-side cancellation tokens; UI's cancel call becomes IPC.
- **`abort sync on account deletion`** wiring continues to function via IPC: account deletion sends `sync.cancel_account`.
- **JMAP push subscription survives one phase as a UI-side task.** The push *trigger* still arrives via the existing `JmapPushReceiver` channel until Phase 4. Between Phase 3 and Phase 4, push events take this round-trip: WebSocket (UI) -> channel (UI) -> IPC `sync.start_account` (UI->Service) -> sync (Service). Documented transitional state, removed in Phase 4.

**Out of scope.**
- Other providers (Phase 5 ports them).
- Push notifications (Phase 4).
- Re-tuning per-account concurrency limit (4) - stays the same.
- New extractors / attachment indexing (Phase 7).
- Optimized cross-store invariant pass (Phase 8 owns the marker-file gating + bounded re-scan windows).

**Touchpoints.**
- `crates/search/src/lib.rs` - lock down the `SearchState` writer half behind the `service-state` crate boundary; UI can no longer construct it.
- `crates/stores/src/body_store.rs` and `crates/stores/src/inline_images.rs` - same.
- `crates/service-api/` - sync methods; `sync.progress`, `sync.completed`, `index.committed` notifications.
- `crates/service/src/handlers/sync.rs` - new.
- `crates/service/src/startup_invariants.rs` - new (minimal pass, gated by sentinel).
- `crates/sync/src/persistence.rs` - now writes through Service-owned writer halves.
- `crates/sync/src/...` and `crates/jmap/src/sync/...` - explicit cancellation checkpoints in per-folder and per-batch loops.
- `crates/app/src/handlers/provider.rs` - rewire `dispatch_sync_delta` to talk to the Service.
- `crates/app/src/update.rs` - `Message::SyncComplete` arrives via IPC notification rather than `Task::perform` callback.
- `crates/app/src/...` (search reader sites) - one shared reader; reload on `index.committed` (debounced).

**Exit criteria.**
- A JMAP sync triggered from the UI runs in the Service process (visible in `top` / `htop`).
- Sync progress events reach the UI status bar in real time (coalesced under load).
- Cancel mid-sync returns within 5 s (cancellation checkpoints exercised).
- The "abort sync on account deletion" wiring continues to function via IPC.
- Search results returned from the UI reader reflect Service-side writes within milliseconds of `index.committed`.
- UI compilation fails if anyone tries to construct a Tantivy `IndexWriter`, write-half `BodyStoreState`, or write-half `InlineImageStoreState` outside the `service` crate.
- Kill-mid-sync test: SIGKILL the Service mid-sync; restart; minimal invariant pass runs (sentinel missing); orphans dropped; sync resumes from last checkpoint.
- First-run on a fresh data dir succeeds without a reader-vs-writer race.

**Risks / open questions.**
- Tantivy writer lock recovery on uncleanly-killed Service. Tantivy ≥0.21 recovers stale writer locks; verify with the kill-mid-write test. Document the version bound in `crates/search/Cargo.toml`.
- Tantivy commit cadence under indexing pressure - too frequent slows things down, too rare loses recent work on crash. Plan needs a policy (commit every N docs or M minutes, batch under sustained pressure).
- Currently `Message::SyncComplete` triggers a navigation reload + thread list refresh. That side effect stays UI-side; Service just notifies.
- Progress event volume on cold-sync of large mailboxes can be IPC-bound if `sync_delta_for_account` reports per-message. The progress reporter shim's coalescing policy needs explicit cadence (e.g. coalesce per account, emit at most every N ms or every K messages, whichever first).

---

## Phase 4 - Move push notifications into the Service

**Goal.** JMAP push receivers run inside the Service. Push events become Service-to-UI notifications that trigger Service-side sync.

**Entry criteria.**
- Phase 3 landed (push triggers sync, which now lives in the Service).

**In scope.**
- JMAP push WebSocket receiver moves into the Service.
- The existing `JmapPushReceiver` channel collapses - the UI no longer subscribes to push directly. Push events arriving at the Service trigger the Service-internal sync path.
- UI gets a `push.event { account_id, service_generation }` notification (`Coalesce { key: account_id }`) for visibility (status bar updates); the actual response (sync) happens entirely in the Service.
- **OAuth refresh runs in-Service.** No IPC handshake. `JmapClient::ensure_valid_token` is called before `start_push`, and an auth resolver is threaded into `push_connection_loop` so reconnects re-resolve the bearer. Refresh is purely DB+HTTPS, both Service-internal; the original "temporary `oauth.refresh_request` IPC" plan was a planning-doc error corrected during Phase 4 plan review.
- **Drain consolidation.** Phase 4 ships an explicit drain consolidation that fixes a pre-existing Phase 3 bug (`lifecycle::run_drain` writes the sentinel before `dispatch.rs` shuts down `SyncRuntime`). The consolidated helper orders: PushRuntime → SyncRuntime → search-writer flush → marker unlink → sentinel write.
- IMAP IDLE follows the same pattern when it lands. Out-of-scope until then.

**Out of scope.**
- IMAP IDLE (still pending; comes when IMAP IDLE itself lands in the codebase).
- Cross-platform OS-level notification surfacing (toast on new mail). Separate work.
- Push state hardening (crash-aware fresh-start). Phase 4 inherits today's resume-from-saved-state behavior; Phase 8 hardens.
- Re-auth re-arm of dead push entries. Phase 8.

**Touchpoints.**
- `crates/service/src/push.rs` - new. PushRuntime, bridge tasks, panic supervisor, auth resolver closure.
- `crates/jmap/src/push.rs` - replace captured `auth_header` with `auth_resolver` parameter.
- `crates/service-api/src/notification.rs` - `push.event` notification variant + catalog test cases (inline at lines 469-585).
- `crates/service/src/lifecycle.rs` + `crates/service/src/dispatch.rs` - drain consolidation.
- `crates/service/src/handlers/sync.rs` - detached push start/cancel piggyback.
- `crates/app/src/handlers/provider.rs` - delete JMAP push subscription wiring entirely.
- `crates/app/src/subscription.rs` - drop the `jmap_push_subscription` recipe.
- `crates/app/src/service_client.rs` - add `Notification::PushEvent` arm.
- `crates/core/src/jmap_push.rs` - deleted.

**Exit criteria.**
- A change pushed to a JMAP mailbox triggers a sync in the Service without the UI being on the call path.
- Status bar still surfaces "new mail arrived" indicators.
- Token expiry mid-subscription survives via the in-Service auth resolver; reconnect uses the refreshed bearer.
- Drain consolidation lands; sentinel writes only after `SyncRuntime::shutdown` completes.

**Risks / open questions.**
- WebSocket lifetime: today the receiver lives as long as the iced subscription. Service-side, it lives as long as the Service. This is strictly more durable - good.

---

## Phase 5 - IMAP cancellation depth + calendar / GAL relocation

**Goal.** Close the residual UI-side sync work after Phase 3 cascaded the email-sync-relocation pattern to all four providers via `ProviderOps::sync_delta`. Specifically: finish IMAP cancellation depth (the `let _cancellation_token = cancellation_token;` incomplete-port markers), relocate calendar sync and GAL refresh into the Service, and collapse `Message::SyncTick` to be entirely IPC kicks with no UI-side provider work.

**Entry criteria.**
- Phase 3 landed for JMAP. The dispatch pattern via `ProviderOps::sync_delta` already drives all four providers.
- Phase 4 landed; consolidated drain shape is established.

**In scope.**
- IMAP per-folder cancellation checkpoints (close the Phase 3 incomplete port).
- New `CalendarRuntime` mirroring `SyncRuntime`'s lifecycle surface but with simpler invariants (no marker-file lifecycle, no four-store writer halves, no invariant-pass entry).
- `calendar.start_account_sync` request + `calendar.completed` notification + `calendar.kick` client notification.
- `gal.kick` client notification with a Service-side handler that iterates supported accounts.
- Drain consolidation extension: `PushRuntime → CalendarRuntime → SyncRuntime → search-writer → sentinel`.
- UI-side teardown of `sync_calendars` and `refresh_gal_caches`; `Message::SyncTick` collapses to four IPC kicks.

**Out of scope.**
- Provider-specific protocol improvements (CONDSTORE/QRESYNC, batch APIs, etc.) - those are tracked in their own roadmap docs.
- IMAP IDLE (rides into Phase 4's pattern when IDLE lands).

**Touchpoints.**
- `crates/imap/src/imap_initial.rs` + `imap_delta.rs` - thread `cancellation_token` into the per-folder loop and per-batch persist points; remove the `let _cancellation_token = cancellation_token;` incomplete-port markers.
- `crates/service/src/calendar.rs` - new `CalendarRuntime`.
- `crates/service/src/handlers/calendar.rs` - new request + kick handlers.
- `crates/service/src/handlers/gal.rs` - new kick handler.
- `crates/service-api/src/calendar.rs` - new wire types.
- `crates/service-api/src/{notification.rs,client_notification.rs,request.rs}` - new variants.
- `crates/service/src/dispatch.rs` - drain step insertion + handler dispatch arms.
- `crates/service/src/boot.rs` - `CalendarRuntime` slot install.
- `crates/app/src/handlers/provider.rs` - **delete** `sync_calendars` and `refresh_gal_caches`; **add** thin `kick_calendar_sync` / `kick_gal_refresh` IPC wrappers.
- `crates/app/src/update.rs` - `Message::SyncTick` collapse + new `Notification::CalendarCompleted` arm.

**Exit criteria.**
- IMAP `let _cancellation_token = cancellation_token;` markers gone; cancellation interrupts mid-fetch.
- Calendar sync runs Service-side via `CalendarRuntime`; UI fires `calendar.kick` on the hourly tick.
- GAL refresh runs Service-side via `gal.kick`; no per-account runtime needed.
- `Message::SyncTick` does no UI-side provider work.
- Drain order: PushRuntime → CalendarRuntime → SyncRuntime → search-writer → sentinel.

**Risks / open questions.**
- IMAP session pooling: per-folder session reuse stays inside the IMAP provider; doesn't change with the cancellation depth fix.
- See `docs/service/phase-5-plan.md` open questions for cadence-ownership (UI vs Service hourly timer), per-account vs per-calendar concurrency, GAL handler concurrency tradeoffs.

**Phase 5 status (as landed).** Of the Phase 5 entries above:
- ✓ Cycle-break prerequisite: extracted `crates/action-types/` so `cal::actions` and `service::actions` share the type contract without forcing `rtsk -> service`. The three edges that kept the cycle alive are gone (action shim, `sync_dispatch` re-export, dead `core::chat::mark_chat_read_remote`). `cargo metadata` is cycle-clean.
- ✓ IMAP cancellation depth: `let _cancellation_token = cancellation_token;` markers gone. `&CancellationToken` threads into per-folder loops, per-batch persist points, and helpers (`batch_delta_check`, `imap_delta_janitor::run_deletion_detection`, `client::sync::delta_check_folders`). Point-checks between RPCs (NOT `tokio::select!` - IMAP is a stateful session and dropping a future mid-FETCH leaves unread response data on the wire).
- ✓ `CalendarRuntime` mirrors `SyncRuntime`'s lifecycle surface with `closed: AtomicBool` from `PushRuntimeInner`. Per-runtime semaphore (cap 4) bounds the post-respawn thundering herd. No marker-file lifecycle - calendar sync is idempotent against CTags/ETags.
- ✓ `service-api` calendar wire types, with **dual notifications**: `CalendarRunCompleted` (MustDeliver, ServiceClient-consumed by per-run_id awaiters, mirrors `SyncCompleted`) + `CalendarChanged` (Coalesce, `CoalesceKey::CalendarChanged(account_id)`, UI-dispatched for view reload). The original "single `calendar.completed`" entry would have left the UI calendar reload path silent - same shape `SyncCompleted` already has, where the routing is consumed inside `ServiceClient`. The dual split was caught during plan revision.
- ✓ `cal::sync::calendar_sync_account_impl` flipped to `&service_state::WriteDbState` at the Service-facing surface. Body derives a `ReadDbState` view internally so deeper helpers (still on `&ReadDbState::with_conn` for writes) compile unchanged - that's the `cal::actions` write-surface escape, retired in Phase 6 alongside the rest of `cal::actions`.
- ✓ `SyncCancelAck` extended with `calendar_run_id: Option<CalendarRunId>` for the **piggyback model** of account-deletion cancel. `handle_cancel_account` calls `CalendarRuntime::cancel_account` after the existing push piggyback; UI's `cancel_and_await` awaits both sync and calendar terminal completions before issuing the DB DELETE. Cleaner than a UI-side fan-out (one IPC vs three) and matches the push pattern. The plan's initial draft assumed an existing `join!` UI-side that didn't exist; the revision pass corrected it.
- ✓ Drain: `PushRuntime → CalendarRuntime → SyncRuntime → search-writer → sentinel`. The calendar-before-sync ordering is **reserved**, not load-bearing today - the action worker is alive throughout the drain, so calendar can drain before or after sync without affecting action-worker availability. Order is fixed so a future RSVP cancel-cleanup wiring is a one-liner. The `// reserved` comment is the contract for that judgement; reviewers should not look for an RSVP path that doesn't exist.
- ✓ Notification drain bounded at 5s aggregate. A wedged GAL handler (60s × N accounts worst case) cannot stall shutdown; past the cap remaining notification tasks are aborted and the count is logged. **Caveat:** handlers wrapping work in `tokio::task::spawn_blocking` see only the *outer* future aborted - the GAL handler's blocking DB writes run to completion regardless. Acceptable because GAL writes are bounded and idempotent; documented at the helper.
- ✓ `Message::SyncTick` collapses to **three notifications + one request fan-out** (the original "four IPC kicks" framing conflated wire-protocol semantics). Calendar and GAL both relocate as Drop-class kicks; the dedicated 1-hour `Message::GalRefreshTick` subscription deleted.
- ✓ UI: `Notification::CalendarChanged` debounced reload. 250ms trailing-edge debouncer collapses an N-account kick batch into one `reload_calendar_events()` call.
- ✓ `production_notification_catalog` gained `CalendarRunCompleted` and `CalendarChanged` so the cross-respawn round-trip test catches both new variants.

**Phase 5 known-gaps.** Phase 6 carries forward:
- **Calendar event mutations (`cal::actions::*`) still run UI-side.** Phase 5 relocates only the periodic provider sync/cache refresh; event mutation relocation is Phase 6.
- **`cal::actions` write-surface escape** (writing through `&ReadDbState::with_conn`) - same shape Phase 4 cleaned up for sync. Phase 6 retires it.
- **`mutated` flag is coarse.** `CalendarRuntime` emits `CalendarChanged` on Completed and Cancelled but not on Failed-with-partial-batch. Tightening requires threading `&AtomicBool` through `cal::sync` end-to-end; deferred to Phase 6 alongside the helper refactor.
- **Phase 9 tray-resident TODO marker** lives in `app/src/subscription.rs` near the `SyncTick` subscription. When tray-resident lands, the cadence moves Service-side (the staleness-gate logic transplants unchanged).
- **Integration test cohort partly Phase 8.** Lifecycle-only unit tests landed (`CalendarRuntime::cancel_account` for missing entries, shutdown safe on empty runtime, `start_account` Err after shutdown). Real runner behavior against a stub provider needs a fake-CalDAV fixture - same caveat as Phase 4's PushRuntime cohort.
- **Five plan-task-13 unit tests slipped from "in scope" to "deferred."** GAL handler mutex test (the plan called this **required** for the `NOTIFY_CAP=4` duplicate-call hazard fix); notification-drain timeout test (verifies the `spawn_blocking` caveat at the abort site); IMAP cancellation unit tests (mid-folder / per-chunk); calendar cancellation unit tests against `calendar_sync_account_impl` with stub Google/Graph/CalDAV providers; `cancel_and_await_cancels_calendar` integration test; `pending_calendars` map mirror tests (sync side has three at `service_client.rs`; calendar side has none). None depend on the fake-CalDAV fixture - they are shape/lifecycle tests dropped for time. Carries forward as Phase-5 cleanup, not Phase 8 fixture work.
- **Gmail and Graph email-sync cancellation is shallow at the entry point only** (Phase 3 retrospective discovered during Phase 5 review). Both providers carry a `cancellation_token` field marked dead (`crates/gmail/src/sync/mod.rs`, `crates/graph/src/sync/mod.rs`) and only check it at entry; long loops continue and can return `Ok`, which Service maps to `Completed` rather than `Cancelled`. Phase 5's CalDAV/Google/Graph *calendar*-sync cancellation now goes deeper than the email-sync paths Phase 3 claimed complete. Worth a Phase 3 retrospective check before next phase.

---

## Phase 6 - Remaining UI write surfaces relocate; full Service-only-writer invariant lands

**Goal.** Every remaining UI-side write path enumerated in the problem-statement inventory moves across the boundary. The type-level read/write split becomes globally compile-enforced: `WriteDbState` (and the write halves of all four state types) is unreachable from the `app` crate. `attachment.fetch` IPC + blob-store eviction/GC land here too, since the blob-store writer (relocated with sync in Phase 3) is already Service-side.

The phase splits into 6a + 6b + 6c at plan-doc level. 6a is the long tail of small, mechanical write surfaces (`docs/service/phase-6a-plan.md`). 6b is OAuth two-step + the attachment cache-miss path + eviction/GC + global write-half lockdown (`docs/service/phase-6b-plan.md`). 6c is calendar event mutations - the genuinely tricky one because series-vs-occurrence + RSVP semantics need their own wire-format design that does not fit the flat `MailOperation`-style list (carved out into its own future plan; not started until 6b lands).

**Entry criteria.**
- Phase 5 landed (all sync runs Service-side; the blob-store writer is already relocated as a sync dependency).
- Attachments roadmap Phase 1a + 1b landed.
- The `oauth.refresh_request` temporary IPC from Phase 4 is in place.

**Phase 6a - small mechanical write surfaces + encryption-key handle.** Plan: `docs/service/phase-6a-plan.md`. **LANDED** (2026-05-06; see commits tagged `phase 6a` and `phase 6a-part-2`).

**Phase 6b - OAuth two-step + `attachment.fetch` + global write-half lockdown.** Plan: `docs/service/phase-6b-plan.md`. **LANDED** (2026-05-06; see commits tagged `phase 6b`). Transitive `cargo metadata` lockdown check (the `app -> cal -> service-state` path) defers to Phase 6c when `cal::actions::*` relocates Service-side and `cal` drops out of `app/Cargo.toml`. Pack-aware revision pass (lease IDs, frame-orphan / repack handler) defers to Phase 1a when the pack store lands.
- Preferences (`prefs.set`).
- Account create / update / delete / reorder (non-OAuth path - `account.create` accepts already-encrypted credential bytes; OAuth two-step is 6b).
- Signature CRUD + reorder.
- Local draft auto-save (`draft.save`) - with explicit ordering against `iced::exit()` on window close: UI emits one synchronous `draft.save` per dirty editor before issuing `service.shutdown`, with a 500 ms per-draft ack ceiling.
- Pinned searches (CRUD + Service-side `pinned_search.kick` for the expire-stale cadence).
- Contacts / groups CRUD.
- Attachment collapse-state preference.
- Calendar visibility toggle (the flat-boolean half of `db/calendar.rs`; event mutations are 6c).
- **Encryption-key handle.** Phase 2 carry-forward 19d. Default option: handle-based. Service holds raw bytes; UI calls `internal.encrypt_for_storage { plaintext } -> ciphertext` per credential persist. Survey of credential-persist call sites confirms no hot path - all run at human-paced cadence (account create, OAuth token persist becomes a no-op after 6b's relocation, password persist).
- **`docs/architecture.md` rewrite** - the doc has not been touched since before Phase 4 and needs the Phase 5 + 6a deltas. Lands as the final commit of 6a.

**Phase 6b - OAuth two-step + `attachment.fetch` + eviction/GC + global lockdown.** Plan: `docs/service/phase-6b-plan.md`.
- **OAuth two-step coordination.** UI captures the redirect (it's the visible app); ships the auth code to Service via `oauth.exchange_code` IPC; Service exchanges + persists in one transaction. The temporary `oauth.refresh_request` from Phase 4 deletes itself; Service refreshes its own tokens. The auth code is a one-shot bearer credential and wraps in the existing redacting type.
- **`attachment.fetch` IPC** for cache-miss reads. Returns `{ content_hash, size, pack_path, offset, length }` not `Vec<u8>` (per backpressure policy); UI re-reads positionally from the pack file at the offset+length window.
- **Eviction policy + GC.** `PackRuntime` mirrors `CalendarRuntime`'s shape (per-account map, panic supervisor, kick handler). LRU + 5 GB size cap default. GC drops blobs whose `messages` rows are gone. Two kicks: `pack.eviction_kick` (5-min cadence with 1 h staleness gate) + `pack.gc_kick` (5-min cadence with 24 h staleness gate).
- **Cross-store invariant pass extends** to include pack-file orphan sweep + index reconciliation, plus marker-file recovery for half-finished account deletions.
- **Global write-half lockdown.** `service-state` constructors become `pub(crate)`. `crates/app/Cargo.toml` drops the `service-state` dependency. CI script enforces the absence of the dependency. `WriteDbState`, `BodyStoreWriteState`, `InlineImageStoreWriteState`, `SearchWriteHandle` constructors unreachable from the `app` crate at compile time.

**Phase 6c - calendar event mutations.** Plan: `docs/service/phase-6c-plan.md` (best-effort first draft, revision pass scheduled after 6b lands). Three flat operations exist today (create / update / delete via `cal::actions::*`), no RSVP, no series-vs-occurrence semantics. 6c relocates the existing surface: typed `CalendarOperation` enum mirroring `MailOperation`, separate `cal_action.execute_plan` IPC (keeps each pipeline's exhaustive-match discipline clean), shared `pending_ops` journal with a `kind` discriminator. The `ActionContext::db` field flips from `&ReadDbState` to `&WriteDbState`; the lock-dance escape pattern goes away with the flip. RSVP and series-vs-occurrence semantics are explicitly out of scope and tracked as future-Phase-6d work; if either lands before 6c starts, the plan needs a revision pass first.

**Out of scope.**
- Settings UI changes for attachment caching policy (attachments Phase 4, lives UI-side; just makes IPC calls).
- Calendar attachments (separate work).
- Provider-specific OAuth quirks - the IPC is provider-neutral; per-provider handling stays in the provider crates' OAuth helpers.

**Touchpoints.**
- `crates/service-api/` - new typed-request modules: `prefs`, `account`, `signature`, `draft`, `pinned_search`, `contacts`, `internal` (6a) + `oauth`, `attachment`, `pack` (6b).
- `crates/service/src/handlers/` - matching handler modules.
- `crates/service/src/oauth/refresh.rs` (6b) - per-provider refresh helpers.
- `crates/service/src/pack.rs` (6b) - `PackRuntime`.
- `crates/app/src/handlers/...` - replace direct DB writes with service-client calls. Type system catches anything missed.
- `crates/app/src/app.rs::from_boot_ready` - drop the `rtsk::load_encryption_key` call (6a); credential encrypt/decrypt routes through `internal.encrypt_for_storage` / `internal.decrypt_for_storage`.
- `crates/service/src/startup_invariants.rs` - extend with pack-file pass (6b).
- `docs/architecture.md` - 6a rewrite + 6b delta.

**Exit criteria** (6a + 6b combined; 6c lands separately).
- `git grep` for `with_write_conn` in `crates/app/src/` returns only the calendar event mutation sites (Phase 6c).
- The `WriteDbState` constructor is unreachable from any UI call site (compile-enforced via crate boundary).
- Write halves of body / inline-image / search states unreachable from UI.
- Cache-miss Open / Save calls succeed via IPC.
- OAuth refresh runs Service-side; the temporary `oauth.refresh_request` is gone.
- UI no longer re-reads `ratatoskr.key`; encryption-key access flows through the Service-owned handle (per Phase 2 carry-forward).
- `docs/architecture.md` reflects the post-Phase-6b state.

**Risks / open questions.**
- Tombstone visibility across processes: Service tombstones a blob, UI tries to read it before the index commit propagates. Service holds the write lock; UI reads see the post-commit state via SQLite WAL - verify with a stress test.
- Concurrent reads of the currently-being-written-to pack must never read past the last fsync'd offset. The pack store API enforces this; verify it survives the IPC boundary.
- OAuth coordination introduces a UI-Service round-trip during the redirect window. OAuth servers expect the redirect-to-token-exchange roundtrip in seconds, not minutes. Phase 6b plan picks bounded request-queue depth + reject-with-`Busy` over a priority dispatch lane; if measurement shows OAuth contention, the priority-lane refactor lands as a follow-up.
- Local draft save vs `iced::exit()` race (6a): UI emits synchronous `draft.save` per dirty editor before `service.shutdown`, 500 ms per-draft ack ceiling. Settled in `phase-6a-plan.md` § Scope.
- Calendar wire format (6c): do not assume `MailOperation` shape. Series/occurrence/RSVP needs its own type. Plan-doc deferred to after 6b lands.

---

## Phase 7 - Attachment text extraction + Tantivy indexing

**Goal.** The forcing function. Cached attachments get text-extracted (per mime-type extractors) and indexed into Tantivy. Search results disambiguate "matched in body" vs. "matched in attachment X." Layers on top of the already-Service-side Tantivy writer (relocated in Phase 3).

**Entry criteria.**
- Phase 3 landed (Tantivy writer + blob-store writer are Service-side; cached attachments exist as a sync side-effect).
- Phase 6 landed (`attachment.fetch` IPC + blob-store eviction/GC).

**In scope.**
- `crates/service/src/text_extract/` - per-mime extractor dispatch. Initial extractors: PDF (Rust crate TBD - `pdf-extract` for v1, with explicit "best effort, skip the weird ones" caveat; `pdfium-render` or `mupdf-rs` evaluated as later upgrades), OOXML (`.docx`/`.xlsx`/`.pptx` - zip + xml text extraction), plain text. Skip lists for opaque binaries (mp4, zip, exe, etc.).
- Pipeline: pre-fetch -> extract -> add to Tantivy doc with `attachment_*` field tags -> commit batched.
- Tantivy schema migration: add `attachment_text`, `attachment_filename`, `attachment_mime` fields.
- Re-index command (`index.rebuild`) for one-shot full re-extraction. Multi-hour acceptable; reports progress via notification.
- Search results carry "match in attachment" annotations.

**Out of scope.**
- OCR for scanned PDFs (substantial separate work).
- Language detection / per-language analyzers (defer until users complain).
- Attachment preview rendering (still out of scope per the attachments problem statement).

**Touchpoints.**
- New: `crates/service/src/text_extract/{mod,pdf,ooxml,plain}.rs`.
- `crates/service/src/handlers/attachment.rs` - extraction triggered after `BlobStore::put`.
- Tantivy schema migration: add `attachment_text` field, optionally `attachment_filename`, `attachment_mime`.
- `crates/search/...` - reader-side adjustments to surface attachment matches.
- `crates/app/src/ui/...` - search result rendering shows the "match in attachment X" annotation.

**Exit criteria.**
- A search query for a phrase known to be inside a cached PDF returns the parent message with an "attachment match" annotation.
- A re-index of an existing mailbox completes successfully (UI must stay open; visible progress).
- Indexing CPU stays Service-side; UI rendering remains responsive during heavy indexing.

**Risks / open questions.**
- PDF extraction crate choice. `pdf-extract` exists but is incomplete; `pdfium-render` requires shipping pdfium binary; `mupdf-rs` ditto. May need to settle for "good for most PDFs, skip the weird ones" in v1.
- Tantivy commit cadence under indexing pressure - too frequent slows things down, too rare loses recent work on crash. Probably commit every N docs or M minutes.
- Indexing memory footprint for very large attachments (extracting a 200 MB PDF). Need a streaming-ish approach or a hard skip threshold.
- PDF/OOXML extractors must run in `tokio::task::spawn_blocking` rather than directly on the runtime. The dispatch loop relies on no task blocking the runtime; an extractor that monopolizes a worker thread could starve heartbeat acks.
- If extractors spawn subprocesses (some PDF tooling does), parent-death must compose. Windows: the Job Object inheritance from Phase 1 handles it for free. Linux: every extractor-spawn site must apply the same `pre_exec` PR_SET_PDEATHSIG + post-prctl `getppid() == 1` recheck the Service uses, otherwise the cascade breaks at the Service-extractor boundary. Phase 7's extractor-spawn helper enforces this as a contract, not a per-call-site choice.

---

## Phase 8 - Crash recovery polish + cross-store invariant pass optimization

**Goal.** The Service surviving / failing / being respawned is fully handled (Phase 1.5 had the minimal version). UI shows visible state when the Service is restarting; queued work is preserved across a Service crash. The cross-store invariant pass (which already exists in minimal form from Phase 3 and Phase 6) gets optimized for large mailboxes.

**Entry criteria.**
- Phases 1-7 landed. Real crashes are happening (or being induced) so we know what hurts.
- Phase 3 minimal cross-store pass + Phase 6 blob-store extension already in place; this phase optimizes them.

**In scope.**
- Respawn with exponential backoff (Phase 1.5 was no-backoff). Replaces the Phase 1.5 fixed 1-second cooldown + sliding-window crashloop guard. The "duplicate boot work on respawn" cost (each respawn re-runs the entire boot sequence including `reconcile_velo_rename`) is acceptable today under the 1-second cooldown but amplifies the lockfile race under a tight crashloop; backoff + crashloop detection together remove the amplification.
- Crashloop detection: if respawn fails N times in M seconds, surface a permanent error state in the UI ("Service can't start - check logs"). Phase 1.5's `CRASHLOOP_THRESHOLD = 3` / `CRASHLOOP_WINDOW = 30s` flat policy gets replaced.
- UI status indicator for Service health (small banner or status bar element). Indicator distinguishes "respawning" from "persistently failing" from "healthy."
- In-flight requests are either (a) replayed if idempotent, (b) failed back to the caller with a clear error if not. Per-method idempotency contract recorded in `service-api`.
- Persistence of the retry queue across Service restarts (already on disk in `pending_ops` table; verify).
- ~~**Race `spawn_inner`'s `health.ping` against direct child-exit observation.**~~ **Landed early during Phase 1.6** (commit pending). The Phase 8 carry-forward got pulled forward when the no-key test cohort started flaking under parallel-test scheduling pressure during Phase 1.6 work, and the projected count of similarly-shaped real-subprocess tests in Phases 2-7 (~15+ across `pre_ack_crash_rolls_back_subprocess`, `compose_send_50mb_attachment`, `bulk_archive_200_threads_under_budget`, etc.) made deferral untenable. Implementation: `ServiceClient::observe_child_exit` polls `Child::try_wait` on a 50 ms interval; `ServiceClient::request_or_observe_child_exit` `tokio::select!`s the request future against the observer; both `spawn_inner` (`health.ping`, 5 s ceiling) and `run_spawn_flow` (`boot.ready`, 600 s ceiling) use it. On child-exit-first, jumps straight to `elevate_initial_boot_error` without waiting for the timeout ceiling. The two flaky tests (`terminal_failure_at_initial_boot_does_not_respawn`, `spawn_with_events_emits_terminal_on_missing_key`) now resolve in milliseconds rather than 5+ seconds. Phase 1.5's two pragmatic mitigations (concurrent `tokio::join!` of abort handles in elevate, `wait_with_kill_watchdog` shrunk to 1 s) remain in place as belt-and-suspenders.
- **Unify the Drop watchdog and `wait_with_kill_watchdog` escalation policies.** Phase 1.5 ships with two kill-escalation paths whose budgets are intentionally different (`Drop` / `async_drop_wait` is the user-quit path with ~1.7 s patience: 200 ms abort + 1 s wait + SIGKILL + 500 ms poll; `wait_with_kill_watchdog` is the respawn path with ~6 s: 5 s wait + start_kill + 1 s wait). The Phase 8 refactor extracts a shared helper that takes the budget shape as a parameter, with a doc-comment naming why each call site picks its budget; loses the rationale-drift risk without losing the distinction. Today's split has a "worth a contract comment" note that this absorbs.
- **Crashloop guard test coverage for evicted-success entries.** Phase 1.5 carry-forward, flagged by `arch` review (claude). `crates/app/src/service_client.rs::record_respawn_and_check_crashloop` is unit-tested for the threshold-trip case but not for "3 crashes -> 3 successful recoveries -> 3 more crashes within window should NOT trip" - successful respawns leave entries in the deque and add to the count. Phase 8 replaces the sliding-window guard with exponential backoff anyway, so the right test lands against the new shape rather than the Phase 1.5 placeholder.
- **Soft-cancel signal for `boot.ready` to avoid mid-COMMIT SIGKILL.** Phase 1.5 carry-forward, flagged by `bugs` review (claude). `crates/service/src/dispatch.rs:185-208` orders `drain_in_flight` before `boot_handle.abort()`; a `boot.ready` parked on `wait_for_ready` keeps drain awaiting until the spawn_blocking migration completes. UI Drop's `wait_with_kill_watchdog` is 1 s, then SIGKILL fires on a Service mid-`COMMIT` (rare; depends on commit timing). SQLite WAL recovers and the next boot redoes the migration - the same "duplicate boot work on respawn" cost the exponential-backoff bullet flags, but worth weighing whether `boot.ready` should respect a soft-cancel signal so the Drop watchdog doesn't escalate at all on big migrations.
- **Two `service_subprocess` tests still flaky.** `service_subprocess_ping_and_shutdown` and `spawn_with_events_emits_terminal_on_missing_key` both `#[ignore]`'d in Phase 2 (`crates/app/tests/service_subprocess.rs`) after `service_subprocess_ping_and_shutdown` hung two test runs back-to-back. The earlier Phase-1.6 fix (`request_or_observe_child_exit` racing the request future against `Child::try_wait`) was thought to close the deadlock for both, but at least one of them resurfaced. Phase 8 owns root-causing the remaining flakiness and re-enabling. Likely candidates: writer-task drain ordering vs `MustDeliver` send (the reverted Phase 2 task 23 attempt introduced an awaited send that didn't exist in Phase 1.5 - see the next bullet), `notifications_in_flight` JoinSet drain in `dispatch.rs::run_service_with_io_and_lifecycle`, or stdout-pipe backpressure interacting with the writer task's exit.
- **Class-aware `boot_progress::emit` helper.** Phase 2 carry-forward (originally task 23). The first attempt to make the helper pick `try_send` for `Coalesce`/`Drop` and awaited `send` for `MustDeliver` introduced a hang in the `service_subprocess` test cohort and was reverted; today's helper still uses `try_send` only, which is structurally incompatible with `MustDeliver` semantics. The contract noted in `crates/service/src/boot_progress.rs` ("`OUTBOUND_QUEUE_CAP=1024` must remain >> Phase-1.5 boot frame count") is doc-only - no per-phase regression test bounds total emit count for any new emitter. Phase 8 owns re-attempting the helper *after* root-causing the flaky-test deadlock above (the two are entangled - the same writer-task drain-ordering issue is the most likely cause of the reverted helper's hang). Either ship the class-aware helper with a fix for the underlying drain bug, or replace the contract with per-emitter regression tests bounding emit count per boot phase. Coalesce-class `try_send` remains correct for `boot.progress` / `sync.progress` regardless.
- **`from_boot_ready` async store init.** Phase 2 carry-forward (originally task 21). Body / inline / search store init still runs synchronously inside `crates/app/src/app.rs::from_boot_ready` after the `boot.ready` handshake returns. On a slow disk this momentarily blocks the splash transition with a frozen view; less load-bearing now that `ActionContext` is Service-side, but the splash-blocking moment remains. Pure UI surgery: relocate the body / inline / search store init to async tasks dispatched from `BootingApp::update`; the `Booting -> Ready` transition fires earlier (right after `BootReady`); async store-init tasks fire `Message::ReadyStoreReady(...)` events that finalize the `ReadyApp` field set incrementally. Lands in Phase 8 because by then the crash-recovery polish already touches the boot path and the state-machine reshape is cheaper alongside that work; can also slot into any earlier phase if a UI engineer happens to be in the area.
- **`SchemaVersionChanged` end-to-end test (`--test-fake-schema=N`).** Phase 2 carry-forward (originally task 20). The mismatch path in `service_client.rs::respawn` is unit-tested via the `Display` contract, but the full subprocess path - where the schema actually changes across a respawn - is not. Phase 2 introduced real schema-version sensitivity (the action worker depends on the schema being what the UI thinks it is); a real-subprocess test that flips the value across SIGKILL and asserts `ClientError::SchemaVersionChanged` arrives via Terminal would catch a regression that today only surfaces on a live mid-deploy upgrade. Add a `--test-fake-schema=N` flag analogous to the existing `--test-fake-version`, then a `crates/app/tests/service_subprocess.rs::test_fake_schema_propagates_via_terminal` that drives the path. Lands in Phase 8 alongside the rest of the crash-recovery / respawn test polish; can slot earlier if a real schema-version regression surfaces in the wild.
- **Phase 2 plan-specified integration tests (T1).** Phase 2 carry-forward (originally tracked in a `discrepancies.md` companion that has been retired into this entry). The plan's `phase-2-plan.md` § "Integration tests (in-process)" + "Real-subprocess smoke tests" names a cohort - `journal_replays_after_respawn`, `post_ack_crash_does_not_roll_back` / `post_ack_crash_replays_subprocess`, `pre_ack_crash_rolls_back_subprocess`, `mark_chat_read_emits_only_action_completed`, `action_skips_search_index_write`, `compose_send_50mb_attachment` / `send_wire_attachment_validation` / `send_wire_oversize_payload_handler_path`, `handler_does_not_drive_batch_execute`, `stale_outcomes_dropped_after_respawn` - that did not land alongside Phase 2. Foundational unit tests did (`recover_stale_leases_resets_active_jobs_and_ops` + idempotent variant for B1's SQL; `unfinalized_mail_plan_jobs_finds_orphans_after_partial_finalize` + `_skips_send_jobs` + `_handles_leased_status` for B4's finalize-on-drain helper; `insert_quiet_job_rejects_unknown_account_id` for B5's FK constraint; `mail_side_mirror_is_exhaustive` for the bidirectional `MailOperation` ↔ `WireMailOperation` mirror), but the end-to-end behavioral path (kill Service mid-execute, respawn, observe journal replay; submit a plan, observe per-op outcomes streaming, observe final `ActionCompleted`) was not exercised - blockers fixed during close-out were validated by reading code paths, not by running them. Today's in-process harness lacks account seeding (the FK constraint requires real `accounts(id)` rows), a "shut down then respawn against the same data dir" pattern, and action-notification reading. Lands here because Phase 8's flaky-test root-cause and class-aware-emit re-attempt are already going to reshape the harness; building T1 against today's framework would mean rebuilding it once that work lands.
- **Cross-store invariant pass optimization.** Replace the Phase 3 / Phase 6 full-table scans with marker-file gating: track a "last clean shutdown" marker per store; scan only what's been written since. Bounded to N seconds on a 200 GB mailbox via per-store cursors.
- **Tantivy orphan iteration in the invariant pass.** Phase 3 carry-forward. The Phase 3 invariant pass clears `history_id` per dirty account and drops body / inline orphans, but Tantivy orphan iteration was deferred - the cursor-clear + next-sync re-index repopulates correctness without it, but unreferenced docs accumulate until then. Add the Tantivy scan (per dirty account: iterate index, drop docs whose `message_id` is no longer in `messages`) alongside the marker-file gating work above so they share the same per-account scan loop. Lands in `crates/service/src/startup_invariants.rs`.
- **PushRuntime integration test cohort.** Phase 4 carry-forward. The Phase 4 plan called for unit / integration / real-subprocess tests covering provider-gating against a seeded DB, bridge-task debounce + sync-kick + notification emission, drain-order-holds-under-shutdown, and account-delete cancel-before-sync. Landing the cohort needs either (a) a fake JMAP WebSocket server fixture (the bridge body needs a real-shaped StateChange producer to exercise debounce + kick), or (b) `test_dummy` constructors on `BodyStoreWriteState` / `InlineImageStoreWriteState` / `SearchWriteHandle` (which don't exist - SyncRuntime today has no in-memory test path either). Both paths are non-trivial infrastructure work that would dwarf Phase 4's behavioral surface; Phase 8 is the right home because the flaky `service_subprocess` tests already on Phase 8's plate need the same fixture work to be re-enabled. The PushEvent wire/class guarantees are covered today by the service-api catalog tests at `crates/service-api/src/notification.rs:469-585`.
- **JMAP push re-auth re-arm.** Phase 4 carry-forward. UI-side re-auth (`AddAccountWizard::new_reauth` at `crates/app/src/handlers/accounts.rs:53`) updates the existing account row in place and does NOT trigger `PushRuntime::start_account`. So a JMAP token-revocation kills push for that account until Service restart, even after the user re-authorizes - the dead `PushRuntime` entry has no path to re-arm. Phase 8 wires push re-arm to a token-refresh-success event (or to a UI-side `account.reauthorized { account_id }` IPC, depending on whether re-auth itself relocates Service-side in Phase 6). Manual workaround in Phase 4: restart the Service. Lands in `crates/service/src/push.rs::PushRuntime` plus an event emission point in the OAuth refresh path.
- **JMAP push state hardening.** Phase 4 carry-forward (subsumes / supersedes the prior "JMAP push state resume" bullet from the original Phase 3 deferral pass). Phase 4 inherits today's behavior: `jmap::push::start_push` unconditionally loads `jmap_push_state.push_state` and sends it in `WebSocketPushEnable`. On crash, Phase 3's invariant pass clears `history_id` so a stale `StateChange` resolves correctly via re-fetch; the resume path is therefore correctness-preserving. Phase 8 hardens it: detect crashed accounts (via the same Phase 3 sync-marker signal) and force a fresh-start by clearing `push_state` before calling `start_push`. Adds an explicit fresh-start knob on `start_push` rather than a pre-call `save_push_disabled` workaround. Strict optimization; correctness preserved without it. Lands in `crates/service/src/push.rs::PushRuntime::start_account`.
- **Account-deletion `is_deleting` gate.** Phase 3 carry-forward. The plan called for an `accounts.is_deleting` schema column + UI-side `SyncTick` filter (skip deleting accounts) + Service-side defense-in-depth check in `SyncRuntime::start_account`. The load-bearing `cancel_and_await` flow shipped without it, so a `SyncTick` firing between the cancel-ack and the row-delete can re-kick a sync against the disappearing account; the cancel races the start, and either the new run gets the cancel (correct outcome) or runs to completion against a half-deleted account (briefly inconsistent until the row delete finalizes). Add the column + both gates so the deletion flow is monotonic. Schema change goes in `crates/db/src/db/schema/01_core.sql`; UI gate in the `SyncTick` account-list filter; Service gate in `SyncRuntime::start_account`.
- Heartbeat policy refinement: distinguish "dispatch loop alive" from "no progress on a long-running task." Generous timeouts on first heartbeat after a sync starts; require N consecutive misses before respawning rather than 1.

**Out of scope.**
- Hot-restart / live state migration of the Service. Crash + cold restart is the model.

**Touchpoints.**
- `crates/app/src/service_client.rs` - backoff + crashloop detection + status reporting + heartbeat policy refinement.
- `crates/app/src/ui/status_bar.rs` - "Service degraded" indicator.
- `crates/service/src/startup_invariants.rs` - extend with marker-file gating + bounded windows; the minimal pass scaffolding already exists from Phases 3 and 6.
- `crates/service/src/boot_progress.rs` - re-attempt the class-aware emit helper (after the flaky-test root-cause lands).
- `crates/app/src/app.rs` - relocate body / inline / search store init to async tasks dispatched from `BootingApp::update`; new `Message::ReadyStoreReady` arms.
- `crates/service/src/main.rs` (or wherever `--test-fake-version` lives) - add `--test-fake-schema=N`; `crates/app/tests/service_subprocess.rs` - new `test_fake_schema_propagates_via_terminal`.
- `crates/app/tests/` and `crates/service/tests/` harness helpers - account seeding (real `accounts(id)` rows for FK-constrained writes), "shut down then respawn against the same data dir" pattern, action-notification reading. Required before T1's behavioral cohort can land.

**Exit criteria.**
- Killing the Service mid-sync results in a respawn within a few seconds (Phase 1.5 already), backoff prevents tight crashloops (new), status indicator surfaces the degraded state (new).
- A persistently failing Service surfaces a clear UI error rather than silent breakage.
- Startup invariant pass runs in <5s on a typical mailbox; <30s on a 200 GB mailbox. Logged stats let us see how often crashes leave us reconciling.
- Heartbeat false-positive rate (load-induced miss interpreted as crash) goes to zero.
- Both `#[ignore]`'d `service_subprocess` tests (`service_subprocess_ping_and_shutdown`, `spawn_with_events_emits_terminal_on_missing_key`) re-enabled and stable.
- Class-aware `boot_progress::emit` helper either re-landed (with the underlying drain bug fixed) or replaced with per-emitter regression tests bounding emit count.
- `--test-fake-schema=N` flag exists; the `SchemaVersionChanged`-via-Terminal e2e test passes.
- Phase 2 plan-specified integration test cohort (T1) exists and runs green: `journal_replays_after_respawn`, `post_ack_crash_does_not_roll_back` / `post_ack_crash_replays_subprocess`, `pre_ack_crash_rolls_back_subprocess`, `mark_chat_read_emits_only_action_completed`, `action_skips_search_index_write`, `compose_send_50mb_attachment` / `send_wire_attachment_validation` / `send_wire_oversize_payload_handler_path`, `handler_does_not_drive_batch_execute`, `stale_outcomes_dropped_after_respawn`. Harness helpers (account seeding, same-data-dir respawn, action-notification reading) land first.
- Splash transition stays responsive on a slow-disk machine (async store init landed; `from_boot_ready` no longer blocks on body / inline / search opens).

**Risks / open questions.**
- Distinguishing "Service crashed" from "Service is just slow under load" in the heartbeat. The N-consecutive-misses policy plus generous first-heartbeat-after-sync timeout should resolve this; tune in the planning session.
- Marker-file format and atomicity. Prefer a small per-store SQLite row over a flat file; SQLite gives atomicity for free.

---

## Phase 9 (optional) - Tray-resident promotion

**Goal.** Closing the UI window doesn't quit the app or kill the Service. Tray icon offers reopen / quit. Push notifications continue to run when the window is closed.

**Entry criteria.**
- Phases 1-8 landed and running well in real use.
- Demand exists from users for "background sync without keeping a window open."

**In scope.**
- Cross-platform tray icon (probably `tray-icon` crate or iced's tray support if available by then).
- "Close button minimizes to tray" preference (off by default; users opt in).
- Tray menu: Open, Quit, possibly Compose.
- The Service lifecycle stays exactly the same - it's still a child of the UI process. The UI process just doesn't exit when the window closes.

**Out of scope.**
- True system-daemon mode. Still rejected.
- Auto-start at user-session login - separate optional follow-up.
- Native OS notification toasts (e.g. for new mail). Separate work.

**Touchpoints.**
- New: `crates/app/src/tray.rs`.
- `crates/app/src/app.rs` - lifecycle changes around window close.

**Exit criteria.**
- App can be configured to minimize-on-close.
- Push notifications continue with window closed; reopening is fast (Service was already running).

**Risks / open questions.**
- Cross-platform tray APIs are uneven; `tray-icon` is the most established Rust crate but has its quirks.
- Quit-vs-minimize disambiguation is a known UX trap.

---

## Out of phases (deliberately deferred)

- **Full system daemon mode** (systemd unit / launchd / Windows Service). Explicit non-goal.
- **Multi-UI** (multiple windows of the app sharing one Service). Conceivable; not a target.
- **OS notification toasts** for new mail / completed actions. Separate work; depends on platform APIs.
- **Schema versioning of the IPC protocol.** UI and Service ship as a tightly coupled pair. If we ever want to support cross-version, that's its own design exercise.
- **Service-as-library** for embedding in other apps. The Service is a Ratatoskr internal; not a reusable building block.
