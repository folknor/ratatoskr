use crate::notification_queue::NotificationQueue;
use dashmap::DashMap;
use serde::de::DeserializeOwned;
use service_api::{
    BootClassification, BootExitCode, BootReadyResponse, BoundedLineReader,
    CalendarRunCompleted, CalendarRunId, CalendarSetVisibilityAck, CalendarSetVisibilityParams,
    CalendarStartAccountSyncParams, CalendarStartAck, CalendarSyncResult, HealthPingResponse,
    JsonRpcErrorObject, JsonRpcRequest, Notification, PROTOCOL_VERSION, ParsedServiceMessage,
    RequestParams, RequestTimeoutKind, ServiceError, ServiceResponse, ShutdownResponse,
    SyncCancelAccountParams, SyncCancelAck, SyncCompleted, SyncResult, SyncRunId,
    SyncStartAccountParams, SyncStartAck, encode_message, parse_service_message,
};
use std::collections::{HashMap, VecDeque, hash_map::Entry};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex, PoisonError, Weak,
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, mpsc, oneshot};

const STDIN_QUEUE_CAP: usize = 1024;
const NOTIFICATION_QUEUE_CAP: usize = 1024;

/// Sliding-window crashloop bound for the post-Ready respawn loop. A
/// runtime crash that produces an `UnexpectedExit` (signal-killed, unknown
/// numeric code) does NOT match `BootExitCode::from_i32`, so the
/// terminal-no-respawn policy doesn't fire and the respawn loop keeps
/// going. The 1 s sleep before respawn is the only other bound, which
/// caps CPU at ~one Service per second forever - per-PID log naming would
/// turn that into 86 400 log files per day. This crashloop guard turns the
/// loop terminal after `CRASHLOOP_THRESHOLD` respawns within
/// `CRASHLOOP_WINDOW`. Phase 8 replaces this with exponential backoff +
/// real telemetry; for v1 a flat threshold is enough.
const CRASHLOOP_WINDOW: Duration = Duration::from_secs(30);
const CRASHLOOP_THRESHOLD: usize = 3;

pub type ServiceNotificationReceiver = Arc<NotificationQueue>;

/// Per-`SyncRunId` waiter slot. Phase 3 task 14: a `start_sync` /
/// `cancel_and_await` caller subscribes to a broadcast channel for the
/// active run; on `Notification::SyncCompleted` arrival, the reader
/// task fans the result out across every subscriber. The
/// `Completed` variant latches a result that arrives *before* any
/// caller subscribes (the subscribe-after-completion race) so a fast
/// `SyncCompleted` is not dropped on the floor. Latched entries are
/// GC'd after `LATCHED_COMPLETED_TTL` if no consumer ever shows up.
enum PendingSync {
    Pending(broadcast::Sender<SyncResult>),
    Completed {
        result: SyncResult,
        latched_at: Instant,
    },
}

const SYNC_BROADCAST_CAPACITY: usize = 8;
const LATCHED_COMPLETED_TTL: Duration = Duration::from_secs(30);

type PendingSyncs = Arc<Mutex<HashMap<SyncRunId, PendingSync>>>;

/// Phase 5 task 9b: per-`CalendarRunId` waiter slot. Mirrors
/// `PendingSync` exactly. Routes `Notification::CalendarRunCompleted`
/// to `start_calendar_sync` / `cancel_and_await` callers.
enum PendingCalendar {
    Pending(broadcast::Sender<CalendarSyncResult>),
    Completed {
        result: CalendarSyncResult,
        latched_at: Instant,
    },
}

type PendingCalendars = Arc<Mutex<HashMap<CalendarRunId, PendingCalendar>>>;

/// Drain every entry: `Pending` senders are dropped (subscribers
/// receive `RecvError::Closed` and surface as
/// `Err(ClientError::ServiceCrashed)`); `Completed` entries are
/// discarded. Used by `handle_crash` and `Drop` so an in-flight
/// `start_sync` caller does not park forever across a Service
/// crash / respawn.
fn fail_pending_syncs(map: &PendingSyncs) {
    let mut guard = map.lock().unwrap_or_else(PoisonError::into_inner);
    guard.clear();
}

/// Mirror of `fail_pending_syncs` for the calendar map.
fn fail_pending_calendars(map: &PendingCalendars) {
    let mut guard = map.lock().unwrap_or_else(PoisonError::into_inner);
    guard.clear();
}

/// Drop `Completed` entries that have aged past `LATCHED_COMPLETED_TTL`.
/// Cheap; called opportunistically (no separate timer task is needed -
/// the next `start_sync` / `cancel_and_await` runs the sweep before
/// inserting). 30 s is a wide safety margin against the sub-second
/// subscribe-after-completion race; revisit if a longer race window
/// surfaces.
fn sweep_latched_completed(map: &PendingSyncs) {
    let mut guard = map.lock().unwrap_or_else(PoisonError::into_inner);
    let cutoff = Instant::now();
    guard.retain(|_, entry| match entry {
        PendingSync::Completed { latched_at, .. } => {
            cutoff.duration_since(*latched_at) < LATCHED_COMPLETED_TTL
        }
        PendingSync::Pending(_) => true,
    });
}

/// Mirror of `sweep_latched_completed` for the calendar map.
fn sweep_latched_calendar_completed(map: &PendingCalendars) {
    let mut guard = map.lock().unwrap_or_else(PoisonError::into_inner);
    let cutoff = Instant::now();
    guard.retain(|_, entry| match entry {
        PendingCalendar::Completed { latched_at, .. } => {
            cutoff.duration_since(*latched_at) < LATCHED_COMPLETED_TTL
        }
        PendingCalendar::Pending(_) => true,
    });
}

/// Mutable per-incarnation state. Replaced atomically on every respawn.
///
/// The dying child + handles are taken out by `handle_crash` (or by `Drop`,
/// whichever races first). `pending`, `next_id`, `notifications`, and
/// `current_generation` live on `ServiceClient` itself because they survive
/// across respawns.
struct RunningState {
    child: Child,
    stdin_tx: mpsc::Sender<Vec<u8>>,
    reader_handle: tokio::task::JoinHandle<()>,
    writer_handle: tokio::task::JoinHandle<()>,
    heartbeat_handle: tokio::task::JoinHandle<()>,
    /// Bumped on every successful spawn (initial = 1, then 2, 3, ...). The
    /// reader task tags every notification with this value at enqueue time;
    /// `handle_crash(dying_generation)` will only act on the state if the
    /// matching generation is still installed.
    generation: u32,
}

/// Immutable respawn configuration. `None` on the test single-shot
/// `spawn_for_test` path: those clients tear down on first crash and the
/// orchestration belongs to the test, not to the client. For the real
/// `spawn_with_events` path, this struct carries everything `handle_crash`
/// needs to launch a replacement Service and re-emit `SpawnEvent`s.
struct RespawnConfig {
    binary_path: PathBuf,
    app_data_dir: PathBuf,
    extra_args: Vec<String>,
    spawn_event_tx: mpsc::Sender<SpawnEvent>,
    /// Captured first `BootReadyResponse` for the schema-version sanity
    /// check on every subsequent respawn. A binary swap that changes the
    /// schema version under us is fatal: there's no safe state we can keep
    /// running in. Stored as `Option` so we can capture it the first time
    /// `boot.ready` returns.
    first_boot_ready: Mutex<Option<BootReadyResponse>>,
}

pub struct ServiceClient {
    state: Mutex<Option<RunningState>>,
    pending: Arc<DashMap<u64, oneshot::Sender<Result<serde_json::Value, ClientError>>>>,
    /// Phase 3 task 14: per-`SyncRunId` waiter slots. Cross-incarnation
    /// state, drained on respawn / Drop. See `PendingSync` for the
    /// `Pending` / `Completed` discipline.
    pending_syncs: PendingSyncs,
    /// Phase 5 task 9b: per-`CalendarRunId` waiter slots. Mirrors
    /// `pending_syncs` exactly; routes `Notification::CalendarRunCompleted`
    /// to `start_calendar_sync` / `cancel_and_await` callers.
    pending_calendars: PendingCalendars,
    next_id: Arc<AtomicU64>,
    notifications: ServiceNotificationReceiver,
    /// Latest generation; bumped before respawn. The dispatch-side drop
    /// (item 15) compares notifications' tagged generation against this.
    current_generation: AtomicU32,
    /// Set by `Drop` to short-circuit any in-flight `handle_crash` respawn.
    /// `handle_crash` checks before allocating a new child and again before
    /// installing the new state, so the worst-case race is a respawn that
    /// raced past the first check and gets cleaned up via the second one.
    is_shutting_down: AtomicBool,
    /// Per-client respawn knobs; `None` on the test single-shot spawn path.
    respawn_config: Option<RespawnConfig>,
    /// Sliding-window timestamps of recent respawns. `handle_crash`
    /// pushes onto this before launching a replacement; the crashloop
    /// guard fires when `CRASHLOOP_THRESHOLD` entries land within
    /// `CRASHLOOP_WINDOW`. See [`CrashloopTracker::record_and_check`].
    respawn_attempts: Mutex<VecDeque<Instant>>,
    /// Cross-platform parent-death tie-up. Held for the lifetime of the
    /// client so the OS-level safety net (Job Object on Windows) survives
    /// any failure in our explicit Drop teardown. Listed last so it drops
    /// after every other field, making the kill-on-job-close fire only as
    /// a true last-resort. Reused across respawns: each new child is
    /// assigned to the same `ProcessGuard` so the safety net stays in place
    /// for the replacement.
    _process_guard: process_lifetime::ProcessGuard,
}

impl std::fmt::Debug for ServiceClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceClient").finish_non_exhaustive()
    }
}

/// Reason for a fatal Service boot failure surfaced through
/// `Message::ServiceBootFailed`. Carries the structured classification when
/// the dying child's exit code matched a known `BootExitCode`, or a plain
/// detail string for everything else (UI-side spawn IO failures, version
/// mismatch, in-flight ServiceCrashed, request timeout). `ClientError`
/// itself is not `Clone` (because `std::io::Error` is not), and iced
/// Messages must be `Clone`, so this is the wire-friendly projection.
/// Visibility note: `pub` (not `pub(crate)`) because `Message::ServiceBootFailed`
/// is a public variant of the public `Message` enum and carries this type.
/// Tightening to `pub(crate)` would emit `private_interfaces` warnings.
/// Constructed only from `from_client_error`; not part of any external API
/// surface in practice (no external crate imports it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootFailureReason {
    Classified(BootClassification),
    Other(String),
}

impl BootFailureReason {
    /// Map the various `ClientError` shapes onto a reason suitable for the
    /// terminal-failure handler. The classified path keeps the structured
    /// info so the UI can surface a friendlier message for the
    /// `AnotherInstanceRunning` case; everything else falls back to the
    /// `Display` of the error.
    ///
    /// Two paths produce a `Classified`:
    /// - `ClientError::BootFailure { classification }`: synthesized by the
    ///   spawn flow when the dying child's exit code mapped to a
    ///   `BootExitCode` (or to `UnexpectedExit`) directly.
    /// - `ClientError::Service(ServiceError::BootFailure { code })`: the
    ///   Service answered `boot.ready` with the structured error before
    ///   exiting. We project this onto the same `BootClassification` shape
    ///   so the UI's per-code message dispatch is uniform across the two
    ///   paths.
    pub(crate) fn from_client_error(error: &ClientError) -> Self {
        match error {
            ClientError::BootFailure { classification } => {
                Self::Classified(*classification)
            }
            ClientError::Service(ServiceError::BootFailure { code }) => {
                Self::Classified(BootClassification::BootFailure { code: *code })
            }
            other => Self::Other(other.to_string()),
        }
    }
}

/// Compute the user-visible message for a terminal boot failure. The
/// `AnotherInstanceRunning` case is the only one with a genuinely
/// user-actionable message ("Ratatoskr is already running"); everything
/// else gets a technical message logged for the next launch's diagnosis
/// per scope item 16's "no UI plumbing for the error dialog yet" exit
/// criterion.
pub(crate) fn terminal_failure_user_message(reason: &BootFailureReason) -> String {
    match reason {
        BootFailureReason::Classified(BootClassification::BootFailure { code }) => match code {
            BootExitCode::AnotherInstanceRunning => {
                "Ratatoskr is already running.".to_string()
            }
            BootExitCode::KeyLoadFailure => {
                "Encryption key missing or unreadable.".to_string()
            }
            BootExitCode::MigrationFailure => "Database migration failed.".to_string(),
            BootExitCode::HandshakeFailure => "Service handshake failed.".to_string(),
            BootExitCode::LockIoFailure => {
                "Could not acquire the Ratatoskr instance lock (check disk space and \
                 directory permissions).".to_string()
            }
        },
        BootFailureReason::Classified(BootClassification::UnexpectedExit { code }) => {
            match code {
                Some(code) => format!("Service exited unexpectedly (code {code})."),
                None => "Service exited unexpectedly (no exit code; signaled).".to_string(),
            }
        }
        BootFailureReason::Other(detail) => format!("Service boot failed: {detail}"),
    }
}

/// Log + stderr-write the terminal-failure message. Returns the message
/// string so the iced handler can use it (and so tests can assert on it).
pub(crate) fn surface_terminal_failure(reason: &BootFailureReason) -> String {
    let message = terminal_failure_user_message(reason);
    log::error!("Fatal: {message}");
    eprintln!("[ui] fatal: {message}");
    message
}

/// Events emitted by the two-phase spawn flow on the receiver returned by
/// [`ServiceClient::spawn_with_events`].
///
/// The flow is:
/// 1. Spawn child subprocess.
/// 2. Verify protocol version via `health.ping`.
/// 3. Emit `ChildSpawned(client)`. The App can now hold the `ServiceClient`
///    and subscribe to notifications (e.g. `boot.progress` for splash
///    rendering).
/// 4. Issue `boot.ready` with a 600s timeout.
/// 5. Emit `BootReady(response)` once the Service has migrated, loaded the
///    key, and finished pending-ops recovery / drafts sweep / participants
///    backfill. The App transitions Booting -> Ready on this event.
///
/// On any failure between those steps the receiver gets a single
/// `Terminal(error)` event and the channel closes; downstream maps it to
/// `Message::ServiceBootFailed` and exits via `iced::exit()`.
///
/// On respawn (post-BootReady reader-EOF or heartbeat hard-error per scope
/// item 16 of `phase-1.5-plan.md`), `handle_crash` re-emits a fresh
/// `ChildSpawned` followed by another `BootReady` (or `Terminal` if the
/// respawn handshake fails). The App's notification consumer has already
/// hooked the shared `NotificationQueue`, so the re-`ChildSpawned` is
/// informational - the queue itself was preserved across the respawn.
#[derive(Debug)]
pub enum SpawnEvent {
    ChildSpawned(Arc<ServiceClient>),
    BootReady(BootReadyResponse),
    Terminal(ClientError),
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("service error: {0}")]
    Service(#[from] ServiceError),
    #[error("request timeout")]
    Timeout,
    #[error("service crashed")]
    ServiceCrashed,
    #[error("not connected")]
    NotConnected,
    #[error("protocol version mismatch: ui={ui}, service={service}")]
    VersionMismatch { ui: u32, service: u32 },
    /// Service boot exited before answering `boot.ready`. Distinct from
    /// `ServiceCrashed` (which means the running Service died after going
    /// Ready) and `VersionMismatch` (which means the protocol negotiation
    /// itself disagreed). Callers pattern-match on the classification to
    /// surface the right user message:
    /// - `BootFailure { code: AnotherInstanceRunning }`: "Ratatoskr is
    ///   already running."
    /// - `BootFailure { code: KeyLoadFailure }`: "Encryption key missing
    ///   or unreadable."
    /// - `BootFailure { code: MigrationFailure }`: "Database migration
    ///   failed."
    /// - `UnexpectedExit { .. }`: "Service exited unexpectedly."
    #[error("service boot failure: {classification:?}")]
    BootFailure { classification: BootClassification },
    /// Respawn observed a `boot.ready` whose `schema_version` does not match
    /// the value captured on the very first `BootReady`. Indicates a binary
    /// swap underneath the running App; there is no safe state we can keep
    /// running in, so the client surfaces this as Terminal. Distinct from
    /// `ServiceCrashed` so the log line names the actual cause.
    #[error("schema_version changed across respawn: was {was}, now {now}")]
    SchemaVersionChanged { was: u32, now: u32 },
    /// Respawn observed a `boot.ready` but `first_boot_ready` was None.
    /// `handle_crash` defers respawn while `first_boot_ready` is None
    /// (initial boot's `run_spawn_flow` owns the Terminal-on-failure
    /// surface), so reaching this path means a refactor broke that
    /// invariant. Treating it as Terminal rather than capturing-and-
    /// continuing is per security review: continuing would lose the
    /// binary-swap detection on every subsequent respawn (the next
    /// comparison would be against the newly-captured value, not the
    /// original). Treating it as `unreachable!()` was rejected because
    /// this code runs in a spawned crash-handler task; a panic there can
    /// vanish as task failure rather than surfacing a fatal user-visible
    /// event.
    #[error("respawn missing first_boot_ready baseline; cannot prove no binary swap")]
    SchemaBaselineMissing,
    #[error("response deserialize: {0}")]
    Deserialize(#[from] serde_json::Error),
}

/// Three-way classification of an `action.execute_plan` outcome, used by
/// the UI's tri-state in-flight tracking (Phase 2 plan scope item 14).
///
/// - `Acked`: handler returned `ActionPlanAck { journaled: true }`. The
///   plan is durable; a subsequent `ServiceCrashed` does NOT trigger
///   optimistic rollback - the journal-driven worker will replay
///   outcomes after respawn.
/// - `AckUnknown`: the IPC future resolved with `ServiceCrashed` or
///   `Timeout` without an observed ack. The Service may or may not
///   have journaled the plan (crash after commit while the ack was
///   still in the OS pipe buffer is observable; so is a client-side
///   timeout that lost a race against a slow journal commit). The UI
///   holds optimistic state and resolves via `action.job_status` after
///   the next `boot.ready`.
/// - `Failed`: the request never reached the journal (`NotConnected`,
///   `ServiceError` validation failure, deserialize failure, terminal
///   `BootFailure` / `SchemaVersionChanged` / `VersionMismatch` /
///   `SchemaBaselineMissing`). UI rolls back optimistic state and
///   surfaces a toast.
///
/// Classification rule lives next to `ClientError` so adding a new
/// variant forces an explicit decision (no `_` arm in `classify`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchOutcome {
    Acked(service_api::ActionPlanAck),
    AckUnknown { reason: String },
    Failed { reason: String },
}

/// Convert an `action.execute_plan` IPC result into a `DispatchOutcome`.
///
/// `Acked` short-circuits any rollback. `AckUnknown` defers rollback
/// until reconciliation. `Failed` rolls back immediately.
pub fn classify_dispatch(
    result: Result<service_api::ActionPlanAck, ClientError>,
) -> DispatchOutcome {
    match result {
        Ok(ack) => DispatchOutcome::Acked(ack),
        Err(ClientError::Timeout) => DispatchOutcome::AckUnknown {
            reason: "request timeout".to_string(),
        },
        Err(ClientError::ServiceCrashed) => DispatchOutcome::AckUnknown {
            reason: "service crashed".to_string(),
        },
        Err(error @ ClientError::Io(_) | error @ ClientError::Deserialize(_)) => {
            // Wire-corruption errors land in the same bucket as a
            // crash: we cannot prove the journal write happened OR
            // didn't, so reconcile rather than guess.
            DispatchOutcome::AckUnknown {
                reason: format!("wire error: {error}"),
            }
        }
        Err(error @ ClientError::NotConnected) => DispatchOutcome::Failed {
            reason: error.to_string(),
        },
        Err(error @ ClientError::Service(_)) => DispatchOutcome::Failed {
            reason: error.to_string(),
        },
        Err(error @ ClientError::VersionMismatch { .. })
        | Err(error @ ClientError::BootFailure { .. })
        | Err(error @ ClientError::SchemaVersionChanged { .. })
        | Err(error @ ClientError::SchemaBaselineMissing) => DispatchOutcome::Failed {
            reason: error.to_string(),
        },
    }
}

impl ServiceClient {
    pub async fn spawn(app_data_dir: &Path) -> Result<Arc<Self>, ClientError> {
        let exe = std::env::current_exe()?;
        Self::spawn_inner(&exe, app_data_dir, &[], None).await
    }

    /// Two-phase spawn that emits `SpawnEvent`s on the returned receiver.
    /// The receiver gets `ChildSpawned` after the version-check ping
    /// succeeds, `BootReady` after the boot.ready handshake completes, and
    /// `Terminal(error)` on any failure (after which the channel closes).
    ///
    /// Phase 1.5's two-phase spawn (per scope item 10 of `phase-1.5-plan.md`)
    /// is what lets the App subscribe to `boot.progress` notifications
    /// while migrations run - the splash needs the `ServiceClient` (for the
    /// notification queue) before the slow `boot.ready` round-trip
    /// completes.
    ///
    /// Clients spawned via this path are respawn-enabled: a post-BootReady
    /// reader-EOF or heartbeat hard-error triggers `handle_crash`, which
    /// launches a replacement Service and re-emits `ChildSpawned` /
    /// `BootReady` on the same receiver.
    pub fn spawn_with_events(app_data_dir: PathBuf) -> mpsc::Receiver<SpawnEvent> {
        let exe = std::env::current_exe();
        let (tx, rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let exe = match exe {
                Ok(exe) => exe,
                Err(error) => {
                    let _ = tx
                        .send(SpawnEvent::Terminal(ClientError::Io(error)))
                        .await;
                    return;
                }
            };
            run_spawn_flow(exe, app_data_dir, Vec::new(), tx).await;
        });
        rx
    }

    /// Test-only variant of `spawn_with_events` that lets tests override the
    /// binary path and pass extra args. Mirrors `spawn_for_test` in shape.
    #[cfg(feature = "test-helpers")]
    pub fn spawn_with_events_for_test(
        binary: PathBuf,
        app_data_dir: PathBuf,
        extra_args: Vec<String>,
    ) -> mpsc::Receiver<SpawnEvent> {
        let (tx, rx) = mpsc::channel(8);
        tokio::spawn(async move {
            run_spawn_flow(binary, app_data_dir, extra_args, tx).await;
        });
        rx
    }

    /// Test-only spawn that lets tests override the binary path and pass
    /// extra args to the Service. Used for spawn-failure (bad binary path)
    /// and version-mismatch (`--test-fake-version=N`) coverage. Compiled
    /// out of release builds via the `test-helpers` feature.
    ///
    /// Clients returned from this path have `respawn_config = None`: a
    /// crash tears down the state and stops, with no respawn attempt. The
    /// test owns orchestration.
    #[cfg(feature = "test-helpers")]
    pub async fn spawn_for_test(
        binary: &Path,
        app_data_dir: &Path,
        extra_args: &[&str],
    ) -> Result<Arc<Self>, ClientError> {
        Self::spawn_inner(binary, app_data_dir, extra_args, None).await
    }

    async fn spawn_inner(
        binary: &Path,
        app_data_dir: &Path,
        extra_args: &[&str],
        respawn_config: Option<RespawnConfig>,
    ) -> Result<Arc<Self>, ClientError> {
        let process_guard = process_lifetime::ProcessGuard::new()?;
        let pending: Arc<
            DashMap<u64, oneshot::Sender<Result<serde_json::Value, ClientError>>>,
        > = Arc::new(DashMap::new());
        let next_id = Arc::new(AtomicU64::new(1));
        let notifications: Arc<NotificationQueue> =
            Arc::new(NotificationQueue::new(NOTIFICATION_QUEUE_CAP));

        // current_generation is initialized to the same value the first
        // reader_task will capture (`generation = 1` below). Without this
        // alignment, the window between `Arc::new(...)` and
        // `install_running_state(spawned, 1)` would have the reader see
        // a live generation of 0 while its captured generation is 1, and
        // `reader_should_enqueue` would drop legitimate first-boot
        // `boot.progress` notifications as stale. install_running_state's
        // store(1) below is now idempotent rather than load-bearing.
        let pending_syncs: PendingSyncs = Arc::new(Mutex::new(HashMap::new()));
        let pending_calendars: PendingCalendars = Arc::new(Mutex::new(HashMap::new()));
        let client = Arc::new(Self {
            state: Mutex::new(None),
            pending: Arc::clone(&pending),
            pending_syncs: Arc::clone(&pending_syncs),
            pending_calendars: Arc::clone(&pending_calendars),
            next_id: Arc::clone(&next_id),
            notifications: Arc::clone(&notifications),
            current_generation: AtomicU32::new(1),
            is_shutting_down: AtomicBool::new(false),
            respawn_config,
            respawn_attempts: Mutex::new(VecDeque::new()),
            _process_guard: process_guard,
        });

        let generation: u32 = 1;
        let spawned = launch_subprocess(
            binary,
            app_data_dir,
            extra_args,
            &client._process_guard,
            Arc::clone(&pending),
            Arc::clone(&next_id),
            Arc::clone(&notifications),
            Arc::downgrade(&client),
            generation,
        )
        .await?;

        client.install_running_state(spawned, generation);

        let ping: HealthPingResponse = match client
            .request_or_observe_child_exit(RequestParams::HealthPing)
            .await
        {
            Ok(ping) => ping,
            Err(error) => {
                // The Service exited or crashed before answering the version-
                // check ping. If the dying child's exit code maps to a known
                // `BootExitCode` (the canonical `AnotherInstanceRunning` case
                // - the second instance against a contended lock exits 71
                // before its dispatch loop even reads stdin), elevate
                // `ServiceCrashed` / `Timeout` into a structured
                // `ClientError::BootFailure` so the terminal-failure handler
                // surfaces the right per-code message instead of the generic
                // "service crashed".
                return Err(client.elevate_initial_boot_error(error).await);
            }
        };
        if ping.version != PROTOCOL_VERSION {
            return Err(ClientError::VersionMismatch {
                ui: PROTOCOL_VERSION,
                service: ping.version,
            });
        }
        log::info!("Service ready (pid={}, gen={generation})", ping.pid);
        Ok(client)
    }

    /// Wait briefly for the dying child and project its exit code into a
    /// `ClientError::BootFailure { classification }`. Used by the initial-
    /// boot path (both the version-check ping and the `boot.ready` round-
    /// trip) to elevate generic `ServiceCrashed` / `Timeout` errors into
    /// structured boot-classification errors when the Service has already
    /// exited with a deterministic code. If the original error is already
    /// classified, or the child has not exited, returns the original.
    ///
    /// Tears the running state down as a side effect (the watchdog wait
    /// requires `&mut Child`); subsequent IPC calls on this client will
    /// return `NotConnected`. That's fine because callers only use this on
    /// the terminal-failure path - the client is on its way to being
    /// dropped anyway.
    /// Run a request and, in parallel, observe `Child::try_wait` on a
    /// short interval. Returns the request result if it resolves first;
    /// returns `Err(ClientError::ServiceCrashed)` immediately when the
    /// child is observed to have exited.
    ///
    /// **Why this exists.** The reader task fails the pending oneshot
    /// on EOF, which under typical load resolves the request quickly.
    /// Under heavy parallel-test scheduling pressure or production
    /// overload, the reader task can be starved long enough that the
    /// per-request timeout ceiling (5 s `health.ping`, 600 s `boot.ready`)
    /// becomes the actual wall time before the request fails - which
    /// produces flaky tests and slow terminal-failure surfacing in
    /// production. `try_wait` is non-blocking and signal-driven on
    /// Unix; the observer thread can fire even when the reader is
    /// stalled. The Phase 2 plan named this as a Phase 8 carry-forward;
    /// we land the structural fix here because the test cohort that
    /// depends on it is growing, not Phase 8.
    ///
    /// On exit-first, the caller is expected to feed
    /// `Err(ServiceCrashed)` into `elevate_initial_boot_error` to
    /// project the dying child's exit code into a structured
    /// `BootFailure`.
    async fn request_or_observe_child_exit<R>(
        self: &Arc<Self>,
        params: RequestParams,
    ) -> Result<R, ClientError>
    where
        R: DeserializeOwned,
    {
        let request_future = self.request::<R>(params);
        let exit_future = self.observe_child_exit();
        tokio::pin!(request_future);
        tokio::pin!(exit_future);
        tokio::select! {
            result = &mut request_future => result,
            () = &mut exit_future => Err(ClientError::ServiceCrashed),
        }
    }

    /// Poll `Child::try_wait` on a 50 ms interval. Returns when the
    /// child has exited, or when the running state has been torn down
    /// (which happens during shutdown / elevate / Drop).
    ///
    /// `try_wait` requires `&mut Child`, so this holds the state lock
    /// briefly per poll - microseconds, since `try_wait` is a
    /// non-blocking `waitpid(WNOHANG)` on Unix and the equivalent
    /// `WaitForSingleObject(0)` shape on Windows.
    async fn observe_child_exit(self: &Arc<Self>) {
        const POLL_INTERVAL: Duration = Duration::from_millis(50);
        loop {
            let exited = {
                let mut guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                match guard.as_mut() {
                    Some(state) => matches!(state.child.try_wait(), Ok(Some(_))),
                    // No state means we're tearing down (shutdown / Drop).
                    // Treat as "no longer running" so the caller can
                    // unwind without waiting.
                    None => true,
                }
            };
            if exited {
                return;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    async fn elevate_initial_boot_error(self: &Arc<Self>, original: ClientError) -> ClientError {
        // Already structured - nothing to elevate.
        match &original {
            ClientError::BootFailure { .. } => return original,
            ClientError::Service(ServiceError::BootFailure { .. }) => return original,
            _ => {}
        }

        // Take the running state and wait briefly for the child to exit. The
        // child has typically already exited by the time we're here (the EOF
        // / writer-task death is what produced `original`); we still need the
        // wait() to reap it and pull the exit code. A 1 s budget is plenty
        // for an already-dead process; on Linux `waitpid` returns
        // immediately on a zombie.
        let dying_state = {
            let mut guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            guard.take()
        };
        let Some(state) = dying_state else {
            return original;
        };
        let RunningState {
            mut child,
            stdin_tx,
            reader_handle,
            writer_handle,
            heartbeat_handle,
            generation: _,
        } = state;
        drop(stdin_tx);
        // Run the three handle joins concurrently rather than sequentially.
        // 200 ms is the budget per handle but the handles are independent
        // (each is awaiting its own task); concurrent join keeps total
        // wall-clock at ~200 ms instead of ~600 ms in the worst case
        // where every join hits the timeout. Empirically these handles
        // are usually already done by the time we reach elevate (the
        // EOF / writer-task death is what produced the original error),
        // so the timeouts almost never fire. Trimming the worst case
        // matters because tests around this path budget against
        // `health.ping`'s 5 s timeout + this elevation - any reduction
        // here gives the test a wider margin against parallel-scheduling
        // jitter.
        let abort_budget = Duration::from_millis(200);
        let _ = tokio::join!(
            tokio::time::timeout(abort_budget, reader_handle),
            tokio::time::timeout(abort_budget, writer_handle),
            tokio::time::timeout(abort_budget, heartbeat_handle),
        );

        let Some(status) = wait_with_kill_watchdog(&mut child, Duration::from_secs(1)).await
        else {
            return original;
        };

        let classification = BootClassification::from_exit_code(status.code());
        log::info!(
            "elevated initial-boot error to {classification:?} (original: {original})",
        );
        ClientError::BootFailure { classification }
    }

    fn install_running_state(&self, spawned: SpawnedSubprocess, generation: u32) {
        let mut guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        *guard = Some(RunningState {
            child: spawned.child,
            stdin_tx: spawned.stdin_tx,
            reader_handle: spawned.reader_handle,
            writer_handle: spawned.writer_handle,
            heartbeat_handle: spawned.heartbeat_handle,
            generation,
        });
        self.current_generation.store(generation, Ordering::SeqCst);
    }

    pub async fn request<R>(&self, params: RequestParams) -> Result<R, ClientError>
    where
        R: DeserializeOwned,
    {
        let value = self.request_value(params).await?;
        serde_json::from_value(value).map_err(ClientError::from)
    }

    /// Submit a resolved action plan for execution.
    ///
    /// The Service handler validates the plan, journals it into the
    /// `action_jobs` / `action_job_ops` tables, and returns
    /// `ActionPlanAck { plan_id, journaled: true }` once the journal
    /// transaction has committed. Per-operation `OperationOutcome`
    /// notifications stream from the worker; `ActionCompleted` closes
    /// the stream when every op has reached terminal status.
    ///
    /// 5 s timeout (handler is just validate + journal + signal); the
    /// worker has no IPC timeout.
    pub async fn execute_plan(
        &self,
        plan: service_api::ActionWirePlan,
    ) -> Result<service_api::ActionPlanAck, ClientError> {
        self.request(RequestParams::ActionExecutePlan { plan }).await
    }

    /// Look up the journaled status of a previously-submitted plan.
    ///
    /// Drives the post-respawn reconciliation flow (Phase 2 plan scope
    /// item 11 / 18d): for every plan in `AckUnknown` state, the UI
    /// resolves to either `Acked` (response is `Journaled`) or
    /// `RollBack` (response is `NotFound`). The query is a fast
    /// SELECT against `action_jobs.job_id`; 5 s timeout is the
    /// conservative default in `RequestParams::ActionJobStatus`.
    pub async fn job_status(
        &self,
        plan_id: service_api::PlanId,
    ) -> Result<service_api::JobStatusResponse, ClientError> {
        self.request(RequestParams::ActionJobStatus { plan_id }).await
    }

    /// Mark every unread message in a contact's chat threads as read
    /// (Phase 2 task 15 / scope item 18c).
    ///
    /// The Service runs the local DB write inside the request handler
    /// and journals the affected threads as a quiet job. The worker
    /// dispatches provider mark-read against each thread and emits a
    /// final `ActionCompleted` (no per-operation `OperationOutcome`
    /// notifications - quiet jobs suppress them). UI fires this
    /// fire-and-forget on chat entry; the ack is informational.
    pub async fn mark_chat_read(
        &self,
        chat_email: String,
    ) -> Result<service_api::MarkChatReadAck, ClientError> {
        self.request(RequestParams::ActionMarkChatRead { chat_email })
            .await
    }

    /// Submit a compose-send for execution (Phase 2 task 13).
    ///
    /// Bytes-ownership transfer: the UI must have already written each
    /// attachment into `<app_data>/staging/<send_id>/<index>.bin`
    /// before calling this. The handler verifies SHA-256, atomically
    /// renames each file into the Service-owned vault, journals the
    /// send as a quiet `kind = 'send'` job, and returns `SendAck`. The
    /// UI's staging directory is its own to clean up after a
    /// successful ack; a Service crash before the ack returns an
    /// error and the staging directory is still load-bearing on the
    /// next attempt.
    ///
    /// 30 s timeout covers SHA-256 verify of typical attachment
    /// payloads. SMTP submit happens on the worker; the eventual
    /// `ActionCompleted` notification (matching `send_id` as
    /// `plan_id`) is the success/failure signal.
    pub async fn send_email(
        &self,
        request: service_api::SendWireRequest,
    ) -> Result<service_api::SendAck, ClientError> {
        self.request(RequestParams::ActionSend {
            request: Box::new(request),
        })
        .await
    }

    /// Send a fire-and-forget UI -> Service notification (Phase 2 plan
    /// scope item 11). No correlation map entry, no oneshot channel,
    /// no timeout. Returns `Ok(())` on successful enqueue into the
    /// outbound writer task; `Err(NotConnected)` if no Service is
    /// currently bound; `Err(ServiceCrashed)` if the writer task has
    /// already shut down.
    ///
    /// The Service runs notification handlers on a separate task pool
    /// with `Drop`-class admission (`NOTIFY_CAP`). If that pool is at
    /// capacity, the Service drops the inbound rather than queue. The
    /// UI's tick policy is the retry strategy: missing one tick is
    /// the documented best-effort guarantee.
    pub async fn send_notification(
        &self,
        notification: service_api::ClientNotification,
    ) -> Result<(), ClientError> {
        let envelope = service_api::JsonRpcClientNotification::new(&notification);
        let bytes = encode_message(&envelope)?;
        let stdin_tx = match self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            Some(state) => state.stdin_tx.clone(),
            None => return Err(ClientError::NotConnected),
        };
        if stdin_tx.send(bytes).await.is_err() {
            return Err(ClientError::ServiceCrashed);
        }
        Ok(())
    }

    pub async fn shutdown(&self) -> Result<(), ClientError> {
        // Tell handle_crash to bail rather than respawning if the dispatch
        // shutdown races with reader-EOF (the Service is exiting; the EOF
        // would otherwise look like a crash).
        self.is_shutting_down.store(true, Ordering::SeqCst);
        let request_result = self.request::<ShutdownResponse>(RequestParams::Shutdown).await;
        match request_result {
            Ok(_) => {
                if self.wait_for_exit(Duration::from_secs(2)).await {
                    return Ok(());
                }
            }
            Err(ClientError::Timeout) => {
                log::warn!("service shutdown request timed out, sending SIGTERM");
            }
            Err(error) => {
                log::warn!("service shutdown request failed: {error}");
            }
        }

        self.send_sigterm();
        if self.wait_for_exit(Duration::from_secs(5)).await {
            return Ok(());
        }
        self.kill_child();
        let _ = self.wait_for_exit(Duration::from_secs(1)).await;
        Ok(())
    }

    pub fn notifications(&self) -> ServiceNotificationReceiver {
        Arc::clone(&self.notifications)
    }

    /// Phase 3 task 14: kick a sync run for `account_id` and await its
    /// terminal `SyncResult`. The IPC ack carries a `run_id`; the
    /// caller subscribes to the `pending_syncs` slot keyed on it. Two
    /// callers issuing concurrent `start_sync` for the same account
    /// receive the same `run_id` from the Service (the second call's
    /// ack carries `already_in_flight: true`) and both subscribers
    /// resolve via the broadcast channel when `Notification::SyncCompleted`
    /// arrives. A fast `SyncCompleted` that races the post-ack
    /// subscriber is latched as `PendingSync::Completed` and consumed
    /// by the late subscriber instead of being dropped.
    pub async fn start_sync(&self, account_id: String) -> Result<SyncResult, ClientError> {
        let ack: SyncStartAck = self
            .request(RequestParams::SyncStartAccount {
                params: SyncStartAccountParams { account_id },
            })
            .await?;
        self.subscribe_or_consume(ack.run_id).await
    }

    /// Phase 5 task 9b: kick a calendar sync run for `account_id` and
    /// await its terminal `CalendarSyncResult`. Mirrors `start_sync`
    /// exactly - the IPC ack carries a `run_id`; the caller subscribes
    /// to the `pending_calendars` slot keyed on it; a fast
    /// `CalendarRunCompleted` that races the post-ack subscriber is
    /// latched and consumed by the late subscriber.
    pub async fn start_calendar_sync(
        &self,
        account_id: String,
    ) -> Result<CalendarSyncResult, ClientError> {
        let ack: CalendarStartAck = self
            .request(RequestParams::CalendarStartAccountSync {
                params: CalendarStartAccountSyncParams { account_id },
            })
            .await?;
        self.subscribe_or_consume_calendar(ack.run_id).await
    }

    /// Phase 6a: toggle calendar visibility via the
    /// `calendar.set_visibility` IPC. Replaces the deleted
    /// `Db::set_calendar_visibility` UI-side write surface.
    pub async fn set_calendar_visibility(
        &self,
        calendar_id: String,
        visible: bool,
    ) -> Result<(), ClientError> {
        let _ack: CalendarSetVisibilityAck = self
            .request(RequestParams::CalendarSetVisibility {
                params: CalendarSetVisibilityParams {
                    calendar_id,
                    visible,
                },
            })
            .await?;
        Ok(())
    }

    /// Phase 3 task 14: cancel any in-flight sync for `account_id` and
    /// await the terminal `SyncResult` (typically `Cancelled`). If the
    /// Service ack reports `was_in_flight: false` (no run was active),
    /// returns `SyncResult::Completed` immediately - there's nothing
    /// to await.
    ///
    /// Phase 5 task 9b: when the Service piggybacks calendar cancel
    /// (ack carries `calendar_run_id`), this also awaits the
    /// CalendarRunCompleted notification before returning so the
    /// account-deletion path's DB DELETE cannot race a calendar runner
    /// mid-write. The sync result is what's returned (the calendar
    /// outcome is best-effort - logged on failure but not surfaced).
    pub async fn cancel_and_await(
        &self,
        account_id: &str,
    ) -> Result<SyncResult, ClientError> {
        let ack: SyncCancelAck = self
            .request(RequestParams::SyncCancelAccount {
                params: SyncCancelAccountParams {
                    account_id: account_id.to_string(),
                },
            })
            .await?;
        let calendar_run_id = ack.calendar_run_id;

        // Await the calendar run terminal completion in parallel with
        // sync's. tokio::join! lets a slow sync cancel and a slow
        // calendar cancel overlap; the deletion path doesn't need to
        // distinguish them.
        let sync_future = async {
            match ack.run_id {
                Some(run_id) => self.subscribe_or_consume(run_id).await,
                None => Ok(SyncResult::Completed),
            }
        };
        let calendar_future = async {
            if let Some(run_id) = calendar_run_id {
                match self.subscribe_or_consume_calendar(run_id).await {
                    Ok(_) => {}
                    Err(e) => {
                        log::warn!(
                            "[calendar] cancel_and_await: failed to await CalendarRunCompleted: {e:?}",
                        );
                    }
                }
            }
        };
        let (sync_result, ()) = tokio::join!(sync_future, calendar_future);
        sync_result
    }

    async fn subscribe_or_consume(
        &self,
        run_id: SyncRunId,
    ) -> Result<SyncResult, ClientError> {
        sweep_latched_completed(&self.pending_syncs);
        let rx_to_await = {
            let mut guard = self
                .pending_syncs
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            match guard.entry(run_id) {
                Entry::Occupied(e) => match e.get() {
                    PendingSync::Completed { result, .. } => {
                        let result = result.clone();
                        e.remove();
                        return Ok(result);
                    }
                    PendingSync::Pending(tx) => tx.subscribe(),
                },
                Entry::Vacant(v) => {
                    let (tx, rx) = broadcast::channel(SYNC_BROADCAST_CAPACITY);
                    v.insert(PendingSync::Pending(tx));
                    rx
                }
            }
        };
        let mut rx = rx_to_await;
        rx.recv().await.map_err(|_| ClientError::ServiceCrashed)
    }

    /// Phase 5 task 9b: mirror of `subscribe_or_consume` for calendar
    /// runs. Pendings keyed by `CalendarRunId`; notifications routed by
    /// `route_calendar_run_completed`.
    async fn subscribe_or_consume_calendar(
        &self,
        run_id: CalendarRunId,
    ) -> Result<CalendarSyncResult, ClientError> {
        sweep_latched_calendar_completed(&self.pending_calendars);
        let rx_to_await = {
            let mut guard = self
                .pending_calendars
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            match guard.entry(run_id) {
                Entry::Occupied(e) => match e.get() {
                    PendingCalendar::Completed { result, .. } => {
                        let result = result.clone();
                        e.remove();
                        return Ok(result);
                    }
                    PendingCalendar::Pending(tx) => tx.subscribe(),
                },
                Entry::Vacant(v) => {
                    let (tx, rx) = broadcast::channel(SYNC_BROADCAST_CAPACITY);
                    v.insert(PendingCalendar::Pending(tx));
                    rx
                }
            }
        };
        let mut rx = rx_to_await;
        rx.recv().await.map_err(|_| ClientError::ServiceCrashed)
    }

    /// Reader-task helper: route an incoming `SyncCompleted`
    /// notification to its waiter set. If no waiters have subscribed
    /// yet, latch the result as `PendingSync::Completed` so a
    /// late subscriber can consume it; otherwise broadcast to every
    /// active subscriber and remove the entry.
    pub(crate) fn route_sync_completed(&self, completed: SyncCompleted) {
        let mut guard = self
            .pending_syncs
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        // Sweep aged latched entries on every route. The cancel-on-delete
        // and `cancel_account` prune-on-finished paths can produce a
        // route with no awaiter; bounding map size to TTL keeps the
        // latch protection without unbounded growth across long sessions.
        guard.retain(|_, entry| match entry {
            PendingSync::Completed { latched_at, .. } => {
                now.duration_since(*latched_at) < LATCHED_COMPLETED_TTL
            }
            PendingSync::Pending(_) => true,
        });
        match guard.entry(completed.run_id) {
            Entry::Occupied(mut e) => match e.get_mut() {
                PendingSync::Pending(tx) => {
                    let _ = tx.send(completed.result);
                    e.remove();
                }
                PendingSync::Completed { .. } => {
                    log::warn!(
                        "duplicate SyncCompleted for run_id {}; dropping",
                        completed.run_id
                    );
                }
            },
            Entry::Vacant(v) => {
                v.insert(PendingSync::Completed {
                    result: completed.result,
                    latched_at: now,
                });
            }
        }
    }

    /// Reader-task helper: route an incoming `CalendarRunCompleted`
    /// notification to its waiter set. Mirror of `route_sync_completed`
    /// for calendar runs.
    ///
    /// Sweeps aged latched entries before inserting. Kick-driven calendar
    /// runs always complete with no awaiter (the kick path on `SyncTick`
    /// never subscribes), so without an in-route sweep every kick that
    /// completes adds a latched entry that only ages out when an
    /// explicit-request awaiter calls `subscribe_or_consume_calendar`.
    /// Explicit-request paths are uncommon, so the map would otherwise
    /// grow with every kick until the next respawn. Sweeping on each
    /// route keeps the latch protecting genuine awaiter-after-completion
    /// races (cancel-on-delete, post-account-add) without unbounded growth.
    pub(crate) fn route_calendar_run_completed(&self, completed: CalendarRunCompleted) {
        let mut guard = self
            .pending_calendars
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        guard.retain(|_, entry| match entry {
            PendingCalendar::Completed { latched_at, .. } => {
                now.duration_since(*latched_at) < LATCHED_COMPLETED_TTL
            }
            PendingCalendar::Pending(_) => true,
        });
        match guard.entry(completed.run_id) {
            Entry::Occupied(mut e) => match e.get_mut() {
                PendingCalendar::Pending(tx) => {
                    let _ = tx.send(completed.result);
                    e.remove();
                }
                PendingCalendar::Completed { .. } => {
                    log::warn!(
                        "duplicate CalendarRunCompleted for run_id {}; dropping",
                        completed.run_id
                    );
                }
            },
            Entry::Vacant(v) => {
                v.insert(PendingCalendar::Completed {
                    result: completed.result,
                    latched_at: now,
                });
            }
        }
    }

    /// Live Service-incarnation counter. The reader task tags every
    /// notification with the generation it captured at spawn time;
    /// `notification_should_dispatch` compares the tag against this value
    /// to drop notifications from a dying-but-still-flushing reader after
    /// a respawn (scope item 20 of `phase-1.5-plan.md`).
    ///
    /// `pub` (not `pub(crate)`) because integration tests in
    /// `crates/app/tests/service_subprocess.rs` live in a separate crate
    /// and need to assert respawn bumped this counter end-to-end. The
    /// counter exposes only the running incarnation count, no secret
    /// state.
    pub fn current_generation(&self) -> u32 {
        self.current_generation.load(Ordering::SeqCst)
    }

    /// Returns the child Service's PID while the child is still alive.
    /// Used by integration tests to verify that the OS-level process exits
    /// after explicit shutdown or after the client is dropped.
    pub fn child_pid(&self) -> Option<u32> {
        let guard = self.state.lock().ok()?;
        let state = guard.as_ref()?;
        state.child.id()
    }

    async fn request_value(
        &self,
        params: RequestParams,
    ) -> Result<serde_json::Value, ClientError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let request = JsonRpcRequest::new(id, &params);
        let bytes = encode_message(&request)?;
        let (tx, rx) = oneshot::channel();
        self.pending.insert(id, tx);
        let mut guard = PendingGuard::new(&self.pending, id);

        // Clone stdin_tx out of the lock so we don't hold the std::sync::Mutex
        // across the .send().await below (the `await_holding_lock` clippy
        // lint is denied workspace-wide).
        let stdin_tx = match self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            Some(state) => state.stdin_tx.clone(),
            None => return Err(ClientError::NotConnected),
        };

        if stdin_tx.send(bytes).await.is_err() {
            return Err(ClientError::ServiceCrashed);
        }

        let response = match params.timeout() {
            RequestTimeoutKind::Finite(timeout) => match tokio::time::timeout(timeout, rx).await {
                Ok(response) => response,
                Err(_) => return Err(ClientError::Timeout),
            },
            RequestTimeoutKind::Infinite => rx.await,
        };

        guard.disarm();
        match response {
            Ok(result) => result,
            Err(_) => Err(ClientError::ServiceCrashed),
        }
    }

    async fn wait_for_exit(&self, timeout: Duration) -> bool {
        let started = Instant::now();
        while started.elapsed() < timeout {
            if self.try_wait_child() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        self.try_wait_child()
    }

    fn try_wait_child(&self) -> bool {
        let Ok(mut guard) = self.state.lock() else {
            return true;
        };
        let Some(state) = guard.as_mut() else {
            // No live state: child either never spawned or already torn down.
            return true;
        };
        match state.child.try_wait() {
            Ok(Some(status)) => {
                log::info!("service exited with status {status}");
                true
            }
            Ok(None) => false,
            Err(error) => {
                log::warn!("failed to wait for service child: {error}");
                true
            }
        }
    }

    fn send_sigterm(&self) {
        #[cfg(unix)]
        {
            let Ok(guard) = self.state.lock() else {
                return;
            };
            let Some(state) = guard.as_ref() else {
                return;
            };
            let Some(pid) = state.child.id() else {
                return;
            };
            let pid = match i32::try_from(pid) {
                Ok(pid) => pid,
                Err(error) => {
                    log::warn!("service pid conversion failed: {error}");
                    return;
                }
            };
            // SAFETY: SIGTERM on a PID we have an open Child handle for; the
            // kernel keeps the PID stable until we wait() on the Child.
            let result = unsafe { libc::kill(pid, libc::SIGTERM) };
            if result != 0 {
                log::warn!(
                    "failed to send SIGTERM to service: {}",
                    std::io::Error::last_os_error(),
                );
            }
        }
    }

    fn kill_child(&self) {
        let Ok(mut guard) = self.state.lock() else {
            return;
        };
        let Some(state) = guard.as_mut() else {
            return;
        };
        if let Err(error) = state.child.start_kill() {
            log::warn!("failed to kill service child: {error}");
        }
    }

    /// Crash handler. Invoked by the reader task on EOF/frame error and by
    /// the heartbeat task on a hard error (per scope item 16 of
    /// `phase-1.5-plan.md`). The `dying_generation` parameter is the
    /// reader-or-heartbeat task's own captured generation; if a newer
    /// incarnation has already taken state (concurrent reader EOF +
    /// heartbeat error, or this is a stale-task callback after a respawn
    /// has already happened), the call bails immediately.
    ///
    /// The respawn algorithm:
    /// 1. Take the dying state (if its generation still matches; otherwise
    ///    another `handle_crash` for a newer incarnation already ran).
    /// 2. Bump current_generation, fail pending requests, drop stdin_tx,
    ///    wait for the dying child with a 5 s watchdog (escalate to
    ///    start_kill on timeout), abort the dying tasks.
    /// 3. Bail if `is_shutting_down` was set (Drop is in progress).
    /// 4. Bail if `respawn_config` is None (test single-shot path) - the
    ///    client tears down on first crash.
    /// 5. Bail if the initial boot hasn't completed yet (first BootReady
    ///    has not been captured): `run_spawn_flow` owns the Terminal-on-
    ///    initial-boot-failure surface; respawning here would race it.
    /// 6. Sleep 1 s (the v1 crashloop bound).
    /// 7. If the dying exit code matches a deterministic `BootExitCode`
    ///    variant (Migration / KeyLoad / AnotherInstance), emit `Terminal`
    ///    and stop - respawning would just hit the same failure.
    /// 8. Launch a replacement subprocess. On any launch failure, emit
    ///    `Terminal`. Otherwise install state, run version-check ping,
    ///    re-emit `ChildSpawned`, run `boot.ready`, schema-version sanity
    ///    check, re-emit `BootReady`. Any failure during these steps emits
    ///    `Terminal`.
    async fn handle_crash(self: Arc<Self>, dying_generation: u32) {
        // Pre-checks BEFORE taking the running state. If we're going to bail
        // (test single-shot, shutting down, or pre-BootReady deferral to
        // run_spawn_flow), leave the state alone so the path that owns the
        // terminal-failure surface can wait on the dying child for its
        // exit code. Without this, run_spawn_flow's `elevate_initial_boot
        // _error` would find the state already taken and miss the
        // classification.

        if self.is_shutting_down.load(Ordering::SeqCst) {
            log::debug!("client is shutting down; not respawning");
            return;
        }
        let Some(respawn) = self.respawn_config.as_ref() else {
            // Test single-shot path: client tears down on first crash, the
            // test owns orchestration. No respawn machinery; bail before
            // touching state.
            return;
        };
        if respawn
            .first_boot_ready
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_none()
        {
            // Initial boot has not produced a BootReady yet. run_spawn_flow's
            // pending request returned Err and its `elevate_initial_boot_error`
            // path will take the state, wait on the child, and emit a
            // classified Terminal. Bumping current_generation here would also
            // be premature - there's no replacement incarnation to discriminate
            // notifications against.
            log::debug!(
                "handle_crash(gen={dying_generation}): initial boot not yet complete; \
                 deferring to run_spawn_flow",
            );
            return;
        }

        let dying_state = {
            let mut guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            match guard.as_ref() {
                Some(state) if state.generation == dying_generation => guard.take(),
                _ => None,
            }
        };
        let Some(dying_state) = dying_state else {
            log::debug!(
                "handle_crash(gen={dying_generation}): state already replaced; bailing",
            );
            return;
        };

        // Bump current_generation: any notification still queued by the
        // dying reader task (tagged with `dying_generation`) will fail the
        // dispatch-side generation check (item 15) and be dropped.
        self.current_generation.fetch_add(1, Ordering::SeqCst);
        fail_pending(&self.pending);
        // Phase 3 task 14: drop every Pending broadcast::Sender so
        // any in-flight start_sync / cancel_and_await future surfaces
        // `Err(ClientError::ServiceCrashed)` rather than parking
        // forever across the respawn. The next SyncTick re-issues
        // start_sync against the new incarnation, generating a fresh
        // run_id.
        fail_pending_syncs(&self.pending_syncs);
        // Phase 5 task 9b: same shape for calendar awaiters.
        fail_pending_calendars(&self.pending_calendars);

        let RunningState {
            mut child,
            stdin_tx,
            reader_handle,
            writer_handle,
            heartbeat_handle,
            generation: _,
        } = dying_state;
        drop(stdin_tx);

        let exit_status = wait_with_kill_watchdog(&mut child, Duration::from_secs(5)).await;
        // Abort the helper tasks before awaiting them; without explicit
        // .abort() calls a still-running heartbeat with a stale
        // weak_client.upgrade() can idle for up to its 30 s interval
        // before its next tick fails-and-exits. Aborting first then
        // awaiting with a 200 ms watchdog gets us the same correctness
        // bound but tightens the post-crash cleanup window.
        reader_handle.abort();
        writer_handle.abort();
        heartbeat_handle.abort();
        let abort_budget = Duration::from_millis(200);
        let _ = tokio::time::timeout(abort_budget, reader_handle).await;
        let _ = tokio::time::timeout(abort_budget, writer_handle).await;
        let _ = tokio::time::timeout(abort_budget, heartbeat_handle).await;
        log::warn!(
            "service crashed (gen={dying_generation}, exit_status={exit_status:?})",
        );

        if self.is_shutting_down.load(Ordering::SeqCst) {
            log::debug!("client is shutting down; not respawning");
            return;
        }

        // 1-second cooldown serves two purposes:
        // (a) bounds CPU under transient crashes (the v1 crashloop guard;
        //     Phase 8 replaces with exponential backoff + crashloop
        //     detection); and
        // (b) gives the dying child's fs2 file lock time to be released by
        //     the kernel before the replacement spawn tries to acquire it.
        //     Without this, the new Service can race the dying child and
        //     exit AnotherInstanceRunning, which under our terminal-failure
        //     policy is fatal - turning a recoverable crash into a hard
        //     exit. Plan item 6 (Architecture / Service-side boot sequence)
        //     calls this race out explicitly.
        tokio::time::sleep(Duration::from_secs(1)).await;
        if self.is_shutting_down.load(Ordering::SeqCst) {
            return;
        }

        // A clean exit (code 0) with `is_shutting_down` already false means
        // the Service exited of its own accord without the UI requesting
        // it - a contract violation per
        // `BootClassification::from_exit_code`'s doc-comment ("Service that
        // exited 0 without going through `client.shutdown()` is broken").
        // Emit Terminal directly rather than letting code 0 fall through
        // the deterministic-boot-failure check below (`from_i32(0)` is
        // None so the path would proceed to crashloop accounting + a
        // respawn that would either hit the same self-exit again or
        // succeed-then-self-exit forever). The is_shutting_down branch at
        // :811 already short-circuits the legitimate Shutdown-ack races.
        if let Some(status) = exit_status
            && status.code() == Some(0)
        {
            log::error!(
                "service exited cleanly (code 0) without a Shutdown request; emitting Terminal",
            );
            let _ = respawn
                .spawn_event_tx
                .send(SpawnEvent::Terminal(ClientError::BootFailure {
                    classification: BootClassification::UnexpectedExit { code: Some(0) },
                }))
                .await;
            return;
        }

        // If the dying Service exited with a deterministic boot-failure
        // code, respawning would hit the same failure. The
        // single-instance-lock case (AnotherInstanceRunning) is interesting:
        // the dying Service is gone, so the lock is free; but if the dying
        // Service exited with that code, somebody else holds the lock and
        // respawning still won't help. Either way, emit Terminal and stop.
        if let Some(status) = exit_status
            && let Some(code) = status.code()
            && let Some(boot_code) = BootExitCode::from_i32(code)
        {
            log::error!(
                "service exited with deterministic boot failure {boot_code:?}; emitting Terminal",
            );
            let _ = respawn
                .spawn_event_tx
                .send(SpawnEvent::Terminal(ClientError::BootFailure {
                    classification: BootClassification::BootFailure { code: boot_code },
                }))
                .await;
            return;
        }

        // Crashloop guard. The 1 s sleep above bounds CPU at one Service
        // per second; under signal-killed crashloops that is still enough
        // to fill `<app_data>/logs/` with thousands of files per hour and
        // exhaust the rest-of-day's disk budget on a busy host. The
        // sliding-window tracker fires Terminal after CRASHLOOP_THRESHOLD
        // respawns within CRASHLOOP_WINDOW. The classification carries the
        // dying child's exit code so the user-visible message names what
        // kind of exit was repeating.
        let now = Instant::now();
        if record_respawn_and_check_crashloop(&self.respawn_attempts, now) {
            let classification =
                BootClassification::from_exit_code(exit_status.and_then(|s| s.code()));
            log::error!(
                "crashloop detected ({CRASHLOOP_THRESHOLD} respawns within {CRASHLOOP_WINDOW:?}); \
                 terminating with {classification:?}",
            );
            let _ = respawn
                .spawn_event_tx
                .send(SpawnEvent::Terminal(ClientError::BootFailure {
                    classification,
                }))
                .await;
            return;
        }

        if let Err(error) = self.respawn(respawn).await {
            log::error!("respawn failed: {error}");
            let _ = respawn
                .spawn_event_tx
                .send(SpawnEvent::Terminal(error))
                .await;
        }
    }

    async fn respawn(self: &Arc<Self>, respawn: &RespawnConfig) -> Result<(), ClientError> {
        let new_gen = self.current_generation.load(Ordering::SeqCst);
        let extra_arg_refs: Vec<&str> =
            respawn.extra_args.iter().map(String::as_str).collect();

        // Early bail before launch_subprocess fires. handle_crash already
        // checked is_shutting_down before sleeping, but Drop could have
        // fired during the 1 s cooldown; without this check we'd
        // briefly fork a Service process that immediately gets torn
        // down, contending the lockfile for as long as the new process
        // takes to acquire-then-release. The post-launch check below
        // remains as defense-in-depth for the narrower race that opens
        // after we cross this gate.
        if self.is_shutting_down.load(Ordering::SeqCst) {
            log::debug!("respawn skipped: client shutting down");
            return Ok(());
        }

        let spawned = launch_subprocess(
            &respawn.binary_path,
            &respawn.app_data_dir,
            &extra_arg_refs,
            &self._process_guard,
            Arc::clone(&self.pending),
            Arc::clone(&self.next_id),
            Arc::clone(&self.notifications),
            Arc::downgrade(self),
            new_gen,
        )
        .await?;

        // Race window: Drop fired between launch_subprocess returning and
        // here. Tear down the new spawn directly (we own everything in
        // `spawned`) rather than installing it. start_kill only sends the
        // signal; without an awaited wait() the OS retains the PID briefly
        // as a zombie. Give it a 200 ms budget so the lockfile is released
        // (the dying parent process has already torn down its own state).
        if self.is_shutting_down.load(Ordering::SeqCst) {
            spawned.reader_handle.abort();
            spawned.writer_handle.abort();
            spawned.heartbeat_handle.abort();
            let mut child = spawned.child;
            if let Err(error) = child.start_kill() {
                log::warn!("start_kill on aborted-respawn child failed: {error}");
            }
            let _ = tokio::time::timeout(Duration::from_millis(200), child.wait()).await;
            return Ok(());
        }
        self.install_running_state(spawned, new_gen);

        // Run the post-install handshake; on failure tear down the freshly
        // installed RunningState before propagating. Without this teardown
        // a Terminal classification leaves the new Service running unmanaged
        // (e.g. VersionMismatch: a fully healthy Service stays alive against
        // an unbumped UI; SchemaVersionChanged: the Service holds a DB whose
        // schema we just declared incompatible) until the App drops the Arc.
        match self.handshake_post_install(respawn, new_gen).await {
            Ok(()) => Ok(()),
            Err(error) => {
                self.tear_down_installed_state(&error).await;
                Err(error)
            }
        }
    }

    async fn handshake_post_install(
        self: &Arc<Self>,
        respawn: &RespawnConfig,
        new_gen: u32,
    ) -> Result<(), ClientError> {
        let ping: HealthPingResponse = self.request(RequestParams::HealthPing).await?;
        if ping.version != PROTOCOL_VERSION {
            return Err(ClientError::VersionMismatch {
                ui: PROTOCOL_VERSION,
                service: ping.version,
            });
        }
        log::info!("Service respawned (pid={}, gen={new_gen})", ping.pid);

        // The shared Arc<NotificationQueue> survived the respawn and the
        // App's notification subscription is still draining it. Re-emitting
        // ChildSpawned is informational so callers that want to log the
        // respawn can observe it; the App's subscription does not need to
        // be re-established.
        let _ = respawn
            .spawn_event_tx
            .send(SpawnEvent::ChildSpawned(Arc::clone(self)))
            .await;

        let response: BootReadyResponse = self.request(RequestParams::BootReady).await?;

        // Schema-version sanity check vs the first BootReady captured by
        // run_spawn_flow. The contract: `handle_crash` defers respawn when
        // `first_boot_ready` is None (initial boot's `run_spawn_flow` owns
        // the Terminal-on-failure surface), so reaching the None arm here
        // means a refactor broke that invariant. We surface that as a
        // distinct Terminal failure rather than capturing the response as
        // a new baseline - the latter would silently lose binary-swap
        // detection on every subsequent respawn because future comparisons
        // would be against the newly-captured value, not the original.
        // See ClientError::SchemaBaselineMissing for the rationale on
        // choosing Terminal over `unreachable!()`.
        enum SchemaCheck {
            Ok,
            Mismatch { was: u32, now: u32 },
            BaselineMissing,
        }
        let check = {
            let guard = respawn
                .first_boot_ready
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            match guard.as_ref() {
                Some(first) if first.schema_version != response.schema_version => {
                    SchemaCheck::Mismatch {
                        was: first.schema_version,
                        now: response.schema_version,
                    }
                }
                Some(_) => SchemaCheck::Ok,
                None => SchemaCheck::BaselineMissing,
            }
        };
        match check {
            SchemaCheck::Ok => {}
            SchemaCheck::Mismatch { was, now } => {
                log::error!(
                    "respawn schema_version changed (was {was}, now {now}); binary swap detected",
                );
                return Err(ClientError::SchemaVersionChanged { was, now });
            }
            SchemaCheck::BaselineMissing => {
                log::error!(
                    "respawn observed first_boot_ready=None; handle_crash should have deferred. \
                     Treating as Terminal: binary-swap detection cannot proceed without a baseline.",
                );
                return Err(ClientError::SchemaBaselineMissing);
            }
        }

        let _ = respawn
            .spawn_event_tx
            .send(SpawnEvent::BootReady(response))
            .await;
        Ok(())
    }

    /// Tear down a `RunningState` that was just installed but failed the
    /// post-install handshake. Mirrors the early-bail teardown at the top
    /// of `respawn()` but operates on installed state rather than a fresh
    /// `SpawnedSubprocess`. Used to keep `Terminal` from leaving an
    /// unmanaged child process alive across the iced unwind window.
    async fn tear_down_installed_state(&self, reason: &ClientError) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(state) = state {
            let RunningState {
                mut child,
                stdin_tx,
                reader_handle,
                writer_handle,
                heartbeat_handle,
                ..
            } = state;
            log::warn!(
                "tearing down respawned Service after post-install failure: {reason}",
            );
            reader_handle.abort();
            writer_handle.abort();
            heartbeat_handle.abort();
            drop(stdin_tx);
            if let Err(error) = child.start_kill() {
                log::warn!(
                    "start_kill on post-install-failed child failed: {error}",
                );
            }
            let _ = tokio::time::timeout(Duration::from_millis(200), child.wait()).await;
        }
    }
}

impl Drop for ServiceClient {
    fn drop(&mut self) {
        self.is_shutting_down.store(true, Ordering::SeqCst);
        let dying_state = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();

        if let Some(state) = dying_state {
            let RunningState {
                mut child,
                stdin_tx,
                reader_handle,
                writer_handle,
                heartbeat_handle,
                ..
            } = state;
            reader_handle.abort();
            writer_handle.abort();
            heartbeat_handle.abort();
            drop(stdin_tx);

            // The aborted writer task carries the ChildStdin handle; awaiting
            // its abort lets the pipe close so the Service sees EOF and exits
            // cleanly. Without driving the runtime here, the abort never makes
            // progress before this Drop returns and we'd fall through to
            // SIGKILL every time on a busy worker. block_in_place +
            // Handle::block_on works on a multi-threaded runtime;
            // single-threaded falls back to polling.
            //
            // Single-threaded fallback caveat: on a `current_thread` runtime,
            // `runtime_for_block_on` returns None, the writer task can't run
            // because we're occupying the only worker, ChildStdin doesn't
            // close, the Service never sees EOF, and we go straight to SIGKILL
            // after the 1.2 s polling budget. Production uses a multi-thread
            // runtime (iced's daemon, plus the Service's own runtime), so
            // this only bites tests that pin a current_thread flavor.
            match runtime_for_block_on() {
                Some(handle) => {
                    tokio::task::block_in_place(|| {
                        handle.block_on(async_drop_wait(
                            &mut child,
                            reader_handle,
                            writer_handle,
                            heartbeat_handle,
                        ));
                    });
                }
                None => {
                    drop((reader_handle, writer_handle, heartbeat_handle));
                    poll_for_exit_blocking(&mut child, Duration::from_millis(1200));
                }
            }

            if !try_wait_child_owned(&mut child) {
                if let Err(error) = child.start_kill() {
                    log::warn!("failed to kill service child during Drop: {error}");
                }
                poll_for_exit_blocking(&mut child, Duration::from_millis(500));
            }
        }

        // Unblock any subscription consumer still parked in `recv()` so it
        // can unwind cleanly instead of waiting on a queue that will never
        // see another producer.
        self.notifications.close();
        fail_pending(&self.pending);
    }
}

async fn async_drop_wait(
    child: &mut Child,
    reader_handle: tokio::task::JoinHandle<()>,
    writer_handle: tokio::task::JoinHandle<()>,
    heartbeat_handle: tokio::task::JoinHandle<()>,
) {
    let abort_deadline = Duration::from_millis(200);
    let abort_started = Instant::now();
    for handle in [reader_handle, writer_handle, heartbeat_handle] {
        let remaining = abort_deadline.saturating_sub(abort_started.elapsed());
        if remaining.is_zero() {
            continue;
        }
        let _ = tokio::time::timeout(remaining, handle).await;
    }
    let exit_deadline = Duration::from_secs(1);
    let exit_started = Instant::now();
    while exit_started.elapsed() < exit_deadline {
        if try_wait_child_owned(child) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn try_wait_child_owned(child: &mut Child) -> bool {
    match child.try_wait() {
        Ok(Some(status)) => {
            log::info!("service exited with status {status}");
            true
        }
        Ok(None) => false,
        Err(error) => {
            log::warn!("failed to wait for service child: {error}");
            true
        }
    }
}

fn poll_for_exit_blocking(child: &mut Child, deadline: Duration) {
    let started = Instant::now();
    while started.elapsed() < deadline {
        if try_wait_child_owned(child) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Wait for `child` with a watchdog. On timeout, escalate to `start_kill`
/// and try one more short wait. Returns `Some(status)` if the child exited;
/// `None` if `wait` errored or both attempts timed out (rare - the OS
/// should not block on a kill -9'd process).
async fn wait_with_kill_watchdog(
    child: &mut Child,
    watchdog: Duration,
) -> Option<std::process::ExitStatus> {
    match tokio::time::timeout(watchdog, child.wait()).await {
        Ok(Ok(status)) => Some(status),
        Ok(Err(error)) => {
            log::warn!("wait failed on dying child: {error}");
            None
        }
        Err(_) => {
            log::warn!("dying child exceeded {watchdog:?}; escalating to start_kill");
            if let Err(error) = child.start_kill() {
                log::warn!("start_kill on dying child failed: {error}");
            }
            tokio::time::timeout(Duration::from_secs(1), child.wait())
                .await
                .ok()
                .and_then(Result::ok)
        }
    }
}

fn runtime_for_block_on() -> Option<tokio::runtime::Handle> {
    let handle = tokio::runtime::Handle::try_current().ok()?;
    if matches!(
        handle.runtime_flavor(),
        tokio::runtime::RuntimeFlavor::MultiThread,
    ) {
        Some(handle)
    } else {
        None
    }
}

struct SpawnedSubprocess {
    child: Child,
    stdin_tx: mpsc::Sender<Vec<u8>>,
    reader_handle: tokio::task::JoinHandle<()>,
    writer_handle: tokio::task::JoinHandle<()>,
    heartbeat_handle: tokio::task::JoinHandle<()>,
}

/// Spawn a subprocess and the three IPC tasks (reader/writer/heartbeat).
/// Used both at initial spawn and on respawn; the only meaningful difference
/// across calls is the `generation` value the reader and heartbeat tasks
/// capture and tag their output with.
///
/// `weak_client` is held weakly (rather than as `Arc<ServiceClient>`) so the
/// reader and heartbeat tasks do NOT keep the client alive. They upgrade the
/// Weak only at crash time to spawn `handle_crash`. If the App has dropped
/// the client (Drop has fired), the upgrade returns None and the task simply
/// exits without firing a respawn.
#[allow(clippy::too_many_arguments)]
async fn launch_subprocess(
    binary: &Path,
    app_data_dir: &Path,
    extra_args: &[&str],
    process_guard: &process_lifetime::ProcessGuard,
    pending: Arc<DashMap<u64, oneshot::Sender<Result<serde_json::Value, ClientError>>>>,
    next_id: Arc<AtomicU64>,
    notifications: Arc<NotificationQueue>,
    weak_client: Weak<ServiceClient>,
    generation: u32,
) -> Result<SpawnedSubprocess, ClientError> {
    let mut command = Command::new(binary);
    command
        .arg("--service")
        .arg("--app-data-dir")
        .arg(app_data_dir);
    for arg in extra_args {
        command.arg(arg);
    }
    command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .kill_on_drop(false);
    process_lifetime::configure_command(&mut command)?;

    let mut child = command.spawn()?;
    // Assign immediately; on Windows the kill-on-job-close protection only
    // fires for processes already in the Job. The window between spawn and
    // assign is small but non-zero; explicit shutdown via the
    // request/timeout/SIGKILL path remains the primary teardown.
    process_guard.assign(&child)?;
    let stdin = child.stdin.take().ok_or(ClientError::NotConnected)?;
    let stdout = child.stdout.take().ok_or(ClientError::NotConnected)?;
    let (stdin_tx, stdin_rx) = mpsc::channel(STDIN_QUEUE_CAP);

    let reader_handle = tokio::spawn(reader_task(
        stdout,
        Arc::clone(&pending),
        notifications,
        Weak::clone(&weak_client),
        generation,
    ));
    let writer_handle = tokio::spawn(writer_task(stdin, stdin_rx));
    let heartbeat_handle = tokio::spawn(heartbeat_task(
        stdin_tx.clone(),
        pending,
        next_id,
        weak_client,
        generation,
    ));

    Ok(SpawnedSubprocess {
        child,
        stdin_tx,
        reader_handle,
        writer_handle,
        heartbeat_handle,
    })
}

/// Drives the two-phase spawn flow for `spawn_with_events`. Builds the
/// `RespawnConfig` upfront so the resulting client carries everything
/// `handle_crash` needs to launch a replacement Service. Runs `spawn_inner`
/// (which does the initial subprocess spawn + version-check ping), emits
/// `ChildSpawned`, issues `boot.ready`, captures the first response for
/// the respawn schema-version sanity check, and emits `BootReady` (or
/// `Terminal(error)` on any failure).
async fn run_spawn_flow(
    binary: PathBuf,
    app_data_dir: PathBuf,
    extra_args: Vec<String>,
    tx: mpsc::Sender<SpawnEvent>,
) {
    let respawn_config = RespawnConfig {
        binary_path: binary.clone(),
        app_data_dir: app_data_dir.clone(),
        extra_args: extra_args.clone(),
        spawn_event_tx: tx.clone(),
        first_boot_ready: Mutex::new(None),
    };

    let extra_arg_refs: Vec<&str> = extra_args.iter().map(String::as_str).collect();
    let client = match ServiceClient::spawn_inner(
        &binary,
        &app_data_dir,
        &extra_arg_refs,
        Some(respawn_config),
    )
    .await
    {
        Ok(client) => client,
        Err(error) => {
            let _ = tx.send(SpawnEvent::Terminal(error)).await;
            return;
        }
    };
    if tx
        .send(SpawnEvent::ChildSpawned(Arc::clone(&client)))
        .await
        .is_err()
    {
        // Receiver dropped before we could deliver ChildSpawned; the App
        // gave up on this spawn. Nothing useful to do; let the client drop.
        return;
    }
    let response: Result<BootReadyResponse, ClientError> =
        client.request_or_observe_child_exit(RequestParams::BootReady).await;
    match response {
        Ok(response) => {
            // Stash for the schema-version sanity check on every subsequent
            // respawn (handle_crash compares respawn boot.ready's
            // schema_version against this one).
            if let Some(rc) = client.respawn_config.as_ref() {
                let mut guard = rc
                    .first_boot_ready
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                *guard = Some(response.clone());
            }
            let _ = tx.send(SpawnEvent::BootReady(response)).await;
        }
        Err(error) => {
            // Elevate `ServiceCrashed` / `Timeout` into a structured
            // `BootFailure` if the Service exited with a known
            // `BootExitCode` (KeyLoadFailure / MigrationFailure /
            // AnotherInstanceRunning). Already-structured errors
            // (ServiceError::BootFailure from the boot.ready handler) pass
            // through unchanged.
            let elevated = client.elevate_initial_boot_error(error).await;
            let _ = tx.send(SpawnEvent::Terminal(elevated)).await;
        }
    }
}

/// Hard limit on consecutive `parse_service_message` failures. A Service
/// emitting persistent malformed JSON (memory corruption, a transitive C-FFI
/// dep that prints into the IPC pipe past the stdio defense, future protocol
/// skew that the wire codec doesn't recognise) is genuinely broken; without
/// a cap, the reader_task warns forever and never triggers the crash handler.
/// `FrameError` already triggers respawn at the framing layer; this caps the
/// JSON-decode-error path at a comparable threshold.
///
/// Counter resets on every successful parse so a single malformed line in a
/// healthy stream doesn't accumulate indefinitely.
const MAX_CONSECUTIVE_PARSE_ERRORS: u32 = 10;

async fn reader_task<R>(
    stdout: R,
    pending: Arc<DashMap<u64, oneshot::Sender<Result<serde_json::Value, ClientError>>>>,
    notifications: Arc<NotificationQueue>,
    weak_client: Weak<ServiceClient>,
    generation: u32,
) where
    R: tokio::io::AsyncRead + Unpin,
{
    let fail_syncs_via_client = |w: &Weak<ServiceClient>| {
        if let Some(c) = w.upgrade() {
            fail_pending_syncs(&c.pending_syncs);
            fail_pending_calendars(&c.pending_calendars);
        }
    };
    let mut lines = BoundedLineReader::new(stdout, service_api::MAX_FRAME_BYTES);
    let mut consecutive_parse_errors: u32 = 0;
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => match parse_service_message(&line) {
                Ok(parsed) => {
                    consecutive_parse_errors = 0;
                    match parsed {
                        ParsedServiceMessage::Response {
                            id: Some(id),
                            response,
                        } => {
                            dispatch_response(&pending, id, response);
                        }
                        ParsedServiceMessage::Response { id: None, response } => {
                            // An uncorrelated response means the Service
                            // answered a request whose id was null (parse
                            // error before the dispatch could correlate).
                            // Log only the discriminant and, for errors,
                            // the JSON-RPC code/message - never the
                            // success payload, which can carry user
                            // content (message bodies, search queries,
                            // etc.) once Phase 2+ methods land.
                            match response {
                                ServiceResponse::Success(_) => {
                                    log::warn!(
                                        "service returned uncorrelated success (payload redacted)",
                                    );
                                }
                                ServiceResponse::Error(error) => {
                                    log::warn!(
                                        "service returned uncorrelated error code={} message={}",
                                        error.code,
                                        error.message,
                                    );
                                }
                            }
                        }
                        ParsedServiceMessage::Notification(notification) => {
                            let upgraded = weak_client.upgrade();
                            let live = upgraded.as_ref().map(|c| c.current_generation());
                            if !reader_should_enqueue(generation, live) {
                                log::debug!(
                                    "reader_task(gen={generation}): dropping stale notification \
                                     (current_generation now {live:?})",
                                );
                                continue;
                            }
                            // Phase 3 task 14: SyncCompleted is consumed
                            // by `start_sync` / `cancel_and_await`
                            // futures via `pending_syncs`, not by the
                            // notification queue. Route + skip enqueue
                            // so a stray UI consumer does not see the
                            // raw frame.
                            if let Notification::SyncCompleted(c) = notification {
                                if let Some(client) = upgraded {
                                    client.route_sync_completed(c);
                                }
                                continue;
                            }
                            // Phase 5 task 9b: same routing for calendar
                            // run completions. Per-run_id awaiters
                            // (`start_calendar_sync`, `cancel_and_await`)
                            // resolve via `pending_calendars`. The UI
                            // never sees the raw frame; CalendarChanged
                            // is the dispatched-to-UI signal for view
                            // reload.
                            if let Notification::CalendarRunCompleted(c) = notification {
                                if let Some(client) = upgraded {
                                    client.route_calendar_run_completed(c);
                                }
                                continue;
                            }
                            let tagged = tag_notification_with_generation(notification, generation);
                            notifications.enqueue(tagged).await;
                        }
                    }
                }
                Err(error) => {
                    consecutive_parse_errors = consecutive_parse_errors.saturating_add(1);
                    log::warn!(
                        "failed to parse service message ({consecutive_parse_errors} consecutive): {error}",
                    );
                    if consecutive_parse_errors >= MAX_CONSECUTIVE_PARSE_ERRORS {
                        log::error!(
                            "reader_task(gen={generation}): {MAX_CONSECUTIVE_PARSE_ERRORS} \
                             consecutive parse errors; treating as Service crash",
                        );
                        fail_pending(&pending);
                        fail_syncs_via_client(&weak_client);
                        trigger_crash_handler(&weak_client, generation);
                        return;
                    }
                }
            },
            Ok(None) => {
                fail_pending(&pending);
                fail_syncs_via_client(&weak_client);
                trigger_crash_handler(&weak_client, generation);
                return;
            }
            Err(error) => {
                log::warn!("service stdout frame error: {error}");
                fail_pending(&pending);
                fail_syncs_via_client(&weak_client);
                trigger_crash_handler(&weak_client, generation);
                return;
            }
        }
    }
}

/// Sliding-window crashloop tracker. Pushes `now` onto the deque, evicts
/// entries older than `CRASHLOOP_WINDOW`, and returns `true` when the deque
/// reaches `CRASHLOOP_THRESHOLD` entries (i.e., this is the threshold-th
/// respawn within the window). Extracted for testability; the `now`
/// parameter lets unit tests drive the clock without depending on real
/// time.
fn record_respawn_and_check_crashloop(
    queue: &Mutex<VecDeque<Instant>>,
    now: Instant,
) -> bool {
    let mut guard = queue.lock().unwrap_or_else(PoisonError::into_inner);
    // Evict expired entries. checked_sub guards against the (effectively
    // never) startup case where Instant::now() is younger than the window.
    if let Some(cutoff) = now.checked_sub(CRASHLOOP_WINDOW) {
        while guard.front().is_some_and(|t| *t < cutoff) {
            guard.pop_front();
        }
    }
    guard.push_back(now);
    guard.len() >= CRASHLOOP_THRESHOLD
}

/// Pre-queue gate for the reader task. Returns `true` if the reader's
/// captured generation still matches the live `current_generation`. Used to
/// drop stale notifications BEFORE they enter the queue, where the per-phase
/// `CoalesceKey::BootProgress` coalesce policy would let a stale-generation
/// `BootProgress` overwrite a fresh-generation one in its slot - the
/// dispatch-side `notification_should_dispatch` check then drops the (stale)
/// replacement, taking the fresh update with it. The reader-side gate closes
/// that window.
///
/// The `live` parameter is `None` when `weak_client.upgrade()` returned None
/// (the App has dropped the client and we're shutting down anyway). Treat
/// that as "no live generation", which collapses to "drop" - the reader is
/// about to exit and the queue will drain.
pub(crate) fn reader_should_enqueue(captured: u32, live: Option<u32>) -> bool {
    matches!(live, Some(current) if current == captured)
}

/// Tag a notification with the reader's fixed generation so the dispatch
/// side (item 15) can drop notifications from a dying-but-still-flushing
/// reader.
///
/// **Phase 2+ contract**: per-variant tag/get logic now lives inside
/// `Notification::set_service_generation` / `service_generation` in
/// `crates/service-api/src/notification.rs`, where the get/set pair is
/// adjacent and the payload struct must implement `WithGeneration` to
/// participate. Side-effecting Phase 2 candidates that MUST be tagged:
/// - `action.completed` (action service)
/// - `push.event` (provider push delivery)
/// - `OperationOutcome` (any future generic outcome notification)
///
/// Untagged variants are reserved for synthetic / test-only payloads.
fn tag_notification_with_generation(
    mut notification: Notification,
    generation: u32,
) -> Notification {
    notification.set_service_generation(generation);
    notification
}

/// Dispatch-side guard for stale notifications. The reader task tags every
/// outgoing notification with its captured generation at enqueue time
/// (`tag_notification_with_generation`); `handle_crash` bumps
/// `ServiceClient::current_generation` before any new spawn. Notifications
/// queued by a dying reader (whose tag matches the now-stale generation)
/// must NOT be applied to UI state belonging to the new incarnation - they
/// would smear the splash with phases from the prior boot, or in Phase 2+
/// apply an `action.completed` to a respawned action service that never
/// dispatched the action in the first place.
///
/// Returns `true` iff the notification should be dispatched. Drops at
/// `debug` level otherwise. Variants whose `service_generation()` is `None`
/// (no cross-respawn discriminator needed) always dispatch.
pub(crate) fn notification_should_dispatch(
    notification: &Notification,
    current_generation: u32,
) -> bool {
    match notification.service_generation() {
        // `gen` is a reserved keyword in edition 2024.
        Some(tagged) if tagged != current_generation => {
            log::debug!(
                "dropping stale notification {} (tagged={tagged}, current={current_generation})",
                notification.method_name(),
            );
            false
        }
        _ => true,
    }
}

/// Spawn a `handle_crash` task on the client iff the Weak still points at
/// a live ServiceClient. The reader/heartbeat task itself returns
/// immediately after this; the spawned task drives the respawn or the
/// Terminal emission.
fn trigger_crash_handler(weak_client: &Weak<ServiceClient>, generation: u32) {
    let Some(client) = weak_client.upgrade() else {
        return;
    };
    tokio::spawn(client.handle_crash(generation));
}

fn dispatch_response(
    pending: &DashMap<u64, oneshot::Sender<Result<serde_json::Value, ClientError>>>,
    id: u64,
    response: ServiceResponse,
) {
    let Some((_, sender)) = pending.remove(&id) else {
        log::debug!("dropping response for unknown service request id {id}");
        return;
    };
    let result = match response {
        ServiceResponse::Success(value) => Ok(value),
        ServiceResponse::Error(error) => Err(client_error_from_rpc(error)),
    };
    let _ = sender.send(result);
}

fn client_error_from_rpc(error: JsonRpcErrorObject) -> ClientError {
    match error.try_into_service_error() {
        Ok(service_error) => ClientError::Service(service_error),
        Err(remaining) => ClientError::Service(ServiceError::Internal(remaining.message)),
    }
}

async fn writer_task<W>(mut stdin: W, mut stdin_rx: mpsc::Receiver<Vec<u8>>)
where
    W: AsyncWrite + Unpin,
{
    while let Some(bytes) = stdin_rx.recv().await {
        if let Err(error) = stdin.write_all(&bytes).await {
            log::warn!("service stdin write failed: {error}");
            break;
        }
        if let Err(error) = stdin.flush().await {
            log::warn!("service stdin flush failed: {error}");
            break;
        }
    }
}

async fn heartbeat_task(
    stdin_tx: mpsc::Sender<Vec<u8>>,
    pending: Arc<DashMap<u64, oneshot::Sender<Result<serde_json::Value, ClientError>>>>,
    next_id: Arc<AtomicU64>,
    weak_client: Weak<ServiceClient>,
    generation: u32,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    loop {
        interval.tick().await;
        let started = Instant::now();
        let result = request_value_raw(
            &stdin_tx,
            &pending,
            &next_id,
            RequestParams::HealthPing,
        )
        .await;
        match result {
            Ok(value) => match serde_json::from_value::<HealthPingResponse>(value) {
                Ok(_) => log::debug!("service heartbeat ok in {:?}", started.elapsed()),
                Err(error) => {
                    // Decode failure is a hard error per scope item 16's
                    // "anything else: hard, respawn" catch-all. A Service
                    // that answered `health.ping` with an unparseable body
                    // is wedged in a state we cannot recover from in-place.
                    log::warn!("service heartbeat decode failed: {error}; triggering respawn");
                    trigger_crash_handler(&weak_client, generation);
                    return;
                }
            },
            Err(ClientError::Timeout) => {
                // Per scope item 16: Timeout is transient and does NOT
                // trigger respawn. A long migration produces a series of
                // 5 s ping timeouts that the heartbeat must ride out.
                log::warn!("service heartbeat missed (timeout); not a respawn trigger");
            }
            Err(error) => {
                // Hard error per scope item 16: the writer task died (the
                // Service is genuinely gone) or the response carried a
                // non-Timeout error. Trigger the crash handler and exit
                // this task; a respawn will spawn a new heartbeat.
                log::warn!("service heartbeat hard error: {error}");
                trigger_crash_handler(&weak_client, generation);
                return;
            }
        }
    }
}

async fn request_value_raw(
    stdin_tx: &mpsc::Sender<Vec<u8>>,
    pending: &Arc<DashMap<u64, oneshot::Sender<Result<serde_json::Value, ClientError>>>>,
    next_id: &Arc<AtomicU64>,
    params: RequestParams,
) -> Result<serde_json::Value, ClientError> {
    let id = next_id.fetch_add(1, Ordering::SeqCst);
    let bytes = encode_message(&JsonRpcRequest::new(id, &params))?;
    let (tx, rx) = oneshot::channel();
    pending.insert(id, tx);
    let mut guard = PendingGuard::new(pending, id);

    // Bound BOTH `stdin_tx.send` and the response `rx` by the per-method
    // timeout so a wedged Service whose writer mpsc has filled cannot park
    // `send` indefinitely. Heartbeat is the load-bearing case: previously
    // the timeout only wrapped `rx`, so a full STDIN_QUEUE_CAP would leave
    // the heartbeat parked on send and silently disable the "Service
    // genuinely dead" detector. Sharing the budget keeps the common path
    // (send completes in microseconds) unchanged.
    let body = async {
        stdin_tx
            .send(bytes)
            .await
            .map_err(|_| ClientError::ServiceCrashed)?;
        match rx.await {
            Ok(response) => response,
            Err(_) => Err(ClientError::ServiceCrashed),
        }
    };
    let response = match params.timeout() {
        RequestTimeoutKind::Finite(timeout) => match tokio::time::timeout(timeout, body).await {
            Ok(response) => response,
            Err(_) => return Err(ClientError::Timeout),
        },
        RequestTimeoutKind::Infinite => body.await,
    };
    guard.disarm();
    response
}

struct PendingGuard<'a> {
    pending: &'a DashMap<u64, oneshot::Sender<Result<serde_json::Value, ClientError>>>,
    id: u64,
    armed: bool,
}

impl<'a> PendingGuard<'a> {
    fn new(
        pending: &'a DashMap<u64, oneshot::Sender<Result<serde_json::Value, ClientError>>>,
        id: u64,
    ) -> Self {
        Self {
            pending,
            id,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.pending.remove(&self.id);
        }
    }
}

fn fail_pending(
    pending: &DashMap<u64, oneshot::Sender<Result<serde_json::Value, ClientError>>>,
) {
    let ids: Vec<u64> = pending.iter().map(|entry| *entry.key()).collect();
    for id in ids {
        if let Some((_, sender)) = pending.remove(&id) {
            let _ = sender.send(Err(ClientError::ServiceCrashed));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dropping the guard without disarming must remove the pending entry.
    /// Covers the request-future-cancelled-mid-flight path.
    #[test]
    fn pending_guard_evicts_entry_when_dropped_armed() {
        let pending: DashMap<u64, oneshot::Sender<Result<serde_json::Value, ClientError>>> =
            DashMap::new();
        let (tx, _rx) = oneshot::channel();
        pending.insert(42, tx);
        {
            let _guard = PendingGuard::new(&pending, 42);
        }
        assert!(!pending.contains_key(&42));
    }

    /// A disarmed guard must leave the pending entry alone (the reader's
    /// dispatch_response already removed and consumed the sender).
    #[test]
    fn pending_guard_leaves_entry_when_disarmed() {
        let pending: DashMap<u64, oneshot::Sender<Result<serde_json::Value, ClientError>>> =
            DashMap::new();
        let (tx, _rx) = oneshot::channel();
        pending.insert(7, tx);
        {
            let mut guard = PendingGuard::new(&pending, 7);
            guard.disarm();
        }
        assert!(pending.contains_key(&7));
    }

    /// Heartbeat task continues after a single Timeout. Drives time via
    /// `tokio::time::pause` so the test runs in milliseconds. After the
    /// first ping times out (no responder reads `pending`), the next
    /// interval tick must fire a second ping - that's the proof of
    /// continuation.
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn heartbeat_continues_past_a_single_timeout() {
        let pending = Arc::new(DashMap::<
            u64,
            oneshot::Sender<Result<serde_json::Value, ClientError>>,
        >::new());
        let next_id = Arc::new(AtomicU64::new(1));
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(8);

        // Weak::new() never upgrades, so the Timeout-doesn't-respawn
        // contract is preserved without needing a real ServiceClient.
        let task = tokio::spawn(heartbeat_task(
            stdin_tx.clone(),
            Arc::clone(&pending),
            Arc::clone(&next_id),
            Weak::<ServiceClient>::new(),
            1,
        ));

        // First interval tick (the initial tick fires immediately on
        // tokio::time::interval, but here `start_paused` means we control
        // when it fires).
        tokio::time::advance(Duration::from_millis(1)).await;
        let _first = stdin_rx
            .recv()
            .await
            .expect("first ping should be enqueued");

        // No responder fills the pending entry. Advance past the
        // HealthPing finite timeout (5 s) so request_value_raw returns
        // ClientError::Timeout. The heartbeat must log and continue rather
        // than exit.
        tokio::time::advance(Duration::from_secs(6)).await;
        // Yield so the heartbeat task can observe the timeout.
        tokio::task::yield_now().await;

        // Advance to the next interval tick (30 s after the prior one).
        tokio::time::advance(Duration::from_secs(30)).await;
        let _second = tokio::time::timeout(Duration::from_secs(1), stdin_rx.recv())
            .await
            .expect("second ping should arrive after the prior timeout")
            .expect("stdin channel closed unexpectedly");

        // We have what we needed. Dropping `stdin_tx` (and the rx) lets
        // the heartbeat exit cleanly on its next send.
        drop(stdin_tx);
        drop(stdin_rx);
        // Don't await `task` directly because it may still be in a sleep;
        // abort to keep the test fast.
        task.abort();
    }

    /// The reader's BootProgress tagging path overwrites whatever the Service
    /// emitted with the reader's own generation. Item 15 leans on this for
    /// its dispatch-side drop.
    #[test]
    fn tag_notification_overwrites_boot_progress_generation() {
        let n = Notification::BootProgress(service_api::BootProgress {
            phase: service_api::BootPhase::LoadingKey,
            message: None,
            // Service emits a placeholder; UI must overwrite.
            service_generation: 0,
        });
        let tagged = tag_notification_with_generation(n, 7);
        match tagged {
            Notification::BootProgress(bp) => {
                assert_eq!(bp.service_generation, 7);
            }
            other => panic!("expected BootProgress; got {other:?}"),
        }
    }

    /// Stale notification (gen != current) drops at dispatch.
    #[test]
    fn notification_should_dispatch_drops_stale_boot_progress() {
        let stale = Notification::BootProgress(service_api::BootProgress {
            phase: service_api::BootPhase::Migrating {
                current: 1,
                total: 10,
            },
            message: None,
            service_generation: 1,
        });
        assert!(!notification_should_dispatch(&stale, 2));
    }

    /// Current notification (gen == current) dispatches normally.
    #[test]
    fn notification_should_dispatch_passes_current_boot_progress() {
        let current = Notification::BootProgress(service_api::BootProgress {
            phase: service_api::BootPhase::LoadingKey,
            message: None,
            service_generation: 5,
        });
        assert!(notification_should_dispatch(&current, 5));
    }

    /// Wire-up between `tag_notification_with_generation` (reader-side)
    /// and `notification_should_dispatch` (consumer-side): a notification
    /// tagged with generation N must dispatch under live=N and must NOT
    /// dispatch under live=M (M != N). Locks the contract that the two
    /// functions agree on which field carries the cross-respawn
    /// discriminator. A future Phase 2 PR that adds a new tagged
    /// notification variant should extend this test with the new payload
    /// to verify both functions reach the same field.
    #[test]
    fn tag_and_dispatch_agree_on_boot_progress_generation() {
        let untagged = Notification::BootProgress(service_api::BootProgress {
            phase: service_api::BootPhase::OpeningDatabase,
            message: None,
            // Untagged: the Service emits 0 here; the reader is expected
            // to overwrite with its captured generation.
            service_generation: 0,
        });
        let tagged_for_5 = tag_notification_with_generation(untagged.clone(), 5);
        assert_eq!(tagged_for_5.service_generation(), Some(5));
        assert!(
            notification_should_dispatch(&tagged_for_5, 5),
            "tag(N) -> dispatch(live=N) must pass"
        );
        assert!(
            !notification_should_dispatch(&tagged_for_5, 6),
            "tag(N) -> dispatch(live=N+1) must drop"
        );

        // Re-tagging with a different value must replace, not accumulate.
        let retagged = tag_notification_with_generation(tagged_for_5, 7);
        assert_eq!(retagged.service_generation(), Some(7));
        assert!(notification_should_dispatch(&retagged, 7));
        assert!(!notification_should_dispatch(&retagged, 5));
    }

    // The "variants without a generation field always dispatch" path is
    // covered by `service_api::tests::service_generation_is_none_for_variants_without_the_field`
    // - the test variant lives behind `#[cfg(test)]` in the service-api
    // crate and is not visible from the app crate's tests.

    /// Production-variant catalog for the cross-respawn dispatch contract.
    /// Returns one of each non-test `Notification` variant the UI can
    /// receive on the wire, so the round-trip test below proves
    /// tag→dispatch flips for every variant.
    ///
    /// **Phase 2+ contract**: when adding a new state-changing variant to
    /// `service_api::Notification` (e.g. `action.completed`, `push.event`,
    /// `OperationOutcome`), add an entry here. If you forget, the
    /// round-trip test won't grow with the new variant - silently
    /// permitting a regression where `set_service_generation`
    /// or `service_generation` skips the new arm. The catalog is the
    /// testable counterpart to the `WithGeneration` trait + the
    /// adjacent get/set methods on `Notification`.
    fn production_notification_catalog() -> Vec<Notification> {
        use service_api::{BootPhase, BootProgress, PushEvent};
        let phases = [
            BootPhase::LoadingKey,
            BootPhase::OpeningDatabase,
            BootPhase::Migrating {
                current: 1,
                total: 1,
            },
            BootPhase::RecoveringPendingOps,
            BootPhase::SweepingQueuedDrafts,
            BootPhase::BackfillingThreadParticipants,
        ];
        let mut catalog: Vec<Notification> = phases
            .into_iter()
            .map(|phase| {
                Notification::BootProgress(BootProgress {
                    phase,
                    message: None,
                    // Untagged: must be overwritten by the reader.
                    service_generation: 0,
                })
            })
            .collect();
        // Phase 4 review-pass: PushEvent was added in task 1 but
        // omitted from this catalog. Without an entry the cross-
        // respawn round-trip test below silently skips the new
        // variant.
        catalog.push(Notification::PushEvent(PushEvent {
            account_id: "acc-1".into(),
            service_generation: 0,
        }));
        // Phase 5: CalendarRunCompleted (MustDeliver) and CalendarChanged
        // (Coalesce per-account). Both carry service_generation and must
        // round-trip the cross-respawn dispatch filter.
        catalog.push(Notification::CalendarRunCompleted(
            service_api::CalendarRunCompleted {
                account_id: "acc-1".into(),
                run_id: service_api::CalendarRunId::new_v7(),
                result: service_api::CalendarSyncResult::Completed,
                mutated: true,
                service_generation: 0,
            },
        ));
        catalog.push(Notification::CalendarChanged(
            service_api::CalendarChanged {
                account_id: "acc-1".into(),
                service_generation: 0,
            },
        ));
        catalog
    }

    /// Catalog-driven regression test: every production notification
    /// variant must round-trip cleanly through tag → dispatch.
    /// Specifically:
    ///   - `tag_notification_with_generation(n, G)` must result in
    ///     `n.service_generation() == Some(G)` (not None - that would
    ///     silently disable the cross-respawn drop), and
    ///   - dispatch must pass under `live=G` and drop under `live=G+1`.
    ///
    /// If a future PR adds a `Notification` variant without (a) updating
    /// `set_service_generation` to call `WithGeneration::set_generation`
    /// on the payload, or (b) adding the variant to
    /// `production_notification_catalog`, this test catches the gap.
    #[test]
    fn every_production_notification_round_trips_through_tagging() {
        for notification in production_notification_catalog() {
            let method = notification.method_name();
            let tagged = tag_notification_with_generation(notification, 7);
            assert_eq!(
                tagged.service_generation(),
                Some(7),
                "set_service_generation must overwrite the tag for production \
                 variant '{method}'. Returning None disables the cross-respawn \
                 drop and reintroduces the race scope item 20 closed.",
            );
            assert!(
                notification_should_dispatch(&tagged, 7),
                "matching generation must dispatch for variant '{method}'",
            );
            assert!(
                !notification_should_dispatch(&tagged, 8),
                "mismatched generation must drop for variant '{method}'",
            );
        }
    }

    /// AnotherInstanceRunning is the one terminal-failure case with a
    /// genuinely user-actionable message: the user can quit the other
    /// running instance and try again. Pin the wording so a refactor
    /// can't silently degrade the message.
    #[test]
    fn terminal_message_for_another_instance_is_user_friendly() {
        let reason = BootFailureReason::Classified(BootClassification::BootFailure {
            code: BootExitCode::AnotherInstanceRunning,
        });
        assert_eq!(
            terminal_failure_user_message(&reason),
            "Ratatoskr is already running.",
        );
    }

    /// Each known BootExitCode maps to a distinct user message - distinct
    /// is what matters; a refactor that collapses two of them into the same
    /// string should fail this test.
    #[test]
    fn terminal_messages_for_known_codes_are_distinct() {
        let messages: Vec<String> = [
            BootExitCode::AnotherInstanceRunning,
            BootExitCode::KeyLoadFailure,
            BootExitCode::MigrationFailure,
            BootExitCode::HandshakeFailure,
            BootExitCode::LockIoFailure,
        ]
        .into_iter()
        .map(|code| {
            terminal_failure_user_message(&BootFailureReason::Classified(
                BootClassification::BootFailure { code },
            ))
        })
        .collect();
        let mut sorted = messages.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), messages.len(), "messages must be distinct");
    }

    /// UnexpectedExit prints whichever exit code we observed, distinguishing
    /// signal-killed (None) from "exited with an unknown numeric code"
    /// (Some(n)) so the diagnostic in the log files isn't ambiguous.
    #[test]
    fn terminal_message_for_unexpected_exit_includes_code() {
        let with_code = BootFailureReason::Classified(BootClassification::UnexpectedExit {
            code: Some(42),
        });
        assert!(
            terminal_failure_user_message(&with_code).contains("42"),
            "unexpected-exit message should contain the code",
        );
        let signal = BootFailureReason::Classified(BootClassification::UnexpectedExit {
            code: None,
        });
        assert!(
            terminal_failure_user_message(&signal).contains("signaled"),
            "no-code path should mention 'signaled' so log readers know it was a signal",
        );
    }

    /// `BootFailureReason::Other` is the catch-all for non-classified
    /// failures (UI-side spawn IO error, version mismatch, request
    /// timeout). The detail string is included verbatim.
    #[test]
    fn terminal_message_for_other_includes_detail() {
        let reason = BootFailureReason::Other("specific upstream failure".to_string());
        assert!(
            terminal_failure_user_message(&reason).contains("specific upstream failure"),
        );
    }

    /// `BootFailureReason::from_client_error` keeps the structured
    /// classification when the upstream error is `ClientError::BootFailure`,
    /// and falls back to `Other(detail)` for everything else. This is what
    /// the spawn_event_stream conversion relies on.
    #[test]
    fn from_client_error_preserves_classification() {
        let classified = ClientError::BootFailure {
            classification: BootClassification::BootFailure {
                code: BootExitCode::KeyLoadFailure,
            },
        };
        match BootFailureReason::from_client_error(&classified) {
            BootFailureReason::Classified(BootClassification::BootFailure {
                code: BootExitCode::KeyLoadFailure,
            }) => {}
            other => panic!("expected Classified KeyLoadFailure, got {other:?}"),
        }

        let crashed = ClientError::ServiceCrashed;
        match BootFailureReason::from_client_error(&crashed) {
            BootFailureReason::Other(_) => {}
            other => panic!("expected Other, got {other:?}"),
        }
    }

    /// Both schema-related Terminal variants flow through `Other(detail)`
    /// (they're not BootClassification cases since they originate from
    /// the respawn handshake, not a child-process exit code). The user-
    /// visible message must name what kind of failure it was so the next
    /// boot's diagnostics can route on it. Locks the Display contract -
    /// a refactor that swapped one variant's #[error] for the other would
    /// not be caught by the surrounding code (both produce String) but
    /// would break log triage.
    #[test]
    fn schema_terminal_variants_have_distinct_user_messages() {
        let mismatch = ClientError::SchemaVersionChanged { was: 100, now: 101 };
        let mismatch_msg =
            terminal_failure_user_message(&BootFailureReason::from_client_error(&mismatch));
        assert!(
            mismatch_msg.contains("schema_version") && mismatch_msg.contains("100"),
            "mismatch message must name the schema_version field and the prior value, got: {mismatch_msg}"
        );

        let missing = ClientError::SchemaBaselineMissing;
        let missing_msg =
            terminal_failure_user_message(&BootFailureReason::from_client_error(&missing));
        assert!(
            missing_msg.contains("baseline"),
            "baseline-missing message must name the missing baseline, got: {missing_msg}"
        );

        // The two messages must not collide - log triage has to be able to
        // distinguish "we detected a swap" from "we lost the ability to
        // detect a swap".
        assert_ne!(mismatch_msg, missing_msg);
    }

    /// Crashloop tracker: the first two respawns are not crashloops; the
    /// third within the window is. Drives the clock manually so the test
    /// doesn't depend on wall time.
    #[test]
    fn crashloop_tracker_fires_on_third_respawn_within_window() {
        let queue: Mutex<VecDeque<Instant>> = Mutex::new(VecDeque::new());
        let t0 = Instant::now();
        assert!(!record_respawn_and_check_crashloop(&queue, t0));
        assert!(!record_respawn_and_check_crashloop(
            &queue,
            t0 + Duration::from_secs(5),
        ));
        assert!(record_respawn_and_check_crashloop(
            &queue,
            t0 + Duration::from_secs(10),
        ));
    }

    /// Crashloop tracker evicts entries older than the window, so a slow
    /// drip of respawns never trips the bound. The first respawn at t0
    /// drops out before the third at t0+CRASHLOOP_WINDOW+1s, leaving the
    /// queue with two recent entries - below threshold.
    #[test]
    fn crashloop_tracker_evicts_old_entries() {
        let queue: Mutex<VecDeque<Instant>> = Mutex::new(VecDeque::new());
        let t0 = Instant::now();
        assert!(!record_respawn_and_check_crashloop(&queue, t0));
        assert!(!record_respawn_and_check_crashloop(
            &queue,
            t0 + Duration::from_secs(15),
        ));
        // CRASHLOOP_WINDOW is 30 s; advance to 31 s past t0 so the first
        // entry has expired.
        assert!(!record_respawn_and_check_crashloop(
            &queue,
            t0 + Duration::from_secs(31),
        ));
    }

    /// Pre-queue gate accepts the reader's notification when generations
    /// match.
    #[test]
    fn reader_should_enqueue_passes_when_generations_match() {
        assert!(reader_should_enqueue(3, Some(3)));
    }

    /// Pre-queue gate drops the reader's notification when generations
    /// differ - a respawn has happened and this reader is from the dying
    /// incarnation.
    #[test]
    fn reader_should_enqueue_drops_when_generations_differ() {
        assert!(!reader_should_enqueue(3, Some(4)));
        assert!(!reader_should_enqueue(3, Some(2)));
    }

    /// Pre-queue gate drops when the live generation is None (the client
    /// has been dropped). The reader is about to exit anyway.
    #[test]
    fn reader_should_enqueue_drops_when_no_live_client() {
        assert!(!reader_should_enqueue(3, None));
    }

    /// End-to-end: a stale `BootProgress` filtered at the reader's pre-queue
    /// gate cannot overwrite a fresh `BootProgress` in the queue's coalesce
    /// slot. Without the reader-side gate, a stale gen=1 `Migrating(1, 10)`
    /// arriving after a fresh gen=2 `Migrating(5, 10)` would replace it
    /// (per-phase coalesce key, generation not part of the key); the
    /// dispatch-side check would then drop the merged (stale-tagged)
    /// notification, losing the fresh update with it. Pin the property:
    /// after the gate, only fresh notifications reach the queue.
    #[tokio::test]
    async fn stale_reader_notifications_never_enter_the_queue() {
        let queue: crate::notification_queue::NotificationQueue<Notification> =
            crate::notification_queue::NotificationQueue::new(8);

        let live_gen = 2u32;

        // Fresh reader (gen=2) enqueues. Gate passes.
        let fresh = tag_notification_with_generation(
            Notification::BootProgress(service_api::BootProgress {
                phase: service_api::BootPhase::Migrating {
                    current: 5,
                    total: 10,
                },
                message: None,
                service_generation: 0,
            }),
            2,
        );
        if reader_should_enqueue(2, Some(live_gen)) {
            queue.enqueue(fresh).await;
        }

        // Stale reader (gen=1) tries to enqueue after a respawn. Gate drops.
        let stale = tag_notification_with_generation(
            Notification::BootProgress(service_api::BootProgress {
                phase: service_api::BootPhase::Migrating {
                    current: 1,
                    total: 10,
                },
                message: None,
                service_generation: 0,
            }),
            1,
        );
        if reader_should_enqueue(1, Some(live_gen)) {
            queue.enqueue(stale).await;
        }

        // Only the fresh notification is in the queue.
        let received = queue
            .recv()
            .await
            .expect("queue should have the fresh notification");
        match received {
            Notification::BootProgress(bp) => {
                assert_eq!(bp.service_generation, 2);
                assert!(matches!(
                    bp.phase,
                    service_api::BootPhase::Migrating { current: 5, total: 10 }
                ));
            }
            other => panic!("expected BootProgress; got {other:?}"),
        }
        queue.close();
        assert!(queue.recv().await.is_none(), "queue must be empty");
    }

    /// End-to-end: tag a notification with the reader's gen, then run the
    /// dispatch-side check after a generation bump. The notification must
    /// be dropped. This is the unit-level proof that the reader+dispatch
    /// pipeline closes the cross-respawn race per scope item 20.
    #[test]
    fn reader_tag_plus_dispatch_drop_filters_stale_notification() {
        let untagged = Notification::BootProgress(service_api::BootProgress {
            phase: service_api::BootPhase::OpeningDatabase,
            message: None,
            service_generation: 0,
        });
        // Reader was spawned at gen=1 and tags accordingly.
        let tagged = tag_notification_with_generation(untagged, 1);
        // A respawn happens; current_generation bumps to 2. The notification
        // (still tagged with the old reader's gen=1) must not pass dispatch.
        assert!(!notification_should_dispatch(&tagged, 2));
        // After-the-fact sanity: a notification tagged with the current gen
        // (i.e., from the new reader) does pass.
        let fresh = tag_notification_with_generation(
            Notification::BootProgress(service_api::BootProgress {
                phase: service_api::BootPhase::OpeningDatabase,
                message: None,
                service_generation: 0,
            }),
            2,
        );
        assert!(notification_should_dispatch(&fresh, 2));
    }

    // Phase 3 task 14: pending_syncs latching + GC + drain.

    fn empty_pending_syncs() -> PendingSyncs {
        Arc::new(Mutex::new(HashMap::new()))
    }

    /// `route_sync_completed` arriving before any subscriber latches
    /// the result; the late subscriber consumes it without parking.
    #[tokio::test]
    async fn pending_syncs_latches_completed_when_no_subscriber() {
        let map = empty_pending_syncs();
        let run_id = SyncRunId::new_v7();

        // Simulate the reader-task arrival path inline.
        {
            let mut g = map.lock().expect("test mutex");
            g.insert(
                run_id,
                PendingSync::Completed {
                    result: SyncResult::Completed,
                    latched_at: Instant::now(),
                },
            );
        }
        // A late "subscribe" finds the latched entry and consumes.
        let mut g = map.lock().expect("test mutex");
        match g.entry(run_id) {
            Entry::Occupied(e) => match e.get() {
                PendingSync::Completed { result, .. } => {
                    let r = result.clone();
                    e.remove();
                    assert!(matches!(r, SyncResult::Completed));
                }
                PendingSync::Pending(_) => panic!("expected latched Completed"),
            },
            Entry::Vacant(_) => panic!("expected latched entry"),
        }
        assert!(g.is_empty(), "consumed entry must be removed");
    }

    /// Subscriber inserts `Pending`; routing arrival broadcasts the
    /// result and removes the entry.
    #[tokio::test]
    async fn pending_syncs_pending_then_route_resolves_subscriber() {
        let map = empty_pending_syncs();
        let run_id = SyncRunId::new_v7();
        let mut rx = {
            let mut g = map.lock().expect("test mutex");
            let (tx, rx) = broadcast::channel(SYNC_BROADCAST_CAPACITY);
            g.insert(run_id, PendingSync::Pending(tx));
            rx
        };
        // Routing path:
        {
            let mut g = map.lock().expect("test mutex");
            if let Entry::Occupied(mut e) = g.entry(run_id) {
                if let PendingSync::Pending(tx) = e.get_mut() {
                    let _ = tx.send(SyncResult::Cancelled);
                }
                e.remove();
            }
        }
        let received = rx.recv().await.expect("subscriber receives");
        assert!(matches!(received, SyncResult::Cancelled));
        assert!(map.lock().expect("test mutex").is_empty());
    }

    /// `fail_pending_syncs` drops every Pending sender so awaiting
    /// subscribers see `RecvError::Closed`. Mirrors the cross-respawn
    /// drain.
    #[tokio::test]
    async fn fail_pending_syncs_closes_pending_subscribers() {
        let map = empty_pending_syncs();
        let run_id = SyncRunId::new_v7();
        let mut rx = {
            let mut g = map.lock().expect("test mutex");
            let (tx, rx) = broadcast::channel(SYNC_BROADCAST_CAPACITY);
            g.insert(run_id, PendingSync::Pending(tx));
            rx
        };
        fail_pending_syncs(&map);
        let recv = rx.recv().await;
        assert!(recv.is_err(), "subscriber should see Closed after drain");
        assert!(map.lock().expect("test mutex").is_empty());
    }

    /// `sweep_latched_completed` drops Completed entries past the TTL
    /// but leaves Pending entries alone.
    #[test]
    fn sweep_latched_drops_aged_completed_keeps_pending() {
        let map = empty_pending_syncs();
        let stale_id = SyncRunId::new_v7();
        let fresh_id = SyncRunId::new_v7();
        let pending_id = SyncRunId::new_v7();
        {
            let mut g = map.lock().expect("test mutex");
            g.insert(
                stale_id,
                PendingSync::Completed {
                    result: SyncResult::Completed,
                    latched_at: Instant::now()
                        .checked_sub(LATCHED_COMPLETED_TTL + Duration::from_secs(1))
                        .expect("monotonic"),
                },
            );
            g.insert(
                fresh_id,
                PendingSync::Completed {
                    result: SyncResult::Completed,
                    latched_at: Instant::now(),
                },
            );
            let (tx, _rx) = broadcast::channel(SYNC_BROADCAST_CAPACITY);
            g.insert(pending_id, PendingSync::Pending(tx));
        }
        sweep_latched_completed(&map);
        let g = map.lock().expect("test mutex");
        assert!(!g.contains_key(&stale_id), "stale Completed must be dropped");
        assert!(g.contains_key(&fresh_id), "fresh Completed must survive");
        assert!(g.contains_key(&pending_id), "Pending must survive");
    }
}
