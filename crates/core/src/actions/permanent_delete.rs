use common::ops::ProviderOps;
use common::types::ProviderCtx;

use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use super::pending::enqueue_if_retryable;
use super::provider::create_provider;
use crate::db::queries::delete_thread;
use crate::progress::NoopProgressReporter;

/// Local DB mutation for permanent delete (idempotent).
pub(crate) async fn permanent_delete_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
) -> Result<(), ActionError> {
    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        delete_thread(&conn, &aid, &tid)
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r.map_err(ActionError::db))
}

/// Provider dispatch for permanent delete (assumes local mutation already applied).
async fn permanent_delete_dispatch(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
) -> ActionOutcome {
    let mlog = MutationLog::begin("permanent_delete", account_id, thread_id);

    let provider_ctx = ProviderCtx {
        account_id,
        db: &ctx.db,
        body_store: &ctx.body_store,
        inline_images: &ctx.inline_images,
        search: &ctx.search,
        progress: &NoopProgressReporter,
    };

    let outcome = match provider.permanent_delete(&provider_ctx, thread_id).await {
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
        "permanentDelete",
        thread_id,
        "{}",
    )
    .await;
    mlog.emit(&outcome);
    outcome
}

/// Permanently delete a single thread. Irreversible.
pub async fn permanent_delete(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
) -> ActionOutcome {
    let mlog = MutationLog::begin("permanent_delete", account_id, thread_id);

    if let Err(e) = permanent_delete_local(ctx, account_id, thread_id).await {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(provider) => permanent_delete_dispatch(ctx, &*provider, account_id, thread_id).await,
        Err(e) => {
            let outcome = ActionOutcome::LocalOnly {
                reason: ActionError::remote(e),
                retryable: true,
            };
            enqueue_if_retryable(
                ctx,
                &outcome,
                account_id,
                "permanentDelete",
                thread_id,
                "{}",
            )
            .await;
            mlog.emit(&outcome);
            outcome
        }
    }
}

/// Permanent delete with a pre-constructed provider (for batch reuse).
pub(crate) async fn permanent_delete_with_provider(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
) -> ActionOutcome {
    let mlog = MutationLog::begin("permanent_delete", account_id, thread_id);

    if let Err(e) = permanent_delete_local(ctx, account_id, thread_id).await {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    permanent_delete_dispatch(ctx, provider, account_id, thread_id).await
}
