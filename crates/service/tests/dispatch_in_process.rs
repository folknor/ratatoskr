use service_api::{
    BoundedLineReader, HealthPingResponse, JsonRpcRequest, ParsedServiceMessage, RequestParams,
    ServiceResponse, ShutdownResponse, parse_service_message, write_message,
};
use tokio::io::{AsyncWriteExt, DuplexStream};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

struct Harness {
    stdin: DuplexStream,
    stdout: BoundedLineReader<DuplexStream>,
    service: tokio::task::JoinHandle<i32>,
}

fn spawn_harness() -> Harness {
    let (client_stdin, service_stdin) = tokio::io::duplex(1024 * 1024);
    let (service_stdout, client_stdout) = tokio::io::duplex(1024 * 1024);
    let service = tokio::spawn(service::run_service_with_io(
        service_stdin,
        service_stdout,
    ));
    Harness {
        stdin: client_stdin,
        stdout: BoundedLineReader::new(client_stdout, service_api::MAX_FRAME_BYTES),
        service,
    }
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
    let line = stdout
        .next_line()
        .await?
        .ok_or_else(|| std::io::Error::other("service closed stdout"))?;
    match parse_service_message(&line) {
        Ok(ParsedServiceMessage::Response { id, response }) => Ok((id, response)),
        Ok(ParsedServiceMessage::Notification(_)) => {
            Err(std::io::Error::other("unexpected notification").into())
        }
        Err(error) => Err(std::io::Error::other(format!(
            "parse_service_message failed: {error}\nline: {line}"
        ))
        .into()),
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
