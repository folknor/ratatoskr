//! Permanent delete action.
//!
//! **Phase 2 search-index contract (scope item 18b / task 16).** This
//! action does NOT touch the Tantivy index. The local DB row is
//! removed and the provider is asked to delete server-side; the
//! search-index entry for the message survives the action. The
//! cross-store invariant pass drops the orphaned doc on the next
//! sentinel-absent boot. The temporary inconsistency window is
//! intentional: relocating the Tantivy writer in lock-step with
//! actions would tangle Phase 2 with the Phase 3 sync surgery for no
//! UI-visible benefit (search readers see "the deleted message
//! disappears" once the next reload follows the next commit; in the
//! meantime, the search-result row's parent thread is gone from
//! `messages` and gets filtered out at result-render time).
//!
//! Type-level guarantee: this action takes `ActionProviderCtx` (no
//! `&SearchReadState` field), so the compiler rejects any direct
//! `ctx.search.*` write from inside the dispatch path. Future
//! contributors who want action-time index writes would have to
//! explicitly extend the type, which forces a design conversation.

use common::ops::ProviderOps;
use common::types::ActionProviderCtx;

use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use super::pending::enqueue_if_retryable;
use super::provider::{classify_provider_error, create_provider};
use db::db::queries::delete_thread;
use db::progress::NoopProgressReporter;

/// Local DB mutation for permanent delete (idempotent).
pub(crate) async fn permanent_delete_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
) -> Result<(), ActionError> {
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    db.with_write(move |conn| delete_thread(conn, &aid, &tid))
        .await
        .map_err(ActionError::db)
}

async fn enqueue_permanent_delete_retry(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    error: &ActionError,
) {
    let retry_outcome = ActionOutcome::LocalOnly {
        reason: error.clone(),
        retryable: true,
    };
    enqueue_if_retryable(
        ctx,
        &retry_outcome,
        account_id,
        "permanentDelete",
        thread_id,
        "{}",
    )
    .await;
}

/// Provider dispatch for permanent delete.
async fn permanent_delete_dispatch(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
) -> ActionOutcome {
    let mlog = MutationLog::begin("permanent_delete", account_id, thread_id);

    let provider_ctx = ActionProviderCtx {
        account_id,
        db: &ctx.db,
        progress: &NoopProgressReporter,
    };

    let outcome = match provider.permanent_delete(&provider_ctx, thread_id).await {
        Ok(()) => match permanent_delete_local(ctx, account_id, thread_id).await {
            Ok(()) => ActionOutcome::Success,
            Err(e) => ActionOutcome::Failed { error: e },
        },
        Err(e) => {
            let msg = e.to_string();
            let error = ActionError::remote(msg);
            enqueue_permanent_delete_retry(ctx, account_id, thread_id, &error).await;
            ActionOutcome::Failed { error }
        }
    };
    mlog.emit(&outcome);
    outcome
}

/// Permanently delete a single thread. Irreversible.
///
/// This is provider-first because IMAP and Graph need local message
/// metadata to address the remote objects. If remote dispatch fails,
/// the local DB is left intact so a later retry still has those refs.
pub async fn permanent_delete(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
) -> ActionOutcome {
    let mlog = MutationLog::begin("permanent_delete", account_id, thread_id);

    match create_provider(&ctx.db, &ctx.write_db, account_id, ctx.encryption_key).await {
        Ok(provider) => permanent_delete_dispatch(ctx, &*provider, account_id, thread_id).await,
        Err(e) => {
            let kind = classify_provider_error(&e);
            let error = ActionError::remote_with_kind(kind, e);
            enqueue_permanent_delete_retry(ctx, account_id, thread_id, &error).await;
            let outcome = ActionOutcome::Failed { error };
            mlog.emit(&outcome);
            outcome
        }
    }
}
