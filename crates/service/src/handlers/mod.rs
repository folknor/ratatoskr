mod action;
mod action_mark_chat_read;
mod action_send;
mod action_status;
mod boot;
mod health;
mod pending_ops_kick;
#[cfg(feature = "test-helpers")]
mod test_helpers;

pub(crate) use action_mark_chat_read::JournaledChatRead;
pub(crate) use action_send::JournaledSend;

use crate::boot::BootSharedState;
use serde_json::Value;
use service_api::{ClientNotification, RequestParams, ServiceError};
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
        RequestParams::ActionExecutePlan { plan } => action::handle(&boot_state, plan).await,
        RequestParams::ActionJobStatus { plan_id } => {
            action_status::handle(&boot_state, plan_id).await
        }
        RequestParams::ActionMarkChatRead { chat_email } => {
            action_mark_chat_read::handle(&boot_state, chat_email).await
        }
        RequestParams::ActionSend { request } => action_send::handle(&boot_state, *request).await,
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

/// Dispatch a UI -> Service notification (Phase 2 plan scope item 11).
///
/// No response is emitted - notifications are fire-and-forget by
/// construction. The handler returns `Result<(), String>` so it can
/// surface a diagnostic into the dispatch log even though the UI never
/// observes it directly.
pub(crate) async fn dispatch_notification(
    notification: ClientNotification,
    boot_state: Arc<BootSharedState>,
) -> Result<(), String> {
    match notification {
        ClientNotification::PendingOpsKick => pending_ops_kick::handle(&boot_state).await,
    }
}
