use super::context::ActionContext;
use super::outcome::ActionOutcome;
use super::provider::create_provider;
use crate::email_actions::{insert_label, remove_label};
use crate::progress::NoopProgressReporter;
use ratatoskr_provider_utils::types::ProviderCtx;

/// Trash a single thread: remove from inbox, add to trash locally, then
/// dispatch to provider.
pub async fn trash(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
) -> ActionOutcome {
    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        remove_label(&conn, &aid, &tid, "INBOX")?;
        insert_label(&conn, &aid, &tid, "TRASH")
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))
    .and_then(|r| r);

    if let Err(e) = local_result {
        return ActionOutcome::Failed { error: e };
    }

    let provider = match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(p) => p,
        Err(e) => {
            log::warn!("Trash local-only (provider create failed): {e}");
            return ActionOutcome::LocalOnly { remote_error: e };
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

    match provider.trash(&provider_ctx, thread_id).await {
        Ok(()) => ActionOutcome::Success,
        Err(e) => {
            let msg = e.to_string();
            log::warn!("Trash remote failed for {account_id}/{thread_id}: {msg}");
            ActionOutcome::LocalOnly { remote_error: msg }
        }
    }
}
