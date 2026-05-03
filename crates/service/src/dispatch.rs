use crate::handlers;
use crate::lifecycle::ServiceLifecycle;
use futures_util::FutureExt;
use serde_json::Value;
use service_api::{
    BoundedLineReader, FrameError, JsonRpcErrorObject, JsonRpcErrorResponse,
    JsonRpcSuccessResponse, ParsedClientMessage, RequestParams, ServiceError, encode_message,
    parse_client_message, ShutdownResponse,
};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::{Semaphore, mpsc};

const OUTBOUND_QUEUE_CAP: usize = 1024;
const MAX_IN_FLIGHT: usize = 64;

pub async fn run_service_with_io<R, W>(reader: R, writer: W) -> i32
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    run_service_with_io_and_lifecycle(reader, writer, ServiceLifecycle::new(None)).await
}

pub(crate) async fn run_service_with_io_and_lifecycle<R, W>(
    reader: R,
    writer: W,
    lifecycle: ServiceLifecycle,
) -> i32
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let started_at = Instant::now();
    let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>(OUTBOUND_QUEUE_CAP);
    let writer_handle = tokio::spawn(writer_task(writer, out_rx));
    let inflight = Arc::new(Semaphore::new(MAX_IN_FLIGHT));
    let mut lines = BoundedLineReader::new(reader, service_api::MAX_FRAME_BYTES);

    loop {
        tokio::select! {
            () = lifecycle.notified() => {
                break;
            }
            line = lines.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        if handle_line(&line, &out_tx, &inflight, started_at, &lifecycle).await {
                            break;
                        }
                    }
                    Ok(None) => {
                        log::info!("service stdin closed");
                        break;
                    }
                    Err(FrameError::TooLarge) => {
                        log::warn!("rejecting oversized frame");
                        send_error(&out_tx, None, JsonRpcErrorObject::parse_error("frame too large")).await;
                    }
                    Err(FrameError::InvalidUtf8(error)) => {
                        log::warn!("service frame had invalid utf-8: {error}");
                        send_error(
                            &out_tx,
                            None,
                            JsonRpcErrorObject::parse_error("invalid utf-8"),
                        )
                        .await;
                    }
                    Err(FrameError::Io(error)) => {
                        log::warn!("service frame io error: {error}");
                        break;
                    }
                }
            }
        }
    }

    let flushed_ok = panic_safe_drain(&lifecycle).await;
    if !flushed_ok {
        log::warn!("shutdown drain completed with errors");
    }
    drop(out_tx);
    let _ = writer_handle.await;
    0
}

async fn handle_line(
    line: &str,
    out_tx: &mpsc::Sender<Vec<u8>>,
    inflight: &Arc<Semaphore>,
    started_at: Instant,
    lifecycle: &ServiceLifecycle,
) -> bool {
    match parse_client_message(line) {
        Ok(ParsedClientMessage::Request {
            id,
            params: RequestParams::Shutdown,
        }) => {
            let flushed_ok = panic_safe_drain(lifecycle).await;
            let result = serde_json::to_value(ShutdownResponse { flushed_ok })
                .map_err(|error| ServiceError::Internal(error.to_string()));
            send_handler_response(out_tx, id, result).await;
            lifecycle.request_shutdown();
            true
        }
        Ok(ParsedClientMessage::Request { id, params }) => {
            spawn_handler(id, params, out_tx.clone(), Arc::clone(inflight), started_at);
            false
        }
        Err(error) => {
            let response_id = error.extracted_id();
            log::warn!("request parse failed: {error}");
            send_error(
                out_tx,
                response_id,
                JsonRpcErrorObject::parse_error(error.to_string()),
            )
            .await;
            false
        }
    }
}

async fn panic_safe_drain(lifecycle: &ServiceLifecycle) -> bool {
    match AssertUnwindSafe(lifecycle.drain()).catch_unwind().await {
        Ok(value) => value,
        Err(panic) => {
            log::error!("shutdown drain panicked: {}", panic_message(panic.as_ref()));
            false
        }
    }
}

fn spawn_handler(
    id: u64,
    params: RequestParams,
    out_tx: mpsc::Sender<Vec<u8>>,
    inflight: Arc<Semaphore>,
    started_at: Instant,
) {
    tokio::spawn(async move {
        let _permit = if params.bypasses_semaphore() {
            None
        } else {
            match inflight.acquire_owned().await {
                Ok(permit) => Some(permit),
                Err(error) => {
                    send_handler_response(
                        &out_tx,
                        id,
                        Err(ServiceError::Internal(error.to_string())),
                    )
                    .await;
                    return;
                }
            }
        };
        let result = dispatch_with_panic_safety(params, started_at).await;
        send_handler_response(&out_tx, id, result).await;
    });
}

async fn dispatch_with_panic_safety(
    params: RequestParams,
    started_at: Instant,
) -> Result<Value, ServiceError> {
    let method = params.method_name();
    let result = AssertUnwindSafe(handlers::dispatch(params, started_at))
        .catch_unwind()
        .await;
    match result {
        Ok(result) => result,
        Err(panic) => Err(ServiceError::Panic {
            method: method.to_string(),
            message: panic_message(panic.as_ref()),
        }),
    }
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = panic.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = panic.downcast_ref::<String>() {
        return message.clone();
    }
    "unknown panic payload".to_string()
}

async fn send_handler_response(
    out_tx: &mpsc::Sender<Vec<u8>>,
    id: u64,
    result: Result<Value, ServiceError>,
) {
    match result {
        Ok(value) => send_success(out_tx, id, value).await,
        Err(error) => send_error(out_tx, Some(id), JsonRpcErrorObject::from(error)).await,
    }
}

async fn send_success(out_tx: &mpsc::Sender<Vec<u8>>, id: u64, value: Value) {
    let response = JsonRpcSuccessResponse::new(id, value);
    match encode_message(&response) {
        Ok(bytes) => {
            let _ = out_tx.send(bytes).await;
        }
        Err(error) => {
            log::error!("failed to encode success response: {error}");
        }
    }
}

async fn send_error(
    out_tx: &mpsc::Sender<Vec<u8>>,
    id: Option<u64>,
    error: JsonRpcErrorObject,
) {
    let response = JsonRpcErrorResponse::new(id, error);
    match encode_message(&response) {
        Ok(bytes) => {
            let _ = out_tx.send(bytes).await;
        }
        Err(error) => {
            log::error!("failed to encode error response: {error}");
        }
    }
}

async fn writer_task<W>(mut writer: W, mut out_rx: mpsc::Receiver<Vec<u8>>)
where
    W: AsyncWrite + Unpin,
{
    while let Some(bytes) = out_rx.recv().await {
        if let Err(error) = writer.write_all(&bytes).await {
            log::warn!("service stdout write failed: {error}");
            break;
        }
        if let Err(error) = writer.flush().await {
            log::warn!("service stdout flush failed: {error}");
            break;
        }
    }
}
