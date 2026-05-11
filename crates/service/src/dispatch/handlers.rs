//! Framing-layer helpers used by the dispatch loop and shutdown drain.
//!
//! Includes the writer task that owns the outbound socket end, the
//! per-request handler spawn (`spawn_handler` + its panic wrapper),
//! the notification handler spawn, and the JSON-RPC encoding helpers
//! (`send_success` / `send_error` / `try_send_error` /
//! `send_handler_response`). None of these own dispatch state; they
//! operate on the channels and join-sets passed in.

use crate::boot;
use crate::handlers;
use futures_util::FutureExt;
use serde_json::Value;
use service_api::{
    JsonRpcErrorObject, JsonRpcErrorResponse, JsonRpcSuccessResponse, RequestParams, ServiceError,
    encode_message,
};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinSet;

/// Writer task. Owns the outbound half of the Service stdio and
/// flushes each encoded frame as it arrives on the mpsc. Exits on the
/// first I/O error so a closed pipe doesn't loop forever; the shutdown
/// drain awaits this task's JoinHandle after every `out_tx` clone has
/// been released.
pub(crate) async fn writer_task<W>(mut writer: W, mut out_rx: mpsc::Receiver<Vec<u8>>)
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

/// Spawn a request handler. Acquires the in-flight permit *inside* the
/// spawned task, runs the typed handler under `catch_unwind`, then
/// sends the encoded response. Heartbeat-class requests
/// (`bypasses_admission()`) skip the permit acquire.
pub(crate) fn spawn_handler(
    id: u64,
    params: RequestParams,
    out_tx: mpsc::Sender<Vec<u8>>,
    inflight: Arc<Semaphore>,
    started_at: Instant,
    handlers_in_flight: &mut JoinSet<()>,
    boot_state: Arc<boot::BootSharedState>,
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
        let _permit = if params.bypasses_admission() {
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
        let result = dispatch_with_panic_safety(params, started_at, boot_state).await;
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

/// Dispatch a UI -> Service notification on the dedicated notification
/// task pool. No response is sent (notifications are id-less by
/// definition); the handler runs to completion or is dropped on shutdown.
pub(crate) fn spawn_notification_handler(
    notification: service_api::ClientNotification,
    notifications_in_flight: &mut JoinSet<()>,
    boot_state: Arc<boot::BootSharedState>,
) {
    let method = notification.method_name();
    log::info!("notification dispatch method={method}");
    notifications_in_flight.spawn(async move {
        let entered = Instant::now();
        let result = AssertUnwindSafe(handlers::dispatch_notification(notification, boot_state))
            .catch_unwind()
            .await;
        let elapsed_ms = entered.elapsed().as_millis();
        match result {
            Ok(Ok(())) => {
                log::info!(
                    "notification dispatch end method={method} elapsed_ms={elapsed_ms} outcome=ok",
                );
            }
            Ok(Err(error)) => {
                log::warn!(
                    "notification dispatch end method={method} elapsed_ms={elapsed_ms} outcome=err: {error}",
                );
            }
            Err(panic) => {
                log::error!(
                    "notification handler panicked method={method}: {}",
                    panic_message(panic.as_ref()),
                );
            }
        }
    });
}

/// Discriminant-only outcome string for dispatch-end logging. Returns
/// the variant name with no payload content - the error message itself
/// is held back from the rolling log file by sensitive-value policy.
fn error_outcome_kind(error: &ServiceError) -> &'static str {
    match error {
        ServiceError::Panic { .. } => "panic",
        ServiceError::InvalidParams { .. } => "invalid_params",
        ServiceError::UnknownMethod(_) => "unknown_method",
        ServiceError::Internal(_) => "internal",
        ServiceError::Backpressure => "backpressure",
        ServiceError::BootFailure { .. } => "boot_failure",
    }
}

async fn dispatch_with_panic_safety(
    params: RequestParams,
    started_at: Instant,
    boot_state: Arc<boot::BootSharedState>,
) -> Result<Value, ServiceError> {
    let method = params.method_name();
    let result = AssertUnwindSafe(handlers::dispatch(params, started_at, boot_state))
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

pub(crate) fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = panic.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = panic.downcast_ref::<String>() {
        return message.clone();
    }
    "unknown panic payload".to_string()
}

pub(crate) async fn send_handler_response(
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

async fn send_error(out_tx: &mpsc::Sender<Vec<u8>>, id: Option<u64>, error: JsonRpcErrorObject) {
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

/// Non-blocking error send used from the dispatch loop. Awaiting
/// `out_tx.send` here would stall stdin reads when the outbound queue
/// is full; for parse errors and frame errors that's the wrong trade
/// - drop the diagnostic and keep reading.
pub(crate) fn try_send_error(
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
