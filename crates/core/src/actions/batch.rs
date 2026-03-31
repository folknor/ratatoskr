//! Batch action executor — groups targets by account, creates one provider
//! per account, dispatches with provider reuse. Parallel across accounts,
//! sequential within each account.

use std::collections::HashMap;
use std::time::Instant;

use common::ops::ProviderOps;

use super::context::ActionContext;
use super::log::MutationLog;
use super::operation::MailOperation;
use super::outcome::{ActionError, ActionOutcome, RemoteFailureKind};
use super::pending::enqueue_if_retryable;
use super::provider::create_provider;
use super::{
    archive, label, mark_read, move_to_folder, mute, permanent_delete, pin, snooze, spam, star,
    trash,
};

/// Maximum consecutive remote failures before short-circuiting to degraded mode.
const MAX_CONSECUTIVE_FAILURES: u32 = 3;

/// Execute operations across multiple threads.
///
/// Each entry is `(account_id, thread_id, operation)` — per-target operations
/// support mixed-value toggles and heterogeneous batches.
///
/// Groups by account, creates one provider per account, dispatches sequentially
/// within each account. Accounts run in parallel.
///
/// **Outcome ordering:** `outcomes[i]` corresponds to `operations[i]` regardless
/// of internal regrouping. All undo/rollback bookkeeping must key off original
/// operation order.
pub async fn batch_execute(
    ctx: &ActionContext,
    operations: Vec<(String, String, MailOperation)>,
) -> Vec<ActionOutcome> {
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
            async move { execute_account_group(&ctx, &account_id, thread_ops).await }
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

    // Create provider once for this account
    let provider = match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(p) => p,
        Err(e) => {
            let kind = classify_provider_error(&e);
            return degraded_fallback(ctx, account_id, &e, kind, thread_ops).await;
        }
    };

    // Dispatch sequentially with short-circuit on consecutive failures
    let mut consecutive_remote_failures: u32 = 0;

    for (idx, thread_id, op) in &thread_ops {
        let _guard = match ctx.try_acquire_flight(account_id, thread_id) {
            Some(g) => g,
            None => {
                results.push((
                    *idx,
                    ActionOutcome::Failed {
                        error: ActionError::invalid_state(
                            "action already in flight for this thread",
                        ),
                    },
                ));
                continue;
            }
        };

        let outcome = if consecutive_remote_failures >= MAX_CONSECUTIVE_FAILURES {
            handle_thread_degraded(
                ctx,
                op,
                account_id,
                thread_id,
                "provider presumed unavailable after consecutive failures",
                RemoteFailureKind::Unknown,
            )
            .await
        } else {
            let o = dispatch_with_provider(ctx, &*provider, op, account_id, thread_id).await;
            if let ActionOutcome::LocalOnly { reason, .. } = &o {
                if reason.is_retryable() {
                    consecutive_remote_failures += 1;
                } else {
                    consecutive_remote_failures = 0;
                }
            } else {
                consecutive_remote_failures = 0;
            }
            o
        };

        results.push((*idx, outcome));
    }

    results
}

/// Degraded fallback when provider creation fails.
async fn degraded_fallback(
    ctx: &ActionContext,
    account_id: &str,
    provider_error: &str,
    error_kind: RemoteFailureKind,
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
            handle_thread_degraded(ctx, &op, account_id, &thread_id, provider_error, error_kind)
                .await;
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
    error_kind: RemoteFailureKind,
) -> ActionOutcome {
    let name = op_name(op);
    let mlog = MutationLog::begin(name, account_id, thread_id);

    match op_local(ctx, op, account_id, thread_id).await {
        Ok(false) => {
            let outcome = ActionOutcome::NoOp;
            mlog.emit(&outcome);
            outcome
        }
        Ok(true) => {
            let retryable = matches!(
                error_kind,
                RemoteFailureKind::Transient | RemoteFailureKind::Unknown
            );
            let outcome = ActionOutcome::LocalOnly {
                reason: ActionError::remote_with_kind(error_kind, provider_error),
                retryable,
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
    )
}

/// Route to the correct `_with_provider` function.
async fn dispatch_with_provider(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    op: &MailOperation,
    account_id: &str,
    thread_id: &str,
) -> ActionOutcome {
    match op {
        MailOperation::Archive => {
            archive::archive_with_provider(ctx, provider, account_id, thread_id).await
        }
        MailOperation::Trash => {
            trash::trash_with_provider(ctx, provider, account_id, thread_id).await
        }
        MailOperation::SetSpam { to } => {
            spam::spam_with_provider(ctx, provider, account_id, thread_id, *to).await
        }
        MailOperation::MoveToFolder { dest, source } => {
            move_to_folder::move_to_folder_with_provider(
                ctx,
                provider,
                account_id,
                thread_id,
                dest,
                source.as_ref(),
            )
            .await
        }
        MailOperation::SetStarred { to } => {
            star::star_with_provider(ctx, provider, account_id, thread_id, *to).await
        }
        MailOperation::SetRead { to } => {
            mark_read::mark_read_with_provider(ctx, provider, account_id, thread_id, *to).await
        }
        MailOperation::PermanentDelete => {
            permanent_delete::permanent_delete_with_provider(ctx, provider, account_id, thread_id)
                .await
        }
        MailOperation::AddLabel { label_id } => {
            label::add_label_with_provider(ctx, provider, account_id, thread_id, label_id).await
        }
        MailOperation::RemoveLabel { label_id } => {
            label::remove_label_with_provider(ctx, provider, account_id, thread_id, label_id).await
        }
        // Local-only ops routed through provider path in mixed batches
        op @ (MailOperation::SetPinned { .. }
        | MailOperation::SetMuted { .. }
        | MailOperation::Snooze { .. }) => {
            dispatch_local_only(ctx, op, account_id, thread_id).await
        }
    }
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
                .map(|()| true)
        }
        MailOperation::RemoveLabel { label_id } => {
            label::remove_label_local(ctx, account_id, thread_id, label_id)
                .await
                .map(|()| true)
        }
        MailOperation::SetPinned { to } => {
            // Local-only action in degraded path — call the action directly
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
        _ => unreachable!("only pin/mute/snooze are local-only"),
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
            serde_json::json!({"folderId": dest, "sourceLabelId": source}).to_string(),
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
        MailOperation::SetPinned { .. }
        | MailOperation::SetMuted { .. }
        | MailOperation::Snooze { .. } => {
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
        MailOperation::SetPinned { .. } => "pin",
        MailOperation::SetMuted { .. } => "mute",
        MailOperation::Snooze { .. } => "snooze",
    }
}

/// Classify a provider creation error for retry policy.
fn classify_provider_error(error: &str) -> RemoteFailureKind {
    let lower = error.to_lowercase();
    // Permanent: provider/account doesn't exist or can't be constructed
    if lower.contains("unknown provider")
        || lower.contains("no rows returned")
        || lower.contains("queryreturnednorows")
        || lower.contains("not found")
        || lower.contains("missing account")
    {
        RemoteFailureKind::Permanent
    } else if lower.contains("timeout")
        || lower.contains("connection refused")
        || lower.contains("dns")
        || lower.contains("network")
    {
        RemoteFailureKind::Transient
    } else {
        RemoteFailureKind::Unknown
    }
}
