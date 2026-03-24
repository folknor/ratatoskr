//! Batch action executor — groups targets by account, creates one provider
//! per account, dispatches with provider reuse. Parallel across accounts,
//! sequential within each account.

use std::collections::HashMap;
use std::time::Instant;

use ratatoskr_provider_utils::ops::ProviderOps;

use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome, RemoteFailureKind};
use super::pending::enqueue_if_retryable;
use super::provider::create_provider;
use super::{archive, label, mark_read, move_to_folder, mute, permanent_delete, pin, spam, star, trash};

/// Classify a provider-creation error as permanent or potentially transient.
///
/// `create_provider` returns plain `String` errors. We classify known
/// permanent patterns (unknown provider, account not found) so the batch
/// executor doesn't spray the retry queue with unrecoverable ops.
fn classify_provider_error(error: &str) -> RemoteFailureKind {
    if error.starts_with("Unknown provider")
        || error.contains("no rows returned")
        || error.contains("QueryReturnedNoRows")
    {
        RemoteFailureKind::Permanent
    } else {
        RemoteFailureKind::Unknown
    }
}

/// After this many consecutive retryable remote failures within one account
/// group, remaining threads are short-circuited (local-only + enqueue).
const MAX_CONSECUTIVE_FAILURES: u32 = 3;

/// Action + parameters for a batch operation.
/// All threads in the batch get the same action.
#[derive(Debug, Clone)]
pub enum BatchAction {
    Archive,
    Trash,
    Spam { is_spam: bool },
    MoveToFolder { folder_id: String, source_label_id: Option<String> },
    Star { starred: bool },
    MarkRead { read: bool },
    PermanentDelete,
    AddLabel { label_id: String },
    RemoveLabel { label_id: String },
    Pin { pinned: bool },
    Mute { muted: bool },
}

/// Execute a batch action across multiple threads.
///
/// Groups targets by account, creates one provider per account, and
/// dispatches sequentially within each account. Accounts run in parallel.
/// Returns outcomes in the same order as `targets`.
pub async fn batch_execute(
    ctx: &ActionContext,
    action: BatchAction,
    targets: Vec<(String, String)>,
) -> Vec<ActionOutcome> {
    let started = Instant::now();
    let total = targets.len();

    // Group by account, preserving original indices
    let mut groups: HashMap<String, Vec<(usize, String)>> = HashMap::new();
    for (i, (account_id, thread_id)) in targets.iter().enumerate() {
        groups
            .entry(account_id.clone())
            .or_default()
            .push((i, thread_id.clone()));
    }
    let account_count = groups.len();

    // Dispatch per-account groups in parallel
    let account_futures: Vec<_> = groups
        .into_iter()
        .map(|(account_id, thread_indices)| {
            let ctx = ctx.clone();
            let action = action.clone();
            async move {
                execute_account_group(&ctx, &action, &account_id, thread_indices).await
            }
        })
        .collect();
    let group_results = futures::future::join_all(account_futures).await;

    // Reassemble in original order
    let mut outcomes = Vec::with_capacity(total);
    outcomes.resize_with(total, || {
        ActionOutcome::Failed {
            error: ActionError::invalid_state("batch reassembly bug"),
        }
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
    let name = action_name(&action);
    log::info!(
        "[action-batch] {name} | {total} threads / {account_count} accounts | \
         {success} ok, {local_only} local-only, {failed} failed | {elapsed}ms"
    );

    outcomes
}

/// Execute one account's thread group.
async fn execute_account_group(
    ctx: &ActionContext,
    action: &BatchAction,
    account_id: &str,
    thread_indices: Vec<(usize, String)>,
) -> Vec<(usize, ActionOutcome)> {
    let mut results = Vec::with_capacity(thread_indices.len());

    // Pin/mute are local-only — no provider needed
    if matches!(action, BatchAction::Pin { .. } | BatchAction::Mute { .. }) {
        for (idx, thread_id) in thread_indices {
            let outcome = dispatch_local_only(ctx, action, account_id, &thread_id).await;
            results.push((idx, outcome));
        }
        return results;
    }

    // Create provider once for this account
    let provider = match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(p) => p,
        Err(e) => {
            // Classify the error — permanent failures (unknown provider, missing
            // account) should not spam the retry queue.
            let kind = classify_provider_error(&e);
            return degraded_fallback(ctx, action, account_id, &e, kind, thread_indices).await;
        }
    };

    // Dispatch sequentially with short-circuit on consecutive failures
    let mut consecutive_remote_failures: u32 = 0;

    for (idx, thread_id) in &thread_indices {
        if consecutive_remote_failures >= MAX_CONSECUTIVE_FAILURES {
            // Short-circuit: per-thread degraded path (retryable — provider was
            // reachable but failing on consecutive operations)
            let outcome = handle_thread_degraded(
                ctx, action, account_id, thread_id,
                "provider presumed unavailable after consecutive failures",
                RemoteFailureKind::Unknown,
            ).await;
            results.push((*idx, outcome));
            continue;
        }

        let outcome = dispatch_with_provider(ctx, &*provider, action, account_id, thread_id).await;

        if let ActionOutcome::LocalOnly { reason, .. } = &outcome {
            if reason.is_retryable() {
                consecutive_remote_failures += 1;
            } else {
                consecutive_remote_failures = 0;
            }
        } else {
            consecutive_remote_failures = 0;
        }

        results.push((*idx, outcome));
    }

    results
}

/// Degraded fallback when provider creation fails.
/// Each thread gets its own _local call and per-thread outcome.
async fn degraded_fallback(
    ctx: &ActionContext,
    action: &BatchAction,
    account_id: &str,
    provider_error: &str,
    error_kind: RemoteFailureKind,
    thread_indices: Vec<(usize, String)>,
) -> Vec<(usize, ActionOutcome)> {
    let mut results = Vec::with_capacity(thread_indices.len());
    for (idx, thread_id) in thread_indices {
        let outcome =
            handle_thread_degraded(ctx, action, account_id, &thread_id, provider_error, error_kind).await;
        results.push((idx, outcome));
    }
    results
}

/// Handle a single thread in degraded mode: apply _local, return per-thread
/// outcome with MutationLog, enqueue only if the error is retryable.
async fn handle_thread_degraded(
    ctx: &ActionContext,
    action: &BatchAction,
    account_id: &str,
    thread_id: &str,
    provider_error: &str,
    error_kind: RemoteFailureKind,
) -> ActionOutcome {
    let name = action_name(action);
    let mlog = MutationLog::begin(name, account_id, thread_id);

    match action_local(ctx, action, account_id, thread_id).await {
        Ok(()) => {
            let retryable = matches!(
                error_kind,
                RemoteFailureKind::Transient | RemoteFailureKind::Unknown
            );
            let outcome = ActionOutcome::LocalOnly {
                reason: ActionError::remote_with_kind(error_kind, provider_error),
                retryable,
            };
            let (op_type, params_json) = enqueue_params(action);
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

// ── Action-specific routing ─────────────────────────────────────────

/// Route to the correct `_with_provider` function.
async fn dispatch_with_provider(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    action: &BatchAction,
    account_id: &str,
    thread_id: &str,
) -> ActionOutcome {
    match action {
        BatchAction::Archive => {
            archive::archive_with_provider(ctx, provider, account_id, thread_id).await
        }
        BatchAction::Trash => {
            trash::trash_with_provider(ctx, provider, account_id, thread_id).await
        }
        BatchAction::Spam { is_spam } => {
            spam::spam_with_provider(ctx, provider, account_id, thread_id, *is_spam).await
        }
        BatchAction::MoveToFolder { folder_id, source_label_id } => {
            move_to_folder::move_to_folder_with_provider(
                ctx, provider, account_id, thread_id, folder_id, source_label_id.as_deref(),
            ).await
        }
        BatchAction::Star { starred } => {
            star::star_with_provider(ctx, provider, account_id, thread_id, *starred).await
        }
        BatchAction::MarkRead { read } => {
            mark_read::mark_read_with_provider(ctx, provider, account_id, thread_id, *read).await
        }
        BatchAction::PermanentDelete => {
            permanent_delete::permanent_delete_with_provider(ctx, provider, account_id, thread_id).await
        }
        BatchAction::AddLabel { label_id } => {
            label::add_label_with_provider(ctx, provider, account_id, thread_id, label_id).await
        }
        BatchAction::RemoveLabel { label_id } => {
            label::remove_label_with_provider(ctx, provider, account_id, thread_id, label_id).await
        }
        BatchAction::Pin { .. } | BatchAction::Mute { .. } => {
            unreachable!("local-only actions don't use provider dispatch")
        }
    }
}

/// Route to the correct `_local` function for degraded/short-circuit paths.
async fn action_local(
    ctx: &ActionContext,
    action: &BatchAction,
    account_id: &str,
    thread_id: &str,
) -> Result<(), ActionError> {
    match action {
        BatchAction::Archive => archive::archive_local(ctx, account_id, thread_id).await,
        BatchAction::Trash => trash::trash_local(ctx, account_id, thread_id).await,
        BatchAction::Spam { is_spam } => spam::spam_local(ctx, account_id, thread_id, *is_spam).await,
        BatchAction::MoveToFolder { folder_id, source_label_id } => {
            move_to_folder::move_local(ctx, account_id, thread_id, folder_id, source_label_id.as_deref()).await
        }
        BatchAction::Star { starred } => star::star_local(ctx, account_id, thread_id, *starred).await,
        BatchAction::MarkRead { read } => mark_read::mark_read_local(ctx, account_id, thread_id, *read).await,
        BatchAction::PermanentDelete => permanent_delete::permanent_delete_local(ctx, account_id, thread_id).await,
        BatchAction::AddLabel { label_id } => {
            label::add_label_local(ctx, account_id, thread_id, label_id).await.map(|_| ())
        }
        BatchAction::RemoveLabel { label_id } => {
            label::remove_label_local(ctx, account_id, thread_id, label_id).await.map(|_| ())
        }
        BatchAction::Pin { .. } | BatchAction::Mute { .. } => {
            unreachable!("local-only actions use direct dispatch")
        }
    }
}

/// Dispatch a local-only action (pin/mute).
async fn dispatch_local_only(
    ctx: &ActionContext,
    action: &BatchAction,
    account_id: &str,
    thread_id: &str,
) -> ActionOutcome {
    match action {
        BatchAction::Pin { pinned } => pin::pin(ctx, account_id, thread_id, *pinned).await,
        BatchAction::Mute { muted } => mute::mute(ctx, account_id, thread_id, *muted).await,
        _ => unreachable!("only pin/mute are local-only"),
    }
}

/// Derive `(operation_type, params_json)` for pending-ops enqueue.
fn enqueue_params(action: &BatchAction) -> (&'static str, String) {
    match action {
        BatchAction::Archive => ("archive", "{}".to_string()),
        BatchAction::Trash => ("trash", "{}".to_string()),
        BatchAction::Spam { is_spam } => ("spam", format!(r#"{{"isSpam":{is_spam}}}"#)),
        BatchAction::MoveToFolder { folder_id, source_label_id } => (
            "moveToFolder",
            serde_json::json!({"folderId": folder_id, "sourceLabelId": source_label_id}).to_string(),
        ),
        BatchAction::Star { starred } => ("star", format!(r#"{{"starred":{starred}}}"#)),
        BatchAction::MarkRead { read } => ("markRead", format!(r#"{{"read":{read}}}"#)),
        BatchAction::PermanentDelete => ("permanentDelete", "{}".to_string()),
        BatchAction::AddLabel { label_id } => {
            ("addLabel", serde_json::json!({"labelId": label_id}).to_string())
        }
        BatchAction::RemoveLabel { label_id } => {
            ("removeLabel", serde_json::json!({"labelId": label_id}).to_string())
        }
        BatchAction::Pin { .. } | BatchAction::Mute { .. } => {
            unreachable!("local-only actions don't enqueue")
        }
    }
}

/// Human-readable action name for logging.
fn action_name(action: &BatchAction) -> &'static str {
    match action {
        BatchAction::Archive => "archive",
        BatchAction::Trash => "trash",
        BatchAction::Spam { .. } => "spam",
        BatchAction::MoveToFolder { .. } => "move_to_folder",
        BatchAction::Star { .. } => "star",
        BatchAction::MarkRead { .. } => "mark_read",
        BatchAction::PermanentDelete => "permanent_delete",
        BatchAction::AddLabel { .. } => "add_label",
        BatchAction::RemoveLabel { .. } => "remove_label",
        BatchAction::Pin { .. } => "pin",
        BatchAction::Mute { .. } => "mute",
    }
}
