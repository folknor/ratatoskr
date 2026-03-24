use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use super::provider::create_provider;
use crate::email_actions::remove_inbox_label;
use crate::progress::NoopProgressReporter;
use ratatoskr_provider_utils::types::ProviderCtx;

/// Archive a single thread: remove from inbox locally, then dispatch to provider.
///
/// This is the first action migrated to the action service. All four
/// providers implement `ProviderOps::archive()` with real API calls:
/// - Gmail: remove INBOX label via modify_thread
/// - Graph: move messages to archive folder
/// - JMAP: update mailbox memberships (remove inbox, add archive)
/// - IMAP: COPY to archive folder + flag delete from inbox
pub async fn archive(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
) -> ActionOutcome {
    let mlog = MutationLog::begin("archive", account_id, thread_id);

    // 1. Local DB mutation (on blocking thread — DB connections are sync)
    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        remove_inbox_label(&conn, &aid, &tid)
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r.map_err(ActionError::db));

    if let Err(e) = local_result {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    // 2. Provider dispatch
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

    let outcome = match provider.archive(&provider_ctx, thread_id).await {
        Ok(()) => ActionOutcome::Success,
        Err(e) => {
            let msg = e.to_string();
            ActionOutcome::LocalOnly { reason: ActionError::remote(msg), retryable: true }
        }
    };
    mlog.emit(&outcome);
    outcome
}
