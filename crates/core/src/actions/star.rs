use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use super::provider::create_provider;
use crate::db::queries::set_thread_starred;
use crate::progress::NoopProgressReporter;
use ratatoskr_provider_utils::types::ProviderCtx;

/// Toggle star on a single thread.
///
/// `starred` is the target value (not "toggle" — the caller resolves
/// current state and passes the new value to avoid TOCTOU issues).
pub async fn star(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    starred: bool,
) -> ActionOutcome {
    let mlog = MutationLog::begin("star", account_id, thread_id);

    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        set_thread_starred(&conn, &aid, &tid, starred)
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r.map_err(ActionError::db));

    if let Err(e) = local_result {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    let provider = match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(p) => p,
        Err(e) => {
            let outcome = ActionOutcome::LocalOnly { reason: ActionError::remote(e), retryable: true };
            mlog.emit(&outcome);
            return outcome;
        }
    };

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
    mlog.emit(&outcome);
    outcome
}
