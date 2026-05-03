use serde_json::Value;
use service_api::{HealthPingResponse, PROTOCOL_VERSION, ServiceError};
use std::time::Instant;

pub(super) async fn handle(started_at: Instant) -> Result<Value, ServiceError> {
    let uptime_ms = started_at.elapsed().as_millis();
    let uptime_ms = u64::try_from(uptime_ms).unwrap_or(u64::MAX);
    serde_json::to_value(HealthPingResponse {
        version: PROTOCOL_VERSION,
        pid: std::process::id(),
        uptime_ms,
    })
    .map_err(|error| ServiceError::Internal(error.to_string()))
}
