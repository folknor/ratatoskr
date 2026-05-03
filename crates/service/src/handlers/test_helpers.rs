//! Handlers for the `test-helpers` feature. Compiled out of release builds.
//!
//! Each handler maps to a `RequestParams::Test*` variant defined in
//! `service-api` under the same feature flag. They exist exclusively to give
//! integration tests a deterministic way to drive panic-safety, version-
//! mismatch, in-flight-cap, and stdio-corruption behaviors.

use serde_json::Value;
use service_api::{HealthPingResponse, ServiceError};
use std::time::Duration;

pub(super) async fn panic_handle() -> Result<Value, ServiceError> {
    panic!("test-helpers: TestPanic handler intentional panic");
}

pub(super) async fn version_handle(version: u32) -> Result<Value, ServiceError> {
    serde_json::to_value(HealthPingResponse {
        version,
        pid: std::process::id(),
        uptime_ms: 0,
    })
    .map_err(|error| ServiceError::Internal(error.to_string()))
}

pub(super) async fn slow_handle(millis: u64) -> Result<Value, ServiceError> {
    tokio::time::sleep(Duration::from_millis(millis)).await;
    Ok(Value::Null)
}

pub(super) async fn println_handle(message: String) -> Result<Value, ServiceError> {
    // Goes through the global stdout HANDLE; with the stdio-defense in place
    // this lands in /dev/null (unix) or NUL (windows) instead of corrupting
    // the JSON-RPC framing. The test asserts that the response on the
    // saved-FD stdout is still well-formed.
    println!("{message}");
    Ok(Value::Null)
}
