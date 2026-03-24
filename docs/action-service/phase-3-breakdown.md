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

8. **`'finalized'` draft status** — in the `mark_draft_sending` validation set but never set by any code. Dead state.

## Sub-Phase Structure

### Phase 3.1: Structured Error Types

**Goal:** Replace `String` errors with a structured enum. Every action returns categorized errors that the app can act on without parsing strings.

**Scope:**
- Define `ActionError` enum with variants covering the ~5 base error patterns found in the inventory:
  - `Db(String)` — local database errors (lock, query, constraint)
  - `Provider(String)` — remote provider operation failed
  - `ProviderNotImplemented(String)` — write-back stub (Google/Graph/CardDAV contacts, IMAP folders)
  - `NotFound(String)` — label/event/contact/draft not found
  - `InvalidState(String)` — state machine violation (draft already sending)
  - `Build(String)` — MIME or payload construction failed
- Replace `ActionOutcome::Failed { error: String }` with `Failed { error: ActionError }`.
- Replace `ActionOutcome::LocalOnly { remote_error: String }` with `LocalOnly { reason: ActionError }`.
- Add `ActionError::user_message(&self) -> &str` — returns a user-facing summary for toast display, without exposing internals (e.g., "Send failed: network error" not "reqwest::Error: connection refused").
- Update all action functions to construct `ActionError` variants instead of `format!()` strings.
- Update all app handlers to use `error.user_message()` instead of raw string display.

**Why first:** Every subsequent sub-phase builds on structured errors. Observability needs error categories. The pending-ops queue needs to serialize error types. Undo needs to know *why* something failed.

**Exit criteria:** No `String` error fields on `ActionOutcome`. All error strings replaced with `ActionError` variants. App handlers use `user_message()` for display.

### Phase 3.2: Outcome Semantics Cleanup

**Goal:** Make `ActionOutcome` variants unambiguous. Each variant means exactly one thing regardless of which action returned it.

**Scope:**
- **Refine `LocalOnly`** into two distinct variants:
  - `LocalOnly { reason: ActionError }` — local mutation succeeded, provider dispatch failed or was skipped. The local state is the desired state (used by local-first actions: email, contacts). Sync may revert.
  - `ProviderOnly { reason: ActionError }` — provider succeeded, local DB update failed. The provider state is canonical; sync will reconcile. (Used by provider-first actions: folder create/rename/delete). Currently these return `Success` with a log warning — they should surface the degraded state.
- **Add `NoOp`** — the action was a no-op (e.g., archiving a thread already not in inbox, deleting a nonexistent label). No undo token should be produced.
- **Clarify `Success`** — means both local and provider succeeded (for actions with provider dispatch) or local succeeded (for local-only-by-design actions like pin/mute). No ambiguity.
- **Add `local_only_by_design: bool`** to `Success`? Or keep it implicit (pin/mute always return `Success`, caller knows they're local-only by the action type)? Decision: keep implicit. The action type is known at the call site. Adding a flag would be noise for the common case.
- Update all action functions and app handlers for the new variants.
- Update `handle_action_completed` to handle `ProviderOnly` — show a "saved, syncing..." type message instead of either success or failure.

**Why second:** Depends on structured errors (3.1) for the `reason` fields. Informs observability (3.3) — log entries need to distinguish LocalOnly from ProviderOnly. Informs pending-ops (3.4) — only LocalOnly actions with provider failure are candidates for retry.

**Exit criteria:** `ActionOutcome` has precise semantics documented per variant. No action returns `Success` when the local state is stale. Folder ops return `ProviderOnly` on local DB failure. The `LocalOnly` variant is only used for local-first actions where local succeeded but provider didn't.

### Phase 3.3: Observability

**Goal:** Structured logging for all mutations — consistent format, duration, correlation.

**Scope:**
- Define a `MutationLog` struct:
  ```rust
  struct MutationLog {
      action: &'static str,      // "archive", "star", "send", etc.
      account_id: String,
      resource_id: String,       // thread_id, event_id, contact_id, etc.
      local_result: &'static str, // "ok", "failed", "skipped"
      remote_result: &'static str, // "ok", "failed", "skipped", "not_implemented"
      error: Option<String>,
      duration_ms: u64,
  }
  ```
- Each action function measures wall-clock duration and emits a structured log at the end.
- Log level: `info` for success, `warn` for LocalOnly/ProviderOnly, `error` for Failed.
- Replace the 30+ ad-hoc `log::warn!` calls with `MutationLog::emit()`.
- Consider JSON-structured logging for machine parsing (optional — depends on whether log aggregation is a near-term need).

**Why third:** Benefits from structured errors (3.1) and clean outcome semantics (3.2). Each log entry references the error category, not a raw string.

**Exit criteria:** Every action function emits exactly one structured log entry per invocation. The format is consistent across all actions. Duration is measured.

### Phase 3.4: Pending-Action Queue Integration

**Goal:** Decide and implement the durability strategy. The `pending_operations` table exists with full retry, compaction, and crash recovery infrastructure — but no action calls it.

**Scope — decision first:**

The `pending_operations` table was built for this. It has:
- Enqueue with operation type + params
- Status machine: pending → executing → (success/failed)
- Retry backoff: 60s, 5m, 15m, 1h (exponential with cap)
- Max retries: 10 (configurable per op)
- Compaction: cancels toggle pairs, collapses sequential moves
- Crash recovery: `recover_executing()` resets stale ops to pending

**The decision:** Which actions should enqueue pending ops on `LocalOnly` (local succeeded, provider failed)?

| Action class | Enqueue on failure? | Rationale |
|---|---|---|
| Archive, trash, spam, move | **Yes** | Silent divergence is the #1 user-visible bug. These must eventually reach the provider. |
| Star, mark_read | **Maybe** | Less critical — sync reconciliation handles it. But if the user stars a message and it unstars on next sync, that's still confusing. |
| Label add/remove | **Yes** | Same reasoning as archive — label state should be durable. |
| Pin, mute | **No** | Local-only by design. No provider dispatch exists. |
| Send | **No** | Send uses its own `local_drafts` state machine. The draft row IS the pending op. |
| Folder create/rename/delete | **No** | Provider-first. If the provider fails, the local state isn't modified. Nothing to retry. |
| Calendar create | **Maybe** | Local-first with `LocalOnly` on failure. Could enqueue for retry. |
| Calendar update/delete | **No** | Provider-first. Failure means local not modified. |
| Contact save | **Maybe** | Local-first with best-effort write-back. Could enqueue for retry, but contacts are lower-priority than email state. |
| Contact delete | **No for JMAP** (provider-first), **Maybe for others** (degraded local-only). |

**Implementation (if decided to integrate):**
- Action functions that return `LocalOnly` call `db_pending_ops_enqueue()` with the action type and parameters.
- A periodic worker (on the sync timer or its own) calls `db_pending_ops_get()` and re-dispatches pending operations through the action service.
- The worker uses `db_pending_ops_increment_retry()` on failure and `db_pending_ops_update_status("completed")` on success.
- On app boot, `db_pending_ops_recover_executing()` resets any operations stuck in `'executing'` state.
- Send's crash recovery: on boot, detect `'sending'` drafts and either retry or surface in outbox UI. This uses the `local_drafts` table, not `pending_operations`.

**Why last:** This is the most complex sub-phase and benefits from stable error types (3.1), clean outcomes (3.2), and observability (3.3). It's also optional in the short term — the action service works without it, just without durability guarantees.

**Exit criteria:** Explicit decision documented per action class. If integrated: email actions enqueue on LocalOnly, periodic worker processes the queue, crash recovery runs on boot.

## Phase Boundaries

- **After 3.1:** Errors are structured. The app can show meaningful error messages. All subsequent phases use `ActionError`.
- **After 3.2:** Outcomes are unambiguous. No more "Success" when the state is degraded. Undo (Phase 4) can reason about what happened.
- **After 3.3:** Every mutation is observable. Debugging sync divergence becomes tractable.
- **After 3.4:** Failed remote dispatches are retried. The most user-visible bug (actions reverting on sync) is addressed.

3.1 and 3.2 are the immediate priority — they make the service honest. 3.3 is straightforward once the types are settled. 3.4 is the big payoff but depends on all three preceding sub-phases.

## Interaction with Phase 4 (Undo)

Phase 3.2 (outcome semantics) must produce variants that Phase 4 can reason about:
- `Success` → produce undo token with full reversal data
- `LocalOnly` → produce undo token for local reversal only (provider was never notified, so undo doesn't need to un-notify)
- `ProviderOnly` → produce undo token for provider reversal (local was never modified, but this is unusual)
- `Failed` → no undo token (nothing happened)
- `NoOp` → no undo token (nothing changed)

Phase 3.4 (pending ops) interacts with undo: if an operation is pending retry, undoing it should cancel the pending op rather than performing a reverse operation.

## What This Does NOT Cover

- **Retry logic details** (backoff tuning, max retries per action type) — Phase 5.
- **Undo token design and execution** — Phase 4.
- **Bulk action semantics** (partial success across 50 threads) — Phase 5.
- **Concurrency and ordering** (conflicting sequential actions on same thread) — Phase 5.
