//! Batch action executor - groups targets by account and dispatches
//! provider-backed mail mutations through the resident bifrost engine.
//! Parallel across accounts,
//! sequential within each account.

use std::collections::HashMap;
use std::time::Instant;

use bifrost_types::ObjectId;
use common::types::NamespaceAttribution;

use super::context::ActionContext;
use super::dispatch_target::{
    RemoteBatchKey, dispatch_bulk_mutation, dispatch_mutation, outcome_from_remote_result,
    resolve_thread_messages,
};
use super::log::MutationLog;
use super::operation::MailOperation;
use super::outcome::{ActionError, ActionOutcome, RemoteFailureKind};
use super::pending::enqueue_if_retryable;
use super::{
    archive, label, label_group, mark_read, move_to_folder, mute, permanent_delete, pin, snooze,
    spam, star, trash,
};
use crate::sync::SyncRuntime;

/// Execute operations across multiple threads.
///
/// Each entry is `(account_id, thread_id, operation)` - per-target operations
/// support mixed-value toggles and heterogeneous batches.
///
/// Groups by account, resolves one resident bifrost engine handle per
/// account, dispatches sequentially within each account. Accounts run in
/// parallel.
///
/// **Outcome ordering:** `outcomes[i]` corresponds to `operations[i]` regardless
/// of internal regrouping. All undo/rollback bookkeeping must key off original
/// operation order.
pub async fn batch_execute(
    ctx: &ActionContext,
    sync_runtime: Option<&SyncRuntime>,
    operations: Vec<(String, String, MailOperation)>,
) -> Vec<ActionOutcome> {
    crate::test_counters::delay_if_configured("action.batch_execute").await;

    crate::test_counters::record("action.batch_execute");

    let started = Instant::now();
    let total = operations.len();

    // Group by account, preserving original indices
    let mut groups: HashMap<String, Vec<(usize, String, MailOperation)>> = HashMap::new();
    for (i, (account_id, thread_id, op)) in operations.iter().enumerate() {
        groups
            .entry(account_id.clone())
            .or_default()
            .push((i, thread_id.clone(), op.clone()));
    }
    let account_count = groups.len();

    // Dispatch per-account groups in parallel
    let account_futures: Vec<_> = groups
        .into_iter()
        .map(|(account_id, thread_ops)| {
            let ctx = ctx.clone();
            async move { execute_account_group(&ctx, sync_runtime, &account_id, thread_ops).await }
        })
        .collect();
    let group_results = futures::future::join_all(account_futures).await;

    // Reassemble in original order
    let mut outcomes = Vec::with_capacity(total);
    outcomes.resize_with(total, || ActionOutcome::Failed {
        error: ActionError::invalid_state("batch reassembly bug"),
    });
    for group in group_results {
        for (idx, outcome) in group {
            outcomes[idx] = outcome;
        }
    }

    // Batch summary log
    let success = outcomes.iter().filter(|o| o.is_success()).count();
    let local_only = outcomes.iter().filter(|o| o.is_local_only()).count();
    let failed = outcomes.iter().filter(|o| o.is_failed()).count();
    let elapsed = started.elapsed().as_millis();
    log::info!(
        "[action-batch] {total} ops / {account_count} accounts | \
         {success} ok, {local_only} local-only, {failed} failed | {elapsed}ms"
    );

    outcomes
}

/// Execute one account's thread group.
async fn execute_account_group(
    ctx: &ActionContext,
    sync_runtime: Option<&SyncRuntime>,
    account_id: &str,
    thread_ops: Vec<(usize, String, MailOperation)>,
) -> Vec<(usize, ActionOutcome)> {
    let mut results = Vec::with_capacity(thread_ops.len());

    // Check if ALL operations are local-only (pin/mute/snooze)
    let all_local_only = thread_ops.iter().all(|(_, _, op)| is_local_only(op));

    if all_local_only {
        for (idx, thread_id, op) in thread_ops {
            let _guard = match ctx.try_acquire_flight(account_id, &thread_id) {
                Some(g) => g,
                None => {
                    results.push((
                        idx,
                        ActionOutcome::Failed {
                            error: ActionError::invalid_state(
                                "action already in flight for this thread",
                            ),
                        },
                    ));
                    continue;
                }
            };
            let outcome = dispatch_local_only(ctx, &op, account_id, &thread_id).await;
            results.push((idx, outcome));
        }
        return results;
    }

    let Some(sync_runtime) = sync_runtime else {
        return degraded_fallback(ctx, account_id, "resident engine unavailable", thread_ops).await;
    };
    let action_account = match sync_runtime.resident_action_account(account_id).await {
        Ok(account) => account,
        Err(error) => return degraded_fallback(ctx, account_id, &error, thread_ops).await,
    };

    let use_bulk = thread_ops.len() > 1;
    let mut guards = Vec::with_capacity(thread_ops.len());
    let mut remote_batches: HashMap<RemoteBatchKey, Vec<PendingRemote>> = HashMap::new();
    for (idx, thread_id, op) in thread_ops {
        let guard = match ctx.try_acquire_flight(account_id, &thread_id) {
            Some(g) => g,
            None => {
                results.push((
                    idx,
                    ActionOutcome::Failed {
                        error: ActionError::invalid_state(
                            "action already in flight for this thread",
                        ),
                    },
                ));
                continue;
            }
        };
        guards.push(guard);

        let namespace = match namespace_preflight(ctx, account_id, &thread_id, &op).await {
            Ok(namespace) => namespace,
            Err(error) => {
                results.push((idx, ActionOutcome::Failed { error }));
                continue;
            }
        };

        // Remote batches are deliberately personal-only. A batch key without
        // namespace would combine a personal archive with a shared archive,
        // and system-role resolution would then be wrong for one of them.
        // Shared actions remain fully remote, just one thread at a time.
        if !use_bulk || namespace != NamespaceAttribution::Personal {
            let outcome =
                dispatch_with_engine(ctx, &action_account, &op, account_id, &thread_id).await;
            results.push((idx, outcome));
            continue;
        }

        if is_local_only(&op) {
            let outcome = dispatch_local_only(ctx, &op, account_id, &thread_id).await;
            results.push((idx, outcome));
            continue;
        }

        let Some(batch_key) = RemoteBatchKey::from_operation(&op) else {
            let outcome =
                dispatch_with_engine(ctx, &action_account, &op, account_id, &thread_id).await;
            results.push((idx, outcome));
            continue;
        };

        if matches!(op, MailOperation::PermanentDelete) {
            match resolve_thread_messages(ctx, account_id, &thread_id, action_account.provider)
                .await
            {
                Ok(targets) => {
                    remote_batches
                        .entry(batch_key)
                        .or_default()
                        .push(PendingRemote {
                            idx,
                            thread_id,
                            op,
                            targets,
                        });
                }
                Err(error) => {
                    results.push((idx, ActionOutcome::Failed { error }));
                }
            }
            continue;
        }

        let pre_local_targets = if is_container_move(&op) {
            match resolve_thread_messages(ctx, account_id, &thread_id, action_account.provider)
                .await
            {
                Ok(targets) => Some(targets),
                Err(error) => {
                    results.push((idx, ActionOutcome::Failed { error }));
                    continue;
                }
            }
        } else {
            None
        };

        let changed = match op_local(ctx, &op, account_id, &thread_id).await {
            Ok(changed) => changed,
            Err(error) => {
                results.push((idx, ActionOutcome::Failed { error }));
                continue;
            }
        };
        if !changed {
            results.push((idx, ActionOutcome::NoOp));
            continue;
        }
        let targets = if let Some(targets) = pre_local_targets {
            targets
        } else {
            match resolve_thread_messages(ctx, account_id, &thread_id, action_account.provider)
                .await
            {
                Ok(targets) => targets,
                Err(error) => {
                    let outcome = ActionOutcome::LocalOnly {
                        retryable: error.is_retryable(),
                        reason: error,
                    };
                    let (op_type, params_json) = enqueue_params(&op);
                    enqueue_if_retryable(
                        ctx,
                        &outcome,
                        account_id,
                        op_type,
                        &thread_id,
                        &params_json,
                    )
                    .await;
                    results.push((idx, outcome));
                    continue;
                }
            }
        };
        remote_batches
            .entry(batch_key)
            .or_default()
            .push(PendingRemote {
                idx,
                thread_id,
                op,
                targets,
            });
    }

    for (batch_key, pending) in remote_batches {
        let ids = pending
            .iter()
            .flat_map(|entry| entry.targets.iter().cloned())
            .collect::<Vec<_>>();
        let remote = dispatch_bulk_mutation(
            &action_account,
            account_id,
            &batch_key,
            ids,
            &NamespaceAttribution::Personal,
        )
        .await;
        match remote {
            Ok(()) => {
                let mdn_on_read = matches!(batch_key, RemoteBatchKey::Read { to: true });
                for entry in pending {
                    let outcome = if matches!(entry.op, MailOperation::PermanentDelete) {
                        match op_local(ctx, &entry.op, account_id, &entry.thread_id).await {
                            Ok(_) => ActionOutcome::Success,
                            Err(error) => ActionOutcome::Failed { error },
                        }
                    } else {
                        ActionOutcome::Success
                    };
                    if mdn_on_read && outcome.is_success() {
                        super::mdn_send::send_mdn_for_read(
                            ctx,
                            &action_account,
                            account_id,
                            &entry.thread_id,
                        )
                        .await;
                    }
                    results.push((entry.idx, outcome));
                }
            }
            Err(reason) => {
                for entry in pending {
                    let outcome = ActionOutcome::LocalOnly {
                        retryable: reason.is_retryable(),
                        reason: reason.clone(),
                    };
                    let (op_type, params_json) = enqueue_params(&entry.op);
                    enqueue_if_retryable(
                        ctx,
                        &outcome,
                        account_id,
                        op_type,
                        &entry.thread_id,
                        &params_json,
                    )
                    .await;
                    results.push((entry.idx, outcome));
                }
            }
        }
    }

    results
}

struct PendingRemote {
    idx: usize,
    thread_id: String,
    op: MailOperation,
    targets: Vec<ObjectId>,
}

/// Degraded fallback when the resident engine handle is unavailable.
async fn degraded_fallback(
    ctx: &ActionContext,
    account_id: &str,
    provider_error: &str,
    thread_ops: Vec<(usize, String, MailOperation)>,
) -> Vec<(usize, ActionOutcome)> {
    let mut results = Vec::with_capacity(thread_ops.len());
    for (idx, thread_id, op) in thread_ops {
        let _guard = match ctx.try_acquire_flight(account_id, &thread_id) {
            Some(g) => g,
            None => {
                results.push((
                    idx,
                    ActionOutcome::Failed {
                        error: ActionError::invalid_state(
                            "action already in flight for this thread",
                        ),
                    },
                ));
                continue;
            }
        };
        let outcome =
            handle_thread_degraded(ctx, &op, account_id, &thread_id, provider_error).await;
        results.push((idx, outcome));
    }
    results
}

/// Handle a single thread in degraded mode.
async fn handle_thread_degraded(
    ctx: &ActionContext,
    op: &MailOperation,
    account_id: &str,
    thread_id: &str,
    provider_error: &str,
) -> ActionOutcome {
    // Degraded label / label-group ops still carry their optimistic-intent
    // lifecycle; route through the label modules with no engine handle so the
    // member intents are written and a single composite retry is enqueued.
    match op {
        MailOperation::AddLabel { label_id } => {
            return label::dispatch_label_via_engine(
                ctx, None, account_id, thread_id, label_id, true,
            )
            .await;
        }
        MailOperation::RemoveLabel { label_id } => {
            return label::dispatch_label_via_engine(
                ctx, None, account_id, thread_id, label_id, false,
            )
            .await;
        }
        MailOperation::ApplyLabelGroup { group_id } => {
            return label_group::dispatch_group_via_engine(
                ctx, None, account_id, thread_id, *group_id, true,
            )
            .await;
        }
        MailOperation::RemoveLabelGroup { group_id } => {
            return label_group::dispatch_group_via_engine(
                ctx, None, account_id, thread_id, *group_id, false,
            )
            .await;
        }
        _ => {}
    }

    let name = op_name(op);
    let mlog = MutationLog::begin(name, account_id, thread_id);

    if matches!(op, MailOperation::PermanentDelete) {
        let error =
            ActionError::remote_with_kind(RemoteFailureKind::Transient, provider_error.to_string());
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
        // Permanent delete is provider-first: local rows carry the remote refs
        // needed for retry, so degraded mode must not delete them locally.
        let outcome = ActionOutcome::Failed { error };
        mlog.emit(&outcome);
        return outcome;
    }

    match op_local(ctx, op, account_id, thread_id).await {
        Ok(false) => {
            let outcome = ActionOutcome::NoOp;
            mlog.emit(&outcome);
            outcome
        }
        Ok(true) => {
            let outcome = ActionOutcome::LocalOnly {
                reason: ActionError::remote_with_kind(
                    RemoteFailureKind::Transient,
                    provider_error.to_string(),
                ),
                retryable: true,
            };
            let (op_type, params_json) = enqueue_params(op);
            enqueue_if_retryable(ctx, &outcome, account_id, op_type, thread_id, &params_json).await;
            mlog.emit(&outcome);
            outcome
        }
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            outcome
        }
    }
}

// ── Operation-specific routing ────────────────────────────────────

fn is_local_only(op: &MailOperation) -> bool {
    matches!(
        op,
        MailOperation::SetPinned { .. }
            | MailOperation::SetMuted { .. }
            | MailOperation::Snooze { .. }
            | MailOperation::Unsnooze
    )
}

fn is_container_move(op: &MailOperation) -> bool {
    matches!(
        op,
        MailOperation::Archive
            | MailOperation::Trash
            | MailOperation::SetSpam { .. }
            | MailOperation::MoveToFolder { .. }
    )
}

async fn dispatch_with_engine(
    ctx: &ActionContext,
    action_account: &crate::bifrost::resident::ResidentActionAccount,
    op: &MailOperation,
    account_id: &str,
    thread_id: &str,
) -> ActionOutcome {
    if is_local_only(op) {
        return dispatch_local_only(ctx, op, account_id, thread_id).await;
    }
    let namespace = match namespace_preflight(ctx, account_id, thread_id, op).await {
        Ok(namespace) => namespace,
        Err(error) => return ActionOutcome::Failed { error },
    };
    // Label / label-group ops route through their own modules so the
    // optimistic-intent lifecycle (confirm / clear / attach the pending
    // action id) and the composite contract (single composite retry row,
    // generation_seen race guard) are preserved across the engine cut.
    match op {
        MailOperation::AddLabel { label_id } => {
            return label::dispatch_label_via_engine(
                ctx,
                Some(action_account),
                account_id,
                thread_id,
                label_id,
                true,
            )
            .await;
        }
        MailOperation::RemoveLabel { label_id } => {
            return label::dispatch_label_via_engine(
                ctx,
                Some(action_account),
                account_id,
                thread_id,
                label_id,
                false,
            )
            .await;
        }
        MailOperation::ApplyLabelGroup { group_id } => {
            return label_group::dispatch_group_via_engine(
                ctx,
                Some(action_account),
                account_id,
                thread_id,
                *group_id,
                true,
            )
            .await;
        }
        MailOperation::RemoveLabelGroup { group_id } => {
            return label_group::dispatch_group_via_engine(
                ctx,
                Some(action_account),
                account_id,
                thread_id,
                *group_id,
                false,
            )
            .await;
        }
        _ => {}
    }
    let name = op_name(op);
    let mlog = MutationLog::begin(name, account_id, thread_id);
    if matches!(op, MailOperation::PermanentDelete) {
        let targets = match resolve_thread_messages(
            ctx,
            account_id,
            thread_id,
            action_account.provider,
        )
        .await
        {
            Ok(targets) => targets,
            Err(error) => {
                let outcome = ActionOutcome::Failed { error };
                mlog.emit(&outcome);
                return outcome;
            }
        };
        let remote = dispatch_mutation(action_account, account_id, op, targets, &namespace).await;
        let outcome = match remote {
            Ok(()) => match op_local(ctx, op, account_id, thread_id).await {
                Ok(_) => ActionOutcome::Success,
                Err(error) => ActionOutcome::Failed { error },
            },
            Err(reason) => ActionOutcome::LocalOnly {
                retryable: reason.is_retryable(),
                reason,
            },
        };
        let (op_type, params_json) = enqueue_params(op);
        enqueue_if_retryable(ctx, &outcome, account_id, op_type, thread_id, &params_json).await;
        mlog.emit(&outcome);
        return outcome;
    }
    let pre_local_targets = if is_container_move(op) {
        match resolve_thread_messages(ctx, account_id, thread_id, action_account.provider).await {
            Ok(targets) => Some(targets),
            Err(error) => {
                let outcome = ActionOutcome::Failed { error };
                mlog.emit(&outcome);
                return outcome;
            }
        }
    } else {
        None
    };
    let changed = match op_local(ctx, op, account_id, thread_id).await {
        Ok(changed) => changed,
        Err(error) => {
            let outcome = ActionOutcome::Failed { error };
            mlog.emit(&outcome);
            return outcome;
        }
    };
    if !changed {
        let outcome = ActionOutcome::NoOp;
        mlog.emit(&outcome);
        return outcome;
    }
    let targets = if let Some(targets) = pre_local_targets {
        targets
    } else {
        match resolve_thread_messages(ctx, account_id, thread_id, action_account.provider).await {
            Ok(targets) => targets,
            Err(error) => {
                let outcome = ActionOutcome::LocalOnly {
                    retryable: error.is_retryable(),
                    reason: error,
                };
                let (op_type, params_json) = enqueue_params(op);
                enqueue_if_retryable(ctx, &outcome, account_id, op_type, thread_id, &params_json)
                    .await;
                mlog.emit(&outcome);
                return outcome;
            }
        }
    };
    let outcome = outcome_from_remote_result(
        dispatch_mutation(action_account, account_id, op, targets, &namespace).await,
    );
    if outcome.is_success() && matches!(op, MailOperation::SetRead { to: true }) {
        super::mdn_send::send_mdn_for_read(ctx, action_account, account_id, thread_id).await;
    }
    let (op_type, params_json) = enqueue_params(op);
    enqueue_if_retryable(ctx, &outcome, account_id, op_type, thread_id, &params_json).await;
    mlog.emit(&outcome);
    outcome
}

/// Route to the correct `_local` function for degraded/short-circuit paths.
async fn op_local(
    ctx: &ActionContext,
    op: &MailOperation,
    account_id: &str,
    thread_id: &str,
) -> Result<bool, ActionError> {
    namespace_preflight(ctx, account_id, thread_id, op).await?;
    match op {
        MailOperation::Archive => archive::archive_local(ctx, account_id, thread_id).await,
        MailOperation::Trash => trash::trash_local(ctx, account_id, thread_id)
            .await
            .map(|()| true),
        MailOperation::SetSpam { to } => spam::spam_local(ctx, account_id, thread_id, *to)
            .await
            .map(|()| true),
        MailOperation::MoveToFolder { dest, source } => {
            move_to_folder::move_local(ctx, account_id, thread_id, dest, source.as_ref())
                .await
                .map(|()| true)
        }
        MailOperation::SetStarred { to } => star::star_local(ctx, account_id, thread_id, *to).await,
        MailOperation::SetRead { to } => {
            mark_read::mark_read_local(ctx, account_id, thread_id, *to)
                .await
                .map(|()| true)
        }
        MailOperation::PermanentDelete => {
            permanent_delete::permanent_delete_local(ctx, account_id, thread_id)
                .await
                .map(|()| true)
        }
        MailOperation::AddLabel { label_id } => {
            label::add_label_local(ctx, account_id, thread_id, label_id)
                .await
                .map(|_| true)
        }
        MailOperation::RemoveLabel { label_id } => {
            label::remove_label_local(ctx, account_id, thread_id, label_id)
                .await
                .map(|_| true)
        }
        MailOperation::ApplyLabelGroup { group_id } => {
            label_group::apply_label_group_local_initial(ctx, account_id, thread_id, *group_id)
                .await
                .map(|()| true)
        }
        MailOperation::RemoveLabelGroup { group_id } => {
            label_group::remove_label_group_local_initial(ctx, account_id, thread_id, *group_id)
                .await
                .map(|()| true)
        }
        MailOperation::SetPinned { to } => {
            // Local-only action in degraded path - call the action directly
            let outcome = pin::pin(ctx, account_id, thread_id, *to).await;
            Ok(outcome.is_success())
        }
        MailOperation::SetMuted { to } => {
            let outcome = mute::mute(ctx, account_id, thread_id, *to).await;
            Ok(outcome.is_success())
        }
        MailOperation::Snooze { until } => {
            let outcome = snooze::snooze(ctx, account_id, thread_id, *until).await;
            Ok(outcome.is_success())
        }
        MailOperation::Unsnooze => {
            let outcome = snooze::unsnooze(ctx, account_id, thread_id).await;
            Ok(outcome.is_success())
        }
    }
}

/// Namespace mutations must be rejected before the optimistic/local half of
/// an action. Public folders are explicitly read-only at the bifrost freeze;
/// returning `NotImplemented` here keeps the outcome terminal instead of
/// producing the misleading LocalOnly fallback used for a later dispatch
/// failure.
async fn namespace_preflight(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    op: &MailOperation,
) -> Result<NamespaceAttribution, ActionError> {
    let account_id = account_id.to_string();
    let thread_id = thread_id.to_string();
    let namespace = ctx
        .db
        .with_read(move |conn| {
            conn.query_row(
                &format!("{THREAD_RIGHTS_CTE}{SHARED_NAMESPACE_SQL}"),
                rusqlite::params![account_id, thread_id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        SharedFolderRights {
                            may_remove: row.get::<_, i64>(3)? != 0,
                            may_set_seen: row.get::<_, i64>(4)? != 0,
                            may_set_keywords: row.get::<_, i64>(5)? != 0,
                        },
                    ))
                },
            )
            .map(Some)
            .or_else(|error| match error {
                db::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                other => Err(format!("read action namespace: {other}")),
            })
        })
        .await
        .map_err(ActionError::db)?;
    // A thread row can legitimately be absent (a local draft, or a thread
    // already deleted by a racing action). Nothing namespaced can be missing
    // its own thread, so treat it as personal and let the real action path
    // report the miss with its own error.
    let Some((kind, namespace_id, provider, rights)) = namespace else {
        return Ok(NamespaceAttribution::Personal);
    };
    let attribution = match (kind.as_deref(), namespace_id.as_deref()) {
        (Some("shared"), Some(owner)) => NamespaceAttribution::Shared {
            owner: owner.to_string(),
        },
        (Some("public"), Some(folder_storage_id)) => NamespaceAttribution::Public {
            folder_storage_id: folder_storage_id.to_string(),
        },
        _ => NamespaceAttribution::Personal,
    };
    validate_namespace_action(&attribution, &provider, rights, op)?;
    Ok(attribution)
}

/// The thread's most restrictive rights across the shared folders it sits in.
/// An unset (`NULL`) right means the provider never reported one, in which
/// case the mutation is allowed through and the server arbitrates.
#[derive(Debug, Clone, Copy)]
struct SharedFolderRights {
    may_remove: bool,
    may_set_seen: bool,
    may_set_keywords: bool,
}

/// Per-right `MIN` over every folder the thread sits in, strictest wins.
/// Only consulted for a `Shared` attribution, where all of the thread's
/// folders are shared ones.
///
/// Two deliberate defaults inside the `MIN`:
/// - the folder row exists but the right is `NULL` (the provider reported no
///   rights at all): PERMITTED, and the server arbitrates;
/// - the folder row is MISSING (the container projection never produced one,
///   so the membership is dangling): DENIED. We cannot claim a write right on
///   a foreign folder we know nothing about, and the whole namespace design
///   fails closed rather than open.
const SHARED_NAMESPACE_SQL: &str = "\
    SELECT t.namespace_kind, t.namespace_id, a.provider, \
           COALESCE((SELECT MIN(right_remove) FROM thread_rights), 1), \
           COALESCE((SELECT MIN(right_set_seen) FROM thread_rights), 1), \
           COALESCE((SELECT MIN(right_set_keywords) FROM thread_rights), 1) \
    FROM threads t JOIN accounts a ON a.id = t.account_id \
    WHERE t.account_id = ?1 AND t.id = ?2";

/// The `thread_rights` CTE `SHARED_NAMESPACE_SQL` reads. Kept separate so the
/// per-right subqueries stay one line each.
const THREAD_RIGHTS_CTE: &str = "\
    WITH thread_rights AS ( \
        SELECT COALESCE(f.right_remove, CASE WHEN f.id IS NULL THEN 0 ELSE 1 END) \
                 AS right_remove, \
               COALESCE(f.right_set_seen, CASE WHEN f.id IS NULL THEN 0 ELSE 1 END) \
                 AS right_set_seen, \
               COALESCE(f.right_set_keywords, CASE WHEN f.id IS NULL THEN 0 ELSE 1 END) \
                 AS right_set_keywords \
        FROM thread_folders tf \
        LEFT JOIN folders f ON f.account_id = tf.account_id AND f.id = tf.folder_id \
        WHERE tf.account_id = ?1 AND tf.thread_id = ?2 \
    ) ";

/// Pure half of namespace preflight. Keeping the policy independent of the
/// database lookup makes the fail-before-local-write contract directly
/// testable and keeps every action entry point on the same rule.
fn validate_namespace_action(
    attribution: &NamespaceAttribution,
    provider: &str,
    rights: SharedFolderRights,
    op: &MailOperation,
) -> Result<(), ActionError> {
    // Pin / mute / snooze never reach a provider, so no namespace, rights, or
    // capability gap can make them unsupported. Refusing them would make a
    // read-only shared folder or a public folder unusable for purely local
    // organisation, which is not what obstacles K, L, and W are about.
    if is_local_only(op) {
        return Ok(());
    }
    if matches!(attribution, NamespaceAttribution::Public { .. }) {
        return Err(ActionError::not_implemented(
            "public-folder items are read-only",
        ));
    }
    if matches!(attribution, NamespaceAttribution::Shared { .. }) && provider == "jmap" {
        return Err(ActionError::not_implemented(
            "JMAP shared-mailbox mutations are not supported",
        ));
    }
    if matches!(attribution, NamespaceAttribution::Shared { .. }) && !permitted_by(rights, op) {
        return Err(ActionError::not_implemented(
            "shared mailbox rights do not permit this mutation",
        ));
    }
    if let MailOperation::MoveToFolder { dest, .. } = op {
        let destination = dest.as_str();
        let expected_prefix = match attribution {
            NamespaceAttribution::Shared { owner } => format!("shared:{owner}:"),
            NamespaceAttribution::Public { .. } => unreachable!("public returns above"),
            NamespaceAttribution::Personal => String::new(),
        };
        let same_namespace = if expected_prefix.is_empty() {
            !destination.starts_with("shared:") && !destination.starts_with("public:")
        } else {
            destination.starts_with(&expected_prefix)
        };
        if !same_namespace {
            return Err(ActionError::not_implemented(
                "cross-namespace moves are not supported",
            ));
        }
    }
    Ok(())
}

/// Map an operation onto the single MYRIGHTS / `effective_rights` bit that
/// actually governs it. Gating a flag change on the item-removal right (as a
/// blanket "read-only folder" check would) refuses mutations the server would
/// have accepted.
fn permitted_by(rights: SharedFolderRights, op: &MailOperation) -> bool {
    match op {
        MailOperation::SetRead { .. } => rights.may_set_seen,
        MailOperation::SetStarred { .. }
        | MailOperation::AddLabel { .. }
        | MailOperation::RemoveLabel { .. }
        | MailOperation::ApplyLabelGroup { .. }
        | MailOperation::RemoveLabelGroup { .. } => rights.may_set_keywords,
        MailOperation::Archive
        | MailOperation::Trash
        | MailOperation::PermanentDelete
        | MailOperation::SetSpam { .. }
        | MailOperation::MoveToFolder { .. } => rights.may_remove,
        MailOperation::SetPinned { .. }
        | MailOperation::SetMuted { .. }
        | MailOperation::Snooze { .. }
        | MailOperation::Unsnooze => true,
    }
}

/// Dispatch a local-only action (pin/mute/snooze). No namespace preflight
/// here: `validate_namespace_action` passes every local-only op by design,
/// so a lookup would only cost a query.
async fn dispatch_local_only(
    ctx: &ActionContext,
    op: &MailOperation,
    account_id: &str,
    thread_id: &str,
) -> ActionOutcome {
    match op {
        MailOperation::SetPinned { to } => pin::pin(ctx, account_id, thread_id, *to).await,
        MailOperation::SetMuted { to } => mute::mute(ctx, account_id, thread_id, *to).await,
        MailOperation::Snooze { until } => snooze::snooze(ctx, account_id, thread_id, *until).await,
        MailOperation::Unsnooze => snooze::unsnooze(ctx, account_id, thread_id).await,
        _ => unreachable!("only pin/mute/snooze/unsnooze are local-only"),
    }
}

/// Derive `(operation_type, params_json)` for pending-ops enqueue.
fn enqueue_params(op: &MailOperation) -> (&'static str, String) {
    match op {
        MailOperation::Archive => ("archive", "{}".to_string()),
        MailOperation::Trash => ("trash", "{}".to_string()),
        MailOperation::SetSpam { to } => ("spam", format!(r#"{{"isSpam":{to}}}"#)),
        MailOperation::MoveToFolder { dest, source } => (
            "moveToFolder",
            serde_json::json!({"folderId": dest, "sourceFolderId": source}).to_string(),
        ),
        MailOperation::SetStarred { to } => ("star", format!(r#"{{"starred":{to}}}"#)),
        MailOperation::SetRead { to } => ("markRead", format!(r#"{{"read":{to}}}"#)),
        MailOperation::PermanentDelete => ("permanentDelete", "{}".to_string()),
        MailOperation::AddLabel { label_id } => (
            "addLabel",
            serde_json::json!({"labelId": label_id}).to_string(),
        ),
        MailOperation::RemoveLabel { label_id } => (
            "removeLabel",
            serde_json::json!({"labelId": label_id}).to_string(),
        ),
        MailOperation::ApplyLabelGroup { group_id } => (
            "applyLabelGroup",
            serde_json::json!({"groupId": group_id.as_i64()}).to_string(),
        ),
        MailOperation::RemoveLabelGroup { group_id } => (
            "removeLabelGroup",
            serde_json::json!({"groupId": group_id.as_i64()}).to_string(),
        ),
        MailOperation::SetPinned { .. }
        | MailOperation::SetMuted { .. }
        | MailOperation::Snooze { .. }
        | MailOperation::Unsnooze => {
            unreachable!("local-only actions don't enqueue")
        }
    }
}

/// Human-readable operation name for logging.
fn op_name(op: &MailOperation) -> &'static str {
    match op {
        MailOperation::Archive => "archive",
        MailOperation::Trash => "trash",
        MailOperation::SetSpam { .. } => "spam",
        MailOperation::MoveToFolder { .. } => "move_to_folder",
        MailOperation::SetStarred { .. } => "star",
        MailOperation::SetRead { .. } => "mark_read",
        MailOperation::PermanentDelete => "permanent_delete",
        MailOperation::AddLabel { .. } => "add_label",
        MailOperation::RemoveLabel { .. } => "remove_label",
        MailOperation::ApplyLabelGroup { .. } => "apply_label_group",
        MailOperation::RemoveLabelGroup { .. } => "remove_label_group",
        MailOperation::SetPinned { .. } => "pin",
        MailOperation::SetMuted { .. } => "mute",
        MailOperation::Snooze { .. } => "snooze",
        MailOperation::Unsnooze => "unsnooze",
    }
}

#[cfg(test)]
mod tests {
    use common::typed_ids::FolderId;

    use super::*;

    fn shared() -> NamespaceAttribution {
        NamespaceAttribution::Shared {
            owner: "alice".into(),
        }
    }

    fn public() -> NamespaceAttribution {
        NamespaceAttribution::Public {
            folder_storage_id: "public:pf".into(),
        }
    }

    fn move_to(dest: &str) -> MailOperation {
        MailOperation::MoveToFolder {
            dest: FolderId::from(dest),
            source: None,
        }
    }

    fn label_op() -> MailOperation {
        MailOperation::AddLabel {
            label_id: common::typed_ids::LabelId::from("tag"),
        }
    }

    const OPEN: SharedFolderRights = SharedFolderRights {
        may_remove: true,
        may_set_seen: true,
        may_set_keywords: true,
    };

    const READ_ONLY: SharedFolderRights = SharedFolderRights {
        may_remove: false,
        may_set_seen: false,
        may_set_keywords: false,
    };

    #[test]
    fn cross_namespace_move_is_rejected_before_local_write() {
        assert!(validate_namespace_action(&shared(), "graph", OPEN, &move_to("archive")).is_err());
        assert!(
            validate_namespace_action(
                &NamespaceAttribution::Personal,
                "graph",
                OPEN,
                &move_to("shared:alice:graph-1"),
            )
            .is_err()
        );
    }

    #[test]
    fn same_namespace_move_is_allowed() {
        assert!(
            validate_namespace_action(&shared(), "graph", OPEN, &move_to("shared:alice:graph-1"))
                .is_ok()
        );
        assert!(
            validate_namespace_action(
                &NamespaceAttribution::Personal,
                "graph",
                OPEN,
                &move_to("archive"),
            )
            .is_ok()
        );
    }

    #[test]
    fn public_namespace_action_fails_without_local_mutation() {
        assert!(
            validate_namespace_action(&public(), "graph", OPEN, &MailOperation::Trash).is_err()
        );
    }

    #[test]
    fn public_namespace_label_action_fails_without_local_mutation() {
        assert!(validate_namespace_action(&public(), "graph", OPEN, &label_op()).is_err());
    }

    #[test]
    fn local_only_ops_are_allowed_in_every_namespace() {
        for namespace in [NamespaceAttribution::Personal, shared(), public()] {
            for op in [
                MailOperation::SetPinned { to: true },
                MailOperation::SetMuted { to: true },
                MailOperation::Snooze { until: 1 },
                MailOperation::Unsnooze,
            ] {
                assert!(
                    validate_namespace_action(&namespace, "jmap", READ_ONLY, &op).is_ok(),
                    "{namespace:?} must still allow {op:?}"
                );
            }
        }
    }

    #[test]
    fn rights_denied_shared_folder_action_fails_before_local_write() {
        assert!(
            validate_namespace_action(
                &shared(),
                "imap",
                READ_ONLY,
                &MailOperation::SetRead { to: true },
            )
            .is_err()
        );
    }

    #[test]
    fn shared_folder_rights_are_checked_per_operation() {
        let seen_only = SharedFolderRights {
            may_remove: false,
            may_set_seen: true,
            may_set_keywords: false,
        };
        assert!(
            validate_namespace_action(
                &shared(),
                "imap",
                seen_only,
                &MailOperation::SetRead { to: true },
            )
            .is_ok(),
            "a folder granting \\Seen must accept mark-as-read"
        );
        assert!(
            validate_namespace_action(&shared(), "imap", seen_only, &MailOperation::Archive)
                .is_err(),
            "the same folder must still refuse a move"
        );
        assert!(
            validate_namespace_action(&shared(), "imap", seen_only, &label_op()).is_err(),
            "the same folder must still refuse a keyword change"
        );
    }

    #[test]
    fn rights_are_ignored_outside_a_shared_namespace() {
        assert!(
            validate_namespace_action(
                &NamespaceAttribution::Personal,
                "imap",
                READ_ONLY,
                &MailOperation::Archive,
            )
            .is_ok()
        );
    }

    #[test]
    fn jmap_shared_namespace_mutation_is_rejected() {
        assert!(
            validate_namespace_action(
                &shared(),
                "jmap",
                OPEN,
                &MailOperation::SetRead { to: true }
            )
            .is_err()
        );
    }
}
