use common::ops::ProviderOps;
use common::typed_ids::{LabelGroupId, LabelId};

use super::context::ActionContext;
use super::label;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use super::pending::enqueue_if_retryable;
use super::provider::create_provider;

pub(crate) async fn apply_label_group_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
) -> Result<Vec<LabelId>, ActionError> {
    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn
            .lock()
            .map_err(|e| ActionError::db(format!("db lock: {e}")))?;
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM label_groups WHERE id = ?1",
                rusqlite::params![group_id.as_i64()],
                |row| row.get(0),
            )
            .map_err(|e| ActionError::db(format!("label group lookup: {e}")))?;
        if exists == 0 {
            return Err(ActionError::not_found("label group not found"));
        }
        conn.execute(
            "INSERT OR IGNORE INTO thread_label_groups (account_id, thread_id, group_id)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![aid, tid, group_id.as_i64()],
        )
        .map_err(|e| ActionError::db(format!("insert thread label group: {e}")))?;
        read_group_member_labels(&conn, &aid, group_id)
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r)
}

pub(crate) async fn remove_label_group_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
) -> Result<Vec<LabelId>, ActionError> {
    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn
            .lock()
            .map_err(|e| ActionError::db(format!("db lock: {e}")))?;
        let labels = read_applied_group_member_labels(&conn, &aid, &tid, group_id)?;
        conn.execute(
            "DELETE FROM thread_label_groups
             WHERE account_id = ?1 AND thread_id = ?2 AND group_id = ?3",
            rusqlite::params![aid, tid, group_id.as_i64()],
        )
        .map_err(|e| ActionError::db(format!("delete thread label group: {e}")))?;
        Ok(labels)
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r)
}

pub(crate) async fn apply_label_group_with_provider(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
) -> ActionOutcome {
    let mlog = MutationLog::begin("apply_label_group", account_id, thread_id);
    let labels = match apply_label_group_local(ctx, account_id, thread_id, group_id).await {
        Ok(labels) => labels,
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
    };
    let outcome = dispatch_member_ops(ctx, provider, account_id, thread_id, labels, true).await;
    mlog.emit(&outcome);
    outcome
}

pub async fn apply_label_group(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
) -> ActionOutcome {
    let mlog = MutationLog::begin("apply_label_group", account_id, thread_id);
    let params_json = serde_json::json!({"groupId": group_id.as_i64()}).to_string();
    match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(provider) => {
            let outcome =
                apply_label_group_with_provider(ctx, &*provider, account_id, thread_id, group_id)
                    .await;
            mlog.emit(&outcome);
            outcome
        }
        Err(e) => {
            if let Err(e) = apply_label_group_local(ctx, account_id, thread_id, group_id).await {
                let outcome = ActionOutcome::Failed { error: e };
                mlog.emit(&outcome);
                return outcome;
            }
            let outcome = ActionOutcome::LocalOnly {
                reason: ActionError::remote(e),
                retryable: true,
            };
            enqueue_if_retryable(
                ctx,
                &outcome,
                account_id,
                "applyLabelGroup",
                thread_id,
                &params_json,
            )
            .await;
            mlog.emit(&outcome);
            outcome
        }
    }
}

pub(crate) async fn remove_label_group_with_provider(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
) -> ActionOutcome {
    let mlog = MutationLog::begin("remove_label_group", account_id, thread_id);
    let labels = match remove_label_group_local(ctx, account_id, thread_id, group_id).await {
        Ok(labels) => labels,
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
    };
    let outcome = dispatch_member_ops(ctx, provider, account_id, thread_id, labels, false).await;
    mlog.emit(&outcome);
    outcome
}

pub async fn remove_label_group(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
) -> ActionOutcome {
    let mlog = MutationLog::begin("remove_label_group", account_id, thread_id);
    let params_json = serde_json::json!({"groupId": group_id.as_i64()}).to_string();
    match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(provider) => {
            let outcome =
                remove_label_group_with_provider(ctx, &*provider, account_id, thread_id, group_id)
                    .await;
            mlog.emit(&outcome);
            outcome
        }
        Err(e) => {
            if let Err(e) = remove_label_group_local(ctx, account_id, thread_id, group_id).await {
                let outcome = ActionOutcome::Failed { error: e };
                mlog.emit(&outcome);
                return outcome;
            }
            let outcome = ActionOutcome::LocalOnly {
                reason: ActionError::remote(e),
                retryable: true,
            };
            enqueue_if_retryable(
                ctx,
                &outcome,
                account_id,
                "removeLabelGroup",
                thread_id,
                &params_json,
            )
            .await;
            mlog.emit(&outcome);
            outcome
        }
    }
}

async fn dispatch_member_ops(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    labels: Vec<LabelId>,
    apply: bool,
) -> ActionOutcome {
    let mut saw_local_only = false;
    for label_id in labels {
        let outcome = if apply {
            label::add_label_with_provider(ctx, provider, account_id, thread_id, &label_id).await
        } else {
            label::remove_label_with_provider(ctx, provider, account_id, thread_id, &label_id).await
        };
        match outcome {
            ActionOutcome::Success | ActionOutcome::NoOp => {}
            ActionOutcome::LocalOnly { .. } => saw_local_only = true,
            ActionOutcome::Failed { error } => return ActionOutcome::Failed { error },
        }
    }
    if saw_local_only {
        ActionOutcome::LocalOnly {
            reason: ActionError::remote("one or more label group member writes failed"),
            retryable: true,
        }
    } else {
        ActionOutcome::Success
    }
}

fn read_group_member_labels(
    conn: &rusqlite::Connection,
    account_id: &str,
    group_id: LabelGroupId,
) -> Result<Vec<LabelId>, ActionError> {
    let mut stmt = conn
        .prepare(
            "SELECT label_id FROM label_group_members
             WHERE account_id = ?1 AND group_id = ?2
             ORDER BY label_id",
        )
        .map_err(|e| ActionError::db(format!("prepare group members: {e}")))?;
    stmt.query_map(rusqlite::params![account_id, group_id.as_i64()], |row| {
        Ok(LabelId(row.get::<_, String>(0)?))
    })
    .map_err(|e| ActionError::db(format!("query group members: {e}")))?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| ActionError::db(format!("map group members: {e}")))
}

fn read_applied_group_member_labels(
    conn: &rusqlite::Connection,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
) -> Result<Vec<LabelId>, ActionError> {
    let mut stmt = conn
        .prepare(
            "SELECT tl.label_id
             FROM thread_labels tl
             INNER JOIN label_group_members lgm
               ON lgm.account_id = tl.account_id AND lgm.label_id = tl.label_id
             WHERE tl.account_id = ?1
               AND tl.thread_id = ?2
               AND lgm.group_id = ?3
             ORDER BY tl.label_id",
        )
        .map_err(|e| ActionError::db(format!("prepare applied group labels: {e}")))?;
    stmt.query_map(
        rusqlite::params![account_id, thread_id, group_id.as_i64()],
        |row| Ok(LabelId(row.get::<_, String>(0)?)),
    )
    .map_err(|e| ActionError::db(format!("query applied group labels: {e}")))?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| ActionError::db(format!("map applied group labels: {e}")))
}
