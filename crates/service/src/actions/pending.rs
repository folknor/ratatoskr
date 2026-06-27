//! Pending-action queue integration.
//!
//! When an email action returns `LocalOnly { retryable: true }`, the action
//! enqueues a pending operation. A periodic worker re-dispatches pending ops
//! through the action service.

use super::context::ActionContext;
use super::operation::MailOperation;
use super::outcome::{ActionError, ActionOutcome};
use crate::sync::SyncRuntime;
use db::db::pending_ops::{
    db_pending_ops_delete_sync, db_pending_ops_enqueue_sync, db_pending_ops_get,
    db_pending_ops_increment_retry_sync, db_pending_ops_recover_executing_sync,
    db_pending_ops_update_status_sync,
};

const STALE_LABEL_INTENT_AGE_SECS: i64 = 48 * 60 * 60;

/// Per-action-type retry policy.
struct RetryPolicy {
    max_retries: i64,
}

/// Look up retry policy by operation type.
///
/// Folder-level actions (archive, trash, spam, move, delete) get the most
/// aggressive retry - silent divergence is the #1 user-visible bug.
/// Label actions get moderate retry. Flag actions (star, read) get lighter
/// retry since sync reconciles them within 5 minutes.
fn retry_policy(operation_type: &str) -> RetryPolicy {
    match operation_type {
        "archive" | "trash" | "spam" | "moveToFolder" | "permanentDelete" => {
            RetryPolicy { max_retries: 10 }
        }
        "addLabel" | "removeLabel" | "applyLabelGroup" | "removeLabelGroup" => {
            RetryPolicy { max_retries: 7 }
        }
        "star" | "markRead" => RetryPolicy { max_retries: 5 },
        _ => RetryPolicy { max_retries: 5 },
    }
}

/// Enqueue a pending operation if the outcome is retryable LocalOnly.
///
/// Called by action functions after determining the outcome. Only enqueues
/// if `retryable` is true AND `reason.is_retryable()` (policy + capability).
///
/// Deduplication is atomic: `db_pending_ops_enqueue_sync` replaces any
/// existing pending/executing op for the same (account_id, resource_id,
/// operation_type) in a single connection hold. This handles both exact
/// duplicates and directional updates (e.g., spam->unspam replaces stale
/// params).
pub async fn enqueue_if_retryable(
    ctx: &ActionContext,
    outcome: &ActionOutcome,
    account_id: &str,
    operation_type: &str,
    resource_id: &str,
    params_json: &str,
) {
    let _ = enqueue_if_retryable_with_id(
        ctx,
        outcome,
        account_id,
        operation_type,
        resource_id,
        params_json,
    )
    .await;
}

pub async fn enqueue_if_retryable_with_id(
    ctx: &ActionContext,
    outcome: &ActionOutcome,
    account_id: &str,
    operation_type: &str,
    resource_id: &str,
    params_json: &str,
) -> Option<String> {
    // Suppressed during retry dispatch to prevent duplicate enqueue.
    if ctx.suppress_pending_enqueue {
        return None;
    }

    if let ActionOutcome::LocalOnly {
        retryable: true,
        reason,
    } = outcome
    {
        if !reason.is_retryable() {
            return None;
        }

        let policy = retry_policy(operation_type);
        let op_id = uuid::Uuid::new_v4().to_string();
        let write_db = ctx.write_db.clone();
        let account_id = account_id.to_string();
        let operation_type = operation_type.to_string();
        let resource_id = resource_id.to_string();
        let params_json = params_json.to_string();
        let enqueue_result = write_db
            .with_write({
                let op_id = op_id.clone();
                move |conn| {
                    db_pending_ops_enqueue_sync(
                        conn,
                        &op_id,
                        &account_id,
                        &operation_type,
                        &resource_id,
                        &params_json,
                        policy.max_retries,
                    )
                }
            })
            .await;
        if let Err(e) = enqueue_result {
            log::warn!("[pending_ops] Failed to enqueue pending op: {e}");
            return None;
        }
        Some(op_id)
    } else {
        None
    }
}

/// Process pending operations from the queue.
///
/// Called periodically (e.g., on SyncTick). Fetches pending ops, groups by
/// account, and re-dispatches each sequentially through the resident engine
/// (via `batch_execute`). Respects the in-flight guard - skips threads with
/// active mutations.
pub async fn process_pending_ops(ctx: &ActionContext, sync_runtime: Option<&SyncRuntime>) {
    let ops = match db_pending_ops_get(&ctx.write_db.writer_pool(), None, Some(20)).await {
        Ok(ops) => ops,
        Err(e) => {
            log::warn!("[pending_ops] Failed to fetch pending ops: {e}");
            return;
        }
    };

    sweep_stale_label_intents(ctx).await;

    if ops.is_empty() {
        return;
    }

    log::info!("[pending_ops] Processing {} pending operations", ops.len());

    // Suppress enqueue during retry to prevent re-enqueue loops.
    let mut retry_ctx = ctx.clone();
    retry_ctx.suppress_pending_enqueue = true;

    // Group by account for provider reuse
    let mut groups: std::collections::HashMap<String, Vec<_>> = std::collections::HashMap::new();
    for op in ops {
        groups.entry(op.account_id.clone()).or_default().push(op);
    }

    // Process sequentially across account groups
    for account_ops in groups.into_values() {
        process_account_group(ctx, &retry_ctx, sync_runtime, account_ops).await;
    }
}

async fn sweep_stale_label_intents(ctx: &ActionContext) {
    let db = ctx.write_db.clone();
    let result = db
        .with_write(move |conn| {
            db::db::queries_extra::delete_stale_pending_thread_label_intents(
                conn,
                STALE_LABEL_INTENT_AGE_SECS,
            )
        })
        .await;

    match result {
        Ok(count) if count > 0 => {
            log::warn!("[pending_ops] Cleared {count} stale pending label intents");
        }
        Err(e) => {
            log::warn!("[pending_ops] stale label intent cleanup failed: {e}");
        }
        _ => {}
    }
}

/// Process one account's pending ops, re-dispatching each through the
/// resident engine.
async fn process_account_group(
    ctx: &ActionContext,
    retry_ctx: &ActionContext,
    sync_runtime: Option<&SyncRuntime>,
    ops: Vec<db::db::pending_ops::PendingOperation>,
) {
    for op in ops {
        // In-flight guard: acquire to block concurrent user mutations,
        // or skip if already in flight from a user action.
        let _guard = match ctx.try_acquire_flight(&op.account_id, &op.resource_id) {
            Some(g) => g,
            None => {
                log::debug!(
                    "[pending_ops] Skipping {} for {}/{} - in flight",
                    op.operation_type,
                    op.account_id,
                    op.resource_id
                );
                continue; // Leave pending, don't increment retry
            }
        };

        // Mark as executing
        let update_result = ctx
            .write_db
            .with_write({
                let op_id = op.id.clone();
                move |conn| db_pending_ops_update_status_sync(conn, &op_id, "executing", None)
            })
            .await;
        if let Err(e) = update_result {
            log::warn!("[pending_ops] Failed to mark op {} executing: {e}", op.id);
            continue;
        }

        let outcome = dispatch_pending_op(
            retry_ctx,
            sync_runtime,
            &op.account_id,
            &op.operation_type,
            &op.resource_id,
            &op.params,
        )
        .await;

        match outcome {
            ActionOutcome::Success | ActionOutcome::NoOp => {
                let _ = ctx
                    .write_db
                    .with_write({
                        let op_id = op.id.clone();
                        move |conn| db_pending_ops_delete_sync(conn, &op_id)
                    })
                    .await;
                log::info!(
                    "[pending_ops] Completed {} for {}/{}",
                    op.operation_type,
                    op.account_id,
                    op.resource_id
                );
            }
            ActionOutcome::LocalOnly { reason, .. } | ActionOutcome::Failed { error: reason } => {
                log::warn!(
                    "[pending_ops] Retry failed for {} {}/{}: {reason}",
                    op.operation_type,
                    op.account_id,
                    op.resource_id
                );
                let _ = ctx
                    .write_db
                    .with_write({
                        let op_id = op.id.clone();
                        move |conn| db_pending_ops_increment_retry_sync(conn, &op_id)
                    })
                    .await;
            }
        }
    }
}

/// Re-dispatch a single pending operation through the action service.
///
/// The operation type determines which action function to call. Params are
/// JSON-encoded operation-specific arguments.
async fn dispatch_pending_op(
    ctx: &ActionContext,
    sync_runtime: Option<&SyncRuntime>,
    account_id: &str,
    operation_type: &str,
    resource_id: &str,
    params_json: &str,
) -> ActionOutcome {
    let params: serde_json::Value = serde_json::from_str(params_json).unwrap_or_default();

    let op = match operation_type {
        "archive" => MailOperation::Archive,
        "trash" => MailOperation::Trash,
        "spam" => {
            let is_spam = params
                .get("isSpam")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            MailOperation::SetSpam { to: is_spam }
        }
        "moveToFolder" => {
            let folder_id = common::typed_ids::FolderId::from(
                params
                    .get("folderId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            );
            let source = params
                .get("sourceFolderId")
                .and_then(serde_json::Value::as_str)
                .map(common::typed_ids::FolderId::from);
            MailOperation::MoveToFolder {
                dest: folder_id,
                source,
            }
        }
        "star" => {
            let starred = params
                .get("starred")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            MailOperation::SetStarred { to: starred }
        }
        "markRead" => {
            let read = params
                .get("read")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            MailOperation::SetRead { to: read }
        }
        "permanentDelete" => MailOperation::PermanentDelete,
        "addLabel" => {
            let label_id = common::typed_ids::LabelId::from(
                params
                    .get("labelId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            );
            MailOperation::AddLabel { label_id }
        }
        "removeLabel" => {
            let label_id = common::typed_ids::LabelId::from(
                params
                    .get("labelId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            );
            MailOperation::RemoveLabel { label_id }
        }
        "applyLabelGroup" => {
            let group_id = common::typed_ids::LabelGroupId(
                params
                    .get("groupId")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(0),
            );
            MailOperation::ApplyLabelGroup { group_id }
        }
        "removeLabelGroup" => {
            let group_id = common::typed_ids::LabelGroupId(
                params
                    .get("groupId")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(0),
            );
            MailOperation::RemoveLabelGroup { group_id }
        }
        other => {
            log::warn!("[pending_ops] Unknown operation type: {other}");
            return ActionOutcome::Failed {
                error: ActionError::invalid_state(format!("Unknown operation type: {other}")),
            };
        }
    };
    let mut outcomes = super::batch_execute(
        ctx,
        sync_runtime,
        vec![(account_id.to_string(), resource_id.to_string(), op)],
    )
    .await;
    outcomes.pop().unwrap_or_else(|| ActionOutcome::Failed {
        error: ActionError::invalid_state("pending op dispatch returned no outcome"),
    })
}

/// Recover from crash - reset stale 'executing' ops to 'pending'.
/// Also resurface stale 'sending' drafts as 'failed'.
/// Call once at app boot.
pub async fn recover_on_boot(ctx: &ActionContext) {
    // 1. Reset stranded executing operations
    match ctx
        .write_db
        .with_write(|conn| db_pending_ops_recover_executing_sync(conn))
        .await
    {
        Ok(count) if count > 0 => {
            log::info!("[pending_ops] Recovered {count} stranded operations on boot");
        }
        Err(e) => {
            log::warn!("[pending_ops] Failed to recover stranded operations: {e}");
        }
        _ => {}
    }

    sweep_stale_label_intents(ctx).await;

    // 2. Resurface stale 'sending' drafts as 'failed'
    let db = ctx.write_db.clone();
    let result = db
        .with_write(|conn| db::db::queries_extra::mark_sending_drafts_failed(conn))
        .await;

    match result {
        Ok(count) if count > 0 => {
            log::info!("[pending_ops] Recovered {count} stale 'sending' drafts on boot");
        }
        Err(e) => {
            log::warn!("[pending_ops] Failed to recover sending drafts: {e}");
        }
        _ => {}
    }
}
