use common::ops::ProviderOps;
use common::typed_ids::{LabelGroupId, LabelId};

use super::context::ActionContext;
use super::label;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use super::pending::enqueue_if_retryable;
use super::provider::create_provider;

/// Distinguishes initial dispatch from a pending-ops drain retry. The
/// preflight (per `docs/labels-unification/redesign.md` "Retry preflight")
/// short-circuits a retry whose intent has been reversed by the user since
/// the queued composite landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DispatchKind {
    Initial,
    Retry,
}

/// Result of the local-DB step of a composite group write. `Skip` signals
/// that the user reversed intent between the original action and a retry
/// drain - the caller resolves to `Success` without dispatching any
/// per-member writes.
enum LocalStep {
    Proceed(Vec<LabelId>),
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
    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn
            .lock()
            .map_err(|e| ActionError::db(format!("db lock: {e}")))?;
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
            let attached: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM thread_label_groups \
                     WHERE account_id = ?1 AND thread_id = ?2 AND group_id = ?3",
                    rusqlite::params![aid, tid, group_id.as_i64()],
                    |row| row.get(0),
                )
                .map_err(|e| {
                    ActionError::db(format!("retry preflight thread_label_groups: {e}"))
                })?;
            if attached == 0 {
                return Ok(LocalStep::Skip);
            }
        }

        conn.execute(
            "INSERT OR IGNORE INTO thread_label_groups (account_id, thread_id, group_id)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![aid, tid, group_id.as_i64()],
        )
        .map_err(|e| ActionError::db(format!("insert thread label group: {e}")))?;
        let labels = read_group_member_labels(&conn, &aid, group_id)?;
        Ok(LocalStep::Proceed(labels))
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r)
}

async fn remove_label_group_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
    kind: DispatchKind,
) -> Result<LocalStep, ActionError> {
    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn
            .lock()
            .map_err(|e| ActionError::db(format!("db lock: {e}")))?;

        if kind == DispatchKind::Retry {
            // User re-applied the group after the queued `removeLabelGroup`.
            // Skip member RemoveLabel dispatches.
            let attached: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM thread_label_groups \
                     WHERE account_id = ?1 AND thread_id = ?2 AND group_id = ?3",
                    rusqlite::params![aid, tid, group_id.as_i64()],
                    |row| row.get(0),
                )
                .map_err(|e| {
                    ActionError::db(format!("retry preflight thread_label_groups: {e}"))
                })?;
            if attached > 0 {
                return Ok(LocalStep::Skip);
            }
        }

        let labels = read_applied_group_member_labels(&conn, &aid, &tid, group_id)?;
        conn.execute(
            "DELETE FROM thread_label_groups
             WHERE account_id = ?1 AND thread_id = ?2 AND group_id = ?3",
            rusqlite::params![aid, tid, group_id.as_i64()],
        )
        .map_err(|e| ActionError::db(format!("delete thread label group: {e}")))?;
        Ok(LocalStep::Proceed(labels))
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r)
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
    let labels = match apply_label_group_local(ctx, account_id, thread_id, group_id, kind).await {
        Ok(LocalStep::Proceed(labels)) => labels,
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
    let outcome = dispatch_member_ops(ctx, provider, account_id, thread_id, labels, true).await;
    enqueue_composite_if_local_only(ctx, &outcome, account_id, thread_id, group_id, true).await;
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
    match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(provider) => {
            let outcome = apply_label_group_with_provider_kind(
                ctx,
                &*provider,
                account_id,
                thread_id,
                group_id,
                kind,
            )
            .await;
            mlog.emit(&outcome);
            outcome
        }
        Err(e) => {
            match apply_label_group_local(ctx, account_id, thread_id, group_id, kind).await {
                Ok(LocalStep::Skip) => {
                    let outcome = ActionOutcome::Success;
                    mlog.emit(&outcome);
                    outcome
                }
                Ok(LocalStep::Proceed(_)) => {
                    let outcome = ActionOutcome::LocalOnly {
                        reason: ActionError::remote(e),
                        retryable: true,
                    };
                    enqueue_composite(ctx, account_id, thread_id, "applyLabelGroup", group_id)
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
    let labels = match remove_label_group_local(ctx, account_id, thread_id, group_id, kind).await {
        Ok(LocalStep::Proceed(labels)) => labels,
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
    let outcome = dispatch_member_ops(ctx, provider, account_id, thread_id, labels, false).await;
    enqueue_composite_if_local_only(ctx, &outcome, account_id, thread_id, group_id, false).await;
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
    match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(provider) => {
            let outcome = remove_label_group_with_provider_kind(
                ctx,
                &*provider,
                account_id,
                thread_id,
                group_id,
                kind,
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
                Ok(LocalStep::Proceed(_)) => {
                    let outcome = ActionOutcome::LocalOnly {
                        reason: ActionError::remote(e),
                        retryable: true,
                    };
                    enqueue_composite(ctx, account_id, thread_id, "removeLabelGroup", group_id)
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

/// Per-member provider dispatch. Runs each `add_label_with_provider` /
/// `remove_label_with_provider` against a clone of the context with
/// `suppress_pending_enqueue = true`, so a per-member failure does NOT
/// enqueue a raw `addLabel` / `removeLabel` row. Those would bypass the
/// composite retry preflight (`docs/labels-unification/redesign.md`
/// "Retry preflight"). Instead the composite caller enqueues a single
/// composite-typed row covering the failed members via
/// [`enqueue_composite_if_local_only`].
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
    apply: bool,
) -> ActionOutcome {
    let mut member_ctx = ctx.clone();
    member_ctx.suppress_pending_enqueue = true;

    let mut saw_local_only = false;
    let mut last_failed: Option<ActionError> = None;
    for label_id in labels {
        let outcome = if apply {
            label::add_label_with_provider(&member_ctx, provider, account_id, thread_id, &label_id)
                .await
        } else {
            label::remove_label_with_provider(
                &member_ctx,
                provider,
                account_id,
                thread_id,
                &label_id,
            )
            .await
        };
        match outcome {
            ActionOutcome::Success | ActionOutcome::NoOp => {}
            ActionOutcome::LocalOnly { .. } => saw_local_only = true,
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

async fn enqueue_composite_if_local_only(
    ctx: &ActionContext,
    outcome: &ActionOutcome,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
    apply: bool,
) {
    if matches!(outcome, ActionOutcome::LocalOnly { .. }) {
        let op = if apply { "applyLabelGroup" } else { "removeLabelGroup" };
        enqueue_composite(ctx, account_id, thread_id, op, group_id).await;
    }
}

async fn enqueue_composite(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    operation_type: &str,
    group_id: LabelGroupId,
) {
    let params_json = serde_json::json!({"groupId": group_id.as_i64()}).to_string();
    let outcome = ActionOutcome::LocalOnly {
        reason: ActionError::remote("composite retry"),
        retryable: true,
    };
    enqueue_if_retryable(
        ctx,
        &outcome,
        account_id,
        operation_type,
        thread_id,
        &params_json,
    )
    .await;
}

fn read_group_member_labels(
    conn: &rusqlite::Connection,
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
    conn: &rusqlite::Connection,
    account_id: &str,
    thread_id: &str,
    group_id: LabelGroupId,
) -> Result<Vec<LabelId>, ActionError> {
    let mut stmt = conn
        .prepare(
            "SELECT tl.label_id
             FROM thread_labels tl
             INNER JOIN label_group_members lgm
               ON lgm.account_id = tl.account_id AND lgm.label_id = tl.label_id
             WHERE tl.account_id = ?1
               AND tl.thread_id = ?2
               AND lgm.group_id = ?3
             ORDER BY tl.label_id",
        )
        .map_err(|e| ActionError::db(format!("prepare applied group labels: {e}")))?;
    stmt.query_map(
        rusqlite::params![account_id, thread_id, group_id.as_i64()],
        |row| Ok(LabelId(row.get::<_, String>(0)?)),
    )
    .map_err(|e| ActionError::db(format!("query applied group labels: {e}")))?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| ActionError::db(format!("map applied group labels: {e}")))
}
