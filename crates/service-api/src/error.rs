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
    #[error("another instance is already running")]
    AnotherInstanceRunning,
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
        match error {
            ServiceError::InvalidParams { message, .. } => Self::invalid_params(message),
            ServiceError::UnknownMethod(method) => {
                Self::method_not_found(format!("unknown method: {method}"))
            }
            ServiceError::Panic { message, .. } | ServiceError::Internal(message) => {
                Self::internal(message)
            }
            ServiceError::AnotherInstanceRunning => Self::internal("another instance is running"),
        }
    }
}
