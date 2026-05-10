use common::ops::ProviderOps;
use common::types::ActionProviderCtx;

use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use super::pending::enqueue_if_retryable;
use super::provider::create_provider;
use db::db::queries::set_thread_starred;
use db::progress::NoopProgressReporter;
use rusqlite::params;

/// Local DB mutation for star. Returns true if state changed.
pub(crate) async fn star_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    starred: bool,
) -> Result<bool, ActionError> {
    let ctx_clone = ctx.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    tokio::task::spawn_blocking(move || {
        ctx_clone.verify_thread_exists(&aid, &tid)?;
        let conn = ctx_clone.db.conn();
        let conn = conn
            .lock()
            .map_err(|e| ActionError::db(format!("db lock: {e}")))?;
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| ActionError::db(format!("begin star transaction: {e}")))?;
        let thread_changed = set_thread_starred(&tx, &aid, &tid, starred)
            .map(|n| n > 0)
            .map_err(ActionError::db)?;
        let message_changed = tx
            .execute(
                "UPDATE messages SET is_starred = ?1 WHERE account_id = ?2 AND thread_id = ?3",
                params![starred, aid, tid],
            )
            .map(|n| n > 0)
            .map_err(|e| ActionError::db(format!("update message starred flags: {e}")))?;
        tx.commit()
            .map_err(|e| ActionError::db(format!("commit star transaction: {e}")))?;
        Ok(thread_changed || message_changed)
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

    let provider_ctx = ActionProviderCtx {
        account_id,
        db: &ctx.db,
        progress: &NoopProgressReporter,
    };

    let outcome = match provider.star(&provider_ctx, thread_id, starred).await {
        Ok(()) => ActionOutcome::Success,
        Err(e) => {
            let msg = e.to_string();
            ActionOutcome::LocalOnly {
                reason: ActionError::remote(msg),
                retryable: true,
            }
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
            let outcome = ActionOutcome::LocalOnly {
                reason: ActionError::remote(e),
                retryable: true,
            };
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
