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
//! can run against a fully-migrated DB and `BootContext` is populated.
//! Each loop iteration drains all queues, then parks on
//! `await_action_worker_wakeup()`. Drain-then-await (not await-then-drain)
//! is load-bearing: a row left `queued` by a previous incarnation that
//! crashed post-journal-commit pre-execution would be invisible to a
//! worker that parked first. tokio::sync::Notify retains one permit, so
//! handler-side notifies that fire while the worker is draining are
//! observed by the next await with no race.
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
    JobTerminalStatus, LeasedOp, LeasedQuietJob, OpStatusCounts, OpTerminalStatus, PerOpJobKind,
    ReplayableOp, count_ops_by_status, finalize_job, lease_next_ready_op,
    lease_next_ready_quiet_job, mark_op_terminal, unemitted_terminal_ops,
    unfinalized_per_op_plan_jobs,
};
use service_api::{
    ActionCompleted, CalendarActionCompleted, CalendarOperationOutcome, CalendarOperationResult,
    Notification, OperationId, OperationOutcome, OperationResult, PlanId, PlanSummary,
    RemoteFailure, WireCalendarOperation, WireMailOperation,
};
use tokio::sync::mpsc;

use super::context::ActionContext;
use super::outcome::{ActionError, ActionOutcome, RemoteFailureKind};
use super::pending::enqueue_if_retryable;
use super::provider::create_provider;
use super::wire_conversion::wire_to_mail;
use common::types::ActionProviderCtx;
use db::progress::NoopProgressReporter;
use crate::boot::BootSharedState;
use crate::boot_progress::send_must_deliver_notification;

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

    // Drain-then-await ordering: an op enqueued before the worker spawned
    // (or a row left `queued` by a previous incarnation that crashed
    // post-journal-commit pre-execution) would be missed if the worker
    // parked first. tokio::sync::Notify retains one permit, so a
    // notify_one that fires while we're draining is observed by the next
    // await call - no race against handler-side notifies during drain.
    loop {
        drain_one_pass(&action_ctx, &out_tx, &owner_bytes).await;
        // Phase 2 task 15: drain any queued mark_chat_read jobs. These
        // are quiet (no per-op outcomes); the worker emits one final
        // `ActionCompleted` per finalized job.
        drain_mark_chat_read_jobs(&action_ctx, &out_tx, &owner_bytes).await;
        // Phase 2 task 13: drain any queued send jobs. Quiet jobs that
        // emit one `ActionCompleted` after SMTP submit (success or
        // failure). The vault directory is unlinked on terminal
        // status regardless of outcome.
        drain_send_jobs(&action_ctx, &out_tx, &owner_bytes, &app_data_dir).await;
        // Phase 2 task 17: walk the snooze table for due threads and
        // unsnooze them via the standard `snooze::unsnooze` action.
        // Triggered by `pending_ops.kick`; same wakeup as the journal
        // / pending-ops drains.
        crate::snooze_runner::drain_due_snoozes(&action_ctx).await;
        // Phase 2 task 18: each wakeup also drains the transient-retry
        // queue (`pending_operations`). Sharing a wakeup signal with
        // the journal drain keeps the worker single-purpose; both
        // queues are typically empty so the extra pass is cheap. The
        // kick handler (`pending_ops.kick`) and the journal handler
        // both fire `boot_state.notify_action_worker()` so either
        // trigger does the same work.
        super::pending::process_pending_ops(&action_ctx).await;
        boot_state.await_action_worker_wakeup().await;
    }
}

async fn drain_mark_chat_read_jobs(
    ctx: &ActionContext,
    out_tx: &mpsc::Sender<Vec<u8>>,
    owner: &[u8; 16],
) {
    loop {
        let job = match lease_next_quiet_job_via_blocking(ctx, "mark_chat_read", owner).await {
            Ok(Some(job)) => job,
            Ok(None) => return,
            Err(error) => {
                log::warn!("action worker: mark_chat_read lease query failed: {error}");
                return;
            }
        };
        if let Err(error) = run_mark_chat_read(ctx, out_tx, &job).await {
            log::warn!(
                "action worker: mark_chat_read job {:?} failed: {error}",
                job.job_id,
            );
            // Best-effort: finalize the job as failed so the lease
            // clears. Recovery would otherwise reset and retry, which
            // is fine, but is wasteful when the failure was due to
            // payload deserialization (deterministic).
            let summary = PlanSummary {
                total: 0,
                local_only: 0,
                remote_succeeded: 0,
                remote_failed: 0,
                conflicts: 0,
            };
            let summary_blob = serde_json::to_vec(&summary).unwrap_or_default();
            let conn = ctx.db.conn();
            let job_id_bytes = job.job_id;
            let _ = tokio::task::spawn_blocking(move || {
                let conn = conn.lock().unwrap_or_else(PoisonError::into_inner);
                finalize_job(&conn, &job_id_bytes, JobTerminalStatus::Failed, &summary_blob)
            })
            .await;
        }
    }
}

async fn lease_next_quiet_job_via_blocking(
    ctx: &ActionContext,
    kind: &'static str,
    owner: &[u8; 16],
) -> Result<Option<LeasedQuietJob>, String> {
    let conn = ctx.db.conn();
    let owner = *owner;
    tokio::task::spawn_blocking(move || {
        let conn = conn.lock().unwrap_or_else(PoisonError::into_inner);
        lease_next_ready_quiet_job(&conn, kind, &owner, LEASE_DURATION_MS)
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))?
}

async fn run_mark_chat_read(
    ctx: &ActionContext,
    out_tx: &mpsc::Sender<Vec<u8>>,
    job: &LeasedQuietJob,
) -> Result<(), String> {
    use crate::handlers::JournaledChatRead;
    let payload: JournaledChatRead = serde_json::from_slice(&job.payload)
        .map_err(|e| format!("deserialize JournaledChatRead: {e}"))?;
    let total = u32::try_from(payload.affected.len()).unwrap_or(u32::MAX);
    mark_chat_read_remote(ctx, payload.affected).await;

    // Finalize the job. Treat as Completed - mark_chat_read_remote
    // enqueues retryable failures into pending_operations rather
    // than surfacing them, so we cannot distinguish a partial
    // failure from a clean run at this layer. The pending_ops
    // periodic drainer eventually reconciles.
    let summary = PlanSummary {
        total,
        local_only: 0,
        remote_succeeded: total,
        remote_failed: 0,
        conflicts: 0,
    };
    let summary_blob =
        serde_json::to_vec(&summary).map_err(|e| format!("serialize PlanSummary: {e}"))?;
    let conn = ctx.db.conn();
    let job_id_bytes = job.job_id;
    tokio::task::spawn_blocking(move || {
        let conn = conn.lock().unwrap_or_else(PoisonError::into_inner);
        finalize_job(&conn, &job_id_bytes, JobTerminalStatus::Completed, &summary_blob)
    })
    .await
    .map_err(|e| format!("spawn_blocking finalize_job: {e}"))??;

    let completion = ActionCompleted {
        plan_id: PlanId(uuid::Uuid::from_bytes(job.job_id)),
        summary,
        service_generation: 0,
    };
    send_must_deliver_notification(out_tx, &Notification::ActionCompleted(completion)).await;
    Ok(())
}

/// Drain queued `kind = 'send'` jobs. Each iteration leases one job,
/// reconstructs a `SendRequest` from the journaled payload (reading
/// attachment bytes from the Service-owned vault), calls
/// `super::send::send_email`, and finalizes with summary
/// `total = remote_succeeded = 1` on success or `remote_failed = 1`
/// on failure. The vault directory is unlinked once the job reaches
/// terminal status regardless of outcome - the journal carries the
/// summary, the bytes are no longer needed.
///
/// Quiet job: no per-operation `OperationOutcome` notifications. The
/// final `ActionCompleted` is the only wire signal the UI receives.
async fn drain_send_jobs(
    ctx: &ActionContext,
    out_tx: &mpsc::Sender<Vec<u8>>,
    owner: &[u8; 16],
    app_data_dir: &std::path::Path,
) {
    loop {
        let job = match lease_next_quiet_job_via_blocking(ctx, "send", owner).await {
            Ok(Some(job)) => job,
            Ok(None) => return,
            Err(error) => {
                log::warn!("action worker: send lease query failed: {error}");
                return;
            }
        };
        if let Err(error) = run_send(ctx, out_tx, &job, app_data_dir).await {
            log::warn!(
                "action worker: send job {:?} failed at worker layer: {error}",
                job.job_id,
            );
            // Worker-layer failure (deserialize, unlinkable vault, etc.):
            // finalize as failed so the lease clears. Recovery would
            // otherwise reset and retry, which would deterministically
            // re-fail in the same way.
            let summary = PlanSummary {
                total: 1,
                local_only: 0,
                remote_succeeded: 0,
                remote_failed: 1,
                conflicts: 0,
            };
            let summary_blob = serde_json::to_vec(&summary).unwrap_or_default();
            let conn = ctx.db.conn();
            let job_id_bytes = job.job_id;
            let _ = tokio::task::spawn_blocking(move || {
                let conn = conn.lock().unwrap_or_else(PoisonError::into_inner);
                finalize_job(&conn, &job_id_bytes, JobTerminalStatus::Failed, &summary_blob)
            })
            .await;

            // Best-effort vault cleanup. If the deserialize failed we
            // never learned the send_id, but the job_id IS the
            // send_id (UI-generated, journaled) so we can derive the
            // vault path directly.
            crate::send_vault::cleanup_vault_dir(
                app_data_dir,
                &PlanId(uuid::Uuid::from_bytes(job.job_id)),
            );
        }
    }
}

async fn run_send(
    ctx: &ActionContext,
    out_tx: &mpsc::Sender<Vec<u8>>,
    job: &LeasedQuietJob,
    app_data_dir: &std::path::Path,
) -> Result<(), String> {
    use crate::handlers::JournaledSend;

    let payload: JournaledSend = serde_json::from_slice(&job.payload)
        .map_err(|e| format!("deserialize JournaledSend: {e}"))?;
    let send_id = payload.send_id;

    // Read each vault file's bytes synchronously inside spawn_blocking
    // (the SendRequest carries inline Vec<u8>; the existing send_email
    // path expects bytes already loaded). Vault unlink runs on the
    // terminal path regardless of success/failure.
    let request = match build_send_request(payload).await {
        Ok(req) => req,
        Err(error) => {
            crate::send_vault::cleanup_vault_dir(app_data_dir, &send_id);
            return Err(error);
        }
    };

    let outcome = super::send::send_email(ctx, request).await;

    let (status, summary) = match &outcome {
        ActionOutcome::Success | ActionOutcome::NoOp => (
            JobTerminalStatus::Completed,
            PlanSummary {
                total: 1,
                local_only: 0,
                remote_succeeded: 1,
                remote_failed: 0,
                conflicts: 0,
            },
        ),
        ActionOutcome::Failed { .. } | ActionOutcome::LocalOnly { .. } => (
            JobTerminalStatus::Failed,
            PlanSummary {
                total: 1,
                local_only: 0,
                remote_succeeded: 0,
                remote_failed: 1,
                conflicts: 0,
            },
        ),
    };

    let summary_blob =
        serde_json::to_vec(&summary).map_err(|e| format!("serialize PlanSummary: {e}"))?;
    let conn = ctx.db.conn();
    let job_id_bytes = job.job_id;
    tokio::task::spawn_blocking(move || {
        let conn = conn.lock().unwrap_or_else(PoisonError::into_inner);
        finalize_job(&conn, &job_id_bytes, status, &summary_blob)
    })
    .await
    .map_err(|e| format!("spawn_blocking finalize_job: {e}"))??;

    crate::send_vault::cleanup_vault_dir(app_data_dir, &send_id);

    let completion = ActionCompleted {
        plan_id: send_id,
        summary,
        service_generation: 0,
    };
    send_must_deliver_notification(out_tx, &Notification::ActionCompleted(completion)).await;
    Ok(())
}

/// Read attachment bytes from the vault and assemble a `SendRequest`
/// for `super::send::send_email`. Runs inside spawn_blocking - the
/// vault files can be tens of megabytes apiece and we must not block
/// the runtime.
async fn build_send_request(
    payload: crate::handlers::JournaledSend,
) -> Result<crate::send::SendRequest, String> {
    tokio::task::spawn_blocking(move || {
        let mut attachments = Vec::with_capacity(payload.attachments.len());
        for att in payload.attachments {
            let data = std::fs::read(&att.vault_path).map_err(|e| {
                format!("read vault file {}: {e}", att.vault_path.display())
            })?;
            attachments.push(crate::send::SendAttachment {
                filename: att.filename,
                mime_type: att.mime,
                data,
                content_id: att.content_id,
            });
        }
        Ok::<_, String>(crate::send::SendRequest {
            draft_id: payload.message.draft_id,
            account_id: payload.account_id,
            from: payload.message.from,
            to: payload.message.to,
            cc: payload.message.cc,
            bcc: payload.message.bcc,
            subject: payload.message.subject,
            body_html: payload.message.body_html,
            body_text: payload.message.body_text,
            attachments,
            in_reply_to: payload.message.in_reply_to,
            references: payload.message.references,
            thread_id: payload.message.thread_id,
        })
    })
    .await
    .map_err(|e| format!("spawn_blocking build_send_request: {e}"))?
}

fn build_action_context(
    db_conn: Arc<Mutex<db::db::Connection>>,
    encryption_key: [u8; 32],
    app_data_dir: &std::path::Path,
) -> Result<ActionContext, String> {
    let body_store = store::body_store::BodyStoreReadState::init(app_data_dir)
        .map_err(|e| format!("BodyStoreReadState::init: {e}"))?;
    let inline_images = store::inline_image_store::InlineImageStoreReadState::init(app_data_dir)
        .map_err(|e| format!("InlineImageStoreReadState::init: {e}"))?;
    let search = search::SearchReadState::init(app_data_dir)
        .map_err(|e| format!("SearchReadState::init: {e}"))?;
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
    // Sweep mail-plan jobs whose ops are all terminal but whose parent
    // job_status is not. This is the recovery path for a crash between
    // mark_op_terminal (clears the op lease) and finalize_job (writes
    // the parent terminal status) - they're separate transactions, so
    // a crash in between leaves the job stranded at queued/leased.
    // lease_next_ready_op then returns None forever for that job, and
    // without this sweep maybe_finalize is never called again.
    let conn = ctx.db.conn();
    let stranded = match tokio::task::spawn_blocking(move || {
        let conn = conn.lock().unwrap_or_else(PoisonError::into_inner);
        unfinalized_per_op_plan_jobs(&conn)
    })
    .await
    {
        Ok(Ok(ids)) => ids,
        Ok(Err(error)) => {
            log::warn!("action worker: unfinalized_per_op_plan_jobs failed: {error}");
            Vec::new()
        }
        Err(error) => {
            log::warn!("action worker: spawn_blocking unfinalized_per_op_plan_jobs: {error}");
            Vec::new()
        }
    };
    for (plan_id, kind) in stranded {
        let _ = maybe_finalize(ctx, out_tx, &plan_id, kind).await;
    }

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
        // ops reached terminal" triggers finalization. Unknown kinds
        // skip finalization (the row is already in 'failed' status
        // and the parent stays as-is for the user-visible support
        // record).
        if let Some(kind) = leased.kind {
            let _ = maybe_finalize(ctx, out_tx, &leased.plan_id, kind).await;
        }
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
    match op.kind {
        Some(PerOpJobKind::MailPlan) => run_one_mail(ctx, out_tx, op).await,
        Some(PerOpJobKind::CalendarPlan) => run_one_calendar(ctx, out_tx, op).await,
        None => {
            // Phase 6c-7: corrupt journal row. Mark the op as failed
            // with a structured reason blob so the lease index drops it
            // and the parent can be finalized as `Failed`. The user-
            // visible record persists in `action_jobs` history for
            // support flows; the worker emits a generic
            // `OperationOutcome::RemoteFailure` so the UI's
            // `pending_action_plans` awaiter (mail) or
            // `pending_calendar_action_plans` awaiter (calendar)
            // doesn't hang. We bias to `OperationResult` here because
            // the parent kind is unknown - both wire shapes' awaiters
            // see the per-plan `*_completed` frame eventually via
            // maybe_finalize-best-effort.
            log::error!(
                "action worker: corrupt journal row for plan {:?}/{}: kind={:?}",
                op.plan_id,
                op.operation_id,
                op.raw_kind,
            );
            let result = OperationResult::RemoteFailure {
                failure: RemoteFailure {
                    provider_message: format!(
                        "JournalCorrupt: unknown kind {:?}",
                        op.raw_kind,
                    ),
                    http_status: None,
                    retryable: false,
                },
            };
            persist_and_emit(ctx, out_tx, &op, OpTerminalStatus::Failed, result).await
        }
    }
}

async fn run_one_mail(
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

async fn run_one_calendar(
    ctx: &ActionContext,
    out_tx: &mpsc::Sender<Vec<u8>>,
    op: LeasedOp,
) -> Result<(), String> {
    let wire_op: WireCalendarOperation = serde_json::from_slice(&op.operation_blob)
        .map_err(|e| format!("deserialize WireCalendarOperation: {e}"))?;
    // Build a CalendarActionContext from the worker's ActionContext.
    // ActionContext.db is `ReadDbState`; the calendar pipeline needs
    // `WriteDbState`. The worker holds a `WriteDbState` clone in
    // `boot_state`; for now we build one from the connection arc.
    let cal_ctx = action_types::CalendarActionContext {
        db: service_state::WriteDbState::from_arc(ctx.db.conn()),
        encryption_key: ctx.encryption_key,
    };
    let cal_op = service_api::CalendarActionWireOperation {
        operation_id: OperationId(op.operation_id),
        account_id: op.account_id.clone(),
        operation: wire_op,
    };
    let mut results = crate::cal_actions::batch_execute(&cal_ctx, vec![cal_op]).await;
    let cal_result = results
        .pop()
        .ok_or_else(|| "cal_actions::batch_execute returned empty outcomes".to_string())?;

    // Persist the wire result blob + emit CalendarOperationOutcome.
    let status = match &cal_result {
        CalendarOperationResult::Success | CalendarOperationResult::LocalOnly { .. } => {
            OpTerminalStatus::Done
        }
        CalendarOperationResult::Failed { .. } => OpTerminalStatus::Failed,
    };
    let outcome_blob = serde_json::to_vec(&cal_result)
        .map_err(|e| format!("serialize CalendarOperationResult: {e}"))?;
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
        let outcome = CalendarOperationOutcome {
            plan_id: PlanId(uuid::Uuid::from_bytes(plan_id)),
            operation_id: OperationId(operation_id),
            result: cal_result,
            service_generation: 0,
        };
        send_must_deliver_notification(
            out_tx,
            &Notification::CalendarOperationOutcome(outcome),
        )
        .await;
    }
    Ok(())
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
        send_must_deliver_notification(out_tx, &Notification::OperationOutcome(outcome)).await;
    }
    Ok(())
}

async fn maybe_finalize(
    ctx: &ActionContext,
    out_tx: &mpsc::Sender<Vec<u8>>,
    plan_id_bytes: &[u8; 16],
    kind: PerOpJobKind,
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

    let plan_id_uuid = PlanId(uuid::Uuid::from_bytes(*plan_id_bytes));
    match kind {
        PerOpJobKind::MailPlan => {
            let completion = ActionCompleted {
                plan_id: plan_id_uuid,
                summary,
                service_generation: 0,
            };
            send_must_deliver_notification(out_tx, &Notification::ActionCompleted(completion))
                .await;
        }
        PerOpJobKind::CalendarPlan => {
            // Phase 6c: the calendar pipeline does not journal a
            // per-op result-list rollup the way mail does (PlanSummary
            // counts terminal statuses without distinguishing
            // CalendarOperationResult variants). The UI's awaiter
            // (Phase 6c-9) keys on plan_id and triggers a reload via
            // CalendarChanged regardless of the exact per-op result;
            // emit an empty results vector to mirror the contract.
            // Phase 6d can populate per-op results if the UI grows a
            // need.
            let completion = CalendarActionCompleted {
                plan_id: plan_id_uuid,
                results: Vec::new(),
                service_generation: 0,
            };
            send_must_deliver_notification(
                out_tx,
                &Notification::CalendarActionCompleted(completion),
            )
            .await;
        }
    }
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
        match op.kind {
            Some(PerOpJobKind::MailPlan) => {
                let result: OperationResult = match serde_json::from_slice(&op.outcome_blob) {
                    Ok(r) => r,
                    Err(error) => {
                        log::warn!(
                            "action worker: failed to deserialize OperationResult replay \
                             outcome for {:?}/{}: {error}",
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
                send_must_deliver_notification(out_tx, &Notification::OperationOutcome(outcome))
                    .await;
            }
            Some(PerOpJobKind::CalendarPlan) => {
                let result: CalendarOperationResult =
                    match serde_json::from_slice(&op.outcome_blob) {
                        Ok(r) => r,
                        Err(error) => {
                            log::warn!(
                                "action worker: failed to deserialize CalendarOperationResult \
                                 replay outcome for {:?}/{}: {error}",
                                op.plan_id,
                                op.operation_id,
                            );
                            continue;
                        }
                    };
                let outcome = CalendarOperationOutcome {
                    plan_id: PlanId(uuid::Uuid::from_bytes(op.plan_id)),
                    operation_id: OperationId(op.operation_id),
                    result,
                    service_generation: 0,
                };
                send_must_deliver_notification(
                    out_tx,
                    &Notification::CalendarOperationOutcome(outcome),
                )
                .await;
            }
            None => {
                log::warn!(
                    "action worker: replay skipped corrupt-kind row {:?}/{}: kind={:?}",
                    op.plan_id,
                    op.operation_id,
                    op.raw_kind,
                );
            }
        }
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

/// Service-side equivalent of `core::chat::mark_chat_read_remote`.
///
/// Phase 2 task 15: dispatched by the worker after the
/// `mark_chat_read` handler journals affected threads. Per-account
/// provider, sequential per-thread dispatch within an account; failed
/// remote calls enqueue a pending op for the periodic retry. The
/// chat read-state mutation has already committed locally inside the
/// handler; this is best-effort remote propagation.
async fn mark_chat_read_remote(ctx: &ActionContext, affected: Vec<(String, String)>) {
    if affected.is_empty() {
        return;
    }
    let mut by_account: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for (aid, tid) in affected {
        by_account.entry(aid).or_default().push(tid);
    }
    for (account_id, thread_ids) in by_account {
        let provider = match create_provider(&ctx.db, &account_id, ctx.encryption_key).await {
            Ok(p) => p,
            Err(error) => {
                for thread_id in &thread_ids {
                    let outcome = ActionOutcome::LocalOnly {
                        reason: ActionError::remote(error.clone()),
                        retryable: true,
                    };
                    enqueue_if_retryable(
                        ctx,
                        &outcome,
                        &account_id,
                        "markRead",
                        thread_id,
                        r#"{"read":true}"#,
                    )
                    .await;
                }
                continue;
            }
        };
        for thread_id in thread_ids {
            let provider_ctx = ActionProviderCtx {
                account_id: &account_id,
                db: &ctx.db,
                progress: &NoopProgressReporter,
            };
            let outcome = match provider.mark_read(&provider_ctx, &thread_id, true).await {
                Ok(()) => ActionOutcome::Success,
                Err(error) => ActionOutcome::LocalOnly {
                    reason: ActionError::remote(error.to_string()),
                    retryable: true,
                },
            };
            enqueue_if_retryable(
                ctx,
                &outcome,
                &account_id,
                "markRead",
                &thread_id,
                r#"{"read":true}"#,
            )
            .await;
        }
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
