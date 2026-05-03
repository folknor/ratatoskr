# The Service - Phase 1.5 Plan: Boot ownership + load-bearing Service

Companion to `phase-1-plan.md`. Implements Phase 1.5 of `implementation-roadmap.md`.

## Context

Phase 1 landed a Service that does nothing except answer `health.ping` and respond to `shutdown`. It can be killed without consequence; the UI hosts schema migrations, the encryption key, pending-ops recovery, and every write surface that exists today. Phase 1.5 flips the dependency: the UI cannot proceed past splash without a healthy Service. The Service owns boot-time schema migrations, the encryption key, the pending-ops boot recovery, and the single-instance lock.

That coupling forces a second piece of work: **respawn**. If the Service crashes during Phases 2-7, the UI must observe it and re-establish the connection rather than dying. Phase 1.5 ships a minimal cold-restart respawn (no backoff, no crashloop detection, no UI status indicator - those are Phase 8). The respawn machinery is the survival kit that lets the new boot coupling not regress UI startup when the Service flakes.

Phase 1.5 is the largest UI-side surgery in the project. The `App` struct currently assumes synchronous initialization: `crate::DB.get().expect("DB not initialized")` works at any point because `OnceLock<Arc<Db>>` is populated before `App::boot` runs. After Phase 1.5, that handle is populated only after a Service handshake that may take 60+ s on a 50 GB schema migration. Many call sites need to flip from "DB always available" to "DB available after boot transitions to Ready."

The phase ships as a single milestone but with a clean commit-level split: respawn machinery, schema migration + key relocation, and the global state-handle refactor are separate failure domains. A bisect on a regression should land on the right commit.

## Scope

### In scope

1. **Respawn loop** in `ServiceClient`. When the child process exits unexpectedly (or the heartbeat task observes a hard error - see "Hard-error definition" below), the client awaits the dying child's `wait()` (with a 5 s watchdog escalating to SIGKILL on Linux / `start_kill` on Windows so a deadlocked-in-cleanup Service cannot hang the respawn) so the file lock is genuinely released, then calls `Command::spawn()` again. Pending requests at crash time fail with `ClientError::ServiceCrashed`. A new boot handshake runs against the replacement Service. **Terminal failures do not respawn** (see scope item 15 below). No backoff, no crashloop detection, no UI status indicator (Phase 8). Respawn while in `Ready` is silent: the UI does not transition `Ready` -> `Booting` (Phase 1.5 has no real workload over the IPC; the existing `Ready` state is unaffected by the Service identity).

2. **Schema migrations relocate to Service boot.** The Service runs the full migration sequence (currently in `crates/db/src/db/migrations.rs` and the velo->ratatoskr rename in `crates/db/src/db/mod.rs:55-71`) before signaling boot-handshake readiness. The UI's `ReadWriteDb::init` no longer runs at app boot; instead the UI waits for the Service's `boot.ready` response, then constructs read-side state.

3. **Encryption-key load relocates to Service boot.** `common::crypto::load_encryption_key` runs Service-side. Missing or unreadable key file is a **fatal Service exit** (exit code `BootExitCode::KeyLoadFailure`), not the silent zero-key fallback `crates/common/src/crypto.rs:42-43` falls back to today. The auto-respawn machinery would otherwise widen the window where data gets written under the zero key.

4. **Pending-ops boot recovery (`pending_ops::recover_on_boot`)** runs Service-side and completes before the boot handshake signals readiness. Stranded "executing" rows reset to "pending" before the UI thinks the Service is ready. The **periodic** drainer relocates with the action service in Phase 2; Phase 1.5 owns boot-time recovery only.

5. **`db_mark_queued_drafts_failed_sync` relocates** from `App::boot` (`crates/app/src/app.rs:330-336`) to Service boot. Today it races the iced runtime startup; on the Service side it slots into the boot sequence cleanly before handshake readiness.

5a. **Thread-participants backfill relocates** from `crates/app/src/handlers/core.rs:1001` (currently triggered after accounts load) to Service boot. The mutation in `backfill_thread_participants_for_account_sync` (`handlers/core.rs:1033`) is a write-side operation; per the problem-statement.md write-surface inventory, it lands Service-side in Phase 1.5. Phase 1.5 runs it once at boot for every account known to the DB; the post-account-load trigger is removed UI-side. Future-account-creation backfill becomes a Phase 2 action-pipeline concern.

6. **Single-instance lock.** Service takes an OS-level file lock on `<app_data>/ratatoskr.lock` at boot, before any DB work. If locked, exit with `BootExitCode::AnotherInstanceRunning`. The UI's child-wait sees the exit code and surfaces "Ratatoskr is already running" rather than "Service crashed." Lock library: `fs2 = "0.4"` (more established than `fd-lock`, simpler API; both work on Linux and Windows). Lock release happens at process exit on both platforms (kernel-managed).

7. **`BootExitCode` enum** in `service-api`. Variants represent **fatal boot-time exits only**; codes are picked outside the clap (=2) / panic (=101) / signal (137 / 143 by shell convention) ranges so `wait().status.code()` cannot ambiguously match a Rust-runtime or shell-induced exit:
   - `HandshakeFailure = 70` (sysexits.h `EX_SOFTWARE`-adjacent territory; reserved for protocol/version mismatch detected Service-side)
   - `AnotherInstanceRunning = 71`
   - `MigrationFailure = 72`
   - `KeyLoadFailure = 73`

   The UI maps the child's exit code to a `BootClassification`:
   - `code == 0` AND `BootReady` was already observed: clean shutdown (Service runtime ran).
   - `code == 0` AND `BootReady` not yet observed: **`UnexpectedExit { code: 0 }`**, NOT a successful boot. A Service that exits 0 before answering `boot.ready` is broken and the UI surfaces a fatal error.
   - `code == None` (signal-killed / panic-aborted, since Rust's `panic = "abort"` -> SIGABRT yields `None` on Unix): `UnexpectedExit { code: None }`.
   - `code == Some(n)` matching a `BootExitCode` variant: `BootFailure { code: BootExitCode }`.
   - `code == Some(n)` not in the variant set: `UnexpectedExit { code: Some(n) }`.

   `Success(0)` deliberately is not a `BootExitCode` variant, since 0-before-handshake is the broken case worth distinguishing.

8. **Boot handshake protocol.** New IPC method `boot.ready` distinct from `health.ping`:
   - `health.ping` keeps its 5 s timeout and frozen envelope; the UI sends it first to verify the protocol version is compatible. This stays the version-check method.
   - `boot.ready` has a 10-minute timeout (large schema migration headroom) and returns `BootReadyResponse { ready: true, migrations_applied: u32, schema_version: u32 }`. The Service answers it only after migrations + key load + pending-ops recovery + queued-drafts sweep all complete.
   - The UI awaits `boot.ready` before considering the Service usable.

9. **`boot.progress` notifications**. Service emits these during long migrations so the UI splash can render progress. Phase staging:
   - `BootPhase::LoadingKey`
   - `BootPhase::OpeningDatabase`
   - `BootPhase::Migrating { current: u32, total: u32 }`
   - `BootPhase::RecoveringPendingOps`
   - `BootPhase::SweepingQueuedDrafts`
   - `BootPhase::BackfillingThreadParticipants`

   Each notification carries an optional human-readable message for the splash text.

   **Coalesce key is per-phase variant, not a single `BootProgress` key.** The CoalesceKey is `BootProgressPhase(BootPhase::Discriminant)` so each variant collapses independently: `Migrating { 1, 10 }` and `Migrating { 5, 10 }` collapse (latest-wins on Migrating progress), but `LoadingKey` and `OpeningDatabase` are independent entries that retain wire order. A single key would let the latest phase clobber earlier phases the UI never got to render, breaking both the user-visible sequence and the integration test that asserts ordered phase delivery.

   `BootPhase::AcquiringLock` is deliberately absent from the variant list. Lock acquisition runs before the writer task is alive, so the notification cannot be emitted; if the lock contends, the Service exits with `BootExitCode::AnotherInstanceRunning` instead, which the UI surfaces directly.

10. **Splash rendering during boot.** Resolves the chicken-and-egg from the roadmap (notifications need a consumer; consumer is App; App is awaiting handshake). Recommended path: split `ServiceClient::spawn` into two phases (see `Architecture` below), so the UI can subscribe to notifications immediately after Phase 1 (spawn + version check) and render boot progress while Phase 2 (`boot.ready`) is in flight.

11. **App state-machine refactor.** `App` becomes a state machine with two states reachable at boot time:
    - `AppState::Booting { service_client, splash }` - active during Phase 2 of spawn. `view()` renders the splash; `update()` consumes `ServiceNotification(BootProgress(...))` to update splash text/progress.
    - `AppState::Ready { db, service_client, ... }` - the existing `App` field set, populated after `boot.ready`.

    `Booting` is the initial state. Transition to `Ready` fires on `Message::ServiceBootReady(BootReadyResponse)`. All current direct `crate::DB.get().expect(...)` call sites move into `Ready`; subscriptions and view paths gate on the discriminant.

12. **Spawn-failure policy.** Fatal at boot (UI exits cleanly). Consistent across phases - avoids a "this used to boot without the Service" regression.

13. **Boot handshake "ready" semantics.** `boot.ready` answered when: schema migrated **and** key loaded **and** pending-ops recovered **and** queued-drafts sweep complete **and** single-instance lock held. Phase 3 extends with "Tantivy index initialized." Phase 6 with "all writers initialized." Each addition is one field on `BootReadyResponse`.

14. **JSON id-space across respawn.** Document: `ServiceClient.next_id` continues across respawn (UI-side perspective) while the new Service starts at id=1 in its own perspective. Harmless in v1 because there are no Server -> UI requests, but documented in `service_client.rs` so a future phase that adds them knows the id space needs to be partitioned.

15. **Terminal failure policy** (the crashloop guard for v1). The 1-second sleep before respawn (open question 3) bounds CPU under transient crashes but is NOT sufficient for deterministic boot failures. `BootExitCode::KeyLoadFailure`, `BootExitCode::MigrationFailure`, and `BootExitCode::AnotherInstanceRunning` are **terminal** in Phase 1.5: the UI logs the cause, surfaces a fatal error message, and `iced::exit()`s rather than respawning. Without this, a missing key file would respawn one Service per second forever, producing 86k log files per day under the per-PID log-naming scheme. `BootClassification::UnexpectedExit { .. }` is also terminal. Only "Service was running and crashed" (reader-EOF after `BootReady` was observed, or heartbeat hard-error) triggers respawn. Phase 8's exponential backoff + crashloop detection extends this; Phase 1.5's job is to never produce an infinite log/CPU loop.

16. **Heartbeat hard-error definition.** The respawn trigger from the heartbeat task is enumerated, not "any error":
    - `Timeout`: **transient**, do not respawn. Logged at warn level. Migrations longer than the 5 s ping timeout are expected during boot; respawn-on-timeout would produce a tight respawn loop on a 60 s migration.
    - `stdin_tx.send` returns `Err`: **hard**, respawn. The writer task died (typically because the child closed its stdin), so the Service is genuinely gone.
    - Reader-task EOF (a separate path): **hard**, respawn.
    - Anything else: **hard**, respawn. New variants must be enumerated in the heartbeat task with comments naming this contract.

17. **DB-only pending-ops recovery.** `pending_ops::recover_on_boot` (`crates/core/src/actions/pending.rs`) currently takes an `ActionContext` that carries the encryption key, content stores, provider/search plumbing, and DB write state. Phase 1.5 only needs DB state repair (resetting stranded "executing" rows). Phase 1.5 introduces a DB-only recovery function (working title `pending_ops::recover_on_boot_db_only`) that takes only the DB connection, runs the same SQL the existing function runs, and is what Phase 1.5's Service boot calls. The original `recover_on_boot` stays for Phase 2 to call from the relocated action service. This avoids dragging Phase 2's `ActionContext` shape into Phase 1.5's boot.

18. **Migration runs in `spawn_blocking`.** `rusqlite` is fully synchronous; a 60 s migration on a tokio runtime worker thread starves the dispatch task that answers `health.ping`, with two visible consequences: heartbeat false-positives (transient, won't respawn per item 16, but will fill the log) and `BootProgress` notifications can't be emitted from a thread blocked inside `INSERT/CREATE` SQL (so the splash freezes mid-migration). All synchronous DB work in the boot sequence (DB open + velo->ratatoskr rename + schema migrations + DB-only pending recovery + queued-drafts sweep + thread-participants backfill) runs under `tokio::task::spawn_blocking`. The progress emitter is the only async-side actor during this window; it pumps `BootPhase::Migrating { current, total }` updates from a callback the migration runner invokes per step.

19. **Migration idempotency contract.** Future migration authors must satisfy a documented contract for the respawn-mid-migration case to be safe:
    - Each migration must be wrapped in a single SQLite transaction; SQLite WAL recovery rolls back partial transactions on the next open. The current v100 migration in `crates/db/src/db/migrations.rs:124-138` already does this (`BEGIN` ... `COMMIT` with the `_migrations` row inserted last); this is the contract.
    - If a future migration MUST batch into multiple committed transactions (per-row backfills are the typical case), each batch must be idempotent and resumable, AND the `_migrations` row must NOT be inserted until every batch has committed. A partial-apply that lacks the `_migrations` row will be re-run from scratch on the next boot.
    - The velo->ratatoskr rename in `crates/db/src/db/mod.rs:55-71` is **not** atomic across `.db`, `.db-wal`, and `.db-shm`. The Phase 1.5 implementation must add explicit recovery for the partial-rename case: if `ratatoskr.db` exists but `ratatoskr.db-wal` is missing while `velo.db-wal` exists, complete the rename before opening the DB.
    - `BootPhase::Migrating { current, total }` may emit values that go BACKWARDS on respawn (first run got to 4/10, second run starts at 0/10 again). Coalesce keying handles the wire compaction; the splash's UX of "moving backwards on respawn" is an accepted user-visible behavior, since respawn-during-migration is rare.

20. **Service generation tag for cross-respawn notification safety.** Notifications have no JSON-RPC id, and the plan keeps a single `NotificationQueue` shared across respawns. A dying Service can enqueue a stale `BootProgress` (or in Phase 2+, a stale `action.completed`) that lands AFTER the respawned Service's identity is established, and the UI cannot tell which incarnation it came from. Each Service incarnation gets a `service_generation: u32` (UI-side counter, bumped on respawn). The reader task tags every notification with the current generation at enqueue time; the App's notification-dispatch handler drops any notification whose generation doesn't match the current one. The handshake response carries the Service's PID so test code can correlate, but the generation is the authoritative discriminator.

    Equivalent stronger guarantee that we explicitly choose against: fully await the old reader task before accepting any notification from the new reader. Theoretically tighter but adds a serialization point on the respawn fast path; with the generation tag, the same correctness emerges from the dispatch side without the pause.

21. **Booting-state Message whitelist.** The `App` state machine refactor (item 11) requires explicit gating of every existing `Message` variant. While in `Booting`, the following are valid and dispatched:
    - `Message::ServiceChildSpawned(client)` -> populate `service_client`.
    - `Message::ServiceBootReady(response)` -> transition to `Ready`.
    - `Message::ServiceBootFailed(classification)` -> log + iced::exit().
    - `Message::ServiceNotification(BootProgress(_))` -> update splash state.
    - `Message::WindowResized` / `Message::WindowMoved` / `Message::WindowCloseRequested` -> apply to the (single) main window.
    - `Message::AppearanceChanged` -> stash for `Ready` to consume.
    - `Message::Noop`, `Message::ModifiersChanged` -> harmless.

    Any other `Message` arriving in `Booting` is logged at debug level and dropped. `view()`, `title()`, `theme()`, `scale_factor()`, and `subscription()` have separate `Booting` implementations: the splash renders in `view()`; `title()` returns "Ratatoskr - Starting"; `theme()` returns the default; `scale_factor()` returns 1.0; `subscription()` is the service-notifications recipe only (no `SyncTick`, `SnoozeTick`, etc., since those need DB state).

    The detailed task list (item 10) gains an explicit audit step: enumerate every `Message` variant currently in `crates/app/src/message.rs`, and for each, declare its `Booting` behavior (handle / drop / forward-to-Ready-after-transition).

### Out of scope

- Backoff between respawns. Phase 8.
- Crashloop detection (N respawns in M seconds). Phase 8.
- UI status indicator showing Service health. Phase 8.
- In-flight request replay across respawn (idempotent vs non-idempotent classification). Phase 8.
- Migration of any actual write surface (sync, action service, drafts, signatures, etc.). Those are Phases 2-6.
- IPC protocol versioning beyond the existing `PROTOCOL_VERSION` constant.
- Tray-resident UI / autostart / daemon promotion. Phases 9+.
- Splash icon assets / branded splash visuals. v1 splash is functional plaintext.

## Architecture

### Two-phase spawn

`ServiceClient::spawn` splits to allow notification subscription before the slow `boot.ready` round-trip completes:

```
ServiceClient::spawn(app_data_dir) -> impl Stream<SpawnEvent>
   |
   +-> SpawnEvent::ChildSpawned(Arc<ServiceClient>)
   |     // After:
   |     //   - subprocess spawned
   |     //   - reader/writer/heartbeat tasks running
   |     //   - notification queue is alive
   |     //   - first health.ping succeeded; PROTOCOL_VERSION verified
   |     // App now holds the client, can subscribe to notifications.
   |
   +-> (boot.progress notifications stream into the queue)
   |
   +-> SpawnEvent::BootReady(BootReadyResponse)
         // After:
         //   - boot.ready returned successfully
         //   - schema migrated, key loaded, pending-ops recovered
         // App transitions Booting -> Ready.
```

Implementation choice: return a `tokio::sync::mpsc::Receiver<SpawnEvent>` from `spawn` rather than a stream, since iced subscriptions consume mpsc cleanly. `App::boot` registers a single Task::stream that maps each event to a Message:
- `SpawnEvent::ChildSpawned` -> `Message::ServiceChildSpawned(Arc<ServiceClient>)`
- `SpawnEvent::BootReady` -> `Message::ServiceBootReady(BootReadyResponse)`
- `SpawnEvent::Failure(BootExitCode)` -> `Message::ServiceBootFailed(BootExitCode)` (fatal)

### Service-side boot sequence

In `run_service_blocking`, after the parent-death recheck, stdio defense, logger init, and panic hook (already in place from Phase 1):

```
1.  Acquire <app_data>/ratatoskr.lock (fs2 file lock).
       On EWOULDBLOCK: exit BootExitCode::AnotherInstanceRunning.
       Hold the file handle for the lifetime of the Service.
       (Lock acquisition fires before the writer task is alive,
        so no BootProgress notification is possible here; the
        AnotherInstanceRunning exit is the user-visible signal.)

2.  Open the Service's IPC stdio (stdio_defense::adopt_into_runtime;
    already present from Phase 1).

3.  Spawn the writer task and start reading from stdin.
       The dispatch loop is now alive and answers health.ping;
       boot.ready blocks until step 9 completes.

4.  Begin emitting boot.progress notifications.
       Send BootPhase::LoadingKey first.

5.  Load encryption key via common::crypto::load_encryption_key
    inside spawn_blocking.
       Missing key: log error, exit BootExitCode::KeyLoadFailure.
       Successful load: stash in OnceCell<[u8; 32]> for Phase 2's
       ActionContext to consume.

6.  Send BootPhase::OpeningDatabase.
       Inside spawn_blocking:
         - Run the velo->ratatoskr rename (with WAL/SHM atomicity
           recovery; see "Migration idempotency" in scope item 19).
         - Open ratatoskr.db.
         - Run schema migrations under a single transaction
           (current contract).
         - During migration, the runner invokes a per-step callback
           that posts BootPhase::Migrating { current, total } updates
           into an mpsc to the async-side progress emitter. The
           emitter pumps notifications onto out_tx without blocking
           the migration thread.
       Migration failure: log, exit BootExitCode::MigrationFailure.

7.  Send BootPhase::RecoveringPendingOps.
       Inside spawn_blocking:
         pending_ops::recover_on_boot_db_only(&conn) - DB-only
         recovery that does not require ActionContext (see scope
         item 17).

8.  Send BootPhase::SweepingQueuedDrafts.
       Inside spawn_blocking:
         db_mark_queued_drafts_failed_sync(&conn).

9.  Send BootPhase::BackfillingThreadParticipants.
       Inside spawn_blocking, for each account in the DB run the
       per-account backfill that handlers/core.rs:1033 currently
       drives UI-side. (See scope item 5a.)

10. Mark boot.ready as answerable.
       The dispatch handler unblocks any pending boot.ready request
       and returns BootReadyResponse.
```

The dispatch loop runs concurrently with steps 4-10 so `health.ping` continues to succeed throughout boot (heartbeat won't false-positive into a respawn during a long migration; per scope item 16 a heartbeat `Timeout` is transient). `boot.ready` requests that arrive during steps 4-10 park on a `tokio::sync::Notify` until step 10 fires. All synchronous DB work runs in `spawn_blocking` (scope item 18) so the dispatch task and the progress emitter are never starved by `rusqlite` blocking the runtime worker thread.

The DB `Connection` opened in step 6 is **not** closed at the end of boot. It moves into a Service-internal `BootContext { key, conn, lock_guard }` held in a `OnceCell` that Phase 2's `ActionContext` will consume. Closing-then-reopening between Phase 1.5 and Phase 2 would waste connection setup on every action; threading the artifacts through avoids it.

### `App` state machine

```rust
pub enum App {
    Booting(BootingApp),
    Ready(ReadyApp),
}

pub struct BootingApp {
    service_client: Option<Arc<ServiceClient>>,  // None until ChildSpawned
    splash: SplashState,                          // current BootPhase + message
    main_window_id: iced::window::Id,
    settings: AppSettings,                        // loaded from disk only;
                                                  // does not need DB
}

pub struct ReadyApp {
    // Today's `App` field set, unchanged.
    db: Arc<Db>,
    service_client: Arc<ServiceClient>,
    service_notifications: ServiceNotificationReceiver,
    sidebar: Sidebar,
    thread_list: ThreadList,
    // ... etc
}

pub struct SplashState {
    phase: BootPhase,
    message: String,
    // For Migrating { current, total }: progress fraction.
    progress: Option<(u32, u32)>,
}
```

Transitions:
- `App::boot()` returns `App::Booting(BootingApp::new(...))` plus a `Task` that drives the spawn stream.
- `Message::ServiceChildSpawned(client)` populates `BootingApp.service_client`.
- `Message::ServiceNotification(BootProgress(phase))` updates `BootingApp.splash`.
- `Message::ServiceBootReady(response)` constructs `ReadyApp` and replaces `*self = App::Ready(ready_app)`. The construction does the work `App::boot` does today: opens `ReadDbState`, loads accounts, builds component state.
- `Message::ServiceBootFailed(code)` logs and `iced::exit()`s.

`App::view()` and `App::update()` match on the discriminant. `Booting` renders the splash; `Ready` runs the existing logic. The Message dispatch follows the explicit Booting whitelist from scope item 21: any non-whitelisted Message reaching `Booting::update` is logged and dropped; this is a hard contract, not a "try and see if it works" pattern. `view()`, `title()`, `theme()`, `scale_factor()`, `subscription()` have separate Booting implementations.

`crate::DB.get().expect(...)` at `crates/app/src/app.rs:151` is the only direct OnceLock call site in app code today (verified by grep). The rest of the codebase passes `Arc<Db>` through call sites; after Phase 1.5, `Arc<Db>` is constructed in the `ServiceBootReady` handler and stored on `ReadyApp`. The `crate::DB` OnceLock is deleted.

`crate::APP_DATA_DIR` is **not** in the same boat: it has multiple non-`App::boot` callers, including:
- `crates/app/src/db/threads.rs:241`
- `crates/app/src/handlers/core.rs:685`
- `crates/app/src/handlers/core.rs:828`
- `crates/app/src/handlers/commands.rs:15`
- `crates/app/src/handlers/pop_out/session.rs:18`

`APP_DATA_DIR` does not depend on the DB or the Service - it's just a path. **Phase 1.5 keeps the `APP_DATA_DIR` static** (initialized synchronously in `main()` before `App::boot`, stable for the process lifetime, accessed from anywhere). The roadmap's "global state-handle refactor" applies to DB only; APP_DATA_DIR is a stable runtime path with no boot-handshake dependency. The plan's earlier draft proposed deleting both, which would have rippled into half a dozen unrelated handlers. Resolved.

### Respawn machinery

`ServiceClient` gains a respawn capability that fires when the reader task observes EOF or the heartbeat task observes a hard error (not `Timeout` - that's the existing transient).

Today `ServiceClient` holds:
- `child: Mutex<Option<Child>>`
- `stdin_tx: Option<mpsc::Sender<Vec<u8>>>` (immutable Option)
- `pending: Arc<DashMap<...>>`
- `next_id: Arc<AtomicU64>`
- `notifications: Arc<NotificationQueue>`
- `reader_handle / writer_handle / heartbeat_handle: Mutex<Option<JoinHandle>>`

To support respawn, mutable fields wrap in a single `Mutex<RunningState>`:

```rust
struct RunningState {
    child: Child,
    stdin_tx: mpsc::Sender<Vec<u8>>,
    reader_handle: JoinHandle<()>,
    writer_handle: JoinHandle<()>,
    heartbeat_handle: JoinHandle<()>,
    /// Bumped on every successful spawn (initial + each respawn).
    /// The reader task tags every notification with this value at
    /// enqueue time; the App's notification dispatcher drops any
    /// notification whose generation does not match the current
    /// one. Closes the stale-notification race across respawns.
    generation: u32,
}

pub struct ServiceClient {
    state: Mutex<Option<RunningState>>,                 // None during respawn
    pending: Arc<DashMap<u64, oneshot::Sender<...>>>,   // shared across respawns
    next_id: Arc<AtomicU64>,                            // continues across respawns
    notifications: Arc<NotificationQueue>,              // shared queue, kept alive
    current_generation: AtomicU32,                      // advanced on respawn
    _process_guard: ProcessGuard,                       // Linux/Windows parent-death
    binary_path: PathBuf,                               // for respawn Command::new
    app_data_dir: PathBuf,                              // for respawn args
    extra_args: Vec<String>,                            // for test-helpers spawns
    spawn_event_tx: mpsc::Sender<SpawnEvent>,           // re-emit ChildSpawned / BootReady
                                                        //   / Terminal on respawn
}
```

Respawn algorithm:

1. Reader task EOF or heartbeat hard error -> `client.handle_crash()`. (See scope item 16 for the precise definition of "hard error".)
2. `handle_crash` takes the `state` lock; if already `None` (another task is respawning), return.
3. Set `state` to `None`. Call `fail_pending(&pending)` so all in-flight requests return `ClientError::ServiceCrashed`. Bump `current_generation` (subsequent stale notifications from the dying reader will be dropped on dispatch).
4. Await the dying child's `wait()` with a **5 s watchdog**: if `wait()` does not return in 5 s (deadlocked panic-handler cleanup, kernel-level lock contention, etc.), escalate via `Child::start_kill()` (Linux SIGKILL / Windows TerminateProcess) and try `wait()` again with a fresh 1 s budget. Without the watchdog, a stuck Service hangs the respawn forever. The dying child must release its file lock before the replacement spawn or the new Service will exit with `AnotherInstanceRunning`.
5. Sleep 1 s before respawn (the v1 crashloop bound from open question 3).
6. Classify the dying Service's exit (per the `BootClassification` mapping in scope item 7). If terminal (`KeyLoadFailure`, `MigrationFailure`, `AnotherInstanceRunning`, or any `UnexpectedExit`), emit `SpawnEvent::Terminal(classification)` on `spawn_event_tx` and **do not respawn**. The App receives this as `Message::ServiceBootFailed` and `iced::exit()`s.
7. Otherwise spawn a new child via the same path as initial spawn. Re-emit `SpawnEvent::ChildSpawned` and `SpawnEvent::BootReady` on `spawn_event_tx`. Populate `state` with the new `RunningState` (with `generation = current_generation.load()`).

The pending requests at crash time fail fast; subsequent requests (issued after respawn) go to the new Service.

Boot-handshake "ready" semantics on respawn: the UI does **not** transition `Ready` -> `Booting`. The new Service's `boot.ready` confirms that schema is still current (it almost always is) and pending-ops recovery rescued any stranded "executing" rows from the crash. The UI logs the respawn and continues. Phase 1.5 has no real workload over the IPC, so a silent respawn is the right user experience; Phase 8 adds the visible status indicator.

The post-respawn `boot.ready` response is consumed for a schema-version sanity check: if the respawned Service reports a different `schema_version` than the original `BootReady`, the binary has been swapped underneath us and the only safe action is to surface a fatal error. (This catches the rare case of an in-place upgrade between respawns; otherwise the schema version is stable.)

**Stale-notification dispatch.** The App's notification handler reads the generation tag on every received notification. If the tag doesn't match `current_generation`, the notification is dropped at debug level. This applies to `BootProgress` in Phase 1.5 and pre-empts every Phase 2+ `MustDeliver` notification (`action.completed`, `index.committed`, `push.event`) from being applied to the wrong Service incarnation.

**Log-file accumulation across respawns** (closes the failure mode that `service.<pid>.log` produces a fresh file per respawn while "keep 3" rolling is per-PID). At Service boot - after the lock is acquired but before opening the DB - the Service unlinks any `service.*.log*` files in `<app_data>/logs/` whose PID is not the current one and whose mtime is older than 24 hours. This bounds the directory under any crashloop that the terminal-failure policy somehow doesn't catch, while preserving recent-history logs for diagnosis.

### `boot.ready` IPC shape

`service-api/src/request.rs`:

```rust
pub enum RequestParams {
    HealthPing,
    Shutdown,
    BootReady,
    // ...
}

impl RequestParams {
    pub fn timeout(&self) -> RequestTimeoutKind {
        match self {
            Self::HealthPing => RequestTimeoutKind::Finite(Duration::from_secs(5)),
            Self::Shutdown => RequestTimeoutKind::Finite(Duration::from_secs(30)),
            Self::BootReady => RequestTimeoutKind::Finite(Duration::from_secs(600)),
            // ...
        }
    }
}
```

`service-api/src/response.rs`:

```rust
pub struct BootReadyResponse {
    pub ready: bool,
    pub schema_version: u32,
    pub migrations_applied: u32,
}
```

`service-api/src/notification.rs`:

```rust
pub enum Notification {
    BootProgress(BootProgress),
    // (Phase 2+ adds OperationOutcome, action.completed, etc.)
}

pub enum BootPhase {
    LoadingKey,
    OpeningDatabase,
    Migrating { current: u32, total: u32 },
    RecoveringPendingOps,
    SweepingQueuedDrafts,
    BackfillingThreadParticipants,
}

impl BootPhase {
    /// Discriminant used as the coalesce key so each phase coalesces
    /// independently. Migrating { 1, 10 } and Migrating { 5, 10 }
    /// collapse into the latest; LoadingKey and OpeningDatabase are
    /// independent entries. A single key would let the latest phase
    /// clobber unrendered earlier phases.
    pub fn coalesce_discriminant(&self) -> BootPhaseKind {
        match self {
            Self::LoadingKey => BootPhaseKind::LoadingKey,
            Self::OpeningDatabase => BootPhaseKind::OpeningDatabase,
            Self::Migrating { .. } => BootPhaseKind::Migrating,
            Self::RecoveringPendingOps => BootPhaseKind::RecoveringPendingOps,
            Self::SweepingQueuedDrafts => BootPhaseKind::SweepingQueuedDrafts,
            Self::BackfillingThreadParticipants => {
                BootPhaseKind::BackfillingThreadParticipants
            }
        }
    }
}

pub struct BootProgress {
    pub phase: BootPhase,
    pub message: Option<String>,
    /// Service incarnation generation, for stale-notification dispatch
    /// after respawn (see "Respawn machinery"). Tagged on the UI side
    /// at reader-task enqueue time, not by the Service.
    pub service_generation: u32,
}

impl Notification {
    pub fn class(&self) -> NotificationClass {
        match self {
            // Per-phase coalesce: each BootPhaseKind collapses on its
            // own key. This preserves the ordered phase sequence on the
            // wire while still letting Migrating { current, total }
            // updates compact.
            Self::BootProgress(progress) => NotificationClass::Coalesce {
                key: CoalesceKey::BootProgress(
                    progress.phase.coalesce_discriminant(),
                ),
            },
        }
    }
}
```

By design only `Coalesce`-class notifications fire during boot, so the `notif_tx` buffer cannot fill before App subscribes regardless of migration duration. Phase 2+ adding any `MustDeliver` boot-time emission would need to revisit this contract.

### File-lock library

`fs2 = "0.4"`, used as:

```rust
use fs2::FileExt;
let lock_file = OpenOptions::new()
    .create(true)
    .write(true)
    .open(app_data_dir.join("ratatoskr.lock"))?;
match lock_file.try_lock_exclusive() {
    Ok(()) => Ok(LockGuard { _file: lock_file }),
    Err(error) if error.kind() == ErrorKind::WouldBlock => {
        Err(BootError::AnotherInstanceRunning)
    }
    Err(error) => Err(error.into()),
}
```

The `LockGuard` is held for the lifetime of the Service; lock release happens in the kernel on process exit (clean or crash). On Linux, `flock`-style: kernel releases on process exit including SIGKILL. On Windows, `fs2` uses `LockFile` (not `LockFileEx`); locks release on handle close, which the kernel does on process exit. Microsoft documents that the released lock may not become available immediately on Windows - factor a brief retry into any future test that races spawn-after-crash; the respawn algorithm's `wait()`-then-spawn ordering already covers the typical case.

`fd-lock` is the alternative; chosen against because `fs2` is more widely used in tokio-adjacent crates. Either would work.

## Detailed task list

In recommended commit order. Each item is one focused commit unless noted.

1. **`service-api`: wire types.** `BootExitCode` (with the 70-73 codes), `BootClassification`, `BootReady` request, `BootReadyResponse`, `BootProgress { phase, message, service_generation }`, `BootPhase`, `BootPhaseKind`, `CoalesceKey::BootProgress(BootPhaseKind)`. Type-only commit; serde round-trip tests, `RequestParams::BootReady.timeout() == 600s` test, `BootPhase::coalesce_discriminant` correctness test.

2. **`app`: `ClientError` taxonomy.** Add `ClientError::BootFailure { classification: BootClassification }` distinct from `ServiceCrashed` and `VersionMismatch`. Phase 1.5 callers can pattern-match on the classification to surface the right user message.

3. **`service`: single-instance lock.** Add `fs2` dep. New `crates/service/src/instance_lock.rs` with `LockGuard` RAII type. Wire into `run_service_blocking` between stdio_defense and the boot sequence. Test: spawn one Service, attempt second spawn against same data dir, assert second exits with `BootExitCode::AnotherInstanceRunning as i32`.

4. **`service`: log-file cleanup at boot.** Unlink `service.*.log*` files older than 24h whose PID is not the current one. Bounds the directory under any path that escapes the terminal-failure policy. Runs after the lock is acquired, before DB open.

5. **`service`: BootPhase emission framework.** Add `boot_progress::emit(phase: BootPhase)` helper that constructs the `Notification::BootProgress` and posts it to the outbound queue via the writer task. Used by every subsequent boot-sequence commit. Land first so the rest can emit without coordination overhead.

6. **`service`: encryption-key load.** Move `load_encryption_key` invocation to Service boot inside `spawn_blocking`. Missing/unreadable: fatal exit `BootExitCode::KeyLoadFailure`. Successful load stashes in `OnceCell<[u8; 32]>` for Phase 2's `ActionContext`. Remove the UI's key-load call site.

7. **`service`: DB-only pending-ops recovery extraction.** New `pending_ops::recover_on_boot_db_only(&Connection)` that does the SQL state repair without `ActionContext`. The existing `recover_on_boot` stays for Phase 2 to call. No behavior change yet; just the API split that Phase 1.5's boot needs.

8. **`service`: schema migrations + velo->ratatoskr rename + recovery.** Move the rename + migration runner to Service boot, all inside `spawn_blocking`. Add explicit recovery for the partial-rename case (`.db` renamed but `.db-wal`/`.db-shm` not yet) - reconcile the trailing files before opening. Emit `BootPhase::OpeningDatabase`, `BootPhase::Migrating { current, total }` via the per-step callback. The migration runner gets a `progress: &dyn FnMut(u32, u32)` parameter that pumps an mpsc to the async-side emitter. Migration failure: `BootExitCode::MigrationFailure`. Hold the connection in `BootContext` past boot; do not close.

9. **`service`: pending-ops boot recovery, queued-drafts sweep, thread-participants backfill.** Wire `pending_ops::recover_on_boot_db_only`, `db_mark_queued_drafts_failed_sync`, and the per-account `backfill_thread_participants_for_account_sync` into the boot sequence. Emit the corresponding `BootPhase`. Remove the UI-side calls (`App::boot`'s queued-draft sweep, `handlers/core.rs:1001`'s post-accounts-load backfill).

10. **`service`: `boot.ready` handler.** New `crates/service/src/handlers/boot.rs`. Awaits a `tokio::sync::Notify` that the boot sequence fires when step 10 completes; returns `BootReadyResponse { ready: true, schema_version, migrations_applied }`. Bypasses both the in-flight semaphore and the admission cap (settle the naming - `bypasses_admission()` rather than `bypasses_semaphore()` - here, since boot.ready is the second user). Test: in-process harness with a faked migration that takes >1 s asserts a parallel `health.ping` returns immediately while `boot.ready` is still pending.

11. **`app`: two-phase spawn.** `ServiceClient::spawn` returns `mpsc::Receiver<SpawnEvent>`. Flow: spawn child -> version-check ping -> emit `ChildSpawned` -> issue `boot.ready` -> emit `BootReady` (or `Terminal(classification)` on failure). Tests with a fake Service exercise both phases and the failure path.

12. **`app`: `Message` variant audit + Booting whitelist enumeration.** Walk every `Message` variant currently in `crates/app/src/message.rs`, and for each declare the Booting behavior (handle / drop / forward-to-Ready-after-transition). Lands as a doc-comment table in `message.rs` so future Message additions are forced to consider Booting behavior. No code change yet.

13. **`app`: `App` state machine.** `App` becomes `enum { Booting(BootingApp), Ready(ReadyApp) }`. Each impl block matches discriminant. `BootingApp` view + update follow the whitelist from item 12. `ReadyApp` is a near-line-by-line move of today's `App` field set; the `ReadyApp::from_boot_ready(...)` constructor performs the work currently in `App::boot` after handshake (load accounts, build sidebar, etc.). New messages: `ServiceChildSpawned`, `ServiceBootReady`, `ServiceBootFailed`. Removes `ServiceReady`. Delete `crate::DB` OnceLock; keep `crate::APP_DATA_DIR`.

14. **`app`: respawn loop with terminal-failure classification.** `ServiceClient::handle_crash`: bump generation, fail pending, `wait()` with 5 s watchdog escalating to `start_kill`, classify exit, emit `Terminal(_)` if terminal else respawn. Test: kill mid-session, assert respawn happens and a subsequent request succeeds; separately, replace the binary with one whose key file is missing and assert `Terminal(BootFailure { KeyLoadFailure })` reaches the App.

15. **`app`: stale-notification dispatch via generation tag.** Reader task tags every notification with `current_generation`. Notification dispatch handler drops mismatches at debug. Test: drive the reader task with a hand-crafted stale `BootProgress` after a generation bump and assert it never reaches the splash.

16. **`app`: terminal-failure UI surfacing.** `Message::ServiceBootFailed(classification)` -> log + `iced::exit()`. AnotherInstanceRunning gets a user-friendly message; everything else gets a technical message. Acceptance is "the app exits cleanly with the right log line"; no UI plumbing for the error dialog yet (that's a separate UX concern).

17. **`docs/service`: update problem-statement.md and implementation-roadmap.md** to reflect this plan's settled items: BootExitCode codes 70-73, two-phase spawn, generation tag, terminal-failure policy, BootPhase set + per-phase coalesce. Update the implementation-roadmap.md "first ping timeout extension" wording to refer to `boot.ready` rather than re-using `health.ping`. Bundle with the implementation commit that ships the protocol per CLAUDE.md.

## File-by-file changes

**New files:**
- `crates/service-api/src/boot.rs` - `BootExitCode`, `BootReadyResponse`, `BootPhase`, `BootProgress`.
- `crates/service/src/instance_lock.rs` - `LockGuard` RAII.
- `crates/service/src/boot.rs` - boot sequence orchestration.
- `crates/service/src/boot_progress.rs` - notification emission helpers.
- `crates/service/src/handlers/boot.rs` - `boot.ready` handler.
- `crates/service/src/key_load.rs` - thin wrapper around `common::crypto::load_encryption_key` with `BootExitCode::KeyLoadFailure` on missing-key.
- `crates/service/src/db_init.rs` - DB open + migration orchestration.
- `crates/app/src/booting_app.rs` - new `BootingApp` and `SplashState`.
- `crates/app/src/ready_app.rs` - relocates today's `App` field set.

**Modified files:**
- `Cargo.toml` (workspace) - add `fs2 = "0.4"` to service dependencies.
- `crates/service-api/src/request.rs` - add `BootReady` variant + 600s timeout. Rename `bypasses_semaphore()` to `bypasses_admission()` to reflect its dual role (per-handler semaphore + dispatch-loop admission cap).
- `crates/service-api/src/notification.rs` - add `BootProgress` variant + per-phase `Coalesce` class via `BootPhaseKind`. Add `service_generation: u32` field to the `BootProgress` payload.
- `crates/service-api/src/error.rs` - no major changes; existing variants handle the new failure shapes.
- `crates/service/src/lib.rs` - new boot sequence wiring (lock -> log cleanup -> stdio adopt -> writer task -> spawn_blocking { key load, DB rename + open + migrations, pending-ops recovery, queued-drafts sweep, thread-participants backfill } -> mark boot.ready answerable). All synchronous work runs in `spawn_blocking` so the dispatch task is never starved.
- `crates/service/src/dispatch.rs` - rename `bypasses_semaphore` callers; admission cap also bypassed for `boot.ready` and `health.ping`.
- `crates/service/src/handlers/mod.rs` - register `boot::handle`.
- `crates/core/src/actions/pending.rs` - extract `recover_on_boot_db_only(&Connection)` from the existing `recover_on_boot(&ActionContext)` so Phase 1.5 can call it without an `ActionContext`.
- `crates/db/src/db/migrations.rs` - migration runner gains a `progress: &mut dyn FnMut(u32, u32)` parameter.
- `crates/db/src/db/mod.rs` - velo->ratatoskr rename gains explicit recovery for the partial-rename case (.db renamed but .db-wal / .db-shm not yet).
- `crates/app/src/main.rs` - no change; mode dispatch already in place.
- `crates/app/src/app.rs` - becomes the discriminant-matching wrapper that delegates to `BootingApp` / `ReadyApp`.
- `crates/app/src/lib.rs` - **delete** the `static DB: OnceLock<Arc<Db>>` line at `lib.rs:50`. **Keep** `APP_DATA_DIR` (it has multiple non-`App::boot` callers and does not depend on the DB; see "App state machine" section).
- `crates/app/src/service_client.rs` - two-phase spawn, respawn loop with terminal-failure classification, `RunningState` mutability refactor, `current_generation: AtomicU32`.
- `crates/app/src/message.rs` - `ServiceReady` removed; add `ServiceChildSpawned(Arc<ServiceClient>)`, `ServiceBootReady(BootReadyResponse)`, `ServiceBootFailed(BootClassification)`. Doc-comment table maps every variant to its Booting-state behavior.
- `crates/app/src/update.rs` - dispatch the new messages; old `ServiceReady` arm rewrites; `ServiceBootFailed` does `iced::exit()`.
- `crates/app/src/handlers/core.rs` - remove the post-accounts-load thread-participants backfill (now Service-side at boot).

**Deleted files:**
- None (Phase 2 will delete the action-context references that move; not Phase 1.5).

**Cargo.lock** changes from the new `fs2` dep. Committed per CLAUDE.md.

**`dev-seed` interaction.** `crates/app/src/lib.rs:64-86` wipes the data dir and re-seeds when built with `--features dev-seed`. After Phase 1.5: dev-seed must produce a `ratatoskr.key` file as part of seeding (otherwise every dev launch hits `KeyLoadFailure` and the terminal-failure policy kicks in). Land this as a one-line addition in `crates/dev-seed/src/lib.rs` alongside item 6. The `dev-seed` integration test (if any) should also acquire the lock cleanly when Phase 1.5 lands.

## Test plan

### Unit tests

- `service-api`: serde round-trip for `BootExitCode`, `BootClassification`, `BootReadyResponse`, `BootPhase`, `BootProgress`. `RequestParams::BootReady.timeout()` returns 600s. `BootProgress` `Notification::class()` returns per-phase `Coalesce` (assert two `Migrating` updates collapse but `LoadingKey` and `OpeningDatabase` do not).
- `service`: `instance_lock::LockGuard` acquires + releases on a temp dir. Second acquire on the same dir returns `WouldBlock`. Lock survives process panic (panic in test, verify next acquire succeeds).
- `service`: log-file cleanup unlinks `service.<other-pid>.log*` files older than 24h, preserves the current PID's file, preserves recent files.
- `service`: boot sequence happy path with a faked DB and key file; verify `BootProgress` sequence in order; verify `boot.ready` returns `ready: true`. Emission ordering: each phase fires before the next phase's work begins.
- `service`: boot sequence with missing key file exits `KeyLoadFailure` (code 73). Corrupt DB (garbage in ratatoskr.db) exits `MigrationFailure` (72). Existing instance lock exits `AnotherInstanceRunning` (71).
- `service`: partial-velo-rename recovery: stage `ratatoskr.db` + `velo.db-wal` + `velo.db-shm` in a temp dir; assert open succeeds and the WAL/SHM files are renamed to ratatoskr counterparts.
- `service`: migration progress callback fires per step; per-phase coalesce keeps `Migrating` collapsed in the queue.
- `service`: `recover_on_boot_db_only` on a fresh DB (no rows) is a no-op. With a stranded "executing" row, that row resets to "pending."
- `app`: `BootingApp::view()` renders the splash for each `BootPhase`. `ReadyApp` field constructor produces the same shape today's `App` has. `BootingApp::update` whitelist drops every non-whitelisted Message at debug level.

### Integration tests (in-process)

- `tests/dispatch_in_process.rs::boot_ready_blocks_until_sequence_completes` - faked Service issues `boot.ready` before steps 5-9 complete; assert the request blocks. Trigger completion; assert the request returns.
- `tests/dispatch_in_process.rs::boot_progress_notifications_emitted_in_order` - drive boot sequence; collect notifications off the wire; assert phase ordering matches the design despite per-phase coalescing.
- `tests/dispatch_in_process.rs::health_ping_succeeds_during_long_migration` - run `boot.ready` against a faked-slow migration (>5 s artificial sleep in the migration step); fire a parallel `health.ping`; assert the ping returns within its 5 s timeout while the migration is still in progress. Confirms the spawn_blocking + dispatch concurrency contract.
- `tests/dispatch_in_process.rs::stale_notifications_dropped_after_generation_bump` - drive a fake reader-task scenario where a generation bump happens, then a stale `BootProgress` arrives; assert it never reaches the splash dispatcher.

### Real-subprocess smoke tests

- `crates/app/tests/service_subprocess.rs::boot_ready_returns_after_migrations` - spawn against a fresh data dir; assert the UI sees `ServiceChildSpawned` followed by `ServiceBootReady` within 30 s.
- `crates/app/tests/service_subprocess.rs::missing_key_file_exits_with_key_load_failure_code` - spawn against a data dir where ratatoskr.key is absent; assert child exit code is 73 (`BootExitCode::KeyLoadFailure`).
- `crates/app/tests/service_subprocess.rs::two_instances_against_same_data_dir_second_exits_already_running` - spawn first Service, attempt second spawn; second exits with code 71 (`AnotherInstanceRunning`).
- `crates/app/tests/service_subprocess.rs::respawn_after_sigkill_succeeds` - spawn Service, SIGKILL the child mid-session, verify a subsequent request succeeds against the respawned child.
- `crates/app/tests/service_subprocess.rs::pending_request_fails_at_respawn` - spawn Service, fire a long-running request, SIGKILL the child mid-flight, assert the pending request returns `ClientError::ServiceCrashed` AND a subsequent request against the respawned child succeeds.
- `crates/app/tests/service_subprocess.rs::terminal_failure_does_not_respawn` - spawn against a data dir with a missing key file (so first boot fails with `KeyLoadFailure`); assert the App receives `Terminal(BootFailure { KeyLoadFailure })` exactly once and no respawn fires. (Build-side: requires the test harness to expose `Message::ServiceBootFailed` observation.)
- `crates/app/tests/service_subprocess.rs::deadlocked_service_drop_escalates_to_kill` - simulate a Service that doesn't respond to anything (`test-helpers` adds a `--test-hang` flag that loops in shutdown drain); SIGKILL parent; assert the watchdog escalation kills the child within 6 s total.

### Manual matrix updates

- `boot.ready` 60+ s migration on a real 50 GB-class DB. Splash renders progress per-phase, including ordered `Migrating { current, total }` updates. UI does not appear frozen.
- Kill the Service mid-migration. Respawn re-runs migrations (SQLite WAL rolls back the partial transaction). UI splash renders the second migration pass; `Migrating { current }` may go backwards (acceptable, see scope item 19).
- BootExitCode visibility on Windows. Verify exit codes 70-73 propagate via `wait().status.code()` correctly. (Linux variant of this is fully automated above.)
- Two instances on Windows: same-data-dir second instance exits with code 71. (Linux: automated.)
- Stale-notification dispatch in steady state: kill the Service mid-Phase-2-`action.completed`-emit; verify the next sync's `action.completed` is correctly attributed to the new Service incarnation.

## Open questions

Settled before this plan landed:

- **Two-phase spawn (option c)** chosen over pre-iced splash (option a) and queue-replay (option b). Option a adds a separate native window with platform-specific quirks; option b can't render live progress because the queue isn't drained until App subscribes. Two-phase spawn lets App subscribe to notifications as soon as the child exists, which is what live splash rendering needs.
- **`fs2` over `fd-lock`** for the file lock, on widely-used-tokio-adjacent grounds. Either works; minor.
- **Schema migration runs Service-side, not in a separate migration tool.** Out-of-process migrations would let us migrate while the UI is still booting; in-process keeps the boot sequence linear and easier to reason about. Migration time for normal mailboxes is sub-second; the 50 GB case is rare.

Resolve in implementation:

Settled by review fan-out before this plan was finalized:

- **`bypasses_semaphore()` renames to `bypasses_admission()`** in Phase 1.5 (single flag governing both the per-handler semaphore and the dispatch-loop admission cap; both `health.ping` and `boot.ready` set it).
- **Splash text source-of-truth**: Service emits English strings in `BootProgress.message`; localization is a separate concern.
- **Respawn timing budget**: 1-second sleep before respawn for transient crashes; terminal failures (scope item 15) skip respawn entirely. Phase 8 replaces with exponential backoff.
- **Migration progress granularity**: the migration runner gains a `progress` callback (item 8 in the task list); Phase 1.5 ships with `Migrating { 1, 1 }` for the current single v100 migration, with the framework in place for future multi-step migrations to emit per-step progress.
- **`AppState` enum** chosen over a trait for iced's match-on-message dispatch ergonomics.
- **`crate::DB` deletion confirmed; `crate::APP_DATA_DIR` retained.** APP_DATA_DIR has multiple non-`App::boot` callers and does not depend on the DB or Service handshake.

Remaining open questions (resolve in implementation):

1. **Whether to add a UI status banner for respawn-while-Ready.** The plan currently says respawn is silent in `Ready`. If implementation surfaces user confusion (e.g. "did my action complete? the Service just respawned"), revisit by adding a transient banner. Costs are small; deferred until a concrete need surfaces.

2. **Schema-version sanity check on respawn.** The post-respawn `boot.ready` schema_version is compared to the original. Do we hard-fail (`iced::exit()`) on mismatch, or surface a recoverable error and let the user retry? Default proposal: hard-fail, since the binary has been swapped underneath us and the safe state is unknown. Confirm during item 14.

3. **Migration progress callback contract for future authors.** The `progress: &mut dyn FnMut(u32, u32)` signature is straightforward, but we need a written contract: callers must invoke it at least once per logical step and the values must be monotonic per migration (with the documented exception of "may decrease across respawn"). Land as a doc-comment in `crates/db/src/db/migrations.rs` alongside item 8.

## Verification (end-to-end)

1. Fresh data dir, `cargo run -p app`. UI shows splash with "Loading key" -> "Opening database" -> "Migrating (1/1)" -> "Recovering pending ops" -> "Sweeping queued drafts" -> "Backfilling thread participants" -> Ready. Total: well under 1 s on a fresh DB. (Lock acquisition has no splash entry by design - if it failed, the Service would exit before splash subscription is even possible.)
2. Repeat against an existing data dir; same sequence, all phases sub-second on a non-migrating boot.
3. `rm <app_data>/ratatoskr.key`; relaunch. UI shows boot splash, then receives `Terminal(BootFailure { KeyLoadFailure })` and exits cleanly with a fatal-error log line. **No respawn loop.**
4. Two `cargo run -p app` instances against the same data dir simultaneously. Second exits with "Ratatoskr is already running" and code 71; first continues normally. **No respawn loop in the second instance.**
5. Kill the Service mid-session (`kill <service-pid>`). UI logs the respawn at debug level; subsequent action (e.g. open a thread) works normally. Generation tag bumps; any in-flight notification from the dying Service is dropped on dispatch.
6. Trigger a long migration (insert a slow step manually for testing). UI splash updates progress; ordered phase sequence preserved; `Migrating { current, total }` updates compact via per-phase coalescing. `health.ping` continues to round-trip throughout migration (heartbeat does not respawn).
7. SIGKILL the Service while it's wedged in a synthetic infinite loop (test-helpers `--test-hang`). UI's respawn watchdog escalates to `start_kill` after 5 s; replacement Service spawns within ~6 s of the original kill.
8. Two-phase spawn timing: from `Command::spawn()` to `Message::ServiceChildSpawned` should be sub-second on a healthy host; from `ServiceChildSpawned` to `ServiceBootReady` is dominated by migration time. Both observable in logs.
9. `brokkr check` clean.
10. `crates/app/tests/service_subprocess.rs` integration tests all pass.

## Promotion criteria

This phase is done when:

- All items in `In scope` are implemented and wired (App reaches `Ready` only after a real `boot.ready` round-trip; respawn machinery handles a SIGKILL of the Service without taking the UI down).
- All `Exit criteria` from the implementation-roadmap.md Phase 1.5 section are satisfied.
- `boot.ready` correctly blocks during simulated long migrations and the UI splash renders progress.
- Manual matrix items run on Linux (the Windows manual matrix items remain pending a real Windows host).
- Reviewer signoff on this plan + the delivered code.

The next phase (Phase 2 - Action service migration) gets its own equivalent plan document at the time it's tackled. Phase 2 leans heavily on the type-level read/write split scaffolding that Phase 1.5 doesn't introduce; the Phase 2 plan owns the `service-state` crate and the `WriteDbState` boundary.
