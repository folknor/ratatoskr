# The Service - Phase 4 Plan: JMAP push relocation

Companion to `phase-1-plan.md`, `phase-1.5-plan.md`, `phase-2-plan.md`, `phase-3-plan.md`. Implements Phase 4 of `implementation-roadmap.md`.

## Revision history

**2026-05-05 - first review revision pass.** The initial draft was reviewed by `review arch,bugs --oneshot` (claude + codex sessions for each). The strategic decisions held (drop OAuth IPC, mirror `SyncRuntime`, push-drains-first, defer crash continuity to Phase 8 - all four reviewers agreed). The execution detail was significantly out of sync with current code state. Changes:

- **OAuth-refresh-just-works claim corrected.** Three of four reviewers verified that `JmapClient::from_account` (`crates/jmap/src/client.rs:229`) does *not* call refresh; refresh lives in `ensure_valid_token` (`client.rs:61`) and the current push path never invokes it before `start_push`. Worse, `start_push` captures a static auth header at construction time (`crates/jmap/src/push.rs:144`) and reuses it across every reconnect, so a refreshed bearer token never propagates into the WebSocket. **Resolution:** Phase 4 explicitly calls `ensure_valid_token` before `start_push`, *and* threads an auth resolver (`Fn() -> impl Future<Output = String>`) into `push_connection_loop` so each reconnect re-resolves the bearer token. The "no IPC handshake needed" decision still holds (refresh is DB+HTTPS, both Service-internal); the implementation cost is non-trivial and gets its own task. See § "OAuth refresh" below.
- **`push_state` resume contradiction.** `start_push` *unconditionally* loads saved state (`push.rs:147`) and sends it in `WebSocketPushEnable` (`push.rs:337`), so the original plan's "Phase 4 ignores `push_state`" was both false and would have required adding a fresh-start API. **Resolution:** flip the framing. Phase 4 inherits today's behavior - `start_push` resumes from saved state where present. This is correct for clean shutdowns and harmless for crashes (Phase 3's invariant pass clears `history_id` for dirty accounts; the resumed push connection just delivers a `StateChange` that triggers a delta sync the cleared cursor turns into a re-fetch). The Phase 8 carry-forward is now "explicit fresh-start knob + crash-aware reset," not "implement resume."
- **Drain order misattributed; current order is racy.** All four reviewers caught that `lifecycle.rs::drain` (`crates/service/src/lifecycle.rs:72`) writes the sentinel *before* `dispatch.rs:321-326` shuts down `SyncRuntime`. The plan's `lifecycle.rs`-extends-with-step-1 framing was fictional. Worse, today's order means a graceful Phase 3 shutdown can already write a `clean_shutdown` sentinel before in-flight sync writes complete - a pre-existing Phase 3 bug. **Resolution:** Phase 4 ships an explicit drain-consolidation task that moves the sentinel write *after* `SyncRuntime::shutdown` in `dispatch.rs`, *then* prepends `PushRuntime::shutdown()`. Code-comment contracts retarget `dispatch.rs::run_dispatch_loop` (or extracted `shutdown_drain` helper).
- **`handle_delete_account` is UI-side, not Service-side.** Same kind of error as `handle_add_account` in the initial draft. The actual delete flow lives at `crates/app/src/handlers/core.rs:631`; it calls `client.cancel_and_await(...)` then writes the DB delete UI-side. **Resolution:** mirror the start-side piggyback. The Service-side `sync.cancel_account` handler also tears down push for that account when an entry exists. Symmetric with the start-side hook on `sync.start_account`. Account-delete itself stays UI-side; Phase 6 relocates it.
- **Cancellation needs explicit `stop_push().await`, not Drop.** Two reviewers caught that dropping `JmapPushManager` doesn't close the WebSocket loop - `push_connection_loop` exits only when `shutdown_rx`'s watched value becomes `true`, which `stop_push()` sets explicitly (`push.rs:199`). **Resolution:** `cancel_account` order is: cancel cooperative token → await bridge handle → `manager.stop_push().await`. `AccountEntry` keeps the manager *inside the bridge task body* (matches today's pattern at `core/src/jmap_push.rs:54`), not on the entry struct - simpler ownership, no drop-order subtlety.
- **`PushEvent` is `Coalesce { key: account_id }`, not `MustDeliver`.** All four reviewers flagged. Status-bar semantics are latest-wins per account; nobody waits on a `PushEvent` future, so dropping on overflow is benign. `MustDeliver` would backpressure the bridge task, parking it on send and delaying the *next* state change's sync kick. The original plan's open-question-2 already foreshadowed this fix; this revision lands it now rather than carrying a known-shaky decision.
- **`sync.start_account` piggyback violates the 5s no-network IPC contract.** `RequestParams::SyncStartAccount.timeout()` is 5 s; JMAP client construction does TLS+HTTPS (and now refresh check). **Resolution:** the piggyback spawns a detached task (`tokio::spawn`); `handle_start_account` returns the `SyncStartAck` immediately. Push setup failures are logged, not surfaced through the IPC ack. Symmetric for the cancel side (`tokio::spawn` the `manager.stop_push()` await).
- **Boot timing inconsistency resolved.** Scope said post-`boot.ready`; task list put startup inside `run_boot_sequence_inner`. The latter would make readiness depend on WebSocket+TLS+OAuth-refresh work, which is unacceptable. **Resolution:** push startup is a *post-ready* runtime task kicked from `dispatch.rs` after the boot handshake completes. No new `BootPhase` variant. The Service holds `Arc<PushRuntime>` for the incarnation; the post-ready task iterates JMAP accounts and `tokio::spawn`s `PushRuntime::start_account` calls (log-and-continue per account; one bad account doesn't block the others).
- **Provider gating moves into `PushRuntime::start_account`.** The Service-side handler (`handle_start_account`, `handle_cancel_account`) doesn't know provider type without a DB read. Cleanest fix: `PushRuntime::start_account` reads the account row internally and no-ops for non-JMAP. Handler stays clean; runtime is self-policing.
- **Re-auth path doesn't re-arm dead push entries.** Re-auth updates the existing account row in place (UI-side `AddAccountWizard::new_reauth` flow); doesn't go through `account.add`. So a token-revocation kills push until Service restart. **Resolution:** explicit Phase 8 carry-forward (a token-refresh-success event re-arms the entry). Phase 4 known-gap, documented in the manual matrix and in the `PushRuntime` module comment.
- **`AccountEntry` ownership simplified.** `AccountEntry { handle, cancel }` only - no `manager` field. The bridge task owns the manager (matches today). Cancel ordering: cancel token → await handle → done (the awaited bridge runs `manager.stop_push().await` on its own exit path).
- **Cosmetic / file-path corrections.** `BootPhase` lives in `crates/service-api/src/boot.rs:113-143`, not `service/src/boot_progress.rs`. Catalog test cases live inline in `crates/service-api/src/notification.rs:469-585`, not in `tests/`. Field name is `service_generation: u32`, not `generation`. `tokio::time::timeout(D, ch.send(n))` is the Phase 3 idiom, not `send_timeout`. The UI's reader-task notification dispatch in `crates/app/src/service_client.rs` also needs the new arm. Status-bar state belongs on `StatusBar`, not on `ReadyApp`. JMAP client `from_account` lives at `client.rs:229-280`, not `45-180`. All corrected throughout.

**2026-05-05 - initial draft.** Four decisions from the roadmap entry that this plan locks down explicitly (and that the code comments must mirror):

- **The temporary `oauth.refresh_request` IPC handshake is dropped.** The roadmap entry assumed OAuth couldn't refresh inside the Service until Phase 6. Re-checking the code: `crates/jmap/src/client.rs:159` already refreshes purely DB-mediated - reads encrypted refresh-token, hits the OAuth token endpoint, persists. The Service has DB access and the encryption key (Phase 1.5 onwards), so it can refresh in-process with zero IPC. What Phase 6 actually relocates is the *initial* OAuth flow (PKCE, browser launch, code exchange), which never appears on the push hot path - push only starts for already-authorized accounts whose refresh-tokens are already in the DB. **No new IPC method ships in Phase 4.** The roadmap's mention of `oauth.refresh_request` is a planning-doc error, corrected here.
- **`PushRuntime` is structurally symmetric with `SyncRuntime`.** Per-account map keyed by `account_id`; panic supervisor wrapping each per-account bridge task; `start_account` / `cancel_account` / `shutdown` API; account-add and account-delete go through the same handlers that already wrap sync.
- **Push drains *before* sync at shutdown.** A `StateChange` arriving as the Service is shutting down must not spawn a sync runner that the about-to-drain `SyncRuntime` cannot accept. Drain order: `PushRuntime::shutdown()` (cancel all bridge tasks, await their `JoinHandle`s) → `SyncRuntime::shutdown()` → search writer flush + drop → sentinel write. The Phase 3 drain order block in `crates/service/src/lifecycle.rs` extends with the new Push step at the front.
- **Crash continuity is explicitly Phase 8.** The `jmap_push_state` table (`crates/db/src/db/schema/10_sync.sql:58`) already stores `push_state`, `ws_url`, `last_connected_at`, and `consecutive_failures` per account, written by the existing `save_*` helpers in `crates/jmap/src/push.rs`. Phase 4 ships cold-restart-and-resync semantics: on Service boot, every JMAP account starts a fresh push subscription, no resume from `push_state`. Resuming at the JMAP cursor (`Email/changes` from the saved state, no full re-fetch) is a Phase 8 optimization. The columns survive the relocation; nothing reads `push_state` for resume in Phase 4.

## Context

Phase 3 moved JMAP sync (and the body / inline / search writers) into the Service, but left one loose end: the JMAP push WebSocket still lives UI-side. Today's transitional flow is `JMAP push event → UI receives → UI sends `sync.start_account` IPC → Service runs sync`. The round-trip is wasted work - the Service is the only correct owner of "long-lived background WebSocket whose only job is to kick a Service-internal subsystem."

Phase 4 collapses the round-trip. The WebSocket loop and its bridge task move into the Service; the bridge task calls `SyncRuntime::start_account(account_id)` directly. The UI keeps no push wiring. A `push.event { account_id, service_generation }` notification (`Coalesce { key: account_id }`) goes UI-side for status-bar updates only; the UI does not act on it beyond rendering. Latest-wins semantics per account; drop-on-overflow is benign because nobody waits on a `PushEvent` future.

This phase is mechanical compared to Phase 3. The hard parts (cancellation, drain order, panic supervision, generation correlation) all landed in Phase 3 and are reused. The new surface area is small: one `PushRuntime` type, one notification variant, and the deletions of the UI-side push subscription + transitional `JmapPushKick` arm.

## Scope

### In scope

- Move `crates/core/src/jmap_push.rs::start_jmap_push_for_account` into `crates/service/src/push.rs::PushRuntime`. The bridge task's debounce window (500 ms) and StateChange-driven kick semantics carry over unchanged.
- New `PushRuntime` in `crates/service/src/push.rs`, structurally mirroring `crates/service/src/sync.rs::SyncRuntime`:
  - `HashMap<String, AccountEntry>` keyed by `account_id`, where `AccountEntry { handle: JoinHandle<()>, cancel: CancellationToken }`. The `JmapPushManager` lives *inside* the bridge task body (matches today's pattern at `core/src/jmap_push.rs:54`); cancel ordering is "cancel token → await handle" (the bridge runs `manager.stop_push().await` on its exit path before returning).
  - `start_account(account_id) -> Result<(), String>` - reads the account row, no-ops for non-JMAP, calls `ensure_valid_token` to refresh if needed, calls `jmap::push::start_push` with an auth resolver, spawns the bridge task under a panic supervisor, inserts into the map.
  - `cancel_account(account_id) -> bool` - cancels the token, awaits the bridge supervisor (which awaits `manager.stop_push()`), returns whether an entry existed.
  - `shutdown()` - cancels and awaits every bridge task. Called *before* `SyncRuntime::shutdown()` in the consolidated drain.
- New `push.event { account_id, service_generation }` notification (`Coalesce { key: account_id }`) emitted from the bridge task on each (debounced) StateChange burst. UI-visible for status-bar use; latest-wins per account; drop-on-overflow is benign. The bridge calls `SyncRuntime::start_account` *first*, then emits the notification, so notification queue pressure cannot delay sync kicks.
- Boot integration: push startup is a *post-ready runtime task* kicked from `dispatch.rs` after the `boot.ready` handshake completes. No new `BootPhase` variant - readiness must not depend on WebSocket+TLS+OAuth-refresh work. The post-ready task iterates JMAP accounts and `tokio::spawn`s `PushRuntime::start_account` calls; per-account failure is log-and-continue.
- Account lifecycle integration:
  - **Account add: piggyback on `SyncRuntime::start_account`, no new IPC, fire-and-forget.** Account creation is fully UI-side today (`crates/app/src/ui/add_account/identity.rs:34` writes the row via `db.with_write_conn(create_account_sync)`); `handle_add_account` is a Phase 6 carry-forward. The post-add flow already kicks an initial sync via `client.start_sync(account_id)`. The Phase 4 hook: the Service-side `sync.start_account` handler `tokio::spawn`s a `PushRuntime::start_account(account_id)` call (detached; result is logged, not surfaced through the IPC ack). Provider-gating happens inside `PushRuntime::start_account` (no-ops for non-JMAP). Fire-and-forget is required because the IPC has a 5 s timeout and JMAP client construction can do TLS+HTTPS+refresh.
  - **Account cancel: piggyback on `SyncRuntime::cancel_account`, also fire-and-forget.** `handle_delete_account` is UI-side today (`crates/app/src/handlers/core.rs:631`); the existing flow calls `client.cancel_and_await` for sync, then deletes the row UI-side. Phase 4's hook lives in the Service-side `sync.cancel_account` handler: `tokio::spawn`s a `PushRuntime::cancel_account(account_id)` call alongside the sync cancel. Symmetric with the start-side piggyback. Account-delete itself stays UI-side; Phase 6 relocates it.
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
│   ├── struct AccountEntry { handle: JoinHandle<()>, cancel: CancellationToken }
│   │       (manager lives inside the bridge task; cancel: token first, await handle, done)
│   ├── pub async fn start_account(&self, account_id: &str) -> Result<(), String>
│   ├── pub async fn cancel_account(&self, account_id: &str) -> bool
│   └── pub async fn shutdown(&self)
└── handlers/sync.rs             ← already exists; SyncRuntime::start_account is what the bridge calls
```

Mirrors `crates/service/src/sync.rs::SyncRuntime` field-by-field where possible. Single source of truth for "this account has a live push subscription."

### Bridge task lifecycle

The bridge task (today: `tokio::spawn` in `core/src/jmap_push.rs:52`) moves into `PushRuntime::start_account`. The bridge owns the `JmapPushManager` for its lifetime - the manager moves into the task body and stays there until the task exits, mirroring today's pattern.

The bridge body becomes a `tokio::select!` between cancellation and the StateChange channel:

1. `cancel_token.cancelled()` arm: break out of the loop. After the loop, the bridge awaits `manager.stop_push().await` to set the watch-shutdown signal that lets `push_connection_loop` exit cleanly (`crates/jmap/src/push.rs:199, 439`). Then drops the manager. Then returns. `JmapPushManager::stop_push` is the only correct way to stop the connection loop; dropping the manager alone is insufficient because `push_connection_loop` holds a `watch::Receiver` (drop of the `watch::Sender` does not flip the watched value to `true`).
2. `rx.recv()` arm: process StateChange.
   a. Coalesce within `PUSH_DEBOUNCE` (500 ms) - existing logic.
   b. On a debounced kick: call `sync_runtime.start_account(account_id)` (in-Service, no IPC). Discard the `SyncStartAck` - the runner now owns the work; correlation by `run_id` is not needed at this layer because the bridge isn't a waiter.
   c. Emit `Notification::PushEvent { account_id, service_generation }` via the class-aware notification path (`Coalesce { key: account_id }`, drop-on-overflow benign). Send pattern: `tokio::time::timeout(D, channel.send(notification))` matching Phase 3's `INDEX_COMMITTED_SEND_TIMEOUT` idiom in `crates/service/src/search_writer.rs:285-298`.

The bridge task exits when:
- `cancel_token` is cancelled (cooperative shutdown) - the manager is stopped, WebSocket closes cleanly via the `stop_push().await` step above.
- `rx.recv()` returns `None` (the JMAP push manager's WebSocket loop ended for its own reasons - server disconnect, max failures). The bridge still calls `manager.stop_push().await` on its way out (idempotent if already stopped) and the entry stays in the map but is dead. `shutdown()` will await its handle harmlessly. Re-arming dead subscriptions is a Phase 8 concern; Phase 4 punts.
- A token-revocation auth error during the initial connect kills the manager early. Same exit path.

**Re-auth dead-entry gap (Phase 4 known-gap, Phase 8 fix):** UI-side re-auth (`AddAccountWizard::new_reauth`) updates the existing account row in place and does *not* go through `account.add` or any IPC that triggers `PushRuntime::start_account`. So a dead entry from token revocation lives until Service restart even after the user re-authorizes. Phase 8 wires push re-arm to a token-refresh-success event. Phase 4 documents the gap in the manual matrix and the `PushRuntime` module comment.

### Panic supervisor

The bridge task is wrapped exactly the same way `SyncRuntime`'s runner is in Phase 3 (see `crates/service/src/sync.rs` § panic supervisor). On `JoinError::is_panic()`, log the panic message, drop the entry from the map, do *not* respawn within Phase 4. The next Service restart will respawn; that's the Phase 4 contract. Phase 8 may add in-Service respawn with backoff.

### Drain order (lifecycle)

**Phase 3's drain is split across two files in a way that's already racy.** `crates/service/src/lifecycle.rs::run_drain` (around lines 72-109) writes the `clean_shutdown` sentinel; `crates/service/src/dispatch.rs:321-326` shuts down `SyncRuntime` *afterwards*. So today's graceful shutdown can write the sentinel before in-flight sync writes complete - the search writer flush and marker unlink also run per-run inside `crates/service/src/sync.rs::run_sync` (lines 401, 438-443), not at drain time, but the sentinel writes nonetheless claim a clean state regardless of whether `SyncRuntime::shutdown` has finished. This is a pre-existing Phase 3 bug; left alone, Phase 4 layering push on top would inherit it.

**Phase 4 ships an explicit drain consolidation.** A new helper `service::lifecycle::shutdown_drain(...)` (or extracted `dispatch::run_shutdown_drain`) collects the drain steps from both files and orders them correctly:

```text
1. PushRuntime::shutdown()                    ← NEW (Phase 4)
2. SyncRuntime::shutdown()                    ← Phase 3 (relocated from dispatch.rs)
3. SearchWriteHandle::flush_now()             ← already per-run in run_sync; verify nothing is in flight
4. drop SearchWriteHandle                     ← Phase 3 sequencing
5. await search-writer JoinHandle             ← Phase 3 sequencing
6. unlink completed sync-markers (best-effort) ← Phase 3 sequencing
7. write clean_shutdown sentinel              ← MOVED here from lifecycle::run_drain
```

The Phase 3 sentinel-write call gets removed from `lifecycle::run_drain` and lives only in the new helper. The dispatch-side `SyncRuntime::shutdown().await` call gets removed from `dispatch.rs:321-326` and lives only in the new helper. The helper is called from the same shutdown handler that today calls `lifecycle::run_drain`.

**Why push drains first:** a `StateChange` arriving while the Service is mid-shutdown would otherwise call `SyncRuntime::start_account` after `SyncRuntime` has begun draining. The `SyncRuntime` would either reject it (best case: surfaces as a benign log line) or accept it and spawn a runner that races the drain (worst case: search writer flushes before the new runner finishes writing, sentinel writes claim a clean state that isn't). Push-first removes the race entirely - by the time `SyncRuntime` starts draining, no new kicks can arrive.

**Sidebar fix attribution:** the Phase 3 status block in `problem-statement.md` § "Phase 3 status (as landed)" gains a closing-out line noting that the drain consolidation was a Phase 3 carry-forward fixed in Phase 4. The `phase-3-plan.md` retirement criterion is unaffected; this fix lands in the same Phase 4 commit that adds the consolidated helper.

### OAuth refresh

**Stays in-Service. No IPC handshake added. But it requires real plumbing - it's not free.**

The strategic decision (no IPC round-trip to the UI) holds: refresh is purely DB-read + HTTPS-POST, and the Service has both. But the initial draft's claim that `JmapClient::from_account` already refreshes was wrong. The actual code state:

- `JmapClient::from_account` (`crates/jmap/src/client.rs:229-280`) reads `access_token` from the DB and constructs the client; it does *not* call `ensure_valid_token`.
- `JmapClient::ensure_valid_token` (`crates/jmap/src/client.rs:61-184`) is what does refresh - takes the per-account refresh lock, hits the OAuth token endpoint via `common::token::refresh_oauth_token`, persists the new access token via `db::queries::persist_refreshed_token`.
- `jmap::push::start_push` (`crates/jmap/src/push.rs:122-200`) captures the bearer token at construction (line 144) and reuses that captured string on every reconnect (lines 246-256, 311-321). The push connection-loop never calls `ensure_valid_token`.

So today's mid-subscription token-expiry behavior is: the first reconnect after expiry fails auth, retries 5 times (`MAX_CONSECUTIVE_FAILURES`), then enters the dead-entry state. This is a pre-existing bug and Phase 4 inherits it unless we fix it.

**Phase 4's two-step fix:**

1. **Refresh-before-`start_push` at startup.** `PushRuntime::start_account` calls `client.ensure_valid_token().await` after constructing the client and before calling `jmap::push::start_push`. Closes the "stale-token-at-bridge-spawn" hole.
2. **Auth resolver threaded into `push_connection_loop`.** `jmap::push::start_push` gains a new parameter: `auth_resolver: Arc<dyn Fn() -> BoxFuture<'static, Result<String, String>> + Send + Sync>`. The connection-loop calls `auth_resolver().await` *before each connect attempt* (initial + every reconnect) instead of using a captured `auth_header`. The resolver implementation re-resolves via `client.ensure_valid_token()` and returns the bearer header string. Closes the mid-subscription expiry hole.

The resolver pattern is chosen over passing `Arc<JmapClient>` so the JMAP crate stays free of `JmapClient` knowledge in its push module - the resolver is opaque. The resolver lives in `crates/service/src/push.rs` and closes over the `JmapClient` it constructs at `start_account` time.

**Refresh-token revocation** still lands the entry dead: when revocation propagates, `ensure_valid_token` returns an error, the resolver propagates it, the connection-loop gives up after `MAX_CONSECUTIVE_FAILURES`. Re-arming on user re-auth is the Phase 8 carry-forward (see § "Bridge task lifecycle" / Re-auth gap).

**What Phase 6 will still relocate** is the *interactive* OAuth flow - PKCE code generation, browser launch, redirect-URI capture, code-for-token exchange. None of that fires on the push hot path: push only runs for accounts that already have valid refresh tokens in the DB. The Phase 4 plumbing covers both ordinary expiry and refresh-token rotation (some providers issue a new refresh token on each refresh; `persist_refreshed_token` already handles that DB-side).

**Code-comment requirement:** the doc-comment of `crates/service/src/push.rs` (module level) must state explicitly: "OAuth refresh runs in-Service. Phase 4 calls `JmapClient::ensure_valid_token` before `jmap::push::start_push`, and threads an auth resolver into `push_connection_loop` so reconnects re-resolve the bearer token. No IPC handshake to the UI is needed; the Phase 4 roadmap entry's `oauth.refresh_request` IPC was removed because refresh is purely DB+HTTPS, both Service-internal."

### Crash continuity (Phase 4 inherits today's behavior; Phase 8 hardens)

The `jmap_push_state` table columns - `push_state`, `ws_url`, `is_push_enabled`, `last_connected_at`, `consecutive_failures` - already exist and are read+written by `crates/jmap/src/push.rs`. Today's `start_push` *unconditionally* loads saved state at construction (`push.rs:147`) and sends it in `WebSocketPushEnable` (`push.rs:337`) - the WebSocket *already* resumes server-side change tracking from the previous incarnation's saved state.

**Phase 4 inherits this behavior unchanged.** The initial draft's framing of "Phase 4 ignores `push_state`" was both factually wrong (the code does read it) and would have required adding a fresh-start API. Re-framing:

- **Clean shutdown then boot:** the saved `push_state` is recent and trustworthy. The WebSocket resumes from it; the server delivers any state changes that occurred during downtime; the bridge task kicks a delta sync. This is the optimal path and Phase 4 ships it for free.
- **Crash then boot:** the saved `push_state` may be stale or torn (writes happen in `save_last_push_state` after each StateChange; a crash mid-write is possible). Phase 3's invariant pass already handles correctness for crashed accounts: `clear_account_history_id` for any account whose marker survived as non-`Completed`. So even if the resumed push delivers a `StateChange` for a stale state, the cleared `history_id` turns the resulting sync into a re-fetch. Correctness is preserved.

**Phase 8 carry-forward** is therefore not "implement resume" - it's "harden resume":
- Detect crashed accounts at push-startup time and force a fresh-start (clear `push_state` before calling `start_push` for accounts whose Phase 3 sync-marker indicated a non-clean exit).
- Add an explicit fresh-start knob on `start_push` (cleaner than a pre-call `save_push_disabled` workaround).
- Add a manual matrix entry for "Service crashed mid-push-write" with the recovery story.

**Code-comment requirement:** the doc-comment of `crates/service/src/push.rs` (module level) must state: "Phase 4 inherits today's resume-from-saved-state behavior in `jmap::push::start_push` (`crates/jmap/src/push.rs:147, 337`). On clean shutdown this is the optimal path. On crash, the resumed connection may deliver a stale `StateChange`; Phase 3's invariant pass already clears `history_id` for crashed accounts, so the resulting delta sync re-fetches the cached window. Phase 8 will harden this with explicit crash-aware fresh-start logic."

### Notification class

`Notification::PushEvent { account_id: String, service_generation: u32 }` is `Coalesce { key: account_id }`. Rationale: status-bar semantics are latest-wins per account ("did *some* push event arrive for this account recently?"); nobody waits on a `PushEvent` future, so drop-on-overflow is benign. `MustDeliver` would backpressure the bridge task on `send`, parking it and delaying the *next* StateChange's sync kick - which is exactly the wrong tradeoff (sync correctness is `MustDeliver`'s job, not status-bar updates).

**Catalog test extension:** the manually-enumerated catalog tests live inline at `crates/service-api/src/notification.rs:469-585`. Add an explicit `PushEvent` case there, plus arms in the `class()`, `service_generation()`, `set_service_generation()` exhaustive matches. Mirrors the Phase 3 pattern for `SyncCompleted` / `IndexCommitted`.

## Detailed task list

In recommended commit order. Each item is one focused commit unless noted.

1. **`service-api`: `PushEvent` notification variant.** New `Notification::PushEvent { account_id: String, service_generation: u32 }`. `WithGeneration` impl. `Notification::class()` arm returns `Coalesce { key: account_id.clone() }`. `service_generation` / `set_service_generation` arms added exhaustively. Catalog test cases at `crates/service-api/src/notification.rs:469-585` gain an explicit `PushEvent` line. New `BootPhase` variant work (or anything in `crates/service-api/src/boot.rs`) is *not* part of this commit - Phase 4 deliberately ships no new boot phase. Type-only commit.

2. **`jmap::push`: auth resolver parameter.** `jmap::push::start_push` gains a new parameter: `auth_resolver: Arc<dyn Fn() -> BoxFuture<'static, Result<String, String>> + Send + Sync>`. `push_connection_loop` calls `auth_resolver().await` before each connect attempt instead of using the captured `auth_header`. The `auth_header` parameter (line 144 today) is removed. All existing call sites - which today are only `crates/core/src/jmap_push.rs` - update to construct the resolver. Tests in `crates/jmap/src/push.rs` may use a closure that returns a fixed token. **No behavior change for Phase 3** beyond the resolver shim - the resolver in `core/src/jmap_push.rs` calls today's static-header logic until task 3 replaces it. Single commit; the JMAP crate alone.

3. **`crates/service/src/push.rs`: `PushRuntime`.** New file. Per-account map (`HashMap<String, AccountEntry { handle, cancel }>`); bridge-task spawn under panic supervisor; `start_account` (reads account row internally - no-ops for non-JMAP, calls `client.ensure_valid_token().await` before `start_push`, supplies a resolver closure that re-resolves via `ensure_valid_token` on each reconnect); `cancel_account` (cancel token, await handle); `shutdown` (cancel + await all). Bridge runs `manager.stop_push().await` on its exit path. Calls `SyncRuntime::start_account` on each debounced kick. Emits `Notification::PushEvent` via `tokio::time::timeout(D, channel.send(n))`. Module-level doc-comment carries all four code-comment-requirement strings (see § "Code-comment requirements").

4. **Drain consolidation (Phase 3 sidebar fix + Phase 4 push step).** Extract a `service::lifecycle::shutdown_drain(...)` (or `dispatch::run_shutdown_drain`) helper that owns the full drain sequence in one place. Move the sentinel write *out of* `lifecycle::run_drain` (`crates/service/src/lifecycle.rs:72-109`) into the new helper, *after* `SyncRuntime::shutdown`. Move the `SyncRuntime::shutdown().await` call out of `dispatch.rs:321-326` into the new helper. Insert `PushRuntime::shutdown()` as the new step 1. Doc-comment on the helper carries the drain-order code-comment-requirement string. Update `problem-statement.md`'s "Phase 3 status (as landed)" section to note the drain consolidation as a Phase 3 carry-forward fixed in Phase 4.

5. **Post-ready push startup task.** In `dispatch.rs` (or wherever the post-`boot.ready` handshake completes), spawn a runtime task that iterates JMAP accounts (`SELECT id FROM accounts WHERE provider = 'jmap'`) and `tokio::spawn`s a `PushRuntime::start_account(account_id)` call per account. Per-account failure is log-and-continue. The Service holds `Arc<PushRuntime>` in the same incarnation-Arc-bag that already holds `Arc<SyncRuntime>`. **No new `BootPhase` variant.** Readiness must not depend on push setup.

6. **Sync-handler piggyback (start + cancel).** `crates/service/src/handlers/sync.rs::handle_start_account` gains a `tokio::spawn(push_runtime.start_account(account_id))` call alongside the existing sync-start work. Detached; result is logged, not surfaced through the `SyncStartAck`. `handle_cancel_account` gains a symmetric `tokio::spawn(push_runtime.cancel_account(account_id))` call. Provider gating happens inside the runtime methods, not here.

7. **UI teardown (single commit; deletions only).** Delete:
   - `crates/app/src/handlers/provider.rs`: `start_jmap_push`, `JmapPushReceiver`, `create_jmap_push_channel`, `jmap_push_subscription`.
   - `crates/app/src/app.rs`: `jmap_push_tx`, `jmap_push_receiver` fields and their construction.
   - `crates/app/src/message.rs`: `JmapPushKick` variant.
   - `crates/app/src/update.rs`: the `Message::JmapPushKick` arm at line 701.
   - `crates/app/src/subscription.rs`: the `jmap_push_subscription` line at 57 + the `use` at line 4.
   - `crates/app/src/handlers/core.rs:1014`: the `start_jmap_push()` call.
   - `crates/core/src/jmap_push.rs`: deleted in its entirety; the bridge logic now lives in `crates/service/src/push.rs`. Remove the `pub mod jmap_push` line in `crates/core/src/lib.rs`.

8. **UI: `PushEvent` notification handling.** Add a `Notification::PushEvent` arm to the reader-task notification dispatch in `crates/app/src/service_client.rs`. New `Message::PushEvent(account_id)` arm in `update.rs`. Updates a status-bar field (extend `StatusBar` struct, not `ReadyApp` directly - status-bar state belongs in `StatusBar`). No sync action - the Service has already kicked sync by the time this arrives.

9. **Test cohort.** Phase 4 unit + integration + real-subprocess tests below. Lands incrementally with the commits above where natural; this task is the close-out commit.

10. **Doc updates.** Update `problem-statement.md` with a new "Phase 4 status (as landed)" section, mirroring Phase 3's. Update `implementation-roadmap.md`'s Phase 4 entry to reflect the corrected OAuth-refresh story and the explicit Phase 8 carry-forwards (push state hardening + re-auth re-arm, both already added to Phase 8 by this revision pass). Bundle with the close-out commit per CLAUDE.md's "no markdown-only commits" rule.

## File-by-file changes

**New files:**
- `crates/service/src/push.rs` - `PushRuntime`, bridge tasks, panic supervisor, auth resolver closure.

**Modified files:**
- `crates/service-api/src/notification.rs` - add `PushEvent` variant; `WithGeneration` impl; class / generation arms; catalog test cases inline at lines 469-585.
- `crates/jmap/src/push.rs` - replace captured `auth_header` parameter with `auth_resolver: Arc<dyn Fn() -> BoxFuture<...>>`; `push_connection_loop` re-resolves before each connect attempt.
- `crates/service/src/lifecycle.rs` - sentinel write *removed* from `run_drain`; relocated into the new consolidated drain helper.
- `crates/service/src/dispatch.rs` - `SyncRuntime::shutdown` call removed from inline shutdown sequence; relocated into the consolidated drain helper. Also: post-`boot.ready` task that spawns push startup per JMAP account.
- `crates/service/src/lib.rs` (or wherever the Service-incarnation Arc-bag lives) - hold `Arc<PushRuntime>`; pass to sync handlers and the consolidated drain helper.
- `crates/service/src/handlers/sync.rs` - `handle_start_account` and `handle_cancel_account` gain detached push hooks.
- `crates/app/src/service_client.rs` - reader-task notification dispatch gains a `Notification::PushEvent` arm.
- `crates/app/src/handlers/provider.rs` - **delete** push-side functions.
- `crates/app/src/app.rs` - **delete** `jmap_push_tx`, `jmap_push_receiver` fields.
- `crates/app/src/message.rs` - **delete** `JmapPushKick`; **add** `PushEvent(String)` variant.
- `crates/app/src/update.rs` - **delete** `JmapPushKick` arm; **add** `PushEvent` arm.
- `crates/app/src/subscription.rs` - **delete** push subscription wiring.
- `crates/app/src/handlers/core.rs` - **delete** `start_jmap_push()` call.
- `crates/app/src/ui/status_bar.rs` (or `StatusBar`'s home file) - new `last_push_at: HashMap<String, Instant>` field on `StatusBar`.
- `crates/core/src/lib.rs` - **delete** `pub mod jmap_push`.

**Deletions:**
- `crates/core/src/jmap_push.rs` - file deleted; logic moved to `crates/service/src/push.rs`. Phase 4 is the first phase that genuinely deletes a file (Phase 3's "no deletions" rule was per-phase, not project-wide).

**Files explicitly NOT touched:**
- `crates/service-api/src/boot.rs` - no new `BootPhase` variant. Push startup is post-ready, not a boot phase.
- `crates/service/src/boot.rs` / `boot_progress.rs` - same reason.

## Code-comment requirements

The decisions from the revision history must appear as code comments where the relevant logic lives, so future readers cannot miss them. All five are blocking on the relevant commit:

1. **`crates/service/src/push.rs` module-level doc-comment** must contain:
   - "OAuth refresh runs in-Service. Phase 4 calls `JmapClient::ensure_valid_token` before `jmap::push::start_push`, and threads an auth resolver into `push_connection_loop` so reconnects re-resolve the bearer token. No IPC handshake to the UI is needed; the Phase 4 roadmap entry's `oauth.refresh_request` IPC was removed because refresh is purely DB+HTTPS, both Service-internal."
   - "Phase 4 inherits today's resume-from-saved-state behavior in `jmap::push::start_push` (`crates/jmap/src/push.rs:147, 337`). On clean shutdown this is the optimal path. On crash, the resumed connection may deliver a stale `StateChange`; Phase 3's invariant pass already clears `history_id` for crashed accounts, so the resulting delta sync re-fetches the cached window. Phase 8 will harden this with explicit crash-aware fresh-start logic."
   - "Re-auth dead-entry gap: UI-side re-auth (`AddAccountWizard::new_reauth`) updates the existing account row in place and does NOT trigger `PushRuntime::start_account`. A token-revocation kills push for that account until Service restart even after the user re-authorizes. Phase 8 wires push re-arm to a token-refresh-success event."

2. **`crates/service/src/push.rs::PushRuntime` type doc-comment** must contain:
   - "Structurally symmetric with `crates/service/src/sync.rs::SyncRuntime`. Per-account map, panic supervisor, lifecycle hooks. Diverging from `SyncRuntime`'s shape is a refactor smell - if you're tempted, fix the shared abstraction instead."

3. **`crates/service/src/push.rs::AccountEntry::cancel`-path comment** must contain:
   - "Cancellation order: cancel cooperative token, then await the bridge handle. The bridge runs `manager.stop_push().await` on its exit path before returning. `stop_push()` is the only correct way to stop the WebSocket connection-loop (`crates/jmap/src/push.rs:199, 439`); dropping the manager alone leaves a `watch::Receiver` held by the connection-loop and the loop never observes the shutdown signal."

4. **The consolidated drain helper** (location TBD - either `crates/service/src/lifecycle.rs::shutdown_drain` or `crates/service/src/dispatch.rs::run_shutdown_drain`) doc-comment must contain:
   - "Drain order: PushRuntime *before* SyncRuntime *before* sentinel write. A `StateChange` arriving mid-shutdown must not call `SyncRuntime::start_account` after `SyncRuntime` has begun draining - that race lets the search writer flush before the new runner finishes writing. The Phase 3 sentinel write previously fired in `lifecycle::run_drain` *before* `dispatch.rs` shut down `SyncRuntime`, claiming a clean state during in-flight sync writes; Phase 4 consolidated the drain to fix that. Push-first prevents the bridge-task race entirely."

5. **`crates/service/src/handlers/sync.rs::handle_start_account`** (and `handle_cancel_account`) inline comment at the `tokio::spawn(push_runtime.start_account(...))` (resp. `cancel_account`) call:
   - `// Detached: PushRuntime::start_account does TLS+HTTPS+OAuth-refresh, which would violate the 5s sync.start_account IPC contract if awaited. Failure is logged inside the runtime, not surfaced through the SyncStartAck.`

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

- `service_subprocess_starts_push_for_jmap_accounts`: spawn the Service with a seeded JMAP account; observe `boot.ready` arrives quickly (push setup happens *after* readiness, not as part of it); observe a `PushEvent` notification on a subsequent state change (driven by a fake JMAP server fixture).
- `service_subprocess_oauth_refresh_during_push`: a JMAP account whose access token expires mid-subscription. The fake JMAP server returns 401 on the captured initial bearer; the auth resolver re-resolves via `ensure_valid_token` (which the test fixture preloads with a refreshed token); observe the WebSocket reconnects with the new bearer and continues. No IPC method named `oauth.refresh_request` ever appears on the wire (regression test for the dropped handshake).
- `service_subprocess_push_resumes_from_saved_state`: clean shutdown after a state change (saving `push_state`); restart Service; observe `WebSocketPushEnable` carries the saved state on the next connection (regression test for the inherited Phase 3 behavior, so a future refactor doesn't accidentally remove it).

### Manual matrix updates

- The "what survives a Service crash" matrix in `problem-statement.md` § "Cross-store crash consistency" gets a new row: "JMAP push subscription". Phase 4 outcome: lost; re-established at next boot. Phase 8 outcome: resumed from saved state. (No table-shape change beyond the new row.)

## Open questions

1. **Account-list source: re-query DB or pass at construction?** `SyncRuntime` re-queries the DB; mirroring is the right call for `PushRuntime`. The post-ready startup task and the `start_account` provider gate both do per-account DB reads. Confirm this matches `SyncRuntime`'s pattern when implementing.
2. **Drain helper home: `lifecycle.rs` or `dispatch.rs`?** The consolidated `shutdown_drain` helper (task 4) needs a single home. Arguments either way: `lifecycle.rs` already owns drain-shaped logic; `dispatch.rs` already owns the `SyncRuntime::shutdown` call site. Pick one and document the reason in a code comment so a future reader doesn't try to re-split the logic.

## Verification (end-to-end)

- A change pushed to a JMAP mailbox triggers a sync inside the Service. The UI is not on the call path. Verify by reading the IPC channel: no `sync.start_account` IPC method call should appear in response to push events.
- Status bar shows "new mail arrived" within the debounce window of the push.
- Token expiry during a Service-side push subscription does not break the connection: the auth resolver re-resolves via `ensure_valid_token`; the WebSocket reconnects with the new bearer.
- Stopping the Service with an in-flight push event does not corrupt sync state: the consolidated drain holds (push → sync → search-writer → sentinel).
- Restarting the Service re-establishes push subscriptions for all JMAP accounts; resume from `push_state` is *enabled* (Phase 4 inherits today's behavior); a single sync kick happens shortly after if state changed during downtime.
- 5s `sync.start_account` IPC timeout is never exceeded by push setup work: push start is detached.
- A token-revocation logged-in test confirms the entry goes dead and the user must restart the Service for push to recover (Phase 4 known-gap, Phase 8 fix). This is a regression-pin, not a feature: if Phase 8 lands and the user doesn't need to restart, the test inverts.

## Promotion criteria

- All Phase 4 tasks landed; `crates/core/src/jmap_push.rs` is gone; `JmapPushKick` is gone; the UI has zero push wiring.
- The auth resolver is wired through `jmap::push::start_push`; mid-subscription token expiry survives reconnect.
- The drain consolidation has landed; sentinel write is *after* `SyncRuntime::shutdown`; push drains first.
- Phase 4 status block added to `problem-statement.md`.
- `phase-4-plan.md` is then retirement-ready: every deferral has an explicit roadmap entry (Phase 8 carry-forwards: push-state hardening, re-auth re-arm, JMAP push-state resume - all already added in earlier passes), every code-comment requirement is present in the relevant file, and no test references this plan as a TODO source.
