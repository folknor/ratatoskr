mod health;
mod shutdown;

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
    }
}
