use ratatoskr_provider_utils::ops::ProviderOps;
use ratatoskr_provider_utils::types::ProviderCtx;

use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use super::pending::enqueue_if_retryable;
use super::provider::create_provider;
use crate::db::queries::set_thread_starred;
use crate::progress::NoopProgressReporter;

/// Local DB mutation for star. Returns true if state changed.
pub(crate) async fn star_local(ctx: &ActionContext, account_id: &str, thread_id: &str, starred: bool) -> Result<bool, ActionError> {
    let ctx_clone = ctx.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    tokio::task::spawn_blocking(move || {
        ctx_clone.verify_thread_exists(&aid, &tid)?;
        let conn = ctx_clone.db.conn();
        let conn = conn.lock().map_err(|e| ActionError::db(format!("db lock: {e}")))?;
        set_thread_starred(&conn, &aid, &tid, starred)
            .map(|n| n > 0)
            .map_err(ActionError::db)
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))?
}

/// Provider dispatch for star (assumes local mutation already applied).
async fn star_dispatch(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    starred: bool,
) -> ActionOutcome {
    let mlog = MutationLog::begin("star", account_id, thread_id);
    let params_json = format!(r#"{{"starred":{starred}}}"#);

    let provider_ctx = ProviderCtx {
        account_id,
        db: &ctx.db,
        body_store: &ctx.body_store,
        inline_images: &ctx.inline_images,
        search: &ctx.search,
        progress: &NoopProgressReporter,
    };

    let outcome = match provider.star(&provider_ctx, thread_id, starred).await {
        Ok(()) => ActionOutcome::Success,
        Err(e) => {
            let msg = e.to_string();
            ActionOutcome::LocalOnly { reason: ActionError::remote(msg), retryable: true }
        }
    };
    enqueue_if_retryable(ctx, &outcome, account_id, "star", thread_id, &params_json).await;
    mlog.emit(&outcome);
    outcome
}

/// Toggle star on a single thread.
pub async fn star(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    starred: bool,
) -> ActionOutcome {
    let mlog = MutationLog::begin("star", account_id, thread_id);
    let params_json = format!(r#"{{"starred":{starred}}}"#);

    match star_local(ctx, account_id, thread_id, starred).await {
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
        Ok(false) => return ActionOutcome::NoOp,
        Ok(true) => {}
    }

    match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(provider) => star_dispatch(ctx, &*provider, account_id, thread_id, starred).await,
        Err(e) => {
            let outcome = ActionOutcome::LocalOnly { reason: ActionError::remote(e), retryable: true };
            enqueue_if_retryable(ctx, &outcome, account_id, "star", thread_id, &params_json).await;
            mlog.emit(&outcome);
            outcome
        }
    }
}

/// Star with a pre-constructed provider (for batch reuse).
pub(crate) async fn star_with_provider(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    starred: bool,
) -> ActionOutcome {
    let mlog = MutationLog::begin("star", account_id, thread_id);

    match star_local(ctx, account_id, thread_id, starred).await {
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
        Ok(false) => return ActionOutcome::NoOp,
        Ok(true) => {}
    }

    star_dispatch(ctx, provider, account_id, thread_id, starred).await
}
