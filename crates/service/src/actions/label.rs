use common::ops::ProviderOps;
use common::typed_ids::LabelId;
use common::types::ActionProviderCtx;
use types::{LabelKind, MailProviderKind};

use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome, RemoteFailureKind};
use super::pending::enqueue_if_retryable_with_id;
use super::provider::{classify_provider_error, create_provider};
use db::db::WriteTarget;
use db::progress::NoopProgressReporter;
use db::db::queries_extra::{PendingLabelIntent, PendingLabelIntentOp};

/// Captured shape of a local-step upsert. The dispatcher needs the parsed
/// label kind (for the provider call), the intent list as actually written
/// (Graph importance expands to two intents), and the generation snapshot
/// the upsert captured (so a later attach / clear keys on the same row).
pub(crate) struct LocalLabelStep {
    label_kind: LabelKind,
    intents: Vec<(String, PendingLabelIntentOp)>,
    generation_seen: i64,
}

/// Local DB mutation for add-label: validate label exists, then write pending
/// user intent. Provider truth is updated only after provider success or sync.
pub(crate) async fn add_label_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
) -> Result<LocalLabelStep, ActionError> {
    label_local_step(
        ctx,
        account_id,
        thread_id,
        label_id,
        PendingLabelIntentOp::Add,
    )
    .await
}

/// Local DB mutation for remove-label: same as `add_label_local` but
/// records a `Remove` intent. Provider truth is updated only after the
/// provider call succeeds or sync converges.
pub(crate) async fn remove_label_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
) -> Result<LocalLabelStep, ActionError> {
    label_local_step(
        ctx,
        account_id,
        thread_id,
        label_id,
        PendingLabelIntentOp::Remove,
    )
    .await
}

async fn label_local_step(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
    op: PendingLabelIntentOp,
) -> Result<LocalLabelStep, ActionError> {
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let lid = label_id.as_str().to_string();
    db.with_write_mapped(move |conn| {
        let label_kind =
            label_kind_for_account_sync(conn, &aid, &lid).map_err(ActionError::db)?;

        let exists = db::db::queries_extra::action_helpers::label_exists_sync(&conn.as_read(), &lid, &aid)
            .map_err(|e| ActionError::db(format!("label lookup: {e}")))?;
        if !exists {
            ensure_typed_tag_label(conn, &aid, &label_kind)
                .map_err(ActionError::db)?
                .ok_or_else(|| ActionError::not_found("label not found for this account"))?;
        }

        if matches!(op, PendingLabelIntentOp::Add)
            && let LabelKind::GraphImportance(level) = label_kind
        {
            let opposite = LabelKind::graph_importance(level.opposite());
            ensure_typed_tag_label(conn, &aid, &opposite).map_err(ActionError::db)?;
        }
        let intents = label_intents(&lid, &label_kind, op);
        let generation_seen = upsert_pending_intents_sync(conn, &aid, &tid, &intents, None)
            .map_err(ActionError::db)?;

        Ok(LocalLabelStep {
            label_kind,
            intents,
            generation_seen,
        })
    }, ActionError::db)
    .await
}

fn label_intents(
    label_id: &str,
    label_kind: &LabelKind,
    op: PendingLabelIntentOp,
) -> Vec<(String, PendingLabelIntentOp)> {
    match (op, label_kind) {
        (PendingLabelIntentOp::Add, LabelKind::GraphImportance(level)) => vec![
            (
                level.opposite().label_id().to_string(),
                PendingLabelIntentOp::Remove,
            ),
            (label_id.to_string(), PendingLabelIntentOp::Add),
        ],
        _ => vec![(label_id.to_string(), op)],
    }
}

fn upsert_pending_intents_sync(
    conn: &impl WriteTarget,
    account_id: &str,
    thread_id: &str,
    intents: &[(String, PendingLabelIntentOp)],
    action_id: Option<&str>,
) -> Result<i64, String> {
    db::db::queries_extra::upsert_pending_thread_label_intents(
        conn,
        account_id,
        thread_id,
        intents.iter().map(|(label_id, op)| PendingLabelIntent {
            label_id,
            op: *op,
        }),
        action_id,
    )
}

async fn attach_pending_action_id(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    intents: Vec<(String, PendingLabelIntentOp)>,
    generation_seen: i64,
    action_id: String,
) {
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    if let Err(e) = db.with_write(move |conn| {
        db::db::queries_extra::attach_action_id_to_pending_thread_label_intents(
            conn,
            &aid,
            &tid,
            intents.iter().map(|(label_id, op)| PendingLabelIntent {
                label_id,
                op: *op,
            }),
            generation_seen,
            &action_id,
        )
    })
    .await
    {
        log::warn!("[actions] attach pending label intent action id failed: {e}");
    }
}

async fn clear_pending_intents_immediate(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    intents: Vec<(String, PendingLabelIntentOp)>,
    generation_seen: i64,
) {
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    if let Err(e) = db.with_write(move |conn| {
        db::db::queries_extra::delete_pending_thread_label_intents_for_labels(
            conn,
            &aid,
            &tid,
            intents.iter().map(|(label_id, op)| PendingLabelIntent {
                label_id,
                op: *op,
            }),
            generation_seen,
        )
    })
    .await
    {
        log::warn!("[actions] clear pending label intent on permanent fail: {e}");
    }
}

async fn confirm_provider_intents(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    intents: Vec<(String, PendingLabelIntentOp)>,
) -> Result<(), ActionError> {
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    db.with_write_mapped(move |conn| {
        let tx = conn
            .transaction()
            .map_err(|e| ActionError::db(format!("begin confirm tx: {e}")))?;
        db::db::queries_extra::confirmed_provider_label_intents(
            &tx,
            &aid,
            &tid,
            intents.iter().map(|(label_id, op)| PendingLabelIntent {
                label_id,
                op: *op,
            }),
        )
        .map_err(ActionError::db)?;
        tx.commit()
            .map_err(|e| ActionError::db(format!("commit confirm tx: {e}")))
    }, ActionError::db)
    .await
}

fn label_kind_for_account_sync(
    conn: &impl WriteTarget,
    account_id: &str,
    label_id: &str,
) -> Result<LabelKind, String> {
    let provider = conn
        .query_row(
            "SELECT provider FROM accounts WHERE id = ?1",
            rusqlite::params![account_id],
            |row| row.get::<_, String>(0),
        )
        .map_err(|e| format!("get account provider: {e}"))?;
    let provider = MailProviderKind::parse(&provider)?;
    LabelKind::parse(label_id, provider)
}

fn ensure_typed_tag_label(
    conn: &impl WriteTarget,
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

/// Classify provider failure for the dispatch decision: permanent failures
/// clear the optimistic intent immediately, retryable failures enqueue and
/// attach the action id.
fn provider_retryability(error: &str) -> bool {
    !matches!(
        classify_provider_error(error),
        RemoteFailureKind::Permanent,
    )
}

/// Provider dispatch for add-label (assumes local mutation already applied).
async fn add_label_dispatch(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
    local: LocalLabelStep,
) -> ActionOutcome {
    add_label_dispatch_inner(ctx, provider, account_id, thread_id, label_id, local, true).await
}

async fn add_label_dispatch_no_enqueue(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
    local: LocalLabelStep,
) -> ActionOutcome {
    add_label_dispatch_inner(ctx, provider, account_id, thread_id, label_id, local, false).await
}

async fn add_label_dispatch_inner(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
    local: LocalLabelStep,
    enqueue_pending: bool,
) -> ActionOutcome {
    let mlog = MutationLog::begin("add_label", account_id, thread_id);
    let params_json = serde_json::json!({"labelId": label_id.as_str()}).to_string();

    let provider_ctx = ActionProviderCtx {
        account_id,
        db: &ctx.db,
        progress: &NoopProgressReporter,
    };
    let LocalLabelStep {
        label_kind,
        intents,
        generation_seen,
    } = local;
    let result = provider
        .add_label(&provider_ctx, thread_id, &label_kind)
        .await;
    let outcome = match result {
        Ok(()) => {
            // Provider accepted the write. If the local confirm (truth-write
            // + generation bump + overlay clear) fails we fall back to
            // retryable LocalOnly so the action is re-driven; provider
            // label add/remove are idempotent, so re-running through the
            // retry queue is safe.
            match confirm_provider_intents(ctx, account_id, thread_id, intents.clone()).await {
                Ok(()) => ActionOutcome::Success,
                Err(error) => ActionOutcome::LocalOnly {
                    reason: error,
                    retryable: true,
                },
            }
        }
        Err(e) => {
            let reason_str = e.to_string();
            ActionOutcome::LocalOnly {
                reason: ActionError::remote(reason_str.clone()),
                retryable: provider_retryability(&reason_str),
            }
        }
    };
    finalize_dispatch_outcome(
        ctx,
        account_id,
        thread_id,
        "addLabel",
        &params_json,
        intents,
        generation_seen,
        &outcome,
        enqueue_pending,
    )
    .await;
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

    let local = match add_label_local(ctx, account_id, thread_id, label_id).await {
        Ok(local) => local,
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
    };

    match create_provider(&ctx.db, &ctx.write_db, account_id, ctx.encryption_key).await {
        Ok(provider) => {
            add_label_dispatch(ctx, &*provider, account_id, thread_id, label_id, local).await
        }
        Err(e) => {
            let outcome = ActionOutcome::LocalOnly {
                reason: ActionError::remote(e.clone()),
                retryable: provider_retryability(&e),
            };
            finalize_dispatch_outcome(
                ctx,
                account_id,
                thread_id,
                "addLabel",
                &params_json,
                local.intents,
                local.generation_seen,
                &outcome,
                true,
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

    let local = match add_label_local(ctx, account_id, thread_id, label_id).await {
        Ok(local) => local,
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
    };

    add_label_dispatch(ctx, provider, account_id, thread_id, label_id, local).await
}

/// Add label with a pre-constructed provider without enqueueing a member retry.
///
/// Composite label-group writes enqueue their own retry row after the member
/// loop, because only the composite retry path has the preflight that detects
/// user-reversed intent. The composite has already upserted the member's
/// pending intent at the group level, so we only validate here and reuse the
/// composite-captured `generation_seen` for finalization.
pub(crate) async fn add_label_with_provider_no_enqueue(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
    generation_seen: i64,
) -> ActionOutcome {
    let mlog = MutationLog::begin("add_label", account_id, thread_id);

    let label_kind = match resolve_member_label_kind(ctx, account_id, label_id).await {
        Ok(label_kind) => label_kind,
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
    };
    let intents = label_intents(label_id.as_str(), &label_kind, PendingLabelIntentOp::Add);
    let local = LocalLabelStep {
        label_kind,
        intents,
        generation_seen,
    };
    add_label_dispatch_no_enqueue(ctx, provider, account_id, thread_id, label_id, local).await
}

/// Provider dispatch for remove-label (assumes local mutation already applied).
async fn remove_label_dispatch(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
    local: LocalLabelStep,
) -> ActionOutcome {
    remove_label_dispatch_inner(ctx, provider, account_id, thread_id, label_id, local, true).await
}

async fn remove_label_dispatch_no_enqueue(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
    local: LocalLabelStep,
) -> ActionOutcome {
    remove_label_dispatch_inner(ctx, provider, account_id, thread_id, label_id, local, false).await
}

async fn remove_label_dispatch_inner(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
    local: LocalLabelStep,
    enqueue_pending: bool,
) -> ActionOutcome {
    let mlog = MutationLog::begin("remove_label", account_id, thread_id);
    let params_json = serde_json::json!({"labelId": label_id.as_str()}).to_string();

    let provider_ctx = ActionProviderCtx {
        account_id,
        db: &ctx.db,
        progress: &NoopProgressReporter,
    };
    let LocalLabelStep {
        label_kind,
        intents,
        generation_seen,
    } = local;
    let result = provider
        .remove_label(&provider_ctx, thread_id, &label_kind)
        .await;
    let outcome = match result {
        Ok(()) => match confirm_provider_intents(ctx, account_id, thread_id, intents.clone()).await
        {
            Ok(()) => ActionOutcome::Success,
            Err(error) => ActionOutcome::LocalOnly {
                reason: error,
                retryable: true,
            },
        },
        Err(e) => {
            let reason_str = e.to_string();
            ActionOutcome::LocalOnly {
                reason: ActionError::remote(reason_str.clone()),
                retryable: provider_retryability(&reason_str),
            }
        }
    };
    finalize_dispatch_outcome(
        ctx,
        account_id,
        thread_id,
        "removeLabel",
        &params_json,
        intents,
        generation_seen,
        &outcome,
        enqueue_pending,
    )
    .await;
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

    let local = match remove_label_local(ctx, account_id, thread_id, label_id).await {
        Ok(local) => local,
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
    };

    match create_provider(&ctx.db, &ctx.write_db, account_id, ctx.encryption_key).await {
        Ok(provider) => {
            remove_label_dispatch(ctx, &*provider, account_id, thread_id, label_id, local).await
        }
        Err(e) => {
            let outcome = ActionOutcome::LocalOnly {
                reason: ActionError::remote(e.clone()),
                retryable: provider_retryability(&e),
            };
            finalize_dispatch_outcome(
                ctx,
                account_id,
                thread_id,
                "removeLabel",
                &params_json,
                local.intents,
                local.generation_seen,
                &outcome,
                true,
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

    let local = match remove_label_local(ctx, account_id, thread_id, label_id).await {
        Ok(local) => local,
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
    };

    remove_label_dispatch(ctx, provider, account_id, thread_id, label_id, local).await
}

/// Remove label with a pre-constructed provider without enqueueing a member
/// retry. See `add_label_with_provider_no_enqueue`.
pub(crate) async fn remove_label_with_provider_no_enqueue(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
    generation_seen: i64,
) -> ActionOutcome {
    let mlog = MutationLog::begin("remove_label", account_id, thread_id);

    let label_kind = match resolve_member_label_kind(ctx, account_id, label_id).await {
        Ok(label_kind) => label_kind,
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
    };
    let intents = label_intents(label_id.as_str(), &label_kind, PendingLabelIntentOp::Remove);
    let local = LocalLabelStep {
        label_kind,
        intents,
        generation_seen,
    };
    remove_label_dispatch_no_enqueue(ctx, provider, account_id, thread_id, label_id, local).await
}

async fn resolve_member_label_kind(
    ctx: &ActionContext,
    account_id: &str,
    label_id: &LabelId,
) -> Result<LabelKind, ActionError> {
    let aid = account_id.to_string();
    let lid = label_id.as_str().to_string();
    ctx.write_db
        .with_write(move |conn| label_kind_for_account_sync(conn, &aid, &lid))
        .await
        .map_err(|e| ActionError::db(format!("label kind lookup: {e}")))
}

#[allow(clippy::too_many_arguments)]
async fn finalize_dispatch_outcome(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    operation_type: &str,
    params_json: &str,
    intents: Vec<(String, PendingLabelIntentOp)>,
    generation_seen: i64,
    outcome: &ActionOutcome,
    enqueue_pending: bool,
) {
    match outcome {
        ActionOutcome::LocalOnly { retryable: true, .. } => {
            if enqueue_pending
                && let Some(action_id) = enqueue_if_retryable_with_id(
                    ctx,
                    outcome,
                    account_id,
                    operation_type,
                    thread_id,
                    params_json,
                )
                .await
            {
                attach_pending_action_id(
                    ctx,
                    account_id,
                    thread_id,
                    intents,
                    generation_seen,
                    action_id,
                )
                .await;
            }
        }
        ActionOutcome::LocalOnly { retryable: false, .. } => {
            // Permanent failure: the action is over. Tear down the
            // optimistic intent now instead of leaving it for the
            // 48h stale-intent sweep.
            clear_pending_intents_immediate(
                ctx,
                account_id,
                thread_id,
                intents,
                generation_seen,
            )
            .await;
        }
        ActionOutcome::Success | ActionOutcome::NoOp | ActionOutcome::Failed { .. } => {}
    }
}
