use crate::boot::BootExitCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
pub enum ServiceError {
    #[error("handler panic in {method}: {message}")]
    Panic { method: String, message: String },
    #[error("invalid params for {method}: {message}")]
    InvalidParams { method: String, message: String },
    #[error("unknown method: {0}")]
    UnknownMethod(String),
    #[error("internal error: {0}")]
    Internal(String),
    /// Service is at its in-flight handler cap. The client should retry after
    /// previously-in-flight requests complete. Returned synchronously by the
    /// dispatch loop without spawning a handler task - this is the bounded-
    /// admission backpressure signal, not a per-handler error.
    #[error("service at capacity (in-flight admission rejected)")]
    Backpressure,
    /// The Service's boot sequence failed. Carries the structured
    /// `BootExitCode` so the UI can surface a friendly per-code message
    /// (e.g. "Encryption key load failed" for `KeyLoadFailure`)
    /// without parsing a Display string. This is the wire shape returned by
    /// the `boot.ready` handler when the boot sequence ends in a fatal
    /// failure that gets to ack before the Service exits; if the Service
    /// exits before the response can be flushed, the UI instead inspects
    /// the dying child's exit code (which carries the same `BootExitCode`).
    #[error("boot sequence failed: {code:?}")]
    BootFailure { code: BootExitCode },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcErrorObject {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcErrorObject {
    pub fn parse_error(message: impl Into<String>) -> Self {
        Self {
            code: -32700,
            message: message.into(),
            data: None,
        }
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            code: -32600,
            message: message.into(),
            data: None,
        }
    }

    pub fn method_not_found(message: impl Into<String>) -> Self {
        Self {
            code: -32601,
            message: message.into(),
            data: None,
        }
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: message.into(),
            data: None,
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: -32603,
            message: message.into(),
            data: None,
        }
    }
}

impl From<ServiceError> for JsonRpcErrorObject {
    fn from(error: ServiceError) -> Self {
        let data = serde_json::to_value(&error).ok();
        let (code, message) = match &error {
            ServiceError::InvalidParams { message, .. } => (-32602, message.clone()),
            ServiceError::UnknownMethod(method) => (-32601, format!("unknown method: {method}")),
            ServiceError::Panic { method, message } => {
                (-32603, format!("handler panic in {method}: {message}"))
            }
            ServiceError::Internal(message) => (-32603, message.clone()),
            // Server-busy in JSON-RPC 2.0 land: -32000 is the start of the
            // implementation-defined server error range. Pick a value within
            // it that's distinguishable from generic Internal.
            ServiceError::Backpressure => (
                -32000,
                "service at capacity (in-flight admission rejected)".to_string(),
            ),
            // Pick a server-error code distinct from `Backpressure` so the
            // wire code alone discriminates these cases even if `data`
            // round-trip fails. `-32001` is also within the implementation-
            // defined server-error range.
            ServiceError::BootFailure { code } => (
                -32001,
                format!("boot sequence failed (exit code {})", code.as_i32()),
            ),
        };
        Self {
            code,
            message,
            data,
        }
    }
}

impl JsonRpcErrorObject {
    /// Recover the original `ServiceError` if it was embedded in `data` by the
    /// `From<ServiceError>` impl above. Returns `Err(self)` if the payload is
    /// missing or unrecognizable so the caller can fall back to message-only
    /// reporting.
    pub fn try_into_service_error(self) -> Result<ServiceError, Self> {
        match self.data.as_ref() {
            Some(value) => match serde_json::from_value::<ServiceError>(value.clone()) {
                Ok(error) => Ok(error),
                Err(_) => Err(self),
            },
            None => Err(self),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_error_round_trips_through_json_rpc_object() {
        let original = ServiceError::Panic {
            method: "health.ping".to_string(),
            message: "boom".to_string(),
        };
        let object = JsonRpcErrorObject::from(original);
        assert_eq!(object.code, -32603);
        assert!(object.data.is_some());
        let recovered = object
            .try_into_service_error()
            .expect("data carries the original variant");
        match recovered {
            ServiceError::Panic { method, message } => {
                assert_eq!(method, "health.ping");
                assert_eq!(message, "boom");
            }
            other => panic!("expected Panic, got {other:?}"),
        }
    }

    #[test]
    fn unknown_method_round_trips_through_json_rpc_object() {
        let original = ServiceError::UnknownMethod("bogus".to_string());
        let object = JsonRpcErrorObject::from(original);
        assert_eq!(object.code, -32601);
        let recovered = object
            .try_into_service_error()
            .expect("data carries the original variant");
        assert!(matches!(recovered, ServiceError::UnknownMethod(name) if name == "bogus"));
    }

    #[test]
    fn missing_data_falls_back_to_self() {
        let object = JsonRpcErrorObject::parse_error("no payload");
        let result = object.try_into_service_error();
        assert!(result.is_err());
    }

    /// `BootFailure` round-trips through the JSON-RPC error object so the UI
    /// recovers the structured `BootExitCode` from the response without
    /// parsing the Display string. Closes the headline initial-boot
    /// classification gap for the case where the Service does answer
    /// `boot.ready` before exiting.
    #[test]
    fn boot_failure_round_trips_through_json_rpc_object() {
        let original = ServiceError::BootFailure {
            code: BootExitCode::KeyLoadFailure,
        };
        let object = JsonRpcErrorObject::from(original);
        assert_eq!(object.code, -32001);
        let recovered = object
            .try_into_service_error()
            .expect("data carries the original variant");
        assert!(matches!(
            recovered,
            ServiceError::BootFailure {
                code: BootExitCode::KeyLoadFailure
            }
        ));
    }

    #[test]
    fn boot_failure_wire_code_is_distinct_from_backpressure() {
        let bp = JsonRpcErrorObject::from(ServiceError::Backpressure);
        let bf = JsonRpcErrorObject::from(ServiceError::BootFailure {
            code: BootExitCode::MigrationFailure,
        });
        assert_ne!(bp.code, bf.code);
    }
}
