use common::ops::ProviderOps;
use common::types::ActionProviderCtx;

use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use super::pending::enqueue_if_retryable;
use super::provider::create_provider;
use db::db::queries::set_thread_read;
use db::progress::NoopProgressReporter;
use rusqlite::params;

/// Local DB mutation for mark-read (idempotent).
pub(crate) async fn mark_read_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    read: bool,
) -> Result<(), ActionError> {
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    db.with_write(move |conn| {
        let tx = conn
            .transaction()
            .map_err(|e| format!("begin mark-read transaction: {e}"))?;
        set_thread_read(&tx, &aid, &tid, read)?;
        tx.execute(
            "UPDATE messages SET is_read = ?1 WHERE account_id = ?2 AND thread_id = ?3",
            params![read, aid, tid],
        )
        .map_err(|e| format!("update message read flags: {e}"))?;
        tx.commit()
            .map_err(|e| format!("commit mark-read transaction: {e}"))?;
        Ok::<(), String>(())
    })
    .await
    .map_err(ActionError::db)
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

    let provider_ctx = ActionProviderCtx {
        account_id,
        db: &ctx.db,
        progress: &NoopProgressReporter,
    };

    let outcome = match provider.mark_read(&provider_ctx, thread_id, read).await {
        Ok(()) => {
            if read {
                super::mdn_send::send_mdn_responses(ctx, provider, account_id, thread_id).await;
            }
            ActionOutcome::Success
        }
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
        "markRead",
        thread_id,
        &params_json,
    )
    .await;
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

    match create_provider(&ctx.db, &ctx.write_db, account_id, ctx.encryption_key).await {
        Ok(provider) => mark_read_dispatch(ctx, &*provider, account_id, thread_id, read).await,
        Err(e) => {
            let outcome = ActionOutcome::LocalOnly {
                reason: ActionError::remote(e),
                retryable: true,
            };
            enqueue_if_retryable(
                ctx,
                &outcome,
                account_id,
                "markRead",
                thread_id,
                &params_json,
            )
            .await;
            mlog.emit(&outcome);
            outcome
        }
    }
}
