# Phase 1.5 Review Discrepancies

Findings from `review arch,bugs --oneshot` that remain open after the resolution pass. Items that have been addressed have been removed per CLAUDE.md's "current gaps only" policy. Most items below are explicit Phase 2 / Phase 8 carry-forwards.

## Phase 2 carry-forwards

### `from_boot_ready` re-loads the encryption key the Service already validated

`crates/app/src/app.rs:181, :241`. Plan item 13 said `crate::DB` was the only direct OnceLock site; that's true, but `from_boot_ready` then opens its own `Db` (`:182`), re-reads the key file via `rtsk::load_encryption_key` (`:241`), inits body / inline-image / search state (`:212-246`), builds `action_ctx` (`:248-264`), and decrypts bootstrap snapshots (`:279-286`). Already on the Phase 2 carry-forward in `implementation-roadmap.md`; what this entry adds is the elevated severity of the duplicate key load. The Service has applied TOCTOU-safe permission repair via `fchmod` on the open fd; the UI re-runs the same TOCTOU check. The key file changing between the two reads (permission race, hostile rewrite, FS glitch) lands the UI on a different key than the Service holds. Treat the IPC plumb-through of `BootContext::encryption_key` as a Phase 2 hard requirement, not a nice-to-have.

### Action dispatch does not bump generation counters before submission

`crates/app/src/handlers/commands.rs:175`, `crates/app/src/update.rs:33`, `crates/app/src/action_resolve.rs:692`. `dispatch_plan` sends the plan to `batch_execute` without bumping `nav_generation` or `thread_generation`. Some actions optimistically mutate visible thread state before dispatch; an older `ThreadsLoaded` or `NavigationLoaded` result can land after the mutation and overwrite it with a stale snapshot. The Phase 1.5 plan already documents "generation counters bump pre-dispatch on plan submission" as Phase 2 architecture (see `phase-1.5-plan.md` / `implementation-roadmap.md` Phase 2). Carry into the Phase 2 plan as an explicit checklist item.

### `crates/app` depends on `service` for `parent_death::ProcessGuard`

`crates/app/src/service_client.rs:1, :103, :357, :1190, :1210`. App -> Service even though Service is conceptually the embedded child. Practically zero cost in the single-binary build; aesthetically wrong. Flag for the Phase 2 `service-state` split since that's the next time the dep graph gets surgery. A `process-lifetime` micro-crate consumed by both is one option.

### `boot_progress::emit` silent drop without a per-phase regression test

`crates/service/src/boot_progress.rs:36-55`. Doc-comment names the contract ("OUTBOUND_QUEUE_CAP=1024 must remain >> Phase-1.5 boot frame count") but no compile-time or test-time enforcement. The integration test at `tests/dispatch_in_process.rs:516-584` asserts ordering against a single-migration fresh DB (one Migrating frame). Phase 2's plan should require: any new boot phase / chatty `MustDeliver` notification ships with a regression test bounding total emit count, or a class-aware emit helper that picks `try_send` vs `send` based on `NotificationClass`.

### `BootSharedState` parking under flood

`crates/service/src/dispatch.rs:62, :320-330` + `crates/service/src/handlers/boot.rs:23-31`. `boot.ready` bypasses `bypasses_admission()` for both the per-handler semaphore and the dispatch-loop `ADMISSION_CAP`, but is still spawned into `handlers_in_flight: JoinSet<()>` and parks on `BootSharedState::wait_for_ready`. Two consequences:

- **Shutdown ordering on a long migration is "wait out the migration"**, not "abort and exit." `drain_in_flight` runs before `boot_handle.abort()` (`:190-208`); a `Shutdown` arriving during a 5-minute migration parks `drain_in_flight` until boot signals readiness. The `Shutdown`'s 30 s IPC timeout means the realistic path is "UI escalates to SIGTERM, sentinel still gets written via Linux SIGTERM handler, but the `flushed_ok=true` ack the UI was waiting for never arrives." Worth a doc-comment naming SIGTERM-escalation as the canonical path for shutdown-during-migration.
- **The `boot.ready` slot is unbounded under a flood.** A misbehaving UI loop could re-issue `boot.ready` and each call would park on the same `Notify` and consume a `JoinSet` entry until boot completes. Cheap fix: track an `AtomicBool` "already in flight" and have subsequent callers either join the existing one or fail fast. Not blocking for v1.

## Phase 8 carry-forwards

### Crashloop guard counts even successful respawns

`crates/app/src/service_client.rs:1400-1414, :1972-2006`. Documented Phase 1.5 behavior ("3 respawns within 30 s = stop") so behaviorally correct. Tests cover the threshold-trip path; nothing covers entries evicted by an explicit success. Phase 8's exponential backoff replaces this - no test worth writing now; flag for the Phase 8 plan.

### Migration-mid-COMMIT SIGKILL via Drop watchdog redoes the entire migration

`crates/service/src/dispatch.rs:185-208`. `drain_in_flight` runs before `boot_handle.abort()`; a `boot.ready` handler parked on `wait_for_ready` keeps drain awaiting until the spawn_blocking migration completes. UI Drop's `wait_with_kill_watchdog` is 1s, then SIGKILL fires on a Service mid-`COMMIT` (rare; depends on commit timing). SQLite WAL recovers; next boot redoes the migration. Documented trade-off. Phase 8's exponential-backoff and respawn polish should weigh whether `boot.ready` should respect a soft-cancel signal.

## Open question

### `BootExitCode::HandshakeFailure = 70` is reserved but unreachable in Phase 1.5

`crates/service-api/src/boot.rs:11-14`. Doc-comment says "Reserved; not emitted by Phase 1.5." Either add a runtime test that asserts no Service code path triggers it (so a future regression that wires it up gets noticed), or delete the variant and re-add when Phase 2+ has a real use site. Wire-stable contracts for unused codes drift over time.

## Architecture-direction observations (no action; just naming what's settled)

- **The action-mutation-gate invariant survives Phase 1.5.** Service holds `db_conn` + `encryption_key` in `BootContext`; action service stays UI-side. Phase 2 consumes both fields cleanly.
- **`service_generation` extends generation-counters across the process boundary** without using the project's branded `GenerationCounter<T>`. Right call (cross-process can't carry a phantom type) but a soft inconsistency. Either build a wire-shaped generation type with the same enforcement story, or carve out an exception in `docs/architecture.md` § Settled Patterns. Resolution pass added a `WithGeneration` trait on payload structs and adjacent `service_generation` / `set_service_generation` methods on `Notification`, but the architecture-level naming exception is still worth recording.
- **Two-phase spawn + state-machine `App` is a clean split.** The `BootingApp` whitelist enforced via the audit table in `crates/app/src/message.rs` is the kind of compile-time-adjacent contract the project favors.
- **`crypto-key` is a good model for future cross-crate primitives.** Dep-free, zeroizing wrapper, TOCTOU-safe permission repair, file-owner UID validation, release-build all-zero hard-fail - all in one place.
