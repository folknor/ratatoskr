# Action Service: Phase 3 Sub-Phase Breakdown

## Context

Phase 2 is complete. Every mutating action (email, send, folder, calendar, contact) flows through the action service. But the service is not yet trustworthy — it uses string errors, has inconsistent failure semantics, no observability, and no durability. Phase 3 fixes this.

## Inventory of Deferred Decisions

From Phases 1-2.6, the following were explicitly deferred to Phase 3:

1. **String error fields** — `ActionOutcome::Failed { error: String }` and `LocalOnly { remote_error: String }`. 20+ distinct error string patterns. No structure, no user-facing categories. (Phase 1 plan, line 63)

2. **Inconsistent `LocalOnly` semantics** — means different things:
   - "Provider failed" (email actions: archive, star, label)
   - "Provider not yet implemented" (Google/Graph/CardDAV contacts)
   - "Provider succeeded but local DB failed" (folder ops — currently returns `Success`, flagged in review)
   - "Missing identity for dispatch" (contacts with no server_id)

3. **No user-facing error categories** — the app handler joins error strings with `"; "` for toast display. No way to distinguish "network timeout" from "permission denied" from "not found" without parsing strings.

4. **`pending_operations` table exists but is never called** — full schema with retry backoff (60s, 5m, 15m, 1h), compaction (cancel toggle pairs, collapse moves), crash recovery (`recover_executing` resets stale ops to `pending`). Zero integration with any action function.

5. **No structured logging** — ad-hoc `log::warn!` across all actions. No consistent format, no duration, no retry count, no correlation ID.

6. **Crash recovery for send** — drafts stuck in `'sending'` state on app crash. Currently the user must re-compose. (Phase 2.3 plan)

7. **Rollback not generalized** — rollback is toggle-specific (bool flip in app handler). Phase 4 (undo) needs a general reversal mechanism. The implementation-phases doc says: "the rollback data structure must be undo-shaped from the start."

8. **`'finalized'` draft status** — in the `mark_draft_sending` validation set but never set by any code. Dead state. Remove in Phase 3.2 (it's not reserved for future use — no design requires it).

## Sub-Phase Structure

### Phase 3.1: Structured Error Types

**Goal:** Replace `String` errors with a structured enum that supports both user-facing messages and machine-readable retry decisions.

**Scope:**

Define `ActionError` with two layers — a top-level category and a kind that carries retry semantics:

```rust
#[derive(Debug, Clone)]
pub enum ActionError {
    /// Local database error (lock, query, constraint).
    Db { message: String },
    /// Remote provider operation failed.
    Remote {
        kind: RemoteFailureKind,
        message: String,
    },
    /// Resource not found (label, event, contact, draft).
    NotFound { resource: String },
    /// State machine violation (e.g., draft already sending).
    InvalidState { message: String },
    /// Payload construction failed (MIME build, JSON serialization).
    Build { message: String },
}

/// Distinguishes retryable from permanent remote failures.
/// Used by Phase 3.4 to decide whether to enqueue a pending op.
#[derive(Debug, Clone, PartialEq)]
pub enum RemoteFailureKind {
    /// Network error, timeout, 5xx — worth retrying.
    Transient,
    /// 4xx, permission denied, invalid request — will not succeed on retry.
    Permanent,
    /// Provider write-back not yet implemented (Google/Graph/CardDAV stubs).
    NotImplemented,
    /// Unknown completion — provider didn't respond clearly.
    Unknown,
}
```

- Replace `ActionOutcome::Failed { error: String }` with `Failed { error: ActionError }`.
- Replace `ActionOutcome::LocalOnly { remote_error: String }` with `LocalOnly { reason: ActionError }`.
- Add `ActionError::user_message(&self) -> String` — returns a user-facing summary for toast display, computed from the category and message. Returns owned `String` because messages may need context (e.g., "Archive failed: network timeout for account Work").
- Update all action functions to construct `ActionError` variants. Where possible, classify remote errors: network/timeout → `Transient`, HTTP 4xx/auth → `Permanent`, stubs → `NotImplemented`. Where the provider error is opaque (just a `String`), default to `Unknown`.
- Update all app handlers to use `error.user_message()` instead of raw string display.

**Why first:** Every subsequent sub-phase builds on structured errors. Observability needs error categories. The pending-ops queue needs `RemoteFailureKind::Transient` to decide whether to enqueue. Phase 4 (undo) needs to know whether the action partially completed.

**Exit criteria:** No `String` error fields on `ActionOutcome`. All error strings replaced with `ActionError` variants. `RemoteFailureKind` distinguishes retryable from permanent failures. App handlers use `user_message()` for display.

### Phase 3.2: Outcome Semantics Cleanup

**Goal:** Make `ActionOutcome` variants unambiguous. Each variant means exactly one thing regardless of which action returned it. Establish retry classification so the meaning of `LocalOnly` is clear.

**Scope:**

**`LocalOnly` gets a retry classification:**

```rust
LocalOnly {
    reason: ActionError,
    /// Whether this failure is a candidate for automatic retry via the
    /// pending-ops queue. Classified per action at the call site.
    retryable: bool,
}
```

The `retryable` flag is set by the action function based on the error kind and the action class. This pulls the "will this ever be retried?" decision into the outcome itself, rather than deferring it entirely to Phase 3.4. The pending-ops worker (3.4) checks `retryable` to decide whether to enqueue.

**Retry classification per action class** (decided here, implemented in 3.4):

| Action class | `retryable` on `LocalOnly`? | Rationale |
|---|---|---|
| Archive, trash, spam, move | **Yes** (if `Transient`/`Unknown`) | Silent divergence is the #1 user-visible bug. |
| Star, mark_read | **Yes** (if `Transient`/`Unknown`) | Reverting on sync is confusing even for minor state. |
| Label add/remove | **Yes** (if `Transient`/`Unknown`) | Label state should be durable. |
| Pin, mute | N/A | Never returns `LocalOnly` (local-only by design). |
| Send | N/A | Uses `local_drafts` state machine, not `LocalOnly`. |
| Folder create/rename/delete | N/A | Provider-first — never returns `LocalOnly`. |
| Calendar create | **No** | Low-priority; no retry mechanism for local-first calendar events yet. |
| Calendar update/delete | N/A | Provider-first. |
| Contact save | **No** | Best-effort write-back; contacts are lower-priority than email state. |
| Contact delete (unimplemented providers) | **No** | Degraded local-only; will be addressed when HTTP calls are wired. |

Default: `retryable = false` unless the action class is in the "Yes" list AND the error kind is `Transient` or `Unknown`. `Permanent` and `NotImplemented` errors are never retryable.

**Drop `ProviderOnly`:**

Reviewer 2 is right — `ProviderOnly` would add a variant used by only three folder functions in a rare edge case. The current behavior (log + return `Success`) is acceptable: the provider state is canonical, sync reconciles quickly, and the sidebar refreshes on next nav load. The log warning is sufficient observability for this case. Not worth the match-arm cost across all consumers.

**Drop `NoOp` from Phase 3.2:**

Both reviewers flagged this as underspecified. Current actions use idempotent writes (`INSERT OR IGNORE`, `DELETE WHERE`) and dispatch to providers regardless of whether the state was already correct. Detecting true no-ops would require pre-checking at both local and remote layers — significant refactoring for marginal value.

`NoOp` is primarily needed for Phase 4 (undo token suppression — "don't produce an undo token if nothing changed"). Defer to Phase 4 where the undo design can define the precise detection criteria.

**Remove `'finalized'` draft status:**

Remove `'finalized'` from the `mark_draft_sending` validation set. It was never set by any code and is not reserved for any future design. Dead state.

**Exit criteria:** `ActionOutcome::LocalOnly` carries `retryable: bool`. Retry classification documented per action class. `'finalized'` removed from draft state machine. No `ProviderOnly` or `NoOp` variants.

### Phase 3.3: Observability

**Goal:** Structured logging for all mutations — consistent format, duration, identity tracking.

**Scope:**

Define a `MutationLog` struct with distinct identity fields for local and remote resources:

```rust
struct MutationLog {
    action: &'static str,       // "archive", "star", "send", etc.
    account_id: String,
    local_id: String,           // thread_id, event_id, contact_id, draft_id
    remote_id: Option<String>,  // remote_event_id, server_id, resource_name (if known)
    local_result: &'static str, // "ok", "failed", "skipped"
    remote_result: &'static str,// "ok", "failed", "skipped", "not_implemented"
    error_kind: Option<&'static str>, // "transient", "permanent", "not_implemented", "db", etc.
    duration_ms: u64,
}
```

- `local_id` and `remote_id` are separate — send has `draft_id` + provider message ID, folder create has local label ID + provider mutation ID, calendar has event ID + `remote_event_id`. Single `resource_id` was too flat.
- Each action function measures wall-clock duration (`Instant::now()` at entry, elapsed at exit) and emits one `MutationLog::emit()` call.
- Log level: `info` for `Success`, `warn` for `LocalOnly`, `error` for `Failed`.
- Replace the 30+ ad-hoc `log::warn!` calls with `MutationLog::emit()`.
- JSON-structured logging: **deferred.** No log aggregation infrastructure exists. Structured fields in the human-readable log are sufficient. If JSON is needed later, `MutationLog` can implement `serde::Serialize` and emit JSON at a different log target.
- Batch cardinality: `MutationLog` is per-action-invocation (one thread, one event, one contact). Batch operations (Phase 5) will emit one log per item. Batch summary logging is a Phase 5 concern.

**Why third:** Benefits from structured errors (3.1) and clean outcome semantics (3.2). Each log entry references the `error_kind` from `ActionError`, not a raw string.

**Exit criteria:** Every action function emits exactly one structured log entry per invocation. The format is consistent across all actions. Duration is measured. `local_id` and `remote_id` are distinct fields.

### Phase 3.4: Pending-Action Queue Integration

**Goal:** Wire the existing `pending_operations` infrastructure for email actions where silent divergence is the #1 user-visible bug.

**Scope:**

The `pending_operations` table is fully implemented:
- Enqueue with operation type + params
- Status machine: pending → executing → (success/failed)
- Retry backoff: 60s, 5m, 15m, 1h (exponential with cap)
- Max retries: 10 (configurable per op)
- Compaction: cancels toggle pairs, collapses sequential moves
- Crash recovery: `recover_executing()` resets stale ops to pending

**Integration points:**
1. Action functions that return `LocalOnly { retryable: true, .. }` call `db_pending_ops_enqueue()` with the action type and parameters.
2. A periodic worker (on the sync timer or its own) calls `db_pending_ops_get()` and re-dispatches pending operations through the action service.
3. The worker uses `db_pending_ops_increment_retry()` on failure and `db_pending_ops_update_status("completed")` on success.
4. On app boot, `db_pending_ops_recover_executing()` resets any operations stuck in `'executing'` state.
5. Send crash recovery: on boot, detect `'sending'` drafts and transition to `'failed'` (same as orphaned `'queued'` drafts). Surfacing in outbox UI is a future concern.

**Which actions enqueue:** Per the classification in 3.2 — archive, trash, spam, move, star, mark_read, label add/remove. All with `retryable: true` and `RemoteFailureKind::Transient` or `Unknown`.

**Which actions do NOT enqueue:** Pin/mute (local-only), send (own state machine), folder ops (provider-first), calendar create (no retry), contact save/delete (low priority). These either never return `LocalOnly` or return it with `retryable: false`.

**Exit criteria:** Email actions enqueue on retryable `LocalOnly`. Periodic worker processes the queue. Crash recovery runs on boot. Send crash recovery transitions stale `'sending'` drafts to `'failed'`.

## Phase Boundaries

- **After 3.1:** Errors are structured with retry semantics. The app shows meaningful messages. All subsequent phases use `ActionError`.
- **After 3.2:** Outcomes are unambiguous with retry classification. Phase 4 can reason about what happened. Phase 3.4 knows which failures to enqueue.
- **After 3.3:** Every mutation is observable. Debugging sync divergence becomes tractable.
- **After 3.4:** Failed remote dispatches are retried. The most user-visible bug (actions reverting on sync) is addressed.

3.1 and 3.2 are the immediate priority — they make the service honest. 3.3 is straightforward once the types are settled. 3.4 is the big payoff but depends on all three preceding sub-phases.

## Interaction with Phase 4 (Undo)

Phase 3.2 (outcome semantics) produces variants that Phase 4 can reason about:
- `Success` → produce undo token with full reversal data
- `LocalOnly { retryable: true }` → produce undo token for local reversal AND cancel the pending op
- `LocalOnly { retryable: false }` → produce undo token for local reversal only
- `Failed` → no undo token (nothing happened)

`NoOp` detection (suppressing undo tokens for actions that didn't change state) is deferred to Phase 4 where the undo design defines the precise criteria.

Phase 3.4 (pending ops) interacts with undo: if an operation is pending retry, undoing it should cancel the pending op (via `db_pending_ops_update_status("cancelled")`) rather than performing a reverse operation.

## What This Does NOT Cover

- **Retry logic tuning** (backoff parameters, max retries per action type) — Phase 5.
- **Undo token design and execution** — Phase 4.
- **Bulk action semantics** (partial success across 50 threads, batch logging) — Phase 5.
- **Concurrency and ordering** (conflicting sequential actions on same thread) — Phase 5.
- **`NoOp` detection** — deferred to Phase 4.
- **`ProviderOnly` variant** — dropped. Folder ops log + return `Success` on local DB failure. Sync reconciles.
