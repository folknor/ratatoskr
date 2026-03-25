//! Pending-action queue integration.
//!
//! When an email action returns `LocalOnly { retryable: true }`, the action
//! enqueues a pending operation. A periodic worker re-dispatches pending ops
//! through the action service.

use super::context::ActionContext;
use super::outcome::{ActionError, ActionOutcome};
use crate::db::pending_ops::{
    db_pending_ops_delete, db_pending_ops_enqueue, db_pending_ops_get,
    db_pending_ops_increment_retry, db_pending_ops_recover_executing,
    db_pending_ops_update_status,
};

/// Per-action-type retry policy.
struct RetryPolicy {
    max_retries: i64,
}

/// Look up retry policy by operation type.
///
/// Folder-level actions (archive, trash, spam, move, delete) get the most
/// aggressive retry — silent divergence is the #1 user-visible bug.
/// Label actions get moderate retry. Flag actions (star, read) get lighter
/// retry since sync reconciles them within 5 minutes.
fn retry_policy(operation_type: &str) -> RetryPolicy {
    match operation_type {
        "archive" | "trash" | "spam" | "moveToFolder" | "permanentDelete" => {
            RetryPolicy { max_retries: 10 }
        }
        "addLabel" | "removeLabel" => RetryPolicy { max_retries: 7 },
        "star" | "markRead" => RetryPolicy { max_retries: 5 },
        _ => RetryPolicy { max_retries: 5 },
    }
}

/// Enqueue a pending operation if the outcome is retryable LocalOnly.
///
/// Called by action functions after determining the outcome. Only enqueues
/// if `retryable` is true AND `reason.is_retryable()` (policy + capability).
///
/// Deduplication is atomic: `db_pending_ops_enqueue` replaces any existing
/// pending/executing op for the same (account_id, resource_id, operation_type)
/// in a single connection hold. This handles both exact duplicates and
/// directional updates (e.g., spam→unspam replaces stale params).
pub async fn enqueue_if_retryable(
    ctx: &ActionContext,
    outcome: &ActionOutcome,
    account_id: &str,
    operation_type: &str,
    resource_id: &str,
    params_json: &str,
) {
    // Suppressed during retry dispatch to prevent duplicate enqueue.
    if ctx.suppress_pending_enqueue {
        return;
    }

    if let ActionOutcome::LocalOnly {
        retryable: true,
        reason,
    } = outcome
    {
        if !reason.is_retryable() {
            return;
        }

        let policy = retry_policy(operation_type);
        let op_id = uuid::Uuid::new_v4().to_string();
        if let Err(e) = db_pending_ops_enqueue(
            &ctx.db,
            op_id,
            account_id.to_string(),
            operation_type.to_string(),
            resource_id.to_string(),
            params_json.to_string(),
            policy.max_retries,
        )
        .await
        {
            log::warn!("[pending_ops] Failed to enqueue {operation_type} for {resource_id}: {e}");
        }
    }
}

/// Process pending operations from the queue.
///
/// Called periodically (e.g., on SyncTick). Fetches pending ops, groups by
/// account (one provider per account), and dispatches sequentially.
/// Respects the in-flight guard — skips threads with active mutations.
pub async fn process_pending_ops(ctx: &ActionContext) {
    let ops = match db_pending_ops_get(&ctx.db, None, Some(20)).await {
        Ok(ops) => ops,
        Err(e) => {
            log::warn!("[pending_ops] Failed to fetch pending ops: {e}");
            return;
        }
    };

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
    for (account_id, account_ops) in groups {
        process_account_group(ctx, &retry_ctx, &account_id, account_ops).await;
    }
}

/// Process one account's pending ops with a shared provider.
async fn process_account_group(
    ctx: &ActionContext,
    retry_ctx: &ActionContext,
    account_id: &str,
    ops: Vec<crate::db::pending_ops::PendingOperation>,
) {
    use super::provider::create_provider;

    let provider = match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(p) => Some(p),
        Err(e) => {
            log::warn!("[pending_ops] Provider creation failed for {account_id}: {e}");
            None
        }
    };

    for op in ops {
        // In-flight guard: acquire to block concurrent user mutations,
        // or skip if already in flight from a user action.
        let _guard = match ctx.try_acquire_flight(&op.account_id, &op.resource_id) {
            Some(g) => g,
            None => {
                log::debug!(
                    "[pending_ops] Skipping {} for {}/{} — in flight",
                    op.operation_type, op.account_id, op.resource_id
                );
                continue; // Leave pending, don't increment retry
            }
        };

        // Mark as executing
        if let Err(e) = db_pending_ops_update_status(
            &ctx.db,
            op.id.clone(),
            "executing".to_string(),
            None,
        )
        .await
        {
            log::warn!("[pending_ops] Failed to mark op {} executing: {e}", op.id);
            continue;
        }

        let outcome = if let Some(ref provider) = provider {
            dispatch_pending_op_with_provider(
                retry_ctx, &**provider, account_id, &op.operation_type, &op.resource_id, &op.params,
            )
            .await
        } else {
            // No provider — use public wrappers (which will also fail to create provider)
            dispatch_pending_op(
                retry_ctx, &op.account_id, &op.operation_type, &op.resource_id, &op.params,
            )
            .await
        };

        match outcome {
            ActionOutcome::Success | ActionOutcome::NoOp => {
                let _ = db_pending_ops_delete(&ctx.db, op.id.clone()).await;
                log::info!(
                    "[pending_ops] Completed {} for {}/{}",
                    op.operation_type, op.account_id, op.resource_id
                );
            }
            ActionOutcome::LocalOnly { reason, .. } | ActionOutcome::Failed { error: reason } => {
                log::warn!(
                    "[pending_ops] Retry failed for {} {}/{}: {reason}",
                    op.operation_type, op.account_id, op.resource_id
                );
                let _ = db_pending_ops_increment_retry(&ctx.db, op.id.clone()).await;
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
    account_id: &str,
    operation_type: &str,
    resource_id: &str,
    params_json: &str,
) -> ActionOutcome {
    let params: serde_json::Value =
        serde_json::from_str(params_json).unwrap_or_default();

    match operation_type {
        "archive" => super::archive::archive(ctx, account_id, resource_id).await,
        "trash" => super::trash::trash(ctx, account_id, resource_id).await,
        "spam" => {
            let is_spam = params
                .get("isSpam")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            super::spam::spam(ctx, account_id, resource_id, is_spam).await
        }
        "moveToFolder" => {
            let folder_id = params
                .get("folderId")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let source = params
                .get("sourceLabelId")
                .and_then(serde_json::Value::as_str);
            super::move_to_folder::move_to_folder(ctx, account_id, resource_id, folder_id, source)
                .await
        }
        "star" => {
            let starred = params
                .get("starred")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            super::star::star(ctx, account_id, resource_id, starred).await
        }
        "markRead" => {
            let read = params
                .get("read")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            super::mark_read::mark_read(ctx, account_id, resource_id, read).await
        }
        "permanentDelete" => {
            super::permanent_delete::permanent_delete(ctx, account_id, resource_id).await
        }
        "addLabel" => {
            let label_id = params
                .get("labelId")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            super::label::add_label(ctx, account_id, resource_id, label_id).await
        }
        "removeLabel" => {
            let label_id = params
                .get("labelId")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            super::label::remove_label(ctx, account_id, resource_id, label_id).await
        }
        other => {
            log::warn!("[pending_ops] Unknown operation type: {other}");
            ActionOutcome::Failed {
                error: ActionError::invalid_state(format!("Unknown operation type: {other}")),
            }
        }
    }
}

/// Re-dispatch a pending op using a pre-constructed provider (for provider reuse).
async fn dispatch_pending_op_with_provider(
    ctx: &ActionContext,
    provider: &dyn ratatoskr_provider_utils::ops::ProviderOps,
    account_id: &str,
    operation_type: &str,
    resource_id: &str,
    params_json: &str,
) -> ActionOutcome {
    let params: serde_json::Value =
        serde_json::from_str(params_json).unwrap_or_default();

    match operation_type {
        "archive" => super::archive::archive_with_provider(ctx, provider, account_id, resource_id).await,
        "trash" => super::trash::trash_with_provider(ctx, provider, account_id, resource_id).await,
        "spam" => {
            let is_spam = params.get("isSpam").and_then(serde_json::Value::as_bool).unwrap_or(true);
            super::spam::spam_with_provider(ctx, provider, account_id, resource_id, is_spam).await
        }
        "moveToFolder" => {
            let folder_id = params.get("folderId").and_then(serde_json::Value::as_str).unwrap_or("");
            let source = params.get("sourceLabelId").and_then(serde_json::Value::as_str);
            super::move_to_folder::move_to_folder_with_provider(ctx, provider, account_id, resource_id, folder_id, source).await
        }
        "star" => {
            let starred = params.get("starred").and_then(serde_json::Value::as_bool).unwrap_or(true);
            super::star::star_with_provider(ctx, provider, account_id, resource_id, starred).await
        }
        "markRead" => {
            let read = params.get("read").and_then(serde_json::Value::as_bool).unwrap_or(true);
            super::mark_read::mark_read_with_provider(ctx, provider, account_id, resource_id, read).await
        }
        "permanentDelete" => {
            super::permanent_delete::permanent_delete_with_provider(ctx, provider, account_id, resource_id).await
        }
        "addLabel" => {
            let label_id = params.get("labelId").and_then(serde_json::Value::as_str).unwrap_or("");
            super::label::add_label_with_provider(ctx, provider, account_id, resource_id, label_id).await
        }
        "removeLabel" => {
            let label_id = params.get("labelId").and_then(serde_json::Value::as_str).unwrap_or("");
            super::label::remove_label_with_provider(ctx, provider, account_id, resource_id, label_id).await
        }
        other => {
            log::warn!("[pending_ops] Unknown operation type: {other}");
            ActionOutcome::Failed {
                error: ActionError::invalid_state(format!("Unknown operation type: {other}")),
            }
        }
    }
}

/// Recover from crash — reset stale 'executing' ops to 'pending'.
/// Also resurface stale 'sending' drafts as 'failed'.
/// Call once at app boot.
pub async fn recover_on_boot(ctx: &ActionContext) {
    // 1. Reset stranded executing operations
    match db_pending_ops_recover_executing(&ctx.db).await {
        Ok(count) if count > 0 => {
            log::info!("[pending_ops] Recovered {count} stranded operations on boot");
        }
        Err(e) => {
            log::warn!("[pending_ops] Failed to recover stranded operations: {e}");
        }
        _ => {}
    }

    // 2. Resurface stale 'sending' drafts as 'failed'
    let db = ctx.db.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        let count = conn
            .execute(
                "UPDATE local_drafts SET sync_status = 'failed' WHERE sync_status = 'sending'",
                [],
            )
            .map_err(|e| format!("recover sending drafts: {e}"))?;
        Ok::<_, String>(count)
    })
    .await;

    match result {
        Ok(Ok(count)) if count > 0 => {
            log::info!("[pending_ops] Recovered {count} stale 'sending' drafts on boot");
        }
        Ok(Err(e)) => {
            log::warn!("[pending_ops] Failed to recover sending drafts: {e}");
        }
        Err(e) => {
            log::warn!("[pending_ops] spawn_blocking failed for draft recovery: {e}");
        }
        _ => {}
    }
}
