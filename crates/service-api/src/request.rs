use serde::{Deserialize, Serialize};
use serde_json::Value;
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
    /// Sent by the UI after the version-check ping; the Service answers it
    /// only after migrations + key load + pending-ops recovery + queued-
    /// drafts sweep + thread-participants backfill have all completed. The
    /// long timeout (10 minutes) covers a 50 GB-class schema migration.
    BootReady,
    /// Always panics in the handler. Used to verify dispatch panic safety.
    #[cfg(feature = "test-helpers")]
    TestPanic,
    /// Returns a `HealthPingResponse` with the requested protocol version.
    /// Used to drive `ClientError::VersionMismatch` from the handshake.
    #[cfg(feature = "test-helpers")]
    TestVersion { version: u32 },
    /// Sleeps for `millis` before responding. Used to verify the in-flight
    /// semaphore cap and the heartbeat-bypasses-semaphore property.
    #[cfg(feature = "test-helpers")]
    TestSlow { millis: u64 },
    /// Calls `println!` (or its global-stdout-handle equivalent on Windows)
    /// before responding. Used to verify the stdio corruption defense.
    #[cfg(feature = "test-helpers")]
    TestPrintln { message: String },
}

impl RequestParams {
    pub fn method_name(&self) -> &'static str {
        match self {
            Self::HealthPing => "health.ping",
            Self::Shutdown => "shutdown",
            Self::BootReady => "boot.ready",
            #[cfg(feature = "test-helpers")]
            Self::TestPanic => "test.panic",
            #[cfg(feature = "test-helpers")]
            Self::TestVersion { .. } => "test.version",
            #[cfg(feature = "test-helpers")]
            Self::TestSlow { .. } => "test.slow",
            #[cfg(feature = "test-helpers")]
            Self::TestPrintln { .. } => "test.println",
        }
    }

    pub fn timeout(&self) -> RequestTimeoutKind {
        match self {
            Self::HealthPing => RequestTimeoutKind::Finite(Duration::from_secs(5)),
            Self::Shutdown => RequestTimeoutKind::Finite(Duration::from_secs(30)),
            Self::BootReady => RequestTimeoutKind::Finite(Duration::from_secs(600)),
            #[cfg(feature = "test-helpers")]
            Self::TestPanic | Self::TestVersion { .. } | Self::TestPrintln { .. } => {
                RequestTimeoutKind::Finite(Duration::from_secs(5))
            }
            #[cfg(feature = "test-helpers")]
            Self::TestSlow { .. } => RequestTimeoutKind::Finite(Duration::from_secs(60)),
        }
    }

    /// Requests that bypass BOTH the per-handler semaphore and the dispatch-
    /// loop admission cap.
    ///
    /// `health.ping` keeps the heartbeat alive under load; `boot.ready` is
    /// special-cased because it parks on a `Notify` until the boot sequence
    /// completes (occupying a semaphore permit while parked would let a long
    /// migration starve other handlers) and because flooding the dispatch
    /// loop with slow requests would otherwise be able to push the boot
    /// handshake out past the admission cap.
    ///
    /// Renamed from `bypasses_semaphore` in Phase 1.5 to reflect the dual
    /// role - the dispatch loop's `ADMISSION_CAP` gate also keys off this
    /// flag.
    pub fn bypasses_admission(&self) -> bool {
        matches!(self, Self::HealthPing | Self::BootReady)
    }

    /// Serialize this request's params into the `params` field of the
    /// JSON-RPC envelope.
    ///
    /// Unit variants serialize to `Value::Null` (the wire-canonical "no
    /// params"). Tuple-shaped variants serialize their inner struct via
    /// `serde_json::to_value`. Each match arm is the canonical extension
    /// point.
    pub fn params_value(&self) -> Value {
        match self {
            Self::HealthPing => Value::Null,
            Self::Shutdown => Value::Null,
            Self::BootReady => Value::Null,
            #[cfg(feature = "test-helpers")]
            Self::TestPanic => Value::Null,
            #[cfg(feature = "test-helpers")]
            Self::TestVersion { version } => serde_json::json!({ "version": version }),
            #[cfg(feature = "test-helpers")]
            Self::TestSlow { millis } => serde_json::json!({ "millis": millis }),
            #[cfg(feature = "test-helpers")]
            Self::TestPrintln { message } => serde_json::json!({ "message": message }),
        }
    }

    pub fn from_method_params(method: &str, params: Option<Value>) -> Result<Self, String> {
        match method {
            "health.ping" => {
                expect_no_params(method, params)?;
                Ok(Self::HealthPing)
            }
            "shutdown" => {
                expect_no_params(method, params)?;
                Ok(Self::Shutdown)
            }
            "boot.ready" => {
                expect_no_params(method, params)?;
                Ok(Self::BootReady)
            }
            #[cfg(feature = "test-helpers")]
            "test.panic" => {
                expect_no_params(method, params)?;
                Ok(Self::TestPanic)
            }
            #[cfg(feature = "test-helpers")]
            "test.version" => {
                #[derive(Deserialize)]
                struct P {
                    version: u32,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("test.version params: {e}"))?;
                Ok(Self::TestVersion { version: p.version })
            }
            #[cfg(feature = "test-helpers")]
            "test.slow" => {
                #[derive(Deserialize)]
                struct P {
                    millis: u64,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("test.slow params: {e}"))?;
                Ok(Self::TestSlow { millis: p.millis })
            }
            #[cfg(feature = "test-helpers")]
            "test.println" => {
                #[derive(Deserialize)]
                struct P {
                    message: String,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("test.println params: {e}"))?;
                Ok(Self::TestPrintln { message: p.message })
            }
            _ => Err(format!("unknown method: {method}")),
        }
    }
}

/// For unit variants that take no params. Future struct-shaped variants
/// should `serde_json::from_value::<TheirParams>(params.unwrap_or(Null))`
/// instead.
fn expect_no_params(method: &str, params: Option<Value>) -> Result<(), String> {
    match params {
        None => Ok(()),
        Some(Value::Object(map)) if map.is_empty() => Ok(()),
        Some(Value::Null) => Ok(()),
        Some(_) => Err(format!("{method} expects no params")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_ready_timeout_is_ten_minutes() {
        assert_eq!(
            RequestParams::BootReady.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(600)),
        );
    }

    #[test]
    fn boot_ready_method_name_is_dotted() {
        assert_eq!(RequestParams::BootReady.method_name(), "boot.ready");
    }

    #[test]
    fn boot_ready_bypasses_admission() {
        assert!(RequestParams::BootReady.bypasses_admission());
    }

    #[test]
    fn health_ping_bypasses_admission() {
        assert!(RequestParams::HealthPing.bypasses_admission());
    }

    #[test]
    fn shutdown_does_not_bypass_admission() {
        assert!(!RequestParams::Shutdown.bypasses_admission());
    }

    #[test]
    fn boot_ready_round_trips_from_method_params() {
        let parsed = RequestParams::from_method_params("boot.ready", None).expect("parse");
        assert_eq!(parsed, RequestParams::BootReady);
        let parsed_null =
            RequestParams::from_method_params("boot.ready", Some(Value::Null)).expect("parse");
        assert_eq!(parsed_null, RequestParams::BootReady);
        assert!(
            RequestParams::from_method_params("boot.ready", Some(serde_json::json!({"x": 1})))
                .is_err()
        );
    }
}
