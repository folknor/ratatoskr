use common::ops::ProviderOps;
use common::types::ProviderCtx;

use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use super::pending::enqueue_if_retryable;
use super::provider::create_provider;
use crate::email_actions::remove_inbox_label;
use crate::progress::NoopProgressReporter;

/// Local DB mutation for archive. Returns true if state changed.
pub(crate) async fn archive_local(ctx: &ActionContext, account_id: &str, thread_id: &str) -> Result<bool, ActionError> {
    let ctx_clone = ctx.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    tokio::task::spawn_blocking(move || {
        ctx_clone.verify_thread_exists(&aid, &tid)?;
        let conn = ctx_clone.db.conn();
        let conn = conn.lock().map_err(|e| ActionError::db(format!("db lock: {e}")))?;
        remove_inbox_label(&conn, &aid, &tid)
            .map(|n| n > 0)
            .map_err(ActionError::db)
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))?
}

/// Provider dispatch for archive (assumes local mutation already applied).
async fn archive_dispatch(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
) -> ActionOutcome {
    let mlog = MutationLog::begin("archive", account_id, thread_id);

    let provider_ctx = ProviderCtx {
        account_id,
        db: &ctx.db,
        body_store: &ctx.body_store,
        inline_images: &ctx.inline_images,
        search: &ctx.search,
        progress: &NoopProgressReporter,
    };

    let outcome = match provider.archive(&provider_ctx, thread_id).await {
        Ok(()) => ActionOutcome::Success,
        Err(e) => {
            let msg = e.to_string();
            ActionOutcome::LocalOnly { reason: ActionError::remote(msg), retryable: true }
        }
    };
    enqueue_if_retryable(ctx, &outcome, account_id, "archive", thread_id, "{}").await;
    mlog.emit(&outcome);
    outcome
}

/// Archive a single thread: remove from inbox locally, then dispatch to provider.
pub async fn archive(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
) -> ActionOutcome {
    let mlog = MutationLog::begin("archive", account_id, thread_id);

    match archive_local(ctx, account_id, thread_id).await {
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
        Ok(false) => return ActionOutcome::NoOp,
        Ok(true) => {}
    }

    match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(provider) => archive_dispatch(ctx, &*provider, account_id, thread_id).await,
        Err(e) => {
            let outcome = ActionOutcome::LocalOnly { reason: ActionError::remote(e), retryable: true };
            enqueue_if_retryable(ctx, &outcome, account_id, "archive", thread_id, "{}").await;
            mlog.emit(&outcome);
            outcome
        }
    }
}

/// Archive with a pre-constructed provider (for batch reuse).
pub(crate) async fn archive_with_provider(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
) -> ActionOutcome {
    let mlog = MutationLog::begin("archive", account_id, thread_id);

    match archive_local(ctx, account_id, thread_id).await {
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
        Ok(false) => return ActionOutcome::NoOp,
        Ok(true) => {}
    }

    archive_dispatch(ctx, provider, account_id, thread_id).await
}
