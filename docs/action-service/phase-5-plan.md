# Action Service: Phase 5 — Bulk Actions and Retry

## Context

Bulk operations are the primary bottleneck. Archiving 50 threads from one account currently creates 50 provider instances (50 reqwest::Client allocations, 50 DB reads + AES-GCM decrypts for tokens) and executes sequentially. Phase 5 introduces provider reuse, per-account parallelism, and retry policy tuning.

The pending-ops retry worker has the same problem — processing 20 retries for one account creates 20 providers.

## Sub-Phases

### 5.1: Provider-reuse action variants

Extract `pub(crate) _with_provider` variants from all 9 action functions that call `create_provider`. The existing public functions become thin wrappers: create provider, then delegate.

**Pattern** (using archive as template from `crates/core/src/actions/archive.rs`):

```rust
// New: accepts pre-constructed provider
pub(crate) async fn archive_with_provider(
    ctx: &ActionContext,
    provider: &dyn ProviderOps,
    account_id: &str,
    thread_id: &str,
) -> ActionOutcome { /* local DB + provider dispatch + enqueue + log */ }

// Existing: thin wrapper
pub async fn archive(ctx: &ActionContext, account_id: &str, thread_id: &str) -> ActionOutcome {
    let provider = match create_provider(&ctx.db, account_id, ctx.encryption_key).await {
        Ok(p) => p,
        Err(e) => { /* same LocalOnly handling as today */ }
    };
    archive_with_provider(ctx, &*provider, account_id, thread_id).await
}
```

**Files:** `archive.rs`, `trash.rs`, `spam.rs`, `move_to_folder.rs`, `star.rs`, `mark_read.rs`, `permanent_delete.rs`, `label.rs` (both add/remove).

**Exit:** All 9 types have `_with_provider`. Public functions delegate. No behavior change.

### 5.2: Batch executor

New file `crates/core/src/actions/batch.rs` with:

```rust
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

pub async fn batch_execute(
    ctx: &ActionContext,
    action: BatchAction,
    targets: Vec<(String, String)>,
) -> Vec<ActionOutcome>
```

Returns `Vec<ActionOutcome>` directly — no wrapper struct. If we later need batch metadata (summary counts, skip reasons), we add fields then. Premature abstraction otherwise.

**Flow:**
1. Group targets by account_id → `HashMap<String, Vec<(usize, String)>>` (preserving original indices).
2. For each account group, spawn a future: create provider once, iterate threads sequentially calling `_with_provider`.
3. `futures::future::join_all` across account groups (parallel per-account, sequential within).
4. Reassemble outcomes in original order via indices.
5. Emit batch summary log: `[action-batch] archive | 50 threads / 3 accounts | 47 ok, 2 local-only, 1 failed | 1234ms`.

**Consecutive-failure short-circuit:** Within an account group, if 3 consecutive threads return `LocalOnly` with `RemoteFailureKind::Transient` or `Unknown`, abort remaining threads in that group with the same `LocalOnly` outcome (provider is presumed dead). This avoids 47 guaranteed failures after the provider dies on thread 3. The threshold of 3 allows for one-off transient errors without premature abort.

Pin/mute are local-only (no provider) — batch executor still groups them but skips provider creation.

**Per-thread MutationLog:** Each `_with_provider` call still emits its own `MutationLog::emit()`. Individual logs are kept at their current level (info/warn/error) for debugging. The batch summary is an additional log line emitted by `batch_execute` at info level for monitoring. Both coexist.

**Files:** New `crates/core/src/actions/batch.rs`, update `crates/core/src/actions/mod.rs`.

### 5.3: Wire app dispatch to batch executor

Replace the sequential loop in `dispatch_action_service_with_params` (`crates/app/src/handlers/commands.rs`) with a single `batch_execute` call. Add helper `to_batch_action(action, params) -> BatchAction`.

**Toggle dispatch** (`dispatch_toggle_action`): Targets carry per-thread values `(account_id, thread_id, new_value)`. These cannot be expressed as a single `BatchAction::Star { starred: bool }` when the batch has mixed values.

**Solution:** Partition targets by new_value, make separate `batch_execute` calls, merge outcomes:

```rust
// Partition
let mut true_indices = Vec::new();
let mut true_targets = Vec::new();
let mut false_indices = Vec::new();
let mut false_targets = Vec::new();
for (i, (aid, tid, val)) in targets.iter().enumerate() {
    if *val {
        true_indices.push(i);
        true_targets.push((aid.clone(), tid.clone()));
    } else {
        false_indices.push(i);
        false_targets.push((aid.clone(), tid.clone()));
    }
}

// Execute both partitions (can run in parallel via join)
let (true_outcomes, false_outcomes) = futures::future::join(
    batch_execute(&ctx, BatchAction::Star { starred: true }, true_targets),
    batch_execute(&ctx, BatchAction::Star { starred: false }, false_targets),
).await;

// Merge back into original order
let mut outcomes = vec![ActionOutcome::Success; targets.len()]; // placeholder
for (idx, outcome) in true_indices.into_iter().zip(true_outcomes) {
    outcomes[idx] = outcome;
}
for (idx, outcome) in false_indices.into_iter().zip(false_outcomes) {
    outcomes[idx] = outcome;
}
```

**Undo tokens for mixed toggles:** The existing `produce_undo_tokens` already handles this correctly. It groups by `(account_id, previous_value)` — producing separate tokens for threads that were starred vs unstarred before the toggle. This works regardless of whether the dispatch was batched or sequential. No change needed.

**Files:** `crates/app/src/handlers/commands.rs`.

**Exit:** Bulk archive of 50 threads creates N providers (N = distinct accounts). Undo, status bar feedback, auto-advance all unchanged.

### 5.4: Pending-ops worker provider reuse

Refactor `process_pending_ops` in `crates/core/src/actions/pending.rs`:
1. Fetch up to 20 ops (unchanged).
2. Group by account_id.
3. Per account group: create provider once, process ops sequentially with `_with_provider` dispatch.

**Execution is sequential across account groups** (not parallel). Retry is background work, not latency-sensitive. Parallelizing retry adds risk of provider contention and complicates error handling for no user-visible benefit. The win here is purely eliminating redundant provider constructions (20 ops for one account → 1 provider instead of 20).

**Files:** `crates/core/src/actions/pending.rs`.

### 5.5: Deduplication on enqueue

In `enqueue_if_retryable`, before inserting, check if a pending op already exists for `(account_id, resource_id, operation_type)` with `status IN ('pending', 'executing')`. If so, skip the insert.

**Why include `executing`:** An op that's currently being retried by the worker should not get a duplicate enqueued alongside it. If the executing op fails, it'll be re-queued by the worker. If it succeeds, the duplicate would be wasted.

**`failed` rows are NOT checked.** A failed op represents exhausted retries — a new user action on the same thread should create a fresh pending op with retry_count=0, superseding the old failure. The old failed row is cleaned up by `db_pending_ops_clear_failed` or the next compaction pass.

**Files:** `crates/core/src/actions/pending.rs`.

### 5.6: Per-action-type retry policy

Define `RetryPolicy { max_retries, backoff_schedule }` mapped by operation type:

| Action class | max_retries | Backoff | Rationale |
|---|---|---|---|
| archive, trash, spam, moveToFolder, permanentDelete | 10 | 30s, 2m, 5m, 15m, 1h | Folder-level: silent divergence is #1 bug |
| addLabel, removeLabel | 7 | 1m, 5m, 15m, 1h | Label state should be durable |
| star, markRead | 5 | 1m, 5m, 15m | Flag-level: sync reconciles |

Pass `max_retries` at enqueue time (column already exists, currently defaulted to 10). Use per-type backoff schedule at retry time (look up from `operation_type` stored on the row).

**Retry exhaustion behavior:** When `retry_count >= max_retries`, the op transitions to `status = 'failed'`. Exhausted ops are:
- **Logged** at `warn` level with the operation details and final error.
- **Left for sync reconciliation.** The next delta sync will pull the canonical server state, resolving the divergence. This is acceptable because the sync timer runs every 5 minutes.
- **Not surfaced in UI in this phase.** A future "pending actions" indicator can query `db_pending_ops_failed_count()`, which already exists. Phase 5 does not build that UI.
- **Eligible for manual retry** via `db_pending_ops_retry_failed()`, which resets failed ops to pending. No UI for this yet, but the function exists.

**Files:** `crates/core/src/actions/pending.rs`, `crates/core/src/db/pending_ops.rs`.

### 5.7: Concurrency guard

**Policy: one mutation at a time per thread, regardless of action type.** This is intentionally stronger than per-action-type guards. Rationale:

- **Ordering anomalies.** If archive and star run concurrently on the same thread, the star may complete first, then archive removes the thread from the inbox. The user sees a starred thread disappear — confusing. Serializing prevents this.
- **Provider state consistency.** Some providers (IMAP) maintain connection state per-mailbox. Concurrent operations on the same thread may interact in unexpected ways.
- **Simplicity.** A per-type guard requires defining a conflict matrix (which operations commute?). The conflict set is provider-dependent (Gmail label operations commute; IMAP folder operations don't). A blanket per-thread lock is correct for all providers.
- **Low cost.** In practice, concurrent operations on the *same thread* are rare — it requires the user clicking while a retry is in-flight for that specific thread. The guard silently skips, and the retry worker picks it up next cycle.

**Implementation:** Add `in_flight: Arc<Mutex<HashSet<String>>>` to `ActionContext` (key: `"{account_id}:{thread_id}"`).

- `batch_execute`: check+insert before dispatch, remove after. Threads already in-flight get `ActionOutcome::Failed { error: ActionError::invalid_state("action already in flight for this thread") }`.
- `process_pending_ops`: check before dispatch. If in-flight, skip the op (leave it pending for next cycle, do NOT increment retry count). Remove guard after completion.

**Files:** `crates/core/src/actions/context.rs`, `crates/core/src/actions/batch.rs`, `crates/core/src/actions/pending.rs`, app initialization site.

## Dependency Order

```
5.1 (provider-reuse variants)
  ├──> 5.2 (batch executor) ──> 5.3 (wire app)
  └──> 5.4 (pending-ops reuse)
5.5 (dedup) — independent
5.6 (retry policy) — independent
5.7 (concurrency guard) — after 5.2
```

Recommended: 5.1 first, then 5.2+5.5+5.6 in parallel, then 5.3+5.4+5.7 in parallel.

## Exit Criteria

1. Bulk archive of 50 threads across 3 accounts creates exactly 3 providers.
2. Accounts execute in parallel; threads within account execute sequentially.
3. Consecutive-failure short-circuit aborts account batch after 3 consecutive remote failures.
4. Partial success reported correctly ("Archived 48 of 50 threads").
5. Undo works for bulk actions (unchanged — one token per account, toggles grouped by prior state).
6. Pending-ops worker creates one provider per account batch, executes sequentially across groups.
7. No duplicate pending ops for same thread (dedup checks `pending` + `executing` status).
8. Retry policy varies by action type; exhausted ops logged and left for sync reconciliation.
9. In-flight guard prevents double-dispatch (same thread, any action type).
10. Batch summary log emitted alongside per-thread mutation logs.

## What Phase 5 Does NOT Do

- **Native provider batching** (JMAP `Email/set`, Graph `/$batch`) — future optimization.
- **Progress/cancellation UI** — future UX polish.
- **User-facing retry status** — no "3 actions pending" indicator. The `db_pending_ops_failed_count()` function exists for future UI.
- **Rate limiting** — sequential-within-account + bounded parallelism makes limits unlikely.

## Verification

- `cargo check --workspace` after each sub-phase
- `cargo clippy -p ratatoskr-core -p app` for lint compliance
- Manual test: select ~50 threads, archive, verify single status bar message + undo works
- Verify logs show batch summary + per-account provider creation (not per-thread)
- Test toggle on mixed selection (some starred, some not) — verify undo restores each thread's prior state
