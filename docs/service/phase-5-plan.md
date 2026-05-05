# The Service - Phase 5 Plan: port sync to other providers + calendar + GAL relocation

Companion to `phase-1-plan.md`, `phase-1.5-plan.md`, `phase-2-plan.md`, `phase-3-plan.md`, `phase-4-plan.md`. Implements Phase 5 of `implementation-roadmap.md`.

## Revision history

**2026-05-05 - post-review revision (arch+bugs sweep).** Initial draft was reviewed by arch and bugs archetypes (claude + codex each, four sessions total). Consolidated changes applied:

- **Dependency cycle prerequisite added.** `service -> cal -> rtsk -> service` cycle blocks any direct `service` use of `cal::sync`. New § "Prerequisite: break the rtsk->service shim" makes this an explicit gate before calendar work starts.
- **Calendar cancellation re-scoped.** Initial draft claimed `calendar_sync_account_impl` already takes a `CancellationToken`. It doesn't - neither do the three provider paths (`sync_google_calendar_account`, `sync_graph_calendar_account`, `sync_caldav_calendar_account`) nor the per-calendar event loops. Threading cancellation through the calendar stack is now task 3a (its own focused commit), distinct from `CalendarRuntime` construction (task 3b).
- **Cadence resolved.** Plan now keeps the 5-min `SyncTick` cadence and adds a Service-side per-account `last_calendar_sync` staleness gate (1h). GAL self-gates via the existing 24h cache. Dead `Message::GalRefreshTick` (no-op placeholder) is deleted as part of the SyncTick work.
- **`CalendarCompleted` routing split into two notifications.** `CalendarRunCompleted` (`MustDeliver`, consumed inside `ServiceClient` by per-`run_id` awaiters, mirrors `SyncCompleted`) + `CalendarChanged` (`Coalesce`, dispatched to UI for view reload, debounced UI-side). Single-notification design conflated awaited-by-caller with dispatched-to-UI - same shape as `SyncCompleted` would have meant the UI reload path never fires.
- **`WriteDbState` for calendar at relocation.** Calendar today writes via `ReadDbState::with_conn` - a write-surface escape. Phase 5 fixes this during relocation by changing `crates/calendar/src/sync.rs` signatures, rather than silently moving the escape into the Service. (Mirrors the Phase 2 lesson Phase 4 had to clean up.)
- **Account-deletion path added.** `calendar.cancel_account` wire method + integration into `client.cancel_and_await` so calendar runners are torn down alongside email sync and push when an account is deleted.
- **Drain rationale rewritten.** Original "calendar before sync because RSVP send needs the action worker" rationale was forward-looking but described as load-bearing. Action worker is alive throughout the consolidated drain anyway. Ordering is now described as "reserved for future calendar->action paths," not as a current dependency. Code-comment text updated to match.
- **IMAP cancellation: stateful-session caveat documented.** `tokio::select!` mid-FETCH leaves the IMAP session with unread response data, which breaks the next command. Plan now picks **point-checks between RPCs** (matches JMAP's actual pattern, gives folder-boundary cancellation) over `select!` + session teardown. Scope expanded to include `imap_delta_janitor`, `client::sync` helpers, and `batch_delta_check`.
- **GAL serialization made required.** Notification dispatcher runs handlers concurrently (`NOTIFY_CAP = 4`); two stale-account kicks back-to-back can duplicate provider calls. Plan now requires a Tokio `Mutex` on the handler entry plus a per-account in-flight set, with a unit test. Notification-drain bound also added so a wedged GAL refresh can't stall shutdown.
- **Factual errors fixed.** `SyncRuntime` does not have `closed: AtomicBool` (that's `PushRuntimeInner`). `boot_state.write_db_state()` doesn't exist - API is `db_conn()` + `encryption_key()`. `CalendarRuntime::start_account` now returns `Result<CalendarStartAck, String>` (test plan expected `Err` after shutdown; original signature returned the ack directly).
- **"Only remaining UI write surface" claim corrected.** Calendar event mutations (`cal::actions::*` from `handlers/calendar.rs`) still run UI-side. Phase 5 relocates only periodic provider sync/cache writes; event-mutation relocation is Phase 6. Documented as a current exception.
- **Phase 9 tray-resident TODO marker added** so a future Service-side scheduler doesn't have to retro-fit cadence ownership.
- **Phrasing: "four IPC kicks"** corrected to "three notifications + one request fan-out."

**2026-05-05 - initial draft.** Phase 5's roadmap entry framed the work as "do for Gmail / Graph / IMAP what Phase 3 did for JMAP." Most of that cascaded for free during Phase 3: the `ProviderOps::sync_delta` trait already abstracts over all four providers; `service::actions::provider::create_provider` already dispatches Gmail/Graph/JMAP/IMAP; `service::sync_dispatch::sync_delta_for_account` already drives any of them through `SyncRuntime::run_sync`. So the *email-sync-relocation* part of Phase 5 is mostly already done as a side-effect of Phase 3.

What's actually still left, scoped here:

- **IMAP cancellation depth.** Phase 3 added fine-grained checkpoints to JMAP's sync path (per-mailbox, per-batch, network calls, token refresh, principal resolution, share-notification polling). Gmail and Graph also accept the cancellation token through `SyncProviderCtx` and check it at sensible boundaries. IMAP's `imap_initial.rs:67` and `imap_delta.rs:52` accept the token, check it once at the entry, then explicitly suppress the unused-variable lint with `let _cancellation_token = cancellation_token;`. The token is dropped, never threaded into the per-folder loop. A user pressing "cancel sync" mid-IMAP-fetch waits for the entire sync to complete. Phase 3 incomplete port; Phase 5 closes.

- **Calendar sync still runs UI-side.** `crates/app/src/handlers/provider.rs::sync_calendars` calls `cal::sync::calendar_sync_account` directly on the UI's `db.write_db_state()` connection on every `Message::SyncTick`. This is the only remaining UI write surface that bypasses the Service entirely (action mutations, sync, push are all Service-side post-Phase-4). Calendar gets a separate `CalendarRuntime` rather than folding into `SyncRuntime` because cadence and concurrency policies differ - email sync runs on a 5-min UI tick (or push), calendar runs on a 1-hour tick.

- **GAL (Global Address List) cache refresh still runs UI-side.** Same shape as calendar: `refresh_gal_caches` at `handlers/provider.rs` runs on a 1-hour UI tick. GAL is fire-and-forget per account, idempotent, no cancellation needed; it gets a notification-driven `gal.kick` IPC mirroring `pending_ops.kick`, no per-account runtime.

- **`Message::SyncTick`'s remaining UI-side branches.** Currently fans out to `sync_all_accounts` (IPC, post-Phase-3), `process_pending_ops` (IPC, post-Phase-2), `refresh_gal_caches` (UI-side, this phase), `sync_calendars` (UI-side, this phase). After Phase 5, all four are IPC kicks. UI's `SyncTick` becomes a pure cadence trigger with no provider work on it.

Strategic decisions that this plan locks in (and that the code comments must mirror):

- **One `SyncRuntime`, four providers, no per-provider runtime split.** `SyncRuntime` is provider-agnostic via `ProviderOps`; the dispatch already works. No `GmailSyncRuntime` etc. Per-provider concurrency policies live inside the provider's own `sync_delta` impl (Gmail/Graph batch size, IMAP per-folder session reuse), not at the runtime layer.

- **Calendar gets a separate `CalendarRuntime` and separate IPC method.** Mirrors `SyncRuntime`'s shape (per-account map, panic supervisor, cancellation token, lifecycle hooks) but with simpler invariants - calendar sync is idempotent (CalDAV CTags / Exchange ETags) and doesn't write to the four-store cluster, so no marker-file lifecycle, no invariant-pass entry. New `calendar.start_account_sync` request method (5 s ack timeout, fire-and-forget runner) and new `calendar.completed { account_id, run_id, result }` notification. Drain order: PushRuntime → CalendarRuntime → SyncRuntime → search-writer → sentinel. Calendar drains *before* Sync because the calendar runner can call into action paths (RSVP send), which thread through the action worker; the action worker shutdown is already part of the consolidated drain.

- **GAL: notification-driven `gal.kick` IPC, no per-account runtime, no cancellation.** GAL refresh is sub-second per account in the steady state (24h cache) and bounded by network round-trips for stale accounts. Service handler iterates accounts, calls existing `refresh_gal_for_account`, returns. Per-account concurrency is not load-bearing at this scale (GAL only fires hourly).

- **IMAP cancellation depth: per-folder loop checkpoint minimum.** Match Phase 3's JMAP coverage at the granularity that matters for IMAP: each folder fetch is a network round-trip; the per-folder loop is the natural break. The `let _cancellation_token = cancellation_token;` pattern is the marker for what to fix.

- **`Message::SyncTick` collapses to three notifications + one request fan-out.** No more UI-side provider work. `sync_calendars` and `refresh_gal_caches` methods on `ReadyApp` (in `handlers/provider.rs`) get deleted entirely; their callers become IPC notification sends. **Two distinct surfaces with distinct semantics** (do not blur):
  - **Cadence-driven kicks** (`calendar.kick`, `gal.kick`) are `ClientNotification`s, fire-and-forget, no per-account targeting. Service-side handler iterates accounts and gates on staleness (calendar: per-account `last_calendar_sync` > 1h; GAL: existing 24h cache).
  - **Explicit-request paths** (`calendar.start_account_sync`) are typed requests with per-account targeting and per-`run_id` completion awaiting. Used for post-account-add, manual "sync now", RSVP-then-resync.
  
  Cadence stays UI-side on the existing 5-min `SyncTick`; staleness gating is Service-side. The dead `Message::GalRefreshTick` (no-op placeholder at `update.rs:697-705`) is deleted.

## Prerequisite: break the rtsk -> service shim before any calendar work

The `crates/calendar/` (`cal`) crate depends on `rtsk` (`crates/calendar/Cargo.toml:14`), and `rtsk` currently depends on `service` (`crates/core/Cargo.toml:45`, kept in place by the action-shim modules `core::actions::* -> service::actions::*`). Adding the `service -> cal` edge that `CalendarRuntime` needs creates a cycle: `service -> cal -> rtsk -> service`. Cargo will reject this.

Resolution must land *before* task 3 of this phase. Three options:

1. **Break `rtsk -> service`.** The action-shim modules in `rtsk` were a Phase 2 transitional layer; with all action call sites going through the Service in Phase 4 they should be inlinable into the app crate or the service crate itself. This is the right long-term move and aligns with the Phase 6 "global lockdown" direction. Cost: cross-crate refactor of ~12 action shim modules.
2. **Move calendar sync to a lower crate.** Lift `calendar_sync_account_impl` and the three provider paths into a crate that doesn't depend on `rtsk` (e.g. a new `cal-sync` sibling). Cost: a non-trivial split of the calendar crate; carries the risk of code being pulled in two directions before the boundaries settle.
3. **Move the callable surface into `service`.** Inline the calendar-sync entry points directly into `crates/service/src/calendar.rs` so `service` doesn't import `cal` at all (it imports `gmail`, `graph`, `caldav` directly). Cost: duplicates the orchestration logic until the `cal` crate's UI-side users can be migrated.

**Decision:** option 1. The `rtsk -> service` edge is a known transitional debt; Phase 5 is a reasonable point to retire it rather than papering over with another shim. New § "Prerequisite tasks" in the detailed task list captures the refactor. If the refactor turns out to be larger than expected mid-implementation, fall back to option 3 (a brief decoupling note in `service::calendar`).

This prerequisite section explicitly retracts the initial draft's silent assumption that `service` could just import `cal` directly.

## Context

Phase 4 closed the JMAP-specific work (push relocation, drain consolidation, OAuth resolver). Phase 5 finishes the email-sync relocation for the remaining three providers and folds in the two non-email subsystems still on the UI's hot path: calendar and GAL.

Most of the email-sync work was structurally complete after Phase 3. What's left has the same shape as Phase 4's "fix the parts that didn't actually land" - IMAP cancellation specifically. Calendar and GAL are smaller relocations (single function each, no cross-store complexity).

The phase ships as one milestone with a clean commit-level split: IMAP cancellation depth → Calendar IPC + Runtime → GAL kick IPC → UI teardown of `sync_calendars` / `refresh_gal_caches` → SyncTick collapse → docs. A regression should bisect to the right commit.

## Scope

### In scope

- **Prerequisite: retire the `rtsk -> service` shim.** See § "Prerequisite: break the rtsk -> service shim". This is a precondition; calendar work is blocked on it.
- **IMAP cancellation depth.** Add `cancellation_token` argument to the per-folder loop entry points in `crates/imap/src/imap_initial.rs` and `crates/imap/src/imap_delta.rs`. Insert **point-checks** (`if cancellation_token.is_cancelled() { return Cancelled }`) at: folder-list iteration boundary, per-folder fetch entry, per-batch persist entry, between RPCs in helpers (`imap_delta::batch_delta_check`, `imap_delta_janitor`, `client::sync`). **Do not use `tokio::select!` mid-FETCH** - dropping a future mid-FETCH leaves the IMAP session with unread response data on the wire, breaking the next command. Point-checks-between-RPCs gives folder-and-RPC-boundary cancellation, which matches JMAP's actual pattern. The `let _cancellation_token = cancellation_token;` markers indicate the entry points to start from. Mirrors Phase 3 task 6 for IMAP specifically.
- **Calendar cancellation plumbing (task 3a).** Thread `&CancellationToken` through `crates/calendar/src/sync.rs::calendar_sync_account_impl`, the public `calendar_sync_account` wrapper, the three provider paths (`sync_google_calendar_account`, `sync_graph_calendar_account`, `sync_caldav_calendar_account`), and the per-calendar event-sync loops. Same shape as the IMAP work: point-checks at calendar-list-entry, per-calendar-entry, per-event-batch boundaries. Without this, `CalendarRuntime::cancel_account` is a stub.
- **Calendar `WriteDbState` migration.** `calendar_sync_account_impl` today takes `&ReadDbState` and writes through `ReadDbState::with_conn` (a write-surface escape). During relocation, change the signature to `&WriteDbState` so the Service constructs the write half explicitly. Don't move the escape into the Service. Sub-task of 3a; lands together so the calendar crate compiles.
- **`service-api` calendar wire types.** New `crates/service-api/src/calendar.rs` with:
  - `CalendarRunId(uuid::Uuid)` (`new_v7`).
  - `CalendarStartAccountSyncParams { account_id }`, `CalendarStartAck { account_id, run_id, already_in_flight }`.
  - `CalendarCancelAccountSyncParams { account_id }` (the account-deletion path - see § "Account-deletion integration").
  - `CalendarSyncResult { Completed | Cancelled | Failed(String) }`.
  - **Two notifications**, mirroring the dual-routing decision in § Architecture:
    - `Notification::CalendarRunCompleted { account_id, run_id, result, service_generation }` - class `MustDeliver`. Consumed inside `ServiceClient` by per-`run_id` awaiters (mirrors `SyncCompleted`). Not enqueued to the UI.
    - `Notification::CalendarChanged { account_id, service_generation }` - class `Coalesce`. Dispatched to UI for view reload; debounced UI-side.
  - `RequestParams::CalendarStartAccountSync` (5 s timeout) and `RequestParams::CalendarCancelAccountSync` (5 s timeout).
  - `ClientNotification::CalendarKick` (class `Drop`) and `ClientNotification::GalKick` (class `Drop`). Following the `pending_ops.kick` shape.
- **`crates/service/src/calendar.rs`: `CalendarRuntime`.** New file. Per-account map keyed by `account_id`; panic supervisor wrapping each runner; `closed: AtomicBool` shutdown guard mirroring `PushRuntimeInner` (NOT `SyncRuntime` - that crate doesn't have the flag); `start_account -> Result<CalendarStartAck, String>` (`Err` on post-shutdown calls); `cancel_account -> bool`; `shutdown`. No marker-file lifecycle - calendar sync is idempotent against provider state (CTags / ETags) and doesn't touch the four-store cluster, so no invariant-pass entry. Mirrors `SyncRuntime`'s API surface where it makes sense; diverges where invariants differ (pin in the type doc-comment).
- **`crates/service/src/handlers/calendar.rs`: handlers.** `handle_start_account_sync` for the request; `handle_cancel_account_sync` for the cancel request; `handle_calendar_kick` for the notification. The kick handler enumerates accounts whose calendar sync is stale (per a Service-side `last_calendar_sync` per-account timestamp; staleness threshold 1h) and spawns runners.
- **`crates/service/src/handlers/gal.rs`: handler.** `handle_gal_kick` notification handler. Iterates **all** accounts (`refresh_gal_for_account` already returns `Ok(0)` for unsupported providers - no `enumerate_supported_accounts` helper needed) and calls `rtsk::contacts::gal::refresh_gal_for_account` per account with the existing 60 s per-account timeout. **Required: serialize handler invocations** via a Tokio `Mutex` on a per-handler shared state (the notification dispatcher runs handlers concurrently with `NOTIFY_CAP = 4`; two stale-account kicks back-to-back without serialization will duplicate provider calls). No per-account runtime.
- **Notification-drain bound.** The current `drain_in_flight(&notifications_in_flight)` in `dispatch.rs` awaits unbounded - a wedged GAL handler can stall shutdown by up to N×60s. Add a hard cap (proposed: 5s aggregate) past which the drain logs a warning and aborts the remaining notification tasks. Phase 4 added a similar `stop_push` ceiling for the same class of problem.
- **Account-deletion integration.** Add `calendar.cancel_account` plumbing into the existing `client.cancel_and_await` path used by account delete (`crates/app/src/handlers/core.rs:686`). Today that path cancels email sync and push only; calendar runner can race the delete (calendar tables CASCADE from `accounts` per `crates/db/src/db/schema/05_calendar.sql:3`). Mirrors how `sync.cancel_account` is wired today.
- **Calendar runtime drain in the consolidated drain.** Order: `PushRuntime -> CalendarRuntime -> SyncRuntime -> search-writer -> sentinel`. Ordering is **reserved**, not currently load-bearing - see § "Drain ordering: reserved, not load-bearing today" for rationale.
- **`Message::SyncTick` collapse + dead-tick removal.** Replace UI-side `sync_calendars` and `refresh_gal_caches` calls with IPC sends (`ClientNotification::CalendarKick` and `ClientNotification::GalKick`). Cadence stays on the existing 5-min `SyncTick`; staleness gating is Service-side (calendar: per-account `last_calendar_sync` timestamp tracked by the `CalendarRuntime`'s kick handler; GAL: existing 24h cache check in `refresh_gal_for_account`). Delete the dead `Message::GalRefreshTick` placeholder and its `iced::time::every` subscription (`subscription.rs:108-112`, `update.rs:697-705`).
- **UI teardown.** Delete `crates/app/src/handlers/provider.rs::sync_calendars`. Delete `crates/app/src/handlers/provider.rs::refresh_gal_caches`. Replace their call sites in `update.rs::Message::SyncTick`.
- **UI: `Notification::CalendarChanged` arm with debouncing.** UI-side dispatcher routes `CalendarChanged` notifications to a debounced reload (proposed: 250ms trailing-edge) so N accounts completing a kick batch produce one reload, not N. `Notification::CalendarRunCompleted` is consumed inside `ServiceClient` - never reaches the UI dispatcher.
- **Code-comment requirements** mirroring the phase-4 pattern (see § "Code-comment requirements" below).
- **Doc updates** to `problem-statement.md` (new "Phase 5 status (as landed)" block + correction to the "remaining UI write surfaces" inventory: calendar event mutations stay UI-side this phase) and `implementation-roadmap.md` (corrections to the Phase 5 entry's scope claims).

### Out of scope

- **IMAP IDLE.** Lands when IMAP IDLE itself lands in the codebase; will follow Phase 4's `PushRuntime` pattern.
- **Provider-protocol improvements** (CONDSTORE/QRESYNC for IMAP, batch APIs for Graph, etc.). Tracked in their own roadmap docs.
- **Calendar event mutations (`cal::actions::*`).** `crates/app/src/handlers/calendar.rs:309,542` and friends still call `cal::actions::*` directly with a UI-side `ActionContext` (`crates/app/src/app.rs:335`). Phase 5 relocates only the *periodic provider sync/cache refresh*; event-mutation relocation is Phase 6 territory. The "only remaining UI write surface" claim from the initial draft was wrong.
- **GAL fetch over Graph / Google Directory.** `update.rs:697-699` notes that GAL fetch over provider-client APIs (`/users`, Google Directory) is blocked on the sync orchestrator providing account-level clients. That's a structural piece that lives in a future phase, not Phase 5. Phase 5 only relocates the *cache-refresh tick wiring* - if a provider-client orchestration lands later, the `refresh_gal_for_account` body picks it up automatically.
- **Per-account runtime for GAL.** GAL is hourly + idempotent + bounded (60s × N accounts × 24h staleness gate). Phase 5 might revisit if benchmark surfaces a problem.
- **Marker-file lifecycle for `CalendarRuntime`.** Calendar sync is idempotent (CTags / ETags); no four-store invariant pass needed. If a future calendar-attachment-cache surface gets built, the runtime gets the marker treatment then.
- **CalDAV-specific protocol features** (sync-collection REPORT, etc.) - inside the calendar provider; not Phase 5.
- **Service-side cadence ownership (tray-resident operation).** Today the iced `SyncTick` subscription drives all four kicks. Once the app gains a tray-resident mode (Phase 9), refresh stops working when the UI window is closed. The Service knows enough to drive its own cadence and should take it over at that point. **TODO marker for Phase 9:** revisit moving the cadence Service-side (a `tokio::time::interval` in the dispatch loop, gated by per-account staleness) when tray-resident lands. Phase 5 deliberately keeps the cadence UI-side to avoid building a Service-side scheduler that has to be re-designed once tray-resident ships.

## Architecture

### `CalendarRuntime` shape

```text
service/
├── calendar.rs                     ← NEW
│   ├── pub struct CalendarRuntime { entries: Mutex<HashMap<String, CalAccountEntry>>, ... }
│   ├── struct CalAccountEntry { handle: JoinHandle<()>, cancel: CancellationToken, run_id: CalendarRunId }
│   ├── pub async fn start_account(&self, account_id: String) -> CalendarStartAck
│   ├── pub async fn cancel_account(&self, account_id: &str) -> bool
│   └── pub async fn shutdown(&self)
└── handlers/
    ├── calendar.rs                 ← NEW (handle_start_account_sync, handle_calendar_kick)
    └── gal.rs                      ← NEW (handle_gal_kick)
```

Mirrors `crates/service/src/sync.rs::SyncRuntime` for the lifecycle surface (per-account map, panic supervisor, start/cancel/shutdown). The `closed: AtomicBool` shutdown guard mirrors `crates/service/src/push.rs::PushRuntimeInner` (line 109) - `SyncRuntime` itself does not currently have the flag; Phase 4's review-pass fix added it to push only. We add it to `CalendarRuntime` because calendar has a kick-driven entry path (the hourly tick) analogous to push's post-ready iteration: any kick arriving during shutdown must be rejected. Diverges intentionally on:
- **No marker-file lifecycle.** Calendar sync is idempotent against CalDAV CTags / Exchange ETags; the provider re-fetches whatever changed regardless of whether the previous run completed. No `clear_account_history_id`-equivalent needed.
- **No body / inline / search writer halves.** Calendar writes only to the calendar tables in the main DB. Signature change required during relocation: `calendar_sync_account_impl` switches from `&ReadDbState` to `&WriteDbState` so the write surface is explicit. (Today it does writes via `ReadDbState::with_conn`, a write-surface escape that Phase 5 fixes during the move.)
- **`start_account` returns `Result<CalendarStartAck, String>`.** Post-shutdown `start_account` returns `Err` (the unit-testable invariant from the Phase 4 review-pass pattern). Initial draft returned the ack directly, which contradicted its own test plan.
- **`service_generation` on `CalendarRunCompleted`** for cross-respawn safety. The request method's `CalendarStartAck` does not need it (caller-local).

### Cancellation: runtime -> handler -> provider chain

For email sync, Phase 3 set up: `SyncRuntime::cancel_account` flips token -> `sync_delta_for_account` checkpoints observe -> provider returns `Cancelled`. Phase 5 mirrors this for calendar - but the calendar stack today has **no** `CancellationToken` parameter at any layer, so the work is full plumbing, not just adding checkpoints.

**Calendar (task 3a):** Thread `&CancellationToken` through:
- `calendar_sync_account_impl` and the public `calendar_sync_account` wrapper (`crates/calendar/src/sync.rs:17,79`)
- `sync_google_calendar_account` (line 244), `sync_graph_calendar_account` (line 269), `sync_caldav_calendar_account` (line 294)
- The per-visible-calendar loops at lines 254 / 279
- `rtsk::caldav::sync::sync_caldav_calendars` and equivalents

Point-checks at calendar-list-entry, per-calendar-entry, per-event-batch boundaries. Same shape as IMAP. Until this lands, `CalendarRuntime::cancel_account` is a stub - the token flips but the runner finishes anyway.

**IMAP (task 2):** Has the same shape gap one layer down. `imap_initial_sync` and `imap_delta_sync` accept the token through their signature but discard it via the `let _cancellation_token = cancellation_token;` markers. Phase 5 plumbs the token into the per-folder loop and the per-batch persist points (mirroring JMAP's per-mailbox / per-batch checkpoints). Scope expands to helpers: `batch_delta_check` (`imap_delta.rs:301`), `imap_delta_janitor` (`imap_delta_janitor.rs:215`), `client::sync` (`client/sync.rs:18`).

**Use point-checks, not `tokio::select!`, for IMAP.** Dropping a future mid-FETCH leaves the IMAP session with unread response data on the wire; the next command sees that data and the session is broken. Point-checks between RPCs (`if cancellation_token.is_cancelled() { return Cancelled }`) give RPC-boundary cancellation, which is what JMAP's pattern actually does. If we ever need true mid-fetch interruption, we'd need `select!` *plus* explicit session teardown on the cancellation arm. Not in Phase 5.

Token-refresh / principal-resolution checkpoints aren't applicable to IMAP (no OAuth refresh on the sync path; principal is the authenticated user).

### Calendar completion notification: dual routing

The initial draft used a single `CalendarCompleted` notification both for per-`run_id` request-completion routing (mirroring `SyncCompleted`) and for UI view refresh. Looking at how `SyncCompleted` actually flows - `service_client.rs:901,1901` consumes it inside `ServiceClient` and does **not** enqueue it as a UI `ServiceNotification` - that single-notification design would mean the UI calendar reload path never fires. Two notifications:

```text
CalendarRunCompleted (MustDeliver)
   |-- account_id, run_id, result, service_generation
   '-- consumed by ServiceClient broadcast subscribers (per-run_id awaiters);
       NOT enqueued to UI message queue.
       Mirrors SyncCompleted exactly.

CalendarChanged (Coalesce)
   |-- account_id, service_generation
   '-- enqueued to UI as ServiceNotification;
       UI dispatcher coalesces and triggers debounced view reload.
       250ms trailing-edge debounce so 5 accounts completing a kick batch
       produce 1 reload, not 5.
```

The Service emits both for any successful run. A failed/cancelled run emits only `CalendarRunCompleted` (no view change to refresh). UI subscribes to `CalendarChanged` only via the dispatcher; explicit-request callers (post-account-add, manual sync now) await `CalendarRunCompleted` via the per-`run_id` channel.

### Drain ordering: reserved, not load-bearing today

```text
1. PushRuntime::shutdown()       (Phase 4)
2. CalendarRuntime::shutdown()   (Phase 5 - NEW)
3. SyncRuntime::shutdown()       (Phase 3, relocated in Phase 4)
4. drop Arc<SyncRuntime>         (releases SearchWriteHandle clone)
5. await search-writer JoinHandle (Phase 4 review-pass fix)
6. lifecycle::drain (sentinel)   (Phase 1.5)
7. drop(out_tx); writer_handle.await
```

**The action-worker rationale doesn't actually hold today.** The action worker is alive throughout *all* of push/sync/calendar/search-writer/sentinel - it's aborted only after the consolidated drain returns and the shutdown ack is sent (`dispatch.rs:362`). So calendar drains before *or* after sync without affecting action-worker availability. The initial draft's "calendar before sync because RSVP send needs the action worker" rationale was forward-looking but described as load-bearing.

**Why the order is still fixed:** if a future change wires calendar's cancellation cleanup to dispatch action plans (RSVP send is the obvious candidate), the order should already be in place so the change is a one-liner. Reordering at that point is more disruptive than picking a defensible default now. The actual `push -> sync` ordering *is* load-bearing (push events call `SyncRuntime::start_account`); calendar has no analogous cross-runtime call site.

Code-comment text in § "Code-comment requirements" reflects this: the comment says the order is reserved, not that there's a current dependency. A reviewer auditing in 6 months won't find an RSVP path and conclude the comment is stale.

### GAL `gal.kick` shape

`ClientNotification::GalKick` (mirroring `PendingOpsKick`). Notification class is `Drop` - a missed kick is harmless; the next tick re-covers. Service handler (corrected APIs - `BootSharedState` exposes `db_conn()` and `encryption_key()`, not a `write_db_state()` method):

```rust
// In dispatch wiring, hold a Tokio Mutex<()> shared across handler invocations.
// NOTIFY_CAP = 4 means handlers run concurrently by default; without this lock,
// two stale-account kicks back-to-back will duplicate provider calls.
static GAL_HANDLER_LOCK: Mutex<()> = Mutex::const_new(());

async fn handle_gal_kick(boot_state: &Arc<BootSharedState>) {
    let _guard = GAL_HANDLER_LOCK.lock().await;
    let Some(conn) = boot_state.db_conn() else { return; };
    let Some(key) = boot_state.encryption_key() else { return; };
    let write_db = WriteDbState::from_arc(conn);
    let read_db = write_db.to_read_state();
    // refresh_gal_for_account self-gates non-supported providers (returns Ok(0));
    // no enumerate_supported_accounts helper needed.
    let account_ids = list_all_account_ids(&read_db).await;
    for account_id in account_ids {
        match tokio::time::timeout(
            Duration::from_secs(60),
            rtsk::contacts::gal::refresh_gal_for_account(&read_db, &account_id, key),
        ).await {
            Ok(Ok(n)) if n > 0 => log::info!("[GAL] Cached {n} entries for {account_id}"),
            Ok(Ok(_)) => {}
            Ok(Err(e)) => log::warn!("[GAL] Refresh failed for {account_id}: {e}"),
            Err(_) => log::warn!("[GAL] Refresh timed out for {account_id}"),
        }
    }
}
```

Notes:
- 60 s per-account timeout preserves today's UI-side `refresh_gal_caches` budget.
- 24 h staleness gate already lives in `refresh_gal_for_account` (`crates/core/src/contacts/gal.rs:181-185`); back-to-back kicks with all accounts fresh are N trivial cache-age reads. No additional 30 s short-circuit needed.
- **The `Mutex` is required, not optional.** Notification dispatcher runs handlers concurrently up to `NOTIFY_CAP = 4` (`dispatch.rs:33,461`). Without the lock, two concurrent `gal.kick` invocations both call `refresh_gal_for_account` for the same stale account and duplicate the provider round-trip. Tested explicitly in the Phase 5 unit tests.

**Notification-drain bound.** The handler can take up to N×60s in the worst case (all accounts stale, all providers timing out). Today's `drain_in_flight(&notifications_in_flight)` (`dispatch.rs:249,506`) awaits unbounded - that means a wedged GAL refresh can stall shutdown for minutes. Phase 5 adds a hard cap (proposed: 5s aggregate) on the notification drain; past it, the drain logs `[shutdown] notification drain timed out, aborting N tasks` and proceeds. Phase 4 added an analogous `stop_push` ceiling.

### `Message::SyncTick` collapse

Today (post-Phase-4):
```rust
Message::SyncTick => {                               // 5-min cadence (subscription.rs:97)
    let sync_task = self.sync_all_accounts();        // IPC: sync.start_account per account
    let pending_task = self.process_pending_ops();   // IPC: pending_ops.kick
    let gal_task = self.refresh_gal_caches();        // UI-SIDE: direct DB write
    let cal_task = self.sync_calendars();            // UI-SIDE: direct DB write
    Task::batch([sync_task, pending_task, gal_task, cal_task])
}

Message::GalRefreshTick => {                         // 1-hour cadence (subscription.rs:108-112)
    // No-op placeholder. `update.rs:697-705` logs and returns;
    // the working GAL refresh is on SyncTick above.
}
```

After Phase 5:
```rust
Message::SyncTick => {                               // 5-min cadence (unchanged)
    let sync_task = self.sync_all_accounts();        // IPC: sync.start_account per account (request fan-out)
    let pending_task = self.process_pending_ops();   // IPC: pending_ops.kick (notification)
    let gal_task = self.kick_gal_refresh();          // IPC: gal.kick (notification, NEW)
    let cal_task = self.kick_calendar_sync();        // IPC: calendar.kick (notification, NEW)
    Task::batch([sync_task, pending_task, gal_task, cal_task])
}

// Message::GalRefreshTick: DELETED. Subscription removed.
```

**Three notifications + one request fan-out** (the initial draft's "four IPC kicks" phrasing conflated the wire-protocol semantics).

**Cadence stays at 5 min on `SyncTick`.** The 5-min cadence is fine for both calendar and GAL because Service-side staleness gates prevent over-frequent provider work:
- **Calendar:** `CalendarRuntime` tracks `last_calendar_sync` per account. The kick handler skips accounts synced < 1h ago. Hourly effective cadence with no UI-side timer.
- **GAL:** `refresh_gal_for_account` already has a 24 h cache check (`gal.rs:181-185`). 12 kicks/hour through stale-account check is N trivial reads.

**Why not a separate hourly subscription?** Two cadences = two timers = two failure modes (one stops working but the other doesn't). One cadence + Service-side gating is the simpler shape and survives the Phase 9 tray-resident move (the gating logic transplants to the Service-side scheduler unchanged).

Both new `kick_*` methods are tiny `Task::perform(client.send_notification(...))` wrappers.

### Account-deletion integration

`crates/app/src/handlers/core.rs:686` (`delete_account`) calls `client.cancel_and_await(&account_id)` before issuing the DB delete. That entry today cancels:
- `SyncRuntime::cancel_account` (Phase 3)
- `PushRuntime::cancel_account` (Phase 4)

Phase 5 must add `CalendarRuntime::cancel_account` to the same surface. Calendar tables CASCADE from `accounts` (`crates/db/src/db/schema/05_calendar.sql:3`), so a calendar runner with an open `WriteDbState` write borrow can race the DELETE FROM accounts and either SQLITE_BUSY or write rows that vanish under it.

Wiring:
1. `service-api`: add `RequestParams::CalendarCancelAccountSync { account_id }`. 5 s timeout.
2. `service`: `dispatch.rs` arm calls `CalendarRuntime::cancel_account(&account_id).await`.
3. `service_client.rs::cancel_and_await`: extend the existing parallel-cancel `join!` to include the calendar request alongside sync and push.

Account-add path also gets a `calendar.start_account_sync` request post-creation (the explicit-request route, distinct from the kick path). Already in scope as the `RequestParams::CalendarStartAccountSync` entry point.

## Detailed task list

In recommended commit order. Each item is one focused commit unless noted.

**Prerequisite commits (block the rest of the phase):**

0a. **Retire the `rtsk -> service` action shim.** Inline `rtsk::actions::*` into the app crate (the only consumer post-Phase-4) or the service crate. Remove the `service = { path = "../service" }` line from `crates/core/Cargo.toml`. Verify no dependency cycle remains. (See § "Prerequisite: break the rtsk -> service shim" for fallback strategies.)

0b. **Add `cal = { path = "../calendar" }` to `crates/service/Cargo.toml`.** Now legal because of 0a.

**Main task list:**

1. **`service-api`: calendar wire types.** New `crates/service-api/src/calendar.rs`: `CalendarRunId`, `CalendarStartAccountSyncParams`, `CalendarStartAck`, `CalendarCancelAccountSyncParams`, `CalendarSyncResult`. Two notification variants:
   - `Notification::CalendarRunCompleted { account_id, run_id, result, service_generation }` - class `MustDeliver`. Mirrors `SyncCompleted` routing.
   - `Notification::CalendarChanged { account_id, service_generation }` - class `Coalesce`. UI-dispatched.
   
   `RequestParams::CalendarStartAccountSync` and `RequestParams::CalendarCancelAccountSync` (5 s timeout each). `ClientNotification::CalendarKick` and `ClientNotification::GalKick` variants (class `Drop`). Catalog tests inline at `crates/service-api/src/notification.rs` and `client_notification.rs`. Type-only commit.

2. **IMAP cancellation depth.** Edit `crates/imap/src/imap_initial.rs`, `crates/imap/src/imap_delta.rs`, `crates/imap/src/imap_delta_janitor.rs`, `crates/imap/src/client/sync.rs`: remove the `let _cancellation_token = cancellation_token;` markers; thread the token into the per-folder loop, per-batch persist points, and helper paths (`batch_delta_check` etc.). **Use point-checks (`if cancellation_token.is_cancelled() { return Cancelled }`) between RPCs, not `tokio::select!`** - rationale in § "Cancellation: runtime -> handler -> provider chain". Mirror Phase 3 task 6's actual checkpoint shape for JMAP.

3a. **Calendar cancellation plumbing + `WriteDbState` migration.** Thread `&CancellationToken` through `crates/calendar/src/sync.rs` (`calendar_sync_account_impl`, `calendar_sync_account`, `sync_google_calendar_account`, `sync_graph_calendar_account`, `sync_caldav_calendar_account`, per-calendar event loops) and `rtsk::caldav::sync::sync_caldav_calendars`. Same point-check shape as IMAP. In the same commit (signature change, must compile together): change the `&ReadDbState` parameter to `&WriteDbState`. Update existing UI-side callers to pass the write half (they hold one already - `db.write_db_state()` returns it). This commit is purely calendar-crate-internal; no Service code yet.

3b. **`crates/service/src/calendar.rs`: `CalendarRuntime`.** Per-account map, panic supervisor (Phase 3 pattern), `closed: AtomicBool` (mirroring `PushRuntimeInner` - **NOT** `SyncRuntime`, which doesn't have the flag), `start_account -> Result<CalendarStartAck, String>` / `cancel_account -> bool` / `shutdown`. Runs `cal::sync::calendar_sync_account_impl` with the cancellation token from 3a. Module-level doc-comment carries the code-comment requirements.

4. **`crates/service/src/handlers/calendar.rs`: handlers.** `handle_start_account_sync` translates the request into `CalendarRuntime::start_account` + serializes the ack (or `Err`). `handle_cancel_account_sync` calls `CalendarRuntime::cancel_account`. `handle_calendar_kick` enumerates accounts whose `last_calendar_sync` is > 1h stale and starts each.

5. **`crates/service/src/handlers/gal.rs`: handler.** `handle_gal_kick` iterates all accounts (no enumerate-supported helper - `refresh_gal_for_account` self-gates) and calls `refresh_gal_for_account` per account with the existing 60 s per-account timeout. **Required:** Tokio `Mutex` on the handler entry point so the `NOTIFY_CAP=4` concurrent-handler dispatcher can't double-fire stale-account fetches. Unit test for the mutex behavior.

6. **`BootSharedState`: install slot for `CalendarRuntime`.** Mirror the `push_runtime` slot pattern: `install_calendar_runtime`, `calendar_runtime`, `take_calendar_runtime`. Boot installs after `SyncRuntime` (since it needs `db_conn` + `encryption_key`). `install_*` is a no-op-if-already-installed guard, mirroring push.

7. **Drain consolidation: insert calendar step + bound notification drain.** `dispatch.rs`'s consolidated drain inserts `CalendarRuntime::shutdown()` between Push and Sync. Same commit: bound `drain_in_flight(&notifications_in_flight)` with a 5 s aggregate cap; past it, log + abort remaining notification tasks. Update the doc-comment on the orchestrating block to reflect both changes.

8. **Dispatch wire-up.** `crates/service/src/dispatch.rs`: register handler arms for `RequestParams::CalendarStartAccountSync`, `RequestParams::CalendarCancelAccountSync`, `ClientNotification::CalendarKick`, `ClientNotification::GalKick`. Mirror the existing `PendingOpsKick` arm pattern.

9. **`service_client.rs::cancel_and_await`: include calendar.** Extend the existing parallel-cancel `join!` to call `calendar.cancel_account_sync` alongside sync and push. Required for account-deletion safety (calendar tables CASCADE from `accounts`).

10. **UI teardown: delete `sync_calendars` and `refresh_gal_caches`; delete `Message::GalRefreshTick`.** Delete the methods in `crates/app/src/handlers/provider.rs`. Delete the `Message::GalRefreshTick` variant, its `iced::time::every` subscription, and the no-op handler. Add `kick_calendar_sync` and `kick_gal_refresh` thin wrappers that send the new client notifications. Update `Message::SyncTick` arm in `update.rs`.

11. **UI: `Notification::CalendarChanged` arm with debounce.** In `update.rs::Message::ServiceNotification` dispatch: route `CalendarChanged` to a 250ms trailing-edge debouncer feeding a single `reload_calendar_events()` call. `Notification::CalendarRunCompleted` is consumed inside `ServiceClient`'s reader task (mirroring `SyncCompleted`) - never reaches the UI dispatcher.

12. **Catalog tests: production_notification_catalog gains `CalendarRunCompleted` and `CalendarChanged`.** (`crates/app/src/service_client.rs`).

13. **Test cohort.** Phase 5 unit / integration / real-subprocess tests. Same caveat as Phase 4: integration tests for `CalendarRuntime` lifecycle need either a fake CalDAV server fixture or `test_dummy` constructors on the writer-state types - so the bulk gets Phase 8'd alongside Phase 4's deferred cohort. What CAN land in Phase 5:
   - IMAP cancellation unit tests (drive the per-folder loop with a cancelled token; assert it returns `Cancelled` between RPCs).
   - Calendar cancellation unit tests (same shape against `calendar_sync_account_impl` with a stub provider).
   - `CalendarRuntime` shutdown-guard tests (the start-after-shutdown -> `Err` invariant).
   - GAL handler mutex test (two concurrent kicks; assert single `refresh_gal_for_account` call per account).
   - Notification-drain timeout test (wedged handler; assert drain returns within 5 s + warning logged).
   - Wire-type round-trips for new calendar/kick types.
   - Cancel-on-account-delete test (`cancel_and_await` cancels all three runtimes).

14. **Doc updates.** Phase 5 status block in `problem-statement.md`. `problem-statement.md` § "Cross-store crash consistency" gets a "Calendar sync state" row. `problem-statement.md` § "remaining UI write surfaces" inventory updated to: GAL refresh moved Service-side; calendar event mutations (`cal::actions::*`) explicitly remain UI-side, deferred to Phase 6. `implementation-roadmap.md` Phase 5 entry corrected to reflect what actually shipped vs what cascaded from Phase 3. Add Phase 9 TODO marker for tray-resident cadence ownership. Bundle with the close-out commit per CLAUDE.md's "no markdown-only commits" rule.

## File-by-file changes

**New files:**
- `crates/service-api/src/calendar.rs` - calendar wire types.
- `crates/service/src/calendar.rs` - `CalendarRuntime`.
- `crates/service/src/handlers/calendar.rs` - start, cancel, kick handlers.
- `crates/service/src/handlers/gal.rs` - `gal.kick` handler.

**Modified files:**
- `crates/core/Cargo.toml` - **delete** `service = { path = "../service" }` line (Prerequisite 0a).
- `crates/service/Cargo.toml` - **add** `cal = { path = "../calendar" }` (Prerequisite 0b).
- `crates/service-api/src/lib.rs` - re-export calendar types.
- `crates/service-api/src/notification.rs` - add `CalendarRunCompleted` (MustDeliver) + `CalendarChanged` (Coalesce) variants + arms + catalog tests.
- `crates/service-api/src/client_notification.rs` - add `CalendarKick`, `GalKick` variants + class arms (both `Drop`).
- `crates/service-api/src/request.rs` - add `CalendarStartAccountSync` and `CalendarCancelAccountSync` variants + 5 s timeouts.
- `crates/imap/src/imap_initial.rs` - thread cancellation through per-folder loop with point-checks (no `tokio::select!`).
- `crates/imap/src/imap_delta.rs` - same; covers `batch_delta_check`.
- `crates/imap/src/imap_delta_janitor.rs` - thread cancellation through helper.
- `crates/imap/src/client/sync.rs` - thread cancellation through client-level helpers.
- `crates/calendar/src/sync.rs` - **substantial change**: add `&CancellationToken` parameter to `calendar_sync_account_impl`, `calendar_sync_account`, `sync_google_calendar_account`, `sync_graph_calendar_account`, `sync_caldav_calendar_account`, and per-calendar event loops. Switch `&ReadDbState` to `&WriteDbState` (write-surface escape fix).
- `crates/core/src/caldav/sync.rs` - `sync_caldav_calendars` accepts cancellation token + point-checks.
- `crates/service/src/boot.rs` - install `CalendarRuntime` slot + construction.
- `crates/service/src/boot_state.rs` (or wherever `BootSharedState` lives) - `install_calendar_runtime`, `calendar_runtime`, `take_calendar_runtime`.
- `crates/service/src/dispatch.rs` - drain step insertion (PushRuntime -> CalendarRuntime -> SyncRuntime); 5 s notification-drain bound; handler dispatch arms for the new request and notification types.
- `crates/service/src/handlers/mod.rs` - export new handler modules.
- `crates/service/src/lib.rs` - `pub mod calendar`.
- `crates/app/src/service_client.rs` - reader-task `Notification::CalendarRunCompleted` consumer (mirrors `SyncCompleted`); `cancel_and_await` extended to call `calendar.cancel_account_sync`; catalog-test entries for both calendar notifications.
- `crates/app/src/update.rs` - `Notification::CalendarChanged` dispatch with 250ms trailing-edge debouncer; `Message::SyncTick` collapse; **delete** `Message::GalRefreshTick` arm.
- `crates/app/src/subscription.rs` - **delete** the `GalRefreshTick` `iced::time::every` subscription (lines 108-112 today).
- `crates/app/src/handlers/provider.rs` - **delete** `sync_calendars` and `refresh_gal_caches`; **add** `kick_calendar_sync` and `kick_gal_refresh`.
- `crates/app/src/handlers/core.rs::delete_account` - implicit (just keeps using `cancel_and_await`, which now also cancels calendar via service_client.rs change).
- `crates/app/src/handlers/calendar.rs` (any UI callers passing `&ReadDbState` to calendar sync) - update call sites to pass `&WriteDbState` per 3a's signature change.

**Deletions of whole files:** none. **Removed code:**
- `Message::GalRefreshTick` and all references (subscription, dispatch arm, no-op handler).
- The `let _cancellation_token = cancellation_token;` markers in IMAP.
- `crates/app/src/handlers/provider.rs::sync_calendars` and `::refresh_gal_caches` bodies.

Calendar event-mutation code in `crates/calendar/src/actions/` and contact code in `crates/core/src/contacts/` is unchanged - only call sites and signatures move.

## Code-comment requirements

The strategic decisions from the revision history must appear as code comments where the relevant logic lives. All blocking on the relevant commit:

1. **`crates/service/src/calendar.rs` module-level doc-comment** must contain:
   - "Structurally symmetric with `crates/service/src/sync.rs::SyncRuntime` for the lifecycle surface (per-account map, panic supervisor, start/cancel/shutdown). The `closed: AtomicBool` shutdown guard mirrors `crates/service/src/push.rs::PushRuntimeInner` (line 109) - SyncRuntime itself does not have the flag. We have it here because Calendar has a kick-driven entry path (the hourly tick) analogous to push's post-ready iteration: any kick arriving during shutdown must be rejected. Diverges intentionally on: no marker-file lifecycle (calendar sync is idempotent against CalDAV CTags / Exchange ETags); no body / inline / search writer halves (calendar writes only to calendar tables); no invariant-pass entry. If you find yourself adding any of those, ask whether the divergence is still justified."
   - "Drains *before* SyncRuntime in the consolidated drain. The order is **reserved**, not currently load-bearing - the action worker is alive throughout the entire consolidated drain, so calendar drains before or after sync without affecting action-worker availability today. The order is fixed so a future change wiring calendar-cancel cleanup to dispatch action plans (RSVP send is the candidate) is a one-liner instead of a drain reshuffle. Don't promote this to 'load-bearing today' rationale unless that wiring lands - reviewers should not look for an RSVP path that doesn't exist."

2. **`crates/imap/src/imap_initial.rs`, `crates/imap/src/imap_delta.rs`, `crates/imap/src/imap_delta_janitor.rs`, and `crates/imap/src/client/sync.rs` per-folder/per-helper loop** must have an inline comment at the cancellation checkpoints:
   - `// Cancellation checkpoint - mirrors JMAP's per-mailbox checkpoint in crates/jmap/src/sync/mod.rs (Phase 3 task 6). The previous incomplete-port pattern was \`let _cancellation_token = cancellation_token;\` immediately after the entry-point check, dropping the token without threading it into the loop. A user pressing "cancel sync" mid-IMAP-sync should not have to wait out the entire sync. Use point-checks between RPCs - NOT \`tokio::select!\` - because IMAP is a stateful session: dropping a future mid-FETCH leaves unread response data on the wire and breaks the next command. If true mid-RPC interruption is ever needed, pair \`select!\` with explicit session teardown on the cancel arm.`

3. **`crates/calendar/src/sync.rs` per-calendar-loop and per-event-batch checkpoints** must have an inline comment:
   - `// Cancellation checkpoint - mirrors the IMAP and JMAP per-mailbox patterns. Calendar sync is idempotent against CalDAV CTags / Exchange ETags, so a cancelled run resumes from wherever the next run finds the provider state - no marker-file repair needed. Point-checks between RPC boundaries, not mid-RPC.`

4. **`crates/service/src/handlers/gal.rs::handle_gal_kick`** doc-comment must contain:
   - "GAL refresh is kick-driven + idempotent + bounded (60 s per-account timeout × account count, gated by 24 h cache check in `refresh_gal_for_account`). No per-account runtime; no cancellation. **Required: serialize handler invocations via the module-level Tokio `Mutex`.** The notification dispatcher runs handlers concurrently up to NOTIFY_CAP=4 (`dispatch.rs:33`); two stale-account kicks back-to-back without serialization will duplicate provider calls. The mutex is load-bearing for correctness, not just performance. If a benchmark surfaces parallelism need, switch to a per-account `HashMap<AccountId, Mutex<()>>` rather than dropping the lock entirely."

5. **The consolidated drain helper's doc-comment in `dispatch.rs`** must be updated to include the calendar step + the notification-drain bound:
   - "Drain order: PushRuntime -> CalendarRuntime -> SyncRuntime -> search-writer -> sentinel. The Calendar-before-Sync ordering is reserved for future calendar-cancel-dispatch-action work (RSVP send), not load-bearing today (action worker is alive throughout the drain). The Push-before-Sync ordering IS load-bearing - push events call `SyncRuntime::start_account`. Notification drain is bounded at 5 s aggregate; past it, we log + abort remaining notification tasks. This prevents a wedged GAL or pending-ops handler from stalling shutdown indefinitely (Phase 5 fix; analogous to Phase 4's `stop_push` ceiling)."

6. **`crates/service/src/calendar.rs::CalendarRuntime::start_account`** must mirror the Phase 4 review-pass `closed: AtomicBool` guard and the lock-released-during-network restructure. Inline comment:
   - "Same shutdown-guard pattern as PushRuntime - check `closed` before the slow path, re-acquire the lock for the insert and re-check both the guard and the duplicate-entry guard. Mirrors `crates/service/src/push.rs::PushRuntime::start_account`. Diverging is a refactor smell. Returns `Result<CalendarStartAck, String>` so post-shutdown calls produce a testable `Err`, not a silently-dropped start."

These comment texts are the contract; reviewers will reject commits that reword them in ways that lose the *why*.

## Test plan

### Unit tests

- `service-api`: serde round-trip for `CalendarRunId`, `CalendarStartAck`, `CalendarSyncResult`, `CalendarRunCompleted`, `CalendarChanged`, `CalendarCancelAccountSyncParams`. `RequestParams::CalendarStartAccountSync.timeout()` and `RequestParams::CalendarCancelAccountSync.timeout()` return 5 s. Catalog cases for `CalendarRunCompleted` (class `MustDeliver`, method name, generation round-trip, `parse_service_message` round-trip) and `CalendarChanged` (class `Coalesce`). Catalog cases for `ClientNotification::CalendarKick` and `GalKick` (class `Drop`).
- `service::calendar`: `CalendarRuntime::cancel_account` returns false when no entry exists; `shutdown` is safe on empty runtime; `start_account` returns `Err` after `shutdown` (the unit-testable invariant from PushRuntime's review-pass pattern); a runner observing a flipped cancellation token returns `Cancelled` (requires task 3a's plumbing - the test would be degenerate without it).
- `imap` cancellation: drive `imap_initial_sync` (or its testable subroutine) with a pre-cancelled token; assert it returns the cancelled error path before any network round-trip. Drive with a token that flips mid-folder-iteration; assert the loop breaks at the next point-check, not at the end. Cover `batch_delta_check`, `imap_delta_janitor`, and `client::sync` helpers explicitly.
- `calendar` cancellation: same shape against `calendar_sync_account_impl` with a stub provider for each of the three paths (Google, Graph, CalDAV).
- `service::handlers::gal`: **mutex behavior test.** Send two `gal.kick` notifications nearly-simultaneously through a test dispatch with `NOTIFY_CAP=4`; stub `refresh_gal_for_account` to record calls per account; assert at most one in-flight call per account at any moment.
- `service::dispatch`: notification-drain timeout test. Wedge a notification handler with `tokio::time::sleep(60s)`; trigger drain; assert it returns within ~5 s and logs the abort warning.

### Integration tests (in-process)

- `calendar_kick_starts_sync_in_service`: spin up a `CalendarRuntime` against a stub provider; send a `calendar.kick` notification; assert per-account starts fire only for accounts whose `last_calendar_sync` is > 1h stale. Same caveat as Phase 4: real provider integration needs a fake CalDAV fixture, deferred.
- `calendar_run_completed_consumed_by_service_client`: assert `Notification::CalendarRunCompleted` is consumed inside `ServiceClient` (per-`run_id` broadcast subscriber) and does NOT reach the UI `ServiceNotification` queue.
- `calendar_changed_routed_to_ui_with_debounce`: assert `Notification::CalendarChanged` reaches the UI dispatcher; N near-simultaneous notifications produce one debounced `reload_calendar_events()` call.
- `calendar_drains_before_sync_at_shutdown`: instrumented version - assert no calendar `start_account` is called after `SyncRuntime::shutdown` begins. Flag explicitly: this is doc-only enforcement today (no compile-time guarantee on drain order); test catches reorderings.
- `cancel_and_await_cancels_calendar`: simulate account-delete; assert `client.cancel_and_await` triggers `CalendarRuntime::cancel_account` alongside sync and push.

### Real-subprocess smoke tests

- `service_subprocess_calendar_kick_routes_to_handler`: spawn the Service with a seeded calendar account; observe a `calendar.kick` notification reaches the handler (via a debug log assertion). No actual CalDAV traffic - that needs the fixture.
- `service_subprocess_imap_cancel_interrupts_mid_sync`: spawn with an IMAP-stub account; trigger an initial sync; cancel mid-sync; assert `SyncCompleted { result: Cancelled }` arrives within a bounded window (proposed: < 2 s after cancel issue). Tests the cancellation-depth fix at RPC boundary granularity.
- `service_subprocess_calendar_cancel_on_account_delete`: spawn with a seeded account; start a calendar sync; trigger account delete; assert no calendar table writes complete after the DELETE FROM accounts (no SQLITE_BUSY, no orphan rows).

### Manual matrix updates

- The "what survives a Service crash" matrix in `problem-statement.md` § "Cross-store crash consistency" gets a new row: "Calendar sync state". Phase 5 outcome: idempotent re-fetch on next sync (the runtime has no marker; the calendar provider's CTag / ETag handling re-fetches what changed). No torn-write recovery needed.

## Open questions

(Most original open questions resolved during the post-review revision; see § "Revision history".)

1. **`CalendarRuntime` per-account concurrency.** SyncRuntime is one-runner-per-account; calendar could plausibly be one-runner-per-account-per-calendar (an account may host multiple calendar collections). For Phase 5 we mirror SyncRuntime's per-account-only granularity; if a benchmark surfaces lock contention on a single account with many calendars, revisit. Not blocking for the phase.
2. **Debounce window for `CalendarChanged`-driven reload.** Plan proposes 250 ms trailing-edge. Alternatives: 0 ms (immediate, accept N reloads for N accounts), or a leading-edge + trailing-edge combo (immediate first reload, debounced last). 250 ms trailing is the simplest and matches typical UI debounce defaults; revisit if calendar tab feels laggy on multi-account kick batches.
3. **Where exactly does `last_calendar_sync` per-account live?** Two options: (a) in-memory on `CalendarRuntime` (lost on Service restart - first kick after restart re-syncs all accounts), (b) persisted in DB (survives restart, requires schema add). (a) is simpler and the cost of re-syncing once per Service restart is negligible (idempotent). Recommended: (a). Confirm during implementation.

**Resolved during this revision** (do not re-litigate):
- Cadence: 5-min `SyncTick` with Service-side staleness gating, no separate hourly subscription.
- Notification routing: split into `CalendarRunCompleted` (MustDeliver, ServiceClient-consumed) + `CalendarChanged` (Coalesce, UI-dispatched).
- GAL handler concurrency: serialized via Tokio Mutex (required, not optional).
- Tray-resident scheduler ownership: deferred to Phase 9 with explicit TODO marker.

## Verification (end-to-end)

- A change pushed to a JMAP mailbox triggers a sync inside the Service (Phase 4 verification carry-over).
- An IMAP initial sync started mid-fetch and then cancelled returns `SyncCompleted { result: Cancelled }` within seconds, not after the full folder list completes (RPC-boundary granularity is fine; mid-RPC interruption is not in scope).
- A calendar sync started and then cancelled returns `CalendarRunCompleted { result: Cancelled }` within seconds (verifies task 3a's plumbing reaches the leaf).
- A `Message::SyncTick` from the UI fires three notifications + one request fan-out (sync_all_accounts requests, pending_ops kick, calendar kick, gal kick) and zero UI-side provider work.
- The deleted `Message::GalRefreshTick` subscription is gone from `subscription.rs`; no compile or test references the variant.
- Stopping the Service mid-calendar-sync does not corrupt any DB state - drain order holds (calendar before sync before sentinel) and the runner observes cancellation at a checkpoint.
- A wedged GAL handler does not stall shutdown beyond ~5 s (notification-drain timeout fires).
- Two near-simultaneous `gal.kick` notifications produce one provider call per stale account, not two (mutex behavior).
- Account deletion via `cancel_and_await` cancels the calendar runner alongside sync and push (no orphan calendar rows post-DELETE FROM accounts).
- A `Notification::CalendarRunCompleted` is consumed inside `ServiceClient`'s reader task; the UI never sees it. A `Notification::CalendarChanged` reaches the UI dispatcher and triggers a debounced reload.
- A user opening a calendar attendee picker sees GAL entries refreshed within ~1 hour of any account being added (the Service's `gal.kick` handler picks it up on the next 5-min tick after the 24h cache age check passes).
- The "Phase 5 status (as landed)" block in `problem-statement.md` documents what Phase 5 actually relocated vs what cascaded from Phase 3, and the "remaining UI write surfaces" inventory correctly lists calendar event mutations as still-UI-side (deferred to Phase 6).

## Promotion criteria

- All Phase 5 tasks landed; IMAP `let _cancellation_token = cancellation_token;` markers are gone; calendar sync stack accepts `&CancellationToken` end-to-end; `Message::GalRefreshTick` is deleted; UI-side `sync_calendars` and `refresh_gal_caches` are deleted; `CalendarRuntime` is in the consolidated drain; notification drain is bounded; GAL handler is serialized; account-delete cancels calendar.
- `rtsk -> service` shim is gone; no dependency cycle remains (`cargo metadata` clean).
- `Message::SyncTick` does no UI-side provider work.
- Calendar sync writes through `&WriteDbState`, not `&ReadDbState`.
- Phase 5 status block added to `problem-statement.md`; remaining UI write-surface inventory updated; Phase 9 tray-resident TODO marker present.
- `phase-5-plan.md` is then retirement-ready: every deferral has an explicit roadmap entry (the Phase 8 test-cohort carry-forward already exists; Phase 5 just adds its own integration tests to that bucket), every code-comment requirement is present in the relevant file.
