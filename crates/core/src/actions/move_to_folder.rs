use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use super::pending::enqueue_if_retryable;
use super::provider::create_provider;
use crate::email_actions::{insert_label, remove_label};
use crate::progress::NoopProgressReporter;
use ratatoskr_provider_utils::types::ProviderCtx;

/// Move a single thread to a different folder.
///
/// `folder_id` is the target folder's label ID (Ratatoskr canonical for
/// system folders, provider-prefixed for user folders).
///
/// `source_label_id` is the folder to remove from. `None` means "don't
/// remove from any source" (just add to target). The caller resolves
/// the source from the current navigation context — it's not always INBOX.
pub async fn move_to_folder(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    folder_id: &str,
    source_label_id: Option<&str>,
) -> ActionOutcome {
    let mlog = MutationLog::begin("move_to_folder", account_id, thread_id);
    let params_json = if let Some(src) = source_label_id {
        format!(r#"{{"folderId":"{folder_id}","sourceLabelId":"{src}"}}"#)
    } else {
        format!(r#"{{"folderId":"{folder_id}"}}"#)
    };

    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let fid = folder_id.to_string();
    let source = source_label_id.map(String::from);
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        if let Some(ref src) = source {
            remove_label(&conn, &aid, &tid, src)?;
        }
        insert_label(&conn, &aid, &tid, &fid)
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
            enqueue_if_retryable(ctx, &outcome, account_id, "moveToFolder", thread_id, &params_json).await;
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

    let outcome = match provider.move_to_folder(&provider_ctx, thread_id, folder_id).await {
        Ok(()) => ActionOutcome::Success,
        Err(e) => {
            let msg = e.to_string();
            ActionOutcome::LocalOnly { reason: ActionError::remote(msg), retryable: true }
        }
    };
    enqueue_if_retryable(ctx, &outcome, account_id, "moveToFolder", thread_id, &params_json).await;
    mlog.emit(&outcome);
    outcome
}
