mod health;
mod shutdown;
#[cfg(feature = "test-helpers")]
mod test_helpers;

use serde_json::Value;
use service_api::{RequestParams, ServiceError};
use std::time::Instant;

pub(crate) async fn dispatch(
    params: RequestParams,
    started_at: Instant,
) -> Result<Value, ServiceError> {
    match params {
        RequestParams::HealthPing => health::handle(started_at).await,
        RequestParams::Shutdown => shutdown::handle().await,
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
