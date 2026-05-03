use service_api::{
    BoundedLineReader, HealthPingResponse, JsonRpcRequest, ParsedServiceMessage, RequestParams,
    ServiceResponse, ShutdownResponse, parse_service_message, write_message,
};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// RAII handle for the per-test data directory. Removes the dir when dropped
/// (panic-on-test-failure included) so smoke runs don't accumulate stray
/// `target/service-smoke-*` directories.
struct DataDirGuard {
    path: PathBuf,
}

impl DataDirGuard {
    fn new() -> std::io::Result<Self> {
        let path = service_smoke_data_dir()?;
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &PathBuf {
        &self.path
    }
}

impl Drop for DataDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[tokio::test]
async fn service_subprocess_ping_and_shutdown() -> TestResult {
    let binary = option_env!("CARGO_BIN_EXE_app")
        .ok_or_else(|| std::io::Error::other("CARGO_BIN_EXE_app not set"))?;
    let data_dir = DataDirGuard::new()?;
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

fn service_smoke_data_dir() -> Result<PathBuf, std::io::Error> {
    Ok(std::env::current_dir()?
        .join("target")
        .join(format!("service-smoke-{}", std::process::id())))
}
