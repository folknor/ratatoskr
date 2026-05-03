use app::service_client::{ClientError, ServiceClient, SpawnEvent};
use service_api::{
    BootExitCode, BoundedLineReader, HealthPingResponse, JsonRpcRequest, ParsedServiceMessage,
    RequestParams, ServiceResponse, ShutdownResponse, parse_service_message, write_message,
};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// RAII handle for the per-test data directory. Removes the dir when dropped
/// (panic-on-test-failure included) so smoke runs don't accumulate stray
/// `target/service-smoke-*` directories.
///
/// Writes a dummy `ratatoskr.key` so the Service's boot-time key load
/// succeeds. Tests that need the missing-key case use
/// `DataDirGuard::without_key`.
struct DataDirGuard {
    path: PathBuf,
}

impl DataDirGuard {
    fn new(suffix: &str) -> std::io::Result<Self> {
        let guard = Self::create(suffix)?;
        write_dummy_key(&guard.path)?;
        Ok(guard)
    }

    fn without_key(suffix: &str) -> std::io::Result<Self> {
        Self::create(&format!("nokey-{suffix}"))
    }

    fn create(suffix: &str) -> std::io::Result<Self> {
        let path = std::env::current_dir()?
            .join("target")
            .join(format!("service-smoke-{}-{}", std::process::id(), suffix));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

fn write_dummy_key(dir: &Path) -> std::io::Result<()> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let key_bytes = [0u8; 32];
    let encoded = STANDARD.encode(key_bytes);
    std::fs::write(dir.join("ratatoskr.key"), encoded)
}

impl Drop for DataDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn binary_path() -> Result<&'static str, std::io::Error> {
    option_env!("CARGO_BIN_EXE_app")
        .ok_or_else(|| std::io::Error::other("CARGO_BIN_EXE_app not set"))
}

#[tokio::test]
async fn service_subprocess_ping_and_shutdown() -> TestResult {
    let binary = binary_path()?;
    let data_dir = DataDirGuard::new("ping_and_shutdown")?;
    let app_data_dir = data_dir.path();

    let mut child = Command::new(binary)
        .arg("--service")
        .arg("--app-data-dir")
        .arg(app_data_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(false)
        .spawn()?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| std::io::Error::other("missing child stdin"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("missing child stdout"))?;
    let mut stdout = BoundedLineReader::new(stdout, service_api::MAX_FRAME_BYTES);

    write_message(
        &JsonRpcRequest::new(1, &RequestParams::HealthPing),
        &mut stdin,
    )
    .await?;
    let (id, response) = read_response(&mut stdout).await?;
    assert_eq!(id, Some(1));
    let ServiceResponse::Success(value) = response else {
        return Err(std::io::Error::other("expected ping success").into());
    };
    let ping: HealthPingResponse = serde_json::from_value(value)?;
    assert_eq!(ping.version, service_api::PROTOCOL_VERSION);

    write_message(
        &JsonRpcRequest::new(2, &RequestParams::Shutdown),
        &mut stdin,
    )
    .await?;
    let (id, response) = read_response(&mut stdout).await?;
    assert_eq!(id, Some(2));
    let ServiceResponse::Success(value) = response else {
        return Err(std::io::Error::other("expected shutdown success").into());
    };
    let shutdown: ShutdownResponse = serde_json::from_value(value)?;
    assert!(shutdown.flushed_ok);

    let status = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await??;
    assert!(status.success());
    Ok(())
}

async fn read_response<R>(
    stdout: &mut BoundedLineReader<R>,
) -> TestResult<(Option<u64>, ServiceResponse)>
where
    R: tokio::io::AsyncRead + Unpin,
{
    // The boot sequence emits `boot.progress` notifications concurrently
    // with the dispatch loop, so we may see one or more notifications
    // before the response we're waiting on. Skip them and keep reading.
    loop {
        let line = stdout
            .next_line()
            .await?
            .ok_or_else(|| std::io::Error::other("service closed stdout"))?;
        match parse_service_message(&line)? {
            ParsedServiceMessage::Response { id, response } => return Ok((id, response)),
            ParsedServiceMessage::Notification(_) => continue,
        }
    }
}

/// Drop ServiceClient without calling shutdown(). The OS-level child must
/// exit promptly via the explicit Drop teardown (abort tasks, close stdin,
/// SIGKILL fallback). No orphan should remain.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn dropping_client_terminates_child_within_one_second() -> TestResult {
    let binary = binary_path()?;
    let data_dir = DataDirGuard::new("drop_no_shutdown")?;
    let client = ServiceClient::spawn_for_test(Path::new(binary), data_dir.path(), &[]).await?;
    let pid = client
        .child_pid()
        .ok_or_else(|| std::io::Error::other("child has no pid"))?;
    drop(client);

    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_millis(1500) {
        if !pid_is_alive(pid)? {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    Err(std::io::Error::other(format!(
        "Service pid {pid} still alive {:?} after Drop",
        started.elapsed()
    ))
    .into())
}

/// Pointing at a non-existent binary must surface a clear error rather than
/// hang. Tests the spawn-failure path of ServiceClient::spawn_for_test.
#[tokio::test]
async fn spawn_failure_against_missing_binary_returns_io_error() -> TestResult {
    let data_dir = DataDirGuard::new("spawn_failure")?;
    let bogus = data_dir.path().join("does-not-exist");
    let result =
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            ServiceClient::spawn_for_test(&bogus, data_dir.path(), &[]),
        )
        .await
        .map_err(|_| std::io::Error::other("spawn hung past timeout"))?;
    match result {
        Err(ClientError::Io(_)) => Ok(()),
        Err(other) => Err(std::io::Error::other(format!(
            "expected ClientError::Io, got {other:?}"
        ))
        .into()),
        Ok(_) => Err(std::io::Error::other("spawn unexpectedly succeeded").into()),
    }
}

/// SIGKILL the helper that spawned the Service; the kernel's
/// PR_SET_PDEATHSIG (set on the child via `pre_exec`) must fire promptly
/// and the Service must exit within ~2 s. Linux-only - macOS is deferred,
/// Windows uses Job Object KILL_ON_JOB_CLOSE which can only be exercised
/// on a real Windows host.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn linux_parent_sigkill_terminates_service_within_two_seconds() -> TestResult {
    use tokio::io::AsyncBufReadExt;

    let service_binary = binary_path()?;
    let helper_binary = option_env!("CARGO_BIN_EXE_parent_death_helper").ok_or_else(|| {
        std::io::Error::other("CARGO_BIN_EXE_parent_death_helper not set")
    })?;
    let data_dir = DataDirGuard::new("parent_sigkill")?;

    let mut helper = Command::new(helper_binary)
        .arg(service_binary)
        .arg(data_dir.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()?;

    let stdout = helper
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("helper has no stdout"))?;
    let mut reader = tokio::io::BufReader::new(stdout);
    let mut line = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        reader.read_line(&mut line),
    )
    .await
    .map_err(|_| std::io::Error::other("helper did not print pid in time"))??;
    let service_pid: u32 = line
        .trim()
        .parse()
        .map_err(|e| std::io::Error::other(format!("parse pid {line:?}: {e}")))?;

    let helper_pid = helper
        .id()
        .ok_or_else(|| std::io::Error::other("helper has no pid"))?;
    let helper_pid = i32::try_from(helper_pid).map_err(std::io::Error::other)?;
    // SAFETY: SIGKILL on a known PID we just spawned. Holding the
    // `kill_on_drop(true)` Child handle keeps the PID stable.
    let kill_result = unsafe { libc::kill(helper_pid, libc::SIGKILL) };
    if kill_result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }

    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_secs(3) {
        if !pid_is_alive(service_pid)? {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    Err(std::io::Error::other(format!(
        "Service pid {service_pid} still alive {:?} after parent SIGKILL",
        started.elapsed()
    ))
    .into())
}

/// Service handler calls `println!` from inside the dispatch loop.
/// Without the stdio-defense (dup the original stdin/stdout to saved fds,
/// redirect the globals to /dev/null), the println would corrupt the
/// JSON-RPC pipe and the next request would fail to parse. With it in
/// place, the TestPrintln response is well-formed and a follow-up ping
/// still round-trips.
#[tokio::test]
async fn println_from_handler_does_not_corrupt_json_rpc_framing() -> TestResult {
    let binary = binary_path()?;
    let data_dir = DataDirGuard::new("println_defense")?;
    let client = ServiceClient::spawn_for_test(Path::new(binary), data_dir.path(), &[]).await?;

    let _: () = client
        .request(RequestParams::TestPrintln {
            message: "STDIO-CORRUPTION-CANARY-XYZ".to_string(),
        })
        .await?;

    let ping: HealthPingResponse = client.request(RequestParams::HealthPing).await?;
    assert_eq!(ping.version, service_api::PROTOCOL_VERSION);

    Ok(())
}

/// Service returns a wrong protocol version (driven by the test-helpers
/// `--test-fake-version` flag); ServiceClient::spawn must surface
/// `ClientError::VersionMismatch` rather than continuing with a bogus
/// peer.
#[tokio::test]
async fn version_mismatch_surfaces_during_handshake() -> TestResult {
    let binary = binary_path()?;
    let data_dir = DataDirGuard::new("version_mismatch")?;
    let result = ServiceClient::spawn_for_test(
        Path::new(binary),
        data_dir.path(),
        &["--test-fake-version=999"],
    )
    .await;
    match result {
        Err(ClientError::VersionMismatch { ui, service }) => {
            assert_eq!(ui, service_api::PROTOCOL_VERSION);
            assert_eq!(service, 999);
            Ok(())
        }
        Err(other) => Err(std::io::Error::other(format!(
            "expected VersionMismatch, got {other:?}"
        ))
        .into()),
        Ok(_) => Err(std::io::Error::other("spawn unexpectedly succeeded").into()),
    }
}

/// EOF on the child's stdout (Service crashed / killed mid-request) must
/// propagate to every pending caller as `ClientError::ServiceCrashed`. The
/// reader task evicts the pending map on EOF; this test verifies the eviction
/// is observable end-to-end.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn pending_request_fails_with_service_crashed_when_child_killed() -> TestResult {
    let binary = binary_path()?;
    let data_dir = DataDirGuard::new("eof_during_pending")?;
    let client = ServiceClient::spawn_for_test(Path::new(binary), data_dir.path(), &[]).await?;
    let pid = client
        .child_pid()
        .ok_or_else(|| std::io::Error::other("child has no pid"))?;

    // Issue a long-running request in the background so the request is
    // genuinely pending when we kill the child. Use TestSlow with a duration
    // longer than the test's overall budget so the only way the future
    // resolves is via the EOF eviction path.
    let request_client = std::sync::Arc::clone(&client);
    let request_task = tokio::spawn(async move {
        request_client
            .request::<()>(RequestParams::TestSlow { millis: 60_000 })
            .await
    });

    // Wait briefly for the request to be in-flight on the wire. The handler
    // is sleeping; the Service has not yet sent a response.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let pid_signed = i32::try_from(pid).map_err(std::io::Error::other)?;
    // SAFETY: SIGKILL on a known PID we just spawned. The ServiceClient
    // holds the Child handle so the kernel keeps the PID stable.
    let kill_result = unsafe { libc::kill(pid_signed, libc::SIGKILL) };
    if kill_result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }

    let outcome = tokio::time::timeout(std::time::Duration::from_secs(3), request_task)
        .await
        .map_err(|_| std::io::Error::other("pending request did not resolve after SIGKILL"))?
        .map_err(|e| std::io::Error::other(format!("request task join: {e}")))?;

    match outcome {
        Err(ClientError::ServiceCrashed) => Ok(()),
        Err(other) => Err(std::io::Error::other(format!(
            "expected ClientError::ServiceCrashed, got {other:?}"
        ))
        .into()),
        Ok(()) => Err(std::io::Error::other(
            "pending request unexpectedly succeeded after SIGKILL",
        )
        .into()),
    }
}

/// `spawn_with_events` against a healthy data dir emits ChildSpawned then
/// BootReady, in that order. Validates the two-phase contract: the App can
/// receive the client (and subscribe to notifications) before the slow
/// boot.ready round-trip completes.
#[tokio::test(flavor = "multi_thread")]
async fn spawn_with_events_emits_child_spawned_then_boot_ready_on_healthy_boot()
-> TestResult {
    let binary = binary_path()?;
    let data_dir = DataDirGuard::new("two_phase_happy")?;
    let mut events = ServiceClient::spawn_with_events_for_test(
        std::path::PathBuf::from(binary),
        data_dir.path().to_path_buf(),
        Vec::new(),
    );

    let first = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
        .await
        .map_err(|_| std::io::Error::other("ChildSpawned did not arrive in time"))?
        .ok_or_else(|| std::io::Error::other("event stream closed without ChildSpawned"))?;
    let client = match first {
        SpawnEvent::ChildSpawned(client) => client,
        SpawnEvent::BootReady(_) => {
            return Err(std::io::Error::other("BootReady arrived before ChildSpawned").into());
        }
        SpawnEvent::Terminal(error) => {
            return Err(std::io::Error::other(format!("unexpected Terminal: {error:?}")).into());
        }
    };

    let second = tokio::time::timeout(std::time::Duration::from_secs(15), events.recv())
        .await
        .map_err(|_| std::io::Error::other("BootReady did not arrive in time"))?
        .ok_or_else(|| std::io::Error::other("event stream closed without BootReady"))?;
    match second {
        SpawnEvent::BootReady(response) => {
            assert!(response.ready);
            assert_eq!(response.schema_version, 100);
            assert_eq!(response.migrations_applied, 1);
        }
        other => {
            return Err(std::io::Error::other(format!(
                "expected BootReady second, got {other:?}"
            ))
            .into());
        }
    }

    // Tear down cleanly.
    let _ = client.shutdown().await;
    Ok(())
}

/// `spawn_with_events` against a data dir without `ratatoskr.key`: ping
/// succeeds (Service is up), so ChildSpawned arrives. boot.ready then
/// fails when the Service exits with KeyLoadFailure. Either the boot.ready
/// reply carries the Service error, or the Service has already exited and
/// the request fails with ServiceCrashed - either is a valid Terminal.
#[tokio::test(flavor = "multi_thread")]
async fn spawn_with_events_emits_terminal_on_missing_key() -> TestResult {
    let binary = binary_path()?;
    let data_dir = DataDirGuard::without_key("two_phase_missing_key")?;
    let mut events = ServiceClient::spawn_with_events_for_test(
        std::path::PathBuf::from(binary),
        data_dir.path().to_path_buf(),
        Vec::new(),
    );

    let first = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
        .await
        .map_err(|_| std::io::Error::other("first event did not arrive"))?
        .ok_or_else(|| std::io::Error::other("event stream closed empty"))?;
    let _client = match first {
        SpawnEvent::ChildSpawned(client) => client,
        SpawnEvent::Terminal(error) => {
            // It's also valid for spawn-time to fail before ChildSpawned if
            // the version-check ping never gets answered - but with the
            // Service successfully starting up + answering ping, this path
            // is unlikely. Fall through with the error.
            return Err(std::io::Error::other(format!(
                "expected ChildSpawned (Service should answer ping), got Terminal: {error:?}"
            ))
            .into());
        }
        SpawnEvent::BootReady(_) => {
            return Err(std::io::Error::other(
                "BootReady should not succeed on missing-key dir",
            )
            .into());
        }
    };

    let second = tokio::time::timeout(std::time::Duration::from_secs(10), events.recv())
        .await
        .map_err(|_| std::io::Error::other("Terminal did not arrive in time"))?
        .ok_or_else(|| std::io::Error::other("event stream closed without second event"))?;
    match second {
        SpawnEvent::Terminal(_error) => {}
        other => {
            return Err(std::io::Error::other(format!(
                "expected Terminal second, got {other:?}"
            ))
            .into());
        }
    }
    Ok(())
}

/// Spawning a Service against a data dir without a `ratatoskr.key` file
/// must exit with `BootExitCode::KeyLoadFailure` (code 73) - that's the
/// terminal-failure signal the UI maps to "Encryption key missing or
/// unreadable" rather than treating it as a generic crash.
#[tokio::test(flavor = "multi_thread")]
async fn missing_key_file_exits_with_key_load_failure_code() -> TestResult {
    let binary = binary_path()?;
    let data_dir = DataDirGuard::without_key("missing_key")?;
    let mut child = Command::new(binary)
        .arg("--service")
        .arg("--app-data-dir")
        .arg(data_dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()?;
    // Hold the parent's writer end of stdin so `child.wait()` does NOT drop
    // it (tokio::process::Child::wait() implicitly drops self.stdin.take()
    // to keep blocked children from hanging forever, but here we want the
    // boot sequence to fail on its own terms - on stdin EOF the dispatch
    // loop would break before the key-load step finishes).
    let _stdin_keepalive = child
        .stdin
        .take()
        .ok_or_else(|| std::io::Error::other("child has no stdin"))?;
    let status = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await??;
    assert_eq!(
        status.code(),
        Some(BootExitCode::KeyLoadFailure.as_i32()),
        "expected KeyLoadFailure (73), got {status:?}"
    );
    Ok(())
}

/// Two `--service` instances against the same data dir: the first takes the
/// fs2 file lock at boot; the second hits the contended path and exits with
/// `BootExitCode::AnotherInstanceRunning` (code 71). Lets the UI surface
/// "Ratatoskr is already running" rather than treating it as a crash.
#[tokio::test(flavor = "multi_thread")]
async fn second_instance_against_same_data_dir_exits_with_already_running() -> TestResult {
    let binary = binary_path()?;
    let data_dir = DataDirGuard::new("instance_lock")?;
    let app_data_dir = data_dir.path();

    // Service A: spawn and wait for it to be past lock acquisition. The
    // ping/pong proves A has reached the dispatch loop, which only happens
    // after the lock is held.
    let mut a = Command::new(binary)
        .arg("--service")
        .arg("--app-data-dir")
        .arg(app_data_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()?;

    let mut a_stdin = a
        .stdin
        .take()
        .ok_or_else(|| std::io::Error::other("missing a stdin"))?;
    let a_stdout = a
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("missing a stdout"))?;
    let mut a_reader = BoundedLineReader::new(a_stdout, service_api::MAX_FRAME_BYTES);

    write_message(
        &JsonRpcRequest::new(1, &RequestParams::HealthPing),
        &mut a_stdin,
    )
    .await?;
    let (id, _response) = read_response(&mut a_reader).await?;
    assert_eq!(id, Some(1));

    // Service B: should exit with code 71 quickly. We don't drive its IPC -
    // the lock check fires before any tokio runtime work.
    let mut b = Command::new(binary)
        .arg("--service")
        .arg("--app-data-dir")
        .arg(app_data_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()?;

    let b_status = tokio::time::timeout(std::time::Duration::from_secs(5), b.wait()).await??;
    assert_eq!(
        b_status.code(),
        Some(BootExitCode::AnotherInstanceRunning.as_i32()),
        "Service B should exit with AnotherInstanceRunning (71); got {b_status:?}"
    );

    // Clean teardown of A so the lock is released.
    write_message(
        &JsonRpcRequest::new(2, &RequestParams::Shutdown),
        &mut a_stdin,
    )
    .await?;
    let (id, _response) = read_response(&mut a_reader).await?;
    assert_eq!(id, Some(2));
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), a.wait()).await??;
    Ok(())
}

/// SIGKILL the running Service mid-session and verify that the
/// respawn machinery (commit 14) brings up a replacement child and re-emits
/// `ChildSpawned` + `BootReady` on the same `SpawnEvent` receiver. A
/// follow-up `health.ping` against the respawned client must succeed,
/// proving end-to-end that the new state is live.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn respawn_after_sigkill_succeeds() -> TestResult {
    let binary = binary_path()?;
    let data_dir = DataDirGuard::new("respawn_after_sigkill")?;
    let mut events = ServiceClient::spawn_with_events_for_test(
        std::path::PathBuf::from(binary),
        data_dir.path().to_path_buf(),
        Vec::new(),
    );

    // Initial ChildSpawned + BootReady.
    let first_event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
        .await
        .map_err(|_| std::io::Error::other("ChildSpawned did not arrive in time"))?
        .ok_or_else(|| std::io::Error::other("event stream closed without ChildSpawned"))?;
    let initial_client = match first_event {
        SpawnEvent::ChildSpawned(client) => client,
        other => {
            return Err(std::io::Error::other(format!(
                "expected initial ChildSpawned, got {other:?}",
            ))
            .into());
        }
    };
    let second_event = tokio::time::timeout(std::time::Duration::from_secs(15), events.recv())
        .await
        .map_err(|_| std::io::Error::other("initial BootReady did not arrive in time"))?
        .ok_or_else(|| std::io::Error::other("event stream closed without BootReady"))?;
    match second_event {
        SpawnEvent::BootReady(_) => {}
        other => {
            return Err(std::io::Error::other(format!(
                "expected initial BootReady, got {other:?}"
            ))
            .into());
        }
    }

    let initial_pid = initial_client
        .child_pid()
        .ok_or_else(|| std::io::Error::other("initial child has no pid"))?;
    let initial_pid_signed = i32::try_from(initial_pid).map_err(std::io::Error::other)?;
    // SAFETY: SIGKILL on a known PID held alive by the ServiceClient's
    // child handle. The client keeps the PID stable until wait().
    let kill_result = unsafe { libc::kill(initial_pid_signed, libc::SIGKILL) };
    if kill_result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }

    // Reader observes EOF, fires handle_crash, runs the 1s respawn cooldown,
    // launches a replacement Service, version-check pings, re-emits
    // ChildSpawned then BootReady. Budget covers the 1s sleep + spawn +
    // boot.ready (sub-second on a non-migrating reopen).
    let respawn_first =
        tokio::time::timeout(std::time::Duration::from_secs(15), events.recv())
            .await
            .map_err(|_| std::io::Error::other("respawn ChildSpawned did not arrive in time"))?
            .ok_or_else(|| std::io::Error::other("event stream closed before respawn"))?;
    let respawned_client = match respawn_first {
        SpawnEvent::ChildSpawned(client) => client,
        other => {
            return Err(std::io::Error::other(format!(
                "expected respawn ChildSpawned, got {other:?}",
            ))
            .into());
        }
    };
    let respawn_second =
        tokio::time::timeout(std::time::Duration::from_secs(15), events.recv())
            .await
            .map_err(|_| std::io::Error::other("respawn BootReady did not arrive in time"))?
            .ok_or_else(|| std::io::Error::other("event stream closed before respawn BootReady"))?;
    match respawn_second {
        SpawnEvent::BootReady(response) => {
            assert!(response.ready);
            assert_eq!(response.schema_version, 100);
        }
        other => {
            return Err(std::io::Error::other(format!(
                "expected respawn BootReady, got {other:?}",
            ))
            .into());
        }
    }

    // ServiceClient is the same allocation across the respawn (the App holds
    // one Arc and respawn replaces internal RunningState in place); proof:
    // the post-respawn ChildSpawned hands back the same Arc.
    assert!(
        std::sync::Arc::ptr_eq(&initial_client, &respawned_client),
        "respawn must replace state in place; the Arc must be identical",
    );

    // Sanity: a health.ping against the respawned subprocess must round-trip
    // through the new RunningState. The new child has a different PID.
    let respawned_pid = respawned_client
        .child_pid()
        .ok_or_else(|| std::io::Error::other("respawned child has no pid"))?;
    assert_ne!(initial_pid, respawned_pid);
    let ping: HealthPingResponse = respawned_client
        .request(RequestParams::HealthPing)
        .await?;
    assert_eq!(ping.version, service_api::PROTOCOL_VERSION);
    assert_eq!(ping.pid, respawned_pid);

    let _ = respawned_client.shutdown().await;
    Ok(())
}

/// Boot-time KeyLoadFailure must NOT trigger respawn: handle_crash sees
/// `first_boot_ready` is None and defers to run_spawn_flow (which already
/// surfaces Terminal). No follow-up events should arrive on the receiver.
/// Closes the crashloop concern from scope item 15: a missing key file
/// would otherwise produce one Service-per-second forever.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn terminal_failure_at_initial_boot_does_not_respawn() -> TestResult {
    let binary = binary_path()?;
    let data_dir = DataDirGuard::without_key("terminal_no_respawn")?;
    let mut events = ServiceClient::spawn_with_events_for_test(
        std::path::PathBuf::from(binary),
        data_dir.path().to_path_buf(),
        Vec::new(),
    );

    let first = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
        .await
        .map_err(|_| std::io::Error::other("first event did not arrive"))?
        .ok_or_else(|| std::io::Error::other("event stream closed empty"))?;
    let _client = match first {
        SpawnEvent::ChildSpawned(client) => client,
        SpawnEvent::Terminal(_) => {
            // Allowed: spawn might fail before ChildSpawned in some
            // environments, in which case there's nothing left to test.
            return Ok(());
        }
        other => {
            return Err(std::io::Error::other(format!(
                "expected ChildSpawned or Terminal, got {other:?}",
            ))
            .into());
        }
    };
    let second = tokio::time::timeout(std::time::Duration::from_secs(10), events.recv())
        .await
        .map_err(|_| std::io::Error::other("Terminal did not arrive in time"))?
        .ok_or_else(|| std::io::Error::other("event stream closed without Terminal"))?;
    match second {
        SpawnEvent::Terminal(_) => {}
        other => {
            return Err(std::io::Error::other(format!(
                "expected Terminal, got {other:?}"
            ))
            .into());
        }
    }

    // Window for respawn would be ~1s sleep + spawn + boot.ready. Wait
    // longer than that and assert no follow-up event arrives.
    let result =
        tokio::time::timeout(std::time::Duration::from_secs(4), events.recv()).await;
    match result {
        Ok(Some(unexpected)) => {
            return Err(std::io::Error::other(format!(
                "expected no respawn after Terminal; got {unexpected:?}"
            ))
            .into());
        }
        Ok(None) => {
            // Sender dropped; that's fine - no respawn fired.
        }
        Err(_) => {
            // Timeout: no event arrived in the budget window. That's the
            // pass condition.
        }
    }
    Ok(())
}

#[cfg(unix)]
fn pid_is_alive(pid: u32) -> std::io::Result<bool> {
    let pid = i32::try_from(pid).map_err(std::io::Error::other)?;
    // SAFETY: kill(pid, 0) only checks reachability + permission; no signal
    // is delivered. The libc ABI is straightforward.
    let result = unsafe { libc::kill(pid, 0) };
    if result == 0 {
        return Ok(true);
    }
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(libc::ESRCH) => Ok(false),
        // EPERM means the process exists but we can't signal it - still alive.
        Some(libc::EPERM) => Ok(true),
        _ => Err(err),
    }
}
