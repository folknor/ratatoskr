//! Trash action.
//!
//! Trash is reversible (the underlying message stays in the provider
//! Trash mailbox until it ages out), so the search-index entry stays
//! around by design while in Trash. The Phase 2 search-index contract
//! described on `permanent_delete.rs` applies to the eventual
//! permanent-delete that resolves a Trash. No action-time Tantivy
//! writes happen here either; the type system enforces it via
//! `ActionProviderCtx`'s lack of a `&SearchReadState` field.

use common::ops::ProviderOps;
use common::types::ActionProviderCtx;

use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use super::pending::enqueue_if_retryable;
use super::provider::create_provider;
use db::db::queries_extra::{insert_folder, remove_folder};
use db::progress::NoopProgressReporter;

/// Local DB mutation for trash (idempotent).
pub(crate) async fn trash_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
) -> Result<(), ActionError> {
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    db.with_conn(move |conn| {
        remove_folder(conn, &aid, &tid, "INBOX")?;
        insert_folder(conn, &aid, &tid, "TRASH").map(|_| ())
    })
    .await
    .map_err(ActionError::db)
}

/// Provider dispatch for trash (assumes local mutation already applied).
async fn trash_dispatch(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
) -> ActionOutcome {
    let mlog = MutationLog::begin("trash", account_id, thread_id);

    let provider_ctx = ActionProviderCtx {
        account_id,
        db: &ctx.db,
        progress: &NoopProgressReporter,
    };

    let outcome = match provider.trash(&provider_ctx, thread_id).await {
        Ok(()) => ActionOutcome::Success,
        Err(e) => {
            let msg = e.to_string();
            ActionOutcome::LocalOnly {
                reason: ActionError::remote(msg),
                retryable: true,
            }
        }
    };
    enqueue_if_retryable(ctx, &outcome, account_id, "trash", thread_id, "{}").await;
    mlog.emit(&outcome);
    outcome
}

/// Trash a single thread.
pub async fn trash(ctx: &ActionContext, account_id: &str, thread_id: &str) -> ActionOutcome {
    let mlog = MutationLog::begin("trash", account_id, thread_id);

    if let Err(e) = trash_local(ctx, account_id, thread_id).await {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(provider) => trash_dispatch(ctx, &*provider, account_id, thread_id).await,
        Err(e) => {
            let outcome = ActionOutcome::LocalOnly {
                reason: ActionError::remote(e),
                retryable: true,
            };
            enqueue_if_retryable(ctx, &outcome, account_id, "trash", thread_id, "{}").await;
            mlog.emit(&outcome);
            outcome
        }
    }
}

/// Trash with a pre-constructed provider (for batch reuse).
pub(crate) async fn trash_with_provider(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
) -> ActionOutcome {
    let mlog = MutationLog::begin("trash", account_id, thread_id);

    if let Err(e) = trash_local(ctx, account_id, thread_id).await {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    trash_dispatch(ctx, provider, account_id, thread_id).await
}
