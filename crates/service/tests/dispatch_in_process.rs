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
    match parse_service_message(&line)? {
        ParsedServiceMessage::Response { id, response } => Ok((id, response)),
        ParsedServiceMessage::Notification(_) => {
            Err(std::io::Error::other("unexpected notification").into())
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
