use crate::notification_queue::NotificationQueue;
use dashmap::DashMap;
use serde::de::DeserializeOwned;
use service_api::{
    BootClassification, BootExitCode, BootReadyResponse, BoundedLineReader, HealthPingResponse,
    JsonRpcErrorObject, JsonRpcRequest, Notification, ParsedServiceMessage, PROTOCOL_VERSION,
    RequestParams, RequestTimeoutKind, ServiceError, ServiceResponse, ShutdownResponse,
    encode_message, parse_service_message,
};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex, PoisonError, Weak,
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};

const STDIN_QUEUE_CAP: usize = 1024;
const NOTIFICATION_QUEUE_CAP: usize = 1024;

pub type ServiceNotificationReceiver = Arc<NotificationQueue>;

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
    /// Cross-platform parent-death tie-up. Held for the lifetime of the
    /// client so the OS-level safety net (Job Object on Windows) survives
    /// any failure in our explicit Drop teardown. Listed last so it drops
    /// after every other field, making the kill-on-job-close fire only as
    /// a true last-resort. Reused across respawns: each new child is
    /// assigned to the same `ProcessGuard` so the safety net stays in place
    /// for the replacement.
    _process_guard: service::parent_death::ProcessGuard,
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
    #[error("response deserialize: {0}")]
    Deserialize(#[from] serde_json::Error),
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
        let process_guard = service::parent_death::ProcessGuard::new()?;
        let pending: Arc<
            DashMap<u64, oneshot::Sender<Result<serde_json::Value, ClientError>>>,
        > = Arc::new(DashMap::new());
        let next_id = Arc::new(AtomicU64::new(1));
        let notifications: Arc<NotificationQueue> =
            Arc::new(NotificationQueue::new(NOTIFICATION_QUEUE_CAP));

        let client = Arc::new(Self {
            state: Mutex::new(None),
            pending: Arc::clone(&pending),
            next_id: Arc::clone(&next_id),
            notifications: Arc::clone(&notifications),
            current_generation: AtomicU32::new(0),
            is_shutting_down: AtomicBool::new(false),
            respawn_config,
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

        let ping: HealthPingResponse = match client.request(RequestParams::HealthPing).await {
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
        // wait() to reap it and pull the exit code. A 2 s budget is plenty
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
        let abort_budget = Duration::from_millis(200);
        let _ = tokio::time::timeout(abort_budget, reader_handle).await;
        let _ = tokio::time::timeout(abort_budget, writer_handle).await;
        let _ = tokio::time::timeout(abort_budget, heartbeat_handle).await;

        let Some(status) = wait_with_kill_watchdog(&mut child, Duration::from_secs(2)).await
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

    /// Live Service-incarnation counter. The reader task tags every
    /// notification with the generation it captured at spawn time;
    /// `notification_should_dispatch` compares the tag against this value
    /// to drop notifications from a dying-but-still-flushing reader after
    /// a respawn (scope item 20 of `phase-1.5-plan.md`).
    pub(crate) fn current_generation(&self) -> u32 {
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
        // run_spawn_flow. handle_crash already deferred the respawn if
        // first_boot_ready was None, so the None arm here is unreachable in
        // normal flow; we treat it as defense-in-depth, log a warning, and
        // capture the response so subsequent respawns at least have a
        // baseline to compare against.
        let mismatch: Option<(u32, u32)> = {
            let mut guard = respawn
                .first_boot_ready
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            match guard.as_ref() {
                Some(first) if first.schema_version != response.schema_version => {
                    Some((first.schema_version, response.schema_version))
                }
                Some(_) => None,
                None => {
                    log::warn!(
                        "respawn observed first_boot_ready=None; capturing now (defensive)",
                    );
                    *guard = Some(response.clone());
                    None
                }
            }
        };
        if let Some((was, now)) = mismatch {
            log::error!(
                "respawn schema_version changed (was {was}, now {now}); binary swap detected",
            );
            return Err(ClientError::SchemaVersionChanged { was, now });
        }

        let _ = respawn
            .spawn_event_tx
            .send(SpawnEvent::BootReady(response))
            .await;
        Ok(())
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
    process_guard: &service::parent_death::ProcessGuard,
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
    service::parent_death::configure_command(&mut command)?;

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
        client.request(RequestParams::BootReady).await;
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

async fn reader_task<R>(
    stdout: R,
    pending: Arc<DashMap<u64, oneshot::Sender<Result<serde_json::Value, ClientError>>>>,
    notifications: Arc<NotificationQueue>,
    weak_client: Weak<ServiceClient>,
    generation: u32,
) where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut lines = BoundedLineReader::new(stdout, service_api::MAX_FRAME_BYTES);
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => match parse_service_message(&line) {
                Ok(ParsedServiceMessage::Response {
                    id: Some(id),
                    response,
                }) => {
                    dispatch_response(&pending, id, response);
                }
                Ok(ParsedServiceMessage::Response { id: None, response }) => {
                    // An uncorrelated response means the Service answered
                    // a request whose id was null (parse error before the
                    // dispatch could correlate). Log only the discriminant
                    // and, for errors, the JSON-RPC code/message - never
                    // the success payload, which can carry user content
                    // (message bodies, search queries, etc.) once Phase 2+
                    // methods land.
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
                Ok(ParsedServiceMessage::Notification(notification)) => {
                    let tagged = tag_notification_with_generation(notification, generation);
                    notifications.enqueue(tagged).await;
                }
                Err(error) => {
                    log::warn!("failed to parse service message: {error}");
                }
            },
            Ok(None) => {
                fail_pending(&pending);
                trigger_crash_handler(&weak_client, generation);
                return;
            }
            Err(error) => {
                log::warn!("service stdout frame error: {error}");
                fail_pending(&pending);
                trigger_crash_handler(&weak_client, generation);
                return;
            }
        }
    }
}

/// Tag `BootProgress` with the reader's fixed generation so the dispatch
/// side (item 15) can drop notifications from a dying-but-still-flushing
/// reader. Phase 2+ adds new notification variants; each one needs a similar
/// generation field if the dispatch must distinguish across respawns.
fn tag_notification_with_generation(
    notification: Notification,
    generation: u32,
) -> Notification {
    match notification {
        Notification::BootProgress(mut progress) => {
            progress.service_generation = generation;
            Notification::BootProgress(progress)
        }
    }
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

    // The "variants without a generation field always dispatch" path is
    // covered by `service_api::tests::service_generation_is_none_for_variants_without_the_field`
    // - the test variant lives behind `#[cfg(test)]` in the service-api
    // crate and is not visible from the app crate's tests.

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
}
