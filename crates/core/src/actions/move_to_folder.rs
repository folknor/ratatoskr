use common::ops::ProviderOps;
use common::typed_ids::FolderId;
use common::types::ProviderCtx;

use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use super::pending::enqueue_if_retryable;
use super::provider::create_provider;
use crate::email_actions::{insert_label, remove_label};
use crate::progress::NoopProgressReporter;

/// Local DB mutation for move-to-folder (idempotent).
pub(crate) async fn move_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    folder_id: &FolderId,
    source_label_id: Option<&FolderId>,
) -> Result<(), ActionError> {
    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let fid = folder_id.as_str().to_string();
    let source = source_label_id.map(|s| s.as_str().to_string());
    tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        if let Some(ref src) = source {
            remove_label(&conn, &aid, &tid, src)?;
        }
        insert_label(&conn, &aid, &tid, &fid).map(|_| ())
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r.map_err(ActionError::db))
}

/// Provider dispatch for move-to-folder (assumes local mutation already applied).
async fn move_dispatch(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    folder_id: &FolderId,
    source_label_id: Option<&FolderId>,
) -> ActionOutcome {
    let mlog = MutationLog::begin("move_to_folder", account_id, thread_id);
    let params_json = serde_json::json!({
        "folderId": folder_id.as_str(),
        "sourceLabelId": source_label_id.map(FolderId::as_str),
    })
    .to_string();

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

/// Move a single thread to a different folder.
pub async fn move_to_folder(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    folder_id: &FolderId,
    source_label_id: Option<&FolderId>,
) -> ActionOutcome {
    let mlog = MutationLog::begin("move_to_folder", account_id, thread_id);
    let params_json = serde_json::json!({
        "folderId": folder_id.as_str(),
        "sourceLabelId": source_label_id.map(FolderId::as_str),
    })
    .to_string();

    if let Err(e) = move_local(ctx, account_id, thread_id, folder_id, source_label_id).await {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(provider) => move_dispatch(ctx, &*provider, account_id, thread_id, folder_id, source_label_id).await,
        Err(e) => {
            let outcome = ActionOutcome::LocalOnly { reason: ActionError::remote(e), retryable: true };
            enqueue_if_retryable(ctx, &outcome, account_id, "moveToFolder", thread_id, &params_json).await;
            mlog.emit(&outcome);
            outcome
        }
    }
}

/// Move to folder with a pre-constructed provider (for batch reuse).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn move_to_folder_with_provider(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    folder_id: &FolderId,
    source_label_id: Option<&FolderId>,
) -> ActionOutcome {
    let mlog = MutationLog::begin("move_to_folder", account_id, thread_id);

    if let Err(e) = move_local(ctx, account_id, thread_id, folder_id, source_label_id).await {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    move_dispatch(ctx, provider, account_id, thread_id, folder_id, source_label_id).await
}
