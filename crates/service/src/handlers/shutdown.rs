use serde_json::Value;
use service_api::{ServiceError, ShutdownResponse};

pub(super) async fn handle() -> Result<Value, ServiceError> {
    serde_json::to_value(ShutdownResponse { flushed_ok: true })
        .map_err(|error| ServiceError::Internal(error.to_string()))
}
