use common::ops::ProviderOps;
use common::typed_ids::LabelId;
use common::types::ActionProviderCtx;

use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use super::pending::enqueue_if_retryable;
use super::provider::create_provider;
use db::progress::NoopProgressReporter;

/// Local DB mutation for add-label: validate label exists and is a label row, then
/// insert into `thread_labels` (idempotent).
///
/// Folder rows are rejected - they use move operations, not add/remove.
pub(crate) async fn add_label_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
) -> Result<(), ActionError> {
    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let lid = label_id.as_str().to_string();
    tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn
            .lock()
            .map_err(|e| ActionError::db(format!("db lock: {e}")))?;

        let label_kind = match db::db::queries_extra::action_helpers::get_label_kind_sync(
            &conn, &lid, &aid,
        )
        .map_err(|e| ActionError::db(format!("label lookup: {e}")))?
        {
            Some(kind) => kind,
            None => ensure_prefixed_tag_label(&conn, &aid, &lid)
                .transpose()
                .map_err(ActionError::db)?
                .ok_or_else(|| ActionError::not_found("label not found for this account"))?,
        };

        if label_kind != "tag" {
            return Err(ActionError::invalid_state(
                "folder rows use move operations, not add/remove",
            ));
        }

        if let Some(opposite) = opposite_importance_label(&lid) {
            db::db::queries_extra::remove_label(&conn, &aid, &tid, opposite)
                .map_err(ActionError::db)?;
        }
        db::db::queries_extra::insert_label(&conn, &aid, &tid, &lid).map_err(ActionError::db)?;

        Ok(())
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r)
}

fn ensure_prefixed_tag_label(
    conn: &rusqlite::Connection,
    account_id: &str,
    label_id: &str,
) -> Option<Result<String, String>> {
    let (name, label_type, sort_order) = if let Some(keyword) = label_id.strip_prefix("kw:") {
        (keyword.to_string(), "user", None)
    } else if let Some(category) = label_id.strip_prefix("cat:") {
        (category.to_string(), "user", None)
    } else if label_id == "importance:high" {
        ("High importance".to_string(), "system", Some(10_000))
    } else if label_id == "importance:low" {
        ("Low importance".to_string(), "system", Some(10_001))
    } else {
        return None;
    };

    Some(
        conn.execute(
            "INSERT OR IGNORE INTO labels \
             (id, account_id, name, type, label_kind, sort_order) \
             VALUES (?1, ?2, ?3, ?4, 'tag', COALESCE(?5, 0))",
            rusqlite::params![label_id, account_id, name, label_type, sort_order],
        )
        .map(|_| "tag".to_string())
        .map_err(|e| format!("ensure prefixed tag label: {e}")),
    )
}

/// Provider dispatch for add-label (assumes local mutation already applied).
async fn add_label_dispatch(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
) -> ActionOutcome {
    let mlog = MutationLog::begin("add_label", account_id, thread_id);
    let params_json = serde_json::json!({"labelId": label_id.as_str()}).to_string();

    let provider_ctx = ActionProviderCtx {
        account_id,
        db: &ctx.db,
        progress: &NoopProgressReporter,
    };

    let result = provider.add_label(&provider_ctx, thread_id, label_id).await;

    let outcome = match result {
        Ok(()) => ActionOutcome::Success,
        Err(e) => {
            let msg = e.to_string();
            ActionOutcome::LocalOnly {
                reason: ActionError::remote(msg),
                retryable: true,
            }
        }
    };
    enqueue_if_retryable(
        ctx,
        &outcome,
        account_id,
        "addLabel",
        thread_id,
        &params_json,
    )
    .await;
    mlog.emit(&outcome);
    outcome
}

/// Apply a label to a single thread.
pub async fn add_label(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
) -> ActionOutcome {
    let mlog = MutationLog::begin("add_label", account_id, thread_id);
    let params_json = serde_json::json!({"labelId": label_id.as_str()}).to_string();

    if let Err(e) = add_label_local(ctx, account_id, thread_id, label_id).await {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(provider) => add_label_dispatch(ctx, &*provider, account_id, thread_id, label_id).await,
        Err(e) => {
            let outcome = ActionOutcome::LocalOnly {
                reason: ActionError::remote(e),
                retryable: true,
            };
            enqueue_if_retryable(
                ctx,
                &outcome,
                account_id,
                "addLabel",
                thread_id,
                &params_json,
            )
            .await;
            mlog.emit(&outcome);
            outcome
        }
    }
}

/// Add label with a pre-constructed provider (for batch reuse).
pub(crate) async fn add_label_with_provider(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
) -> ActionOutcome {
    let mlog = MutationLog::begin("add_label", account_id, thread_id);

    if let Err(e) = add_label_local(ctx, account_id, thread_id, label_id).await {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    add_label_dispatch(ctx, provider, account_id, thread_id, label_id).await
}

/// Local DB mutation for remove-label: validate label exists and is a label row, then
/// delete from `thread_labels` (idempotent).
///
/// Container labels (folders) are rejected - they use move operations, not add/remove.
pub(crate) async fn remove_label_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
) -> Result<(), ActionError> {
    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let lid = label_id.as_str().to_string();
    tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn
            .lock()
            .map_err(|e| ActionError::db(format!("db lock: {e}")))?;

        let label_kind = db::db::queries_extra::action_helpers::get_label_kind_sync(
            &conn, &lid, &aid,
        )
        .map_err(|e| ActionError::db(format!("label lookup: {e}")))?
        .ok_or_else(|| ActionError::not_found("label not found for this account"))?;

        if label_kind != "tag" {
            return Err(ActionError::invalid_state(
                "folder rows use move operations, not add/remove",
            ));
        }

        db::db::queries_extra::remove_label(&conn, &aid, &tid, &lid).map_err(ActionError::db)?;

        Ok(())
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r)
}

/// Provider dispatch for remove-label (assumes local mutation already applied).
async fn remove_label_dispatch(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
) -> ActionOutcome {
    let mlog = MutationLog::begin("remove_label", account_id, thread_id);
    let params_json = serde_json::json!({"labelId": label_id.as_str()}).to_string();

    let provider_ctx = ActionProviderCtx {
        account_id,
        db: &ctx.db,
        progress: &NoopProgressReporter,
    };

    let result = provider
        .remove_label(&provider_ctx, thread_id, label_id)
        .await;

    let outcome = match result {
        Ok(()) => ActionOutcome::Success,
        Err(e) => {
            let msg = e.to_string();
            ActionOutcome::LocalOnly {
                reason: ActionError::remote(msg),
                retryable: true,
            }
        }
    };
    enqueue_if_retryable(
        ctx,
        &outcome,
        account_id,
        "removeLabel",
        thread_id,
        &params_json,
    )
    .await;
    mlog.emit(&outcome);
    outcome
}

/// Remove a label from a single thread.
pub async fn remove_label(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
) -> ActionOutcome {
    let mlog = MutationLog::begin("remove_label", account_id, thread_id);
    let params_json = serde_json::json!({"labelId": label_id.as_str()}).to_string();

    if let Err(e) = remove_label_local(ctx, account_id, thread_id, label_id).await {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(provider) => {
            remove_label_dispatch(ctx, &*provider, account_id, thread_id, label_id).await
        }
        Err(e) => {
            let outcome = ActionOutcome::LocalOnly {
                reason: ActionError::remote(e),
                retryable: true,
            };
            enqueue_if_retryable(
                ctx,
                &outcome,
                account_id,
                "removeLabel",
                thread_id,
                &params_json,
            )
            .await;
            mlog.emit(&outcome);
            outcome
        }
    }
}

/// Remove label with a pre-constructed provider (for batch reuse).
pub(crate) async fn remove_label_with_provider(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
) -> ActionOutcome {
    let mlog = MutationLog::begin("remove_label", account_id, thread_id);

    if let Err(e) = remove_label_local(ctx, account_id, thread_id, label_id).await {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    remove_label_dispatch(ctx, provider, account_id, thread_id, label_id).await
}

fn opposite_importance_label(label_id: &str) -> Option<&'static str> {
    match label_id {
        "importance:high" => Some("importance:low"),
        "importance:low" => Some("importance:high"),
        _ => None,
    }
}
