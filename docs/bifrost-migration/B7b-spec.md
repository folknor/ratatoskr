# B7b technical-implementation-spec: calendar actions on the bifrost `Account` write surface

## Required reading (clause 10)

Read these before implementing; they are the ground this spec is built on and
judged against.

- `reference/technical-implementation-spec.md` - the contract this document is
  written against (every clause reference below - "clause 5", "clause 8" - is to
  that document).
- `reference/architecture.md` - the cross-cutting architecture contract. Binding
  here specifically: crate boundaries (`cal` today DOES depend on `core`/`rtsk` -
  `calendar/Cargo.toml:14` is `rtsk = { path = "../core" }` and `lib.rs:1` uses
  `rtsk::provider::http` - alongside `service-state` and `action-types`; the real
  invariant is the reverse: `core` must NOT depend on `cal`, because adding `cal`
  to `core` is a circular dep. The load-bearing boundary B7b works is cal-vs-service
  (§ 4.1), not cal-vs-core),
  the `ActionOutcome` / `MutationLog` calendar-action taxonomy, calendar workflow
  layering, and the `CalendarActionContext` sibling of `ActionContext`.
- `docs/bifrost-migration.md` - the source TODO. B7b is defined at the
  "B7b. Calendar actions" bullet; its two parked questions (CalDAV change-stream
  asymmetry; iMIP scope) are resolved in the § 7 decomposition note. Read the
  whole B7 section: B7b reuses B7a's landed id-translation seam and must not
  re-litigate the decomposition.
- `reference/glossary/harness.md` - required because B7b's gates are
  per-provider calendar action-writeback sync-harness scripts run against
  `saehrimnir`. Read it before writing or running any `.lua` script or
  `brokkr sync-bench` / `brokkr service-test`.
- `docs/calendar/problem-statement.md` - the calendar subsystem's design context
  (the read/write seam, the `CalendarRuntime` supervisor, the local-first vs
  provider-first write policy this spec preserves).

B7b touches no folders/labels tables and no overlay surfaces, so
`reference/glossary/folders-labels.md` and `.../overlay-surfaces.md` are not in
scope. `UI.md` is in scope only for the small reading-pane / event-detail RSVP
affordance brick (§ 4.6); read it before that brick.

## 1. The goal (clause 7: the target as concrete artifacts)

After B7b there is exactly one calendar write path, and it goes through the
bifrost `Account` trait. The four-way `match effective { google | graph | jmap |
caldav }` provider dispatch in `crates/calendar/src/actions.rs` is gone, replaced
by a single call site per operation against an `Arc<dyn bifrost_types::Account>`
opened from the SAME `build_calendar_account_factory` router B7a already uses for
reads. Every typed-client dependency the calendar write path pulled in
(`GmailClient`, `GraphClient`, `JmapClient`, ratatoskr's own CalDAV client and
iCal generator) is deleted from the `cal` crate.

Concretely, the target surface:

- `crates/calendar/src/actions.rs` keeps its four public entry points and their
  local-first / provider-first policy, but their provider legs dispatch through
  bifrost:
  - `create_calendar_event(ctx, account_id, calendar_remote_id, input) -> ActionOutcome`
    - local insert (unchanged), then `account.event_create(EventCreate)`.
  - `update_calendar_event(ctx, account_id, event_id, input) -> ActionOutcome`
    - provider-first `account.event_update(EventId, EventPatch)` for synced
      events; local-only path unchanged.
  - `delete_calendar_event(ctx, account_id, event_id) -> ActionOutcome`
    - provider-first `account.event_delete(EventId)`; local-only path unchanged.
  - NEW `rsvp_calendar_event(ctx, account_id, event_id, status: RsvpStatus) -> ActionOutcome`
    - provider-first `account.event_rsvp(EventId, RsvpStatus)`, then local
      `rsvp_status` write-back.
- The `enum CalendarProvider`, `create_calendar_provider`, `input_to_json`,
  `dispatch_create` / `dispatch_update` / `dispatch_delete`, and the
  `CalendarEventDto` return plumbing are DELETED.
- One new helper replaces `create_calendar_provider`:
  ```rust
  async fn open_calendar_account(
      ctx: &CalendarActionContext,
      account_id: &str,
  ) -> Result<Arc<dyn bifrost_types::Account>, ActionError>;
  ```
  It builds the factory via
  `service::bifrost::factory::build_calendar_account_factory(&ctx.read_db,
  ctx.write_db.writer_pool(), account_id, ctx.encryption_key)` and calls
  `factory.open(AccountId(account_id.into()))`. (See § 4.1 for the crate-boundary
  resolution: the factory builder is Service-side, so the builder call moves to
  the Service `cal_actions` layer and the `Arc<dyn Account>` is passed IN to the
  `cal` action functions, mirroring how `calendar_sync_account_impl` receives its
  `factory` argument.)
- The B7a id-translation seam (`crates/calendar/src/idmap.rs`) grows the WRITE
  direction it was designed to host. New public functions (§ 4.3):
  - `event_id_for_writeback` (ALREADY EXISTS - built by B7a, currently unused;
    B7b is its first caller).
  - `epoch_to_event_time(epoch_secs, is_all_day, timezone) -> EventTime` - the
    inverse of the existing `event_time_to_epoch`.
  - `parse_availability` / `parse_visibility` / `parse_status` / `parse_rsvp` -
    the inverses of the existing `availability` / `visibility` / `status` /
    `rsvp` string mappers.
  - `input_to_event_create(calendar_remote_id, &CalendarEventInput) -> EventCreate`.
  - `input_to_event_patch(&CalendarEventInput) -> EventPatch`.
- `crates/service-api/src/cal_action.rs` gains a fourth wire variant
  `WireCalendarOperation::RsvpEvent { event_id, response }`, and
  `crates/service/src/cal_actions/mod.rs::run_one` gains the matching arm.
- Files DELETED whole: `crates/calendar/src/google.rs`,
  `crates/calendar/src/graph.rs`, `crates/calendar/src/caldav/mod.rs`,
  `crates/calendar/src/caldav/ical.rs` (the write-side iCal generator), and the
  `crates/jmap/src/calendar_sync/` write functions (`create_event_remote` /
  `update_event_remote` / `delete_event_remote` in `protocol.rs`). See § 2.3 for
  the exact residue and § 5 for the stopping rule.

End state: `cal`'s only calendar dependency on a provider is the bifrost
`Account` trait; the `google_calendar_*_impl` / `graph_calendar_*_impl` /
`caldav_*_impl` / `jmap::calendar_sync::*_remote` functions no longer exist; the
`etag`-threading writeback contract is retired (§ 4.4); and the app can RSVP to a
synced meeting invite from the reading pane through the same pipeline.

## 2. Survey of the ground (clause 8)

### 2.1 What B7a already laid (available, reuse - do not rebuild)

Reconciled against `crates/calendar/src/idmap.rs`, `crates/calendar/src/sync.rs`,
and `crates/service/src/bifrost/factory.rs` as they stand:

- `build_calendar_account_factory(db, writer, account_id, key) -> Result<Option<Arc<dyn AccountFactory>>, _>`
  (`factory.rs:170`) already resolves calendar-provider precedence correctly:
  `calendar_provider = "caldav"` (or `provider = "caldav"` with a non-empty
  `caldav_url`) builds a `CalDavAccountFactory` from the account's dedicated
  `caldav_username` / `caldav_password` via Basic auth; Gmail/Graph/JMAP fall
  through to `build_account_factory`; IMAP-only returns `Ok(None)`. B7b's write
  path opens its `Account` from this exact function, so calendar-over-mail
  precedence and the CalDAV credential split are inherited, not re-derived.
  Caveat (R1 finding 6): this is a behavior CHANGE, not byte-identical
  preservation. Today's `create_calendar_provider` (`actions.rs:96-125`) routes
  purely on `calendar_provider.unwrap_or(provider)` string equality
  (`"google_api" | "gmail_api" | "graph" | "jmap" | "caldav"`); the factory
  (`factory.rs:181-208`) uses the richer rule (`calendar_provider == "caldav"`
  OR `provider == "caldav"` with a non-empty `caldav_url`, else
  `MailProviderKind::parse`). They disagree on the edge case
  `provider == "caldav"` + a mail-provider `calendar_provider`: the legacy code
  picks the mail provider, the factory picks CalDAV. B7b adopts the factory's
  (arguably more correct) rule. Flag this in the landing commit so it is a known
  intentional change, not a silent one.
- `factory.open(AccountId(id))` yields an `Arc<dyn Account>` whose
  `capabilities().pim_methods` carries the per-method write flags
  `event_create` / `event_update` / `event_delete` / `event_rsvp`
  (`bifrost-types` `capabilities.rs:248-251`). The read path already gates on
  `pim_methods.calendars_list` / `.events_in_range` (`sync.rs:78`); the write
  path gates identically on the write flags (§ 4.5).
- `idmap::event_id_for_writeback(provider, remote_event_id, etag, calendar_remote_id)
  -> (EventId, Option<CalendarId>)` (`idmap.rs:84`) already reconstructs the
  composite `calendar_id::event_id` EventId for Gmail/Graph and the bare id for
  JMAP/CalDAV. It exists precisely for B7b and currently has zero callers - B7b
  is its consumer. (It takes `_etag`, unused, consistent with § 4.4.)
- `idmap::calendar_remote_id`, `provider_name`, and the forward value mappers
  (`status` / `availability` / `visibility` / `rsvp`, all bifrost-enum ->
  ratatoskr-string) are landed; B7b adds only their inverses.

### 2.2 What B7b rips out (the per-provider write dispatch)

`crates/calendar/src/actions.rs` today (730 lines) is the whole target. Its
structure:

- `enum CalendarProvider { Google(GmailClient) | Graph(GraphClient) |
  Jmap(JmapClient) | CalDav { account_id } }` (lines 59-64) - the four-way axis
  B7b collapses. This is the calendar analog of the `sync.rs` provider-string
  branch B7a already deleted; B7b finishes the job on the write side.
- `create_calendar_provider(ctx, account_id)` (lines 70-125) - re-reads
  `provider` / `calendar_provider` and constructs a typed client per branch.
  DELETED; replaced by `open_calendar_account` (§ 4.1), which delegates provider
  precedence to the shared factory rather than re-implementing it (the current
  function's doc-comment even admits it duplicates
  `calendar_sync_account_impl`'s routing - that duplication is the smell B7b
  removes).
- `input_to_json` (lines 133-146) - lowers `CalendarEventInput` into the
  provider-specific JSON the three JSON providers parse. DELETED; the lowering
  target becomes bifrost's typed `EventCreate` / `EventPatch` (§ 4.3), no JSON.
- `dispatch_create` / `dispatch_update` / `dispatch_delete` (lines 150-305) - the
  per-variant four-arm matches. DELETED; each collapses to one bifrost call.
- The action functions `create_calendar_event` / `update_calendar_event` /
  `delete_calendar_event` (lines 365-729) - KEPT, but their provider legs are
  rewired. The local-first / provider-first / local-only policy, the
  `MutationLog` emission points, the `ActionOutcome` variants, and the
  `with_write_mapped` DB shape all stay byte-for-byte where they touch the DB;
  only the provider dispatch inside each changes.

### 2.3 What B7b deletes outside `actions.rs` (the now-orphaned write helpers)

After B7a deleted the read-sync bodies, these files hold ONLY the write helpers
`actions.rs` dispatches to. Once § 2.2 removes the dispatch, they are dead:

- `crates/calendar/src/google.rs` - `google_calendar_create_event_impl` /
  `_update_` / `_delete_` (plus the `google_calendar_api_base` const machinery in
  `lib.rs:12-27` that only these use). DELETE the three impls; the
  `GOOGLE_CALENDAR_*` consts and `google_calendar_api_base()` go too (grep
  confirms no other caller after the impls are gone).
- `crates/calendar/src/graph.rs` - `graph_calendar_create_event_impl` /
  `_update_` / `_delete_`. DELETE whole file.
- `crates/calendar/src/caldav/mod.rs` - `caldav_create_event_impl` /
  `_update_` / `_delete_`, `load_caldav_account_config`, and the CalDav config
  accessor struct. DELETE whole file.
- `crates/calendar/src/caldav/ical.rs` - the `BEGIN:VCALENDAR` / `VEVENT` iCal
  GENERATOR used only by `caldav_*_impl` writes (bifrost-caldav now generates its
  own iCalendar on the write path). DELETE whole file; the `caldav` module
  directory disappears with it.
- `crates/jmap/src/calendar_sync/protocol.rs` - `create_event_remote` /
  `update_event_remote` / `delete_event_remote` (re-exported at
  `calendar_sync/mod.rs:17`). DELETE these three functions and the re-export.
  Confirm during teardown whether `calendar_sync/mod.rs` / `payload.rs` /
  `persist.rs` retain any non-write use (B7a deleted `crates/calendar/src/jmap.rs`
  but the `jmap` crate's own `calendar_sync` module was read-and-write); if the
  module is write-only residue, delete it whole, else delete only the three
  functions. This is a survey-time determination, not a guess.
- `crates/calendar/src/types.rs::CalendarEventDto` (and the
  `CalendarEventInput = CalendarEventDto` alias) - the DTO the deleted
  `dispatch_*` helpers returned. After § 2.2 nothing in `cal` returns a
  `CalendarEventDto`. Confirm no out-of-crate consumer (the app's own
  `CalendarEventInput` in `app/src/db/types.rs` is a DISTINCT type - do not
  conflate); delete the DTO if unreferenced, else narrow it. `CalendarInfoDto` /
  `CalendarInfoInput` are unrelated (calendar metadata, not events) and stay.

`crates/calendar/src/lib.rs` loses its `pub mod caldav; pub mod google; pub mod
graph;` declarations and the Google-calendar const block. The `cal` crate's
`Cargo.toml` drops its `gmail` / `graph` / `jmap` dependencies (confirm none
survive via a non-calendar path before removing - this is a § 1
maximal-integration deletion, gated by `brokkr check`).

### 2.4 The wire + Service + app layers the rewire preserves

- `service-api::WireCalendarOperation` (`cal_action.rs:90`) has three variants
  today (`CreateEvent` / `UpdateEvent` / `DeleteEvent`). B7b ADDS a fourth,
  `RsvpEvent { event_id, response }`. Note (R1 finding 5): this IS a genuine
  wire-frame change - a new variant on `WireCalendarOperation`
  (serde-compatible, but an addition). Do not overclaim it as pre-anticipated:
  the "future RSVP ... intents in Phase 6d can layer in N-op plans without
  changing the wire frame" doc-comment lives on `CalendarActionWireOperation`
  (`cal_action.rs:120-122`, the per-op PLAN struct) and speaks to multi-OP
  plans, not to new operation KINDS on the inner enum.
- `service::cal_actions::batch_execute` / `run_one` (`cal_actions/mod.rs:47`) -
  the sequential dispatcher. B7b: (a) `run_one` gains an `RsvpEvent` arm; (b) the
  `Arc<dyn Account>` is opened HERE (or in a thin wrapper) once per op and passed
  into the `cal::actions::*` functions (§ 4.1). `wire_input_to_domain` and
  `outcome_to_wire` are unchanged except `outcome_to_wire` already handles all
  three `ActionOutcome` variants exhaustively, so RSVP reuses it.
- `service::handlers::cal_action::handle` (`handlers/cal_action.rs`) - journals
  the plan and validates; it already calls `wire_input_to_domain` for
  validation. Add `RsvpEvent` to any exhaustive match there.
- App: `handlers/calendar.rs::handle_save_event` (line 601) builds Create/Update
  plans; `handlers/calendar.rs:315` builds Delete plans. B7b adds an RSVP plan
  builder (§ 4.6). The harness's `wire_calendar_operation` parser
  (`app/src/harness/mod.rs:3731`) gains an `rsvp_event` arm so scripts can drive
  it.

### 2.5 A naming smell to confirm, not fix, in passing

`WireCalendarOperation::CreateEvent.calendar_remote_id` is populated app-side
from `session.draft.calendar_id` (a LOCAL calendar id -
`handlers/calendar.rs:638`), and `actions.rs::create_calendar_event` treats its
third argument as a local id (`lookup_calendar_remote_id`). The wire field name
says "remote" but carries a local id. B7b preserves this behavior exactly (local
id in, `lookup_calendar_remote_id` resolves it to the bifrost-native remote id
for `EventCreate.calendar_id`). Flag it in the landing commit as a follow-up
rename; do not change the contract inside B7b (it would ripple into the app and
harness for no correctness gain). Lateral finding, logged not fixed.

## 3. The split (clause 6: keep/revert, ordered so the tree stays green)

Three landings, each independently green under `brokkr check` and its named
gates.

### B7b-1 - idmap write direction (pure addition, no cutover)

Add the inverse mappers and lowering functions to `idmap.rs` (§ 4.3) with unit
tests. Nothing calls them yet; `event_id_for_writeback` gains its first test.
This lands first because B7b-2 depends on it and it carries zero behavioral risk
(new pure functions + tests). Gate: `brokkr test -p cal idmap`.

### B7b-2 - rewire create/update/delete + delete the enum + delete the helpers

The trunk. Replace `actions.rs`'s provider dispatch with bifrost calls (§ 4.1,
4.2), move the factory-open into the Service `cal_actions` layer, delete the
`CalendarProvider` enum and every § 2.3 write helper in the SAME landing (the
enum and the helpers are mutually load-bearing - deleting one without the other
does not compile, so they cut over atomically). Gate: the three existing
provider families' action-writeback harness scripts (§ 6), `brokkr check`.

### B7b-3 - RSVP (wire variant + action fn + app affordance + iMIP routing)

Add `WireCalendarOperation::RsvpEvent`, `cal::actions::rsvp_calendar_event`, the
`run_one` arm, the app reading-pane / event-detail RSVP plan builder, and the
iMIP-invite-to-`event_rsvp` routing for a synced invite (§ 4.6). This is last
because it is additive over the B7b-2 trunk (a fourth operation on the same
opened `Account`) and carries its own new gate. Gate: a new per-provider RSVP
writeback harness assertion, `brokkr check`.

Ordering rationale: B7b-1 is dormant so it cannot break green; B7b-2 is the
atomic cutover; B7b-3 only adds. No landing leaves a half-deleted enum or an
env/routing switch.

## 4. The bricks

### 4.1 Opening the `Account` (crate-boundary resolution)

`build_calendar_account_factory` lives in `crates/service/` and depends on the
Service's factory graph; `cal` cannot call it without a circular dep (the same
reason `calendar_sync_account_impl` RECEIVES its `factory: Arc<dyn
AccountFactory>` as a parameter rather than building it - `sync.rs:40`,
`service/src/calendar.rs:440-458`). B7b mirrors that exactly:

- The `cal::actions::*` functions take an opened account, not a context they open
  from. Concretely, thread `account: &Arc<dyn bifrost_types::Account>` (or
  `&dyn Account`) as a parameter, alongside the existing `ctx:
  &CalendarActionContext` (kept for the DB writer half + `encryption_key` +
  `read_db`).
- `service::cal_actions::run_one` opens the account ONCE per op before
  dispatch:
  ```rust
  let factory = crate::bifrost::factory::build_calendar_account_factory(
      &ctx.read_db, ctx.write_db.writer_pool(), &op.account_id, ctx.encryption_key,
  ).await;
  ```
  `Ok(None)` (IMAP-only, no calendar backend) -> the op resolves to
  `ActionOutcome::Failed`/`LocalOnly` with a "no calendar backend for account"
  reason without touching the provider (matches the read path's `Ok(None)`
  no-op). `Ok(Some(factory))` -> `factory.open(AccountId(op.account_id.clone()))`;
  an open error maps to `Failed` (update/delete/rsvp) or `LocalOnly`
  (create), preserving the current `create_calendar_provider`-error mapping in
  each action function.
- Update/Delete/RSVP resolve the event's OWN `account_id` from the DB first
  (unchanged - `lookup_event_meta`), then open against THAT id, preserving the
  multi-account "event's own account is authoritative" guarantee
  (`actions.rs:466-467`). So `run_one` cannot always pre-open for Update/Delete
  before the meta lookup; the account id is only known mid-function. The chosen
  shape is trait injection: `cal` depends on a small `CalendarAccountOpener`
  trait provided by the Service, so `cal` never names
  `build_calendar_account_factory`.

  This seam is the spec's softest and most consequential; it is pinned here, not
  left to implementation time (R1/R2 flagged the underspecification and an
  internal contradiction - see below):
  ```rust
  #[async_trait]
  pub trait CalendarAccountOpener: Send + Sync {
      // Returns None for an IMAP-only account (factory Ok(None), no calendar
      // backend). Returns the ProtocolKind alongside the account so the write
      // path never re-parses a provider string (resolves § 4.2's sourcing).
      async fn open(
          &self,
          account_id: &str,
      ) -> Result<Option<(Arc<dyn bifrost_types::Account>, ProtocolKind)>, ActionError>;
  }
  ```
  Contradiction resolved: an earlier draft gave the trait
  `Option<Arc<dyn Account>>` here while § 4.2 later required
  `(Arc<dyn Account>, ProtocolKind)`. The trait now returns the tuple; § 4.2's
  "option (b)" and this signature are the same thing.

  Ownership and placement (pinned; R1 finding 1):
  - The context holds the opener as an OWNED `Arc<dyn CalendarAccountOpener>`,
    NOT `&dyn ...`. `CalendarActionContext` (`action-types/src/context.rs:133`)
    is `#[derive(Clone)]`, owns its three fields by value, and has no lifetime
    parameter. A `&dyn` field would force a lifetime onto the struct, break the
    `Clone` derive, and ripple `&CalendarActionContext` into
    `&CalendarActionContext<'_>` at every call site. So: add
    `pub opener: Arc<dyn CalendarAccountOpener>` (`'static`) to
    `CalendarActionContext`.
  - Crate-dep consequence, stated because it is not free: to hold the opener,
    `CalendarActionContext` lives in `action-types`, and the trait signature
    names `bifrost_types::Account` / `ProtocolKind`. `action-types/Cargo.toml`
    today depends on `db`, `service-api`, `service-state`, `store`, `search` -
    NOT `bifrost-types`. B7b ADDS a `bifrost-types` dependency to
    `action-types`. This is a real graph edge; confirm it does not open an
    `app -> bifrost-types` path the Phase 6b/6c lockdown tests forbid (it should
    not - `app` does not depend on `action-types`, per the context.rs:120-125
    note - but the lockdown test is the gate, run it).
  - The Service provides the impl wrapping `build_calendar_account_factory` +
    `factory.open`, capturing the `ProtocolKind` it already knows when it
    selects the factory branch (CalDAV vs `MailProviderKind`).

### 4.2 The three rewired action functions

Each keeps its outer structure (MutationLog, local DB writes, ActionOutcome
variants). Only the provider leg changes.

**`create_calendar_event`** (`actions.rs:365`):
- Local insert unchanged (`create_calendar_event_sync`), still returns
  `(event_id, calendar_remote_id)` where `calendar_remote_id` is resolved from
  the local calendar id via `lookup_calendar_remote_id` (§ 2.5).
- Provider leg: `open_calendar_account` (or injected opener) -> capability gate
  on `pim_methods.event_create` (false -> `LocalOnly { retryable: false }`) ->
  `account.event_create(idmap::input_to_event_create(&calendar_remote_id,
  &input))`.
- On `Ok(EventId)`: strip the composite EventId back to the native remote id
  (the returned `EventId` is bifrost's composite form; reuse the same rsplit rule
  `idmap::event_remote_id` applies, exposed as a small
  `idmap::strip_event_id(provider, &EventId) -> String` helper) and persist it
  via `set_calendar_event_remote_id_and_etag(conn, &event_id, &native_remote_id,
  None)`. etag is `None` (§ 4.4) - the next sync fills it. `ActionOutcome::Success`.
  - ID-STABILITY REQUIREMENT (R1 finding 2), the one place to assert rather than
    assume: the `native_remote_id` stored at create time MUST equal what the next
    read sync computes as this event's upsert key, or a transient duplicate
    survives until reconcile. For Gmail/Graph `strip_event_id` reverses the
    composite and `event_remote_id` = `native_event_id` matches. For CalDAV the
    two diverge by construction: `event_remote_id` = the bare href/native_id
    (`idmap.rs:80-82`), but the read-sync DEDUP key is
    `make_google_event_id(uid, ...)` = `caldav:{uid}::...` (`idmap.rs:68-78`).
    JMAP returns a bare id. So the create-time stored id must be the SAME key the
    read-sync upsert uses for that provider, not merely "a" remote id. This is a
    correctness assertion the § 6.2 harness scripts MUST make explicit (create,
    then read-sync, and assert exactly one local row - no duplicate), not an
    assumption the etag-is-None note covers (that note addresses etag freshness,
    not id identity).
- On `Err`: `LocalOnly { reason, retryable: false }` (unchanged policy - the
  local row persists with `remote_event_id = NULL`).

**`update_calendar_event`** (`actions.rs:468`):
- Meta lookup + local-only branch for unsynced events: UNCHANGED.
- Synced branch: capability gate on `pim_methods.event_update` (false ->
  `Failed`) -> build the writeback EventId via
  `idmap::event_id_for_writeback(provider, remote_event_id, None,
  Some(&calendar_remote_id))` (provider read from `account.capabilities()` /
  the account's `ProtocolKind` - see § 4.3 on sourcing `provider`) ->
  `account.event_update(event_id, idmap::input_to_event_patch(&input))`.
- On `Ok(())`: write the edited fields locally via
  `update_calendar_event_fields_and_etag(conn, &eid, &params, None)` - same call
  as today but etag `None` (§ 4.4). `Success`.
- On `Err`: `Failed` (unchanged).

**`delete_calendar_event`** (`actions.rs:619`):
- Meta lookup + local-only delete for unsynced: UNCHANGED.
- Synced branch: capability gate on `pim_methods.event_delete` (false ->
  `Failed`) -> `account.event_delete(idmap::event_id_for_writeback(...).0)` ->
  on `Ok`, local `delete_calendar_event_sync` (unchanged, with the same
  "provider succeeded, local cleanup best-effort" warn-on-failure semantics at
  `actions.rs:722`). On `Err`: `Failed`.

Sourcing `provider: ProtocolKind` for `event_id_for_writeback`: the opened
`account` does not directly expose its `ProtocolKind`. Two clean options: (a)
read it from the `calendars.provider` / `accounts.calendar_provider` column
already loaded in the meta/calendar lookup and map the string to `ProtocolKind`
(inverse of `idmap::provider_name`); (b) have the injected opener return
`(Arc<dyn Account>, ProtocolKind)`. Option (b) is cleaner - the Service knows the
`ProtocolKind` when it builds the factory (`MailProviderKind` / the caldav
branch) - so the opener returns the provider kind alongside the account. Specify
option (b); it removes a string round-trip and a fallible re-parse.

### 4.2a All-day exclusive-end normalization (R2 finding 3, in scope)

The bifrost all-day contract is exclusive-end (`calendar.rs:98-114`), and
`epoch_to_event_time` (§ 4.3) is a correct inverse ONLY when handed an exclusive
end. The app does not currently produce one: `build_wire_input`
(`handlers/calendar.rs:750-760`) builds `start_time` and `end_time` from the same
`draft.start_date`, so an all-day save formats both endpoints to the same date -
a zero-day span. B7b adds an app-side normalization brick:

- When `draft.all_day` is true, `build_wire_input` (or a helper it calls) sets
  the wire `end_time` to an epoch whose UTC date is at least ONE day after the
  start date (exclusive-end), independent of the hidden end hour/minute fields.
  A single-day all-day event thus lowers to start = day D, end = day D+1,
  matching the 86_400-second round-trip the forward `event_time_to_epoch` test
  already asserts.
- App-level tests (new): a single-day all-day save produces a one-day exclusive
  span; a multi-day all-day save preserves the correct exclusive end. These live
  with `build_wire_input` in the `app` crate (a `brokkr test -p app` case, added
  to § 6.2), because the bug is in the app lowering, not in `idmap`.

This is IN scope for B7b-2: without it the rewired create/update path writes a
malformed all-day range to every provider, which the § 6.2 harness create-case
assertion (sent fields match) would catch as a regression.

### 4.3 The idmap write functions (B7b-1)

All in `crates/calendar/src/idmap.rs`, each with a unit test mirroring the
existing forward-direction tests.

- `epoch_to_event_time(epoch_secs: i64, is_all_day: bool, timezone: Option<&str>)
  -> EventTime`: inverse of `event_time_to_epoch` (`idmap.rs:100`). All-day ->
  `EventTime { value: "%Y-%m-%d" (UTC-midnight date), timezone: None }`,
  honoring the exclusive-end all-day contract (bifrost `calendar.rs:98-114`) -
  the caller passes the exclusive end for the end field, matching how
  `event_time_to_epoch` already round-trips a one-day all-day event to an
  86_400-second span. Timed with a `timezone` -> RFC 3339 in that zone; timed
  without -> RFC 3339 UTC (`...Z`). Round-trip property test:
  `event_time_to_epoch(epoch_to_event_time(t, all_day, tz), all_day) == t` for
  the DST-gap, ambiguous, floating, and all-day cases the forward tests already
  cover.
  - UPSTREAM ALL-DAY BUG this converter cannot fix alone (R2 finding 3): the
    converter is a correct inverse ONLY when its end epoch already represents the
    EXCLUSIVE following date. Today it does not. `build_wire_input`
    (`handlers/calendar.rs:750-760`) derives BOTH `start_time` and `end_time`
    from the same `draft.start_date` (differing only by hidden hour/minute
    fields), so for an all-day event both epochs format to the SAME date. Feeding
    those to `epoch_to_event_time` yields a zero-day span (JMAP `P0D`; Google/
    Graph an invalid or empty all-day range). The FIX is an upstream
    normalization brick, not a converter change (§ 4.2a): the app must model the
    all-day end as an explicit exclusive date at least one day after the start.
    Without it the converter is correct but its input is wrong.
- `parse_availability(&str) -> EventAvailability`, `parse_visibility(&str) ->
  EventVisibility`, `parse_rsvp(&str) -> RsvpStatus`: case-insensitive inverses
  of the existing forward mappers (`idmap.rs:166-208`). Unknown / unrecognized ->
  the enum's neutral default (`EventAvailability::Busy`; `EventVisibility::Default`;
  `RsvpStatus::NeedsAction`) rather than the `Unknown` variant, because these feed
  WRITE payloads where `Unknown` is not a legal thing to send. State this choice
  in the doc-comment.
  - NO `parse_status` (R1 finding 3): an earlier draft mandated
    `parse_status(&str) -> EventStatus` with a required unit test, but it has NO
    production consumer. `CalendarEventInput` (`actions.rs:44-55`) carries no
    status field; `input_to_event_create` hardcodes `status = Confirmed` and
    `input_to_event_patch` leaves `status: None`. `parse_status` is dropped as
    dead code; its test is dropped from § 6.1.
  - None-vs-unknown collapse (R1 minor): `input_to_event_create` /
    `input_to_event_patch` map `availability: Option<String>` /
    `visibility: Option<String>` into the non-optional bifrost enums via
    `.as_deref().map(parse_availability).unwrap_or(EventAvailability::Busy)` (and
    `parse_visibility` / `EventVisibility::Default`). So a `None` from the form
    and an unrecognized string collapse to the SAME neutral default - state this
    explicitly so it is not read as a lost distinction.
- `input_to_event_create(calendar_remote_id: &str, input: &CalendarEventInput)
  -> EventCreate`: maps the ratatoskr `CalendarEventInput` (title, description,
  location, start/end epoch, is_all_day, timezone, recurrence_rule, availability,
  visibility) onto bifrost `EventCreate`. Fidelity notes baked into the
  doc-comment: `status` = `Confirmed` (create form has no status field),
  `organizer` = `None`, `attendees` = empty (the current create form collects
  none - a fidelity ceiling inherited from today's `input_to_json`, which also
  sent none; NOT a regression), `recurrence` = `EventRecurrence { rrule:
  input.recurrence_rule.clone(), ..default() }`. `calendar_id =
  CalendarId(calendar_remote_id.into())`.
- `input_to_event_patch(input: &CalendarEventInput) -> EventPatch`: every scalar
  field the update form edits becomes `Some(..)`; unedited-by-form fields
  (`attendees`, `status`) stay `None` (patch semantics - don't clobber). `title`
  etc. are `Option<Option<String>>` in `EventPatch`; a non-empty edited string ->
  `Some(Some(s))`. Because the current update form always sends all scalar fields
  (it is a full-form save, not a partial patch), map all of title/description/
  location/start/end/is_all_day/availability/visibility/recurrence to `Some(..)`;
  leave `calendar_id: None` (calendar moves are not an update-form operation) and
  `attendees: None`.

`strip_event_id(provider, &EventId) -> String` (create write-back): factor the
`rsplit_once("::")` rule already inside `idmap::native_event_id`
(`idmap.rs:58-66`) into a public helper the create path uses to store the native
remote id after `event_create` returns a composite EventId.

### 4.4 Retiring the etag writeback contract (design decision, stated per clause 2)

Legacy ratatoskr threaded `etag` through the write path for optimistic
concurrency: `dispatch_update`/`_delete` took `etag: Option<&str>` and the
CalDAV/Graph impls sent `If-Match`. Bifrost's `Account::event_update` /
`event_delete` take NO etag argument - concurrency is the Account impl's
responsibility, advertised via `capabilities().mutation.concurrency =
StateBased` (`bifrost-types` `capabilities.rs:64-72`). So B7b DELETES etag
threading from the write path entirely:

- The `etag` column on `calendar_events` STAYS - it is populated by the READ
  sync (`idmap::to_event_row` sets `etag: event.etag.clone()`, `sync.rs`
  upserts it) and read by the view layer.
- Writes no longer CONSUME the local etag (no `If-Match` computed consumer-side)
  and no longer PRODUCE one (bifrost returns `()` from update, `EventId` from
  create - neither carries an etag). The `set_calendar_event_remote_id_and_etag`
  / `update_calendar_event_fields_and_etag` calls pass `etag: None`; a follow-up
  read sync refreshes the true server etag.
- `event_id_for_writeback`'s `_etag` parameter (already `_`-prefixed by B7a) stays
  unused, confirming B7a anticipated this. Do not add a caller that populates it.

This is a genuine simplification - but the "concurrency moves into bifrost where
`MutationConcurrency::StateBased` is enforced per provider" rationale is FACTUALLY
TOO BROAD as first written (R2 finding 4). The capability flag is per-account and
is NOT `StateBased` everywhere: Gmail advertises `MutationConcurrency::None`
(bifrost `google/src/account/capabilities.rs:18`), and bifrost's CalDAV
`event_delete` sends NO `If-Match` (`caldav/src/account.rs:807-815`) even though
its `event_update` does (`:799`). The legacy ratatoskr CalDAV path, by contrast,
did an `If-Match`-conditioned delete with a precondition-failed refetch-and-retry
(`caldav/mod.rs:169-188`). So for at least Gmail (all methods) and CalDAV delete,
dropping consumer-side etag is an INTENTIONAL DOWNGRADE to last-write-wins, not a
lossless hand-off to a stronger bifrost guarantee.

State it honestly rather than over-claim:

- The accurate framing is: concurrency handling becomes the Account impl's
  responsibility, advertised per-account via `capabilities().mutation.concurrency`.
  Where a provider is `StateBased` (e.g. Graph/JMAP update) bifrost enforces it;
  where it is `None` (Gmail) or where the specific method skips `If-Match`
  (bifrost CalDAV delete) the effective semantics are last-write-wins.
- Losing the legacy CalDAV-delete `If-Match` + retry is the one concrete
  regression. It is accepted for B7b (single-user, user-initiated deletes; the
  window is tiny and the next read sync reconciles) but must be NAMED in the
  landing commit as a deliberate last-write-wins downgrade for CalDAV delete, not
  silently dropped. If a reviewer deems it unacceptable, the fix belongs in
  bifrost's CalDAV `event_delete` (add the conditional there), not in a revived
  consumer-side If-Match.

Documented here so a reviewer does not read the dropped etag as either a lost
feature OR an over-sold "it all moves to bifrost" win.

### 4.5 Capability gating (mirrors the read path)

Before each provider dispatch, read the opened account's flag:

- create -> `capabilities().pim_methods.event_create`
- update -> `.event_update`
- delete -> `.event_delete`
- rsvp -> `.event_rsvp`

A `false` flag short-circuits WITHOUT calling the provider: create ->
`LocalOnly { reason: "account's calendar backend does not support event
creation", retryable: false }`; update/delete/rsvp -> `Failed { error:
not_supported(..) }`. This matches `sync.rs:78`'s `calendars_list` /
`events_in_range` gate and means an IMAP-only account (factory `Ok(None)`) and a
calendar backend that lists but cannot write both degrade cleanly rather than
erroring at the wire.

### 4.6 RSVP + iMIP (B7b-3)

**Domain action.** New `cal::actions::rsvp_calendar_event(ctx, account_id,
event_id, status: RsvpStatus) -> ActionOutcome`:
- `MutationLog::begin("rsvp_calendar_event", account_id, event_id)`.
- `lookup_event_meta` for the event's own `account_id` + `remote_event_id` +
  `calendar_id` (same authoritative-account rule as update/delete). No
  `remote_event_id` -> `Failed { not_found("cannot RSVP to an unsynced event") }`
  (an RSVP has no local-only meaning).
- Open account (by the event's account), gate on `pim_methods.event_rsvp` (false
  -> `Failed`).
- `account.event_rsvp(idmap::event_id_for_writeback(provider, remote_event_id,
  None, calendar_remote_id).0, status)`.
- On `Ok`: write `rsvp_status` locally (a targeted
  `UPDATE calendar_events SET rsvp_status = ?1 WHERE id = ?2` via a new
  `set_calendar_event_rsvp_sync` query in
  `db/.../queries_extra/calendars/crud.rs`, using `idmap::rsvp(status)` for the
  string form). `Success`. On `Err`: `Failed`.

**Response-delivery contract (R2 finding 2, pinned).** bifrost's trait is
`event_rsvp(EventId, RsvpStatus)` (`bifrost types/src/account.rs:686`) - it CANNOT
carry a "notify the organizer" choice, and providers do NOT uniformly send the
reply: Graph explicitly posts `send_response: false`
(`bifrost graph/src/account/calendar.rs:229`). The § 4.6 prose that claimed
"Graph/Google send the attendee reply" is therefore wrong and is corrected below.
The calendar UI contract (`problem-statement.md:283`) wants an optional "Email
organizer" checkbox above the RSVP buttons - a choice this wire op cannot
represent. B7b pins ONE contract: RSVP updates the user's own participation
status ONLY and does NOT email the organizer (accepting bifrost's current
`send_response: false` Graph behavior as the uniform semantics). The "Email
organizer" checkbox is OUT of B7b scope and named in § 5; wiring it needs a
bifrost trait extension (a delivery flag on `event_rsvp`) plus a matching wire
field, which B7b does not undertake. State this in the app affordance so the
checkbox is either absent or disabled-with-tooltip, not silently ignored.

**Wire + Service.** Add `WireCalendarOperation::RsvpEvent { event_id: String,
response: RsvpResponse }` (`cal_action.rs`), where `RsvpResponse` is a TYPED wire
enum (`accepted` / `declined` / `tentative`), NOT a bare `String` (R2 finding 2):
a free `String` mapped through a defaulting `parse_rsvp` would silently rewrite
malformed input to `NeedsAction`, changing the requested operation. If a string
must be kept for wire-shape symmetry, `run_one` parses it with a FALLIBLE parser
and resolves an unrecognized value to `Failed { invalid_argument(..) }`, never to
a silent `NeedsAction` default. `run_one` gains the arm;
`outcome_to_wire` already handles `Success`/`Failed` (RSVP never yields
`LocalOnly`). `handlers/cal_action.rs`'s validation match gains the variant.
`app/src/harness/mod.rs:3731` gains an `rsvp_event` parse arm.

**App affordance.** The event-detail / reading-pane invite surface
(`app/src/ui/calendar/event_detail.rs`, and the meeting-invite reading pane) gets
Accept / Decline / Tentative controls that build a one-op `CalendarActionPlan`
with `RsvpEvent` and dispatch through the existing `execute_plan` client path
(`service_client.rs:1144`). Read `UI.md` before this brick. The completion path
reuses the existing `pending_calendar_action_plans` latch (no new IPC).

**iMIP scope (resolving the parked question, § 7 of the TODO).** The
email-embedded ICS path (`common/email_parsing.rs`) today extracts ONLY the MIME
`method` (`extract_imip_method`, REQUEST/REPLY/CANCEL); there is NO ICS `UID`
parser, and every hydration path stores `meeting_invite_uid: None`
(`service/src/bifrost/consumer/hydrate.rs:756`). So the UID-resolution path the
spec relied on does not exist yet (R2 finding 1). B7b must either add the missing
prerequisite or scope the reading-pane path out; it adds the prerequisite:

- UID-CAPTURE BRICK (new, B7b-3): during hydration, parse the ICS payload's
  `UID` and persist it into `messages.meeting_invite_uid` (the column and its
  partial index `idx_messages_invite_uid` on `(account_id, meeting_invite_uid)`
  already exist - `db/.../schema/02_mail.sql:271`). This requires the ICS bytes
  at hydrate time; if they are not already in hand, define an on-demand
  attachment-fetch for the `text/calendar` part rather than assuming they are
  present. Without this brick the reading-pane RSVP has no UID to resolve.
- The resolution lookup is `(account_id, uid)`-scoped against the calendar cache
  (`calendar_events.uid`) with explicit ambiguity handling (multiple synced
  events for one UID -> treat as a miss / disable, do not guess).

B7b wires the reading-pane invite's Accept/Decline to `event_rsvp` FOR AN INVITE
THAT RESOLVES TO A SYNCED CALENDAR EVENT: on a hit it dispatches
`RsvpEvent { event_id }` exactly like the detail-pane RSVP. On a miss (invite for
an event not yet in any synced calendar) the reading-pane RSVP is DISABLED with a
"sync this calendar to respond" affordance - emitting a STANDALONE iMIP REPLY
email for an unsynced invite is an email-SEND concern (it composes a
`multipart` REPLY and routes through the mail send path), explicitly OUT of B7b
scope and named as belonging to the mail-send surface (B4/B5), not calendar
actions. This is a scope boundary, not deferral: the TODO's "routes through
`event_rsvp`" is precisely the cache-backed path, and `event_rsvp` requires an
`EventId` that only a synced event has. Provider-side REPLY emission is
whatever each bifrost `event_rsvp` impl does - and that is NOT uniform (Graph
posts `send_response: false`, § 4.6 delivery contract); ratatoskr does not
hand-roll the REPLY and, per the delivery contract, does not request organizer
notification in B7b.

## 5. Stopping rule (clause 9)

B7b's blast radius ends at the calendar WRITE path. Explicitly OUT of scope:

- The calendar READ sync (`sync.rs`) - B7a owns it; B7b does not touch
  `calendar_sync_account_impl` or the reconcile.
- The stale-calendar reap-vs-hide lifecycle - that is B7c
  (`calendars.unlisted_since`), a separate TODO with its own schema change. B7b
  keeps B7a's retain-and-skip policy untouched.
- Standalone iMIP REPLY email emission for an UNSYNCED invite - a mail-send
  concern (§ 4.6), named and excluded, not deferred B7b work.
- The RSVP "Email organizer" checkbox (`problem-statement.md:283`) - OUT of B7b
  (§ 4.6 delivery contract). It needs a bifrost `event_rsvp` trait extension
  (a delivery flag) plus a wire field; B7b pins the no-notify contract and does
  not add the choice. Named, not silently dropped.
- The CalDAV-delete `If-Match` guard (§ 4.4) - INTENTIONALLY dropped to
  last-write-wins because bifrost's CalDAV `event_delete` does not send
  `If-Match`. Accepted for B7b and named in the landing commit; a stronger guard,
  if wanted, belongs in bifrost, not a revived consumer-side mechanism.

In scope and NOT deferred (added by the R1/R2 reconciliation): the app-side
all-day exclusive-end normalization (§ 4.2a) and the ICS `UID`-capture brick that
populates `messages.meeting_invite_uid` (§ 4.6) - both are prerequisites the
original draft assumed existed and are now first-class B7b work.
- The `CreateEvent.calendar_remote_id` naming rename (§ 2.5) - logged as a
  follow-up, behavior preserved.
- Calendar event ATTENDEE editing on create/update - the current form collects no
  attendees; B7b maps `attendees: empty/None` (§ 4.3), preserving today's
  fidelity ceiling. Adding attendee editing is a UI feature, not this rewire.
- Series-vs-occurrence update semantics (edit-this-vs-all) - not modeled today;
  B7b sends the master patch as the form does now. Out of scope.

Nothing named in the B7b TODO bullet is deferred: create/update/delete are
rewired (§ 4.2), RSVP is added (§ 4.6), the `CalendarProvider` enum is deleted
(§ 2.2), the write helpers are deleted (§ 2.3), and the B7a seam is reused
(§ 2.1).

## 6. Verification per brick (clause 5)

Exact commands. The calendar write path is a provider-IO boundary, so the primary
gates are per-provider sync-harness action-writeback scripts against
`saehrimnir`, backed by idmap unit tests. Read `reference/glossary/harness.md`
first.

### 6.1 B7b-1 (idmap write direction) gates

Deterministic, unit-pinnable:

- `brokkr test -p cal idmap` - runs the new inverse-mapper and round-trip tests
  alongside the existing forward tests. New cases required:
  `idmap_epoch_to_event_time_roundtrips_all_day`,
  `idmap_epoch_to_event_time_roundtrips_zoned_and_floating`,
  `idmap_parse_availability_visibility_rsvp_inverse` (no `parse_status` - dropped
  as dead code, R1 finding 3),
  `idmap_input_to_event_create_maps_recurrence_and_defaults`,
  `idmap_input_to_event_patch_sends_all_form_fields`,
  `idmap_strip_event_id_reverses_composite_for_gmail_graph`, and a first-caller
  test for `event_id_for_writeback` (`idmap_event_id_for_writeback_*`).

### 6.2 B7b-2 (create/update/delete rewire) gates

Behavioral, per provider - a compile-only replacement must fail. Each script
lists the calendar, creates/updates/deletes an event through the action pipeline,
and asserts the mutation reached the mock provider AND round-trips back on the
next read sync. Model on `gmail-action-writeback.lua` /
`graph-action-writeback.lua` (mail writeback) and the existing
`*-calendar-*.lua` read scripts. NEW scripts to author (a script is a brick of
this spec, § clause 5 - authored before the gate it backs):

- `crates/app/tests/sync-harness/gcal-calendar-action-writeback.lua`
- `crates/app/tests/sync-harness/graph-calendar-action-writeback.lua`
- `crates/app/tests/sync-harness/jmap-calendar-action-writeback.lua`
- `crates/app/tests/sync-harness/caldav-calendar-action-writeback.lua`

Run each:
- `brokkr service-test crates/app/tests/sync-harness/gcal-calendar-action-writeback.lua`
- `brokkr service-test crates/app/tests/sync-harness/graph-calendar-action-writeback.lua`
- `brokkr service-test crates/app/tests/sync-harness/jmap-calendar-action-writeback.lua`
- `brokkr service-test crates/app/tests/sync-harness/caldav-calendar-action-writeback.lua`

Each script MUST assert: (a) create -> event exists on the mock with the sent
fields, local row gains the provider remote id, AND a follow-up read sync leaves
exactly ONE local row for that event - no transient duplicate (the id-stability
requirement of § 4.2; this is the one place to assert cross-seam id identity
rather than assume it, R1 finding 2, and it matters most for CalDAV where the
create-time id and the read-sync dedup key are constructed differently); (b)
update -> mock reflects the edited fields, INCLUDING an all-day create/update
whose mock range is a one-day exclusive span, not a zero-day span (R2 finding 3);
(c) delete -> event gone from the mock and from the local cache; (d) the
capability-gated no-op (point a mutation at an IMAP-only or write-incapable
account and assert `LocalOnly`/`Failed` without a provider call).

Plus the Service-side unit gate for the outcome mapping and the opener wiring:
- `brokkr test -p service cal_actions` (covers `outcome_to_wire`, the new
  `RsvpEvent` `run_one` arm once B7b-3 lands, and the factory `Ok(None)` no-op).

Plus the app-side all-day lowering gate (R2 finding 3, § 4.2a):
- `brokkr test -p app build_wire_input` (or the enclosing test name) - asserts a
  single-day all-day save lowers to a one-day exclusive span and a multi-day
  all-day save preserves the exclusive end. This is an `app`-crate test because
  the normalization lives in `build_wire_input`, not `idmap`.

And the universal gate: `brokkr check`.

### 6.3 B7b-3 (RSVP + iMIP) gates

- Extend each of the four B7b-2 scripts (or add `*-calendar-rsvp-writeback.lua`
  siblings) to: sync an event with the authed user as an attendee, RSVP
  Accept/Decline via the `RsvpEvent` op, assert the mock recorded the
  participation change, and assert the local `rsvp_status` column updated. Run
  via the same `brokkr service-test <script>` invocations.
- `brokkr test -p cal rsvp` - a unit test for `rsvp_calendar_event`'s
  unsynced-event `Failed` path and the `parse_rsvp` mapping.
- iMIP tests, split by where the code actually lives (R2 finding 1 - a single
  `-p common` gate cannot cover an `app`-side helper):
  - `brokkr test -p common imip` covers the ICS `UID` PARSER added to
    `email_parsing` (§ 4.6 UID-capture brick) and `extract_imip_method`.
  - the UID-RESOLUTION helper (enable-vs-disable of the reading-pane RSVP,
    `(account_id, uid)`-scoped with ambiguity handling) lives in `app`, so its
    test is `brokkr test -p app` - not `-p common`.
  - hydration persistence of `meeting_invite_uid` is covered by a
    `brokkr test -p service` case (hydrate populates the column, not `None`).
- `brokkr check`.

### 6.4 Performance note (clause 5 / clause 10)

Calendar actions are user-initiated single mutations, not a sync hot path, and
the `cal_action.execute_plan` request already carries a 5-second finite timeout
(`request.rs:1540`). But the standing contract
(`reference/technical-implementation-spec.md:84-90`) is explicit that a spec
touching a "sync, provider, storage, or Service hot path" owes a recorded
`brokkr sync-bench` gate (elapsed, provider-request count, peak RSS), and this is
a PROVIDER path - so a bare "no perf instrument applies" waiver conflicts with the
contract (R2 finding 6). A 5-second IPC timeout is not a request-count or latency
budget, and opening an `Account` per op can perform provider discovery plus extra
reads, which is a measurable regression surface (e.g. an extra `event_get` per
create versus the legacy typed clients).

Pinned resolution: B7b-2's harness scripts MUST pin the per-op PROVIDER-REQUEST
COUNT for create/update/delete (assert the exact request sequence the mock
received) as the minimum contract-satisfying instrument. This turns "no batch, no
per-request budget" into a measured, regression-gated fact rather than an
assumption. If the pinned counts show the per-op `Account`-open adds provider
round-trips over the legacy path, record a `brokkr sync-bench` baseline as the
contract prefers; the decision is made against measured harness output, not
waived here.

## 7. Stance (structural over micro)

B7b is a full rewrite of the calendar write dispatch, not a local tweak: the
four-way provider enum is deleted, four provider write modules and a JMAP write
module are removed, the consumer-side etag/If-Match concurrency mechanism is
retired (delegated to each bifrost Account's advertised concurrency, with the
Gmail/CalDAV-delete last-write-wins downgrade named honestly in § 4.4, not
papered over), and the write path is unified onto one call site per operation. The result has no env switch, no transitional adapter, no
per-provider residue - the § 3 split stages the cutover through ordering
(dormant idmap addition, atomic enum+helper deletion, additive RSVP), never
through a runtime flag. Cleanliness is the deliverable: after B7b, "how does
ratatoskr write a calendar event?" has exactly one answer.

## 8. Open items reconciled into the spec (no deferral holes)

- **etag drop** (§ 4.4): mostly a maximal-integration win, but NOT a lossless
  hand-off - Gmail is `MutationConcurrency::None` and bifrost CalDAV delete sends
  no `If-Match`, so for those methods it is an intentional last-write-wins
  downgrade, named in the landing commit (corrected per R2 finding 4). Not
  deferral.
- **Provider kind sourcing** (§ 4.1/§ 4.2): resolved by having the injected
  opener return `(Arc<dyn Account>, ProtocolKind)`, removing a fragile string
  re-parse - and the opener signature is now pinned to that tuple, fixing the
  earlier Option-vs-tuple contradiction (R1 finding 1, R2 finding 5).
- **iMIP unsynced-invite reply** (§ 4.6, § 5): named as a mail-send concern and
  excluded with a rationale, not silently dropped. The synced-invite path now
  carries its missing prerequisite (ICS `UID` capture, § 4.6) as explicit work
  (R2 finding 1).
- **CalendarEventDto / jmap calendar_sync module disposition** (§ 2.3): resolved
  as survey-time "delete if unreferenced, else narrow" determinations with the
  exact grep to run, not left to implementation discovery.
- **Naming smell** (§ 2.5): behavior preserved, rename logged as a follow-up.

## 9. Review reconciliation (R1 Opus + R2 codex xhigh)

Both review reports were validated finding-by-finding against the actual
ratatoskr and bifrost source. All findings held up; each is folded into the
section named below. No finding was rejected as wrong. Two R1 minors were pure
confirmations (no defect, no edit) and are logged as such.

Folded findings:

- R1-1 / R2-5 (opener seam under-specified + self-contradictory) -> § 4.1: pinned
  owned `Arc<dyn CalendarAccountOpener>` on `CalendarActionContext`, the
  `action-types -> bifrost-types` new dep it forces, and the tuple return that
  reconciles with § 4.2.
- R1-2 (create-path id stability for JMAP/CalDAV) -> § 4.2 + § 6.2 explicit
  no-duplicate harness assertion.
- R1-3 (`parse_status` dead code) -> § 4.3 dropped, § 6.1 test dropped.
- R1-4 (crate-boundary statement backwards) -> Required-reading bullet corrected
  (`cal` depends on `core`; invariant is `core` must not depend on `cal`).
- R1-5 (wire-frame anticipation overclaim) -> § 2.4 corrected (adding `RsvpEvent`
  is a genuine new enum variant; the quoted doc is about N-op plans on a
  different type).
- R1-6 (provider routing not byte-identical) -> § 2.1 caveat: B7b adopts the
  factory's richer rule; the `provider == "caldav"` + mail `calendar_provider`
  edge case changes behavior.
- R2-1 (iMIP UID resolution path does not exist) -> § 4.6 UID-capture brick +
  `(account_id, uid)` scoping/ambiguity; § 6.3 test-location split fixed.
- R2-2 (RSVP delivery semantics contradict bifrost + UI contract) -> § 4.6
  delivery contract pinned (no organizer email in B7b; typed/fallible response,
  no silent `NeedsAction`); § 5 lists the "Email organizer" checkbox as OUT.
- R2-3 (all-day lowering produces a zero-day event) -> § 4.2a normalization brick
  + § 6.2 app + harness assertions.
- R2-4 (etag retirement rationale too broad) -> § 4.4 rewritten to name the
  Gmail/CalDAV-delete last-write-wins downgrade honestly.
- R2-6 (perf waiver conflicts with the standing contract) -> § 6.4: harness now
  MUST pin provider-request counts, with a sync-bench baseline if counts regress.

Confirmations (no defect, no edit): R1's `_etag`-anticipated-by-B7a note (verified
- `event_id_for_writeback` takes `_etag`, zero callers) and R1's "option (a)
read-the-DB-column is equally valid" endorsement of the `ProtocolKind` sourcing
choice.

Lateral finding (outside both reports): the spec file previously ended with a
stray `</content></invoke>` fragment (a generation artifact); removed during this
reconciliation.
