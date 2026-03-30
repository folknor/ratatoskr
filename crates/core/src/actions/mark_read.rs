use common::ops::ProviderOps;
use common::types::ProviderCtx;

use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use super::pending::enqueue_if_retryable;
use super::provider::create_provider;
use crate::db::queries::set_thread_read;
use crate::progress::NoopProgressReporter;

/// Local DB mutation for mark-read (idempotent).
pub(crate) async fn mark_read_local(ctx: &ActionContext, account_id: &str, thread_id: &str, read: bool) -> Result<(), ActionError> {
    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        set_thread_read(&conn, &aid, &tid, read).map(|_| ())
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r.map_err(ActionError::db))
}

/// Provider dispatch for mark-read (assumes local mutation already applied).
async fn mark_read_dispatch(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    read: bool,
) -> ActionOutcome {
    let mlog = MutationLog::begin("mark_read", account_id, thread_id);
    let params_json = format!(r#"{{"read":{read}}}"#);

    let provider_ctx = ProviderCtx {
        account_id,
        db: &ctx.db,
        body_store: &ctx.body_store,
        inline_images: &ctx.inline_images,
        search: &ctx.search,
        progress: &NoopProgressReporter,
    };

    let outcome = match provider.mark_read(&provider_ctx, thread_id, read).await {
        Ok(()) => ActionOutcome::Success,
        Err(e) => {
            let msg = e.to_string();
            ActionOutcome::LocalOnly { reason: ActionError::remote(msg), retryable: true }
        }
    };
    enqueue_if_retryable(ctx, &outcome, account_id, "markRead", thread_id, &params_json).await;
    mlog.emit(&outcome);
    outcome
}

/// Set read/unread state on a single thread.
pub async fn mark_read(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    read: bool,
) -> ActionOutcome {
    let mlog = MutationLog::begin("mark_read", account_id, thread_id);
    let params_json = format!(r#"{{"read":{read}}}"#);

    if let Err(e) = mark_read_local(ctx, account_id, thread_id, read).await {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(provider) => mark_read_dispatch(ctx, &*provider, account_id, thread_id, read).await,
        Err(e) => {
            let outcome = ActionOutcome::LocalOnly { reason: ActionError::remote(e), retryable: true };
            enqueue_if_retryable(ctx, &outcome, account_id, "markRead", thread_id, &params_json).await;
            mlog.emit(&outcome);
            outcome
        }
    }
}

/// Mark read with a pre-constructed provider (for batch reuse).
pub(crate) async fn mark_read_with_provider(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    read: bool,
) -> ActionOutcome {
    let mlog = MutationLog::begin("mark_read", account_id, thread_id);

    if let Err(e) = mark_read_local(ctx, account_id, thread_id, read).await {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    mark_read_dispatch(ctx, provider, account_id, thread_id, read).await
}
