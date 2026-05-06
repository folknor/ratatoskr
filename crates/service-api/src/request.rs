use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

use crate::action::{ActionWirePlan, PlanId, SendWireRequest};
use crate::calendar::{
    CalendarCancelAccountSyncParams, CalendarSetVisibilityParams, CalendarStartAccountSyncParams,
};
use crate::thread_ui_state::ThreadUiStateSetParams;
use crate::sync::{SyncCancelAccountParams, SyncStartAccountParams};

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
    /// Look up the journaled status of a previously-submitted plan.
    /// Used by the UI's `AckUnknown` reconciliation path (Phase 2 plan
    /// scope item 11 / 18d): after a `boot.ready` post-respawn, the UI
    /// calls this for every plan whose ack was lost on the wire to
    /// resolve to either `Acked` (Journaled) or `RollBack` (NotFound).
    ///
    /// Read-only SELECT against the journal; the 5 s timeout is
    /// conservative. Doesn't bypass admission - it's just a fast query.
    ActionJobStatus { plan_id: PlanId },
    /// Phase 2 plan scope item 18c: the chat read-on-view side effect
    /// relocates as a quiet journal job. Handler resolves affected
    /// threads, runs the local DB write, journals the affected list
    /// for deterministic replay, returns `MarkChatReadAck`. Worker
    /// dispatches provider mark-read against each thread.
    ActionMarkChatRead { chat_email: String },
    /// Phase 2 plan scope item 5: compose-send relocates as a quiet
    /// journal job. Handler validates the request, transfers each
    /// attachment from `<app_data>/staging/<send_id>/` into a
    /// Service-owned vault under `<app_data>/send_vault/<send_id>/`
    /// (atomic rename + SHA-256 verify), journals the send as
    /// `kind = 'send'`, and returns `SendAck`. Worker reads the
    /// journaled vault paths, builds the MIME message, and submits
    /// via SMTP.
    ///
    /// 30 s handler timeout covers SHA-256 verification of typical
    /// attachment payloads (200 MB total verifies in ~400 ms;
    /// gigabyte-class verifies in a few seconds). SMTP upload itself
    /// runs on the worker, not the handler.
    ///
    /// Boxed to keep the `RequestParams` discriminant compact - the
    /// inline-bytes-free `SendWireRequest` is still large (HTML + text
    /// bodies + recipients + attachment metadata for many files) and
    /// would otherwise dominate the enum size.
    ActionSend { request: Box<SendWireRequest> },
    /// Phase 3 plan scope item 1: kick a sync run for the given account.
    /// The handler returns within microseconds (acquires the per-account
    /// map lock, spawns a runner if one is not already in flight, acks).
    /// Sync work runs in the spawned task; the eventual `sync.completed`
    /// notification carries the run's outcome.
    ///
    /// 5 s timeout: the handler is bounded enqueue + spawn work, never
    /// blocking on the network.
    SyncStartAccount { params: SyncStartAccountParams },
    /// Phase 3 plan scope item 1: cancel an in-flight sync run for the
    /// given account. Flips the runner's `CancellationToken`; the runner
    /// observes at the next checkpoint and emits `sync.completed` with
    /// `Cancelled`. The ack carries the active `run_id` so the caller
    /// can subscribe and await the cancellation outcome.
    ///
    /// 5 s timeout: the handler returns immediately after flipping the
    /// token; cancellation propagation is asynchronous.
    SyncCancelAccount { params: SyncCancelAccountParams },
    /// Phase 5: explicit-request calendar sync (manual "Sync now",
    /// post-account-add, RSVP-then-resync). The handler returns within
    /// microseconds: it acquires the per-account map, spawns or returns
    /// an existing runner's id, and acks. The kick-driven path
    /// (cadence + staleness gate) uses `ClientNotification::CalendarKick`
    /// instead and does not surface this request type.
    ///
    /// 5 s timeout: bounded handler work, never blocking on the network.
    CalendarStartAccountSync {
        params: CalendarStartAccountSyncParams,
    },
    /// Phase 5: explicit-request calendar cancel. Account-deletion
    /// cancel is piggybacked server-side inside `handle_cancel_account`
    /// (mirroring push); this request type is reserved for the
    /// explicit-request path.
    ///
    /// 5 s timeout: handler returns immediately after flipping the
    /// runner's cancellation token; cancellation propagation is async.
    CalendarCancelAccountSync {
        params: CalendarCancelAccountSyncParams,
    },
    /// Phase 6a (`docs/service/phase-6a-plan.md`): set the
    /// `is_visible` flag on a single `calendars` row. The flat-boolean
    /// half of the calendar UI write surface; event mutations are
    /// Phase 6c.
    ///
    /// 5 s timeout: handler is one bounded `with_conn` write.
    CalendarSetVisibility {
        params: CalendarSetVisibilityParams,
    },
    /// Phase 6a: per-thread UI state writes (`thread_ui_state` table,
    /// keyed on `(account_id, thread_id)`). Today's only field is
    /// `attachments_collapsed`; the IPC carries the full row so future
    /// thread-scoped UI flags can extend without a new method.
    ///
    /// 5 s timeout: handler is one bounded `with_conn` upsert.
    ThreadUiStateSet { params: ThreadUiStateSetParams },
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
            Self::ActionJobStatus { .. } => "action.job_status",
            Self::ActionMarkChatRead { .. } => "action.mark_chat_read",
            Self::ActionSend { .. } => "action.send",
            Self::SyncStartAccount { .. } => "sync.start_account",
            Self::SyncCancelAccount { .. } => "sync.cancel_account",
            Self::CalendarStartAccountSync { .. } => "calendar.start_account_sync",
            Self::CalendarCancelAccountSync { .. } => "calendar.cancel_account_sync",
            Self::CalendarSetVisibility { .. } => "calendar.set_visibility",
            Self::ThreadUiStateSet { .. } => "thread_ui_state.set",
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
            Self::ActionJobStatus { .. } => RequestTimeoutKind::Finite(Duration::from_secs(5)),
            // Handler-only budget: mark_chat_read_local + journal + ack.
            // Provider mark-read happens on the worker.
            Self::ActionMarkChatRead { .. } => RequestTimeoutKind::Finite(Duration::from_secs(10)),
            // Handler budget: validate + per-attachment SHA-256 verify
            // + atomic rename to vault + journal + ack. SMTP is on the
            // worker. 30 s comfortably covers the verify step for
            // realistic attachment sizes (gigabyte-class hashes in a
            // few seconds on commodity hardware).
            Self::ActionSend { .. } => RequestTimeoutKind::Finite(Duration::from_secs(30)),
            // Handler-only budget: enqueue + spawn (or look up an
            // existing runner and return the ack). No network or DB
            // work in the handler path.
            Self::SyncStartAccount { .. } => RequestTimeoutKind::Finite(Duration::from_secs(5)),
            // Handler-only budget: flip the token + return the active
            // `run_id`. Cancellation propagation is async.
            Self::SyncCancelAccount { .. } => RequestTimeoutKind::Finite(Duration::from_secs(5)),
            // Handler-only budgets for the calendar request pair.
            // Same shape as the sync pair above.
            Self::CalendarStartAccountSync { .. } => {
                RequestTimeoutKind::Finite(Duration::from_secs(5))
            }
            Self::CalendarCancelAccountSync { .. } => {
                RequestTimeoutKind::Finite(Duration::from_secs(5))
            }
            Self::CalendarSetVisibility { .. } => RequestTimeoutKind::Finite(Duration::from_secs(5)),
            Self::ThreadUiStateSet { .. } => RequestTimeoutKind::Finite(Duration::from_secs(5)),
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
            Self::ActionJobStatus { plan_id } => serde_json::json!({ "plan_id": plan_id }),
            Self::ActionMarkChatRead { chat_email } => {
                serde_json::json!({ "chat_email": chat_email })
            }
            Self::ActionSend { request } => serde_json::json!({ "request": request }),
            Self::SyncStartAccount { params } => serde_json::json!({ "params": params }),
            Self::SyncCancelAccount { params } => serde_json::json!({ "params": params }),
            Self::CalendarStartAccountSync { params } => serde_json::json!({ "params": params }),
            Self::CalendarCancelAccountSync { params } => {
                serde_json::json!({ "params": params })
            }
            Self::CalendarSetVisibility { params } => serde_json::json!({ "params": params }),
            Self::ThreadUiStateSet { params } => serde_json::json!({ "params": params }),
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
            "action.job_status" => {
                #[derive(Deserialize)]
                struct P {
                    plan_id: PlanId,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("action.job_status params: {e}"))?;
                Ok(Self::ActionJobStatus { plan_id: p.plan_id })
            }
            "action.mark_chat_read" => {
                #[derive(Deserialize)]
                struct P {
                    chat_email: String,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("action.mark_chat_read params: {e}"))?;
                Ok(Self::ActionMarkChatRead {
                    chat_email: p.chat_email,
                })
            }
            "action.send" => {
                #[derive(Deserialize)]
                struct P {
                    request: SendWireRequest,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("action.send params: {e}"))?;
                Ok(Self::ActionSend {
                    request: Box::new(p.request),
                })
            }
            "sync.start_account" => {
                #[derive(Deserialize)]
                struct P {
                    params: SyncStartAccountParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("sync.start_account params: {e}"))?;
                Ok(Self::SyncStartAccount { params: p.params })
            }
            "sync.cancel_account" => {
                #[derive(Deserialize)]
                struct P {
                    params: SyncCancelAccountParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("sync.cancel_account params: {e}"))?;
                Ok(Self::SyncCancelAccount { params: p.params })
            }
            "calendar.start_account_sync" => {
                #[derive(Deserialize)]
                struct P {
                    params: CalendarStartAccountSyncParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("calendar.start_account_sync params: {e}"))?;
                Ok(Self::CalendarStartAccountSync { params: p.params })
            }
            "calendar.cancel_account_sync" => {
                #[derive(Deserialize)]
                struct P {
                    params: CalendarCancelAccountSyncParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("calendar.cancel_account_sync params: {e}"))?;
                Ok(Self::CalendarCancelAccountSync { params: p.params })
            }
            "calendar.set_visibility" => {
                #[derive(Deserialize)]
                struct P {
                    params: CalendarSetVisibilityParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("calendar.set_visibility params: {e}"))?;
                Ok(Self::CalendarSetVisibility { params: p.params })
            }
            "thread_ui_state.set" => {
                #[derive(Deserialize)]
                struct P {
                    params: ThreadUiStateSetParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("thread_ui_state.set params: {e}"))?;
                Ok(Self::ThreadUiStateSet { params: p.params })
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
    fn action_send_method_name_is_dotted() {
        let req = SendWireRequest {
            send_id: PlanId::new_v7(),
            from_account_id: "acc-1".into(),
            message: crate::action::SendWireMessage {
                draft_id: "d".into(),
                from: "a@b".into(),
                to: vec!["c@d".into()],
                cc: Vec::new(),
                bcc: Vec::new(),
                subject: None,
                body_html: String::new(),
                body_text: String::new(),
                in_reply_to: None,
                references: None,
                thread_id: None,
            },
            attachments: Vec::new(),
        };
        assert_eq!(
            RequestParams::ActionSend {
                request: Box::new(req),
            }
            .method_name(),
            "action.send",
        );
    }

    #[test]
    fn action_send_timeout_is_thirty_seconds() {
        let req = SendWireRequest {
            send_id: PlanId::new_v7(),
            from_account_id: "acc-1".into(),
            message: crate::action::SendWireMessage {
                draft_id: "d".into(),
                from: "a@b".into(),
                to: vec!["c@d".into()],
                cc: Vec::new(),
                bcc: Vec::new(),
                subject: None,
                body_html: String::new(),
                body_text: String::new(),
                in_reply_to: None,
                references: None,
                thread_id: None,
            },
            attachments: Vec::new(),
        };
        assert_eq!(
            RequestParams::ActionSend {
                request: Box::new(req),
            }
            .timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(30)),
        );
    }

    #[test]
    fn action_send_round_trips_from_method_params() {
        use crate::action::{SendAttachmentSource, SendWireAttachment, SendWireMessage};

        let req = SendWireRequest {
            send_id: PlanId::new_v7(),
            from_account_id: "acc-1".into(),
            message: SendWireMessage {
                draft_id: "draft-9".into(),
                from: "Alice <alice@example.com>".into(),
                to: vec!["bob@example.com".into()],
                cc: Vec::new(),
                bcc: Vec::new(),
                subject: Some("hello".into()),
                body_html: "<p>hi</p>".into(),
                body_text: "hi".into(),
                in_reply_to: None,
                references: None,
                thread_id: None,
            },
            attachments: vec![SendWireAttachment {
                source: SendAttachmentSource::StagingFile {
                    relative_path: "0.bin".into(),
                    content_hash: [3u8; 32],
                },
                size: 42,
                mime: "application/pdf".into(),
                filename: "x.pdf".into(),
                content_id: None,
            }],
        };
        let original = RequestParams::ActionSend {
            request: Box::new(req),
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn sync_start_account_method_name_is_dotted() {
        let p = RequestParams::SyncStartAccount {
            params: SyncStartAccountParams {
                account_id: "acc-1".into(),
            },
        };
        assert_eq!(p.method_name(), "sync.start_account");
    }

    #[test]
    fn sync_start_account_timeout_is_five_seconds() {
        let p = RequestParams::SyncStartAccount {
            params: SyncStartAccountParams {
                account_id: "acc-1".into(),
            },
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn sync_start_account_does_not_bypass_admission() {
        let p = RequestParams::SyncStartAccount {
            params: SyncStartAccountParams {
                account_id: "acc-1".into(),
            },
        };
        assert!(!p.bypasses_admission());
    }

    #[test]
    fn sync_start_account_round_trips_from_method_params() {
        let original = RequestParams::SyncStartAccount {
            params: SyncStartAccountParams {
                account_id: "acc-1".into(),
            },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn sync_cancel_account_method_name_is_dotted() {
        let p = RequestParams::SyncCancelAccount {
            params: SyncCancelAccountParams {
                account_id: "acc-1".into(),
            },
        };
        assert_eq!(p.method_name(), "sync.cancel_account");
    }

    #[test]
    fn sync_cancel_account_timeout_is_five_seconds() {
        let p = RequestParams::SyncCancelAccount {
            params: SyncCancelAccountParams {
                account_id: "acc-1".into(),
            },
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn sync_cancel_account_round_trips_from_method_params() {
        let original = RequestParams::SyncCancelAccount {
            params: SyncCancelAccountParams {
                account_id: "acc-1".into(),
            },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn calendar_set_visibility_method_name_is_dotted() {
        let p = RequestParams::CalendarSetVisibility {
            params: CalendarSetVisibilityParams {
                calendar_id: "cal-1".into(),
                visible: true,
            },
        };
        assert_eq!(p.method_name(), "calendar.set_visibility");
    }

    #[test]
    fn calendar_set_visibility_timeout_is_five_seconds() {
        let p = RequestParams::CalendarSetVisibility {
            params: CalendarSetVisibilityParams {
                calendar_id: "cal-1".into(),
                visible: true,
            },
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn calendar_set_visibility_round_trips_from_method_params() {
        let original = RequestParams::CalendarSetVisibility {
            params: CalendarSetVisibilityParams {
                calendar_id: "cal-1".into(),
                visible: false,
            },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn thread_ui_state_set_method_name_is_dotted() {
        let p = RequestParams::ThreadUiStateSet {
            params: ThreadUiStateSetParams {
                account_id: "acc-1".into(),
                thread_id: "thread-1".into(),
                attachments_collapsed: Some(true),
            },
        };
        assert_eq!(p.method_name(), "thread_ui_state.set");
    }

    #[test]
    fn thread_ui_state_set_timeout_is_five_seconds() {
        let p = RequestParams::ThreadUiStateSet {
            params: ThreadUiStateSetParams {
                account_id: "acc-1".into(),
                thread_id: "thread-1".into(),
                attachments_collapsed: Some(true),
            },
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn thread_ui_state_set_round_trips_from_method_params() {
        let original = RequestParams::ThreadUiStateSet {
            params: ThreadUiStateSetParams {
                account_id: "acc-1".into(),
                thread_id: "thread-1".into(),
                attachments_collapsed: Some(false),
            },
        };
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
