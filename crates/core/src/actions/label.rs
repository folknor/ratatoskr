use common::ops::ProviderOps;
use common::typed_ids::TagId;
use common::types::ProviderCtx;

use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use super::pending::enqueue_if_retryable;
use super::provider::create_provider;
use crate::progress::NoopProgressReporter;

/// Local DB mutation for add-label: validate label exists and is a tag, then
/// insert into `thread_labels` (idempotent).
///
/// Container labels (folders) are rejected - they use move operations, not add/remove.
pub(crate) async fn add_label_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    label_id: &TagId,
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

        let label_kind = crate::db::queries_extra::action_helpers::get_label_kind_sync(
            &conn, &lid, &aid,
        )
        .map_err(|e| ActionError::db(format!("label lookup: {e}")))?
        .ok_or_else(|| ActionError::not_found("label not found for this account"))?;

        if label_kind != "tag" {
            return Err(ActionError::invalid_state(
                "container labels use move operations, not add/remove",
            ));
        }

        crate::email_actions::insert_label(&conn, &aid, &tid, &lid).map_err(ActionError::db)?;

        Ok(())
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
    label_id: &TagId,
) -> ActionOutcome {
    let mlog = MutationLog::begin("add_label", account_id, thread_id);
    let params_json = serde_json::json!({"labelId": label_id.as_str()}).to_string();

    let provider_ctx = ProviderCtx {
        account_id,
        db: &ctx.db,
        body_store: &ctx.body_store,
        inline_images: &ctx.inline_images,
        search: &ctx.search,
        progress: &NoopProgressReporter,
    };

    let result = provider.add_tag(&provider_ctx, thread_id, label_id).await;

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
    label_id: &TagId,
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
    label_id: &TagId,
) -> ActionOutcome {
    let mlog = MutationLog::begin("add_label", account_id, thread_id);

    if let Err(e) = add_label_local(ctx, account_id, thread_id, label_id).await {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    add_label_dispatch(ctx, provider, account_id, thread_id, label_id).await
}

/// Local DB mutation for remove-label: validate label exists and is a tag, then
/// delete from `thread_labels` (idempotent).
///
/// Container labels (folders) are rejected - they use move operations, not add/remove.
pub(crate) async fn remove_label_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    label_id: &TagId,
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

        let label_kind = crate::db::queries_extra::action_helpers::get_label_kind_sync(
            &conn, &lid, &aid,
        )
        .map_err(|e| ActionError::db(format!("label lookup: {e}")))?
        .ok_or_else(|| ActionError::not_found("label not found for this account"))?;

        if label_kind != "tag" {
            return Err(ActionError::invalid_state(
                "container labels use move operations, not add/remove",
            ));
        }

        crate::email_actions::remove_label(&conn, &aid, &tid, &lid).map_err(ActionError::db)?;

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
    label_id: &TagId,
) -> ActionOutcome {
    let mlog = MutationLog::begin("remove_label", account_id, thread_id);
    let params_json = serde_json::json!({"labelId": label_id.as_str()}).to_string();

    let provider_ctx = ProviderCtx {
        account_id,
        db: &ctx.db,
        body_store: &ctx.body_store,
        inline_images: &ctx.inline_images,
        search: &ctx.search,
        progress: &NoopProgressReporter,
    };

    let result = provider
        .remove_tag(&provider_ctx, thread_id, label_id)
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
    label_id: &TagId,
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
    label_id: &TagId,
) -> ActionOutcome {
    let mlog = MutationLog::begin("remove_label", account_id, thread_id);

    if let Err(e) = remove_label_local(ctx, account_id, thread_id, label_id).await {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    remove_label_dispatch(ctx, provider, account_id, thread_id, label_id).await
}
