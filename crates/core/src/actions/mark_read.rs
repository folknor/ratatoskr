use super::context::ActionContext;
use super::outcome::{ActionError, ActionOutcome};
use super::provider::create_provider;
use crate::db::queries::set_thread_read;
use crate::progress::NoopProgressReporter;
use ratatoskr_provider_utils::types::ProviderCtx;

/// Set read/unread state on a single thread.
///
/// `read` is the target value.
pub async fn mark_read(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    read: bool,
) -> ActionOutcome {
    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        set_thread_read(&conn, &aid, &tid, read)
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r.map_err(ActionError::db));

    if let Err(e) = local_result {
        return ActionOutcome::Failed { error: e };
    }

    let provider = match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(p) => p,
        Err(e) => {
            log::warn!("Mark read local-only (provider create failed): {e}");
            return ActionOutcome::LocalOnly { reason: ActionError::remote(e) };
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

    match provider.mark_read(&provider_ctx, thread_id, read).await {
        Ok(()) => ActionOutcome::Success,
        Err(e) => {
            let msg = e.to_string();
            log::warn!("Mark read remote failed for {account_id}/{thread_id}: {msg}");
            ActionOutcome::LocalOnly { reason: ActionError::remote(msg) }
        }
    }
}
