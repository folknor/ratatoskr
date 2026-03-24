use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use super::provider::create_provider;
use crate::progress::NoopProgressReporter;
use ratatoskr_provider_utils::types::ProviderCtx;

/// Apply a label to a single thread.
///
/// The service owns the `label_kind` routing: tags use name-based category ops
/// (`apply_category`), containers use ID-based tag ops (`add_tag`).
///
/// Note: `apply_category` declares its second parameter as `message_id` in the
/// `ProviderOps` trait, but we pass `thread_id`. This matches existing behavior
/// — all four providers resolve thread-level operations internally. Fixing the
/// trait contract is a Phase 6 concern.
pub async fn add_label(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    label_id: &str,
) -> ActionOutcome {
    let mlog = MutationLog::begin("add_label", account_id, thread_id);

    // 1. Look up label metadata + local DB mutation in one spawn_blocking call
    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let lid = label_id.to_string();
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| ActionError::db(format!("db lock: {e}")))?;

        // Look up label metadata (name + kind) for provider routing.
        // Scoped by account_id — label identity is (account_id, label_id).
        let (label_name, label_kind) = conn
            .query_row(
                "SELECT name, label_kind FROM labels \
                 WHERE id = ?1 AND account_id = ?2 LIMIT 1",
                rusqlite::params![lid, aid],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    ActionError::not_found("label not found for this account")
                }
                other => ActionError::db(format!("label lookup: {other}")),
            })?;

        // Local DB mutation (INSERT OR IGNORE — idempotent)
        crate::email_actions::insert_label(&conn, &aid, &tid, &lid)
            .map_err(ActionError::db)?;

        Ok((label_name, label_kind))
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r);

    let (label_name, label_kind) = match local_result {
        Ok(info) => info,
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
    };

    // 2. Provider dispatch
    let provider = match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(p) => p,
        Err(e) => {
            let outcome = ActionOutcome::LocalOnly {
                reason: ActionError::remote(e),
                retryable: true,
            };
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

    // Route by label_kind: tags use name-based category ops,
    // containers use ID-based tag ops.
    let result = if label_kind == "tag" {
        provider
            .apply_category(&provider_ctx, thread_id, &label_name)
            .await
    } else {
        provider
            .add_tag(&provider_ctx, thread_id, label_id)
            .await
    };

    let outcome = match result {
        Ok(()) => ActionOutcome::Success,
        Err(e) => {
            let msg = e.to_string();
            ActionOutcome::LocalOnly {
                reason: ActionError::remote(msg),
                retryable: true,
            }
        }
    };
    mlog.emit(&outcome);
    outcome
}

/// Remove a label from a single thread.
///
/// Same routing as `add_label`: tags use `remove_category` (name-based),
/// containers use `remove_tag` (ID-based).
pub async fn remove_label(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    label_id: &str,
) -> ActionOutcome {
    let mlog = MutationLog::begin("remove_label", account_id, thread_id);

    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let lid = label_id.to_string();
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| ActionError::db(format!("db lock: {e}")))?;

        let (label_name, label_kind) = conn
            .query_row(
                "SELECT name, label_kind FROM labels \
                 WHERE id = ?1 AND account_id = ?2 LIMIT 1",
                rusqlite::params![lid, aid],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    ActionError::not_found("label not found for this account")
                }
                other => ActionError::db(format!("label lookup: {other}")),
            })?;

        // Local DB mutation
        crate::email_actions::remove_label(&conn, &aid, &tid, &lid)
            .map_err(ActionError::db)?;

        Ok((label_name, label_kind))
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r);

    let (label_name, label_kind) = match local_result {
        Ok(info) => info,
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
    };

    let provider = match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(p) => p,
        Err(e) => {
            let outcome = ActionOutcome::LocalOnly {
                reason: ActionError::remote(e),
                retryable: true,
            };
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

    let result = if label_kind == "tag" {
        provider
            .remove_category(&provider_ctx, thread_id, &label_name)
            .await
    } else {
        provider
            .remove_tag(&provider_ctx, thread_id, label_id)
            .await
    };

    let outcome = match result {
        Ok(()) => ActionOutcome::Success,
        Err(e) => {
            let msg = e.to_string();
            ActionOutcome::LocalOnly {
                reason: ActionError::remote(msg),
                retryable: true,
            }
        }
    };
    mlog.emit(&outcome);
    outcome
}
