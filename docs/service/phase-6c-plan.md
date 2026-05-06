# The Service - Phase 6c Plan: calendar event mutations relocate

Companion to `phase-6a-plan.md` and `phase-6b-plan.md`. Implements the third sub-phase of Phase 6 of `implementation-roadmap.md`.

> **Best-effort first draft.** Authored alongside the 6a/6b plans, before either has landed. Revisions are expected after Phase 6b closes for two reasons: (1) the global write-half lockdown changes how `ActionContext` is constructed inside Service handlers, which directly affects the relocation here; (2) a clean look at the post-6b state may reveal that the email action pipeline has further evolved in ways calendar should follow. Treat this as a structural skeleton, not a final commit-by-commit plan.

## Revision history

**2026-05-06 - initial draft.** Written before 6a/6b land. The cal::actions inventory in this draft reflects the surface as of commit `60037640`: three operations (create, update, delete), no RSVP, no series-vs-occurrence semantics. RSVP and series semantics are out of scope here and tracked as future-Phase-6d work; if either lands before 6c, the plan needs an explicit revision pass.

**2026-05-06 - post-6a/6b-arch-review revision.** Two arch-review items landed against 6c's framing while 6a/6b were under review: (1) inventory should reference symbols, not file:line numbers, so a commit reorder does not invalidate the plan (raised against 6a, applies equally to 6c); (2) the unknown-`kind` worker handling should emit a `JournalCorrupt` notification rather than silent skip. Plan revised accordingly.

**2026-05-06 - post-6c-arch-review revision (large).** Two reviewers caught a critical naming error and several cascading consequences. Major revisions:
- **Journal table corrected.** The plan referenced `pending_ops` everywhere; the actual durable journal is `action_jobs` + `action_job_ops` (`crates/db/src/db/schema/12_actions.sql`). `action_jobs.kind` already has `CHECK (kind IN ('mail_plan', 'send', 'mark_chat_read'))`. Phase 6c **widens the CHECK** to add `'calendar_plan'`; it does not add a column, and there is no `'mail'` default. (`pending_operations` is the per-op transient retry queue, separate from the journal.)
- **`ActionContext::db` flip is workspace-wide, not calendar-scoped.** `ActionContext` is shared by every email action; flipping the field cascades through `service::actions::{archive,label,move,send,...}`. Plan now lands a sibling `CalendarActionContext { db: WriteDbState, encryption_key: [u8; 32] }` instead of mutating the shared one - calendar's needs are narrower (no `body_store`, `inline_images`, `search`, `in_flight`) and a separate type avoids the cross-cutting refactor.
- **`Create` was not replay-idempotent.** Original wire shape carried no stable event id; a Service crash after local insert but before terminal finalize would replay and create duplicates. Wire `CalendarOperation::CreateEvent` now carries a `proposed_event_id: String` minted UI-side; Service uses INSERT-or-IGNORE on replay.
- **Wire payloads can't carry `ActionOutcome`.** `ActionOutcome` is not serde and lives in `action-types`. Wire result is now `CalendarOperationResult` (a serde-derived sibling of `OperationResult`) with explicit `Success | LocalOnly { reason } | Failed { error }` variants. Service-side dispatcher converts `ActionOutcome -> CalendarOperationResult` at the IPC boundary, same shape mail uses.
- **`CalendarEventInput` moves to `service-api`.** Today it lives in `cal::actions` and lacks serde. Post-6c it sits in `service-api/src/cal_action.rs` with serde derived; the cal crate re-imports it. This is what lets `app/Cargo.toml` drop the `cal` dependency entirely.
- **Cargo dep graph closure.** After 6c, `crates/app/Cargo.toml` no longer lists `cal`; the only `cal::*` references in app today are the three action functions and `CalendarEventInput`, both of which become IPC + wire-types post-6c. Closing this is what makes 6b's transitive `cargo metadata` lockdown check pass.
- **Cross-kind scheduling pinned down.** Worker leases use `lease_next_ready_op` and `lease_next_ready_quiet_job(kind, ...)`. Plan picks a single FIFO queue across all kinds (oldest ready op of any kind dispatches first) - preserves submit order across mail+calendar, no per-kind starvation.
- **`AckUnknown` reconciliation.** Calendar plans share `action_jobs` so the existing `action.job_status` IPC covers them transparently. Plan documents this rather than introducing a sibling `cal_action.job_status`.
- **`JournalCorrupt` row finalization.** Original "skip + once-per-lifetime" left the row in the journal indefinitely. Revised: emit notification, then transition the row to `status='failed'` with a `JournalCorrupt` reason blob. Lease index drops it; user-visible record persists in `action_jobs` history for support flows.
- **Single `CalendarOperation` enum, no wire mirror.** Mail's `WireMailOperation` exists because `MailOperation` carries `FolderId`/`TagId` newtypes that don't serde directly. Calendar carries `String`s and serializable types - no need for a mirror layer. Plan collapses to one `#[derive(Serialize, Deserialize)] CalendarOperation` enum.
- **`CalendarActionIntent` deferred.** The intent layer exists in mail because user intents (`Archive thread X`) resolve to N operations per thread. Calendar today is 1:1; plan deletes the intent shim and reintroduces it when RSVP/series-vs-occurrence land in Phase 6d.
- **Provider-first vs local-first asymmetry documented.** `LocalOnly` is reachable only for `CreateEvent` (today's create writes locally first, provider second). Update / Delete are provider-first for synced events; their wire result is `Success | Failed`, never `LocalOnly`. Per-variant comment in the wire-types module.
- **Inventory references by symbol.** `Db::create_calendar_event`, `Db::update_calendar_event`, `Db::delete_calendar_event`, and `cal::actions::{create_calendar_event, update_calendar_event, delete_calendar_event}` instead of file:line.
- **Architecture-doc dependency made explicit.** 6c's task to "Strike `cal::actions` from § Current Exceptions" depends on 6a having added it during the architecture rewrite. Documented as an entry criterion.

## Context

After Phase 6a/6b close, the UI write surface that bypasses the Service is just calendar events. By symbol:

- `Db::create_calendar_event` - calls `create_calendar_event_sync` via `with_write_conn`
- `Db::update_calendar_event` - same shape
- `Db::delete_calendar_event` - same shape

These wrap three action functions in the `cal` crate:

- `cal::actions::create_calendar_event` (called from `handle_calendar_create`)
- `cal::actions::update_calendar_event` (called from `handle_calendar_update`)
- `cal::actions::delete_calendar_event` (called from `handle_calendar_delete`)

Each handler in `crates/app/src/handlers/calendar.rs` builds an `ActionContext` and dispatches; the context construction at `app.rs::from_boot_ready` is the Pattern B `Db::write_db_state()` site that 6a's lockdown explicitly allow-listed for 6c removal.

The action functions already use the action-types pipeline (`ActionContext`, `ActionError`, `ActionOutcome`, `MutationLog`) and run a "provider-first for synced events, local-only fallback" pattern. The relocation is *making the call cross the IPC boundary* - not designing a new action pipeline. The pipeline shape stays the same; what changes is who holds the `WriteDbState` when the local-side write fires, and the `app.rs` ActionContext construction goes away.

The Phase 5 plan documented `cal::actions` as a known write-surface escape ("`crates/calendar/src/actions.rs` imports `rtsk::actions::{ActionContext, ActionError, ActionOutcome, MutationLog}`. Phase 6's calendar event-mutation relocation will flip those helpers."). Phase 6c is the commit that actually flips them and removes the last allow-listed Pattern B site.

## Scope

### Entry criteria

- **Phase 5 landed** (calendar/GAL relocation, IMAP cancellation depth, JMAP calendar arm).
- **Phase 6a landed**, with `docs/architecture.md` § Current Exceptions explicitly carrying `cal::actions` as a known UI-side write-surface escape. 6c removes that exception entry.
- **Phase 6b landed**, with the direct-dep + constructor-visibility lockdown checks in place. The transitive `cargo metadata` check is wired but enforcement is gated until 6c lands - 6c's lockdown task closes the `app -> cal -> service-state` path that prevents transitive enforcement before 6c.

### In scope

- **`CalendarOperation` serde-derived enum.** Single enum, no wire mirror. Mail's two-enum split (`MailOperation` / `WireMailOperation`) exists because `FolderId` and `TagId` are typed newtypes that need a mirror layer; calendar carries `String`s and serde-able structs, so a single `#[derive(Serialize, Deserialize)] CalendarOperation` is the wire type. Three variants matching today's three action functions:
  - `CreateEvent { proposed_event_id: String, account_id: String, calendar_id: String, input: CalendarEventInput }`. The UI mints `proposed_event_id` (UUID) before sending; Service uses INSERT-or-IGNORE on replay so a Service crash mid-create cannot duplicate the event row.
  - `UpdateEvent { event_id: String, input: CalendarEventInput }`. account_id is looked up from the event row, not passed.
  - `DeleteEvent { event_id: String }`.
- **`CalendarEventInput` moves to `service-api`.** Today it lives in `cal::actions` and lacks serde. The relocation derives serde, declares the canonical wire shape, and lets the cal crate re-import it. This is what makes the cargo-graph closure work.
- **`CalendarOperationResult` wire result.** Serde-derived sibling of mail's `OperationResult`:
  - `Success`
  - `LocalOnly { reason: String }` - reachable only for `CreateEvent` (per-variant comment in the wire-types module documents this).
  - `Failed { error: String }` - the only result `UpdateEvent` and `DeleteEvent` can return on provider failure.
  Service-side dispatcher converts `ActionOutcome -> CalendarOperationResult` at the IPC boundary, matching the shape mail uses with `OperationResult`. `ActionOutcome` (in `action-types`) is not serde and never crosses the wire.
- **`CalendarActionPlan` + `cal_action.execute_plan` IPC.** Separate from `action.execute_plan` so email and calendar pipelines stay orthogonal at the wire level even though they share the durable journal.
- **`CalendarActionContext` (sibling, not flip).** New type in `action-types` with the narrow shape calendar needs:
  ```rust
  pub struct CalendarActionContext {
      pub db: WriteDbState,
      pub encryption_key: [u8; 32],
  }
  ```
  `cal::actions::*` functions take `&CalendarActionContext` instead of the email-shaped `&ActionContext`. The `db.conn().lock()` escape in `cal::actions` gets replaced with `ctx.db.with_conn(...)`. The shared `ActionContext` is unchanged - email actions are unaffected. The original draft's "flip `ActionContext::db` from `&ReadDbState` to `&WriteDbState`" was workspace-wide; this revision keeps the change surface narrow.
- **UI handlers strip the `cal::actions::*` direct calls.** `handle_calendar_create`, `handle_calendar_update`, `handle_calendar_delete` in `crates/app/src/handlers/calendar.rs` route through `client.execute_calendar_plan(...)`.
- **Completion-notification routing.** Per-operation `CalendarOperationOutcome { plan_id, op_index, result: CalendarOperationResult }` stream back from the worker. Final `CalendarActionCompleted { plan_id, results: Vec<CalendarOperationOutcome> }` mirrors `ActionCompleted`. Both `MustDeliver` class.
- **Journal: widen `action_jobs.kind` CHECK constraint.** Today: `CHECK (kind IN ('mail_plan', 'send', 'mark_chat_read'))`. Migration adds `'calendar_plan'`. SQLite cannot ALTER CHECK in place, so the migration is rebuild-table style (CREATE new table -> INSERT SELECT -> DROP old -> rename). No backfill, no default mismatch - existing rows already carry their correct `kind`.
- **Cross-kind scheduling: single FIFO queue.** Worker leases the oldest ready op of any kind first. Preserves submit order across mail+calendar; no per-kind starvation hazard. Calendar plans use `action_job_ops` for per-op streaming the same way `mail_plan` does today.
- **`AckUnknown` reconciliation via existing `action.job_status`.** Calendar plans live in the same `action_jobs` table, so the Phase 2 `action.job_status` IPC covers them transparently. UI's respawn reconciliation reuses the existing path; no `cal_action.job_status` needed.
- **`JournalCorrupt` row finalization.** Worker encountering an unknown `kind` (e.g., schema drift, future-build rollback): (1) emit `Notification::JournalCorrupt { row_id, kind, account_id }` (`MustDeliver` class), (2) transition the row to `status='failed'` with a structured `JournalCorrupt` reason blob, (3) skip. The lease index drops it; the user-visible record persists in `action_jobs` history for support flows. Avoids the indefinite-leak hazard "skip + once-per-lifetime" would create.
- **Cargo-graph closure.** `crates/app/Cargo.toml` drops `cal = { path = "../calendar" }`. The only `cal::*` references in app today are the three action functions (now IPCs) and `CalendarEventInput` (now in `service-api`). Dropping the dep is what makes 6b's transitive `cargo metadata` lockdown check pass; enforcement enables in 6c.
- **`docs/architecture.md` update.** Strike `cal::actions` from § Current Exceptions (added by 6a). Add a parallel "Calendar action pipeline" paragraph alongside § "Action service as mutation gate."

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

### Two pipelines, one journal table

Email pipeline today: `MailActionIntent` -> `resolve_intent` -> `ActionExecutionPlan` -> `ActionWirePlan` -> `action.execute_plan` IPC -> `service::actions::batch_execute` -> `OperationOutcome` notifications -> `handle_action_completed`. Persists to `action_jobs` (kind = `mail_plan` / `send` / `mark_chat_read`) + `action_job_ops` for multi-op plans.

Calendar pipeline post-6c: `CalendarOperation` -> `cal_action.execute_plan` IPC -> `service::cal_actions::batch_execute` -> `CalendarOperationOutcome` notifications -> `handle_calendar_action_completed`. Persists to the same `action_jobs` table with `kind = 'calendar_plan'`. No intent-layer shim - calendar today maps user gestures 1:1 to operations, so the intent layer is no-op overhead until RSVP/series semantics arrive in Phase 6d.

```text
            UI                                Service
   MailActionIntent  -> resolve_intent  -> action.execute_plan  ->  service::actions::batch_execute
                                                                       (action_jobs.kind = mail_plan)
   CalendarOperation                    -> cal_action.execute_plan  -> service::cal_actions::batch_execute
                                                                       (action_jobs.kind = calendar_plan)
```

The migration widens the existing CHECK on `action_jobs.kind` from `('mail_plan', 'send', 'mark_chat_read')` to `('mail_plan', 'send', 'mark_chat_read', 'calendar_plan')`. SQLite cannot ALTER CHECK in place, so the migration is rebuild-table style (CREATE new table -> INSERT SELECT existing rows -> DROP old -> rename). No backfill, no default mismatch.

### Why a separate IPC instead of a `MailOrCalendarOperation` union?

- The `WireMailOperation` enum is exhaustively matched at seven sites (`completion_behavior`, `dispatch_with_provider`, `op_local`, `enqueue_params`, `op_name`, `to_wire_op`, `wire_to_mail`). Folding calendar into that union forces every email-side site to add calendar arms that mostly fall through. The compiler-enforced exhaustiveness becomes a tax instead of a guard.
- Calendar mutations have semantics email mutations do not (provider-first vs local-first, ETag-based concurrency, the eventual series scope). Forcing them through the email-shaped pipeline means the email pipeline grows special-case branches.
- `action_jobs` storage is shared anyway; the existing `kind` discriminator gives crash recovery the dispatch information it needs without a wire-type union.

### `CalendarActionContext` (sibling, not flip)

`action_types::ActionContext` today holds `db: ReadDbState` and is the shared context for every email action (`service::actions::{archive,label,move,send,mark_chat_read,...}`). Flipping `db` to `&WriteDbState` would cascade through every email-side dispatch site - a workspace-wide refactor framed as calendar-scoped work. The arch review flagged this; the revision adds a sibling type instead:

```rust
pub struct CalendarActionContext {
    pub db: WriteDbState,
    pub encryption_key: [u8; 32],
}
```

`cal::actions::create_calendar_event` and friends take `&CalendarActionContext`. The `db.conn().lock()` escape pattern in today's calendar action functions gets replaced with `ctx.db.with_conn(...)`. The shared `ActionContext` is unchanged - email actions are unaffected. The narrow context also matches what calendar actually needs - no `body_store`, `inline_images`, `search`, `in_flight` fields populated with placeholders.

UI-side `cal::actions::*` direct calls disappear - `CalendarActionContext::new` is `pub(crate)` to `action-types` so only Service handlers can construct one. Compile-time enforcement that calendar mutations no longer run UI-side.

### Wire result mapping (`ActionOutcome` does not cross the wire)

`ActionOutcome` (`action-types`) is not serde and intentionally so - it is the in-process domain type that mail's wire layer maps to `OperationResult` at the IPC boundary. Calendar uses the same shape: `CalendarOperationResult` is a serde-derived sibling of `OperationResult` with three variants:

```rust
#[derive(Serialize, Deserialize)]
pub enum CalendarOperationResult {
    Success,
    LocalOnly { reason: String },
    Failed { error: String },
}
```

Service-side dispatcher converts `ActionOutcome -> CalendarOperationResult` at the IPC boundary. `LocalOnly` is reachable only for `CreateEvent` (today's create writes locally first, provider second; the policy survives the relocation); `UpdateEvent` and `DeleteEvent` are provider-first for synced events and return `Success | Failed`. Per-variant comments in the wire-types module document the asymmetry so future readers do not handle reachable variants for the wrong op kind.

### Idempotent create on replay

Today's `create_calendar_event` mints a fresh UUID inside the action function and inserts it. If a Service crash happens between the local insert and the terminal journal finalize, replay creates a duplicate row and a duplicate provider-side event.

Phase 6c moves the UUID mint UI-side: `CreateEvent { proposed_event_id: String, ... }`. The Service-side handler uses INSERT-or-IGNORE on `action_jobs` and on the `calendar_events` row - replay is a no-op for the local insert and the provider call uses the same `proposed_event_id` so the provider-side dedup (where supported) catches it too. CalDAV adds an idempotency token to the create payload; Google/Graph use `request-id`-style headers to dedupe; JMAP CalendarEvent/set with the same client-supplied id is idempotent by spec.

### Notification routing

Calendar gets two notifications:

- `CalendarOperationOutcome { plan_id, op_index, result: CalendarOperationResult }` - per-operation result, streamed as the worker processes the plan.
- `CalendarActionCompleted { plan_id, results: Vec<CalendarOperationOutcome> }` - terminal frame, one per plan.

Both `MustDeliver` class. The UI's `ServiceClient` reader maintains a `pending_calendar_action_plans: HashMap<PlanId, ...>` mirror of the mail-side `pending_action_plans` map (Phase 2 task 3); fast-completion-races-late-subscriber is handled by the same latch pattern Phase 5 used for `CalendarRunCompleted`. (A unified `pending_op_plans` map keyed on `PlanId` with a kind tag is a tempting consolidation, but mail-side already uses its own type and unifying would refactor the mail surface; defer to a future cleanup.)

The `CalendarChanged` notification (Phase 5) keeps its purpose: UI debounced-reload on calendar-table mutations, regardless of whether the mutation came from a sync run or a user action. The action handler emits `CalendarChanged` after a successful local write so the UI sees it through the existing dispatch path.

### `AckUnknown` reconciliation

Mail uses `action.job_status` after Service respawn to decide whether to keep optimistic state or roll back. Because calendar plans live in the same `action_jobs` table, the Phase 2 `action.job_status` IPC covers them transparently - no `cal_action.job_status` sibling needed. The UI's respawn reconciliation reuses the existing path; the only change is that `JobStatus` now also reports `kind = 'calendar_plan'` rows, which the UI handles via the existing match on `JobStatus.kind`.

UI optimistic-state policy for calendar: today calendar mutations round-trip Service synchronously and the UI does not apply optimistic state; the editor parks until `CalendarActionCompleted` arrives. Post-6c policy stays the same - calendar is non-optimistic. On respawn the UI awaits the next `CalendarChanged` and re-reads. Documented in the handler-level comment.

### Cross-kind scheduling

Worker leases use `lease_next_ready_op` (for `mail_plan` op-streaming) and `lease_next_ready_quiet_job(kind, ...)` (for `send` / `mark_chat_read`). Phase 6c calendar plans use `action_job_ops` for per-op streaming the same way `mail_plan` does. Cross-kind scheduling is **single FIFO across all kinds** - the worker leases the oldest ready op of any kind first. Preserves submit order across mail+calendar; no per-kind starvation hazard. A bulk mail-plan submission cannot indefinitely starve a calendar delete and vice versa. (If measurement reveals a real interleaving issue, kind-fairness becomes a follow-up tightening.)

### Unknown-`kind` row handling

A row with an unknown `kind` (corrupt journal, future-build rollback, schema drift) gets handled in three steps:

1. Worker emits `Notification::JournalCorrupt { row_id, kind, account_id }` (`MustDeliver` class). UI surfaces a status-bar error.
2. Worker transitions the row to `status='failed'` with a structured `JournalCorrupt` reason blob in the result column. The lease index drops it.
3. Worker skips the row and continues.

The user-visible record persists in `action_jobs` history for support flows. The original draft proposed "skip + once-per-lifetime" which would have left the row in the lease index indefinitely; the revised path prevents that leak.

### Cargo-graph closure

`crates/calendar/Cargo.toml` declares `service-state = { path = "../service-state" }` (Phase 5 made `cal::sync` Service-side). `crates/app/Cargo.toml` declares `cal = { path = "../calendar" }`. The transitive path `app -> cal -> service-state` exists today; 6b's transitive lockdown check fails until 6c removes it.

Closure path:

- `CalendarEventInput` moves from `cal::actions` to `service-api::cal_action`. The cal crate re-imports it; UI imports from service-api.
- The three `cal::actions::*` functions stay in the cal crate but become callable only from `service::cal_actions::batch_execute`; UI no longer imports them.
- `crates/app/Cargo.toml` drops `cal = { path = "../calendar" }`. The compile errors that cascade from this drop are the enforcement mechanism: any UI source file still reaching for cal types fails to build.
- 6b's transitive `cargo metadata` lockdown check enables in this commit (or the next - the check runs in CI; gating it behind a feature flag during 6c rollout is acceptable as long as the flag flips on by the end of 6c).

## Detailed task list

In recommended commit order. Each item is one focused commit unless noted.

**0. Inventory + revision pass.** Re-verify the `cal::actions` surface against the codebase as 6b lands. RSVP, series-modify, or new operation types added between 6a and 6c would expand scope. Document any deltas before starting.

**1. `action_jobs.kind` CHECK widening migration.** SQLite cannot ALTER CHECK in place. Migration is rebuild-table style: CREATE new table with `CHECK (kind IN ('mail_plan', 'send', 'mark_chat_read', 'calendar_plan'))` -> INSERT SELECT existing rows -> DROP old table -> ALTER TABLE rename. Triggers and indexes are recreated on the new table. No backfill, no default mismatch.

**2. `service-api` wire types.** New `service-api/src/cal_action.rs` with `CalendarOperation` (single enum, serde-derived, no wire mirror), `CalendarActionPlan`, `CalendarOperationResult` (`Success | LocalOnly { reason } | Failed { error }`), `CalendarOperationOutcome { plan_id, op_index, result }`, `CalendarActionCompleted`, `CalendarActionAck`. Move `CalendarEventInput` here from `cal::actions` and derive serde. Round-trip tests per type.

**3. `CalendarActionContext` in `action-types`.** New struct (sibling of `ActionContext`, narrower shape). Constructor `pub(crate)` so only Service handlers can mint one.

**4. `cal_action.execute_plan` request handler.** Service-side `service/src/handlers/cal_action.rs`. Validates the plan (account-id consistency, no duplicate op-ids), enqueues into `action_jobs` with `kind='calendar_plan'`, returns ack. Worker dispatches to the new pipeline asynchronously; the handler does not block on completion.

**5. `service::cal_actions::batch_execute`.** Service-side dispatcher: per-operation, build a `CalendarActionContext` from `BootSharedState`, call the relocated `cal::actions::*` function, capture the `ActionOutcome`, convert to `CalendarOperationResult`, emit `CalendarOperationOutcome`. After all operations complete, emit `CalendarActionCompleted`.

**6. `cal::actions::*` signature change to `&CalendarActionContext`.** Replace `db.conn().lock()` escapes with `ctx.db.with_conn(...)`. The shared `ActionContext` is unchanged - email actions are unaffected. The compile errors cascade only through cal-side call sites. Inside `create_calendar_event`, accept `proposed_event_id` from the wire and use INSERT-or-IGNORE on the local row.

**7. Worker integration.** Worker dispatches by `action_jobs.kind`. Existing email kinds keep their existing handlers; `'calendar_plan'` routes through `service::cal_actions::batch_execute`. Cross-kind scheduling is single FIFO (oldest ready op wins, regardless of kind). Unknown-kind rows go through the `JournalCorrupt` finalize-to-failed path.

**8. UI handlers strip direct calls.** `handle_calendar_create`, `handle_calendar_update`, `handle_calendar_delete` in `crates/app/src/handlers/calendar.rs` route through `client.execute_calendar_plan(...)`. UI mints `proposed_event_id` (UUID) inside `handle_calendar_create` before sending. Per-operation outcomes flow back through the existing `Message::ServiceNotification` arm, dispatched by notification kind into a new `Message::CalendarActionCompleted` arm.

**9. `pending_calendar_action_plans` map mirror.** UI's `ServiceClient` gains a `HashMap<PlanId, ...>` mirroring the mail-side equivalent. Subscribe-or-consume pattern handles fast-completion races. Mirror the Phase 5 `pending_calendars` test cohort here.

**10. Cargo-graph closure: drop `cal` from `app/Cargo.toml`.** With `CalendarEventInput` in `service-api` and the action functions behind IPCs, the only remaining `cal::*` references in app are the wire types (now imported from service-api). Remove the dep; fix any cascade compile errors. **This is what enables 6b's transitive lockdown check.**

**11. Lockdown verification.** Switch 6b's transitive `cargo metadata` check from gated to enforced. Verify `app -> ... -> service-state` returns no path. Constructor-visibility integration test (from 6b task 10c) covers the `CalendarActionContext::new` flip too.

**12. `docs/architecture.md` update.** Strike `cal::actions` from § Current Exceptions (added by 6a's rewrite). Add a parallel "Calendar action pipeline" paragraph alongside § "Action service as mutation gate". Mark `implementation-roadmap.md` Phase 6c entry "LANDED."

## File-by-file changes

**New files:**
- `crates/service-api/src/cal_action.rs` - calendar wire types: `CalendarOperation` (single serde-derived enum), `CalendarActionPlan`, `CalendarOperationResult`, `CalendarOperationOutcome`, `CalendarActionCompleted`, `CalendarActionAck`, plus the relocated `CalendarEventInput`.
- `crates/service/src/handlers/cal_action.rs` - request handler.
- `crates/service/src/cal_actions/mod.rs` - dispatcher.

**Modified files:**
- `crates/db/src/db/migrations.rs` - new rebuild-table migration widening `action_jobs.kind` CHECK.
- `crates/service-api/src/lib.rs` - module declaration.
- `crates/service-api/src/request.rs` - new `RequestParams::CalActionExecutePlan` variant + 5 s timeout.
- `crates/service-api/src/notification.rs` - new `Notification::CalendarOperationOutcome` + `CalendarActionCompleted` + `JournalCorrupt`.
- `crates/service/src/dispatch.rs` - new request arm + worker dispatch by `action_jobs.kind`.
- `crates/service/src/actions/worker.rs` - widen kind dispatch to include `'calendar_plan'`; unknown-kind path emits `JournalCorrupt` notification + finalizes row to failed.
- `crates/calendar/src/actions.rs` - context type change to `&CalendarActionContext`; lock-dance removal; `proposed_event_id` parameter on `create_calendar_event`. `CalendarEventInput` import switches to `service-api`.
- `crates/calendar/src/lib.rs` - re-export `CalendarEventInput` from `service-api` so existing `cal::actions::CalendarEventInput` paths keep working internally.
- `crates/action-types/src/lib.rs` - new `CalendarActionContext` type; `ActionContext` unchanged.
- `crates/app/src/service_client.rs` - new `execute_calendar_plan` async wrapper, `pending_calendar_action_plans` map, completion-notification handling.
- `crates/app/src/handlers/calendar.rs` - replace `cal::actions::*` calls with `client.execute_calendar_plan(...)`. Mint `proposed_event_id` UI-side.
- `crates/app/src/db/calendar.rs` - delete `create_calendar_event` / `update_calendar_event` / `delete_calendar_event` helpers (the last `Db::with_write_conn` callers in the app crate go away here).
- `crates/app/src/update.rs` - new `Message::CalendarActionCompleted` arm.
- `crates/app/Cargo.toml` - drop `cal = { path = "../calendar" }`. The compile cascade is the enforcement.
- `crates/app/src/db/connection.rs` - delete `Db::with_write_conn` (last caller is gone after task 8 + the `app.rs:336` allow-listed site closes here).
- `docs/architecture.md` - Strike `cal::actions` from § Current Exceptions; add Calendar action pipeline paragraph.
- `docs/service/implementation-roadmap.md` - mark Phase 6c "LANDED".

## Code-comment requirements

1. **`crates/service-api/src/cal_action.rs` module-level doc-comment** must contain:
   - "Separate from `WireMailOperation` because the email pipeline's seven exhaustively-matched dispatch sites would gain mostly-fall-through calendar arms otherwise. Calendar and mail share the `action_jobs` journal via the existing `kind` CHECK constraint (widened in 6c to include `'calendar_plan'`); the wire types stay orthogonal. Single enum (no wire mirror) because calendar carries `String`s and serde-able structs - no typed-newtype mirror layer is needed."

2. **`crates/service-api/src/cal_action.rs::CalendarOperationResult` per-variant comment** must document:
   - "`LocalOnly` is reachable only for `CreateEvent`. `UpdateEvent` and `DeleteEvent` are provider-first for synced events and return `Success | Failed`. The asymmetry survives Phase 6c relocation because the underlying action functions encode the policy."

3. **`crates/calendar/src/actions.rs` module-level doc-comment** must add:
   - "Phase 6c introduced `CalendarActionContext` as a sibling of `ActionContext` (which is shared by every email action and stays unchanged). The lock-dance pattern (`db.conn().lock()`) that used to escape the read constraint is gone. Construction of `CalendarActionContext` is `pub(crate)` to `action-types` post-6c; UI source files cannot mint one, which is the compile-time gate that makes calendar mutations Service-only."

4. **`crates/calendar/src/actions.rs::create_calendar_event`** must contain:
   - "`proposed_event_id` is minted UI-side and used as the local row id with INSERT-or-IGNORE. Replay safety: a Service crash between the local insert and the terminal journal finalize replays the same id, which is a no-op locally; the provider call uses the same id for provider-side dedup (CalDAV idempotency token, Google/Graph request-id, JMAP client-supplied id)."

5. **`crates/db/src/db/migrations.rs` migration entry** must contain:
   - "Widens `action_jobs.kind` CHECK from `('mail_plan', 'send', 'mark_chat_read')` to add `'calendar_plan'`. Rebuild-table migration (SQLite cannot ALTER CHECK). No backfill, no default - existing rows already carry their correct `kind`. A row with an unknown `kind` (schema drift, future-build rollback) emits `Notification::JournalCorrupt`, transitions to `status='failed'` with a structured reason blob, and the worker skips it."

6. **`crates/service/src/cal_actions/mod.rs::batch_execute`** must contain:
   - "`CalendarOperationOutcome` is `MustDeliver` class - the UI's `pending_calendar_action_plans` map keys on `plan_id` and unblocks the awaiting caller when the matching `CalendarActionCompleted` arrives. Same latch pattern Phase 5 used for `CalendarRunCompleted`. `ActionOutcome` is converted to `CalendarOperationResult` at this boundary - the wire type is serde-derived; `ActionOutcome` is not."

7. **`docs/architecture.md` § "Action service as mutation gate"** new paragraph (parallel to the existing email paragraph):
   - "Calendar event mutations flow through a sibling pipeline introduced in Phase 6c: `CalendarOperation -> cal_action.execute_plan IPC -> service::cal_actions::batch_execute -> CalendarOperationOutcome notifications -> handle_calendar_action_completed`. The two pipelines share the `action_jobs` journal via the existing `kind` CHECK constraint (widened to include `'calendar_plan'`) but use orthogonal wire types so each retains exhaustive-match discipline. Calendar uses a sibling `CalendarActionContext` rather than the email-shared `ActionContext`. The `cal::actions::*` write-surface escape that Phase 5 documented as a Current Exception is gone after 6c."

## Test plan

### Unit tests

- Wire-type round-trips for every variant of `CalendarOperation` and `CalendarOperationResult`.
- `CalendarOperation` exhaustive-match shape regression test (mirroring `mail_side_mirror_is_exhaustive`).
- `action_jobs.kind` CHECK widening migration: pre-migration the constraint rejects `'calendar_plan'`; post-migration it accepts it. Existing rows survive with unchanged `kind` values.
- `proposed_event_id` replay-idempotency: invoke `create_calendar_event` twice with the same id; assert the second call is a no-op locally and the row count stays at 1.
- Unknown-kind `JournalCorrupt` finalization: insert a row with a `kind` value the migration's CHECK rejects (only possible by direct SQL bypassing the constraint, e.g., a future-build rollback simulated at the DB layer); run worker; assert notification fires once + row is finalized to `status='failed'` + lease index does not pick it up on subsequent passes.

### Integration tests (in-process)

- `calendar_create_event_round_trips_through_ipc`: UI ships a create plan; Service writes the local row using the UI-supplied `proposed_event_id`; UI observes the row via the read path.
- `calendar_create_event_replays_idempotently_after_crash`: run create up to the local-insert-success state; simulate Service crash; replay the same plan; assert the local row is unchanged and no provider duplicate is created.
- `calendar_update_event_provider_failure_returns_failed`: stub provider that fails; assert `CalendarOperationResult::Failed` (not `LocalOnly`) on update of a synced event.
- `calendar_create_event_provider_failure_returns_local_only`: stub provider that fails; assert `CalendarOperationResult::LocalOnly` on create.
- `calendar_action_completion_late_subscriber_catches_fast_completion`: mirror of the mail-side latch test (Phase 2 task 3).
- `action_jobs_dispatches_by_kind_after_respawn`: enqueue one mail op + one calendar op; respawn Service mid-flight; assert each gets dispatched to the right pipeline on recovery.
- `cross_kind_fifo_scheduling`: enqueue mail-then-calendar-then-mail; assert worker leases in submit order regardless of kind.
- `action_job_status_returns_calendar_plan_kind`: respawn Service after submitting a calendar plan; `action.job_status` IPC returns the row with `kind='calendar_plan'`; UI's reconciliation handles it via the existing match.

### Real-subprocess smoke tests

- `service_subprocess_calendar_create_event`: real IPC, real CalDAV stub, observe the local row + provider call.
- `service_subprocess_action_jobs_kind_widening_survives_respawn`: kill Service mid-plan; restart; assert the unfinished operation completes via the right pipeline.
- `service_subprocess_lockdown_transitive_check`: assert the `cargo metadata` graph from `app` has no path to `service-state` post-task-10.

### Manual matrix updates

- Calendar event create / update / delete via the UI editor against each provider (Google, Graph, JMAP, CalDAV).
- Verify `ActionOutcome::LocalOnly` surface in the UI - the user sees the event locally with a "not synced" indicator if provider create fails.

## Open questions

- **`CalendarActionContext::new` visibility nuance.** `pub(crate)` to `action-types` blocks UI sources. Test fixtures need a way in; resolution is a `#[cfg(any(test, feature = "test-helpers"))]` `pub fn new_for_testing(...)` adjacent. Decide on the exact feature gate during task 3.
- **Calendar plan-id allocation.** Email uses a UI-side `PlanId` minted at intent-resolution time. Calendar reuses the same allocator (one `PlanId` space) - one less concept to track, and cross-pipeline plan-id collision is impossible because mail and calendar plans live in distinct UI-side maps.
- **Notification class for `CalendarOperationOutcome`.** Plan picks `MustDeliver` because the UI-side latch pattern requires it. If measurement shows per-operation outcomes are too noisy on bulk operations (a future "delete all events in series" affordance), revisit.
- **Repack-style reclaim for `action_jobs` history.** `JournalCorrupt`-finalized rows accumulate over time. Plan defers a periodic-purge story to a future sweeper; if dogfooding shows the table growing problematically, add a `pack.gc_kick`-shaped sweeper.

## Verification (end-to-end)

- The 6a CI lockdown script returns clean: no `Db::with_write_conn`, `Db::with_write_conn_sync`, or `Db::write_db_state` references in `crates/app/src/`. The `app.rs:336` allow-listed entry is removed.
- The 6b transitive `cargo metadata` lockdown check returns clean: no path from `app` reaches `service-state` (direct or transitive).
- `crates/calendar/src/actions.rs` no longer calls `db.conn().lock()`; all writes go through `ctx.db.with_conn(...)`. The functions take `&CalendarActionContext`.
- `crates/app/src/db/calendar.rs` no longer exists.
- `crates/app/Cargo.toml` no longer lists `cal` as a dependency.
- A calendar event create / update / delete flows through the IPC end-to-end against each of the four providers.
- Replaying a create (Service crash mid-plan) does not produce duplicate local rows or duplicate provider events.
- `docs/architecture.md` no longer lists `cal::actions` as a Current Exception; the new "Calendar action pipeline" paragraph is present.

## Promotion criteria

- All items in `In scope` landed.
- All items in `Out of scope` (RSVP, series-vs-occurrence, calendar attachments) named in the implementation-roadmap Phase 6c stanza as future-Phase-6d work.
- The Phase 6 stanza of `implementation-roadmap.md` shows 6a, 6b, and 6c all "LANDED".
- `docs/architecture.md` reflects the post-Phase-6c state without a Calendar Current Exception.
- `phase-6c-plan.md` is then retirement-ready: every deferral has an explicit roadmap entry; every code-comment requirement is present in the relevant file.

## Notes for the post-6b revision pass

When 6b lands, walk through this plan and check:

- **Has the lockdown enforcement story changed?** Phase 6b lands the direct-dep + constructor-visibility checks but defers the transitive `cargo metadata` check to 6c. If 6b's plan changes that sequencing, this plan's task 11 changes accordingly.
- **Has the cross-store invariant pass extension informed any calendar-table invariants?** Phase 6b extends the pass to attachment cache; if the architectural lessons there suggest a calendar-event invariant pass, add it to scope.
- **Has Phase 6b changed the notification class catalog?** The plan picks `MustDeliver` for `CalendarOperationOutcome` and `JournalCorrupt`; if 6b introduced a new class better suited, swap.
- **Has the cal::actions surface grown?** RSVP or series-modify landing between 6a and 6c expands the in-scope wire types significantly enough to warrant a fresh draft.
- **Has the marker-helper from 6b grown a place for action-journal recovery?** 6b introduces `crates/service/src/markers/` for sync/push/account-delete recovery. `action_jobs` already has its own respawn-recovery story (Phase 2), but if 6b's helper is general enough to host it, calendar's `JournalCorrupt` finalize path may benefit from re-using the `MarkerFile<T>` shape.
