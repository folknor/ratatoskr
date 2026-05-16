use common::ops::ProviderOps;
use common::typed_ids::LabelId;
use common::types::ActionProviderCtx;
use types::LabelKind;

use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use super::pending::enqueue_if_retryable;
use super::provider::create_provider;
use db::progress::NoopProgressReporter;

/// Local DB mutation for add-label: validate label exists, then insert into
/// `thread_labels` (idempotent).
pub(crate) async fn add_label_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
) -> Result<(), ActionError> {
    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let lid = label_id.as_str().to_string();
    tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn
            .lock()
            .map_err(|e| ActionError::db(format!("db lock: {e}")))?;
        let label_kind =
            label_kind_for_account_sync(&conn, &aid, &lid).map_err(ActionError::db)?;

        let exists = db::db::queries_extra::action_helpers::label_exists_sync(&conn, &lid, &aid)
            .map_err(|e| ActionError::db(format!("label lookup: {e}")))?;
        if !exists {
            ensure_typed_tag_label(&conn, &aid, &label_kind)
                .map_err(ActionError::db)?
                .ok_or_else(|| ActionError::not_found("label not found for this account"))?;
        }

        if let LabelKind::GraphImportance(level) = label_kind {
            let opposite = level.opposite().label_id();
            db::db::queries_extra::remove_label(&conn, &aid, &tid, opposite)
                .map_err(ActionError::db)?;
        }
        db::db::queries_extra::insert_label(&conn, &aid, &tid, &lid).map_err(ActionError::db)?;

        Ok(())
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r)
}

fn label_kind_for_account_sync(
    conn: &rusqlite::Connection,
    account_id: &str,
    label_id: &str,
) -> Result<LabelKind, String> {
    let provider = db::db::queries_extra::get_account_provider_sync(conn, account_id)?;
    LabelKind::parse(label_id, provider)
}

async fn label_kind_for_account(
    ctx: &ActionContext,
    account_id: &str,
    label_id: &LabelId,
) -> Result<LabelKind, ActionError> {
    let aid = account_id.to_string();
    let lid = label_id.as_str().to_string();
    ctx.db
        .with_conn(move |conn| label_kind_for_account_sync(conn, &aid, &lid))
        .await
        .map_err(|e| ActionError::db(format!("label kind lookup: {e}")))
}

fn ensure_typed_tag_label(
    conn: &rusqlite::Connection,
    account_id: &str,
    label: &LabelKind,
) -> Result<Option<()>, String> {
    let Some((name, sort_order, is_undeletable)) = label_write_metadata(label) else {
        return Ok(None);
    };
    let label_id = label.storage_id();

    // ON CONFLICT with OR semantics on is_undeletable repairs a pre-existing
    // row that was synced before the invariant landed. This matches
    // the OR rule in `upsert_labels` so the invariant from redesign.md
    // "is_undeletable" holds regardless of writer.
    conn.execute(
        "INSERT INTO labels \
         (id, account_id, name, sort_order, is_undeletable) \
         VALUES (?1, ?2, ?3, COALESCE(?4, 0), ?5) \
         ON CONFLICT(account_id, id) DO UPDATE SET \
           is_undeletable = (labels.is_undeletable OR excluded.is_undeletable)",
        rusqlite::params![label_id, account_id, name, sort_order, is_undeletable],
    )
    .map(|_| Some(()))
    .map_err(|e| format!("ensure typed tag label: {e}"))
}

fn label_write_metadata(label: &LabelKind) -> Option<(String, Option<i64>, bool)> {
    match label {
        LabelKind::GmailUser(_) => None,
        LabelKind::GraphCategory(category) => Some((category.as_str().to_string(), None, false)),
        LabelKind::GraphImportance(level) => Some((
            level.display_name().to_string(),
            Some(level.sort_order()),
            true,
        )),
        LabelKind::JmapKeyword(keyword) | LabelKind::ImapKeyword(keyword) => {
            Some((keyword.as_str().to_string(), None, false))
        }
    }
}

/// Provider dispatch for add-label (assumes local mutation already applied).
async fn add_label_dispatch(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
) -> ActionOutcome {
    add_label_dispatch_inner(ctx, provider, account_id, thread_id, label_id, true).await
}

async fn add_label_dispatch_no_enqueue(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
) -> ActionOutcome {
    add_label_dispatch_inner(ctx, provider, account_id, thread_id, label_id, false).await
}

async fn add_label_dispatch_inner(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
    enqueue_pending: bool,
) -> ActionOutcome {
    let mlog = MutationLog::begin("add_label", account_id, thread_id);
    let params_json = serde_json::json!({"labelId": label_id.as_str()}).to_string();

    let provider_ctx = ActionProviderCtx {
        account_id,
        db: &ctx.db,
        progress: &NoopProgressReporter,
    };
    let label = match label_kind_for_account(ctx, account_id, label_id).await {
        Ok(label) => label,
        Err(e) => {
            let outcome = ActionOutcome::LocalOnly {
                reason: e,
                retryable: false,
            };
            mlog.emit(&outcome);
            return outcome;
        }
    };
    let result = provider.add_label(&provider_ctx, thread_id, &label).await;
    let outcome = match result {
        Ok(()) => ActionOutcome::Success,
        Err(e) => ActionOutcome::LocalOnly {
            reason: ActionError::remote(e.to_string()),
            retryable: true,
        },
    };
    if enqueue_pending {
        enqueue_if_retryable(
            ctx,
            &outcome,
            account_id,
            "addLabel",
            thread_id,
            &params_json,
        )
        .await;
    }
    mlog.emit(&outcome);
    outcome
}

/// Apply a label to a single thread.
pub async fn add_label(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
) -> ActionOutcome {
    let mlog = MutationLog::begin("add_label", account_id, thread_id);
    let params_json = serde_json::json!({"labelId": label_id.as_str()}).to_string();

    if let Err(e) = add_label_local(ctx, account_id, thread_id, label_id).await {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(provider) => add_label_dispatch(ctx, &*provider, account_id, thread_id, label_id).await,
        Err(e) => {
            let outcome = ActionOutcome::LocalOnly {
                reason: ActionError::remote(e),
                retryable: true,
            };
            enqueue_if_retryable(
                ctx,
                &outcome,
                account_id,
                "addLabel",
                thread_id,
                &params_json,
            )
            .await;
            mlog.emit(&outcome);
            outcome
        }
    }
}

/// Add label with a pre-constructed provider (for batch reuse).
pub(crate) async fn add_label_with_provider(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
) -> ActionOutcome {
    let mlog = MutationLog::begin("add_label", account_id, thread_id);

    if let Err(e) = add_label_local(ctx, account_id, thread_id, label_id).await {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    add_label_dispatch(ctx, provider, account_id, thread_id, label_id).await
}

/// Add label with a pre-constructed provider without enqueueing a member retry.
///
/// Composite label-group writes enqueue their own retry row after the member
/// loop, because only the composite retry path has the preflight that detects
/// user-reversed intent.
pub(crate) async fn add_label_with_provider_no_enqueue(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
) -> ActionOutcome {
    let mlog = MutationLog::begin("add_label", account_id, thread_id);

    if let Err(e) = add_label_local(ctx, account_id, thread_id, label_id).await {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    add_label_dispatch_no_enqueue(ctx, provider, account_id, thread_id, label_id).await
}

/// Local DB mutation for remove-label: validate label exists, then delete from
/// `thread_labels` (idempotent).
pub(crate) async fn remove_label_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
) -> Result<(), ActionError> {
    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let lid = label_id.as_str().to_string();
    tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn
            .lock()
            .map_err(|e| ActionError::db(format!("db lock: {e}")))?;

        let exists = db::db::queries_extra::action_helpers::label_exists_sync(&conn, &lid, &aid)
            .map_err(|e| ActionError::db(format!("label lookup: {e}")))?;
        if !exists {
            return Err(ActionError::not_found("label not found for this account"));
        }

        db::db::queries_extra::remove_label(&conn, &aid, &tid, &lid).map_err(ActionError::db)?;

        Ok(())
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r)
}

/// Provider dispatch for remove-label (assumes local mutation already applied).
async fn remove_label_dispatch(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
) -> ActionOutcome {
    remove_label_dispatch_inner(ctx, provider, account_id, thread_id, label_id, true).await
}

async fn remove_label_dispatch_no_enqueue(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
) -> ActionOutcome {
    remove_label_dispatch_inner(ctx, provider, account_id, thread_id, label_id, false).await
}

async fn remove_label_dispatch_inner(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
    enqueue_pending: bool,
) -> ActionOutcome {
    let mlog = MutationLog::begin("remove_label", account_id, thread_id);
    let params_json = serde_json::json!({"labelId": label_id.as_str()}).to_string();

    let provider_ctx = ActionProviderCtx {
        account_id,
        db: &ctx.db,
        progress: &NoopProgressReporter,
    };
    let label = match label_kind_for_account(ctx, account_id, label_id).await {
        Ok(label) => label,
        Err(e) => {
            let outcome = ActionOutcome::LocalOnly {
                reason: e,
                retryable: false,
            };
            mlog.emit(&outcome);
            return outcome;
        }
    };
    let result = provider
        .remove_label(&provider_ctx, thread_id, &label)
        .await;
    let outcome = match result {
        Ok(()) => ActionOutcome::Success,
        Err(e) => ActionOutcome::LocalOnly {
            reason: ActionError::remote(e.to_string()),
            retryable: true,
        },
    };
    if enqueue_pending {
        enqueue_if_retryable(
            ctx,
            &outcome,
            account_id,
            "removeLabel",
            thread_id,
            &params_json,
        )
        .await;
    }
    mlog.emit(&outcome);
    outcome
}

/// Remove a label from a single thread.
pub async fn remove_label(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
) -> ActionOutcome {
    let mlog = MutationLog::begin("remove_label", account_id, thread_id);
    let params_json = serde_json::json!({"labelId": label_id.as_str()}).to_string();

    if let Err(e) = remove_label_local(ctx, account_id, thread_id, label_id).await {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(provider) => {
            remove_label_dispatch(ctx, &*provider, account_id, thread_id, label_id).await
        }
        Err(e) => {
            let outcome = ActionOutcome::LocalOnly {
                reason: ActionError::remote(e),
                retryable: true,
            };
            enqueue_if_retryable(
                ctx,
                &outcome,
                account_id,
                "removeLabel",
                thread_id,
                &params_json,
            )
            .await;
            mlog.emit(&outcome);
            outcome
        }
    }
}

/// Remove label with a pre-constructed provider (for batch reuse).
pub(crate) async fn remove_label_with_provider(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
) -> ActionOutcome {
    let mlog = MutationLog::begin("remove_label", account_id, thread_id);

    if let Err(e) = remove_label_local(ctx, account_id, thread_id, label_id).await {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    remove_label_dispatch(ctx, provider, account_id, thread_id, label_id).await
}

/// Remove label with a pre-constructed provider without enqueueing a member
/// retry. See `add_label_with_provider_no_enqueue`.
pub(crate) async fn remove_label_with_provider_no_enqueue(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
) -> ActionOutcome {
    let mlog = MutationLog::begin("remove_label", account_id, thread_id);

    if let Err(e) = remove_label_local(ctx, account_id, thread_id, label_id).await {
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    remove_label_dispatch_no_enqueue(ctx, provider, account_id, thread_id, label_id).await
}
