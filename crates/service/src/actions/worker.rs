//! Action worker (Phase 2 task 9c).
//!
//! Drains the `action_jobs` / `action_job_ops` journal: leases ready
//! ops, dispatches via `batch_execute`, persists per-op outcomes,
//! emits `OperationOutcome` notifications, and finalizes the parent
//! job (`action.completed` notification) when all ops reach terminal
//! status.
//!
//! The worker runs as one tokio task spawned alongside the boot task
//! in `dispatch::run_service_with_io_and_lifecycle`. It awaits
//! `BootSharedState::wait_for_ready()` first so the journal helpers
//! can run against a fully-migrated DB and `BootContext` is populated;
//! after that it parks on `await_action_worker_wakeup()` until the
//! `action.execute_plan` handler signals new work.
//!
//! Phase 2 simplifications (vs the plan's full design):
//! - **No per-account semaphore.** The worker leases one op at a time
//!   and dispatches sequentially. The `batch_execute` call still
//!   handles per-account provider construction internally; the lost
//!   parallelism is acceptable for the action-service workload (one
//!   action per click typically) and can be reintroduced when bulk
//!   plans surface as a hot path. The plan's `action_job_ops_ready`
//!   partial index still drives the lease query.
//! - **No graceful shutdown.** The worker is dropped on service exit
//!   (tokio runtime teardown). A long-running batch_execute call
//!   gets cancelled at its next await point; the lease it took stays
//!   `leased` in the journal, and the next boot's
//!   `recover_stale_leases` resets it to `pending`.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use db::db::ReadDbState;
use db::db::action_journal::{
    JobTerminalStatus, LeasedOp, OpStatusCounts, OpTerminalStatus, ReplayableOp,
    count_ops_by_status, finalize_job, lease_next_ready_op, mark_op_terminal,
    unemitted_terminal_ops,
};
use service_api::{
    ActionCompleted, Notification, OperationId, OperationOutcome, OperationResult, PlanId,
    PlanSummary, RemoteFailure, WireMailOperation,
};
use tokio::sync::mpsc;

use super::context::ActionContext;
use super::outcome::{ActionError, ActionOutcome, RemoteFailureKind};
use super::wire_conversion::wire_to_mail;
use crate::boot::BootSharedState;
use crate::boot_progress::enqueue_notification;

/// Lease duration for ops the worker is currently executing. The next
/// boot's `recover_stale_leases` resets any lease whose
/// `lease_expires_at` is in the past, so this is a recovery-only
/// upper bound - the live worker doesn't need to renew it for ops
/// that complete inside the duration. 10 minutes is generous and
/// covers slow-network provider calls.
const LEASE_DURATION_MS: i64 = 10 * 60 * 1000;

/// Spawn the action worker. Returns the join handle so the caller can
/// abort on shutdown if needed.
pub(crate) fn spawn(
    boot_state: Arc<BootSharedState>,
    out_tx: mpsc::Sender<Vec<u8>>,
    app_data_dir: PathBuf,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run(boot_state, out_tx, app_data_dir))
}

async fn run(
    boot_state: Arc<BootSharedState>,
    out_tx: mpsc::Sender<Vec<u8>>,
    app_data_dir: PathBuf,
) {
    if boot_state.wait_for_ready().await.is_err() {
        log::info!("action worker: boot failed, exiting");
        return;
    }
    let Some(db_conn) = boot_state.db_conn() else {
        log::warn!("action worker: db_conn missing post-boot");
        return;
    };
    let Some(encryption_key) = boot_state.encryption_key() else {
        log::warn!("action worker: encryption_key missing post-boot");
        return;
    };
    let action_ctx = match build_action_context(db_conn, encryption_key, &app_data_dir) {
        Ok(ctx) => ctx,
        Err(error) => {
            log::error!("action worker: failed to build ActionContext: {error}");
            return;
        }
    };

    // On startup, replay any unemitted terminal outcomes from the
    // previous incarnation. The UI's per-plan applied_outcomes set
    // dedupes against what it already saw.
    replay_unemitted(&action_ctx, &out_tx).await;

    let worker_uuid = uuid::Uuid::now_v7();
    let owner_bytes = *worker_uuid.as_bytes();

    log::info!("action worker started (uuid={worker_uuid})");

    loop {
        boot_state.await_action_worker_wakeup().await;
        drain_one_pass(&action_ctx, &out_tx, &owner_bytes).await;
    }
}

fn build_action_context(
    db_conn: Arc<Mutex<db::db::Connection>>,
    encryption_key: [u8; 32],
    app_data_dir: &std::path::Path,
) -> Result<ActionContext, String> {
    let body_store = store::body_store::BodyStoreState::init(app_data_dir)
        .map_err(|e| format!("BodyStoreState::init: {e}"))?;
    let inline_images = store::inline_image_store::InlineImageStoreState::init(app_data_dir)
        .map_err(|e| format!("InlineImageStoreState::init: {e}"))?;
    let search = search::SearchState::init(app_data_dir)
        .map_err(|e| format!("SearchState::init: {e}"))?;
    Ok(ActionContext {
        db: ReadDbState::from_arc(db_conn),
        body_store,
        inline_images,
        search,
        encryption_key,
        suppress_pending_enqueue: false,
        in_flight: Arc::new(Mutex::new(HashSet::new())),
    })
}

/// Drain ready ops until the journal has none. Each iteration leases
/// one op, dispatches via `batch_execute`, persists the outcome, and
/// emits the wire notifications. Returns when `lease_next_ready_op`
/// returns `None` (no more pending rows).
async fn drain_one_pass(
    ctx: &ActionContext,
    out_tx: &mpsc::Sender<Vec<u8>>,
    owner: &[u8; 16],
) {
    loop {
        let leased = match lease_next_via_blocking(ctx, owner).await {
            Ok(Some(op)) => op,
            Ok(None) => return,
            Err(error) => {
                log::warn!("action worker: lease query failed: {error}");
                return;
            }
        };

        match run_one(ctx, out_tx, leased.clone()).await {
            Ok(()) => {}
            Err(error) => {
                log::warn!(
                    "action worker: op {:?}/{} dispatch failed: {error}",
                    leased.plan_id,
                    leased.operation_id,
                );
                // Best-effort: persist as failed so the lease clears.
                // The error is the worker's failure to even run the op
                // (deserialize / spawn_blocking), distinct from a
                // provider-level RemoteFailure that batch_execute
                // would have surfaced as `Failed`.
                let result = OperationResult::RemoteFailure {
                    failure: RemoteFailure {
                        provider_message: error,
                        http_status: None,
                        retryable: false,
                    },
                };
                let _ = persist_and_emit(ctx, out_tx, &leased, OpTerminalStatus::Failed, result)
                    .await;
            }
        }

        // Check parent-job completion regardless of whether the op
        // succeeded - one failed op doesn't stop the job; only "all
        // ops reached terminal" triggers finalization.
        let _ = maybe_finalize(ctx, out_tx, &leased.plan_id).await;
    }
}

async fn lease_next_via_blocking(
    ctx: &ActionContext,
    owner: &[u8; 16],
) -> Result<Option<LeasedOp>, String> {
    let conn = ctx.db.conn();
    let owner = *owner;
    tokio::task::spawn_blocking(move || {
        let conn = conn.lock().unwrap_or_else(PoisonError::into_inner);
        lease_next_ready_op(&conn, &owner, LEASE_DURATION_MS)
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))?
}

async fn run_one(
    ctx: &ActionContext,
    out_tx: &mpsc::Sender<Vec<u8>>,
    op: LeasedOp,
) -> Result<(), String> {
    let wire_op: WireMailOperation = serde_json::from_slice(&op.operation_blob)
        .map_err(|e| format!("deserialize WireMailOperation: {e}"))?;
    let mail_op = wire_to_mail(wire_op);
    let outcomes = super::batch_execute(
        ctx,
        vec![(op.account_id.clone(), op.thread_id.clone(), mail_op)],
    )
    .await;
    let outcome = outcomes
        .into_iter()
        .next()
        .ok_or_else(|| "batch_execute returned empty outcomes for one input op".to_string())?;
    let (status, result) = action_outcome_to_wire(outcome);
    persist_and_emit(ctx, out_tx, &op, status, result).await
}

async fn persist_and_emit(
    ctx: &ActionContext,
    out_tx: &mpsc::Sender<Vec<u8>>,
    op: &LeasedOp,
    status: OpTerminalStatus,
    result: OperationResult,
) -> Result<(), String> {
    let outcome_blob = serde_json::to_vec(&result)
        .map_err(|e| format!("serialize OperationResult: {e}"))?;
    let conn = ctx.db.conn();
    let plan_id = op.plan_id;
    let operation_id = op.operation_id;
    let blob_for_blocking = outcome_blob.clone();
    tokio::task::spawn_blocking(move || {
        let conn = conn.lock().unwrap_or_else(PoisonError::into_inner);
        mark_op_terminal(&conn, &plan_id, operation_id, status, &blob_for_blocking)
    })
    .await
    .map_err(|e| format!("spawn_blocking mark_op_terminal: {e}"))??;

    if !op.quiet {
        let outcome = OperationOutcome {
            plan_id: PlanId(uuid::Uuid::from_bytes(plan_id)),
            operation_id: OperationId(operation_id),
            result,
            service_generation: 0,
        };
        enqueue_notification(out_tx, &Notification::OperationOutcome(outcome));
    }
    Ok(())
}

async fn maybe_finalize(
    ctx: &ActionContext,
    out_tx: &mpsc::Sender<Vec<u8>>,
    plan_id_bytes: &[u8; 16],
) -> Result<(), String> {
    let conn = ctx.db.conn();
    let plan_id = *plan_id_bytes;
    let counts: OpStatusCounts = tokio::task::spawn_blocking(move || {
        let conn = conn.lock().unwrap_or_else(PoisonError::into_inner);
        count_ops_by_status(&conn, &plan_id)
    })
    .await
    .map_err(|e| format!("spawn_blocking count_ops_by_status: {e}"))??;
    if counts.non_terminal() > 0 {
        return Ok(());
    }
    let summary = PlanSummary {
        total: counts.total(),
        local_only: 0, // not tracked in journal; `Done` rolls up Success+LocalOnly
        remote_succeeded: counts.done,
        remote_failed: counts.failed,
        conflicts: counts.conflict,
    };
    let terminal_status = if counts.done == counts.total() {
        JobTerminalStatus::Completed
    } else if counts.done == 0 {
        JobTerminalStatus::Failed
    } else {
        JobTerminalStatus::Partial
    };
    let summary_blob =
        serde_json::to_vec(&summary).map_err(|e| format!("serialize PlanSummary: {e}"))?;
    let conn = ctx.db.conn();
    let plan_id_for_blocking = *plan_id_bytes;
    tokio::task::spawn_blocking(move || {
        let conn = conn.lock().unwrap_or_else(PoisonError::into_inner);
        finalize_job(&conn, &plan_id_for_blocking, terminal_status, &summary_blob)
    })
    .await
    .map_err(|e| format!("spawn_blocking finalize_job: {e}"))??;

    let completion = ActionCompleted {
        plan_id: PlanId(uuid::Uuid::from_bytes(*plan_id_bytes)),
        summary,
        service_generation: 0,
    };
    enqueue_notification(out_tx, &Notification::ActionCompleted(completion));
    Ok(())
}

async fn replay_unemitted(ctx: &ActionContext, out_tx: &mpsc::Sender<Vec<u8>>) {
    let conn = ctx.db.conn();
    let result: Result<Vec<ReplayableOp>, String> = tokio::task::spawn_blocking(move || {
        let conn = conn.lock().unwrap_or_else(PoisonError::into_inner);
        unemitted_terminal_ops(&conn)
    })
    .await
    .unwrap_or_else(|e| Err(format!("spawn_blocking: {e}")));
    let ops = match result {
        Ok(ops) => ops,
        Err(error) => {
            log::warn!("action worker: replay query failed: {error}");
            return;
        }
    };
    if ops.is_empty() {
        return;
    }
    log::info!("action worker: replaying {} unemitted terminal outcomes", ops.len());
    for op in ops {
        if op.quiet {
            continue;
        }
        let result: OperationResult = match serde_json::from_slice(&op.outcome_blob) {
            Ok(r) => r,
            Err(error) => {
                log::warn!(
                    "action worker: failed to deserialize replay outcome for {:?}/{}: {error}",
                    op.plan_id,
                    op.operation_id,
                );
                continue;
            }
        };
        let outcome = OperationOutcome {
            plan_id: PlanId(uuid::Uuid::from_bytes(op.plan_id)),
            operation_id: OperationId(op.operation_id),
            result,
            service_generation: 0,
        };
        enqueue_notification(out_tx, &Notification::OperationOutcome(outcome));
    }
}

/// Map an `ActionOutcome` (from `batch_execute`) to the wire-shaped
/// `OperationResult` + the journal's `OpTerminalStatus`. The wire
/// shape is intentionally narrower than the domain shape - provider-
/// specific error variants collapse into `RemoteFailure` with a
/// `retryable` flag derived from `RemoteFailureKind`.
fn action_outcome_to_wire(outcome: ActionOutcome) -> (OpTerminalStatus, OperationResult) {
    match outcome {
        ActionOutcome::Success | ActionOutcome::NoOp => {
            (OpTerminalStatus::Done, OperationResult::Success)
        }
        ActionOutcome::LocalOnly { .. } => {
            (OpTerminalStatus::Done, OperationResult::LocalOnly)
        }
        ActionOutcome::Failed { error } => match error {
            ActionError::NotFound(detail) | ActionError::InvalidState(detail) => (
                OpTerminalStatus::Conflict,
                OperationResult::ConflictRejected { detail },
            ),
            ActionError::Remote { kind, message } => {
                let retryable = matches!(
                    kind,
                    RemoteFailureKind::Transient | RemoteFailureKind::Unknown
                );
                (
                    OpTerminalStatus::Failed,
                    OperationResult::RemoteFailure {
                        failure: RemoteFailure {
                            provider_message: message,
                            http_status: None,
                            retryable,
                        },
                    },
                )
            }
            ActionError::Db(message) | ActionError::Build(message) => (
                OpTerminalStatus::Failed,
                OperationResult::RemoteFailure {
                    failure: RemoteFailure {
                        provider_message: message,
                        http_status: None,
                        retryable: false,
                    },
                },
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_success_maps_to_done() {
        let (s, r) = action_outcome_to_wire(ActionOutcome::Success);
        assert!(matches!(s, OpTerminalStatus::Done));
        assert!(matches!(r, OperationResult::Success));
    }

    #[test]
    fn outcome_noop_maps_to_done() {
        let (s, r) = action_outcome_to_wire(ActionOutcome::NoOp);
        assert!(matches!(s, OpTerminalStatus::Done));
        assert!(matches!(r, OperationResult::Success));
    }

    #[test]
    fn outcome_local_only_maps_to_done() {
        let (s, r) = action_outcome_to_wire(ActionOutcome::LocalOnly {
            reason: ActionError::Remote {
                kind: RemoteFailureKind::Transient,
                message: "timeout".into(),
            },
            retryable: true,
        });
        assert!(matches!(s, OpTerminalStatus::Done));
        assert!(matches!(r, OperationResult::LocalOnly));
    }

    #[test]
    fn outcome_failed_remote_transient_is_retryable() {
        let (s, r) = action_outcome_to_wire(ActionOutcome::Failed {
            error: ActionError::Remote {
                kind: RemoteFailureKind::Transient,
                message: "503 service unavailable".into(),
            },
        });
        assert!(matches!(s, OpTerminalStatus::Failed));
        match r {
            OperationResult::RemoteFailure { failure } => {
                assert!(failure.retryable);
                assert!(failure.provider_message.contains("503"));
            }
            other => panic!("expected RemoteFailure, got {other:?}"),
        }
    }

    #[test]
    fn outcome_failed_remote_permanent_is_not_retryable() {
        let (s, r) = action_outcome_to_wire(ActionOutcome::Failed {
            error: ActionError::Remote {
                kind: RemoteFailureKind::Permanent,
                message: "401 unauthorized".into(),
            },
        });
        assert!(matches!(s, OpTerminalStatus::Failed));
        match r {
            OperationResult::RemoteFailure { failure } => {
                assert!(!failure.retryable);
            }
            other => panic!("expected RemoteFailure, got {other:?}"),
        }
    }

    #[test]
    fn outcome_failed_not_found_is_conflict() {
        let (s, r) = action_outcome_to_wire(ActionOutcome::Failed {
            error: ActionError::NotFound("thread t1 not found".into()),
        });
        assert!(matches!(s, OpTerminalStatus::Conflict));
        match r {
            OperationResult::ConflictRejected { detail } => {
                assert!(detail.contains("not found"));
            }
            other => panic!("expected ConflictRejected, got {other:?}"),
        }
    }

    #[test]
    fn outcome_failed_db_is_failed_not_retryable() {
        let (s, r) = action_outcome_to_wire(ActionOutcome::Failed {
            error: ActionError::Db("row locked".into()),
        });
        assert!(matches!(s, OpTerminalStatus::Failed));
        match r {
            OperationResult::RemoteFailure { failure } => {
                assert!(!failure.retryable);
            }
            other => panic!("expected RemoteFailure, got {other:?}"),
        }
    }
}
