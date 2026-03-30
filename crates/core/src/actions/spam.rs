use common::ops::ProviderOps;
use common::types::ProviderCtx;

use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use super::pending::enqueue_if_retryable;
use super::provider::create_provider;
use crate::email_actions::{insert_label, remove_label};
use crate::progress::NoopProgressReporter;

/// Local DB mutation for spam (idempotent).
pub(crate) async fn spam_local(ctx: &ActionContext, account_id: &str, thread_id: &str, is_spam: bool) -> Result<(), ActionError> {
    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        if is_spam {
            remove_label(&conn, &aid, &tid, "INBOX")?;
            insert_label(&conn, &aid, &tid, "SPAM").map(|_| ())
        } else {
            remove_label(&conn, &aid, &tid, "SPAM")?;
            insert_label(&conn, &aid, &tid, "INBOX").map(|_| ())
        }
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r.map_err(ActionError::db))
}

/// Provider dispatch for spam (assumes local mutation already applied).
async fn spam_dispatch(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    is_spam: bool,
) -> ActionOutcome {
    let mlog = MutationLog::begin("spam", account_id, thread_id);
    let params_json = format!(r#"{{"isSpam":{is_spam}}}"#);

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
    enqueue_if_retryable(ctx, &outcome, account_id, "spam", thread_id, &params_json).await;
    mlog.emit(&outcome);
    outcome
}

/// Mark or unmark a single thread as spam.
pub async fn spam(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    is_spam: bool,
) -> ActionOutcome {
    let mlog = MutationLog::begin("spam", account_id, thread_id);
    let params_json = format!(r#"{{"isSpam":{is_spam}}}"#);

    if let Err(e) = spam_local(ctx, account_id, thread_id, is_spam).await {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(provider) => spam_dispatch(ctx, &*provider, account_id, thread_id, is_spam).await,
        Err(e) => {
            let outcome = ActionOutcome::LocalOnly { reason: ActionError::remote(e), retryable: true };
            enqueue_if_retryable(ctx, &outcome, account_id, "spam", thread_id, &params_json).await;
            mlog.emit(&outcome);
            outcome
        }
    }
}

/// Spam with a pre-constructed provider (for batch reuse).
pub(crate) async fn spam_with_provider(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    is_spam: bool,
) -> ActionOutcome {
    let mlog = MutationLog::begin("spam", account_id, thread_id);

    if let Err(e) = spam_local(ctx, account_id, thread_id, is_spam).await {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    spam_dispatch(ctx, provider, account_id, thread_id, is_spam).await
}
