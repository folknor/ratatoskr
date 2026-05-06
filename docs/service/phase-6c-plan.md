# The Service - Phase 6c Plan: calendar event mutations relocate

Companion to `phase-6a-plan.md` and `phase-6b-plan.md`. Implements the third sub-phase of Phase 6 of `implementation-roadmap.md`.

> **Best-effort first draft.** Authored alongside the 6a/6b plans, before either has landed. Revisions are expected after Phase 6b closes for two reasons: (1) the global write-half lockdown changes how `ActionContext` is constructed inside Service handlers, which directly affects the relocation here; (2) a clean look at the post-6b state may reveal that the email action pipeline has further evolved in ways calendar should follow. Treat this as a structural skeleton, not a final commit-by-commit plan.

## Revision history

**2026-05-06 - initial draft.** Written before 6a/6b land. The cal::actions inventory in this draft reflects the surface as of commit `60037640`: three operations (create, update, delete), no RSVP, no series-vs-occurrence semantics. RSVP and series semantics are out of scope here and tracked as future-Phase-6d work; if either lands before 6c, the plan needs an explicit revision pass.

## Context

After Phase 6a/6b close, three UI write surfaces remain:

- `crates/app/src/db/calendar.rs:47` - `create_calendar_event_sync` via `with_write_conn`
- `crates/app/src/db/calendar.rs:58` - `update_calendar_event_sync` via `with_write_conn`
- `crates/app/src/db/calendar.rs:107` - `delete_calendar_event_sync` via `with_write_conn`

These are reachable through three UI call sites:

- `crates/app/src/handlers/calendar.rs:320` - delete via `cal::actions::delete_calendar_event`
- `crates/app/src/handlers/calendar.rs:561` - update via `cal::actions::update_calendar_event`
- `crates/app/src/handlers/calendar.rs:585` - create via `cal::actions::create_calendar_event`

The action functions already use the action-types pipeline (`ActionContext`, `ActionError`, `ActionOutcome`, `MutationLog`) and run a "provider-first for synced events, local-only fallback" pattern. The relocation is *making the call cross the IPC boundary* - not designing a new action pipeline. The pipeline shape stays the same; what changes is who holds the `WriteDbState` when the local-side write fires.

The Phase 5 plan documented `cal::actions` as a known write-surface escape ("`crates/calendar/src/actions.rs:13` imports `rtsk::actions::{ActionContext, ActionError, ActionOutcome, MutationLog}`. Phase 6's calendar event-mutation relocation will flip those helpers."). Phase 6c is the commit that actually flips them.

## Scope

### In scope

- **`CalendarOperation` enum** mirroring `MailOperation`'s shape. Three variants matching today's three action functions:
  - `Create { account_id, calendar_id, input: CalendarEventInput }`
  - `Update { event_id, input: CalendarEventInput }` (account_id is looked up from the event row, not passed)
  - `Delete { event_id }` (same)
- **`WireCalendarOperation` wire mirror** with serde + a round-trip test (mirroring the `WireMailOperation` discipline established in Phase 2).
- **`CalendarActionPlan` + `cal_action.execute_plan` IPC.** Separate from `action.execute_plan` so the email and calendar pipelines stay orthogonal at the wire level even though they share the journal and completion-notification machinery.
- **`cal::actions::*` signature flip.** `ActionContext` builds with `&WriteDbState` Service-side. Today the action functions take `&ActionContext` whose `db` field is `&ReadDbState`; they currently call `db.conn().lock()` to escape the read constraint. After 6c the field is `&WriteDbState` and the lock dance is gone.
- **UI handlers strip the `cal::actions::*` direct calls.** `handlers/calendar.rs:320,561,585` route through `client.execute_calendar_plan(...)`.
- **Completion-notification routing.** Per-operation `CalendarOperationOutcome` notifications stream back like `OperationOutcome` does for email. Final `CalendarActionCompleted` notification mirrors `ActionCompleted`.
- **Journal coexistence.** The existing `pending_ops` table (Phase 2) gains a `kind` discriminator so calendar and mail operations share storage but the worker dispatches to the right pipeline. Schema migration: one column add, one backfill (existing rows are mail).
- **`docs/architecture.md` update.** Strike `cal::actions` from § Current Exceptions. The "Action service as mutation gate" section already captured the email pipeline shape; add a parallel paragraph on the calendar pipeline.

### Out of scope

- **RSVP semantics.** Not implemented today (`grep "rsvp" crates/calendar/src/actions.rs` returns nothing). When RSVP lands, it gets a new `CalendarOperation::RespondToInvite { event_id, response, comment, send_response }` variant. The wire enum is exhaustively matched, so the addition will surface as compile errors at every dispatch site.
- **Series-vs-occurrence semantics.** Today's `update_calendar_event` operates on the master event; recurrence is a string column on the event row. Modifying "this occurrence only" or "this and all future" requires:
  - A new `EventScope` enum (`Single`, `Occurrence(date)`, `Series(from_date)`).
  - Provider-side support for VCALENDAR overrides / Google instance APIs / Graph instance APIs / JMAP CalendarEvent/set with instance handling.
  - DB schema for occurrence overrides (likely a new `calendar_event_overrides` table keyed on master event id + recurrence-id).
  - UI affordances ("modify this / this-and-future / all").
  Each of those is a project on its own. Phase 6c relocates the three flat operations that exist today and leaves series semantics as future-Phase-6d work.
- **Calendar attachment handling.** Calendar events can carry attachments (CalDAV ATTACH, Microsoft fileAttachment, Google attachments). Today's operations do not write these. Relocation does not introduce them.
- **Cross-store invariant pass extension to calendar.** Phase 6b extends the pass to the attachment cache. Calendar events are pure SQLite; no cross-store reconciliation is needed for them. If event-level invariants ever become non-trivial (e.g., orphaned overrides whose master is gone), that goes in the Phase 6d schema-extension work.
- **`MailOperation` and `WireMailOperation` renaming.** The "Mail" prefix is accurate post-6c (the calendar pipeline lives in a sibling enum, not a shared one). Renaming for symmetry is bikeshed.

## Architecture

### Two pipelines, one journal

The email action pipeline today: `MailActionIntent` -> `resolve_intent` -> `ActionExecutionPlan` -> `ActionWirePlan` -> `action.execute_plan` IPC -> `service::actions::batch_execute` -> `OperationOutcome` notifications -> `handle_action_completed`.

The calendar pipeline post-6c: `CalendarActionIntent` -> `resolve_calendar_intent` -> `CalendarExecutionPlan` -> `CalendarWirePlan` -> `cal_action.execute_plan` IPC -> `service::cal_actions::batch_execute` -> `CalendarOperationOutcome` notifications -> `handle_calendar_action_completed`.

Both pipelines write into the same `pending_ops` table. The discriminator column (added in 6c migration) is read by the worker on boot to dispatch each pending operation to the right pipeline.

```text
            UI                                Service
   MailActionIntent      ─────►  action.execute_plan  ─►  service::actions::batch_execute
                                                                ├─ pending_ops.kind = "mail"
   CalendarActionIntent  ─────►  cal_action.execute_plan ─►  service::cal_actions::batch_execute
                                                                └─ pending_ops.kind = "calendar"
```

### Why a separate IPC instead of a `MailOrCalendarOperation` union?

- The `WireMailOperation` enum is exhaustively matched at seven sites (`completion_behavior`, `dispatch_with_provider`, `op_local`, `enqueue_params`, `op_name`, `to_wire_op`, `wire_to_mail`). Folding calendar into that union would force every email-side site to add calendar arms that mostly fall through. The compiler-enforced exhaustiveness becomes a tax instead of a guard.
- Calendar mutations have semantics email mutations do not (provider-first vs local-first, ETag-based concurrency, the eventual series scope). Forcing them through the email-shaped pipeline means the email pipeline grows special-case branches.
- `pending_ops` storage is shared anyway; the discriminator column gives crash recovery the dispatch information it needs without a wire-type union.

### `ActionContext` flip: `&ReadDbState` → `&WriteDbState`

`action_types::ActionContext` today holds `db: ReadDbState` (or `&ReadDbState` depending on how it's threaded). The `cal::actions` functions reach inside via `db.conn().lock()` to do raw rusqlite writes - a write-surface escape Phase 5 explicitly documented.

After 6c (Service-side):

```rust
pub struct ActionContext<'a> {
    pub db: &'a WriteDbState,
    pub encryption_key: [u8; 32],
    // ... other fields unchanged
}
```

The `db.conn().lock()` escape goes away because `WriteDbState::with_conn` is the legitimate write API. `cal::actions::create_calendar_event` and friends use `ctx.db.with_conn(|conn| ...)` instead of the lock dance.

UI-side `cal::actions::*` direct calls disappear - the `ActionContext::new` constructor that took `&ReadDbState` is removed (or made `pub(crate)` to the action-types crate so only Service handlers can construct one). Compile-time enforcement that calendar mutations no longer run UI-side.

### Notification routing

Calendar gets two notifications:

- `CalendarOperationOutcome { plan_id, op_index, outcome: ActionOutcome }` - per-operation result, streamed as the worker processes the plan.
- `CalendarActionCompleted { plan_id, results: Vec<CalendarOperationOutcome> }` - terminal frame, one per plan.

Both are `MustDeliver` class (mirroring `OperationOutcome` / `ActionCompleted`). The UI's `ServiceClient` reader maintains a `pending_calendar_action_plans: HashMap<PlanId, ...>` mirror of the mail-side `pending_action_plans` map (Phase 2 task 3); fast-completion-races-late-subscriber is handled by the same latch pattern Phase 5 used for `CalendarRunCompleted`.

The `CalendarChanged` notification (Phase 5) keeps its purpose: UI debounced-reload on calendar-table mutations, regardless of whether the mutation came from a sync run or a user action. The action handler emits `CalendarChanged` after a successful local write so the UI sees it through the existing dispatch path.

### Provider-first vs local-first

Today's `cal::actions::create_calendar_event` writes locally first, then dispatches to the provider; provider failure leaves the event with `remote_event_id = NULL` (`ActionOutcome::LocalOnly`). Today's `update_calendar_event` and `delete_calendar_event` for synced events go provider-first - the local write only fires after the provider succeeds.

This split survives the relocation. The action functions encode the policy; the Service handler is just the IPC envelope. Tests in 6c lock the policy in (one create test asserting LocalOnly on provider failure, one update test asserting Failed on provider failure).

### Plan resolution and intent layer

Email actions go through `MailActionIntent::resolve_intent`. Calendar mutations today come from the editor session (compose modal, recurrence editor) - they are already concrete `CalendarEventInput` payloads, not abstract intents. Phase 6c introduces a thin `CalendarActionIntent` enum that mostly maps 1:1 to operations today:

```rust
pub enum CalendarActionIntent {
    CreateEvent { account_id: String, calendar_id: String, input: CalendarEventInput },
    UpdateEvent { event_id: String, input: CalendarEventInput },
    DeleteEvent { event_id: String },
}
```

`resolve_calendar_intent` collapses each intent into a `CalendarExecutionPlan { operations: Vec<CalendarOperation> }`. Plans are usually one operation today; the vec lets the future "delete a series of events at once" affordance fit without a new shape.

### Worker integration

The Service-side action worker (Phase 2) processes `pending_ops` rows in order. After 6c migration:

- Worker reads the `kind` column from each row.
- `kind = "mail"` -> existing dispatch via `service::actions::batch_execute`.
- `kind = "calendar"` -> new dispatch via `service::cal_actions::batch_execute`.

The worker itself is provider-agnostic at this level - the dispatchers handle the provider routing.

## Detailed task list

In recommended commit order. Each item is one focused commit unless noted.

**0. Inventory + revision pass.** Re-verify the cal::actions surface against the codebase as 6b lands. RSVP, series-modify, or new operation types added between 6a and 6c would expand scope. Document any deltas before starting; if the surface has grown materially, this plan needs revision before tasks 1-9 begin.

**1. `pending_ops` discriminator migration.** Add `kind TEXT NOT NULL DEFAULT 'mail'` column. Backfill existing rows (single UPDATE; the column default does it for new rows). Worker reads the column on boot.

**2. `service-api` wire types.** New `service-api/src/cal_action.rs` with `CalendarOperation`, `WireCalendarOperation`, `CalendarActionPlan`, `CalendarOperationOutcome`, `CalendarActionCompleted`, `CalendarActionAck`. Round-trip tests per type.

**3. `cal_action.execute_plan` request handler.** Service-side `service/src/handlers/cal_action.rs`. Validates the plan (account-id consistency, no duplicate op-ids), enqueues into `pending_ops` with `kind='calendar'`, returns ack. Worker dispatches to the new pipeline asynchronously; the handler does not block on completion.

**4. `service::cal_actions::batch_execute`.** Service-side dispatcher: per-operation, build an `ActionContext` from `BootSharedState` (now with `&WriteDbState`), call the relocated `cal::actions::*` function, capture the `ActionOutcome`, emit `CalendarOperationOutcome`. After all operations complete, emit `CalendarActionCompleted`.

**5. `cal::actions::*` signature flip.** Change `ActionContext::db` from `&ReadDbState` to `&WriteDbState`. Replace `db.conn().lock()` escapes with `ctx.db.with_conn(...)`. Remove the public `ActionContext::new` constructor that took `&ReadDbState` (or downgrade to `pub(crate)` to action-types). The compile errors that cascade from this change are the lockdown's enforcement teeth.

**6. UI-side intent + plan helpers.** New `crates/app/src/cal_action_resolve.rs` with `CalendarActionIntent::resolve_intent`. New `crates/app/src/cal_action_wire.rs` with `to_wire_calendar_op` (mirrors `to_wire_op`).

**7. UI handlers strip direct calls.** `handlers/calendar.rs:320,561,585` route through `client.execute_calendar_plan(...)`. Per-operation outcomes flow back through the existing `Message::ServiceNotification` arm, dispatched by notification kind into a new `Message::CalendarActionCompleted` arm.

**8. `pending_calendar_action_plans` map mirror.** UI's `ServiceClient` gains a `HashMap<PlanId, ...>` mirroring the mail-side equivalent. Subscribe-or-consume pattern handles fast-completion races. Mirror the Phase 5 `pending_calendars` test cohort here once the broader Phase 6 test cohort lands.

**9. `docs/architecture.md` update.** Strike `cal::actions` from § Current Exceptions. Add a parallel "Calendar action pipeline" paragraph alongside § "Action service as mutation gate". Mark the implementation-roadmap.md Phase 6c entry "LANDED."

## File-by-file changes

**New files:**
- `crates/service-api/src/cal_action.rs` - calendar wire types.
- `crates/service/src/handlers/cal_action.rs` - request handler.
- `crates/service/src/cal_actions/mod.rs` - dispatcher.
- `crates/app/src/cal_action_resolve.rs` - intent + plan helpers.
- `crates/app/src/cal_action_wire.rs` - wire conversion.

**Modified files:**
- `crates/db/src/db/migrations.rs` - new migration adding `pending_ops.kind` column with backfill.
- `crates/service-api/src/lib.rs` - module declaration.
- `crates/service-api/src/request.rs` - new `RequestParams::CalAction(...)` variant + 5 s timeout.
- `crates/service-api/src/notification.rs` - new `Notification::CalendarOperationOutcome` + `CalendarActionCompleted`.
- `crates/service/src/dispatch.rs` - new request arm + worker dispatch by `kind`.
- `crates/calendar/src/actions.rs` - signature flip, lock-dance removal.
- `crates/action-types/src/lib.rs` - `ActionContext::db` field type change; constructor visibility flip.
- `crates/app/src/service_client.rs` - new `execute_calendar_plan` async wrapper, `pending_calendar_action_plans` map, completion-notification handling.
- `crates/app/src/handlers/calendar.rs` - replace `cal::actions::*` calls with `client.execute_calendar_plan(...)`.
- `crates/app/src/db/calendar.rs` - delete `create_calendar_event` / `update_calendar_event` / `delete_calendar_event` helpers (and the `with_write_conn` calls inside them).
- `crates/app/src/update.rs` - new `Message::CalendarActionCompleted` arm.
- `docs/architecture.md` - per § "Architecture-doc update" above.
- `docs/service/implementation-roadmap.md` - mark Phase 6c "LANDED".

## Code-comment requirements

1. **`crates/service-api/src/cal_action.rs` module-level doc-comment** must contain:
   - "Separate from `WireMailOperation` because the email pipeline's seven exhaustively-matched dispatch sites would gain mostly-fall-through calendar arms otherwise. Calendar and mail share the `pending_ops` journal via a `kind` discriminator column; the wire types stay orthogonal."

2. **`crates/calendar/src/actions.rs` module-level doc-comment** must add:
   - "Phase 6c flipped `ActionContext::db` from `&ReadDbState` to `&WriteDbState`. The lock-dance pattern (`db.conn().lock()`) that used to escape the read constraint is gone. Construction of `ActionContext` is `pub(crate)` to `action-types` post-6c; UI source files cannot mint one, which is the compile-time gate that makes calendar mutations Service-only."

3. **`crates/db/src/db/migrations.rs` migration entry** must contain:
   - "`pending_ops.kind` column added in migration N. `'mail'` for existing rows + new mail operations; `'calendar'` for new calendar operations. Worker dispatches by this column on boot - a row with an unknown kind is a corrupt journal and the worker logs + skips rather than crashing."

4. **`crates/service/src/cal_actions/mod.rs::batch_execute`** must contain:
   - "`CalendarOperationOutcome` is `MustDeliver` class - the UI's `pending_calendar_action_plans` map keys on `plan_id` and unblocks the awaiting caller when the matching `CalendarActionCompleted` arrives. Same latch pattern Phase 5 used for `CalendarRunCompleted`."

5. **`docs/architecture.md` § "Action service as mutation gate"** new paragraph (parallel to the existing email paragraph):
   - "Calendar event mutations flow through a sibling pipeline introduced in Phase 6c: `CalendarActionIntent → resolve_calendar_intent → CalendarExecutionPlan → CalendarWirePlan → cal_action.execute_plan IPC → service::cal_actions::batch_execute → CalendarOperationOutcome notifications → handle_calendar_action_completed`. The two pipelines share the `pending_ops` journal via a `kind` discriminator but use orthogonal wire types so each retains exhaustive-match discipline. The `cal::actions::*` write-surface escape that Phase 5 documented as a Current Exception is gone after 6c."

## Test plan

### Unit tests

- Wire-type round-trips for every variant of `WireCalendarOperation`.
- `CalendarOperation` exhaustive-match shape regression test (mirroring `mail_side_mirror_is_exhaustive`).
- `pending_ops` migration backfill: pre-migration row count, post-migration row count, all rows have `kind='mail'`.

### Integration tests (in-process)

- `calendar_create_event_round_trips_through_ipc`: UI ships a create plan; Service writes the local row; UI observes the row via the read path.
- `calendar_update_event_provider_failure_returns_failed`: stub provider that fails; assert `ActionOutcome::Failed` (not LocalOnly) on update of a synced event.
- `calendar_create_event_provider_failure_returns_local_only`: stub provider that fails; assert `ActionOutcome::LocalOnly` on create.
- `calendar_action_completion_late_subscriber_catches_fast_completion`: mirror of the mail-side latch test (Phase 2 task 3).
- `pending_ops_dispatches_by_kind_after_respawn`: enqueue one mail op + one calendar op; respawn Service mid-flight; assert each gets dispatched to the right pipeline on recovery.

### Real-subprocess smoke tests

- `service_subprocess_calendar_create_event`: real IPC, real CalDAV stub, observe the local row + provider call.
- `service_subprocess_pending_ops_kind_discriminator_survives_respawn`: kill Service mid-plan; restart; assert the unfinished operation completes via the right pipeline.

### Manual matrix updates

- Calendar event create / update / delete via the UI editor against each provider (Google, Graph, JMAP, CalDAV).
- Verify `ActionOutcome::LocalOnly` surface in the UI - the user sees the event locally with a "not synced" indicator if provider create fails.

## Open questions

- **`ActionContext` constructor visibility.** `pub` (unchanged) lets test fixtures construct contexts; `pub(crate)` to `action-types` blocks UI sources. The compile-time gate is what we want, but blanket `pub(crate)` makes test-only construction awkward. Likely resolution: `pub fn new(...)` becomes `pub(crate) fn new(...)`, plus a `#[cfg(any(test, feature = "test-helpers"))]` `pub fn new_for_testing(...)` adjacent. Decide during task 5.
- **Calendar plan-id allocation.** Email uses a UI-side `PlanId` minted at intent-resolution time. Calendar can reuse the same allocator (one `PlanId` space) or have its own. Plan picks "shared allocator" - one less concept to track, and cross-pipeline plan-id collision is impossible because mail and calendar plans live in distinct UI-side maps.
- **Notification class for `CalendarOperationOutcome`.** Plan picks `MustDeliver` because the UI-side latch pattern requires it. If measurement shows per-operation outcomes are too noisy on bulk operations (e.g., a future "delete all events in series" affordance), revisit.

## Verification (end-to-end)

- `git grep with_write_conn crates/app/src/` returns nothing except (possibly) test-only fixtures.
- `crates/calendar/src/actions.rs` no longer calls `db.conn().lock()`; all writes go through `ctx.db.with_conn(...)`.
- `crates/app/src/db/calendar.rs` no longer exists, or contains only read helpers.
- A calendar event create / update / delete flows through the IPC end-to-end against each of the four providers.
- A Service crash mid-plan recovers with the right operation kind dispatching to the right pipeline.
- `docs/architecture.md` no longer lists `cal::actions` as a Current Exception.

## Promotion criteria

- All items in `In scope` landed.
- All items in `Out of scope` (RSVP, series-vs-occurrence, calendar attachments) explicitly tracked - either in the next phase plan or in a roadmap entry.
- The "Phase 6 status" section of `implementation-roadmap.md` shows 6a, 6b, and 6c all "LANDED".
- `docs/architecture.md` reflects the post-Phase-6c state without a Calendar Current Exception.
- `phase-6c-plan.md` is then retirement-ready: every deferral has an explicit roadmap entry; every code-comment requirement is present in the relevant file.

## Notes for the post-6b revision pass

When 6b lands, walk through this plan and check:

- **Has the `ActionContext` shape changed?** Phase 6b's lockdown work might rework how `WriteDbState` constructors are exposed. If the constructor pattern Phase 6c assumes (`WriteDbState::from_arc` inside Service handlers) has been replaced, the task-5 signature flip rewrites accordingly.
- **Has the cross-store invariant pass extension informed any calendar-table invariants?** Phase 6b extends the pass to attachment cache; if the architectural lessons there suggest a calendar-event invariant pass, add it to scope.
- **Has Phase 6b changed the notification class catalog?** The plan picks `MustDeliver` for `CalendarOperationOutcome`; if 6b introduced a new class better suited to per-operation calendar outcomes, swap.
- **Has the cal::actions surface grown?** RSVP or series-modify landing between 6a and 6c expands the in-scope wire types.
