//! Pending-action queue integration.
//!
//! When an email action returns `LocalOnly { retryable: true }`, the action
//! enqueues a pending operation. A periodic worker re-dispatches pending ops
//! through the action service.

use super::context::ActionContext;
use super::outcome::{ActionError, ActionOutcome};
use crate::db::pending_ops::{
    db_pending_ops_delete, db_pending_ops_enqueue, db_pending_ops_get,
    db_pending_ops_increment_retry, db_pending_ops_recover_executing, db_pending_ops_update_status,
};

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
/// Respects the in-flight guard - skips threads with active mutations.
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
                    "[pending_ops] Skipping {} for {}/{} - in flight",
                    op.operation_type,
                    op.account_id,
                    op.resource_id
                );
                continue; // Leave pending, don't increment retry
            }
        };

        // Mark as executing
        if let Err(e) =
            db_pending_ops_update_status(&ctx.db, op.id.clone(), "executing".to_string(), None)
                .await
        {
            log::warn!("[pending_ops] Failed to mark op {} executing: {e}", op.id);
            continue;
        }

        let outcome = if let Some(ref provider) = provider {
            dispatch_pending_op_with_provider(
                retry_ctx,
                &**provider,
                account_id,
                &op.operation_type,
                &op.resource_id,
                &op.params,
            )
            .await
        } else {
            // No provider - use public wrappers (which will also fail to create provider)
            dispatch_pending_op(
                retry_ctx,
                &op.account_id,
                &op.operation_type,
                &op.resource_id,
                &op.params,
            )
            .await
        };

        match outcome {
            ActionOutcome::Success | ActionOutcome::NoOp => {
                let _ = db_pending_ops_delete(&ctx.db, op.id.clone()).await;
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
    let params: serde_json::Value = serde_json::from_str(params_json).unwrap_or_default();

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
            let folder_id = crate::provider::typed_ids::FolderId::from(
                params
                    .get("folderId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            );
            let source = params
                .get("sourceLabelId")
                .and_then(serde_json::Value::as_str)
                .map(crate::provider::typed_ids::FolderId::from);
            super::move_to_folder::move_to_folder(
                ctx,
                account_id,
                resource_id,
                &folder_id,
                source.as_ref(),
            )
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
            let label_id = crate::provider::typed_ids::TagId::from(
                params
                    .get("labelId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            );
            super::label::add_label(ctx, account_id, resource_id, &label_id).await
        }
        "removeLabel" => {
            let label_id = crate::provider::typed_ids::TagId::from(
                params
                    .get("labelId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            );
            super::label::remove_label(ctx, account_id, resource_id, &label_id).await
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
    provider: &dyn crate::provider::ops::ProviderOps,
    account_id: &str,
    operation_type: &str,
    resource_id: &str,
    params_json: &str,
) -> ActionOutcome {
    let params: serde_json::Value = serde_json::from_str(params_json).unwrap_or_default();

    match operation_type {
        "archive" => {
            super::archive::archive_with_provider(ctx, provider, account_id, resource_id).await
        }
        "trash" => super::trash::trash_with_provider(ctx, provider, account_id, resource_id).await,
        "spam" => {
            let is_spam = params
                .get("isSpam")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            super::spam::spam_with_provider(ctx, provider, account_id, resource_id, is_spam).await
        }
        "moveToFolder" => {
            let folder_id = crate::provider::typed_ids::FolderId::from(
                params
                    .get("folderId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            );
            let source = params
                .get("sourceLabelId")
                .and_then(serde_json::Value::as_str)
                .map(crate::provider::typed_ids::FolderId::from);
            super::move_to_folder::move_to_folder_with_provider(
                ctx,
                provider,
                account_id,
                resource_id,
                &folder_id,
                source.as_ref(),
            )
            .await
        }
        "star" => {
            let starred = params
                .get("starred")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            super::star::star_with_provider(ctx, provider, account_id, resource_id, starred).await
        }
        "markRead" => {
            let read = params
                .get("read")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            super::mark_read::mark_read_with_provider(ctx, provider, account_id, resource_id, read)
                .await
        }
        "permanentDelete" => {
            super::permanent_delete::permanent_delete_with_provider(
                ctx,
                provider,
                account_id,
                resource_id,
            )
            .await
        }
        "addLabel" => {
            let label_id = crate::provider::typed_ids::TagId::from(
                params
                    .get("labelId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            );
            super::label::add_label_with_provider(ctx, provider, account_id, resource_id, &label_id)
                .await
        }
        "removeLabel" => {
            let label_id = crate::provider::typed_ids::TagId::from(
                params
                    .get("labelId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            );
            super::label::remove_label_with_provider(
                ctx,
                provider,
                account_id,
                resource_id,
                &label_id,
            )
            .await
        }
        other => {
            log::warn!("[pending_ops] Unknown operation type: {other}");
            ActionOutcome::Failed {
                error: ActionError::invalid_state(format!("Unknown operation type: {other}")),
            }
        }
    }
}

/// Synchronous DB-only boot recovery for the Service.
///
/// Resets stranded `pending_operations.status = 'executing'` rows to
/// 'pending' and stale `local_drafts.sync_status = 'sending'` rows to
/// 'failed'. Mirrors what `recover_on_boot` does, minus the
/// `ActionContext` plumbing - the Service's boot sequence calls this
/// from inside `tokio::task::spawn_blocking` after opening the DB
/// connection, before any provider/encryption-key state has been
/// constructed.
///
/// `recover_on_boot` continues to exist for Phase 2's relocated periodic
/// drainer (which still wants the rest of `ActionContext`).
pub fn recover_on_boot_db_only(conn: &crate::db::Connection) -> Result<(), String> {
    let pending_count = conn
        .execute(
            "UPDATE pending_operations SET status = 'pending' WHERE status = 'executing'",
            [],
        )
        .map_err(|e| format!("recover executing ops: {e}"))?;
    if pending_count > 0 {
        log::info!(
            "[pending_ops] Recovered {pending_count} stranded executing operations on boot"
        );
    }

    let drafts_count = conn
        .execute(
            "UPDATE local_drafts SET sync_status = 'failed' WHERE sync_status = 'sending'",
            [],
        )
        .map_err(|e| format!("recover sending drafts: {e}"))?;
    if drafts_count > 0 {
        log::info!("[pending_ops] Recovered {drafts_count} stale 'sending' drafts on boot");
    }

    Ok(())
}

/// Recover from crash - reset stale 'executing' ops to 'pending'.
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

#[cfg(test)]
mod recover_on_boot_db_only_tests {
    use super::*;
    use crate::db::Connection;

    /// Minimal schema for the two tables `recover_on_boot_db_only` touches.
    /// Mirrors the relevant columns from `crates/db/src/db/schema/`; we
    /// don't need the full schema for these tests.
    fn make_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            "
            CREATE TABLE pending_operations (
                id TEXT PRIMARY KEY,
                account_id TEXT,
                operation_type TEXT,
                resource_id TEXT,
                params TEXT,
                status TEXT,
                retry_count INTEGER DEFAULT 0,
                max_retries INTEGER DEFAULT 5
            );
            CREATE TABLE local_drafts (
                id TEXT PRIMARY KEY,
                account_id TEXT,
                sync_status TEXT
            );
            ",
        )
        .expect("schema setup");
        conn
    }

    #[test]
    fn recover_on_boot_db_only_resets_executing_pending_ops_to_pending() {
        let conn = make_conn();
        conn.execute(
            "INSERT INTO pending_operations (id, status) VALUES (?1, 'executing')",
            ["op-1"],
        )
        .expect("insert");
        conn.execute(
            "INSERT INTO pending_operations (id, status) VALUES (?1, 'pending')",
            ["op-2"],
        )
        .expect("insert");
        conn.execute(
            "INSERT INTO pending_operations (id, status) VALUES (?1, 'failed')",
            ["op-3"],
        )
        .expect("insert");

        recover_on_boot_db_only(&conn).expect("recovery");

        let executing: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pending_operations WHERE status = 'executing'",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(executing, 0, "no executing rows should remain");
        let pending: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pending_operations WHERE status = 'pending'",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(pending, 2, "executing op-1 should now be pending");
        let failed: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pending_operations WHERE status = 'failed'",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(failed, 1, "non-executing rows should be untouched");
    }

    #[test]
    fn recover_on_boot_db_only_resurfaces_sending_drafts_as_failed() {
        let conn = make_conn();
        conn.execute(
            "INSERT INTO local_drafts (id, sync_status) VALUES (?1, 'sending')",
            ["draft-1"],
        )
        .expect("insert");
        conn.execute(
            "INSERT INTO local_drafts (id, sync_status) VALUES (?1, 'draft')",
            ["draft-2"],
        )
        .expect("insert");

        recover_on_boot_db_only(&conn).expect("recovery");

        let sending: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM local_drafts WHERE sync_status = 'sending'",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(sending, 0, "no sending drafts should remain");
        let failed: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM local_drafts WHERE sync_status = 'failed'",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(failed, 1, "previously-sending draft should be failed");
        let untouched: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM local_drafts WHERE sync_status = 'draft'",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(untouched, 1, "non-sending drafts should be untouched");
    }

    #[test]
    fn recover_on_boot_db_only_is_noop_on_empty_db() {
        let conn = make_conn();
        recover_on_boot_db_only(&conn).expect("recovery on empty");
    }
}
