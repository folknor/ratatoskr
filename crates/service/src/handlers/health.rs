use crate::boot::BootSharedState;
use serde_json::Value;
use service_api::{HealthPingResponse, PROTOCOL_VERSION, ServiceError};
use std::sync::Arc;
use std::time::Instant;

pub(super) async fn handle(
    started_at: Instant,
    boot_state: &Arc<BootSharedState>,
) -> Result<Value, ServiceError> {
    let uptime_ms = started_at.elapsed().as_millis();
    let uptime_ms = u64::try_from(uptime_ms).unwrap_or(u64::MAX);
    serde_json::to_value(HealthPingResponse {
        version: boot_state
            .config()
            .fake_protocol_version
            .unwrap_or(PROTOCOL_VERSION),
        pid: std::process::id(),
        uptime_ms,
    })
    .map_err(|error| ServiceError::Internal(error.to_string()))
}
