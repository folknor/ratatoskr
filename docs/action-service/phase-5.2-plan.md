# Action Service: Phase 5.2 Detailed Plan — Batch Executor

## Goal

A single `batch_execute` function in core that groups targets by account, creates one provider per account, and dispatches actions with provider reuse. Parallel across accounts, sequential within each account. Consecutive-failure short-circuit to avoid N guaranteed failures after a dead provider.

## Current State (after 5.1 + fixes)

Each action file has three layers (from commit b04f638):
- `_local` (private): DB mutation only — e.g., `archive_local(ctx, account_id, thread_id)`
- `_dispatch` (private): provider dispatch + enqueue + log — e.g., `archive_dispatch(ctx, provider, account_id, thread_id)`
- `_with_provider` (`pub(crate)`): `_local` → `_dispatch`
- Public wrapper: `_local` → `create_provider` → `_dispatch`

Pin/mute are local-only (no `_with_provider`, no `_dispatch`, no provider).

**Pre-step for 5.2:** Make each `_local` function `pub(crate)`. They're currently private. The batch executor needs them for the provider-creation-failure and short-circuit fallback paths.

`futures = "0.3"` is already a dependency of the core crate.

## New File: `crates/core/src/actions/batch.rs`

### Types

```rust
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
```

Returns `Vec<ActionOutcome>` directly — no wrapper struct.

### Function Signature

```rust
pub async fn batch_execute(
    ctx: &ActionContext,
    action: BatchAction,
    targets: Vec<(String, String)>,
) -> Vec<ActionOutcome>
```

### Execution Flow

**Step 1: Group by account.**

```rust
let mut groups: HashMap<String, Vec<(usize, String)>> = HashMap::new();
for (i, (account_id, thread_id)) in targets.iter().enumerate() {
    groups.entry(account_id.clone()).or_default().push((i, thread_id.clone()));
}
```

**Step 2: Check if action needs a provider.**

```rust
let needs_provider = !matches!(action, BatchAction::Pin { .. } | BatchAction::Mute { .. });
```

**Step 3: Dispatch per-account groups in parallel.**

For each account group, spawn a future that calls `execute_account_group`. Use `futures::future::join_all` across groups.

**Step 4: Reassemble outcomes in original order.**

```rust
let mut outcomes = Vec::with_capacity(targets.len());
outcomes.resize_with(targets.len(), || ActionOutcome::Failed {
    error: ActionError::invalid_state("batch reassembly bug"),
});
for group in group_results {
    for (idx, outcome) in group {
        outcomes[idx] = outcome;
    }
}
```

The sentinel `Failed` should never survive — every index is covered by exactly one group.

**Step 5: Emit batch summary log.**

```rust
log::info!(
    "[action-batch] {action_name} | {total} threads / {account_count} accounts | \
     {success} ok, {local_only} local-only, {failed} failed | {elapsed}ms"
);
```

This is in addition to per-thread `MutationLog` entries (emitted by every path — see below).

### Per-Account Group Execution

```rust
async fn execute_account_group(
    ctx: &ActionContext,
    action: &BatchAction,
    account_id: &str,
    thread_indices: Vec<(usize, String)>,
) -> Vec<(usize, ActionOutcome)>
```

**For local-only actions (pin/mute):** Call `pin()`/`mute()` directly per thread. These already emit their own `MutationLog`. No provider.

**For provider actions:**

1. Create provider once via `create_provider`.
2. If provider creation fails → enter **degraded fallback** (see below).
3. Iterate threads sequentially, calling `_with_provider` for each.
4. Track consecutive retryable remote failures. After 3 → enter **short-circuit fallback** for remaining threads.

### Degraded Fallback (provider-creation failure)

When `create_provider` fails, each thread is handled individually:

```rust
let provider_err = e; // the create_provider error string
for (idx, thread_id) in thread_indices {
    let mlog = MutationLog::begin(action_name, account_id, &thread_id);
    match action_local(ctx, action, account_id, &thread_id).await {
        Ok(()) => {
            let outcome = ActionOutcome::LocalOnly {
                reason: ActionError::remote(provider_err.clone()),
                retryable: true,
            };
            let (op_type, params_json) = enqueue_params(action);
            enqueue_if_retryable(ctx, &outcome, account_id, op_type, &thread_id, &params_json).await;
            mlog.emit(&outcome);
            results.push((idx, outcome));
        }
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            results.push((idx, outcome));
        }
    }
}
```

**Key rules:**
- Each thread gets its own `_local` call — outcome depends on whether `_local` succeeded or failed.
- `_local` success → `LocalOnly` (local applied, remote unavailable). Enqueued for retry.
- `_local` failure → `Failed` (nothing applied). NOT enqueued.
- Each thread gets its own `MutationLog` emission for observability.

### Consecutive-Failure Short-Circuit

```rust
const MAX_CONSECUTIVE_FAILURES: u32 = 3;

let mut consecutive_remote_failures: u32 = 0;

for (idx, thread_id) in thread_indices {
    if consecutive_remote_failures >= MAX_CONSECUTIVE_FAILURES {
        // Short-circuit: handle per-thread via degraded path
        let mlog = MutationLog::begin(action_name, account_id, &thread_id);
        match action_local(ctx, action, account_id, &thread_id).await {
            Ok(()) => {
                let outcome = ActionOutcome::LocalOnly {
                    reason: ActionError::remote("provider presumed unavailable"),
                    retryable: true,
                };
                let (op_type, params_json) = enqueue_params(action);
                enqueue_if_retryable(ctx, &outcome, account_id, op_type, &thread_id, &params_json).await;
                mlog.emit(&outcome);
                results.push((idx, outcome));
            }
            Err(e) => {
                let outcome = ActionOutcome::Failed { error: e };
                mlog.emit(&outcome);
                results.push((idx, outcome));
            }
        }
        continue;
    }

    // Normal path: _with_provider handles _local + dispatch + enqueue + log
    let outcome = dispatch_with_provider(ctx, provider, action, account_id, &thread_id).await;

    if let ActionOutcome::LocalOnly { reason, .. } = &outcome {
        if reason.is_retryable() {
            consecutive_remote_failures += 1;
        } else {
            consecutive_remote_failures = 0;
        }
    } else {
        consecutive_remote_failures = 0;
    }

    results.push((idx, outcome));
}
```

**Key rules:**
- Short-circuited threads are NOT mass-assigned `LocalOnly`. Each gets its own `_local` call, its own outcome, its own `MutationLog`.
- `_local` failure on a short-circuited thread → `Failed`, not `LocalOnly`.
- Counter resets on `Success`, `Failed`, or non-retryable `LocalOnly`.
- The first 3 failures go through `_with_provider` which handles its own `_local` + enqueue + log internally.

### Helper Functions

**Action-specific dispatch routing:**

```rust
async fn dispatch_with_provider(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    action: &BatchAction,
    account_id: &str,
    thread_id: &str,
) -> ActionOutcome {
    match action {
        BatchAction::Archive => archive::archive_with_provider(ctx, provider, account_id, thread_id).await,
        BatchAction::Trash => trash::trash_with_provider(ctx, provider, account_id, thread_id).await,
        BatchAction::Spam { is_spam } => spam::spam_with_provider(ctx, provider, account_id, thread_id, *is_spam).await,
        BatchAction::MoveToFolder { folder_id, source_label_id } => {
            move_to_folder::move_to_folder_with_provider(ctx, provider, account_id, thread_id, folder_id, source_label_id.as_deref()).await
        }
        BatchAction::Star { starred } => star::star_with_provider(ctx, provider, account_id, thread_id, *starred).await,
        BatchAction::MarkRead { read } => mark_read::mark_read_with_provider(ctx, provider, account_id, thread_id, *read).await,
        BatchAction::PermanentDelete => permanent_delete::permanent_delete_with_provider(ctx, provider, account_id, thread_id).await,
        BatchAction::AddLabel { label_id } => label::add_label_with_provider(ctx, provider, account_id, thread_id, label_id).await,
        BatchAction::RemoveLabel { label_id } => label::remove_label_with_provider(ctx, provider, account_id, thread_id, label_id).await,
        BatchAction::Pin { .. } | BatchAction::Mute { .. } => unreachable!("local-only actions don't use provider dispatch"),
    }
}
```

**Action-specific local fallback:**

```rust
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
        BatchAction::AddLabel { label_id } => add_label_local(ctx, account_id, thread_id, label_id).await.map(|_| ()),
        BatchAction::RemoveLabel { label_id } => remove_label_local(ctx, account_id, thread_id, label_id).await.map(|_| ()),
        BatchAction::Pin { .. } | BatchAction::Mute { .. } => unreachable!("local-only actions use direct calls"),
    }
}
```

Note: label `_local` returns `Result<(String, String), ActionError>` (label metadata). The `action_local` helper discards the metadata with `.map(|_| ())` — it's only needed for provider routing, which doesn't happen in the fallback path.

**Enqueue params derivation:**

```rust
fn enqueue_params(action: &BatchAction) -> (&'static str, String) {
    match action {
        BatchAction::Archive => ("archive", "{}".to_string()),
        BatchAction::Trash => ("trash", "{}".to_string()),
        BatchAction::Spam { is_spam } => ("spam", format!(r#"{{"isSpam":{is_spam}}}"#)),
        BatchAction::MoveToFolder { folder_id, source_label_id } => {
            ("moveToFolder", serde_json::json!({"folderId": folder_id, "sourceLabelId": source_label_id}).to_string())
        }
        BatchAction::Star { starred } => ("star", format!(r#"{{"starred":{starred}}}"#)),
        BatchAction::MarkRead { read } => ("markRead", format!(r#"{{"read":{read}}}"#)),
        BatchAction::PermanentDelete => ("permanentDelete", "{}".to_string()),
        BatchAction::AddLabel { label_id } => ("addLabel", serde_json::json!({"labelId": label_id}).to_string()),
        BatchAction::RemoveLabel { label_id } => ("removeLabel", serde_json::json!({"labelId": label_id}).to_string()),
        BatchAction::Pin { .. } | BatchAction::Mute { .. } => unreachable!("local-only actions don't enqueue"),
    }
}
```

**Action name for logging:**

```rust
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
```

## Observability Rules

Every code path emits a per-thread `MutationLog`:

| Path | Who emits MutationLog? |
|---|---|
| Normal (`_with_provider`) | `_dispatch` inside `_with_provider` |
| Provider-creation failure (degraded) | Batch executor's per-thread fallback loop |
| Short-circuit | Batch executor's per-thread fallback loop |
| Pin/mute (local-only) | `pin()`/`mute()` functions directly |

The batch summary log is **additional** — emitted by `batch_execute` after all threads complete.

## Files Changed

| File | Change |
|---|---|
| `crates/core/src/actions/batch.rs` | **New**: `BatchAction`, `batch_execute`, helpers |
| `crates/core/src/actions/mod.rs` | Add `pub mod batch; pub use batch::{batch_execute, BatchAction};` |
| `crates/core/src/actions/archive.rs` | `archive_local`: private → `pub(crate)` |
| `crates/core/src/actions/trash.rs` | `trash_local`: private → `pub(crate)` |
| `crates/core/src/actions/spam.rs` | `spam_local`: private → `pub(crate)` |
| `crates/core/src/actions/move_to_folder.rs` | `move_local`: private → `pub(crate)` |
| `crates/core/src/actions/star.rs` | `star_local`: private → `pub(crate)` |
| `crates/core/src/actions/mark_read.rs` | `mark_read_local`: private → `pub(crate)` |
| `crates/core/src/actions/permanent_delete.rs` | `permanent_delete_local`: private → `pub(crate)` |
| `crates/core/src/actions/label.rs` | `add_label_local`, `remove_label_local`: private → `pub(crate)` |

## Lint Compliance

- `batch_execute`: 3 params.
- `execute_account_group`: 4 params.
- `dispatch_with_provider`: 5 params.
- `action_local`: 4 params.
- `enqueue_params`: 1 param.
- No function should exceed 100 lines. `execute_account_group` is the longest — provider creation + main loop + short-circuit. If it approaches 100, extract the degraded fallback into a `handle_thread_degraded` helper.
- `cognitive_complexity`: match arms in dispatch helpers are flat.

## Exit Criteria

1. `batch_execute` compiles and is exported from `actions`.
2. Groups targets by account — N accounts = N provider constructions max.
3. Accounts execute in parallel via `futures::future::join_all`.
4. Outcomes are in the same order as input targets.
5. Consecutive-failure short-circuit after 3 retryable remote failures per account.
6. Degraded and short-circuit paths compute per-thread outcomes (not mass-assigned).
7. Every thread gets a `MutationLog` emission regardless of path (normal, degraded, short-circuit).
8. Pin/mute skip provider creation entirely.
9. Batch summary log emitted at info level.
10. `cargo check --workspace` + `cargo clippy -p ratatoskr-core` clean.
