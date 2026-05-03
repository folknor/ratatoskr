use crate::notification_queue::NotificationQueue;
use dashmap::DashMap;
use serde::de::DeserializeOwned;
use service_api::{
    BootClassification, BootReadyResponse, BoundedLineReader, HealthPingResponse,
    JsonRpcErrorObject, JsonRpcRequest, Notification, ParsedServiceMessage, PROTOCOL_VERSION,
    RequestParams, RequestTimeoutKind, ServiceError, ServiceResponse, ShutdownResponse,
    encode_message, parse_service_message,
};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};

const STDIN_QUEUE_CAP: usize = 1024;
const NOTIFICATION_QUEUE_CAP: usize = 1024;

pub type ServiceNotificationReceiver = Arc<NotificationQueue>;

pub struct ServiceClient {
    child: Mutex<Option<Child>>,
    stdin_tx: Option<mpsc::Sender<Vec<u8>>>,
    pending: Arc<DashMap<u64, oneshot::Sender<Result<serde_json::Value, ClientError>>>>,
    next_id: Arc<AtomicU64>,
    notifications: ServiceNotificationReceiver,
    reader_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    writer_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    heartbeat_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Cross-platform parent-death tie-up. Held for the lifetime of the
    /// client so the OS-level safety net (Job Object on Windows) survives
    /// any failure in our explicit Drop teardown. Listed last so it drops
    /// after every other field, making the kill-on-job-close fire only as
    /// a true last-resort.
    _process_guard: service::parent_death::ProcessGuard,
}

impl std::fmt::Debug for ServiceClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceClient").finish_non_exhaustive()
    }
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
    #[error("response deserialize: {0}")]
    Deserialize(#[from] serde_json::Error),
}

impl ServiceClient {
    pub async fn spawn(app_data_dir: &Path) -> Result<Arc<Self>, ClientError> {
        let exe = std::env::current_exe()?;
        Self::spawn_inner(&exe, app_data_dir, &[]).await
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
            run_spawn_flow(&exe, app_data_dir, Vec::new(), tx).await;
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
            run_spawn_flow(&binary, app_data_dir, extra_args, tx).await;
        });
        rx
    }

    /// Test-only spawn that lets tests override the binary path and pass
    /// extra args to the Service. Used for spawn-failure (bad binary path)
    /// and version-mismatch (`--test-fake-version=N`) coverage. Compiled
    /// out of release builds via the `test-helpers` feature.
    #[cfg(feature = "test-helpers")]
    pub async fn spawn_for_test(
        binary: &Path,
        app_data_dir: &Path,
        extra_args: &[&str],
    ) -> Result<Arc<Self>, ClientError> {
        Self::spawn_inner(binary, app_data_dir, extra_args).await
    }

    async fn spawn_inner(
        binary: &Path,
        app_data_dir: &Path,
        extra_args: &[&str],
    ) -> Result<Arc<Self>, ClientError> {
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
        let process_guard = service::parent_death::ProcessGuard::new()?;

        let mut child = command.spawn()?;
        // Assign immediately; on Windows the kill-on-job-close protection
        // only fires for processes already in the Job. The window between
        // spawn and assign is small but non-zero; explicit shutdown via the
        // request/timeout/SIGKILL path remains the primary teardown.
        process_guard.assign(&child)?;
        let stdin = child.stdin.take().ok_or(ClientError::NotConnected)?;
        let stdout = child.stdout.take().ok_or(ClientError::NotConnected)?;
        let (stdin_tx, stdin_rx) = mpsc::channel(STDIN_QUEUE_CAP);
        let pending = Arc::new(DashMap::new());
        let next_id = Arc::new(AtomicU64::new(1));
        let notifications: Arc<NotificationQueue> =
            Arc::new(NotificationQueue::new(NOTIFICATION_QUEUE_CAP));

        let reader_handle = tokio::spawn(reader_task(
            stdout,
            Arc::clone(&pending),
            Arc::clone(&notifications),
        ));
        let writer_handle = tokio::spawn(writer_task(stdin, stdin_rx));

        let client = Arc::new(Self {
            child: Mutex::new(Some(child)),
            stdin_tx: Some(stdin_tx),
            pending,
            next_id,
            notifications,
            reader_handle: Mutex::new(Some(reader_handle)),
            writer_handle: Mutex::new(Some(writer_handle)),
            heartbeat_handle: Mutex::new(None),
            _process_guard: process_guard,
        });

        let ping: HealthPingResponse = client.request(RequestParams::HealthPing).await?;
        if ping.version != PROTOCOL_VERSION {
            return Err(ClientError::VersionMismatch {
                ui: PROTOCOL_VERSION,
                service: ping.version,
            });
        }
        log::info!("Service ready (pid={})", ping.pid);

        let heartbeat = tokio::spawn(heartbeat_task(
            client.stdin_tx.as_ref().ok_or(ClientError::NotConnected)?.clone(),
            Arc::clone(&client.pending),
            Arc::clone(&client.next_id),
        ));
        if let Ok(mut guard) = client.heartbeat_handle.lock() {
            *guard = Some(heartbeat);
        }

        Ok(client)
    }

    pub async fn request<R>(&self, params: RequestParams) -> Result<R, ClientError>
    where
        R: DeserializeOwned,
    {
        let value = self.request_value(params).await?;
        serde_json::from_value(value).map_err(ClientError::from)
    }

    pub async fn shutdown(&self) -> Result<(), ClientError> {
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

    /// Returns the child Service's PID while the child is still alive.
    /// Used by integration tests to verify that the OS-level process exits
    /// after explicit shutdown or after the client is dropped.
    pub fn child_pid(&self) -> Option<u32> {
        let guard = self.child.lock().ok()?;
        let child = guard.as_ref()?;
        child.id()
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

        let Some(stdin_tx) = self.stdin_tx.as_ref() else {
            return Err(ClientError::NotConnected);
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
        let Ok(mut guard) = self.child.lock() else {
            return true;
        };
        let Some(child) = guard.as_mut() else {
            return true;
        };
        match child.try_wait() {
            Ok(Some(status)) => {
                log::info!("service exited with status {status}");
                *guard = None;
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
            let Ok(guard) = self.child.lock() else {
                return;
            };
            let Some(child) = guard.as_ref() else {
                return;
            };
            let Some(pid) = child.id() else {
                return;
            };
            let pid = match i32::try_from(pid) {
                Ok(pid) => pid,
                Err(error) => {
                    log::warn!("service pid conversion failed: {error}");
                    return;
                }
            };
            let result = unsafe { libc::kill(pid, libc::SIGTERM) };
            if result != 0 {
                log::warn!("failed to send SIGTERM to service: {}", std::io::Error::last_os_error());
            }
        }
    }

    fn kill_child(&self) {
        let Ok(mut guard) = self.child.lock() else {
            return;
        };
        let Some(child) = guard.as_mut() else {
            return;
        };
        if let Err(error) = child.start_kill() {
            log::warn!("failed to kill service child: {error}");
        }
    }
}

impl Drop for ServiceClient {
    fn drop(&mut self) {
        let reader = take_join_handle(&self.reader_handle);
        let heartbeat = take_join_handle(&self.heartbeat_handle);
        let writer = take_join_handle(&self.writer_handle);
        for handle in [reader.as_ref(), heartbeat.as_ref(), writer.as_ref()]
            .into_iter()
            .flatten()
        {
            handle.abort();
        }
        let _ = self.stdin_tx.take();

        // The dropped writer task carries the ChildStdin handle; awaiting its
        // abort lets the pipe close so the Service sees EOF and exits cleanly.
        // Without driving the runtime here, the abort never makes progress
        // before this Drop returns and we'd fall through to SIGKILL every
        // time on a busy worker. block_in_place + Handle::block_on works on
        // a multi-threaded runtime; single-threaded falls back to polling.
        //
        // Single-threaded fallback caveat: on a `current_thread` runtime,
        // `runtime_for_block_on` returns None, the writer task can't run
        // because we're occupying the only worker, ChildStdin doesn't
        // close, the Service never sees EOF, and we go straight to SIGKILL
        // after the 1.2 s polling budget. Production uses a multi-thread
        // runtime (iced's daemon, plus the Service's own runtime), so this
        // only bites tests that pin a current_thread flavor. Documented
        // rather than fixed: a current_thread-aware Drop would need the
        // task to drain via a different mechanism (e.g. a dedicated
        // shutdown thread), which is more machinery than the failure mode
        // warrants.
        match runtime_for_block_on() {
            Some(handle) => {
                tokio::task::block_in_place(|| {
                    handle.block_on(self.async_drop_wait(reader, heartbeat, writer));
                });
            }
            None => {
                drop((reader, heartbeat, writer));
                self.poll_for_exit(Duration::from_millis(1200));
            }
        }

        if !self.try_wait_child() {
            self.kill_child();
            self.poll_for_exit(Duration::from_millis(500));
        }

        // Unblock any subscription consumer still parked in `recv()` so it
        // can unwind cleanly instead of waiting on a queue that will never
        // see another producer.
        self.notifications.close();
        fail_pending(&self.pending);
    }
}

impl ServiceClient {
    async fn async_drop_wait(
        &self,
        reader: Option<tokio::task::JoinHandle<()>>,
        heartbeat: Option<tokio::task::JoinHandle<()>>,
        writer: Option<tokio::task::JoinHandle<()>>,
    ) {
        let abort_deadline = Duration::from_millis(200);
        let abort_started = Instant::now();
        for handle in [reader, heartbeat, writer].into_iter().flatten() {
            let remaining = abort_deadline.saturating_sub(abort_started.elapsed());
            if remaining.is_zero() {
                continue;
            }
            let _ = tokio::time::timeout(remaining, handle).await;
        }
        let exit_deadline = Duration::from_secs(1);
        let exit_started = Instant::now();
        while exit_started.elapsed() < exit_deadline {
            if self.try_wait_child() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn poll_for_exit(&self, deadline: Duration) {
        let started = Instant::now();
        while started.elapsed() < deadline {
            if self.try_wait_child() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn runtime_for_block_on() -> Option<tokio::runtime::Handle> {
    let handle = tokio::runtime::Handle::try_current().ok()?;
    if matches!(
        handle.runtime_flavor(),
        tokio::runtime::RuntimeFlavor::MultiThread
    ) {
        Some(handle)
    } else {
        None
    }
}

fn take_join_handle(
    slot: &Mutex<Option<tokio::task::JoinHandle<()>>>,
) -> Option<tokio::task::JoinHandle<()>> {
    slot.lock().ok().and_then(|mut guard| guard.take())
}

async fn reader_task<R>(
    stdout: R,
    pending: Arc<DashMap<u64, oneshot::Sender<Result<serde_json::Value, ClientError>>>>,
    notifications: Arc<NotificationQueue>,
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
                            log::warn!("service returned uncorrelated success (payload redacted)");
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
                    enqueue_notification(&notifications, notification).await;
                }
                Err(error) => {
                    log::warn!("failed to parse service message: {error}");
                }
            },
            Ok(None) => {
                fail_pending(&pending);
                return;
            }
            Err(error) => {
                log::warn!("service stdout frame error: {error}");
                fail_pending(&pending);
                return;
            }
        }
    }
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

async fn enqueue_notification(notifications: &NotificationQueue, notification: Notification) {
    notifications.enqueue(notification).await;
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
                Err(error) => log::warn!("service heartbeat decode failed: {error}"),
            },
            Err(ClientError::Timeout) => {
                log::warn!("service heartbeat missed (timeout)");
            }
            Err(error) => {
                log::warn!("service heartbeat exiting: {error}");
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

/// Drives the two-phase spawn flow for `spawn_with_events`. Runs the
/// existing `spawn_inner` (which does spawn + version-check ping), emits
/// `ChildSpawned`, issues `boot.ready`, and emits `BootReady` or
/// `Terminal(error)`.
async fn run_spawn_flow(
    binary: &Path,
    app_data_dir: PathBuf,
    extra_args: Vec<String>,
    tx: mpsc::Sender<SpawnEvent>,
) {
    let extra_arg_refs: Vec<&str> = extra_args.iter().map(String::as_str).collect();
    let client = match ServiceClient::spawn_inner(binary, &app_data_dir, &extra_arg_refs).await {
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
            let _ = tx.send(SpawnEvent::BootReady(response)).await;
        }
        Err(error) => {
            let _ = tx.send(SpawnEvent::Terminal(error)).await;
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

        let task = tokio::spawn(heartbeat_task(
            stdin_tx.clone(),
            Arc::clone(&pending),
            Arc::clone(&next_id),
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
}
