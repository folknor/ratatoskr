//! `boot.ready` handler.
//!
//! Awaits the Service boot sequence's completion (key load, DB open +
//! migrations, pending-ops recovery, queued-drafts sweep, thread-
//! participants backfill) and returns a `BootReadyResponse` to the UI. On
//! boot failure, returns a structured `ServiceError::BootFailure { code }`
//! so the UI can recover the same `BootExitCode` discriminator it would
//! observe from the dying child's exit code. Whichever path the failure
//! takes - response-flushed-before-exit or exit-before-response - the UI
//! reaches the same classified terminal-failure surface.
//!
//! The handler bypasses both the per-handler semaphore and the dispatch-
//! loop admission cap (via `RequestParams::bypasses_admission()`), because
//! it parks on a `Notify` for what may be tens of seconds during a long
//! migration; holding a semaphore permit while parked would let a long
//! migration starve other handlers.
//!
//! Phase 1.5 carry-forward 19h / Phase 2 task 22: at most one parallel
//! parked handler. A second `boot.ready` arriving while the first is
//! still parked fails fast with `ServiceError::Backpressure` rather
//! than queueing a new `JoinSet` slot. If the result is already
//! cached (boot completed but the original ack was lost), the
//! second caller satisfies the request from the cache instead.

use crate::boot::BootSharedState;
use serde_json::Value;
use service_api::ServiceError;
use std::sync::Arc;

pub(super) async fn handle(state: &Arc<BootSharedState>) -> Result<Value, ServiceError> {
    // First check the cache - if boot already finished, satisfy the
    // request without taking the in-flight slot. This handles the
    // "ack got lost on the wire, UI retries" case which is otherwise
    // an obscure failure path.
    if let Some(result) = state.cached_result() {
        return result_to_value(result);
    }
    // Cache miss: try to claim the in-flight slot. If another handler
    // already parked, fail fast - otherwise we'd balloon the
    // JoinSet linearly under a UI bug or future surface that
    // re-issues boot.ready.
    let _guard = match state.try_claim_boot_ready_slot() {
        Some(guard) => guard,
        None => {
            log::warn!("boot.ready: another handler already parked; failing fast");
            return Err(ServiceError::Backpressure);
        }
    };
    let response = state
        .wait_for_ready()
        .await
        .map_err(|failure| ServiceError::BootFailure {
            code: failure.as_exit_code(),
        })?;
    serde_json::to_value(&response).map_err(|error| ServiceError::Internal(error.to_string()))
}

fn result_to_value(
    result: Result<service_api::BootReadyResponse, crate::boot::BootFailure>,
) -> Result<Value, ServiceError> {
    let response = result.map_err(|failure| ServiceError::BootFailure {
        code: failure.as_exit_code(),
    })?;
    serde_json::to_value(&response).map_err(|error| ServiceError::Internal(error.to_string()))
}
