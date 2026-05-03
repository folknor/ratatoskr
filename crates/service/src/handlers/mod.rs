mod health;
#[cfg(feature = "test-helpers")]
mod test_helpers;

use serde_json::Value;
use service_api::{RequestParams, ServiceError};
use std::time::Instant;

/// Dispatch a request to its handler.
///
/// `RequestParams::Shutdown` is intentionally not handled here - the dispatch
/// loop intercepts it directly so the drain + sentinel + ack ordering is
/// explicit at the lifecycle layer. Treat reaching this arm as a bug.
pub(crate) async fn dispatch(
    params: RequestParams,
    started_at: Instant,
) -> Result<Value, ServiceError> {
    match params {
        RequestParams::HealthPing => health::handle(started_at).await,
        RequestParams::Shutdown => Err(ServiceError::Internal(
            "shutdown reached handler dispatch; lifecycle layer should have intercepted".into(),
        )),
        // Stub. The real handler lands with the boot sequence (it parks on a
        // Notify until migrations + key load + recovery complete). Phase 1.5
        // commit 1 only ships the wire types; no UI call site reaches this
        // until the two-phase spawn lands.
        RequestParams::BootReady => Err(ServiceError::Internal(
            "boot.ready handler not yet wired (Phase 1.5 commit 10)".into(),
        )),
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
