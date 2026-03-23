# Action Service: Problem Statement

## The Missing Invariant

Every mutating email action must execute through one authoritative path that defines local mutation, remote dispatch, failure handling, undo capture, and observability.

This invariant does not exist in the codebase today. Correctness of email state mutations depends on handler authors manually remembering a sequence of steps, in the right order, with the right error handling, for the right provider. There is no mechanism — at the type level, crate boundary, or runtime — that enforces this.

## Symptoms

**No single execution path.** Each email action (archive, trash, star, read/unread, label apply/remove, move, snooze, delete, pin, mute) is implemented independently in UI-layer handler code. Each handler makes its own decisions about what steps to perform and in what order.

**Provider logic in the UI layer.** The app crate contains `label_kind` dispatch branches, constructs `ProviderCtx` structs with dummy dependencies, and imports provider-specific types. The app crate can — and does — reach through the abstraction boundary to make provider-specific calls directly.

**Inconsistent error handling.** Some write operations log errors, some swallow them, some surface them to the status bar. There is no policy and no common error type. Each handler author makes their own call.

**Incomplete provider dispatch.** Some actions update the local DB but never notify the provider. Others have provider write-back for some providers but not others. There is no inventory of what works and no way to determine coverage without reading every handler.

**Undo without execution history.** Undo tokens are created for 13 email commands at the UI layer, but the tokens are based on what the user clicked, not what actually executed. Some actions never reached the provider. Some may have partially completed. The undo system cannot reliably reverse what it cannot observe.

## Consequences

**Data divergence.** Local DB operations succeed, provider write-back is missing or fails silently, and the next sync pulls the old state from the server. The user sees actions revert: archived threads reappear in the inbox, read status flips back, labels vanish. This is the most user-visible failure and the primary motivation for this work.

**Inability to reason about correctness.** No one can answer "what happens when a user archives a thread?" without reading the specific handler code, checking whether it has provider dispatch, checking which providers it covers, and checking what happens on failure. This question should have one answer, not one per call site.

**Compounding cost of new features.** Every new feature that modifies email state (RSVP responses, contact write-back, calendar event creation, shared mailbox send-as, public folder replies) must independently solve: provider resolution, context setup, local DB update, remote dispatch, error handling, undo capture. Each will do it differently. The mess grows linearly with feature count.

**Regressions are invisible.** When a handler is modified or a new one is added, there is no test, type check, or compilation boundary that verifies the full write path is intact. A refactor can silently drop the provider dispatch step and no one will know until a user reports that actions don't stick.

## Current State by Mutation Class

Actions are not uniform. They group into classes with different provider semantics:

**Label/keyword mutations** (apply label, remove label): Additive tag semantics. Provider operation depends on `label_kind` — tags use flag/property set operations (IMAP STORE +FLAGS, Exchange PATCH categories, JMAP keyword set), containers use move semantics. Currently wired with provider write-back.

**Folder move/copy** (archive, move-to-folder): Exclusive-membership semantics at the provider level. Archive is "remove from inbox" (Gmail: label remove, IMAP: COPY to archive + DELETE from inbox, Exchange: folder move). Move-to-folder is similar but targets a specific folder. Currently local DB only — no provider dispatch.

**Boolean flags** (read/unread, star, pin, mute): Toggle operations on thread-level fields. Star has provider write-back. Read/unread, pin, and mute are local DB only. Pin and mute have no native provider equivalent on most providers — they are local-only concepts, and that is intentional. The service must distinguish "local-only by design" from "local-only because provider dispatch is missing."

**Destructive actions** (trash, permanent delete, mark as spam): Remove from current view, add to a system folder or delete entirely. Trash and spam are label/folder moves at the provider level. Permanent delete is irreversible. Currently local DB only — no provider dispatch.

**Deferred actions** (snooze): Remove from inbox, set a timer, return on expiry. Involves both a label mutation (remove INBOX) and a temporal state change (snooze_until). Currently local DB only.

## Required Properties of Any Solution

### Single authoritative execution path

Every email state mutation enters through one function or method. The caller provides an action and a target. Everything else — local DB update, provider resolution, remote dispatch, failure handling, undo context, observability — is the service's responsibility. The caller cannot skip steps because the API does not expose them as separate operations.

### Provider abstraction hidden from callers

The app crate (and any future UI crate) must not import provider crates, construct provider contexts, or branch on provider-specific properties like `label_kind`. The service resolves the provider internally. If the app crate can compile without provider crate dependencies, the boundary is enforced.

### Explicit partial-failure policy

The service must define and consistently implement a policy for every failure mode:

- **Local success, remote failure.** Is the local mutation rolled back, marked as pending, left for sync reconciliation, or surfaced as degraded? The policy may differ by action class (e.g., star might tolerate local-only, but archive must not silently diverge).
- **Local failure.** Remote dispatch is not attempted. Error surfaced to caller.
- **Remote timeout with unknown completion.** The service does not know whether the provider applied the change. What is the local state? How does sync reconciliation handle this?
- **App shutdown mid-flight.** An action was dispatched but the result was never received. On next launch, is there a pending-action queue, or does sync reconciliation handle it?
- **Provider accepts request but sync later reflects different state.** Another client or server-side rule reversed the change. This is sync's problem, but the action service's local staging decisions affect what conflicts sync must resolve.

### Intentional local-only actions

Some mutations are local-only by design (pin, mute). The service must distinguish these from mutations that are local-only because provider dispatch is missing. This distinction must be explicit in the action definitions, not implicit in whether someone remembered to write the provider call.

### Unsupported provider operations

When a provider does not support an operation (e.g., IMAP server without custom keyword support, Exchange at its 25-category cap), the service must have a defined behavior: error, no-op, or local-only degraded mode. This is product policy, not provider implementation detail. The service owns it.

### Structured outcomes

Every action returns a structured result that the caller can act on without parsing strings or interpreting error variants. The result must convey: success, partial success, failure with reason, and whether user-visible feedback is warranted. The result type is part of the API contract.

### Undo based on executed operations

Undo tokens are produced by the service based on what actually happened, not what the caller requested. If local succeeded but remote failed, the undo token reflects that. If the action was a no-op (e.g., archiving a thread already not in inbox), no undo token is produced. Undo execution goes through the same service path as the original action.

Undo and failure rollback are the same mechanism. An optimistic update that fails needs rollback. An undo that the user triggers needs rollback. Both reverse local state and potentially remote state. Both need to know what was done. The service must design these together.

### Bulk actions as first-class

Bulk operations (select 50 threads, hit archive) are a primary use case, not an afterthought. The service must define:

- Whether mixed-account selections are handled by the service or pre-partitioned by the caller.
- Partial success semantics — if 48 of 50 succeed, what does the caller see?
- Whether dispatch is per-item, batched per-account, or provider-dependent.
- Undo token shape for multi-item actions (one token for the batch, or one per item?).
- Progress and cancellation semantics for long-running bulk operations.

### Concurrency and ordering

Users can perform actions faster than providers can process them. The service must define behavior for:

- Conflicting sequential actions on the same thread (archive, then immediately un-archive while the first call is in flight).
- Multiple concurrent actions across different threads on the same account.
- Whether actions are serialized per-thread, per-account, or fully concurrent.

### Observability

If silent divergence and inconsistent error handling are problems, consistent observability is a requirement:

- Structured logs for all mutations (action, target, local result, remote result).
- A durable status for pending or failed remote dispatch, if the failure policy involves deferred retry.
- Consistent user-surfaceable result categories (the caller should not need to interpret raw errors to decide what to show the user).

### Testability

The service is a natural seam for testing write operations. It must be testable without real providers and without a running UI. This means the provider dependency must be injectable (trait object, factory, or similar), and the service must not depend on UI framework types.

## Non-Goals

**Read path / sync redesign.** The sync orchestrator handles the read path (server → local DB). This document concerns only the write path (user action → local DB → provider). The two interact at the failure policy boundary (sync reconciles what the write path left behind), but the sync architecture is not in scope.

**UI-layer contracts.** Overlay exclusivity, settings entry centralization, compose routing, calendar pop-out awareness — these are UI concerns that benefit from the same "centralize and enforce" principle but are architecturally separate from the data mutation path.

**Provider internals.** How each provider implements its operations (Gmail API calls, IMAP commands, JMAP methods, Graph requests) is not in scope. The service dispatches through `ProviderOps`; what happens inside each provider crate is that crate's concern.

**Conflict resolution.** When local state and server state diverge due to concurrent modification by another client or server-side rules, the sync pipeline resolves conflicts. The action service makes choices about local staging and pending state that affect what conflicts sync must resolve — those choices are in scope. The sync reconciliation algorithm itself is not.
