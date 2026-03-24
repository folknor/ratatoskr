use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use super::provider::create_provider;
use crate::email_actions::{insert_label, remove_label};
use crate::progress::NoopProgressReporter;
use ratatoskr_provider_utils::types::ProviderCtx;

/// Mark or unmark a single thread as spam.
///
/// `is_spam` is the target state (true = mark as spam, false = un-spam).
/// The caller resolves the current state and passes the direction explicitly.
pub async fn spam(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    is_spam: bool,
) -> ActionOutcome {
    let mlog = MutationLog::begin("spam", account_id, thread_id);

    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        if is_spam {
            remove_label(&conn, &aid, &tid, "INBOX")?;
            insert_label(&conn, &aid, &tid, "SPAM")
        } else {
            remove_label(&conn, &aid, &tid, "SPAM")?;
            insert_label(&conn, &aid, &tid, "INBOX")
        }
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

    let outcome = match provider.spam(&provider_ctx, thread_id, is_spam).await {
        Ok(()) => ActionOutcome::Success,
        Err(e) => {
            let msg = e.to_string();
            ActionOutcome::LocalOnly { reason: ActionError::remote(msg), retryable: true }
        }
    };
    mlog.emit(&outcome);
    outcome
}
