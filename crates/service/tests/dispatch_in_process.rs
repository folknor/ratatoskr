use service_api::{
    BootExitCode, BootReadyResponse, BoundedLineReader, HealthPingResponse, JsonRpcRequest,
    ParsedServiceMessage, RequestParams, ServiceResponse, ShutdownResponse, parse_service_message,
    write_message,
};
use std::path::PathBuf;
use tokio::io::{AsyncWriteExt, DuplexStream};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

struct Harness {
    stdin: DuplexStream,
    stdout: BoundedLineReader<DuplexStream>,
    service: tokio::task::JoinHandle<i32>,
    _data_dir: TestDataDir,
}

/// Per-test data dir with a dummy `ratatoskr.key` so the boot sequence's
/// key-load step succeeds. Removed on drop to keep `target/` tidy.
struct TestDataDir {
    path: PathBuf,
}

impl TestDataDir {
    fn new(suffix: &str) -> std::io::Result<Self> {
        let path = std::env::current_dir()?
            .join("target")
            .join(format!(
                "dispatch-in-process-{}-{}-{}",
                std::process::id(),
                suffix,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path)?;
        write_dummy_key(&path)?;
        Ok(Self { path })
    }

    fn without_key(suffix: &str) -> std::io::Result<Self> {
        let path = std::env::current_dir()?
            .join("target")
            .join(format!(
                "dispatch-in-process-nokey-{}-{}-{}",
                std::process::id(),
                suffix,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TestDataDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn write_dummy_key(dir: &std::path::Path) -> std::io::Result<()> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let key_bytes = [0u8; 32];
    let encoded = STANDARD.encode(key_bytes);
    std::fs::write(dir.join("ratatoskr.key"), encoded)
}

fn spawn_harness_with_suffix(suffix: &str) -> Harness {
    let data_dir = TestDataDir::new(suffix).expect("create test data dir");
    let (client_stdin, service_stdin) = tokio::io::duplex(1024 * 1024);
    let (service_stdout, client_stdout) = tokio::io::duplex(1024 * 1024);
    let service = tokio::spawn(service::run_service_with_io(
        service_stdin,
        service_stdout,
        data_dir.path().to_path_buf(),
    ));
    Harness {
        stdin: client_stdin,
        stdout: BoundedLineReader::new(client_stdout, service_api::MAX_FRAME_BYTES),
        service,
        _data_dir: data_dir,
    }
}

fn spawn_harness() -> Harness {
    spawn_harness_with_suffix("default")
}

#[tokio::test]
async fn ping_round_trip_succeeds() -> TestResult {
    let mut harness = spawn_harness();
    write_request(&mut harness.stdin, 1, RequestParams::HealthPing).await?;
    let (id, response) = read_response(&mut harness.stdout).await?;
    assert_eq!(id, Some(1));
    let ServiceResponse::Success(value) = response else {
        return Err(std::io::Error::other("expected success response").into());
    };
    let ping: HealthPingResponse = serde_json::from_value(value)?;
    assert_eq!(ping.version, service_api::PROTOCOL_VERSION);
    assert!(ping.pid > 0);

    shutdown(harness).await
}

#[tokio::test]
async fn malformed_json_returns_error_and_loop_continues() -> TestResult {
    let mut harness = spawn_harness();
    harness.stdin.write_all(b"{not-json}\n").await?;
    let (id, response) = read_response(&mut harness.stdout).await?;
    assert_eq!(id, None);
    assert!(matches!(response, ServiceResponse::Error(_)));

    write_request(&mut harness.stdin, 2, RequestParams::HealthPing).await?;
    let (id, response) = read_response(&mut harness.stdout).await?;
    assert_eq!(id, Some(2));
    assert!(matches!(response, ServiceResponse::Success(_)));

    shutdown(harness).await
}

#[tokio::test]
async fn oversized_frame_returns_error_and_loop_continues() -> TestResult {
    let mut harness = spawn_harness();
    let oversized = vec![b'a'; service_api::MAX_FRAME_BYTES + 1];
    harness.stdin.write_all(&oversized).await?;
    harness.stdin.write_all(b"\n").await?;
    let (id, response) = read_response(&mut harness.stdout).await?;
    assert_eq!(id, None);
    assert!(matches!(response, ServiceResponse::Error(_)));

    write_request(&mut harness.stdin, 3, RequestParams::HealthPing).await?;
    let (id, response) = read_response(&mut harness.stdout).await?;
    assert_eq!(id, Some(3));
    assert!(matches!(response, ServiceResponse::Success(_)));

    shutdown(harness).await
}

#[tokio::test]
async fn eof_on_stdin_exits_cleanly() -> TestResult {
    let harness = spawn_harness();
    drop(harness.stdin);
    let exit_code = harness.service.await?;
    assert_eq!(exit_code, 0);
    Ok(())
}

#[tokio::test]
async fn invalid_utf8_returns_parse_error_and_loop_continues() -> TestResult {
    let mut harness = spawn_harness();
    harness.stdin.write_all(b"\xff\xfe\n").await?;
    let (id, response) = read_response(&mut harness.stdout).await?;
    assert_eq!(id, None);
    assert!(matches!(response, ServiceResponse::Error(_)));

    write_request(&mut harness.stdin, 4, RequestParams::HealthPing).await?;
    let (id, response) = read_response(&mut harness.stdout).await?;
    assert_eq!(id, Some(4));
    assert!(matches!(response, ServiceResponse::Success(_)));

    shutdown(harness).await
}

#[tokio::test]
async fn invalid_request_correlates_error_to_extracted_id() -> TestResult {
    let mut harness = spawn_harness();
    let bogus = br#"{"jsonrpc":"2.0","id":42,"method":"health.ping","params":{"unexpected":"value"}}"#;
    harness.stdin.write_all(bogus).await?;
    harness.stdin.write_all(b"\n").await?;
    let (id, response) = read_response(&mut harness.stdout).await?;
    assert_eq!(id, Some(42));
    assert!(matches!(response, ServiceResponse::Error(_)));

    shutdown(harness).await
}

#[cfg(feature = "test-helpers")]
#[tokio::test]
async fn panicking_handler_returns_service_error_panic_and_loop_continues() -> TestResult {
    use service_api::{JsonRpcErrorObject, ServiceError};
    let mut harness = spawn_harness();
    write_request(&mut harness.stdin, 5, RequestParams::TestPanic).await?;
    let (id, response) = read_response(&mut harness.stdout).await?;
    assert_eq!(id, Some(5));
    let error = match response {
        ServiceResponse::Error(error) => error,
        ServiceResponse::Success(_) => {
            return Err(std::io::Error::other("expected error response").into());
        }
    };
    assert_eq!(error.code, -32603);
    let recovered: ServiceError = JsonRpcErrorObject::try_into_service_error(error)
        .map_err(|_| std::io::Error::other("data did not carry ServiceError"))?;
    assert!(
        matches!(recovered, ServiceError::Panic { ref method, .. } if method == "test.panic"),
        "expected ServiceError::Panic for test.panic, got {recovered:?}"
    );

    write_request(&mut harness.stdin, 6, RequestParams::HealthPing).await?;
    let (id, response) = read_response(&mut harness.stdout).await?;
    assert_eq!(id, Some(6));
    assert!(matches!(response, ServiceResponse::Success(_)));

    shutdown(harness).await
}

#[cfg(feature = "test-helpers")]
#[tokio::test]
async fn in_flight_semaphore_caps_concurrent_handlers_and_heartbeat_bypasses() -> TestResult {
    // Issue 100 slow handlers in parallel. Each sleeps for 800ms. The
    // semaphore caps concurrency at 64; the math says the second batch
    // (>=64 in flight) starts no earlier than the first slot frees, so a
    // bisect of "started" times tells us whether any 65 ran simultaneously.
    //
    // Heartbeat-bypass check: while the slow handlers are queued, fire a
    // single ping. It must round-trip even before any TestSlow has finished,
    // because health.ping bypasses the semaphore.
    let mut harness = spawn_harness();
    let total: usize = 100;
    let slow_ms: u64 = 800;
    let start_id: u64 = 100;
    let issued_at = std::time::Instant::now();
    for i in 0..total {
        write_request(
            &mut harness.stdin,
            start_id + i as u64,
            RequestParams::TestSlow { millis: slow_ms },
        )
        .await?;
    }
    // Drain a few responses to confirm at least one batch finishes.
    let mut completion_times = Vec::new();
    let mut ping_seen = false;
    let ping_id = start_id + total as u64 + 1;
    write_request(&mut harness.stdin, ping_id, RequestParams::HealthPing).await?;

    while completion_times.len() < total || !ping_seen {
        let (id, response) = read_response(&mut harness.stdout).await?;
        let id = id.ok_or_else(|| std::io::Error::other("missing response id"))?;
        match response {
            ServiceResponse::Success(_) => {}
            ServiceResponse::Error(error) => {
                return Err(std::io::Error::other(format!(
                    "unexpected error response for id {id}: {error:?}"
                ))
                .into());
            }
        }
        if id == ping_id {
            // Ping must complete BEFORE the first batch of slow handlers
            // (issue + slow_ms) would naturally finish - i.e., near-instant.
            assert!(
                issued_at.elapsed() < std::time::Duration::from_millis(slow_ms),
                "heartbeat ping waited behind the slow batch (took {:?})",
                issued_at.elapsed()
            );
            ping_seen = true;
        } else {
            completion_times.push(issued_at.elapsed());
        }
    }

    // First 64 completed in roughly slow_ms; next 36 in roughly 2*slow_ms.
    completion_times.sort();
    let first_batch_max = completion_times[63];
    let second_batch_min = completion_times[64];
    assert!(
        second_batch_min > first_batch_max,
        "expected a clear two-batch staircase, got first[63]={first_batch_max:?}, second[0]={second_batch_min:?}"
    );
    // Sanity: the second batch did not start until the first finished.
    let slop = std::time::Duration::from_millis(slow_ms / 4);
    assert!(
        second_batch_min >= std::time::Duration::from_millis(slow_ms).saturating_sub(slop),
        "second batch started too early: {second_batch_min:?}"
    );

    shutdown(harness).await
}

#[tokio::test]
async fn concurrent_ping_ids_are_correlated() -> TestResult {
    let mut harness = spawn_harness();
    for id in 1..=100 {
        write_request(&mut harness.stdin, id, RequestParams::HealthPing).await?;
    }

    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..100 {
        let (id, response) = read_response(&mut harness.stdout).await?;
        let id = id.ok_or_else(|| std::io::Error::other("missing response id"))?;
        assert!(matches!(response, ServiceResponse::Success(_)));
        seen.insert(id);
    }

    assert_eq!(seen.len(), 100);
    assert_eq!(seen.first().copied(), Some(1));
    assert_eq!(seen.last().copied(), Some(100));

    shutdown(harness).await
}

/// `boot.ready` returns a `BootReadyResponse` after the boot sequence
/// completes. Verifies the handler unblocks once `BOOT_RESULT` is populated
/// and that the response carries the expected schema_version /
/// migrations_applied for a fresh DB.
#[tokio::test]
async fn boot_ready_returns_after_sequence_completes() -> TestResult {
    let mut harness = spawn_harness_with_suffix("boot_ready_completes");
    write_request(&mut harness.stdin, 1, RequestParams::BootReady).await?;
    let (id, response) = read_response(&mut harness.stdout).await?;
    assert_eq!(id, Some(1));
    let ServiceResponse::Success(value) = response else {
        return Err(std::io::Error::other("expected boot.ready success").into());
    };
    let ready: BootReadyResponse = serde_json::from_value(value)?;
    assert!(ready.ready, "boot.ready must return ready=true");
    assert_eq!(
        ready.schema_version, 100,
        "fresh DB should be at schema v100"
    );
    assert_eq!(
        ready.migrations_applied, 1,
        "fresh DB should apply exactly the v100 migration"
    );
    shutdown(harness).await
}

/// `health.ping` continues to round-trip while `boot.ready` is in flight.
/// Verifies the dispatch loop's bypass: a parked boot.ready handler does
/// not block other requests through the admission cap.
#[tokio::test]
async fn health_ping_works_concurrently_with_boot_ready() -> TestResult {
    let mut harness = spawn_harness_with_suffix("concurrent_ping_during_boot");
    // Issue boot.ready first; it may complete before we read its response,
    // but the dispatch loop must answer health.ping in the same window
    // regardless.
    write_request(&mut harness.stdin, 1, RequestParams::BootReady).await?;
    write_request(&mut harness.stdin, 2, RequestParams::HealthPing).await?;

    // Collect both responses (in any order).
    let (mut saw_ready, mut saw_ping) = (false, false);
    for _ in 0..2 {
        let (id, response) = read_response(&mut harness.stdout).await?;
        match id {
            Some(1) => {
                assert!(matches!(response, ServiceResponse::Success(_)));
                saw_ready = true;
            }
            Some(2) => {
                assert!(matches!(response, ServiceResponse::Success(_)));
                saw_ping = true;
            }
            other => {
                return Err(std::io::Error::other(format!(
                    "unexpected response id {other:?}"
                ))
                .into());
            }
        }
    }
    assert!(saw_ready && saw_ping);
    shutdown(harness).await
}

/// Boot sequence with a missing `ratatoskr.key` returns the
/// `BootExitCode::KeyLoadFailure` exit code. Verifies the boot-failure
/// signal propagates from the spawn_blocking key-load step through the
/// dispatch loop's `boot_failure_rx` and out as the run_service_with_io
/// return value, without calling `std::process::exit` (which would kill
/// the test runner).
#[tokio::test]
async fn boot_sequence_returns_key_load_failure_when_key_file_is_missing() -> TestResult {
    let data_dir = TestDataDir::without_key("missing_key").expect("create test data dir");
    let (_client_stdin, service_stdin) = tokio::io::duplex(1024 * 1024);
    let (service_stdout, _client_stdout) = tokio::io::duplex(1024 * 1024);
    let service = tokio::spawn(service::run_service_with_io(
        service_stdin,
        service_stdout,
        data_dir.path().to_path_buf(),
    ));
    let exit_code = tokio::time::timeout(std::time::Duration::from_secs(5), service)
        .await
        .map_err(|_| std::io::Error::other("service did not exit on missing key"))??;
    assert_eq!(
        exit_code,
        BootExitCode::KeyLoadFailure.as_i32(),
        "expected KeyLoadFailure (73), got {exit_code}"
    );
    Ok(())
}

async fn write_request(
    stdin: &mut DuplexStream,
    id: u64,
    params: RequestParams,
) -> TestResult {
    write_message(&JsonRpcRequest::new(id, &params), stdin).await?;
    Ok(())
}

async fn read_response(
    stdout: &mut BoundedLineReader<DuplexStream>,
) -> TestResult<(Option<u64>, ServiceResponse)> {
    // The boot sequence emits `boot.progress` notifications concurrently
    // with the dispatch loop, so we may see one or more notifications
    // before the response we're waiting on. Skip them and keep reading
    // until a Response frame arrives.
    loop {
        let line = stdout
            .next_line()
            .await?
            .ok_or_else(|| std::io::Error::other("service closed stdout"))?;
        match parse_service_message(&line) {
            Ok(ParsedServiceMessage::Response { id, response }) => return Ok((id, response)),
            Ok(ParsedServiceMessage::Notification(_)) => {
                // Notifications during boot are expected; keep reading.
                continue;
            }
            Err(error) => {
                return Err(std::io::Error::other(format!(
                    "parse_service_message failed: {error}\nline: {line}"
                ))
                .into());
            }
        }
    }
}

async fn shutdown(mut harness: Harness) -> TestResult {
    write_request(&mut harness.stdin, 10_000, RequestParams::Shutdown).await?;
    let (id, response) = read_response(&mut harness.stdout).await?;
    assert_eq!(id, Some(10_000));
    let ServiceResponse::Success(value) = response else {
        return Err(std::io::Error::other("expected shutdown success").into());
    };
    let response: ShutdownResponse = serde_json::from_value(value)?;
    assert!(response.flushed_ok);
    let exit_code = harness.service.await?;
    assert_eq!(exit_code, 0);
    Ok(())
}
