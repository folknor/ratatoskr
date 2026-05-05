# The Service - Phase 5 Plan: port sync to other providers + calendar + GAL relocation

Companion to `phase-1-plan.md`, `phase-1.5-plan.md`, `phase-2-plan.md`, `phase-3-plan.md`, `phase-4-plan.md`. Implements Phase 5 of `implementation-roadmap.md`.

## Revision history

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

- **`Message::SyncTick` collapses to four IPC kicks.** No more UI-side provider work. `sync_calendars` and `refresh_gal_caches` methods on `ReadyApp` (in `handlers/provider.rs`) get deleted entirely; their callers become IPC notification sends. The 1-hour calendar / GAL cadences move to `SyncTick` (5 min) gated by per-account "last calendar sync" / "last GAL refresh" timestamps that the Service tracks - or stay on the 1-hour UI tick and just dispatch via IPC. Decision: keep the cadence UI-side (the 1-hour tick is a simple iced timer), move the work Service-side. UI sends `calendar.kick` and `gal.kick` notifications on the 1-hour cadence; Service handlers iterate accounts.

  Wait, that contradicts the per-account `calendar.start_account_sync` request. Resolution: `calendar.start_account_sync` is for explicit-request paths (manual "sync now", post-account-add, RSVP-then-resync). The hourly tick uses a `calendar.kick` notification (no per-account targeting; Service drains all accounts whose `last_calendar_sync` is stale). Same shape as `pending_ops.kick`. Two surfaces, two semantics.

## Context

Phase 4 closed the JMAP-specific work (push relocation, drain consolidation, OAuth resolver). Phase 5 finishes the email-sync relocation for the remaining three providers and folds in the two non-email subsystems still on the UI's hot path: calendar and GAL.

Most of the email-sync work was structurally complete after Phase 3. What's left has the same shape as Phase 4's "fix the parts that didn't actually land" - IMAP cancellation specifically. Calendar and GAL are smaller relocations (single function each, no cross-store complexity).

The phase ships as one milestone with a clean commit-level split: IMAP cancellation depth → Calendar IPC + Runtime → GAL kick IPC → UI teardown of `sync_calendars` / `refresh_gal_caches` → SyncTick collapse → docs. A regression should bisect to the right commit.

## Scope

### In scope

- **IMAP cancellation depth.** Add `cancellation_token` argument to the per-folder loop entry points in `crates/imap/src/imap_initial.rs` and `crates/imap/src/imap_delta.rs`. Insert checkpoints at: folder-list iteration boundary, per-folder fetch entry, per-batch persist entry. The `let _cancellation_token = cancellation_token;` markers indicate where to start. Mirrors Phase 3 task 6 for IMAP specifically.
- **`service-api` calendar wire types.** New `crates/service-api/src/calendar.rs` with:
  - `CalendarRunId(uuid::Uuid)` (`new_v7`).
  - `CalendarStartAccountSyncParams { account_id }`, `CalendarStartAck { account_id, run_id, already_in_flight }`.
  - `CalendarSyncResult { Completed | Cancelled | Failed(String) }`, `CalendarCompleted { account_id, run_id, result, service_generation }`.
  - `Notification::CalendarCompleted` variant (`MustDeliver`).
  - `Notification::CalendarKick` and `Notification::GalKick` notifications - wait, kicks are *client-to-service*, so they're `ClientNotification` variants in `crates/service-api/src/client_notification.rs`, not on the `Notification` enum. Following the `pending_ops.kick` shape.
  - `RequestParams::CalendarStartAccountSync` with 5 s timeout.
- **`crates/service/src/calendar.rs`: `CalendarRuntime`.** New file. Per-account map keyed by `account_id`; panic supervisor wrapping each runner; `start_account` / `cancel_account` / `shutdown`. No marker-file lifecycle - calendar sync is idempotent against provider state (CTags / ETags) and doesn't touch the four-store cluster, so no invariant-pass entry. Mirrors `SyncRuntime`'s API surface where it makes sense; diverges where invariants differ (pin in the type doc-comment).
- **`crates/service/src/handlers/calendar.rs`: handlers.** `handle_start_account_sync` for the request; `handle_calendar_kick` for the notification. The kick handler enumerates accounts whose calendar sync is stale and spawns runners.
- **`crates/service/src/handlers/gal.rs`: handler.** `handle_gal_kick` notification handler. Enumerates accounts, calls `core::contacts::gal::refresh_gal_for_account` per account (fire-and-forget per account internally; the handler awaits all). No per-account runtime.
- **Calendar runtime drain in the consolidated drain.** Order: `PushRuntime → CalendarRuntime → SyncRuntime → search-writer → sentinel`. Calendar before Sync because calendar paths can produce action-pipeline writes (RSVP send) that need the action worker still alive.
- **UI teardown.** Delete `crates/app/src/handlers/provider.rs::sync_calendars`. Delete `crates/app/src/handlers/provider.rs::refresh_gal_caches`. Replace their call sites in `update.rs::Message::SyncTick` (and any other dispatchers) with `client.send_notification(ClientNotification::CalendarKick)` and `client.send_notification(ClientNotification::GalKick)` calls.
- **`Message::CalendarSyncComplete`** dispatch arm collapses or routes from `Notification::CalendarCompleted` (UI-side dispatcher in `update.rs`).
- **Code-comment requirements** mirroring the phase-4 pattern (see § "Code-comment requirements" below).
- **Doc updates** to `problem-statement.md` (new "Phase 5 status (as landed)" block) and `implementation-roadmap.md` (corrections to the Phase 5 entry's scope claims).

### Out of scope

- **IMAP IDLE.** Lands when IMAP IDLE itself lands in the codebase; will follow Phase 4's `PushRuntime` pattern.
- **Provider-protocol improvements** (CONDSTORE/QRESYNC for IMAP, batch APIs for Graph, etc.). Tracked in their own roadmap docs.
- **GAL fetch over Graph / Google Directory.** `update.rs:697-699` notes that GAL fetch over provider-client APIs (`/users`, Google Directory) is blocked on the sync orchestrator providing account-level clients. That's a structural piece that lives in a future phase, not Phase 5. Phase 5 only relocates the *cache-refresh tick wiring* - if a provider-client orchestration lands later, the `refresh_gal_for_account` body picks it up automatically.
- **Per-account runtime for GAL.** GAL is hourly + idempotent; no runner needed. Phase 5 might revisit if benchmark surfaces a problem.
- **Marker-file lifecycle for `CalendarRuntime`.** Calendar sync is idempotent (CTags / ETags); no four-store invariant pass needed. If a future calendar-attachment-cache surface gets built, the runtime gets the marker treatment then.
- **CalDAV-specific protocol features** (sync-collection REPORT, etc.) - inside the calendar provider; not Phase 5.

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

Mirrors `crates/service/src/sync.rs::SyncRuntime` field-by-field for the lifecycle surface (per-account map, panic supervisor, `closed: AtomicBool` shutdown guard from the Phase 4 review-pass fix). Diverges intentionally on:
- **No marker-file lifecycle.** Calendar sync is idempotent against CalDAV CTags / Exchange ETags; the provider re-fetches whatever changed regardless of whether the previous run completed. No `clear_account_history_id`-equivalent needed.
- **No body / inline / search writer halves.** Calendar writes only to the calendar tables in the main DB.
- **No `service_generation` on the wire ack** (the request method returns `CalendarStartAck` directly), but the `CalendarCompleted` notification carries `service_generation` for cross-respawn safety.

### Cancellation: gel runtime → handler → provider chain

For email sync, Phase 3 set up: `SyncRuntime::cancel_account` flips token → `sync_delta_for_account` checkpoints observe → provider returns `Cancelled`. Phase 5 mirrors this for calendar.

For IMAP specifically, the chain has a missing link. `imap_initial_sync` and `imap_delta_sync` accept the token through their signature but discard it via the `let _cancellation_token = cancellation_token;` markers. Phase 5 plumbs the token into the per-folder loop and the per-batch persist points (mirroring JMAP's per-mailbox / per-batch checkpoints). Token-refresh / principal-resolution checkpoints aren't applicable to IMAP (no OAuth refresh on the sync path; principal is the authenticated user).

### Calendar runtime drain ordering

```text
1. PushRuntime::shutdown()       (Phase 4)
2. CalendarRuntime::shutdown()   (Phase 5 - NEW)
3. SyncRuntime::shutdown()       (Phase 3, relocated in Phase 4)
4. drop Arc<SyncRuntime>         (releases SearchWriteHandle clone)
5. await search-writer JoinHandle (Phase 4 review-pass fix)
6. lifecycle::drain (sentinel)   (Phase 1.5)
7. drop(out_tx); writer_handle.await
```

Calendar drains *before* Sync because calendar runners can dispatch action plans (RSVP send → SMTP via the action worker). The action worker is aborted later in the dispatch shutdown sequence; while sync's runners can be cancelled abruptly without losing email-side state (Phase 3 invariant pass repairs), calendar runners that have an in-flight action need the action worker to still be drainable. By draining calendar first, we ensure a calendar runner observing cancellation can hand off any in-flight action through the standard `client.execute_plan` path before the worker shuts down.

In practice today, no calendar code dispatches actions on the cancellation path - this is forward-looking. Code-comment lock-in below.

### GAL `gal.kick` shape

`ClientNotification::GalKick` (mirroring `PendingOpsKick`). Notification class is `Drop` - a missed kick is harmless; the next hourly tick re-covers. Service handler:

```rust
async fn handle_gal_kick(boot_state: &Arc<BootSharedState>) {
    let Some(db) = boot_state.write_db_state() else { return; };
    let Some(key) = boot_state.encryption_key() else { return; };
    let read_db = db.to_read_state();
    let account_ids = enumerate_supported_accounts(&read_db).await;
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

Note: this preserves the per-account 60 s timeout that today's UI-side `refresh_gal_caches` uses. The handler doesn't need a runtime because GAL is bounded (1-hour cadence × supported accounts × 60 s timeout = bounded total wall time).

Concurrency: serial per-handler-invocation. Two `gal.kick` notifications arriving back-to-back during one in-flight handler would queue (single-handler-at-a-time) or serialize via a Tokio mutex on `BootSharedState`. The simpler choice - serial - is fine for the hourly cadence; if a future optimization parallelizes, that's a follow-up.

### `Message::SyncTick` collapse

Today (post-Phase-4):
```rust
Message::SyncTick => {
    let sync_task = self.sync_all_accounts();        // IPC: sync.start_account per account
    let pending_task = self.process_pending_ops();   // IPC: pending_ops.kick
    let gal_task = self.refresh_gal_caches();        // UI-SIDE: direct DB write
    let cal_task = self.sync_calendars();            // UI-SIDE: direct DB write
    Task::batch([sync_task, pending_task, gal_task, cal_task])
}
```

After Phase 5:
```rust
Message::SyncTick => {
    let sync_task = self.sync_all_accounts();        // IPC: sync.start_account per account
    let pending_task = self.process_pending_ops();   // IPC: pending_ops.kick
    let gal_task = self.kick_gal_refresh();          // IPC: gal.kick (NEW)
    let cal_task = self.kick_calendar_sync();        // IPC: calendar.kick (NEW)
    Task::batch([sync_task, pending_task, gal_task, cal_task])
}
```

Both new methods are tiny `Task::perform(client.send_notification(...))` wrappers. The cadence stays UI-side (the iced `time::every` subscription); only the work moves.

## Detailed task list

In recommended commit order. Each item is one focused commit unless noted.

1. **`service-api`: calendar wire types.** New `crates/service-api/src/calendar.rs`: `CalendarRunId`, `CalendarStartAccountSyncParams`, `CalendarStartAck`, `CalendarSyncResult`, `CalendarCompleted`. `Notification::CalendarCompleted` variant added; `class()` returns `MustDeliver` (the request's caller awaits the matching completion via a per-`run_id` broadcast, mirroring sync). `service_generation` arms exhaustive. `RequestParams::CalendarStartAccountSync` with 5 s timeout. `ClientNotification::CalendarKick` and `ClientNotification::GalKick` variants (class `Drop`). Catalog tests inline at `crates/service-api/src/notification.rs` and `client_notification.rs`. Type-only commit.

2. **IMAP cancellation depth.** Edit `crates/imap/src/imap_initial.rs` and `crates/imap/src/imap_delta.rs`: remove the `let _cancellation_token = cancellation_token;` markers; thread the token into the per-folder loop and per-batch persist points. Add `tokio::select!` checkpoints at every awaited network call inside the per-folder body so cancellation interrupts mid-fetch, not just at folder boundaries. Mirror Phase 3 task 6's checkpoint shape for JMAP.

3. **`crates/service/src/calendar.rs`: `CalendarRuntime`.** Per-account map, panic supervisor (Phase 3 pattern), `closed: AtomicBool` (Phase 4 review-pass pattern), `start_account` / `cancel_account` / `shutdown`. Runs `cal::sync::calendar_sync_account_impl` (the `_impl` form takes a `CancellationToken`; today's wrapper at `cal::sync::calendar_sync_account` doesn't - we either add a `_with_cancel` variant or change the wrapper signature). Module-level doc-comment carries the code-comment requirements.

4. **`crates/service/src/handlers/calendar.rs`: handlers.** `handle_start_account_sync` translates the request into `CalendarRuntime::start_account` + serializes the ack. `handle_calendar_kick` enumerates accounts and starts each.

5. **`crates/service/src/handlers/gal.rs`: handler.** `handle_gal_kick` enumerates supported accounts and calls `refresh_gal_for_account` per account with the existing 60 s per-account timeout.

6. **`BootSharedState`: install slot for `CalendarRuntime`.** Mirror the `push_runtime` slot pattern: `install_calendar_runtime`, `calendar_runtime`, `take_calendar_runtime`. Boot installs after `SyncRuntime` (since it needs `db_conn` + `encryption_key`).

7. **Drain consolidation: insert calendar step.** `dispatch.rs`'s consolidated drain inserts `CalendarRuntime::shutdown()` between Push and Sync. Update the doc-comment on the orchestrating block to reflect the new step.

8. **Dispatch wire-up.** `crates/service/src/dispatch.rs`: register handler arms for `RequestParams::CalendarStartAccountSync`, `ClientNotification::CalendarKick`, `ClientNotification::GalKick`. Mirror the existing `PendingOpsKick` arm pattern.

9. **UI teardown: delete `sync_calendars` and `refresh_gal_caches`.** Delete the methods in `crates/app/src/handlers/provider.rs`. Add `kick_calendar_sync` and `kick_gal_refresh` thin wrappers that send the new client notifications. Update `Message::SyncTick` arm in `update.rs`. Remove `cal::sync::calendar_sync_account` and `rtsk::contacts::gal::refresh_gal_for_account` from the app's call graph (they live in their respective crates, just not called from the app crate anymore).

10. **UI: `Notification::CalendarCompleted` arm.** In `update.rs::Message::ServiceNotification` dispatch and `service_client.rs`'s reader-task routing. Behavior: log + recompute calendar view if the calendar tab is active. Maps to existing `Message::CalendarSyncComplete` for view refresh.

11. **Catalog test: production_notification_catalog gains `CalendarCompleted`.** (`crates/app/src/service_client.rs`).

12. **Test cohort.** Phase 5 unit / integration / real-subprocess tests. Same caveat as Phase 4: integration tests for `CalendarRuntime` lifecycle need either a fake CalDAV server fixture or `test_dummy` constructors on the writer-state types - so the bulk gets Phase 8'd alongside Phase 4's deferred cohort. What CAN land in Phase 5: IMAP cancellation unit tests (drive the per-folder loop with a cancelled token; assert it returns `Cancelled` mid-batch), CalendarRuntime shutdown-guard tests (the start-after-shutdown invariant is unit-testable today), wire-type round-trips for the new calendar/kick types.

13. **Doc updates.** Phase 5 status block in `problem-statement.md`. `implementation-roadmap.md` Phase 5 entry corrected to reflect what actually shipped vs what cascaded from Phase 3. Bundle with the close-out commit per CLAUDE.md's "no markdown-only commits" rule.

## File-by-file changes

**New files:**
- `crates/service-api/src/calendar.rs` - calendar wire types.
- `crates/service/src/calendar.rs` - `CalendarRuntime`.
- `crates/service/src/handlers/calendar.rs` - request + kick handlers.
- `crates/service/src/handlers/gal.rs` - `gal.kick` handler.

**Modified files:**
- `crates/service-api/src/lib.rs` - re-export calendar types.
- `crates/service-api/src/notification.rs` - add `CalendarCompleted` variant + arms + catalog test.
- `crates/service-api/src/client_notification.rs` - add `CalendarKick`, `GalKick` variants + class arms.
- `crates/service-api/src/request.rs` - add `CalendarStartAccountSync` variant + 5 s timeout.
- `crates/imap/src/imap_initial.rs` - thread cancellation through per-folder loop.
- `crates/imap/src/imap_delta.rs` - thread cancellation through per-folder loop.
- `crates/calendar/src/sync.rs` - the `_impl` form may need a `cancellation_token` parameter if it doesn't already.
- `crates/service/src/boot.rs` - install `CalendarRuntime` slot + construction.
- `crates/service/src/dispatch.rs` - drain step insertion + handler dispatch arms.
- `crates/service/src/handlers/mod.rs` - export new handler modules.
- `crates/service/src/lib.rs` - `pub mod calendar`.
- `crates/app/src/service_client.rs` - reader-task `Notification::CalendarCompleted` arm; catalog-test entry.
- `crates/app/src/update.rs` - `Notification::CalendarCompleted` dispatch; `Message::SyncTick` collapse.
- `crates/app/src/handlers/provider.rs` - **delete** `sync_calendars` and `refresh_gal_caches`; **add** `kick_calendar_sync` and `kick_gal_refresh`.

**No deletions of whole files.** Calendar and GAL code in `crates/calendar/` and `crates/core/src/contacts/gal.rs` are unchanged - only the call site moves.

## Code-comment requirements

The strategic decisions from the revision history must appear as code comments where the relevant logic lives. All blocking on the relevant commit:

1. **`crates/service/src/calendar.rs` module-level doc-comment** must contain:
   - "Structurally symmetric with `crates/service/src/sync.rs::SyncRuntime` for the lifecycle surface (per-account map, panic supervisor, closed flag, start/cancel/shutdown). Diverges intentionally on: no marker-file lifecycle (calendar sync is idempotent against CalDAV CTags / Exchange ETags); no body / inline / search writer halves (calendar writes only to calendar tables); no invariant-pass entry. If you find yourself adding any of those, ask whether the divergence is still justified."
   - "Drains *before* SyncRuntime in the consolidated drain. Calendar runners can dispatch action plans (RSVP send → SMTP via the action worker) on the cancellation path; sync runners cannot. Draining calendar first ensures the action worker is still alive when calendar's cancellation path may need it. Forward-looking - no calendar code dispatches actions on cancel today, but the ordering is fixed so a future change doesn't re-introduce a drain race."

2. **`crates/imap/src/imap_initial.rs` and `crates/imap/src/imap_delta.rs` per-folder loop** must have an inline comment at the cancellation checkpoint:
   - `// Cancellation checkpoint - mirrors JMAP's per-mailbox checkpoint in crates/jmap/src/sync/mod.rs (Phase 3 task 6). The previous incomplete-port pattern was \`let _cancellation_token = cancellation_token;\` immediately after the entry-point check, dropping the token without threading it into the loop. A user pressing "cancel sync" mid-IMAP-fetch should not have to wait out the entire sync.`

3. **`crates/service/src/handlers/gal.rs::handle_gal_kick`** doc-comment must contain:
   - "GAL refresh is hourly + idempotent + bounded (60 s per-account timeout × supported-account count). No per-account runtime; no cancellation. If a benchmark surfaces this as a bottleneck, parallelize per-account; if a kill-mid-refresh problem surfaces, add a runtime. Today neither applies."

4. **The consolidated drain helper's doc-comment in `dispatch.rs`** must be updated to include the calendar step:
   - "Drain order: PushRuntime → CalendarRuntime → SyncRuntime → search-writer → sentinel. Calendar drains before Sync because calendar runners can dispatch action plans on cancellation; the action worker is still alive at this drain step. Sync drains after because no sync runner needs another subsystem alive at cancel time."

5. **`crates/service/src/calendar.rs::CalendarRuntime::start_account`** must mirror the Phase 4 review-pass `closed: AtomicBool` guard and the lock-released-during-network restructure. Inline comment:
   - "Same shutdown-guard pattern as PushRuntime - check `closed` before the slow path, re-acquire the lock for the insert and re-check both the guard and the duplicate-entry guard. Mirrors `crates/service/src/push.rs::PushRuntime::start_account`. Diverging is a refactor smell."

These comment texts are the contract; reviewers will reject commits that reword them in ways that lose the *why*.

## Test plan

### Unit tests

- `service-api`: serde round-trip for `CalendarRunId`, `CalendarStartAck`, `CalendarSyncResult`, `CalendarCompleted`. `RequestParams::CalendarStartAccountSync.timeout()` returns 5 s. Catalog cases for `CalendarCompleted` (class, method name, generation round-trip, `parse_service_message` round-trip). Catalog cases for `ClientNotification::CalendarKick` and `GalKick` (class `Drop`).
- `service::calendar`: `CalendarRuntime::cancel_account` returns false when no entry exists; `shutdown` is safe on empty runtime; `start_account` returns `Err` after `shutdown` (the unit-testable invariant from Phase 4 review-pass pattern).
- `imap` cancellation: drive `imap_initial_sync` (or its testable subroutine) with a pre-cancelled token; assert it returns the cancelled error path before any network round-trip. Drive with a token that flips mid-folder-iteration; assert the loop breaks at the next checkpoint, not at the end.

### Integration tests (in-process)

- `calendar_kick_starts_sync_in_service`: spin up a `CalendarRuntime` against a stub provider; send a `calendar.kick` notification; assert per-account starts fire. Same caveat as Phase 4: real provider integration needs a fake CalDAV fixture, deferred.
- `calendar_drains_before_sync_at_shutdown`: instrumented version - assert no calendar `start_account` is called after `SyncRuntime::shutdown` begins.
- `gal_kick_iterates_accounts_serially`: stub `refresh_gal_for_account`; send `gal.kick`; assert one in-flight at a time per the simpler-is-fine concurrency decision.

### Real-subprocess smoke tests

- `service_subprocess_calendar_kick_routes_to_handler`: spawn the Service with a seeded calendar account; observe a `calendar.kick` notification reaches the handler (via a debug log assertion). No actual CalDAV traffic - that needs the fixture.
- `service_subprocess_imap_cancel_interrupts_mid_fetch`: spawn with an IMAP-stub account; trigger an initial sync; cancel mid-fetch; assert `SyncCompleted { result: Cancelled }` arrives within a bounded window. Tests the cancellation-depth fix.

### Manual matrix updates

- The "what survives a Service crash" matrix in `problem-statement.md` § "Cross-store crash consistency" gets a new row: "Calendar sync state". Phase 5 outcome: idempotent re-fetch on next sync (the runtime has no marker; the calendar provider's CTag / ETag handling re-fetches what changed). No torn-write recovery needed.

## Open questions

1. **Does the Service drive calendar's hourly cadence itself, or does the UI keep the timer?** Plan says UI keeps the timer, sends `calendar.kick` notifications. The argument for moving the timer Service-side is "calendar continues to refresh while the UI is closed (tray-resident promotion in Phase 9)." For Phase 5 we keep it UI-side; Phase 9 revisits.
2. **`CalendarRuntime` per-account concurrency.** SyncRuntime is one-runner-per-account; calendar can plausibly be one-runner-per-account-per-calendar (an account may host multiple calendar collections). For Phase 5 we mirror SyncRuntime's per-account-only granularity; if a benchmark surfaces lock contention on a single account with many calendars, revisit.
3. **GAL handler concurrency.** Plan says serial-per-invocation. Two `gal.kick` notifications back-to-back: do we serialize via a Tokio Mutex (next blocked until current finishes), or does the second call short-circuit if a refresh ran recently? The latter avoids piling up work; the former is simpler to reason about. Recommended: short-circuit if last refresh was <30 s ago, otherwise serialize. Confirm during implementation.

## Verification (end-to-end)

- A change pushed to a JMAP mailbox triggers a sync inside the Service (Phase 4 verification carry-over).
- An IMAP initial sync started mid-fetch and then cancelled returns `SyncCompleted { result: Cancelled }` within seconds, not after the full folder list completes.
- A `Message::SyncTick` from the UI fires four IPC paths (sync, pending_ops, calendar, gal) and zero UI-side provider work.
- Stopping the Service mid-calendar-sync does not corrupt any DB state - drain order holds (calendar before sync before sentinel).
- A user opening a calendar attendee picker sees GAL entries refreshed within ~1 hour of any account being added (the Service's `gal.kick` handler picks it up on the next tick).
- The "Phase 5 status (as landed)" block in `problem-statement.md` documents what Phase 5 actually relocated vs what cascaded from Phase 3.

## Promotion criteria

- All Phase 5 tasks landed; IMAP `let _cancellation_token = cancellation_token;` markers are gone; UI-side `sync_calendars` and `refresh_gal_caches` are deleted; `CalendarRuntime` is in the consolidated drain.
- `Message::SyncTick` does no UI-side provider work.
- Phase 5 status block added to `problem-statement.md`.
- `phase-5-plan.md` is then retirement-ready: every deferral has an explicit roadmap entry (the Phase 8 test-cohort carry-forward already exists; Phase 5 just adds its own integration tests to that bucket), every code-comment requirement is present in the relevant file.
