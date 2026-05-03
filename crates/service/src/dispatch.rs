use crate::handlers;
use crate::lifecycle::ServiceLifecycle;
use futures_util::FutureExt;
use serde_json::Value;
use service_api::{
    BoundedLineReader, FrameError, JsonRpcErrorObject, JsonRpcErrorResponse,
    JsonRpcSuccessResponse, ParsedClientMessage, RequestParams, ServiceError, ShutdownResponse,
    encode_message, parse_client_message,
};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinSet;

const OUTBOUND_QUEUE_CAP: usize = 1024;
const MAX_IN_FLIGHT: usize = 64;
/// Hard cap on tasks the dispatch loop has spawned but not yet reaped. Sized
/// at 2x `MAX_IN_FLIGHT`: one set actively executing (holding semaphore
/// permits), one set waiting briefly for a permit to free up. Beyond this
/// the request is rejected with `ServiceError::Backpressure` synchronously,
/// so a pathological client cannot balloon Service memory by flooding stdin.
const ADMISSION_CAP: usize = 2 * MAX_IN_FLIGHT;

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
    // Track every spawned handler task so a Shutdown request can drain them
    // before we ack. Without this, an in-flight Phase 2+ mutation could still
    // be running when the UI sees `flushed_ok: true` and starts terminating.
    let mut handlers_in_flight: JoinSet<()> = JoinSet::new();
    let mut lines = BoundedLineReader::new(reader, service_api::MAX_FRAME_BYTES);
    let mut pending_shutdown_id: Option<u64> = None;

    loop {
        // Reap any tasks that have completed since the last iteration so
        // `handlers_in_flight.len()` reflects truly-still-running handlers
        // when we use it as the admission gate below.
        reap_finished(&mut handlers_in_flight);

        tokio::select! {
            () = lifecycle.notified() => {
                break;
            }
            line = lines.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        match handle_line(
                            &line,
                            &out_tx,
                            &inflight,
                            &mut handlers_in_flight,
                            started_at,
                        ).await {
                            HandleOutcome::Continue => {}
                            HandleOutcome::Shutdown(id) => {
                                pending_shutdown_id = Some(id);
                                break;
                            }
                        }
                    }
                    Ok(None) => {
                        log::info!("service stdin closed");
                        break;
                    }
                    Err(FrameError::TooLarge) => {
                        log::warn!("rejecting oversized frame");
                        try_send_error(
                            &out_tx,
                            None,
                            JsonRpcErrorObject::parse_error("frame too large"),
                        );
                    }
                    Err(FrameError::InvalidUtf8(error)) => {
                        log::warn!("service frame had invalid utf-8: {error}");
                        try_send_error(
                            &out_tx,
                            None,
                            JsonRpcErrorObject::parse_error("invalid utf-8"),
                        );
                    }
                    Err(FrameError::Io(error)) => {
                        log::warn!("service frame io error: {error}");
                        break;
                    }
                }
            }
        }
    }

    // Drain in-flight handlers BEFORE running the lifecycle drain. This
    // ensures any Phase 2+ mutation actually finishes before we write the
    // sentinel and ack the Shutdown request. The dispatch loop has already
    // stopped reading new requests by the time we reach this point.
    drain_in_flight(&mut handlers_in_flight).await;

    let flushed_ok = panic_safe_drain(&lifecycle).await;
    if !flushed_ok {
        log::warn!("shutdown drain completed with errors");
    }

    // If the loop exited because of a Shutdown request, ack only after the
    // drain above completes - `flushed_ok: true` means the sentinel was
    // written and every in-flight handler has returned.
    if let Some(id) = pending_shutdown_id {
        // Framing-layer logging hook: same shape as spawn_handler. Records
        // the outcome (ok / internal) and elapsed-since-shutdown-arrival;
        // never the response payload.
        let outcome = if flushed_ok { "ok" } else { "internal" };
        log::info!("dispatch end method=shutdown id={id} outcome={outcome}");
        let result = serde_json::to_value(ShutdownResponse { flushed_ok })
            .map_err(|error| ServiceError::Internal(error.to_string()));
        send_handler_response(&out_tx, id, result).await;
        lifecycle.request_shutdown();
    }

    drop(out_tx);
    let _ = writer_handle.await;
    0
}

enum HandleOutcome {
    Continue,
    /// Shutdown request received. Caller should break the dispatch loop and
    /// ack with the supplied id after the in-flight drain completes.
    Shutdown(u64),
}

async fn handle_line(
    line: &str,
    out_tx: &mpsc::Sender<Vec<u8>>,
    inflight: &Arc<Semaphore>,
    handlers_in_flight: &mut JoinSet<()>,
    started_at: Instant,
) -> HandleOutcome {
    match parse_client_message(line) {
        Ok(ParsedClientMessage::Request {
            id,
            params: RequestParams::Shutdown,
        }) => {
            log::info!("dispatch start method=shutdown id={id}");
            HandleOutcome::Shutdown(id)
        }
        Ok(ParsedClientMessage::Request { id, params }) => {
            // Bypass the admission gate for heartbeat-class requests so a
            // flood of slow handlers can't starve the UI's health check.
            // Non-bypass requests must fit under ADMISSION_CAP - beyond that
            // we synchronously reject with Backpressure rather than spawning
            // unbounded waiters.
            if !params.bypasses_semaphore() && handlers_in_flight.len() >= ADMISSION_CAP {
                send_handler_response(out_tx, id, Err(ServiceError::Backpressure)).await;
                return HandleOutcome::Continue;
            }
            spawn_handler(
                id,
                params,
                out_tx.clone(),
                Arc::clone(inflight),
                started_at,
                handlers_in_flight,
            );
            HandleOutcome::Continue
        }
        Err(error) => {
            let response_id = error.extracted_id();
            log::warn!("request parse failed: {error}");
            try_send_error(
                out_tx,
                response_id,
                JsonRpcErrorObject::parse_error(error.to_string()),
            );
            HandleOutcome::Continue
        }
    }
}

/// Reap completed tasks without blocking. Called between dispatch-loop
/// iterations so the JoinSet's `len()` is an honest count of still-running
/// handlers when the admission gate consults it.
fn reap_finished(handlers: &mut JoinSet<()>) {
    while let Some(result) = handlers.try_join_next() {
        if let Err(error) = result {
            log::warn!("in-flight handler join error: {error}");
        }
    }
}

async fn drain_in_flight(handlers: &mut JoinSet<()>) {
    while let Some(result) = handlers.join_next().await {
        if let Err(error) = result {
            // A handler task panic surfaced as a JoinError. The per-handler
            // catch_unwind already converted handler panics into
            // ServiceError::Panic, so reaching here implies the wrapper
            // itself panicked or the task was cancelled. Log and continue
            // so a single bad handler can't hold up shutdown.
            log::warn!("in-flight handler join failed during drain: {error}");
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
    handlers_in_flight: &mut JoinSet<()>,
) {
    let method = params.method_name();
    // Framing-layer logging hook: record method + id only at dispatch entry.
    // Never the params payload - once Phase 2+ methods carry user content
    // (message bodies, search queries, OAuth codes) in params, payload-level
    // logging would silently bypass the RedactedString net.
    log::info!("dispatch start method={method} id={id}");
    handlers_in_flight.spawn(async move {
        let entered = Instant::now();
        // Acquire the in-flight permit *inside* the spawned task - never
        // in the dispatch loop. Acquiring upfront would stall the dispatch
        // loop's stdin read whenever MAX_IN_FLIGHT slow handlers are
        // running, queuing fast methods behind slow ones. The dispatch
        // loop's ADMISSION_CAP gate keeps the number of waiters bounded.
        let _permit = if params.bypasses_semaphore() {
            None
        } else {
            match inflight.acquire_owned().await {
                Ok(permit) => Some(permit),
                Err(error) => {
                    log::warn!(
                        "dispatch end method={method} id={id} elapsed_ms={} outcome=internal",
                        entered.elapsed().as_millis(),
                    );
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
        // Framing-layer logging hook: record outcome discriminant only.
        // The error variant name lands; the error message does not, since
        // ServiceError::Panic and ServiceError::Internal can carry caller-
        // -supplied content.
        let elapsed_ms = entered.elapsed().as_millis();
        let outcome = match &result {
            Ok(_) => "ok",
            Err(error) => error_outcome_kind(error),
        };
        log::info!("dispatch end method={method} id={id} elapsed_ms={elapsed_ms} outcome={outcome}");
        send_handler_response(&out_tx, id, result).await;
    });
}

/// Discriminant-only outcome string for dispatch-end logging. Returns the
/// variant name with no payload content - the error message itself is held
/// back from the rolling log file by sensitive-value policy.
fn error_outcome_kind(error: &ServiceError) -> &'static str {
    match error {
        ServiceError::Panic { .. } => "panic",
        ServiceError::InvalidParams { .. } => "invalid_params",
        ServiceError::UnknownMethod(_) => "unknown_method",
        ServiceError::Internal(_) => "internal",
        ServiceError::Backpressure => "backpressure",
    }
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

/// Non-blocking error send used from the dispatch loop. Awaiting `out_tx.send`
/// here would stall stdin reads when the outbound queue is full; for parse
/// errors and frame errors that's the wrong trade - drop the diagnostic and
/// keep reading.
fn try_send_error(
    out_tx: &mpsc::Sender<Vec<u8>>,
    id: Option<u64>,
    error: JsonRpcErrorObject,
) {
    let response = JsonRpcErrorResponse::new(id, error);
    match encode_message(&response) {
        Ok(bytes) => {
            if let Err(send_err) = out_tx.try_send(bytes) {
                log::warn!("dropped diagnostic error response: {send_err}");
            }
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
