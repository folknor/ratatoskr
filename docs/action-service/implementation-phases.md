# Action Service: Implementation Phases

This document sequences the implementation of the action service described in `problem-statement.md`. Each phase is independently shippable and improves the system even if subsequent phases are delayed or redesigned.

The phases are ordered by two principles: (1) establish the contract boundary early so new code is forced through it, and (2) tackle the hardest design decisions (failure semantics, undo) only after the basic path is proven with real actions flowing through it.

---

## Phase 1: Foundation — One Action, End-to-End

**Goal:** Prove the action service pattern with one real action, making only the design commitments necessary to ship it.

Archive is the first action because it is the most common user-initiated mutation, immediately user-visible (thread leaves inbox), and exercises both local DB mutation (remove INBOX label) and provider dispatch (Gmail label remove, IMAP COPY+DELETE, Exchange folder move, JMAP mailbox update). If the pattern works for archive, it works for everything.

**Scope:**
- Create the action service module. Crate placement is a Phase 1 decision, but the bias is toward starting inside `ratatoskr-core` and extracting later if needed — avoid premature crate boundaries while the API is still being discovered.
- Define just enough types to ship archive: an action context (dependencies the service needs), a result type (success or failure with reason), and the archive function itself.
- Implement archive: local DB mutation + provider dispatch through `ProviderOps`.
- Migrate the app crate's archive handler to call the service.
- Verify the app crate no longer performs archive's provider dispatch directly.

**What this phase discovers (not decides prematurely):**
- The context shape will emerge from what archive actually needs. Don't over-design it for actions that don't exist yet.
- The result type starts minimal (success/failure). It will grow when Phase 3 forces the failure policy.
- The async model is whatever archive needs. Generalize in Phase 2 when more actions reveal the pattern.

**What this phase does NOT do:**
- No failure policy beyond "log and return error."
- No undo tokens.
- No bulk actions.
- No pending-action queue or retry.

**Exit criteria:** Archive flows through the service end-to-end (local + remote). The app handler is a one-liner. The pattern is proven and ready for replication.

---

## Phase 2: Migrate Remaining Actions

**Goal:** Move all email actions through the service. Establish minimal concurrency semantics. Eliminate inline provider dispatch from the app crate.

**Scope:**
- Migrate each mutation class:
  - **Folder moves:** trash, mark-as-spam, move-to-folder.
  - **Boolean flags with provider sync:** star, read/unread.
  - **Boolean flags, local-only by design:** pin, mute. These go through the service with an explicit local-only marker so the distinction between "local by design" and "local because nobody wired the provider" is visible in the code.
  - **Label mutations:** apply label, remove label. Relocate existing provider write-back from app into service.
  - **Destructive:** permanent delete.
  - **Deferred:** snooze.
- Each action explicitly declares whether it requires provider dispatch, is local-only by design, or is local-only due to missing provider support.
- Remove all direct provider construction and dispatch from the app crate's handlers.
- Define minimal concurrency semantics: at minimum, actions on the same thread must not interleave. This does not require a full concurrency framework — it may be as simple as per-thread serialization or a check-and-warn. The point is to have a defined stance before multiple concurrent actions are possible, not to solve the general problem.

**Design decisions made in this phase:**
- Per-action provider dispatch policy.
- How local-only-by-design is represented.
- How unsupported provider operations are handled.
- Minimal concurrency contract (enough to be safe, not necessarily optimal).

**What this phase does NOT do:**
- No change to failure handling — still "log and return error."
- No undo execution.
- No bulk actions.

**Internal sequencing:** Phase 2 migrates ~10 actions. The order matters — label mutations should migrate before folder moves, since folder moves are label mutations on Gmail. Detailed internal ordering is determined during Phase 2 planning, not here.

**Exit criteria:** Every email action goes through the service. The app crate does not construct providers or dispatch provider calls. `handlers/commands.rs` contains only service calls + UI state management.

**Value after this phase:** The execution path is centralized and the provider abstraction is hidden from the app crate. This is structural progress — the right code is in the right place. But failure handling is still rudimentary. Actions that fail remotely after succeeding locally will still silently diverge until Phase 3 defines what to do about it.

---

## Phase 3: Failure Policy and Structured Outcomes

**Goal:** Define and implement the partial-failure semantics. This is where the service becomes trustworthy.

**Scope:**
- Define the failure policy per mutation class:
  - **Local success + remote failure.** The policy may differ by action class. Archive and trash likely need pending-retry or rollback — silent divergence is unacceptable for folder-level actions. Star and read/unread may tolerate local-only with eventual sync reconciliation. The policy must be explicit per action, not a blanket rule.
  - **Remote timeout / unknown completion.** Define local state behavior.
  - **App shutdown mid-flight.** Decide whether a pending-action table is needed or whether sync reconciliation is sufficient.
- Expand the result type to convey: success, partial success (local ok / remote failed), failure, no-op. Include a user-facing result category so the app can show appropriate feedback without interpreting error internals.
- Implement structured logging for all mutations (action, target, local result, remote result, duration).
- If the failure policy requires a pending-action table (for durability or retry), define the schema and implement it.

**Design decisions made in this phase:**
- The actual partial-failure policy — this is the hardest design work in the entire effort.
- Whether a pending-action queue exists and what it contains.
- The mature shape of the result type.
- Observability format and level.

**Constraint from Phase 4:** The rollback mechanism designed here must be a general state-reversal primitive, not a failure-specific one. Phase 4 will reuse it for user-initiated undo — same mechanism, different trigger. This does not mean implementing undo in Phase 3, but the rollback data structure must be undo-shaped from the start. If rollback captures "what was done and how to reverse it," undo gets that for free. If rollback only captures "something failed," Phase 4 will have to redesign it.

**What this phase does NOT do:**
- No retry logic (just failure recording and policy). Retry comes in Phase 5.
- No undo execution (but the rollback data is designed to support it).

**Exit criteria:** Every action returns a structured outcome. Failure cases are handled consistently per the defined policy. Mutations are observable. The most common user-facing bug (actions reverting on sync) is either prevented or explicitly surfaced.

---

## Phase 4: Undo

**Goal:** Undo tokens reflect executed operations. Undo execution goes through the service.

**Scope:**
- Redesign `UndoToken` to be produced by the service based on what actually happened, not what the caller requested. The token captures local mutation details and remote dispatch result.
- Implement undo execution through the service — `actions::undo(token)` performs the inverse operation via the same action service path.
- Handle edge cases: undo a no-op (no token produced), undo when remote failed (only reverse local), undo after sync has already reconciled (token is stale).
- Connect undo to the failure/rollback mechanism from Phase 3 — they are the same operation with different triggers (user-initiated vs failure-initiated).

**Design decisions made in this phase:**
- Undo token structure and what it captures.
- Undo staleness detection.
- Whether undo is best-effort or guaranteed.
- Interaction between undo and the pending-action queue (if one exists from Phase 3).

**Exit criteria:** Undo reverses what actually happened. Undo for partially-completed actions does the right thing. The UI's undo stack uses service-produced tokens.

---

## Phase 5: Bulk Actions and Retry

**Goal:** Handle batch operations and remote dispatch reliability.

**Scope:**
- **Bulk actions:** The service accepts a batch of targets for a single action. Define: mixed-account partitioning (service handles it), partial success reporting (per-item outcomes), batch undo token shape (informed by Phase 4 experience).
- **Retry:** If Phase 3 established a pending-action queue, implement retry with backoff for failed remote dispatches. Define retry limits and exhaustion behavior (surface to user, leave for sync, drop).

**Design decisions made in this phase:**
- Retry policy (backoff strategy, limits, exhaustion behavior).
- Bulk undo granularity (one token per batch or per item).
- How bulk partial success is reported to the caller.

**What this phase does NOT do:**
- No progress reporting or cancellation UI. Those are UX polish that can follow independently.

**Exit criteria:** Bulk archive of 50 mixed-account threads works correctly with partial failure handling and undo. Failed remote dispatches are retried per policy.

---

## Phase 6: Enforce the Boundary

**Goal:** Make the compilation boundary airtight. The app crate physically cannot bypass the service.

**Scope:**
- Remove all provider crate dependencies from the app crate's `Cargo.toml`.
- Remove `create_provider()` and any provider construction helpers from the app crate.
- Remove `ProviderCtx` construction from the app crate.
- Audit and remove any remaining `label_kind` branches, provider-specific imports, or direct provider type usage in the app crate.
- If any legitimate app-crate need for provider access remains (e.g., account setup, OAuth flows), define a narrow, explicit API for it rather than exposing the full provider surface.

**Design decisions made in this phase:**
- What (if anything) the app crate is still allowed to know about providers.
- How account setup / OAuth flows access provider crates without opening a back door.

**Exit criteria:** Removing any provider crate from the app's dependency list does not break compilation (because the app doesn't use them). The action service is the only write path for email state mutations.

---

## Phase Boundaries and Replanning

Each phase is designed to be independently valuable:

- **After Phase 1:** One action works end-to-end. The pattern is proven.
- **After Phase 2:** All actions go through the service. The path is centralized and provider logic is out of the app crate. Failure handling is still rudimentary — this is structural progress, not correctness.
- **After Phase 3:** The service is trustworthy. Failure handling is consistent, observable, and explicitly defined. This is where the "silent divergence" problem is actually solved.
- **After Phase 4:** Undo works correctly for the first time.
- **After Phase 5:** Bulk operations are handled and remote dispatch is reliable.
- **After Phase 6:** The boundary is enforced by the compiler.

Phases 1–3 are the critical path — they take the system from "no contract" to "trustworthy contract." Phase 4–5 can be reordered based on what hurts most in practice. Phase 6 is a cleanup pass that can happen any time after Phase 2.

Detailed planning for each phase happens before that phase starts, not upfront. The design decisions listed in each phase are the ones that must be resolved during that phase's planning — attempting to resolve them all now would produce speculative answers.
