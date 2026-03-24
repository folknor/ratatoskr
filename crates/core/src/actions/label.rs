use ratatoskr_provider_utils::ops::ProviderOps;
use ratatoskr_provider_utils::types::ProviderCtx;

use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use super::pending::enqueue_if_retryable;
use super::provider::create_provider;
use crate::progress::NoopProgressReporter;

/// Local DB mutation for add-label: look up label metadata + insert (idempotent).
/// Returns `(label_name, label_kind)` for provider routing.
async fn add_label_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    label_id: &str,
) -> Result<(String, String), ActionError> {
    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let lid = label_id.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| ActionError::db(format!("db lock: {e}")))?;

        let (label_name, label_kind) = conn
            .query_row(
                "SELECT name, label_kind FROM labels \
                 WHERE id = ?1 AND account_id = ?2 LIMIT 1",
                rusqlite::params![lid, aid],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    ActionError::not_found("label not found for this account")
                }
                other => ActionError::db(format!("label lookup: {other}")),
            })?;

        crate::email_actions::insert_label(&conn, &aid, &tid, &lid)
            .map_err(ActionError::db)?;

        Ok((label_name, label_kind))
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r)
}

/// Provider dispatch for add-label (assumes local mutation already applied).
async fn add_label_dispatch(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    label_id: &str,
    label_name: &str,
    label_kind: &str,
) -> ActionOutcome {
    let mlog = MutationLog::begin("add_label", account_id, thread_id);
    let params_json = serde_json::json!({"labelId": label_id}).to_string();

    let provider_ctx = ProviderCtx {
        account_id,
        db: &ctx.db,
        body_store: &ctx.body_store,
        inline_images: &ctx.inline_images,
        search: &ctx.search,
        progress: &NoopProgressReporter,
    };

    let result = if label_kind == "tag" {
        provider.apply_category(&provider_ctx, thread_id, label_name).await
    } else {
        provider.add_tag(&provider_ctx, thread_id, label_id).await
    };

    let outcome = match result {
        Ok(()) => ActionOutcome::Success,
        Err(e) => {
            let msg = e.to_string();
            ActionOutcome::LocalOnly { reason: ActionError::remote(msg), retryable: true }
        }
    };
    enqueue_if_retryable(ctx, &outcome, account_id, "addLabel", thread_id, &params_json).await;
    mlog.emit(&outcome);
    outcome
}

/// Apply a label to a single thread.
pub async fn add_label(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    label_id: &str,
) -> ActionOutcome {
    let mlog = MutationLog::begin("add_label", account_id, thread_id);
    let params_json = serde_json::json!({"labelId": label_id}).to_string();

    let (label_name, label_kind) = match add_label_local(ctx, account_id, thread_id, label_id).await {
        Ok(info) => info,
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
    };

    match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(provider) => {
            add_label_dispatch(ctx, &*provider, account_id, thread_id, label_id, &label_name, &label_kind).await
        }
        Err(e) => {
            let outcome = ActionOutcome::LocalOnly { reason: ActionError::remote(e), retryable: true };
            enqueue_if_retryable(ctx, &outcome, account_id, "addLabel", thread_id, &params_json).await;
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
    label_id: &str,
) -> ActionOutcome {
    let mlog = MutationLog::begin("add_label", account_id, thread_id);

    let (label_name, label_kind) = match add_label_local(ctx, account_id, thread_id, label_id).await {
        Ok(info) => info,
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
    };

    add_label_dispatch(ctx, provider, account_id, thread_id, label_id, &label_name, &label_kind).await
}

/// Local DB mutation for remove-label: look up label metadata + remove (idempotent).
/// Returns `(label_name, label_kind)` for provider routing.
async fn remove_label_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    label_id: &str,
) -> Result<(String, String), ActionError> {
    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let lid = label_id.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| ActionError::db(format!("db lock: {e}")))?;

        let (label_name, label_kind) = conn
            .query_row(
                "SELECT name, label_kind FROM labels \
                 WHERE id = ?1 AND account_id = ?2 LIMIT 1",
                rusqlite::params![lid, aid],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    ActionError::not_found("label not found for this account")
                }
                other => ActionError::db(format!("label lookup: {other}")),
            })?;

        crate::email_actions::remove_label(&conn, &aid, &tid, &lid)
            .map_err(ActionError::db)?;

        Ok((label_name, label_kind))
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
    label_id: &str,
    label_name: &str,
    label_kind: &str,
) -> ActionOutcome {
    let mlog = MutationLog::begin("remove_label", account_id, thread_id);
    let params_json = serde_json::json!({"labelId": label_id}).to_string();

    let provider_ctx = ProviderCtx {
        account_id,
        db: &ctx.db,
        body_store: &ctx.body_store,
        inline_images: &ctx.inline_images,
        search: &ctx.search,
        progress: &NoopProgressReporter,
    };

    let result = if label_kind == "tag" {
        provider.remove_category(&provider_ctx, thread_id, label_name).await
    } else {
        provider.remove_tag(&provider_ctx, thread_id, label_id).await
    };

    let outcome = match result {
        Ok(()) => ActionOutcome::Success,
        Err(e) => {
            let msg = e.to_string();
            ActionOutcome::LocalOnly { reason: ActionError::remote(msg), retryable: true }
        }
    };
    enqueue_if_retryable(ctx, &outcome, account_id, "removeLabel", thread_id, &params_json).await;
    mlog.emit(&outcome);
    outcome
}

/// Remove a label from a single thread.
pub async fn remove_label(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    label_id: &str,
) -> ActionOutcome {
    let mlog = MutationLog::begin("remove_label", account_id, thread_id);
    let params_json = serde_json::json!({"labelId": label_id}).to_string();

    let (label_name, label_kind) = match remove_label_local(ctx, account_id, thread_id, label_id).await {
        Ok(info) => info,
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
    };

    match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(provider) => {
            remove_label_dispatch(ctx, &*provider, account_id, thread_id, label_id, &label_name, &label_kind).await
        }
        Err(e) => {
            let outcome = ActionOutcome::LocalOnly { reason: ActionError::remote(e), retryable: true };
            enqueue_if_retryable(ctx, &outcome, account_id, "removeLabel", thread_id, &params_json).await;
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
    label_id: &str,
) -> ActionOutcome {
    let mlog = MutationLog::begin("remove_label", account_id, thread_id);

    let (label_name, label_kind) = match remove_label_local(ctx, account_id, thread_id, label_id).await {
        Ok(info) => info,
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
    };

    remove_label_dispatch(ctx, provider, account_id, thread_id, label_id, &label_name, &label_kind).await
}
