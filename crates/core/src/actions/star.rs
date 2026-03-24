use ratatoskr_provider_utils::ops::ProviderOps;
use ratatoskr_provider_utils::types::ProviderCtx;

use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use super::pending::enqueue_if_retryable;
use super::provider::create_provider;
use crate::db::queries::set_thread_starred;
use crate::progress::NoopProgressReporter;

/// Local DB mutation for star (idempotent).
async fn star_local(ctx: &ActionContext, account_id: &str, thread_id: &str, starred: bool) -> Result<(), ActionError> {
    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        set_thread_starred(&conn, &aid, &tid, starred)
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r.map_err(ActionError::db))
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

    if let Err(e) = star_local(ctx, account_id, thread_id, starred).await {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
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

    if let Err(e) = star_local(ctx, account_id, thread_id, starred).await {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    star_dispatch(ctx, provider, account_id, thread_id, starred).await
}
