# The Service - Phase 5 Plan: port sync to other providers + calendar + GAL relocation

Companion to `phase-1-plan.md`, `phase-1.5-plan.md`, `phase-2-plan.md`, `phase-3-plan.md`, `phase-4-plan.md`. Implements Phase 5 of `implementation-roadmap.md`.

## Revision history

**2026-05-05 - second-review revision (arch+bugs sweep, second pass).** The post-review revision was sent back through arch+bugs (claude + codex each, four sessions). Consolidated changes from that second pass:

- **Prerequisite scope expanded.** The cycle break is more than the action shim. `crates/core/src/lib.rs:55` re-exports `service::sync_dispatch`; `crates/core/src/chat.rs` uses `service::actions/provider/pending` APIs directly; `crates/calendar/src/actions.rs:13` imports `rtsk::actions::{ActionContext, ActionError, ActionOutcome, MutationLog}`. Phase 5 defers calendar event mutations to Phase 6, so `cal::actions` keeps those imports throughout the phase. § "Prerequisite" now enumerates all consumers and names the shared-action-types extraction (or fallback to option 3) that has to happen for the cycle to actually break.
- **Task 3a `WriteDbState` migration cannot land "purely calendar-crate-internal".** `Db::write_db_state()` returns `ReadDbState`, not `service_state::WriteDbState` (`crates/app/src/db/connection.rs:32`), and `app` is forbidden from depending on `service-state` (`docs/architecture.md:47`). Re-ordered: 3a is now cancellation plumbing only (still `&ReadDbState`); the signature flip moves into 3b alongside Service runtime introduction, and the UI call sites are deleted in the same commit.
- **`service -> rtsk` dependency added explicitly.** GAL handler calls `rtsk::contacts::gal::refresh_gal_for_account`. Service does not currently depend on rtsk; the file-by-file list only added `cal`. Both deps now listed in § "Prerequisite" and § "File-by-file changes".
- **Task 9 (`cancel_and_await`) corrected.** Plan claimed an existing parallel-cancel `join!`; in fact `cancel_and_await` sends one IPC (`SyncCancelAccount`) and push-cancel is piggybacked server-side inside `handle_cancel_account`. Calendar-cancel-on-delete now mirrors push: piggyback inside `handle_cancel_account`. `RequestParams::CalendarCancelAccountSync` is reserved for explicit-request cancel only (manual sync now, RSVP).
- **Cancel ack carries `run_id`.** `CalendarRuntime::cancel_account -> bool` loses the correlation key needed to await the terminal `CalendarRunCompleted` for the deletion path. New `CalendarCancelAck { run_id: Option<CalendarRunId> }` mirrors `SyncCancelAck`; `cancel_and_await` awaits both sync and calendar terminal completions before issuing the DB DELETE.
- **Explicit-request calendar path fully wired.** Initial revision named the request method but didn't list the wiring tasks. Added: pending-run map in `ServiceClient`, reader-task `CalendarRunCompleted` consumption (mirrors `SyncCompleted`), respawn failure handling, `start_calendar_sync` UI helper, concrete call-site updates (account-add post-creation, manual "sync now", RSVP-then-resync).
- **`CalendarChanged` fires on partial mutations, not just success.** `crates/calendar/src/sync.rs:249` upserts discovered calendars before per-calendar event loops run, and per-calendar results are applied independently (line 263). A cancellation after a committed batch would leave the UI stale under the original "successful runs only" rule. `CalendarRuntime` now tracks `mutated: bool` per-run; `CalendarChanged` fires whenever local rows changed regardless of final result.
- **`CalendarChanged` `CoalesceKey` defined.** `CoalesceKey::CalendarChanged { account_id }` (account-scoped). Queue tests assert two notifications for different accounts pass through; two for the same account coalesce.
- **GAL Mutex reframed.** Original rationale named a per-account hazard (two concurrent stale-account kicks duplicate provider calls) but proposed a global handler `Mutex<()>` that serializes across all kicks. Phase 5 keeps the global mutex as the simplest correct shape, but documents it as a coarsening of the load-bearing form (per-account in-flight set inside `refresh_gal_for_account`). Future-work direction is now explicit so a future reviewer doesn't conclude "this is just sequential anyway, drop the lock."
- **`CalendarRuntime` concurrency cap.** Plan didn't bound parallel runners. Service respawn within the 5-min cadence + in-memory `last_calendar_sync` reset triggers an N-account thundering herd of parallel TLS sessions and DB writes. Added: per-runtime semaphore (mirrors `SyncRuntime`'s pattern). Open question 3 closed: in-memory + semaphore.
- **Notification-drain + `spawn_blocking` contract.** `refresh_gal_for_account` performs DB writes via `spawn_blocking`. Aborting an outer async task does not stop a running blocking closure. Documented contract: drain-timeout aborts the async wrapper; the blocking work runs to completion. Acceptable for GAL (writes are bounded and idempotent); explicit so handlers added later don't silently inherit the constraint.
- **IMAP cancel-latency budget realigned.** Point-checks-between-RPCs against `IMAP_FETCH_TIMEOUT` (tens of seconds) cannot meet a 2 s subprocess test. Real-provider bound stated as `IMAP_FETCH_TIMEOUT` upper; stubbed-server subprocess test flushes within the original tighter budget.
- **GAL widened-DB-read load acknowledged.** Service-side handler iterates all accounts; today's UI-side `provider in {graph, gmail_api}` filter goes away. Adds two `with_conn` reads per account per kick (`gal_cache_age` + `get_account_provider_sync`); steady-state cost is small, called out so it doesn't surface later as a "silent regression."
- **`cal::actions` write-surface escape called out as Out of scope.** `crates/calendar/src/actions.rs` writes via `&ReadDbState` today (same escape Phase 4 cleaned up for sync). Phase 5's `WriteDbState` migration covers `calendar_sync_account_impl` only; action-mutation paths get the migration in Phase 6 alongside relocation.
- **Stale draft contradictions cleaned up.** Strategic-decision text in § Initial draft about "calendar before sync because RSVP path uses action worker" was superseded by the "reserved, not load-bearing" framing later in the doc; reconciled in place. Earlier-draft "per-account in-flight set" reference for GAL conflicted with the global-mutex decision; clarified as the documented future-work direction. § Initial draft's "1-hour UI tick" for GAL was wrong - GAL today runs on the 5-min `SyncTick` (`Message::GalRefreshTick` is a no-op placeholder).
- **Drain-ordering test scoped accurately.** `calendar_drains_before_sync_at_shutdown` enforces a reserved-not-load-bearing invariant; the test docstring says explicitly that when the RSVP wiring lands, the test transitions to load-bearing in the same commit that drops the `// reserved` comment.
- **`CalendarChanged` debounce vs UI-mutation interaction documented.** UI-originated mutations (`cal::actions::*`) trigger their own UI reload today; a subsequent Service-pushed `CalendarChanged` for the same change fires the debounced reload - duplicate render, identical DB state. Acceptable; flagged for the Phase 6 mutation-relocation consolidation.
- **Cancel-on-delete asymmetry resolved.** Under the piggyback model, cancel-and-await's terminal-completion guarantee for calendar requires the new `CalendarCancelAck { run_id }` (above). With it, the deletion path awaits both sync and calendar runs before issuing the DELETE.
- **Line-number drift updated.** `imap_initial.rs:67` -> `:72`; `handlers/calendar.rs:309,542` -> `:320,561,585`; `schema/05_calendar.sql:3` -> `:5`. Substantive claims hold.

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

- **GAL (Global Address List) cache refresh still runs UI-side.** Same shape as calendar: `refresh_gal_caches` at `handlers/provider.rs` runs on the 5-min `SyncTick` today (the separate `Message::GalRefreshTick` 1-hour subscription is a no-op placeholder; the working refresh is on `SyncTick`, gated by the 24 h cache age check inside `refresh_gal_for_account`). GAL is fire-and-forget per account, idempotent, no cancellation needed; it gets a notification-driven `gal.kick` IPC mirroring `pending_ops.kick`, no per-account runtime.

- **`Message::SyncTick`'s remaining UI-side branches.** Currently fans out to `sync_all_accounts` (IPC, post-Phase-3), `process_pending_ops` (IPC, post-Phase-2), `refresh_gal_caches` (UI-side, this phase), `sync_calendars` (UI-side, this phase). After Phase 5, all four are IPC kicks. UI's `SyncTick` becomes a pure cadence trigger with no provider work on it.

Strategic decisions that this plan locks in (and that the code comments must mirror):

- **One `SyncRuntime`, four providers, no per-provider runtime split.** `SyncRuntime` is provider-agnostic via `ProviderOps`; the dispatch already works. No `GmailSyncRuntime` etc. Per-provider concurrency policies live inside the provider's own `sync_delta` impl (Gmail/Graph batch size, IMAP per-folder session reuse), not at the runtime layer.

- **Calendar gets a separate `CalendarRuntime` and separate IPC method.** Mirrors `SyncRuntime`'s shape (per-account map, panic supervisor, cancellation token, lifecycle hooks) but with simpler invariants - calendar sync is idempotent (CalDAV CTags / Exchange ETags) and doesn't write to the four-store cluster, so no marker-file lifecycle, no invariant-pass entry. New `calendar.start_account_sync` request method (5 s ack timeout, fire-and-forget runner) and a **dual notification** shape: `calendar.run_completed { account_id, run_id, result, service_generation }` (MustDeliver, ServiceClient-consumed for per-`run_id` awaiters) plus `calendar.changed { account_id, service_generation }` (Coalesce, UI-dispatched for view reload). See § "Calendar completion notification: dual routing" for why a single notification was wrong. Drain order: PushRuntime → CalendarRuntime → SyncRuntime → search-writer → sentinel. The Calendar-before-Sync ordering is **reserved, not load-bearing today** - the action worker is alive throughout the consolidated drain, so calendar can drain before or after sync without affecting action-worker availability. The order is fixed so a future change wiring calendar-cancel cleanup to dispatch action plans (RSVP send) is a one-liner instead of a drain reshuffle.

- **GAL: notification-driven `gal.kick` IPC, no per-account runtime, no cancellation.** GAL refresh is sub-second per account in the steady state (24h cache) and bounded by network round-trips for stale accounts. Service handler iterates accounts, calls existing `refresh_gal_for_account`, returns. Per-account concurrency is not load-bearing at this scale (GAL only fires hourly).

- **IMAP cancellation depth: per-folder loop checkpoint minimum.** Match Phase 3's JMAP coverage at the granularity that matters for IMAP: each folder fetch is a network round-trip; the per-folder loop is the natural break. The `let _cancellation_token = cancellation_token;` pattern is the marker for what to fix.

- **`Message::SyncTick` collapses to three notifications + one request fan-out.** No more UI-side provider work. `sync_calendars` and `refresh_gal_caches` methods on `ReadyApp` (in `handlers/provider.rs`) get deleted entirely; their callers become IPC notification sends. **Two distinct surfaces with distinct semantics** (do not blur):
  - **Cadence-driven kicks** (`calendar.kick`, `gal.kick`) are `ClientNotification`s, fire-and-forget, no per-account targeting. Service-side handler iterates accounts and gates on staleness (calendar: per-account `last_calendar_sync` > 1h; GAL: existing 24h cache).
  - **Explicit-request paths** (`calendar.start_account_sync`) are typed requests with per-account targeting and per-`run_id` completion awaiting. Used for post-account-add, manual "sync now", RSVP-then-resync.
  
  Cadence stays UI-side on the existing 5-min `SyncTick`; staleness gating is Service-side. The dead `Message::GalRefreshTick` (no-op placeholder at `update.rs:697-705`) is deleted.

## Prerequisite: break the rtsk -> service shim before any calendar work

The `crates/calendar/` (`cal`) crate depends on `rtsk` (`crates/calendar/Cargo.toml:14`), and `rtsk` currently depends on `service` (`crates/core/Cargo.toml:45`). Adding the `service -> cal` edge that `CalendarRuntime` needs creates a cycle: `service -> cal -> rtsk -> service`. Cargo will reject this. **Adding `service -> rtsk`** (which the GAL handler needs - it calls `rtsk::contacts::gal::refresh_gal_for_account`) does the same.

The `rtsk -> service` edge is wider than the action shim. Concretely:

- **Action shim modules.** `rtsk::actions::*` re-exports `service::actions::*`. Phase 2 transitional layer.
- **Sync-dispatch re-export.** `crates/core/src/lib.rs:55` re-exports `service::sync_dispatch` so callers can write `rtsk::sync_dispatch::...`.
- **`core::chat`.** `crates/core/src/chat.rs:8` (and friends) imports `service::actions`, `service::provider`, `service::pending` directly - not through a shim.
- **`cal::actions`.** `crates/calendar/src/actions.rs:13` imports `rtsk::actions::{ActionContext, ActionError, ActionOutcome, MutationLog}`. **These are the action types defined in `service::actions`**, re-exported via the shim. Calendar event mutations are deferred to Phase 6, so `cal::actions` must keep using these types throughout Phase 5.

The last bullet is the trap. Even if all `rtsk::actions::*` re-exports go away, `cal::actions` still needs `ActionContext` / `ActionError` / `ActionOutcome` / `MutationLog` from somewhere that isn't `service` (else `service -> cal -> service` reappears).

Resolution options:

1. **Extract the shared action types into a new crate (e.g., `action-types`) or extend `service-state`.** Both `cal` and `service` depend on the new crate. `service::actions::*` keeps the orchestration; `action-types` exports the contract types. Then break `rtsk -> service` by inlining `core::chat`'s `service::*` consumers and the `lib.rs:55` `sync_dispatch` re-export into the appropriate crate (likely `service` for chat orchestration; the `sync_dispatch` re-export deletes outright now that callers can name it directly). Cost: one new crate + ~12 action-shim deletions + chat crate-boundary refactor. **Recommended.**
2. **Pull calendar event-mutation relocation into Phase 5** (originally Phase 6). Then `cal::actions` goes away entirely, `service::actions` becomes the only home, and the cycle has nothing to bridge. Cost: doubles the phase scope.
3. **Inline calendar-sync entry points into `service::calendar`.** `service` doesn't import `cal` at all (it imports `gmail`, `graph`, `caldav` directly). Cost: duplicates orchestration logic; `cal::sync` stays alive for UI-side callers (which Phase 5 deletes anyway, but `cal::actions` keeps importing `cal::sync` types if any are shared). Acts as the fallback if option 1 turns out to be larger than expected mid-implementation.

**Decision:** option 1. Option 3 stays as the explicit fallback. Option 2 is rejected: bundling Phase 6 work into Phase 5 hides the `WriteDbState` migration shape that's currently the cleanest to discuss in isolation.

**`service -> rtsk` is added in the same prerequisite stretch** (after `rtsk -> service` is broken). The GAL handler depends on it; calendar handlers may grow to depend on it as well. This is the inverse of today's edge; it's only legal once the cycle is gone.

This prerequisite section explicitly retracts the initial draft's silent assumption that `service` could just import `cal` directly, and the post-review revision's understatement that the cycle was "just" the action shim.

## Context

Phase 4 closed the JMAP-specific work (push relocation, drain consolidation, OAuth resolver). Phase 5 finishes the email-sync relocation for the remaining three providers and folds in the two non-email subsystems still on the UI's hot path: calendar and GAL.

Most of the email-sync work was structurally complete after Phase 3. What's left has the same shape as Phase 4's "fix the parts that didn't actually land" - IMAP cancellation specifically. Calendar and GAL are smaller relocations (single function each, no cross-store complexity).

The phase ships as one milestone with a clean commit-level split: IMAP cancellation depth → Calendar IPC + Runtime → GAL kick IPC → UI teardown of `sync_calendars` / `refresh_gal_caches` → SyncTick collapse → docs. A regression should bisect to the right commit.

## Scope

### In scope

- **Prerequisite: retire the `rtsk -> service` shim.** See § "Prerequisite: break the rtsk -> service shim". This is a precondition; calendar work is blocked on it.
- **IMAP cancellation depth.** Add `cancellation_token` argument to the per-folder loop entry points in `crates/imap/src/imap_initial.rs` and `crates/imap/src/imap_delta.rs`. Insert **point-checks** (`if cancellation_token.is_cancelled() { return Cancelled }`) at: folder-list iteration boundary, per-folder fetch entry, per-batch persist entry, between RPCs in helpers (`imap_delta::batch_delta_check`, `imap_delta_janitor`, `client::sync`). **Do not use `tokio::select!` mid-FETCH** - dropping a future mid-FETCH leaves the IMAP session with unread response data on the wire, breaking the next command. Point-checks-between-RPCs gives folder-and-RPC-boundary cancellation, which matches JMAP's actual pattern. The `let _cancellation_token = cancellation_token;` markers indicate the entry points to start from. Mirrors Phase 3 task 6 for IMAP specifically.
- **Calendar cancellation plumbing (task 3a, `&ReadDbState` preserved).** Thread `&CancellationToken` through `crates/calendar/src/sync.rs::calendar_sync_account_impl`, the public `calendar_sync_account` wrapper, the three provider paths (`sync_google_calendar_account`, `sync_graph_calendar_account`, `sync_caldav_calendar_account`), and the per-calendar event-sync loops. Same shape as the IMAP work: point-checks at calendar-list-entry, per-calendar-entry, per-event-batch boundaries. Signature stays `&ReadDbState` for this commit (UI callers still call this directly; flipping to `&WriteDbState` would force `app -> service-state`, which is forbidden by `docs/architecture.md:47`). Without 3a, `CalendarRuntime::cancel_account` is a stub.
- **Calendar `WriteDbState` migration (task 3b, lands with Service runtime).** `calendar_sync_account_impl` today writes through `ReadDbState::with_conn` (a write-surface escape). The signature flip to `&WriteDbState` and the **deletion of UI-side `sync_calendars`** land in the same commit as `CalendarRuntime`'s introduction (the only remaining caller is the new Service runtime, which has a `WriteDbState`). Don't smuggle the escape into the Service. The `db.write_db_state()` UI helper currently returns `ReadDbState`; that's fine - it stays untouched, and the deleted UI-side calendar sync caller doesn't need the type at all.
- **`service-api` calendar wire types.** New `crates/service-api/src/calendar.rs` with:
  - `CalendarRunId(uuid::Uuid)` (`new_v7`).
  - `CalendarStartAccountSyncParams { account_id }`, `CalendarStartAck { account_id, run_id, already_in_flight }`.
  - `CalendarCancelAccountSyncParams { account_id }` (explicit-request cancel only - the account-deletion path piggybacks server-side, see § "Account-deletion integration").
  - `CalendarCancelAck { account_id, run_id: Option<CalendarRunId> }` - mirrors `SyncCancelAck`. The `run_id` lets the client await the corresponding `CalendarRunCompleted` for terminal-completion semantics. `None` means no run was in flight at cancel time.
  - `CalendarSyncResult { Completed | Cancelled | Failed(String) }`.
  - **Two notifications**, mirroring the dual-routing decision in § Architecture:
    - `Notification::CalendarRunCompleted { account_id, run_id, result, mutated: bool, service_generation }` - class `MustDeliver`. Consumed inside `ServiceClient` by per-`run_id` awaiters (mirrors `SyncCompleted`). Not enqueued to the UI. The `mutated` flag is informational on this notification; the UI reload signal is `CalendarChanged`.
    - `Notification::CalendarChanged { account_id, service_generation }` - class `Coalesce`, **`CoalesceKey::CalendarChanged { account_id }`** (account-scoped: two notifications for different accounts pass through; two for the same account collapse to one). Dispatched to UI for view reload; debounced UI-side. Fires whenever `mutated == true`, regardless of run result (`Completed`/`Cancelled`/`Failed` after a partial commit all emit). See § "CalendarChanged: partial-mutation emission" for why successful-runs-only was wrong.
  - `RequestParams::CalendarStartAccountSync` (5 s timeout) and `RequestParams::CalendarCancelAccountSync` (5 s timeout).
  - `ClientNotification::CalendarKick` (class `Drop`) and `ClientNotification::GalKick` (class `Drop`). Following the `pending_ops.kick` shape.
- **`crates/service/src/calendar.rs`: `CalendarRuntime`.** New file. Per-account map keyed by `account_id`; panic supervisor wrapping each runner; `closed: AtomicBool` shutdown guard mirroring `PushRuntimeInner` (NOT `SyncRuntime` - that crate doesn't have the flag); `start_account -> Result<CalendarStartAck, String>` (`Err` on post-shutdown calls); `cancel_account -> CalendarCancelAck` (returns `run_id: Option<CalendarRunId>` for terminal-completion correlation); `shutdown`. **Per-runtime semaphore** (mirroring `SyncRuntime`) bounds concurrent runners; default cap matches `SyncRuntime`'s. Without it, a Service respawn within the 5-min cadence + an in-memory `last_calendar_sync` reset triggers an N-account thundering herd of parallel TLS sessions and DB writes. No marker-file lifecycle - calendar sync is idempotent against provider state (CTags / ETags) and doesn't touch the four-store cluster, so no invariant-pass entry. Mirrors `SyncRuntime`'s API surface where it makes sense; diverges where invariants differ (pin in the type doc-comment). Each runner tracks a `mutated: bool` flag set when it commits a calendar-table write; the flag is read at run completion to decide whether to emit `CalendarChanged` alongside `CalendarRunCompleted`.
- **`crates/service/src/handlers/calendar.rs`: handlers.** `handle_start_account_sync` for the request; `handle_cancel_account_sync` for the cancel request; `handle_calendar_kick` for the notification. The kick handler enumerates accounts whose calendar sync is stale (per a Service-side `last_calendar_sync` per-account timestamp; staleness threshold 1h) and spawns runners.
- **`crates/service/src/handlers/gal.rs`: handler.** `handle_gal_kick` notification handler. Iterates **all** accounts (`refresh_gal_for_account` already returns `Ok(0)` for unsupported providers - no `enumerate_supported_accounts` helper needed) and calls `rtsk::contacts::gal::refresh_gal_for_account` per account with the existing 60 s per-account timeout. **Serialize handler invocations** via a single global Tokio `Mutex<()>` at the handler entry. The hazard is per-account (two concurrent stale-account kicks duplicate provider calls because `refresh_gal_for_account` only writes the cache after the network round-trip completes), so the load-bearing fix lives inside `refresh_gal_for_account` (per-account in-flight set). The global handler mutex is the cheaper coarsening: it prevents *any* concurrent kicks. Acceptable because `NOTIFY_CAP = 4` only fires concurrent handlers under burst load, and GAL kicks are 5-min cadenced. Future-work direction: per-account `HashMap<AccountId, Mutex<()>>` inside the handler, or per-account in-flight set inside `refresh_gal_for_account` itself. Caller-side filter (`provider in {graph, gmail_api}` at `handlers/provider.rs:99`) goes away; widens the per-kick DB read load by two `with_conn` reads per account (`gal_cache_age` + `get_account_provider_sync`) - steady-state cost negligible at typical N. No per-account runtime.
- **Notification-drain bound.** The current `drain_in_flight(&notifications_in_flight)` in `dispatch.rs` awaits unbounded - a wedged GAL handler can stall shutdown by up to N×60s. Add a hard cap (proposed: 5s aggregate) past which the drain logs a warning and aborts the remaining notification tasks. Phase 4 added a similar `stop_push` ceiling for the same class of problem. **Caveat to document at the abort site:** `refresh_gal_for_account` performs DB writes via `tokio::task::spawn_blocking` (`crates/core/src/contacts/gal.rs:212`). Aborting the outer async task does not stop a running blocking closure; the blocking work runs to completion. Acceptable for GAL because the writes are bounded and idempotent. Any future notification handler with abortable behavior must either keep its blocking work cancellation-aware or accept the same contract; the doc-comment in § "Code-comment requirements" calls this out.
- **Account-deletion integration (piggyback model).** Today `client.cancel_and_await` (`crates/app/src/handlers/core.rs:686` -> `crates/app/src/service_client.rs:884`) sends *one* IPC: `RequestParams::SyncCancelAccount`. Push cancel is **piggybacked server-side** inside `handle_cancel_account` (`crates/service/src/handlers/sync.rs:77-79`); the UI does not fan out a separate push-cancel request. Phase 5 mirrors this pattern for calendar: `handle_cancel_account` also calls `CalendarRuntime::cancel_account` after the existing push piggyback, and the response carries `calendar_run_id: Option<CalendarRunId>` alongside the existing sync `run_id`. The client uses both run_ids to await both terminal completions (`SyncCompleted` + `CalendarRunCompleted`) before issuing the DELETE. Calendar tables CASCADE from `accounts` (`crates/db/src/db/schema/05_calendar.sql:5`); the cancel-and-await guarantee prevents a runner's open `WriteDbState` write borrow from racing the DELETE FROM accounts. The `RequestParams::CalendarCancelAccountSync` request type is reserved for **explicit-request** cancel only (manual sync now, RSVP-then-resync), not for deletion.
- **Calendar runtime drain in the consolidated drain.** Order: `PushRuntime -> CalendarRuntime -> SyncRuntime -> search-writer -> sentinel`. Ordering is **reserved**, not currently load-bearing - see § "Drain ordering: reserved, not load-bearing today" for rationale.
- **`Message::SyncTick` collapse + dead-tick removal.** Replace UI-side `sync_calendars` and `refresh_gal_caches` calls with IPC sends (`ClientNotification::CalendarKick` and `ClientNotification::GalKick`). Cadence stays on the existing 5-min `SyncTick`; staleness gating is Service-side (calendar: per-account `last_calendar_sync` timestamp tracked by the `CalendarRuntime`'s kick handler; GAL: existing 24h cache check in `refresh_gal_for_account`). Delete the dead `Message::GalRefreshTick` placeholder and its `iced::time::every` subscription (`subscription.rs:108-112`, `update.rs:697-705`).
- **UI teardown.** Delete `crates/app/src/handlers/provider.rs::sync_calendars`. Delete `crates/app/src/handlers/provider.rs::refresh_gal_caches`. Replace their call sites in `update.rs::Message::SyncTick`.
- **UI: `Notification::CalendarChanged` arm with debouncing.** UI-side dispatcher routes `CalendarChanged` notifications to a debounced reload (proposed: 250ms trailing-edge) so N accounts completing a kick batch produce one reload, not N. `Notification::CalendarRunCompleted` is consumed inside `ServiceClient` - never reaches the UI dispatcher.
- **Code-comment requirements** mirroring the phase-4 pattern (see § "Code-comment requirements" below).
- **Doc updates** to `problem-statement.md` (new "Phase 5 status (as landed)" block + correction to the "remaining UI write surfaces" inventory: calendar event mutations stay UI-side this phase) and `implementation-roadmap.md` (corrections to the Phase 5 entry's scope claims).

### Out of scope

- **IMAP IDLE.** Lands when IMAP IDLE itself lands in the codebase; will follow Phase 4's `PushRuntime` pattern.
- **Provider-protocol improvements** (CONDSTORE/QRESYNC for IMAP, batch APIs for Graph, etc.). Tracked in their own roadmap docs.
- **Calendar event mutations (`cal::actions::*`).** `crates/app/src/handlers/calendar.rs:320,561,585` and friends still call `cal::actions::*` directly with a UI-side `ActionContext` (`crates/app/src/app.rs:335`). Phase 5 relocates only the *periodic provider sync/cache refresh*; event-mutation relocation is Phase 6 territory. The "only remaining UI write surface" claim from the initial draft was wrong.
- **`cal::actions` write-surface escape.** `crates/calendar/src/actions.rs` writes via `&ReadDbState::with_conn` today (the same write-surface escape Phase 4 cleaned up for sync). Phase 5's `WriteDbState` migration covers `calendar_sync_account_impl` only - the action-mutation paths keep the escape until Phase 6 relocates them alongside the rest of `cal::actions`. Called out so it doesn't surface as a "new" finding next phase.
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

UI subscribes to `CalendarChanged` only via the dispatcher; explicit-request callers (post-account-add, manual sync now) await `CalendarRunCompleted` via the per-`run_id` channel.

**Interaction with UI-side mutations.** `cal::actions::*` (event mutations) still runs UI-side this phase and triggers its own calendar reload after each mutation. A subsequent Service-pushed `CalendarChanged` for the same change will fire the debounced reload again - duplicate render against identical DB state. Acceptable; the duplicate render cost is small, and the consolidation lands when calendar event mutations relocate Service-side in Phase 6 (one path, one reload signal).

### CalendarChanged: partial-mutation emission

`CalendarChanged` cannot be conditioned on `result == Completed`. `crates/calendar/src/sync.rs:249` upserts discovered calendars before per-calendar event loops execute, and per-calendar results are applied independently (line 263). A run cancelled or failed *after* a committed batch has already mutated calendar rows; the UI must reload to surface them.

Concrete emission rule: each `CalendarRuntime` runner carries a `mutated: bool` flag set on the first calendar-table write of the run. At terminal completion (`Completed`/`Cancelled`/`Failed`), the runner emits `CalendarRunCompleted` unconditionally and emits `CalendarChanged` if `mutated == true`. The `mutated` flag also rides along on `CalendarRunCompleted` (informational; explicit-request callers can use it to decide whether their own UI refresh is worth a debounce-skip).

This is symmetric with how `SyncCompleted` already carries enough metadata for the UI to know whether a reload is needed - the `CalendarChanged` notification is the dispatch-layer expression of the same fact.

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
// Global handler mutex - coarsens the load-bearing per-account hazard
// (concurrent stale-account kicks duplicate provider calls because
// refresh_gal_for_account writes the cache after the network round-trip).
// The cleanest fix lives inside refresh_gal_for_account (per-account in-flight
// set); this global lock is the cheaper "no concurrent kicks at all" form.
// NOTIFY_CAP = 4 means handlers run concurrently by default.
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
- **The global `Mutex` is correct but coarsens the real hazard.** Notification dispatcher runs handlers concurrently up to `NOTIFY_CAP = 4` (`dispatch.rs:33,461`). Without any lock, two concurrent `gal.kick` invocations both call `refresh_gal_for_account` for the same stale account and duplicate the provider round-trip. The hazard is per-account; the global lock prevents *any* concurrent kicks (slow account A blocks fresh accounts B..N's cache-age checks for the slow run's duration). Acceptable at 5-min cadence + 24h cache - the lock is held only while the iteration runs sequentially through accounts, and most calls hit the cache-age short-circuit. Future-work direction: per-account `HashMap<AccountId, Mutex<()>>` inside the handler, or per-account in-flight set inside `refresh_gal_for_account`. Tested explicitly in the Phase 5 unit tests (assert single in-flight per-account at any moment - the test passes against either the global-lock form or a per-account form, so future tightening doesn't invalidate the test).
- **`spawn_blocking` interaction with notification-drain abort.** `refresh_gal_for_account` performs DB writes via `tokio::task::spawn_blocking` (`crates/core/src/contacts/gal.rs:212`). If shutdown's notification-drain timeout fires and aborts the GAL handler's outer async task, the blocking closure runs to completion regardless. Acceptable because the writes are bounded and idempotent; documented as the contract for any future abortable notification handler.
- **Widened DB read load.** Today's UI-side `refresh_gal_caches` filters callers to `provider in {graph, gmail_api}` (`handlers/provider.rs:99`); the Service-side handler iterates all accounts (`refresh_gal_for_account` self-gates non-supported providers with `Ok(0)` at `crates/core/src/contacts/gal.rs:208`). Adds two `with_conn` reads per account per kick (`gal_cache_age` + `get_account_provider_sync`). Steady-state cost is small at typical N; flagged so it doesn't surface later as a "silent regression."

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

0a. **Extract shared action types out of `service::actions`.** Move `ActionContext`, `ActionError`, `ActionOutcome`, `MutationLog` into a new crate (`action-types`) or extend `service-state`. Both `cal::actions` and `service::actions` depend on the new crate. This is what makes 0b safe - without it, breaking `rtsk -> service` strands `cal::actions`'s imports.

0b. **Retire the `rtsk -> service` edges.** Three concrete cuts:
   - Delete the action-shim modules `rtsk::actions::*` (Phase 2 transitional layer).
   - Delete the `service::sync_dispatch` re-export at `crates/core/src/lib.rs:55`; update the handful of callers to import from `service` directly (in their new home, since `app`/`service` are the legitimate consumers).
   - Migrate `crates/core/src/chat.rs`'s `service::actions/provider/pending` imports into a more appropriate crate (likely inlining into `service` if `core::chat`'s callers are app-side).
   - Remove `service = { path = "../service" }` from `crates/core/Cargo.toml`. Verify `cargo metadata` is cycle-clean.

0c. **Add new dependency edges to `crates/service/Cargo.toml`.** `cal = { path = "../calendar" }` for `CalendarRuntime`. `rtsk = { path = "../core" }` for the GAL handler. Both legal because 0b removed the inverse edge. (Fallback if 0a turns out larger than expected: option 3 in § "Prerequisite" - inline calendar-sync entry points directly into `service::calendar` so `service` doesn't import `cal`. `rtsk` dependency still needs adding either way.)

**Main task list:**

1. **`service-api`: calendar wire types.** New `crates/service-api/src/calendar.rs`: `CalendarRunId`, `CalendarStartAccountSyncParams`, `CalendarStartAck`, `CalendarCancelAccountSyncParams`, `CalendarSyncResult`. Two notification variants:
   - `Notification::CalendarRunCompleted { account_id, run_id, result, service_generation }` - class `MustDeliver`. Mirrors `SyncCompleted` routing.
   - `Notification::CalendarChanged { account_id, service_generation }` - class `Coalesce`. UI-dispatched.
   
   `RequestParams::CalendarStartAccountSync` and `RequestParams::CalendarCancelAccountSync` (5 s timeout each). `ClientNotification::CalendarKick` and `ClientNotification::GalKick` variants (class `Drop`). Catalog tests inline at `crates/service-api/src/notification.rs` and `client_notification.rs`. Type-only commit.

2. **IMAP cancellation depth.** Edit `crates/imap/src/imap_initial.rs`, `crates/imap/src/imap_delta.rs`, `crates/imap/src/imap_delta_janitor.rs`, `crates/imap/src/client/sync.rs`: remove the `let _cancellation_token = cancellation_token;` markers; thread the token into the per-folder loop, per-batch persist points, and helper paths (`batch_delta_check` etc.). **Use point-checks (`if cancellation_token.is_cancelled() { return Cancelled }`) between RPCs, not `tokio::select!`** - rationale in § "Cancellation: runtime -> handler -> provider chain". Mirror Phase 3 task 6's actual checkpoint shape for JMAP.

3a. **Calendar cancellation plumbing (`&ReadDbState` preserved).** Thread `&CancellationToken` through `crates/calendar/src/sync.rs` (`calendar_sync_account_impl`, `calendar_sync_account`, `sync_google_calendar_account`, `sync_graph_calendar_account`, `sync_caldav_calendar_account`, per-calendar event loops) and `rtsk::caldav::sync::sync_caldav_calendars`. Same point-check shape as IMAP. **Signature stays `&ReadDbState`**: flipping it would force `app -> service-state`, forbidden by `docs/architecture.md:47`. UI callers continue to use the existing entrypoint unchanged. This commit is purely calendar-crate-internal; no Service code yet.

3b. **`crates/service/src/calendar.rs`: `CalendarRuntime` + `WriteDbState` flip + UI caller deletion.** Three things land together because they're co-dependent:
   - New `CalendarRuntime`: per-account map, panic supervisor (Phase 3 pattern), `closed: AtomicBool` (mirroring `PushRuntimeInner` - **NOT** `SyncRuntime`, which doesn't have the flag), per-runtime semaphore (mirrors `SyncRuntime`), `start_account -> Result<CalendarStartAck, String>` / `cancel_account -> CalendarCancelAck` / `shutdown`. Tracks `mutated: bool` per-run for the `CalendarChanged` emission rule.
   - Flip `calendar_sync_account_impl`'s parameter from `&ReadDbState` to `&WriteDbState`. Service holds a `WriteDbState`; the runtime constructs it from `BootSharedState::db_conn()` + `encryption_key()`.
   - Delete `crates/app/src/handlers/provider.rs::sync_calendars` and its `Message::SyncTick` call site (replaced by the IPC kick in task 10). This is what makes the signature flip safe - no UI-side caller is left.
   
   Module-level doc-comment carries the code-comment requirements.

4. **`crates/service/src/handlers/calendar.rs`: handlers.** `handle_start_account_sync` translates the request into `CalendarRuntime::start_account` + serializes the ack (or `Err`). `handle_cancel_account_sync` calls `CalendarRuntime::cancel_account`. `handle_calendar_kick` enumerates accounts whose `last_calendar_sync` is > 1h stale and starts each.

5. **`crates/service/src/handlers/gal.rs`: handler.** `handle_gal_kick` iterates all accounts (no enumerate-supported helper - `refresh_gal_for_account` self-gates) and calls `refresh_gal_for_account` per account with the existing 60 s per-account timeout. **Required:** Tokio `Mutex` on the handler entry point so the `NOTIFY_CAP=4` concurrent-handler dispatcher can't double-fire stale-account fetches. Unit test for the mutex behavior.

6. **`BootSharedState`: install slot for `CalendarRuntime`.** Mirror the `push_runtime` slot pattern: `install_calendar_runtime`, `calendar_runtime`, `take_calendar_runtime`. Boot installs after `SyncRuntime` (since it needs `db_conn` + `encryption_key`). `install_*` is a no-op-if-already-installed guard, mirroring push.

7. **Drain consolidation: insert calendar step + bound notification drain.** `dispatch.rs`'s consolidated drain inserts `CalendarRuntime::shutdown()` between Push and Sync. Same commit: bound `drain_in_flight(&notifications_in_flight)` with a 5 s aggregate cap; past it, log + abort remaining notification tasks. Update the doc-comment on the orchestrating block to reflect both changes.

8. **Dispatch wire-up.** `crates/service/src/dispatch.rs`: register handler arms for `RequestParams::CalendarStartAccountSync`, `RequestParams::CalendarCancelAccountSync`, `ClientNotification::CalendarKick`, `ClientNotification::GalKick`. Mirror the existing `PendingOpsKick` arm pattern.

9. **Account-deletion piggyback: `handle_cancel_account` calls calendar.** Today push-cancel is piggybacked server-side inside `crates/service/src/handlers/sync.rs::handle_cancel_account` (lines 77-79); the UI does not fan out a separate push-cancel IPC. Phase 5 mirrors this for calendar: extend `handle_cancel_account` to call `CalendarRuntime::cancel_account`, and extend the response (`SyncCancelAck` -> shape that also carries `calendar_run_id: Option<CalendarRunId>`). On the client, `service_client.rs::cancel_and_await` awaits both sync and calendar terminal completions (`SyncCompleted` + `CalendarRunCompleted`) before issuing the DB DELETE. This is *not* a UI-side `join!` and does not require `RequestParams::CalendarCancelAccountSync` (that request is reserved for explicit-request cancel - manual sync now, RSVP-then-resync). Required for deletion safety: calendar tables CASCADE from `accounts`.

9b. **Explicit-request calendar path wiring.** Mirrors the existing sync request shape end-to-end:
   - `ServiceClient`: pending-run map keyed by `CalendarRunId` (mirrors `pending_sync_runs`); reader-task arm consumes `Notification::CalendarRunCompleted` and resolves the matching one-shot.
   - Respawn failure handling: on Service respawn, walk the pending map and resolve each one-shot with `CalendarSyncResult::Failed("service_respawned")` (mirrors the sync-side respawn behavior).
   - UI helper `client.start_calendar_sync(account_id) -> impl Future<Output = CalendarSyncResult>`: sends `RequestParams::CalendarStartAccountSync`, registers the returned `run_id` in the pending map, awaits the one-shot.
   - Call-site updates: post-account-creation in `handlers/account_*.rs`, manual "Sync now" button (if/where it exists in the calendar tab), and the RSVP-then-resync flow (Phase 6 will fold this into the action pipeline; for Phase 5 we just call `start_calendar_sync` from the existing RSVP success path).

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
- `crates/core/Cargo.toml` - **delete** `service = { path = "../service" }` line (Prerequisite 0a). Also delete the `service::sync_dispatch` re-export at `crates/core/src/lib.rs:55` and migrate `core::chat`'s `service::actions/provider/pending` imports out of `core` (likely into the action shim's new home, see prerequisite).
- `crates/service/Cargo.toml` - **add** `cal = { path = "../calendar" }` and **add** `rtsk = { path = "../core" }` (Prerequisite 0b - the latter is required for the GAL handler's `rtsk::contacts::gal::refresh_gal_for_account` call).
- (new) `crates/action-types/Cargo.toml` (or extension to `service-state`) - new home for `ActionContext`, `ActionError`, `ActionOutcome`, `MutationLog` so `cal::actions` can keep importing them without re-introducing the cycle. See § "Prerequisite" option 1.
- `crates/calendar/Cargo.toml` - swap `rtsk` action-type imports for the new shared crate (see prerequisite).
- `crates/service-api/src/lib.rs` - re-export calendar types.
- `crates/service-api/src/notification.rs` - add `CalendarRunCompleted { account_id, run_id, result, mutated, service_generation }` (MustDeliver) + `CalendarChanged { account_id, service_generation }` (Coalesce, `CoalesceKey::CalendarChanged { account_id }`) variants + arms + catalog tests (including the same-account / different-account coalesce-key behavior).
- `crates/service-api/src/client_notification.rs` - add `CalendarKick`, `GalKick` variants + class arms (both `Drop`).
- `crates/service-api/src/request.rs` - add `CalendarStartAccountSync` and `CalendarCancelAccountSync` variants + 5 s timeouts. Extend `SyncCancelAck` (or replace with a new shape) so the response carries `calendar_run_id: Option<CalendarRunId>` alongside the existing sync `run_id` for the deletion-piggyback path.
- `crates/imap/src/imap_initial.rs` - thread cancellation through per-folder loop with point-checks (no `tokio::select!`).
- `crates/imap/src/imap_delta.rs` - same; covers `batch_delta_check`.
- `crates/imap/src/imap_delta_janitor.rs` - thread cancellation through helper.
- `crates/imap/src/client/sync.rs` - thread cancellation through client-level helpers.
- `crates/calendar/src/sync.rs` - **substantial change**: add `&CancellationToken` parameter to `calendar_sync_account_impl`, `calendar_sync_account`, `sync_google_calendar_account`, `sync_graph_calendar_account`, `sync_caldav_calendar_account`, and per-calendar event loops. Switch `&ReadDbState` to `&WriteDbState` (write-surface escape fix).
- `crates/core/src/caldav/sync.rs` - `sync_caldav_calendars` accepts cancellation token + point-checks.
- `crates/service/src/boot.rs` - install `CalendarRuntime` slot + construction.
- `crates/service/src/boot_state.rs` (or wherever `BootSharedState` lives) - `install_calendar_runtime`, `calendar_runtime`, `take_calendar_runtime`.
- `crates/service/src/dispatch.rs` - drain step insertion (PushRuntime -> CalendarRuntime -> SyncRuntime); 5 s notification-drain bound; handler dispatch arms for the new request and notification types.
- `crates/service/src/handlers/sync.rs::handle_cancel_account` - extend to call `CalendarRuntime::cancel_account` after the existing push piggyback; return the calendar `run_id` in the ack response.
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

7. **`crates/service/src/calendar.rs` per-runner emission rule** (where `CalendarRunCompleted` and `CalendarChanged` get sent) must have an inline comment:
   - "`CalendarChanged` cannot be conditioned on `result == Completed`. `crates/calendar/src/sync.rs:249` upserts discovered calendars before per-calendar event loops execute, and per-calendar results are applied independently (line 263). A run cancelled or failed *after* a committed batch has already mutated calendar rows; the UI must reload to surface them. Emit `CalendarChanged` whenever `mutated == true`, regardless of `result`. The `mutated` flag also rides along on `CalendarRunCompleted` for explicit-request callers."

8. **`crates/service/src/dispatch.rs` notification-drain abort site** must have an inline comment:
   - "Notification handlers that wrap blocking work in `tokio::task::spawn_blocking` (GAL is the live example - `crates/core/src/contacts/gal.rs:212`) will see the *outer* future aborted on drain timeout, but the blocking closure runs to completion regardless. Acceptable because GAL writes are bounded and idempotent. Any handler added later that doesn't satisfy that contract must either keep its blocking work cancellation-aware or accept the same drain-timeout semantics."

These comment texts are the contract; reviewers will reject commits that reword them in ways that lose the *why*.

## Test plan

### Unit tests

- `service-api`: serde round-trip for `CalendarRunId`, `CalendarStartAck`, `CalendarCancelAck`, `CalendarSyncResult`, `CalendarRunCompleted` (with `mutated`), `CalendarChanged`, `CalendarCancelAccountSyncParams`. `RequestParams::CalendarStartAccountSync.timeout()` and `RequestParams::CalendarCancelAccountSync.timeout()` return 5 s. Catalog cases for `CalendarRunCompleted` (class `MustDeliver`, method name, generation round-trip, `parse_service_message` round-trip) and `CalendarChanged` (class `Coalesce`, `CoalesceKey::CalendarChanged { account_id }`). Coalesce-queue test: two `CalendarChanged` for the same account collapse; two for different accounts both pass through. Catalog cases for `ClientNotification::CalendarKick` and `GalKick` (class `Drop`).
- `service::calendar`: `CalendarRuntime::cancel_account` returns `CalendarCancelAck { run_id: None }` when no entry exists and `Some(run_id)` for an in-flight runner; `shutdown` is safe on empty runtime; `start_account` returns `Err` after `shutdown` (the unit-testable invariant from PushRuntime's review-pass pattern); a runner observing a flipped cancellation token returns `Cancelled` (requires task 3a's plumbing - the test would be degenerate without it); `mutated` flag flips on first calendar-table write and stays true through subsequent batches (drives `CalendarChanged` emission); semaphore caps concurrent runners at the configured limit.
- `service::calendar` partial-mutation emission: simulate a runner that writes one calendar batch and then observes a cancellation; assert both `CalendarRunCompleted { result: Cancelled, mutated: true }` and `CalendarChanged` are emitted.
- `imap` cancellation: drive `imap_initial_sync` (or its testable subroutine) with a pre-cancelled token; assert it returns the cancelled error path before any network round-trip. Drive with a token that flips mid-folder-iteration; assert the loop breaks at the next point-check, not at the end. Cover `batch_delta_check`, `imap_delta_janitor`, and `client::sync` helpers explicitly.
- `calendar` cancellation: same shape against `calendar_sync_account_impl` with a stub provider for each of the three paths (Google, Graph, CalDAV).
- `service::handlers::gal`: **mutex behavior test.** Send two `gal.kick` notifications nearly-simultaneously through a test dispatch with `NOTIFY_CAP=4`; stub `refresh_gal_for_account` to record calls per account; assert at most one in-flight call per account at any moment. The assertion is shape-stable across "global mutex" and "per-account in-flight set" - a future tightening from the former to the latter doesn't invalidate the test.
- `service::dispatch`: notification-drain timeout test. Wedge a notification handler with `tokio::time::sleep(60s)`; trigger drain; assert it returns within ~5 s and logs the abort warning.

### Integration tests (in-process)

- `calendar_kick_starts_sync_in_service`: spin up a `CalendarRuntime` against a stub provider; send a `calendar.kick` notification; assert per-account starts fire only for accounts whose `last_calendar_sync` is > 1h stale. Same caveat as Phase 4: real provider integration needs a fake CalDAV fixture, deferred.
- `calendar_run_completed_consumed_by_service_client`: assert `Notification::CalendarRunCompleted` is consumed inside `ServiceClient` (per-`run_id` broadcast subscriber) and does NOT reach the UI `ServiceNotification` queue.
- `calendar_changed_routed_to_ui_with_debounce`: assert `Notification::CalendarChanged` reaches the UI dispatcher; N near-simultaneous notifications produce one debounced `reload_calendar_events()` call.
- `calendar_drains_before_sync_at_shutdown`: instrumented; assert no calendar `start_account` is called after `SyncRuntime::shutdown` begins. **Test docstring must say**: this enforces a reserved-not-load-bearing invariant. When a future commit wires calendar-cancel cleanup to dispatch action plans (RSVP send), the same commit drops the `// reserved` comment in `service/src/dispatch.rs` and the test transitions to load-bearing. Don't relax this test ahead of that commit.
- `cancel_and_await_cancels_calendar`: simulate account-delete; assert `client.cancel_and_await` resolves only after both `SyncCompleted` and `CalendarRunCompleted` arrive for the deleted account. Verifies the piggyback path inside `handle_cancel_account` calls `CalendarRuntime::cancel_account` and the response carries `calendar_run_id`.

### Real-subprocess smoke tests

- `service_subprocess_calendar_kick_routes_to_handler`: spawn the Service with a seeded calendar account; observe a `calendar.kick` notification reaches the handler (via a debug log assertion). No actual CalDAV traffic - that needs the fixture.
- `service_subprocess_imap_cancel_interrupts_mid_sync`: spawn with an IMAP-stub account; trigger an initial sync; cancel mid-sync; assert `SyncCompleted { result: Cancelled }` arrives within a bounded window. **Latency bound depends on the test fixture, not the production behavior**: against the in-process IMAP stub (which flushes promptly between RPCs), assert < 2 s. Against a real provider, the worst-case bound is `IMAP_FETCH_TIMEOUT` (`crates/imap/src/connection.rs`) - tens of seconds - because point-checks-between-RPCs only observes cancellation when the *next* RPC enters the loop. The verification block reflects this distinction so a real-provider QA pass doesn't fail a 2 s assertion. Tests the cancellation-depth fix at RPC-boundary granularity.
- `service_subprocess_calendar_cancel_on_account_delete`: spawn with a seeded account; start a calendar sync; trigger account delete; assert no calendar table writes complete after the DELETE FROM accounts (no SQLITE_BUSY, no orphan rows).

### Manual matrix updates

- The "what survives a Service crash" matrix in `problem-statement.md` § "Cross-store crash consistency" gets a new row: "Calendar sync state". Phase 5 outcome: idempotent re-fetch on next sync (the runtime has no marker; the calendar provider's CTag / ETag handling re-fetches what changed). No torn-write recovery needed.

## Open questions

(Most original open questions resolved during the post-review revision; see § "Revision history".)

1. **`CalendarRuntime` per-account concurrency.** SyncRuntime is one-runner-per-account; calendar could plausibly be one-runner-per-account-per-calendar (an account may host multiple calendar collections). For Phase 5 we mirror SyncRuntime's per-account-only granularity; if a benchmark surfaces lock contention on a single account with many calendars, revisit. Not blocking for the phase.
2. **Debounce window for `CalendarChanged`-driven reload.** Plan proposes 250 ms trailing-edge. Alternatives: 0 ms (immediate, accept N reloads for N accounts), or a leading-edge + trailing-edge combo (immediate first reload, debounced last). 250 ms trailing is the simplest and matches typical UI debounce defaults; revisit if calendar tab feels laggy on multi-account kick batches.
**Resolved during this revision** (do not re-litigate):
- Cadence: 5-min `SyncTick` with Service-side staleness gating, no separate hourly subscription.
- Notification routing: split into `CalendarRunCompleted` (MustDeliver, ServiceClient-consumed) + `CalendarChanged` (Coalesce, UI-dispatched, `CoalesceKey::CalendarChanged { account_id }`).
- `CalendarChanged` emission: fires whenever `mutated == true`, regardless of run result.
- GAL handler concurrency: serialized via global Tokio `Mutex<()>` as a coarsening of the per-account hazard. Future tightening to per-account form is documented but out of scope.
- `last_calendar_sync` location: in-memory on `CalendarRuntime`, paired with a per-runtime semaphore to bound the post-respawn thundering-herd cost. Schema persistence not adopted.
- Account-deletion cancel: piggyback inside `handle_cancel_account` (mirrors push), not a UI-side fan-out. `RequestParams::CalendarCancelAccountSync` is reserved for explicit-request cancel only.
- Cycle break: option 1 (extract action types into a shared crate) with option 3 (inline calendar-sync into `service::calendar`) as the documented mid-implementation fallback.
- Tray-resident scheduler ownership: deferred to Phase 9 with explicit TODO marker.

## Verification (end-to-end)

- A change pushed to a JMAP mailbox triggers a sync inside the Service (Phase 4 verification carry-over).
- An IMAP initial sync started mid-fetch and then cancelled returns `SyncCompleted { result: Cancelled }` at the next RPC boundary, not after the full folder list completes. Worst-case latency is bounded by `IMAP_FETCH_TIMEOUT` against a real provider (tens of seconds); against the in-process IMAP stub, < 2 s. RPC-boundary granularity is fine; mid-RPC interruption is not in scope.
- A calendar sync started and then cancelled returns `CalendarRunCompleted { result: Cancelled }` within seconds (verifies task 3a's plumbing reaches the leaf).
- A `Message::SyncTick` from the UI fires three notifications + one request fan-out (sync_all_accounts requests, pending_ops kick, calendar kick, gal kick) and zero UI-side provider work.
- The deleted `Message::GalRefreshTick` subscription is gone from `subscription.rs`; no compile or test references the variant.
- Stopping the Service mid-calendar-sync does not corrupt any DB state - drain order holds (calendar before sync before sentinel) and the runner observes cancellation at a checkpoint.
- A wedged GAL handler does not stall shutdown beyond ~5 s (notification-drain timeout fires).
- Two near-simultaneous `gal.kick` notifications produce one provider call per stale account, not two (mutex behavior).
- Account deletion via `cancel_and_await` cancels the calendar runner alongside sync and push (no orphan calendar rows post-DELETE FROM accounts).
- A `Notification::CalendarRunCompleted` is consumed inside `ServiceClient`'s reader task; the UI never sees it. A `Notification::CalendarChanged` reaches the UI dispatcher and triggers a debounced reload.
- A calendar run cancelled or failed *after* a partial-batch commit emits `CalendarChanged` (UI reloads to surface the partial change), not just `CalendarRunCompleted`.
- A Service respawn during a `SyncTick` storm does not produce an N-account thundering herd of parallel calendar runners - the per-runtime semaphore caps concurrency.
- `cargo metadata` reports no `rtsk <-> service` cycle; `service -> rtsk` and `service -> cal` are present; `app` does not depend on `service-state`.
- A user opening a calendar attendee picker sees GAL entries refreshed within ~1 hour of any account being added (the Service's `gal.kick` handler picks it up on the next 5-min tick after the 24h cache age check passes).
- The "Phase 5 status (as landed)" block in `problem-statement.md` documents what Phase 5 actually relocated vs what cascaded from Phase 3, and the "remaining UI write surfaces" inventory correctly lists calendar event mutations as still-UI-side (deferred to Phase 6).

## Promotion criteria

- All Phase 5 tasks landed; IMAP `let _cancellation_token = cancellation_token;` markers are gone; calendar sync stack accepts `&CancellationToken` end-to-end; `Message::GalRefreshTick` is deleted; UI-side `sync_calendars` and `refresh_gal_caches` are deleted; `CalendarRuntime` is in the consolidated drain; notification drain is bounded; GAL handler is serialized; account-delete cancels calendar via the `handle_cancel_account` piggyback.
- Shared action types live in `action-types` (or extended `service-state`); `rtsk -> service` is gone (action shim, `sync_dispatch` re-export, `core::chat`'s direct `service::*` imports all retired); no dependency cycle remains (`cargo metadata` clean). `service -> rtsk` and `service -> cal` are added.
- `Message::SyncTick` does no UI-side provider work.
- Calendar sync writes through `&WriteDbState`, not `&ReadDbState` (for the sync path; `cal::actions` write-surface escape stays until Phase 6 - explicitly out of scope).
- `CalendarRuntime` has a concurrency-bounding semaphore; in-memory `last_calendar_sync` does not regress respawn behavior into a thundering herd.
- `CalendarChanged` fires on partial mutations (cancellation/failure after a committed batch still reloads UI); coalesce key is account-scoped.
- `CalendarCancelAck { run_id: Option<_> }` exists and is used by the deletion-piggyback path; `cancel_and_await` awaits both sync and calendar terminal completions before issuing the DELETE.
- Phase 5 status block added to `problem-statement.md`; remaining UI write-surface inventory updated (calendar event mutations + `cal::actions` write-surface escape both noted as Phase 6); Phase 9 tray-resident TODO marker present.
- `phase-5-plan.md` is then retirement-ready: every deferral has an explicit roadmap entry (the Phase 8 test-cohort carry-forward already exists; Phase 5 just adds its own integration tests to that bucket), every code-comment requirement is present in the relevant file.
