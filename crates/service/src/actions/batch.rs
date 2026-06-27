//! Batch action executor - groups targets by account and dispatches
//! provider-backed mail mutations through the resident bifrost engine.
//! Parallel across accounts,
//! sequential within each account.

use std::collections::HashMap;
use std::time::Instant;

use bifrost_types::ObjectId;

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

        if !use_bulk {
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
        let remote = dispatch_bulk_mutation(&action_account, account_id, &batch_key, ids).await;
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
        let remote = dispatch_mutation(action_account, account_id, op, targets).await;
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
        dispatch_mutation(action_account, account_id, op, targets).await,
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

/// Dispatch a local-only action (pin/mute/snooze).
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
