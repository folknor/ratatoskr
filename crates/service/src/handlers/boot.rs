//! `boot.ready` handler.
//!
//! Awaits the Service boot sequence's completion (key load, DB open +
//! migrations, pending-ops recovery, queued-drafts sweep, thread-
//! participants backfill) and returns a `BootReadyResponse` to the UI. On
//! boot failure, returns a `ServiceError::Internal` describing the
//! classification - though the UI usually observes failure via the dying
//! child's exit code rather than a `boot.ready` reply, since the dispatch
//! loop breaks out and the writer task closes shortly after the failure
//! signal propagates.
//!
//! The handler bypasses both the per-handler semaphore and the dispatch-
//! loop admission cap (via `RequestParams::bypasses_admission()`), because
//! it parks on a `Notify` for what may be tens of seconds during a long
//! migration; holding a semaphore permit while parked would let a long
//! migration starve other handlers.

use crate::boot::BootSharedState;
use serde_json::Value;
use service_api::ServiceError;
use std::sync::Arc;

pub(super) async fn handle(state: &Arc<BootSharedState>) -> Result<Value, ServiceError> {
    let response = state.wait_for_ready().await.map_err(|failure| {
        ServiceError::Internal(format!(
            "boot sequence failed: {failure:?} (exit code {})",
            failure.as_exit_code().as_i32()
        ))
    })?;
    serde_json::to_value(&response).map_err(|error| ServiceError::Internal(error.to_string()))
}
