use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestTimeoutKind {
    Finite(Duration),
    Infinite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestParams {
    HealthPing,
    Shutdown,
}

impl RequestParams {
    pub fn method_name(&self) -> &'static str {
        match self {
            Self::HealthPing => "health.ping",
            Self::Shutdown => "shutdown",
        }
    }

    pub fn timeout(&self) -> RequestTimeoutKind {
        match self {
            Self::HealthPing => RequestTimeoutKind::Finite(Duration::from_secs(5)),
            Self::Shutdown => RequestTimeoutKind::Finite(Duration::from_secs(30)),
        }
    }

    pub fn bypasses_semaphore(&self) -> bool {
        matches!(self, Self::HealthPing)
    }

    pub fn params_value(&self) -> Value {
        match self {
            Self::HealthPing | Self::Shutdown => json!({}),
        }
    }

    pub fn from_method_params(method: &str, params: Option<Value>) -> Result<Self, String> {
        match method {
            "health.ping" => {
                validate_empty_params(method, params)?;
                Ok(Self::HealthPing)
            }
            "shutdown" => {
                validate_empty_params(method, params)?;
                Ok(Self::Shutdown)
            }
            _ => Err(format!("unknown method: {method}")),
        }
    }
}

fn validate_empty_params(method: &str, params: Option<Value>) -> Result<(), String> {
    match params {
        None => Ok(()),
        Some(Value::Object(map)) if map.is_empty() => Ok(()),
        Some(Value::Null) => Ok(()),
        Some(_) => Err(format!("{method} expects empty params")),
    }
}
