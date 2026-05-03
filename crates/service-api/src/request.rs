use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

use crate::action::ActionWirePlan;

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
    /// Submit a resolved-and-planned action for execution. The Service
    /// handler validates the plan, journals it into `action_jobs` +
    /// `action_job_ops` (per Phase 2 plan scope item 18a), signals the
    /// worker pool, and returns `ActionPlanAck { plan_id, journaled }`.
    /// Per-operation `OperationOutcome` notifications stream from the
    /// worker; `ActionCompleted` closes the stream.
    ///
    /// The 5 s timeout is the **handler** budget (validate + insert
    /// rows + signal `tokio::sync::Notify`). The worker has no IPC
    /// timeout - it runs to completion or until respawn.
    ActionExecutePlan { plan: ActionWirePlan },
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
            Self::ActionExecutePlan { .. } => "action.execute_plan",
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
        // `Shutdown` does NOT set `bypasses_admission()`, but the dispatch
        // loop intercepts it in `handle_line` before reaching the
        // admission check, so the per-handler semaphore and the dispatch-
        // loop admission cap are both effectively bypassed for Shutdown
        // by virtue of dispatch-loop interception. The 30 s timeout below
        // is the budget for the in-flight drain to complete before the
        // UI escalates to SIGTERM.
        match self {
            Self::HealthPing => RequestTimeoutKind::Finite(Duration::from_secs(5)),
            Self::Shutdown => RequestTimeoutKind::Finite(Duration::from_secs(30)),
            Self::BootReady => RequestTimeoutKind::Finite(Duration::from_secs(600)),
            // Handler-only budget: validate + journal + signal worker.
            // The worker has no IPC timeout (per Phase 2 plan scope
            // item 3, which split execution off the request future
            // because the dispatch loop sends the response only after
            // the handler returns).
            Self::ActionExecutePlan { .. } => RequestTimeoutKind::Finite(Duration::from_secs(5)),
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
            Self::ActionExecutePlan { plan } => serde_json::json!({ "plan": plan }),
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
            "action.execute_plan" => {
                #[derive(Deserialize)]
                struct P {
                    plan: ActionWirePlan,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("action.execute_plan params: {e}"))?;
                Ok(Self::ActionExecutePlan { plan: p.plan })
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
    fn action_execute_plan_timeout_is_five_seconds() {
        let plan = ActionWirePlan {
            plan_id: crate::action::PlanId::new_v7(),
            operations: Vec::new(),
        };
        assert_eq!(
            RequestParams::ActionExecutePlan { plan }.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn action_execute_plan_method_name_is_dotted() {
        let plan = ActionWirePlan {
            plan_id: crate::action::PlanId::new_v7(),
            operations: Vec::new(),
        };
        assert_eq!(
            RequestParams::ActionExecutePlan { plan }.method_name(),
            "action.execute_plan",
        );
    }

    #[test]
    fn action_execute_plan_does_not_bypass_admission() {
        let plan = ActionWirePlan {
            plan_id: crate::action::PlanId::new_v7(),
            operations: Vec::new(),
        };
        assert!(
            !RequestParams::ActionExecutePlan { plan }.bypasses_admission(),
            "action.execute_plan is bounded handler work; admission cap applies",
        );
    }

    #[test]
    fn action_execute_plan_round_trips_from_method_params() {
        use crate::action::{
            ActionWireOperation, OperationId, PlanId, WireFolderId, WireMailOperation,
        };

        let plan = ActionWirePlan {
            plan_id: PlanId::new_v7(),
            operations: vec![
                ActionWireOperation {
                    operation_id: OperationId(0),
                    account_id: "acc-1".into(),
                    thread_id: "thr-9".into(),
                    operation: WireMailOperation::MoveToFolder {
                        dest: WireFolderId("inbox".into()),
                        source: Some(WireFolderId("archive".into())),
                    },
                },
            ],
        };
        let original = RequestParams::ActionExecutePlan { plan };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
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
