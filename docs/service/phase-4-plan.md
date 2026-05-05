# The Service - Phase 4 Plan: JMAP push relocation

Companion to `phase-1-plan.md`, `phase-1.5-plan.md`, `phase-2-plan.md`, `phase-3-plan.md`. Implements Phase 4 of `implementation-roadmap.md`.

## Revision history

**2026-05-05 - initial draft.** Four decisions from the roadmap entry that this plan locks down explicitly (and that the code comments must mirror):

- **The temporary `oauth.refresh_request` IPC handshake is dropped.** The roadmap entry assumed OAuth couldn't refresh inside the Service until Phase 6. Re-checking the code: `crates/jmap/src/client.rs:159` already refreshes purely DB-mediated - reads encrypted refresh-token, hits the OAuth token endpoint, persists. The Service has DB access and the encryption key (Phase 1.5 onwards), so it can refresh in-process with zero IPC. What Phase 6 actually relocates is the *initial* OAuth flow (PKCE, browser launch, code exchange), which never appears on the push hot path - push only starts for already-authorized accounts whose refresh-tokens are already in the DB. **No new IPC method ships in Phase 4.** The roadmap's mention of `oauth.refresh_request` is a planning-doc error, corrected here.
- **`PushRuntime` is structurally symmetric with `SyncRuntime`.** Per-account map keyed by `account_id`; panic supervisor wrapping each per-account bridge task; `start_account` / `cancel_account` / `shutdown` API; account-add and account-delete go through the same handlers that already wrap sync.
- **Push drains *before* sync at shutdown.** A `StateChange` arriving as the Service is shutting down must not spawn a sync runner that the about-to-drain `SyncRuntime` cannot accept. Drain order: `PushRuntime::shutdown()` (cancel all bridge tasks, await their `JoinHandle`s) → `SyncRuntime::shutdown()` → search writer flush + drop → sentinel write. The Phase 3 drain order block in `crates/service/src/lifecycle.rs` extends with the new Push step at the front.
- **Crash continuity is explicitly Phase 8.** The `jmap_push_state` table (`crates/db/src/db/schema/10_sync.sql:58`) already stores `push_state`, `ws_url`, `last_connected_at`, and `consecutive_failures` per account, written by the existing `save_*` helpers in `crates/jmap/src/push.rs`. Phase 4 ships cold-restart-and-resync semantics: on Service boot, every JMAP account starts a fresh push subscription, no resume from `push_state`. Resuming at the JMAP cursor (`Email/changes` from the saved state, no full re-fetch) is a Phase 8 optimization. The columns survive the relocation; nothing reads `push_state` for resume in Phase 4.

## Context

Phase 3 moved JMAP sync (and the body / inline / search writers) into the Service, but left one loose end: the JMAP push WebSocket still lives UI-side. Today's transitional flow is `JMAP push event → UI receives → UI sends `sync.start_account` IPC → Service runs sync`. The round-trip is wasted work - the Service is the only correct owner of "long-lived background WebSocket whose only job is to kick a Service-internal subsystem."

Phase 4 collapses the round-trip. The WebSocket loop and its bridge task move into the Service; the bridge task calls `SyncRuntime::start_account(account_id)` directly. The UI keeps no push wiring. A `push.event { account_id }` notification (`MustDeliver`) goes UI-side for status-bar updates only; the UI does not act on it beyond rendering.

This phase is mechanical compared to Phase 3. The hard parts (cancellation, drain order, panic supervision, generation correlation) all landed in Phase 3 and are reused. The new surface area is small: one `PushRuntime` type, one notification variant, and the deletions of the UI-side push subscription + transitional `JmapPushKick` arm.

## Scope

### In scope

- Move `crates/core/src/jmap_push.rs::start_jmap_push_for_account` into `crates/service/src/push.rs::PushRuntime`. The bridge task's debounce window (500 ms) and StateChange-driven kick semantics carry over unchanged.
- New `PushRuntime` in `crates/service/src/push.rs`, structurally mirroring `crates/service/src/sync.rs::SyncRuntime`:
  - `HashMap<String, AccountEntry>` keyed by `account_id`, where `AccountEntry` carries the bridge task's `JoinHandle` + cancellation token + the `JmapPushManager`.
  - `start_account(account_id) -> Result<(), String>` - constructs the JMAP client, calls `jmap::push::start_push`, spawns the bridge task under a panic supervisor, inserts into the map.
  - `cancel_account(account_id) -> bool` - cancels the token, awaits the bridge supervisor, returns whether an entry existed.
  - `shutdown()` - cancels and awaits every bridge task. Called *before* `SyncRuntime::shutdown()`.
- New `push.event { account_id, generation }` notification (`MustDeliver`) emitted from the bridge task on each (debounced) StateChange burst. UI-visible for status-bar use; the bridge calls `SyncRuntime::start_account` *first*, then emits the notification.
- Boot integration: after `boot.ready` ack and the existing post-boot account enumeration, the Service's boot sequence calls `PushRuntime::start_account` for every JMAP account. New boot phase: `StartingPushSubscriptions`.
- Account lifecycle integration:
  - **Account add: piggyback on `SyncRuntime::start_account`, no new IPC.** Account creation is fully UI-side today (`crates/app/src/ui/add_account/identity.rs:34` writes the row via `db.with_write_conn(create_account_sync)`); `handle_add_account` is a Phase 6 carry-forward. The post-add flow already kicks an initial sync via `client.start_sync(account_id)`. The Phase 4 hook: have the Service-side `sync.start_account` handler also call `PushRuntime::start_account(account_id)` when `provider == "jmap"` and the entry isn't already in the map. Idempotent. This covers boot-time iteration (the explicit boot hook) and post-add (the piggyback) without introducing a temporary `account.added` IPC that Phase 6 would just delete.
  - `handle_delete_account` (Service-side, already exists post-Phase-2) calls `PushRuntime::cancel_account` *before* `SyncRuntime::cancel_account` and the DB delete - same drain-order rationale (a push event mid-delete must not race the sync cancel).
- UI-side teardown (single phase, single commit):
  - Delete `crates/app/src/handlers/provider.rs::start_jmap_push`, `JmapPushReceiver` type alias, `create_jmap_push_channel`, and `jmap_push_subscription`.
  - Delete `App::jmap_push_tx` / `App::jmap_push_receiver` fields.
  - Delete `Message::JmapPushKick` variant and its `update.rs:701` arm.
  - Delete `subscription.rs::jmap_push_subscription` recipe wiring.
  - Delete the `start_jmap_push()` call site in `handlers/core.rs:1014`.
  - Add a `Notification::PushEvent { account_id }` arm in `update.rs` that updates status-bar state (existing `last_push_at` field or equivalent; if no such field exists today, a small `HashMap<String, Instant>` on `ReadyApp` is fine).

### Out of scope

- **OAuth `oauth.refresh_request` IPC handshake.** Not needed; Service refreshes in-process. See revision-history note.
- **Crash continuity / push state resume.** Phase 8. The `jmap_push_state` table's `push_state` column is not consulted in Phase 4; a fresh subscription opens on every Service boot.
- **IMAP IDLE.** Pending; lands when IMAP IDLE itself lands in the codebase.
- **OS-level toast notifications.** Separate work, not part of the Service relocation.
- **Graph (Microsoft) subscription relocation.** Tracked for whichever phase ports Graph push; not Phase 4. Phase 4 is JMAP-only because that's the only push surface the codebase has today.

## Architecture

### `PushRuntime` shape

```text
service/
├── push.rs                      ← NEW
│   ├── pub struct PushRuntime { entries: Mutex<HashMap<String, AccountEntry>>, ... }
│   ├── struct AccountEntry { handle: JoinHandle<()>, cancel: CancellationToken, manager: JmapPushManager }
│   ├── pub async fn start_account(&self, account_id: &str) -> Result<(), String>
│   ├── pub async fn cancel_account(&self, account_id: &str) -> bool
│   └── pub async fn shutdown(&self)
└── handlers/sync.rs             ← already exists; SyncRuntime::start_account is what the bridge calls
```

Mirrors `crates/service/src/sync.rs::SyncRuntime` field-by-field where possible. Single source of truth for "this account has a live push subscription."

### Bridge task lifecycle

The bridge task (today: `tokio::spawn` in `core/src/jmap_push.rs:52`) moves into `PushRuntime::start_account`. Its body is unchanged in spirit:

1. Drain `rx.recv()` for `StateChange` events.
2. Coalesce within `PUSH_DEBOUNCE` (500 ms) - existing logic.
3. On a debounced kick: call `sync_runtime.start_account(account_id)` (in-Service, no IPC). Discard the `SyncStartAck` - the runner now owns the work; correlation by `run_id` is not needed at this layer because the bridge task isn't a waiter.
4. Emit `Notification::PushEvent { account_id, generation }` via the class-aware notification path (`MustDeliver`, queue cap 1024; `send_timeout(30s, ...)` for backpressure-safe delivery, mirroring Phase 3's `IndexCommitted` send).

The bridge task exits when:
- `cancel_token.is_cancelled()` (cooperative shutdown) - the manager is dropped, WebSocket closes cleanly.
- `rx.recv()` returns `None` (the JMAP push manager's WebSocket loop ended for its own reasons - server disconnect, max failures). The `PushRuntime` entry stays in the map but is dead; `shutdown()` will await its handle harmlessly. Re-arming dead subscriptions is a Phase 8 concern; Phase 4 punts (a Service restart re-arms everything).

### Panic supervisor

The bridge task is wrapped exactly the same way `SyncRuntime`'s runner is in Phase 3 (see `crates/service/src/sync.rs` § panic supervisor). On `JoinError::is_panic()`, log the panic message, drop the entry from the map, do *not* respawn within Phase 4. The next Service restart will respawn; that's the Phase 4 contract. Phase 8 may add in-Service respawn with backoff.

### Drain order (lifecycle)

`crates/service/src/lifecycle.rs`'s shutdown drain extends:

```text
1. PushRuntime::shutdown()        ← NEW (Phase 4)
2. SyncRuntime::shutdown()        ← Phase 3
3. SearchWriteHandle::flush_now() ← Phase 3
4. drop SearchWriteHandle         ← Phase 3
5. await search-writer JoinHandle ← Phase 3
6. unlink completed sync-markers  ← Phase 3
7. write clean_shutdown sentinel  ← Phase 3
```

**Why push drains first:** a `StateChange` arriving while the Service is mid-shutdown would otherwise call `SyncRuntime::start_account` after `SyncRuntime` has begun draining. The `SyncRuntime` would either reject it (best case: surfaces as a benign log line) or accept it and spawn a runner that races the drain (worst case: search writer flushes before the new runner finishes writing, sentinel writes claim a clean state that isn't). Push-first removes the race entirely - by the time `SyncRuntime` starts draining, no new kicks can arrive.

### OAuth refresh

**Stays in-Service. No IPC handshake added.**

The `JmapClient::from_account` path in `crates/jmap/src/client.rs:45-180` already does:
1. Reads encrypted `access_token`, `refresh_token`, `token_expires_at` from the DB.
2. If the access token is near expiry: takes the per-account refresh lock, calls `common::token::refresh_oauth_token` (HTTPS POST to the OAuth token endpoint), persists the new access token via `db::queries::persist_refreshed_token`.
3. Returns a client built against the fresh token.

This works inside the Service unchanged. The Service has DB access, the encryption key, and outbound HTTPS - everything the refresh flow needs. The refresh token itself is in the DB; rotating it (some providers issue a new refresh token on each refresh) is also DB-mediated and works Service-side.

What Phase 6 will relocate is the *interactive* OAuth flow - PKCE code generation, browser launch, redirect-URI capture, code-for-token exchange. None of that ever fires on the push hot path: by definition, push only runs for accounts that already have valid refresh tokens in the DB. If a refresh token is revoked or invalidated mid-Phase-4, the JMAP client returns an auth error; the bridge task logs it, the manager exits, the entry goes dead. The user re-authorizing the account is a UI-side flow (still UI-side until Phase 6) that triggers a new `account.add` and a new `PushRuntime::start_account` call.

**Code-comment requirement:** the doc-comment of `crates/service/src/push.rs` (module level) must state explicitly: "OAuth refresh runs in-Service via the existing DB-mediated `JmapClient::from_account` path. No IPC handshake to the UI is needed; the original Phase 4 roadmap entry's `oauth.refresh_request` IPC method is unnecessary and was removed in the Phase 4 plan revision."

### Crash continuity (deferred to Phase 8)

The `jmap_push_state` table columns - `push_state`, `ws_url`, `is_push_enabled`, `last_connected_at`, `consecutive_failures` - already exist and are written by `crates/jmap/src/push.rs`'s `save_*` helpers. Phase 4 keeps writing them (the existing `start_push` path does so unconditionally) but does *not* read `push_state` to resume.

On Service boot: every JMAP account opens a fresh WebSocket subscription. The first `StateChange` after the connection is established triggers a sync, which uses the JMAP `Email/changes` cursor (the `history_id` carried by `accounts`) to fetch only what changed - that's the existing sync-side delta semantics. So "fresh subscription" does not mean "full re-fetch"; it just means "no resume from the WebSocket-level push state."

Phase 8 adds: on boot, read `last_push_state` per account; pass it to `start_push` so the WebSocket connection can resume server-side change tracking from where the previous incarnation left off. This is a strict optimization (avoids a redundant sync kick on boot if nothing changed during the downtime); correctness is preserved without it.

**Code-comment requirement:** the doc-comment of `crates/service/src/push.rs` (module level) must state: "Phase 4 ships cold-restart-and-resync: every Service boot opens fresh WebSocket subscriptions and ignores the saved `jmap_push_state.push_state`. Phase 8 will add resume-from-saved-state as an optimization to avoid redundant boot-time sync kicks. The `save_*` helpers continue writing the column so the data is there when Phase 8 reads it."

### Notification class

`Notification::PushEvent { account_id, generation }` is `MustDeliver`. Rationale: a status-bar update is user-visible (a push notification icon, a "last sync at" timestamp); silently dropping it produces a UI that looks out-of-date. The notification queue is sized for `MustDeliver` traffic at Phase 3's cap (1024); push events are rare enough on aggregate that they cannot starve other classes.

**Catalog test extension:** the existing manually-enumerated catalog test (`crates/service-api/tests/notification_catalog.rs` or wherever it lives today) needs a `PushEvent` line. The Phase 3 plan called this out for `SyncCompleted` / `IndexCommitted`; same applies here.

## Detailed task list

In recommended commit order. Each item is one focused commit unless noted.

1. **`service-api`: `PushEvent` notification variant.** New `Notification::PushEvent { account_id: String, generation: u32 }`. `WithGeneration` impl. `Notification::class()` arm returns `MustDeliver`. `service_generation` / `set_service_generation` arms added exhaustively. Catalog test gains an explicit `PushEvent` case. Type-only commit.

2. **`crates/service/src/push.rs`: `PushRuntime`.** New file. Per-account map; bridge-task spawn under panic supervisor; `start_account` / `cancel_account` / `shutdown`. Uses `jmap::push::start_push` directly (the JMAP-side WebSocket loop is unchanged). Calls `SyncRuntime::start_account` on each debounced kick (in-Service, no IPC). Emits `Notification::PushEvent` after the kick. Module-level doc-comment includes the OAuth-refresh-is-in-Service note and the crash-continuity-deferred-to-Phase-8 note (see "Code-comment requirements" below).

3. **Boot integration.** New `BootPhase::StartingPushSubscriptions` enum variant. `crates/service/src/boot.rs` constructs the `PushRuntime` after `SyncRuntime` is up; iterates JMAP accounts; calls `PushRuntime::start_account` for each. Emit `BootProgress` per account-started (Coalesce class - bursty, status-bar-only). The Service's main loop holds the `Arc<PushRuntime>` for the lifetime of the incarnation.

4. **Lifecycle drain extension.** `crates/service/src/lifecycle.rs`'s shutdown drain inserts `PushRuntime::shutdown()` as step 1, before `SyncRuntime::shutdown()`. Doc-comment on `lifecycle.rs::shutdown` (or whichever function owns the drain) updated to describe the new step and *why* push drains first - reference the Architecture § "Drain order" rationale verbatim.

5. **Account-lifecycle hooks.** `crates/service/src/handlers/sync.rs::handle_start_account` (the Phase 3 `sync.start_account` handler) gains an opportunistic `PushRuntime::start_account` call: if `provider == "jmap"` and the account isn't already in the push map, start it. Idempotent; covers the post-add path without a new IPC method (account creation itself is Phase 6 carry-forward; see Scope § "Account add"). `handle_delete_account` (Service-side) calls `PushRuntime::cancel_account` *before* `SyncRuntime::cancel_account`, mirroring the drain-order rationale at a per-account granularity.

6. **UI teardown (single commit; deletions only).** Delete:
   - `crates/app/src/handlers/provider.rs`: `start_jmap_push`, `JmapPushReceiver`, `create_jmap_push_channel`, `jmap_push_subscription`.
   - `crates/app/src/app.rs`: `jmap_push_tx`, `jmap_push_receiver` fields and their construction.
   - `crates/app/src/message.rs`: `JmapPushKick` variant.
   - `crates/app/src/update.rs`: the `Message::JmapPushKick` arm at line 701.
   - `crates/app/src/subscription.rs`: the `jmap_push_subscription` line at 57 + the `use` at line 4.
   - `crates/app/src/handlers/core.rs:1014`: the `start_jmap_push()` call.
   - `crates/core/src/jmap_push.rs`: deleted in its entirety; the bridge logic now lives in `crates/service/src/push.rs`. Remove the `pub mod jmap_push` line in `crates/core/src/lib.rs`.

7. **UI: `PushEvent` notification handling.** New `Message::PushEvent(account_id)` arm in `update.rs`. Updates a `last_push_at: HashMap<String, Instant>` on `ReadyApp` (or extends an existing equivalent); status-bar view reads it. No sync action - the Service has already kicked sync by the time this arrives.

8. **Test cohort.** Phase 4 unit + integration + real-subprocess tests below. Lands incrementally with the commits above where natural; this task is the close-out commit.

9. **Doc updates.** Update `problem-statement.md`'s Phase 4 status block (new "Phase 4 status (as landed)" section, mirroring Phase 3's). Update `implementation-roadmap.md`'s Phase 4 entry to reflect the dropped OAuth handshake + the explicit Phase 8 punt for crash continuity. Bundle with the close-out commit per CLAUDE.md's "no markdown-only commits" rule.

## File-by-file changes

**New files:**
- `crates/service/src/push.rs` - `PushRuntime`, bridge tasks, panic supervisor.

**Modified files:**
- `crates/service-api/src/notification.rs` - add `PushEvent` variant; `WithGeneration` impl; class / generation arms.
- `crates/service-api/tests/<catalog>.rs` - explicit `PushEvent` case.
- `crates/service/src/boot.rs` - new `StartingPushSubscriptions` phase; PushRuntime construction + per-account start.
- `crates/service/src/boot_progress.rs` - extend phase enum.
- `crates/service/src/lifecycle.rs` - drain extension (step 1).
- `crates/service/src/lib.rs` (or `dispatch.rs` / `service.rs` - wherever the Service-incarnation Arc-bag lives) - hold `Arc<PushRuntime>`; pass to handlers that need it.
- `crates/service/src/handlers/<account-add>.rs`, `<account-delete>.rs` - hook calls.
- `crates/app/src/handlers/provider.rs` - **delete** push-side functions.
- `crates/app/src/app.rs` - **delete** `jmap_push_tx`, `jmap_push_receiver` fields.
- `crates/app/src/message.rs` - **delete** `JmapPushKick`.
- `crates/app/src/update.rs` - **delete** `JmapPushKick` arm; **add** `PushEvent` arm.
- `crates/app/src/subscription.rs` - **delete** push subscription wiring.
- `crates/app/src/handlers/core.rs` - **delete** `start_jmap_push()` call.
- `crates/core/src/lib.rs` - **delete** `pub mod jmap_push`.

**Deletions:**
- `crates/core/src/jmap_push.rs` - file deleted; logic moved to `crates/service/src/push.rs`. Phase 4 is the first phase that genuinely deletes a file (Phase 3's "no deletions" rule was per-phase, not project-wide).

## Code-comment requirements

The four explicit decisions from the revision history must appear as code comments where the relevant logic lives, so future readers cannot miss them. All four are blocking on the relevant commit:

1. **`crates/service/src/push.rs` module-level doc-comment** must contain:
   - "OAuth refresh runs in-Service via the existing DB-mediated `JmapClient::from_account` path (`crates/jmap/src/client.rs:45-180`). No IPC handshake to the UI is needed; the Phase 4 roadmap entry's `oauth.refresh_request` was removed in the Phase 4 plan revision after re-checking that refresh is purely DB+HTTPS, both of which the Service has."
   - "Phase 4 ships cold-restart-and-resync: every Service boot opens fresh WebSocket subscriptions and ignores `jmap_push_state.push_state`. Phase 8 will add resume-from-saved-state. The `save_*` helpers in `crates/jmap/src/push.rs` continue writing the column so the data is there when Phase 8 reads it."

2. **`crates/service/src/push.rs::PushRuntime` type doc-comment** must contain:
   - "Structurally symmetric with `crates/service/src/sync.rs::SyncRuntime`. Per-account map, panic supervisor, lifecycle hooks for account-add / account-delete / Service shutdown. Diverging from `SyncRuntime`'s shape is a refactor smell - if you're tempted, fix the shared abstraction instead."

3. **`crates/service/src/lifecycle.rs::shutdown`'s doc-comment** must contain:
   - "Drain order: PushRuntime *before* SyncRuntime. A `StateChange` arriving mid-shutdown must not call `SyncRuntime::start_account` after `SyncRuntime` has begun draining - that race lets the search writer flush before the new runner finishes writing, and the sentinel claims a clean state that isn't. Push-first removes the race entirely."

4. **`crates/service/src/handlers/<account-delete>.rs`'s `handle_delete_account`** must have an inline comment at the `cancel_account` calls:
   - `// Push *before* sync, same drain-order rationale as `lifecycle.rs::shutdown`: a push event mid-delete must not race the sync cancel.`

These comment texts are the contract; reviewers will reject commits that reword them in ways that lose the *why*.

## Test plan

### Unit tests

- `service-api`: serde round-trip for `Notification::PushEvent`. Catalog test gains an explicit `PushEvent` case (auto-coverage was Phase 3's mistaken assumption; this plan locks it in).
- `service::push`: `PushRuntime::start_account` inserts an entry, `cancel_account` removes it, `shutdown` cancels and awaits all entries (assert no leaked tasks via `JoinSet::is_empty()` post-shutdown). Panic supervisor: a bridge task that panics is observed via `JoinError::is_panic()`, the entry is removed from the map, and `shutdown` succeeds. Bridge task on `rx.recv() returns None`: entry stays in map (dead), `shutdown` awaits the handle without hanging.
- `service::push`: bridge-task debounce window. Inject 5 `StateChange` events within 500 ms; assert `SyncRuntime::start_account` is called exactly once and `Notification::PushEvent` is emitted exactly once.

### Integration tests (in-process)

- `push_event_kicks_sync_in_service`: spin up a `PushRuntime` against a fake JMAP push manager that emits one `StateChange`; assert the bridge calls `SyncRuntime::start_account(account_id)` and a `Notification::PushEvent` arrives at the notification queue. No UI involvement.
- `push_drains_before_sync_at_shutdown`: with a `StateChange` racing in 50 ms before shutdown begins, assert no sync runner is spawned after `SyncRuntime::shutdown()` starts. (Achieve via instrumented `SyncRuntime::start_account` that records call timestamps; the assertion is "no call landed after `SyncRuntime::shutdown` was entered.")
- `account_delete_cancels_push_before_sync`: simulated delete-account flow; assert `PushRuntime::cancel_account` is awaited before `SyncRuntime::cancel_account` is called.

### Real-subprocess smoke tests

- `service_subprocess_starts_push_for_jmap_accounts`: spawn the Service with a seeded JMAP account; observe `BootProgress::StartingPushSubscriptions` arrives; observe a `PushEvent` notification on a subsequent state change (driven by a fake JMAP server fixture).
- `service_subprocess_oauth_refresh_during_push`: a JMAP account whose access token expires mid-subscription; observe the in-Service refresh fires (logged), the WebSocket continues, and no IPC method named `oauth.refresh_request` ever appears on the wire (regression test for the dropped handshake).

### Manual matrix updates

- The "what survives a Service crash" matrix in `problem-statement.md` § "Cross-store crash consistency" gets a new row: "JMAP push subscription". Phase 4 outcome: lost; re-established at next boot. Phase 8 outcome: resumed from saved state. (No table-shape change beyond the new row.)

## Open questions

1. **Does the Service keep its own JMAP account list, or does it re-query the DB?** Phase 3's `SyncRuntime` re-queries the DB; same shape applies here (one `SELECT id FROM accounts WHERE provider = 'jmap'` at boot, plus the post-add piggyback hook for incremental). Confirm parity with `SyncRuntime`'s approach.
2. **`PushEvent` rate-limiting.** A pathological JMAP server could emit `StateChange` faster than the 500 ms debounce coalesces (e.g., bursts spaced exactly 501 ms apart). Phase 3's notification queue cap is 1024 `MustDeliver` slots; sustained > 1 push event/sec per account across 5 accounts × 1 hour = 18000 events, well past the cap. The notification-class catalog says `MustDeliver` does *not* drop on overflow; it backpressures the sender. The bridge task is async, so backpressure parks the bridge - which delays the next sync kick. **Decision for Phase 4:** accept the parking. The pathological server is a hypothetical; if it surfaces, Phase 8's notification-class refinement can downgrade `PushEvent` to `Coalesce`. Document this in the `PushRuntime` module comment.

## Verification (end-to-end)

- A change pushed to a JMAP mailbox triggers a sync inside the Service. The UI is not on the call path. Verify by tcpdumping the IPC channel: no `sync.start_account` IPC method call should appear in response to push events.
- Status bar shows "new mail arrived" within the debounce window of the push.
- Token expiry during a Service-side push subscription does not break the connection - the in-Service refresh fires; the WebSocket stays alive.
- Stopping the Service with an in-flight push event does not corrupt sync state - drain order holds.
- Restarting the Service re-establishes push subscriptions for all JMAP accounts; no resume from `push_state` (fresh subscription); a single sync kick happens shortly after if anything changed during downtime.

## Promotion criteria

- All Phase 4 tasks landed; `crates/core/src/jmap_push.rs` is gone; `JmapPushKick` is gone; the UI has zero push wiring.
- Phase 4 status block added to `problem-statement.md`.
- `phase-4-plan.md` is then retirement-ready: every deferral has an explicit roadmap entry (currently: crash continuity → Phase 8 § "JMAP push state resume", which the close-out commit adds), every code-comment requirement is present in the relevant file, and no test references this plan as a TODO source.
