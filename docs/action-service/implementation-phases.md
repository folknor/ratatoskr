# Action Service: Implementation Phases

This document sequences the implementation of the action service described in `problem-statement.md`. Each phase is independently shippable and improves the system even if subsequent phases are delayed or redesigned.

The phases are ordered by two principles: (1) establish the contract boundary early so new code is forced through it, and (2) tackle the hardest design decisions (failure semantics, concurrency) only after the basic path is proven with real actions flowing through it.

---

## Phase 1: Foundation — One Action, End-to-End ✅

**Status:** Complete. See `phase-1-plan.md`.

Archive flows through the action service (local DB + `ProviderOps::archive()`). `ActionContext`, `ActionOutcome`, and `create_provider` live in core. The app handler delegates to the service. Outcomes are surfaced to the user. Auto-advance is conditional on success.

---

## Phase 2: Migrate All Write Operations

Phase 2 is decomposed into sub-phases because provider write operations are not uniform. Some have real implementations across all four providers. Others require resolving provider-specific interfaces, new trait methods, or design decisions about provider capability gaps.

### Phase 2.1: Thread actions with uniform provider support

**Goal:** Migrate the thread-level actions where all four providers have real `ProviderOps` implementations. Same pattern as archive — mechanical replication.

**Scope:**
- **Folder moves (all providers implemented):** trash, spam, move_to_folder.
- **Boolean flags with provider dispatch (all providers implemented):** star, mark_read.
- **Destructive (all providers implemented):** permanent_delete.
- **Local-only by design:** pin, mute. Go through the service with an explicit local-only marker.
- Remove the legacy `dispatch_email_db_action` function and all remaining inline DB mutations for these actions from the app crate.

**Snooze is deferred from 2.1.** No `ProviderOps::snooze()` exists, and Gmail's native snooze has no equivalent on other providers. This requires a design decision (local-only by design? provider dispatch where supported? new trait method?) that doesn't belong in a mechanical migration phase. Snooze gets its own planning when it's prioritized.

**Concurrency semantics are deferred.** Defining ordering guarantees is a design decision, not a mechanical migration. Current behavior (fire-and-forget async per action) is preserved. Concurrency is addressed in Phase 5 when the full action set is in place and the real contention patterns are visible.

**Exit criteria:** All thread-level email actions go through the service. `dispatch_email_db_action` is deleted. `handlers/commands.rs` contains only service calls + UI state management for these actions.

### Phase 2.2: Label routing

**Goal:** Migrate label apply/remove into the service, owning the `label_kind` dispatch.

**Scope:**
- Move `provider_label_write_back` from the app crate into the service.
- The service owns the routing decision: `label_kind = 'tag'` → `apply_category`/`remove_category`, `label_kind = 'container'` → `add_tag`/`remove_tag`.
- IMAP's intentional no-op on `add_tag`/`remove_tag` must be represented explicitly (not silently swallowed).
- Address the labels unification spec's direction: `apply_category`/`remove_category` are supposed to become redundant in favor of `add_tag`/`remove_tag`. Decide whether Phase 2.2 consolidates them or preserves the current split for now.

**Exit criteria:** Label apply/remove goes through the service. The app crate no longer contains `label_kind` branches or `provider_label_write_back`.

### Phase 2.3: Drafts and send

**Goal:** Bring draft lifecycle and send into the action service.

**Scope:**
- send_email, create_draft, update_draft, delete_draft.
- Currently, send is deferred to the sync pipeline (local_drafts queue). The action service may own the local staging and let sync handle dispatch, or take over dispatch entirely. This is a design decision for this sub-phase.
- Draft auto-save is currently local-only. Decide whether provider draft sync goes through the service or remains in the sync pipeline.

**Note:** If the local staging vs remote dispatch semantics prove entangled with failure policy, this sub-phase may be better sequenced after Phase 3 rather than before it.

**Exit criteria:** The app crate's compose send and draft save paths go through the service. The local_drafts staging pattern is either owned by the service or explicitly delegated to sync with a documented rationale.

### Phase 2.4: Folder management

**Goal:** Bring folder CRUD into the action service.

**Scope:**
- create_folder, rename_folder, delete_folder.
- All four providers have real implementations on `ProviderOps`.
- Not yet wired from any UI — this phase defines the service API and wires it so that when folder management UI is built, it goes through the service from day one.

**Exit criteria:** Folder CRUD functions exist in the action service. When folder management UI is built, it calls the service.

### Phase 2.5: Calendar event write-back

**Goal:** Wire calendar event mutations through an authoritative write path.

**Scope:**
- Create, update, delete calendar events.
- Provider implementations exist for Graph, Gmail, and JMAP but are never called from app handlers. Events are local DB only today.
- Calendar write operations are not on `ProviderOps` — they use separate per-provider APIs.

**Note:** Calendar and contact writes are different domains from email actions. They may warrant their own service modules (`core::calendar_actions`, `core::contact_actions`) rather than expanding `core::actions` into a grab-bag. The shared infrastructure (context, outcome types, provider resolution pattern) should be reusable, but the domain logic should not be forced into the same module. Decide during planning.

**Exit criteria:** Calendar event save/delete goes through a centralized write path with provider dispatch where supported.

### Phase 2.6: Contact write-back

**Goal:** Wire contact mutations through an authoritative write path.

**Scope:**
- Save/delete contacts to providers.
- JMAP is furthest along (full implementation exists). Google and Graph have scaffolding but need HTTP calls wired. CardDAV needs PUT support.
- Same module placement question as 2.5 — likely `core::contact_actions` or similar, not `core::actions`.

**Exit criteria:** Contact save for synced contacts goes through a centralized write path with provider dispatch where supported.

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
- Remove `create_provider()` wrapper and any provider construction helpers from the app crate.
- Remove `ProviderCtx` construction from the app crate.
- Audit and remove any remaining `label_kind` branches, provider-specific imports, or direct provider type usage in the app crate.
- If any legitimate app-crate need for provider access remains (e.g., account setup, OAuth flows), define a narrow, explicit API for it rather than exposing the full provider surface.

**Design decisions made in this phase:**
- What (if anything) the app crate is still allowed to know about providers.
- How account setup / OAuth flows access provider crates without opening a back door.

**Exit criteria:** Removing any provider crate from the app's dependency list does not break compilation (because the app doesn't use them). The action service is the only write path for all provider mutations.

---

## Phase Boundaries and Replanning

Each phase is designed to be independently valuable:

- **After Phase 1:** One action works end-to-end. The pattern is proven. ✅
- **After Phase 2.1:** All thread-level email actions go through the service.
- **After Phase 2.2:** Label routing is centralized. No `label_kind` branches in the app crate.
- **After Phase 2.3:** Draft and send lifecycle goes through the service.
- **After Phase 2.4–2.6:** Folder, calendar, and contact writes go through the service.
- **After Phase 3:** The service is trustworthy. Failure handling is consistent, observable, and explicitly defined.
- **After Phase 4:** Undo works correctly for the first time.
- **After Phase 5:** Bulk operations are handled and remote dispatch is reliable.
- **After Phase 6:** The boundary is enforced by the compiler.

Phases 2.1–2.2 are the immediate priority. Phases 2.3–2.6 can be ordered by product need. Phases 3–5 build on the migrated actions. Phase 6 is a cleanup pass that can happen any time after Phase 2 is substantially complete.

Phase 2 sub-phases are ordered by implementation risk: 2.1 is mechanical (uniform provider support), 2.2 requires resolving the label routing design, 2.3–2.6 each require interface decisions (new traits, provider capability gaps, sync pipeline interaction).

Detailed planning for each phase happens before that phase starts, not upfront.
