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

### Phase 2.1: Thread actions with uniform provider support ✅

**Status:** Complete.

All thread-level actions (trash, spam, move_to_folder, star, mark_read, permanent_delete, pin, mute, snooze) flow through the action service. Snooze is local-only by design (no `ProviderOps::snooze()`). Concurrency semantics addressed in Phase 5.

### Phase 2.2: Label routing ✅

**Status:** Complete. See `phase-2.2-plan.md`.

Label apply/remove flows through `actions::add_label()`/`actions::remove_label()`. The service owns the `label_kind` routing (tag → `apply_category`/`remove_category`, container → `add_tag`/`remove_tag`). `provider_label_write_back` and all `label_kind` branches deleted from the app crate. `apply_category`/`remove_category` consolidation deferred to labels unification. `handle_action_completed` extended with a generic non-toggle/non-removes-from-view feedback path.

### Phase 2.3: Send ✅

**Status:** Complete. See `phase-2.3-plan.md`.

Send flows through `actions::send_email()`: MIME build on `spawn_blocking`, draft persisted as `'pending'` → `'sending'` (state-machine validated), `ProviderOps::send_email()` dispatched, `mark_draft_sent`/`mark_draft_failed` on completion. Compose window stays open during send with dedicated `Message::SendCompleted` and `dispatch_send`. `delete_draft()` exists for future use (no call site yet). Orphaned `'queued'` drafts resurfaced as `'failed'` on boot. Draft auto-save and provider draft sync (`create_draft`/`update_draft`) deferred — separate features.

### Phase 2.4: Folder management ✅

**Status:** Complete. See `phase-2.4-plan.md`.

`create_folder`, `rename_folder`, `delete_folder` in `core::actions::folder`. Provider-first pattern (provider assigns ID/metadata, local DB updated best-effort). `delete_folder` explicitly cleans up `thread_labels` rows (no FK cascade). `build_provider_ctx` helper extracted. `ProviderFolderMutation` re-exported from actions. No UI exists yet — functions are ready. Note: IMAP returns "not supported" for all three folder operations — UI must gate these for IMAP accounts.

### Phase 2.5: Calendar event write-back ✅

**Status:** Complete. See `phase-2.5-plan.md`.

Calendar event create/update/delete in `ratatoskr_calendar::actions`. Lives in the calendar crate (not core) due to circular dependency — calendar depends on core, core can't depend on calendar. Uses typed provider clients (`GmailClient`, `GraphClient`, `JmapClient`, CalDAV config) via `CalendarProvider` enum. Create is local-first (instant feedback, `LocalOnly` on provider failure). Update/delete are provider-first for synced events, local-only for unsynced. All four providers wired (Google, Graph, JMAP, CalDAV). App handler delegates via existing `CalendarMessage::EventSaved`/`EventDeleted` callbacks.

### Phase 2.6: Contact write-back ✅

**Status:** Complete. See `phase-2.6-plan.md`.

Contact save/delete in `core::actions::contacts`. JMAP write-back fully wired (save + delete via `ContactCard/set`). Google/Graph/CardDAV return `LocalOnly` with descriptive errors (scaffolding exists, HTTP not wired). Save is local-first with best-effort write-back. Delete is provider-first for JMAP, degraded local-only for others. Settings UI source bug fixed (synced identity preserved through save path). `server_id` added to `ContactEntry`/`ContactEditorState` for unambiguous provider dispatch. `db_upsert_contact_full` extracted to core.

---

## Phase 3: Failure Policy and Structured Outcomes ✅

**Status:** Complete. See `phase-3-breakdown.md` and `phase-3.1-plan.md`.

**Goal:** Define and implement the partial-failure semantics. This is where the service becomes trustworthy.

Sub-phases: 3.1 (ActionError enum + RemoteFailureKind), 3.2 (retryable: bool on LocalOnly, retry classification per action class), 3.3 (MutationLog with duration + identity tracking), 3.4 (pending_operations queue wired — email actions enqueue on retryable LocalOnly, periodic worker processes queue, crash recovery on boot).

**Exit criteria (all met):** Every action returns a structured outcome. Failure cases are handled consistently per the defined policy. Mutations are observable. The most common user-facing bug (actions reverting on sync) is either prevented or explicitly surfaced.

---

## Phase 4: Undo ✅

**Status:** Complete. See `phase-4-plan.md`.

**Goal:** Undo tokens reflect executed operations. Undo execution goes through the service.

`UndoToken` produced by `produce_undo_tokens` from `ActionOutcome` + thread IDs, grouped by account. `dispatch_undo` pops the stack and calls `execute_compensation` which dispatches inverse actions with `suppress_pending_enqueue = true`. `cancel_pending_ops_for_token` cancels matching pending ops before compensation. `UndoCompleted` message reports results. One token per account for multi-account batches. `PermanentDelete` produces no token.

**Exit criteria (all met):** Undo reverses what actually happened. Undo for partially-completed actions does the right thing. The UI's undo stack uses service-produced tokens.

---

## Phase 5: Bulk Actions and Retry ✅

**Status:** Complete. See `phase-5-plan.md` and `phase-5.2-plan.md`.

**Goal:** Handle batch operations and remote dispatch reliability.

`batch_execute()` groups targets by account, creates one provider per account, dispatches sequentially within each account, parallel across accounts via `futures::future::join_all`. Consecutive-failure short-circuit (threshold: 3) aborts remaining threads with per-thread degraded outcomes. `FlightGuard` RAII ensures one mutation per thread at a time. Toggle actions partitioned by target value in the app layer. Per-type retry policy (folder: 10 retries / [30s,2m,5m,15m,1h], label: 7 / default, flag: 5 / [1m,5m,15m]). Atomic dedup on enqueue with replace semantics. Pending-ops worker reuses providers per account group. Exhausted retries logged and left for sync reconciliation.

**Exit criteria (all met):** Bulk archive of 50 mixed-account threads works correctly with partial failure handling and undo. Failed remote dispatches are retried per policy.

---

## Phase 6: Enforce the Boundary ✅

**Status:** Complete. See `phase-6-plan.md`.

**Goal:** Make the compilation boundary airtight. The app crate physically cannot bypass the service.

All 5 provider crate dependencies removed from `crates/app/Cargo.toml`. Sync dispatch moved to `core::sync_dispatch`. JMAP push moved to `core::jmap_push` with continuous push via iced subscription + debounce. `load_encryption_key` re-exported from core. `create_provider` is `pub(crate)` — not accessible to downstream crates. Provider re-exports in `core/src/lib.rs` are `pub(crate)`. IMAP account verification wrapped in `core::account::verify_imap`. Calendar crate depends on provider crates directly (not through core re-exports).

**Exit criteria (all met):** The app crate has zero provider dependencies. The action service is the only write path for all provider mutations. The boundary is enforced by the compiler.

---

## Remaining Work (Not Phased)

- ~~**Snooze**~~ — Wired to action service (local-only by design). `SnoozeTick` subscription + boot-time check unsnooze due threads.
- **`apply_category`/`remove_category` consolidation** — deferred from Phase 2.2. Categories dropped in labels unification Phase 6; only `add_tag`/`remove_tag` remain.
- **Action test suite** — action functions are testable (pure async, injectable `ActionContext`), but test coverage is minimal.
- **User-facing retry status** — `db_pending_ops_failed_count()` exists but no UI surfaces pending/failed retry state.
- **Native provider batching** — JMAP `Email/set` and Graph `/$batch` support batch requests natively. Current implementation uses sequential per-thread calls with provider reuse. IMAP keyword operations are batched by folder. Native HTTP batching for other actions is a future optimization.
