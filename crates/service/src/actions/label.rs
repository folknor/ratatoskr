use bifrost_types::ObjectId;
use common::typed_ids::LabelId;
use types::{LabelKind, MailProviderKind};

use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use super::pending::enqueue_if_retryable_with_id;
use crate::bifrost::resident::ResidentActionAccount;
use db::db::WriteTarget;
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
    db.with_write_mapped(
        move |conn| {
            let label_kind =
                label_kind_for_account_sync(conn, &aid, &lid).map_err(ActionError::db)?;

            let exists = db::db::queries_extra::action_helpers::label_exists_sync(
                &conn.as_read(),
                &lid,
                &aid,
            )
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
        },
        ActionError::db,
    )
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
        intents
            .iter()
            .map(|(label_id, op)| PendingLabelIntent { label_id, op: *op }),
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
    if let Err(e) = db
        .with_write(move |conn| {
            db::db::queries_extra::attach_action_id_to_pending_thread_label_intents(
                conn,
                &aid,
                &tid,
                intents
                    .iter()
                    .map(|(label_id, op)| PendingLabelIntent { label_id, op: *op }),
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
    if let Err(e) = db
        .with_write(move |conn| {
            db::db::queries_extra::delete_pending_thread_label_intents_for_labels(
                conn,
                &aid,
                &tid,
                intents
                    .iter()
                    .map(|(label_id, op)| PendingLabelIntent { label_id, op: *op }),
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
    db.with_write_mapped(
        move |conn| {
            let tx = conn
                .transaction()
                .map_err(|e| ActionError::db(format!("begin confirm tx: {e}")))?;
            db::db::queries_extra::confirmed_provider_label_intents(
                &tx,
                &aid,
                &tid,
                intents
                    .iter()
                    .map(|(label_id, op)| PendingLabelIntent { label_id, op: *op }),
            )
            .map_err(ActionError::db)?;
            tx.commit()
                .map_err(|e| ActionError::db(format!("commit confirm tx: {e}")))
        },
        ActionError::db,
    )
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

fn intent_op(add: bool) -> PendingLabelIntentOp {
    if add {
        PendingLabelIntentOp::Add
    } else {
        PendingLabelIntentOp::Remove
    }
}

/// Apply or remove a single label on a thread through the resident bifrost
/// engine, preserving the optimistic-intent lifecycle: the local step writes
/// the pending thread-label intent, the engine dispatch runs, then on success
/// the intent is confirmed (overlay cleared, truth written), on a retryable
/// failure it is enqueued and the pending action id attached, and on a
/// permanent failure it is cleared immediately (rather than waiting for the
/// 48h stale sweep).
///
/// `action_account == None` is the degraded path (no resident engine handle):
/// the local write lands and a retryable pending row is enqueued, exactly as
/// the legacy provider-construction-failure path behaved.
pub(crate) async fn dispatch_label_via_engine(
    ctx: &ActionContext,
    action_account: Option<&ResidentActionAccount>,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
    add: bool,
) -> ActionOutcome {
    let (op_type, mlog_name) = if add {
        ("addLabel", "add_label")
    } else {
        ("removeLabel", "remove_label")
    };
    let mlog = MutationLog::begin(mlog_name, account_id, thread_id);
    let params_json = serde_json::json!({"labelId": label_id.as_str()}).to_string();

    let local = match label_local_step(ctx, account_id, thread_id, label_id, intent_op(add)).await {
        Ok(local) => local,
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
    };
    let LocalLabelStep {
        label_kind,
        intents,
        generation_seen,
    } = local;

    let outcome = match action_account {
        None => ActionOutcome::LocalOnly {
            reason: ActionError::remote("resident engine unavailable"),
            retryable: true,
        },
        Some(account) => {
            match super::dispatch_target::resolve_thread_messages(
                ctx,
                account_id,
                thread_id,
                account.provider,
            )
            .await
            {
                Err(e) => ActionOutcome::LocalOnly {
                    retryable: e.is_retryable(),
                    reason: e,
                },
                Ok(ids) => match super::dispatch_target::dispatch_label_engine(
                    account,
                    account_id,
                    &label_kind,
                    add,
                    ids,
                )
                .await
                {
                    Ok(()) => {
                        match confirm_provider_intents(ctx, account_id, thread_id, intents.clone())
                            .await
                        {
                            Ok(()) => ActionOutcome::Success,
                            Err(error) => ActionOutcome::LocalOnly {
                                reason: error,
                                retryable: true,
                            },
                        }
                    }
                    Err(e) => ActionOutcome::LocalOnly {
                        retryable: e.is_retryable(),
                        reason: e,
                    },
                },
            }
        }
    };

    finalize_dispatch_outcome(
        ctx,
        account_id,
        thread_id,
        op_type,
        &params_json,
        intents,
        generation_seen,
        &outcome,
        true,
    )
    .await;
    mlog.emit(&outcome);
    outcome
}

/// Per-member engine dispatch for a composite label-group write. Reuses the
/// group-captured `generation_seen` and the group-resolved message `ids`, and
/// does NOT enqueue a per-member retry row (the composite enqueues one
/// composite-typed row covering the failed members - the contract on
/// `label_group::DispatchKind`). Member intents are still confirmed on success
/// and cleared on a permanent failure.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn dispatch_member_label_via_engine(
    ctx: &ActionContext,
    action_account: &ResidentActionAccount,
    account_id: &str,
    thread_id: &str,
    label_id: &LabelId,
    add: bool,
    generation_seen: i64,
    ids: &[ObjectId],
) -> ActionOutcome {
    let (op_type, mlog_name) = if add {
        ("addLabel", "add_label")
    } else {
        ("removeLabel", "remove_label")
    };
    let mlog = MutationLog::begin(mlog_name, account_id, thread_id);
    let params_json = serde_json::json!({"labelId": label_id.as_str()}).to_string();

    let label_kind = match resolve_member_label_kind(ctx, account_id, label_id).await {
        Ok(label_kind) => label_kind,
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
    };
    let intents = label_intents(label_id.as_str(), &label_kind, intent_op(add));

    let outcome = match super::dispatch_target::dispatch_label_engine(
        action_account,
        account_id,
        &label_kind,
        add,
        ids.to_vec(),
    )
    .await
    {
        Ok(()) => match confirm_provider_intents(ctx, account_id, thread_id, intents.clone()).await
        {
            Ok(()) => ActionOutcome::Success,
            Err(error) => ActionOutcome::LocalOnly {
                reason: error,
                retryable: true,
            },
        },
        Err(e) => ActionOutcome::LocalOnly {
            retryable: e.is_retryable(),
            reason: e,
        },
    };

    finalize_dispatch_outcome(
        ctx,
        account_id,
        thread_id,
        op_type,
        &params_json,
        intents,
        generation_seen,
        &outcome,
        false,
    )
    .await;
    mlog.emit(&outcome);
    outcome
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
        ActionOutcome::LocalOnly {
            retryable: true, ..
        } => {
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
        ActionOutcome::LocalOnly {
            retryable: false, ..
        } => {
            // Permanent failure: the action is over. Tear down the
            // optimistic intent now instead of leaving it for the
            // 48h stale-intent sweep.
            clear_pending_intents_immediate(ctx, account_id, thread_id, intents, generation_seen)
                .await;
        }
        ActionOutcome::Success | ActionOutcome::NoOp | ActionOutcome::Failed { .. } => {}
    }
}
