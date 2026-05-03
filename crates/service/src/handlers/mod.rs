mod action_status;
mod boot;
mod health;
#[cfg(feature = "test-helpers")]
mod test_helpers;

use crate::boot::BootSharedState;
use serde_json::Value;
use service_api::{RequestParams, ServiceError};
use std::sync::Arc;
use std::time::Instant;

/// Dispatch a request to its handler.
///
/// `RequestParams::Shutdown` is intentionally not handled here - the dispatch
/// loop intercepts it directly so the drain + sentinel + ack ordering is
/// explicit at the lifecycle layer. Treat reaching this arm as a bug.
pub(crate) async fn dispatch(
    params: RequestParams,
    started_at: Instant,
    boot_state: Arc<BootSharedState>,
) -> Result<Value, ServiceError> {
    match params {
        RequestParams::HealthPing => health::handle(started_at).await,
        RequestParams::Shutdown => Err(ServiceError::Internal(
            "shutdown reached handler dispatch; lifecycle layer should have intercepted".into(),
        )),
        RequestParams::BootReady => boot::handle(&boot_state).await,
        // Stub until Phase 2 task 9 lands the handler+worker. Returning
        // `Internal` rather than `unreachable!()` so the dispatch path
        // stays exhaustive without panicking if a UI accidentally
        // sends `action.execute_plan` against a Service from before
        // the action service relocated. Once task 9 lands this arm
        // routes to `crate::handlers::action::handle_execute_plan`.
        RequestParams::ActionExecutePlan { .. } => Err(ServiceError::Internal(
            "action.execute_plan handler not yet implemented (Phase 2 task 9)".into(),
        )),
        RequestParams::ActionJobStatus { plan_id } => {
            action_status::handle(&boot_state, plan_id).await
        }
        #[cfg(feature = "test-helpers")]
        RequestParams::TestPanic => test_helpers::panic_handle().await,
        #[cfg(feature = "test-helpers")]
        RequestParams::TestVersion { version } => test_helpers::version_handle(version).await,
        #[cfg(feature = "test-helpers")]
        RequestParams::TestSlow { millis } => test_helpers::slow_handle(millis).await,
        #[cfg(feature = "test-helpers")]
        RequestParams::TestPrintln { message } => test_helpers::println_handle(message).await,
    }
}
