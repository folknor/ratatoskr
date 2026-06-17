use common::ops::ProviderOps;
use common::typed_ids::{LabelGroupId, LabelId};

use super::context::ActionContext;
use super::label;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome, RemoteFailureKind};
use super::pending::enqueue_if_retryable_with_id;
use super::provider::{classify_provider_error, create_provider};
use db::db::WriteTarget;
use db::db::queries_extra::{PendingLabelIntent, PendingLabelIntentOp};

/// Distinguishes initial dispatch from a pending-ops drain retry. The
/// retry preflight short-circuits a queued composite whose intent has
/// been reversed by the user since it landed: an Apply retry whose
/// group is no longer rendered on the thread (overlay-aware) skips its
/// queued member `AddLabel` dispatches, and vice versa for Remove. This
/// prevents stale retries from resurrecting or re-clearing a pill
/// against current user intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DispatchKind {
    Initial,
    Retry,
}

/// Result of the local-DB step of a composite group write. `Skip` signals
/// that the user reversed intent between the original action and a retry
/// drain - the caller resolves to `Success` without dispatching any
/// per-member writes. The `Proceed` payload carries the composite-captured
/// `generation_seen` so the per-member dispatchers and the composite
/// attach / clear paths all key against the same intent snapshot.
enum LocalStep {
    Proceed {
        labels: Vec<LabelId>,
        generation_seen: i64,
    },
    Skip,
}

pub(crate) async fn apply_label_group_local_initial(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
) -> Result<(), ActionError> {
    apply_label_group_local(ctx, account_id, thread_id, group_id, DispatchKind::Initial)
        .await
        .map(|_| ())
}

pub(crate) async fn remove_label_group_local_initial(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
) -> Result<(), ActionError> {
    remove_label_group_local(ctx, account_id, thread_id, group_id, DispatchKind::Initial)
        .await
        .map(|_| ())
}

async fn apply_label_group_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
    kind: DispatchKind,
) -> Result<LocalStep, ActionError> {
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    db.with_write_mapped(
        move |conn| {
            let exists: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM label_groups WHERE id = ?1",
                    rusqlite::params![group_id.as_i64()],
                    |row| row.get(0),
                )
                .map_err(|e| ActionError::db(format!("label group lookup: {e}")))?;
            if exists == 0 {
                return Err(ActionError::not_found("label group not found"));
            }

            if kind == DispatchKind::Retry {
                // User reversed intent: queued `applyLabelGroup` is no longer
                // current. Skip member dispatch and resolve as success.
                if !thread_renders_group_for_user(conn, &aid, &tid, group_id)? {
                    return Ok(LocalStep::Skip);
                }
            }

            let labels = read_group_member_labels(conn, &aid, group_id)?;
            let generation_seen =
                upsert_group_intents(conn, &aid, &tid, &labels, PendingLabelIntentOp::Add)?;
            Ok(LocalStep::Proceed {
                labels,
                generation_seen,
            })
        },
        ActionError::db,
    )
    .await
}

async fn remove_label_group_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
    kind: DispatchKind,
) -> Result<LocalStep, ActionError> {
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    db.with_write_mapped(
        move |conn| {
            if kind == DispatchKind::Retry {
                // User re-applied the group after the queued `removeLabelGroup`.
                // Skip member RemoveLabel dispatches.
                if thread_renders_group_for_user(conn, &aid, &tid, group_id)? {
                    return Ok(LocalStep::Skip);
                }
            }

            let labels = read_applied_group_member_labels(conn, &aid, &tid, group_id)?;
            let generation_seen =
                upsert_group_intents(conn, &aid, &tid, &labels, PendingLabelIntentOp::Remove)?;
            Ok(LocalStep::Proceed {
                labels,
                generation_seen,
            })
        },
        ActionError::db,
    )
    .await
}

fn thread_renders_group_for_user(
    conn: &impl WriteTarget,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
) -> Result<bool, ActionError> {
    let fragment = db::db::queries_extra::user_visible_label_group_rendered_fragment(
        "t.account_id",
        "t.id",
        "lg.id = ?3",
    );
    let sql = format!(
        "SELECT EXISTS (
           SELECT 1 FROM threads t
           WHERE t.account_id = ?1
             AND t.id = ?2
             AND {fragment}
         )"
    );
    conn.query_row(
        &sql,
        rusqlite::params![account_id, thread_id, group_id.as_i64()],
        |row| row.get::<_, i64>(0),
    )
    .map(|value| value != 0)
    .map_err(|e| ActionError::db(format!("group render preflight: {e}")))
}

fn upsert_group_intents(
    conn: &impl WriteTarget,
    account_id: &str,
    thread_id: &str,
    labels: &[LabelId],
    op: PendingLabelIntentOp,
) -> Result<i64, ActionError> {
    db::db::queries_extra::upsert_pending_thread_label_intents(
        conn,
        account_id,
        thread_id,
        labels.iter().map(|label_id| PendingLabelIntent {
            label_id: label_id.as_str(),
            op,
        }),
        None,
    )
    .map_err(ActionError::db)
}

pub(crate) async fn apply_label_group_with_provider(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
) -> ActionOutcome {
    apply_label_group_with_provider_kind(
        ctx,
        provider,
        account_id,
        thread_id,
        group_id,
        DispatchKind::Initial,
    )
    .await
}

pub(crate) async fn apply_label_group_with_provider_retry(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
) -> ActionOutcome {
    apply_label_group_with_provider_kind(
        ctx,
        provider,
        account_id,
        thread_id,
        group_id,
        DispatchKind::Retry,
    )
    .await
}

async fn apply_label_group_with_provider_kind(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
    kind: DispatchKind,
) -> ActionOutcome {
    let mlog = MutationLog::begin("apply_label_group", account_id, thread_id);
    let (labels, generation_seen) =
        match apply_label_group_local(ctx, account_id, thread_id, group_id, kind).await {
            Ok(LocalStep::Proceed {
                labels,
                generation_seen,
            }) => (labels, generation_seen),
            Ok(LocalStep::Skip) => {
                let outcome = ActionOutcome::Success;
                mlog.emit(&outcome);
                return outcome;
            }
            Err(e) => {
                let outcome = ActionOutcome::Failed { error: e };
                mlog.emit(&outcome);
                return outcome;
            }
        };
    let outcome = dispatch_member_ops(
        ctx,
        provider,
        account_id,
        thread_id,
        labels.clone(),
        generation_seen,
        true,
    )
    .await;
    finalize_composite_outcome(
        ctx,
        &outcome,
        account_id,
        thread_id,
        group_id,
        true,
        &labels,
        generation_seen,
    )
    .await;
    mlog.emit(&outcome);
    outcome
}

#[cfg(test)]
pub(crate) async fn apply_label_group(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
) -> ActionOutcome {
    apply_label_group_with_kind(ctx, account_id, thread_id, group_id, DispatchKind::Initial).await
}

pub(crate) async fn apply_label_group_retry(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
) -> ActionOutcome {
    apply_label_group_with_kind(ctx, account_id, thread_id, group_id, DispatchKind::Retry).await
}

async fn apply_label_group_with_kind(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
    kind: DispatchKind,
) -> ActionOutcome {
    let mlog = MutationLog::begin("apply_label_group", account_id, thread_id);
    match create_provider(&ctx.db, &ctx.write_db, account_id, ctx.encryption_key).await {
        Ok(provider) => {
            let outcome = apply_label_group_with_provider_kind(
                ctx, &*provider, account_id, thread_id, group_id, kind,
            )
            .await;
            mlog.emit(&outcome);
            outcome
        }
        Err(e) => match apply_label_group_local(ctx, account_id, thread_id, group_id, kind).await {
            Ok(LocalStep::Skip) => {
                let outcome = ActionOutcome::Success;
                mlog.emit(&outcome);
                outcome
            }
            Ok(LocalStep::Proceed {
                labels,
                generation_seen,
            }) => {
                let outcome = ActionOutcome::LocalOnly {
                    reason: ActionError::remote(e.clone()),
                    retryable: composite_retryability(&e),
                };
                finalize_composite_outcome(
                    ctx,
                    &outcome,
                    account_id,
                    thread_id,
                    group_id,
                    true,
                    &labels,
                    generation_seen,
                )
                .await;
                mlog.emit(&outcome);
                outcome
            }
            Err(le) => {
                let outcome = ActionOutcome::Failed { error: le };
                mlog.emit(&outcome);
                outcome
            }
        },
    }
}

pub(crate) async fn remove_label_group_with_provider(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
) -> ActionOutcome {
    remove_label_group_with_provider_kind(
        ctx,
        provider,
        account_id,
        thread_id,
        group_id,
        DispatchKind::Initial,
    )
    .await
}

pub(crate) async fn remove_label_group_with_provider_retry(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
) -> ActionOutcome {
    remove_label_group_with_provider_kind(
        ctx,
        provider,
        account_id,
        thread_id,
        group_id,
        DispatchKind::Retry,
    )
    .await
}

async fn remove_label_group_with_provider_kind(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
    kind: DispatchKind,
) -> ActionOutcome {
    let mlog = MutationLog::begin("remove_label_group", account_id, thread_id);
    let (labels, generation_seen) =
        match remove_label_group_local(ctx, account_id, thread_id, group_id, kind).await {
            Ok(LocalStep::Proceed {
                labels,
                generation_seen,
            }) => (labels, generation_seen),
            Ok(LocalStep::Skip) => {
                let outcome = ActionOutcome::Success;
                mlog.emit(&outcome);
                return outcome;
            }
            Err(e) => {
                let outcome = ActionOutcome::Failed { error: e };
                mlog.emit(&outcome);
                return outcome;
            }
        };
    let outcome = dispatch_member_ops(
        ctx,
        provider,
        account_id,
        thread_id,
        labels.clone(),
        generation_seen,
        false,
    )
    .await;
    finalize_composite_outcome(
        ctx,
        &outcome,
        account_id,
        thread_id,
        group_id,
        false,
        &labels,
        generation_seen,
    )
    .await;
    mlog.emit(&outcome);
    outcome
}

#[cfg(test)]
pub(crate) async fn remove_label_group(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
) -> ActionOutcome {
    remove_label_group_with_kind(ctx, account_id, thread_id, group_id, DispatchKind::Initial).await
}

pub(crate) async fn remove_label_group_retry(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
) -> ActionOutcome {
    remove_label_group_with_kind(ctx, account_id, thread_id, group_id, DispatchKind::Retry).await
}

async fn remove_label_group_with_kind(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
    kind: DispatchKind,
) -> ActionOutcome {
    let mlog = MutationLog::begin("remove_label_group", account_id, thread_id);
    match create_provider(&ctx.db, &ctx.write_db, account_id, ctx.encryption_key).await {
        Ok(provider) => {
            let outcome = remove_label_group_with_provider_kind(
                ctx, &*provider, account_id, thread_id, group_id, kind,
            )
            .await;
            mlog.emit(&outcome);
            outcome
        }
        Err(e) => {
            match remove_label_group_local(ctx, account_id, thread_id, group_id, kind).await {
                Ok(LocalStep::Skip) => {
                    let outcome = ActionOutcome::Success;
                    mlog.emit(&outcome);
                    outcome
                }
                Ok(LocalStep::Proceed {
                    labels,
                    generation_seen,
                }) => {
                    let outcome = ActionOutcome::LocalOnly {
                        reason: ActionError::remote(e.clone()),
                        retryable: composite_retryability(&e),
                    };
                    finalize_composite_outcome(
                        ctx,
                        &outcome,
                        account_id,
                        thread_id,
                        group_id,
                        false,
                        &labels,
                        generation_seen,
                    )
                    .await;
                    mlog.emit(&outcome);
                    outcome
                }
                Err(le) => {
                    let outcome = ActionOutcome::Failed { error: le };
                    mlog.emit(&outcome);
                    outcome
                }
            }
        }
    }
}

fn composite_retryability(error: &str) -> bool {
    !matches!(classify_provider_error(error), RemoteFailureKind::Permanent,)
}

/// Per-member provider dispatch. Runs each member write through an explicit
/// no-enqueue helper, so a per-member failure does NOT enqueue a raw
/// `addLabel` / `removeLabel` row. Those would bypass the composite retry
/// preflight described on `DispatchKind` above. Instead the composite
/// caller enqueues a single composite-typed row covering the failed
/// members via [`enqueue_composite_if_local_only`].
///
/// Continues past per-member `Failed` outcomes so a single hard error
/// (e.g. a member whose label row was deleted between member-read and
/// dispatch) does not abandon the remaining members. LocalOnly takes
/// precedence over Failed so the composite retry path activates whenever
/// any member is retryable.
async fn dispatch_member_ops(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
    labels: Vec<LabelId>,
    generation_seen: i64,
    apply: bool,
) -> ActionOutcome {
    let mut saw_local_only = false;
    let mut last_failed: Option<ActionError> = None;
    for label_id in labels {
        let outcome = if apply {
            label::add_label_with_provider_no_enqueue(
                ctx,
                provider,
                account_id,
                thread_id,
                &label_id,
                generation_seen,
            )
            .await
        } else {
            label::remove_label_with_provider_no_enqueue(
                ctx,
                provider,
                account_id,
                thread_id,
                &label_id,
                generation_seen,
            )
            .await
        };
        match outcome {
            ActionOutcome::Success | ActionOutcome::NoOp => {}
            ActionOutcome::LocalOnly {
                retryable: true, ..
            } => saw_local_only = true,
            ActionOutcome::LocalOnly {
                retryable: false,
                reason,
            } => {
                // Member dispatcher already cleared its own intent for the
                // permanent failure; the composite surfaces it as Failed.
                last_failed = Some(reason);
            }
            ActionOutcome::Failed { error } => last_failed = Some(error),
        }
    }
    if saw_local_only {
        ActionOutcome::LocalOnly {
            reason: ActionError::remote("one or more label group member writes failed"),
            retryable: true,
        }
    } else if let Some(error) = last_failed {
        ActionOutcome::Failed { error }
    } else {
        ActionOutcome::Success
    }
}

#[allow(clippy::too_many_arguments)]
async fn finalize_composite_outcome(
    ctx: &ActionContext,
    outcome: &ActionOutcome,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
    apply: bool,
    labels: &[LabelId],
    generation_seen: i64,
) {
    match outcome {
        ActionOutcome::LocalOnly {
            retryable: true, ..
        } => {
            let op_name = if apply {
                "applyLabelGroup"
            } else {
                "removeLabelGroup"
            };
            let intent_op = if apply {
                PendingLabelIntentOp::Add
            } else {
                PendingLabelIntentOp::Remove
            };
            if let Some(action_id) =
                enqueue_composite(ctx, account_id, thread_id, op_name, group_id).await
            {
                attach_group_action_id(
                    ctx,
                    account_id,
                    thread_id,
                    labels,
                    intent_op,
                    generation_seen,
                    action_id,
                )
                .await;
            }
        }
        ActionOutcome::LocalOnly {
            retryable: false, ..
        } => {
            // Composite-level permanent failure (e.g. permanent
            // create_provider error): tear down every member intent
            // the local step wrote rather than waiting for the stale
            // sweep.
            clear_group_intents_immediate(
                ctx,
                account_id,
                thread_id,
                labels,
                generation_seen,
                apply,
            )
            .await;
        }
        ActionOutcome::Success | ActionOutcome::NoOp | ActionOutcome::Failed { .. } => {}
    }
}

async fn clear_group_intents_immediate(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    labels: &[LabelId],
    generation_seen: i64,
    apply: bool,
) {
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let label_ids: Vec<String> = labels
        .iter()
        .map(|label_id| label_id.as_str().to_string())
        .collect();
    let op = if apply {
        PendingLabelIntentOp::Add
    } else {
        PendingLabelIntentOp::Remove
    };
    if let Err(e) = db
        .with_write(move |conn| {
            db::db::queries_extra::delete_pending_thread_label_intents_for_labels(
                conn,
                &aid,
                &tid,
                label_ids
                    .iter()
                    .map(|label_id| PendingLabelIntent { label_id, op }),
                generation_seen,
            )
        })
        .await
    {
        log::warn!("[actions] clear composite intents on permanent fail: {e}");
    }
}

async fn enqueue_composite(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    operation_type: &str,
    group_id: LabelGroupId,
) -> Option<String> {
    let params_json = serde_json::json!({"groupId": group_id.as_i64()}).to_string();
    let outcome = ActionOutcome::LocalOnly {
        reason: ActionError::remote("composite retry"),
        retryable: true,
    };
    enqueue_if_retryable_with_id(
        ctx,
        &outcome,
        account_id,
        operation_type,
        thread_id,
        &params_json,
    )
    .await
}

async fn attach_group_action_id(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    labels: &[LabelId],
    op: PendingLabelIntentOp,
    generation_seen: i64,
    action_id: String,
) {
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let label_ids: Vec<String> = labels
        .iter()
        .map(|label_id| label_id.as_str().to_string())
        .collect();
    if let Err(e) = db
        .with_write(move |conn| {
            db::db::queries_extra::attach_action_id_to_pending_thread_label_intents(
                conn,
                &aid,
                &tid,
                label_ids
                    .iter()
                    .map(|label_id| PendingLabelIntent { label_id, op }),
                generation_seen,
                &action_id,
            )
        })
        .await
    {
        log::warn!("[actions] attach composite label intent action id failed: {e}");
    }
}

fn read_group_member_labels(
    conn: &impl WriteTarget,
    account_id: &str,
    group_id: LabelGroupId,
) -> Result<Vec<LabelId>, ActionError> {
    let mut stmt = conn
        .prepare(
            "SELECT label_id FROM label_group_members
             WHERE account_id = ?1 AND group_id = ?2
             ORDER BY label_id",
        )
        .map_err(|e| ActionError::db(format!("prepare group members: {e}")))?;
    stmt.query_map(rusqlite::params![account_id, group_id.as_i64()], |row| {
        Ok(LabelId(row.get::<_, String>(0)?))
    })
    .map_err(|e| ActionError::db(format!("query group members: {e}")))?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| ActionError::db(format!("map group members: {e}")))
}

fn read_applied_group_member_labels(
    conn: &impl WriteTarget,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
) -> Result<Vec<LabelId>, ActionError> {
    let visible_label =
        db::db::queries_extra::user_visible_label_exists_fragment("?1", "?2", "lgm.label_id");
    let mut stmt = conn
        .prepare(&format!(
            "SELECT lgm.label_id
             FROM label_group_members lgm
             WHERE lgm.account_id = ?1
               AND lgm.group_id = ?3
               AND {visible_label}
             ORDER BY lgm.label_id"
        ))
        .map_err(|e| ActionError::db(format!("prepare applied group labels: {e}")))?;
    stmt.query_map(
        rusqlite::params![account_id, thread_id, group_id.as_i64()],
        |row| Ok(LabelId(row.get::<_, String>(0)?)),
    )
    .map_err(|e| ActionError::db(format!("query applied group labels: {e}")))?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| ActionError::db(format!("map applied group labels: {e}")))
}
