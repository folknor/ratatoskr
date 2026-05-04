# The Service - Phase 3 Plan: JMAP sync relocation + Tantivy/body/inline writer lockdown

Companion to `phase-1-plan.md`, `phase-1.5-plan.md`, and `phase-2-plan.md`. Implements Phase 3 of `implementation-roadmap.md`.

## Revision history

**2026-05-04 - arch review revisions.** The first draft of this plan was reviewed by `review arch` (claude + codex sessions); both reviewers flagged a launch-blocker and three structural concerns. Changes incorporated below:

- **`SearchWriteState::on_commit` callback removed.** The original design called `Handle::current().block_on(send)` from inside `IndexWriter::commit()`, claiming the call was always reached from a `spawn_blocking` thread. That claim was false against today's code (`crates/search/src/lib.rs:306` is `pub async fn`, called from `crates/sync/src/persistence.rs:58` and `crates/jmap/src/sync/storage.rs:65` on async paths) and would have deadlocked. **Replaced with a Service-internal writer task** (`tokio::spawn` runner, mpsc command queue, `block_in_place` for the sync tantivy section). `index.committed` notifications fire from the writer task's natural async context, no `block_on`. See § "Search writer task" below.
- **Writer-task design lands in Phase 3, not deferred to Phase 5.** Resolves the deadlock mechanically; collapses ~500 fsyncs to ~50 on a 50k-msg cold sync via size-or-time-triggered commit batching; removes the `Arc<Mutex<IndexWriter>>` mutex-contention scaling cliff before Phase 5 ports other providers onto it.
- **Marker-file gating lands in Phase 3 (minimal form), not Phase 8.** A 5-minute splash freeze on dirty boot of a 1M-message mailbox is unshippable. Phase 3 ships per-account sync markers that bound the scan to "what was in flight at crash." Phase 8 refines with bounded re-scan windows and visible status reporting.
- **Invariant pass extended to DB→store gap detection.** Today's JMAP persistence writes DB first, then body/inline/search concurrently; a crash mid-persist leaves DB rows claiming cached bodies the body store never wrote. The original plan only scanned for store→DB orphans (one direction). Pass now also walks `messages` for store gaps and clears the `body_cached` / equivalent flag so the next sync re-fetches.
- **`pending_syncs` correlation re-shaped around `SyncRunId`.** The original one-oneshot-per-account design couldn't represent user waiters + delete waiters + duplicate callers + crash cleanup. `SyncStartAck` now returns a `run_id`; `SyncCompleted` carries `{account_id, run_id, result}`; the UI's pending map is keyed by `run_id` with `tokio::sync::broadcast` for multiple waiters per run.
- **Account-delete `cancel_and_await` handles `Err(ServiceCrashed)`.** The flow proceeds with deletion if the Service died during cancellation (the sync is provably no longer writing).
- **JMAP push transitional dispatch replaces today's `Message::SyncComplete` mapping.** Today's `subscription.rs:50` maps push events directly to `SyncComplete`; Phase 3 must replace that with a `client.start_sync` IPC kick (the round-trip stays UI-side; Phase 4 lifts the WebSocket).
- **Catalog test gains explicit cases.** The original plan claimed the existing catalog test "automatically covers" new variants; in fact the catalog is manually enumerated. Explicit `SyncCompleted` and `IndexCommitted` cases land alongside the wire types.
- **`service-state` stays single-crate for Phase 3.** Reviewers split on whether to break it apart per subsystem now (claude pushed harder; codex was OK with one crate kept narrow). Decision: keep one crate for Phase 3; document the split trigger ("if Phase 6's six additional write halves push the dependency surface across rusqlite + tantivy + zstd + provider-specific deps in a way that hurts incremental compile, split per subsystem then"). The cost of splitting later is mechanical type-moves; the cost of splitting now is three new crates with concurrent Phase 3 work in flight.

**2026-05-04 - second arch + bugs review pass.** The first revision pass introduced new structural surfaces; both `review arch --session` against the revisions and `review bugs --oneshot` flagged blockers and high-severity issues in those new surfaces. Changes incorporated below:

- **`pending_syncs` reshaped to handle subscribe-after-completion race (B1).** First revision's design subscribed *after* the request ack returned, racing fast `SyncCompleted` notifications. Replaced with a `PendingSync { Pending(broadcast::Sender) | Completed(SyncResult) }` enum: late subscribers find a latched result instead of parking forever. See Architecture § "UI sync dispatch rewiring".
- **`route_sync_completed` uses `lock().await`, not `now_or_never().expect("not contended")` (B2).** The `expect` was reachable - `start_sync` and `cancel_and_await` both hold the same async mutex across `subscribe` calls. The reader task is async-context anyway; the awaited lock is the right primitive.
- **Sync runner panic supervisor wired (B3).** `run_sync` is wrapped in a supervisor task that observes `JoinError` and emits a synthetic `SyncResult::Failed("runner panicked: ...")` so subscribers and `cancel_and_await` cannot strand. Marker survives panicked runs; invariant pass on next boot repairs.
- **Drain order corrected (H4).** Cancel + await runners FIRST; only then send `FlushNow` to the search writer + drop `SearchWriteHandle` + await writer task's join; only then unlink markers (cleanly-completed runs only); only then write the sentinel. Earlier shape lost post-flush index writes from runners that hadn't yet stopped producing.
- **`IndexCommitted` send has a 30 s deadline (H5).** `notification_tx.send_timeout(...)` rather than unbounded `send().await`. On timeout: log a warning, drop the notification, continue. The index is committed; the UI will catch up on the next `IndexCommitted` (or via the periodic reader-reload safety net added in this pass).
- **Cancellation checkpoint list expanded (H6).** First revision listed per-mailbox + per-batch + pre-network. Now also covers: token refresh / WebSocket reconnect, shared-account sync loops, principal resolution, share-notification polling, and the Calendar + Contacts entries. Token threading enumerated through every JMAP module that does network I/O during a sync.
- **Invariant pass replaced with "nuke history_id" approach (H7, H8, H9, H10).** First revision's per-message DB→store gap repair scanned `messages WHERE updated_at >= ?` and cleared `body_cached` flags. Both reviewers flagged: those columns don't exist; clearing flags doesn't make JMAP delta re-fetch (cursor only fetches *changed* IDs); store-write errors after DB commit can be silently swallowed; drain unlinks markers before sentinel so failed-then-clean-shutdown loses repair signal. Phase 3 now ships a coarser but provably-correct repair: a marker for a non-`Completed` run causes the boot path to call `clear_account_history_id(account_id)` for that account, forcing the next sync to be initial-style (re-fetches the cached-window from the provider). The orphan-drop scans (Tantivy / body / inline scoped to the dirty account) still run because they're cheap. DB→store gap *detection* is dropped from Phase 3; the nuke-cursor approach makes per-row repair unnecessary.
- **Marker survival policy specified.** Markers are written with `status: "in_progress"`; the runner updates to `status: "completed" | "cancelled" | "failed"` on exit. Drain unlinks only `completed` markers. `failed` and `cancelled` and any `in_progress` (Service died) survive into the next boot's invariant pass.
- **`spawn_search_writer` lives in `service`, not `service-state` (dep-graph inversion).** First revision had `service-state::spawn_search_writer` taking a `NotificationSender` from the `service` crate - dep inversion. Resolution: `service-state` exports `SearchWriteHandle` + `WriterCommand` types; `service::search_writer::spawn` is the construction point.
- **Multi-thread runtime flavor pinned.** `tokio::task::block_in_place` panics on `current_thread`. Service's tokio runtime is asserted multi-thread at boot; tests using the writer task use `#[tokio::test(flavor = "multi_thread")]`.
- **Stale "Out of scope" paragraph removed.** First revision's body had `Arc<Mutex<IndexWriter>>` listed as the Phase 3 choice in § "Out of scope"; that contradicted everything else in the doc.
- **`is_deleting` gates `SyncTick` and Service-side `start_account`.** First revision flipped the UI flag immediately but didn't say what stops the next tick from kicking sync against the row-about-to-be-deleted. Both ends now check.
- **Marker file write atomicity.** Temp-file-then-rename. Parse-failure on boot: treat as fully-dirty (force history_id clear). Stated explicitly in § "Sentinel file lifecycle".
- **`started_at` uses monotonic time** (UTC unix-millis stays in the file for human-readability, but the cursor-clear decision doesn't depend on it - the marker's *existence* with `status != completed` is sufficient signal). Sidesteps NTP / suspend-resume clock-skew.
- **Wire types corrected.** `SyncResult::ServiceCrashed` and `AlreadyInFlight` are not on the wire; they're UI-side conditions surfaced via `ClientError`. `SyncResult` on the wire is `Completed | Cancelled | Failed(String)` only. Architecture text and pseudocode updated.
- **mpsc / broadcast / `pending_docs` capacities pinned.** Search writer mpsc: 256. UI broadcast per `run_id`: 8. Notification queue: existing 1024. `pending_docs` increments on `Index`; `Delete` adds 1 per id; `Clear` resets to 0; `FlushNow` resets to 0; `first_uncommitted` clears on every commit.

## Context

Phase 2 moved action *execution* across the boundary; Phase 3 moves the next workload, JMAP delta sync, plus everything that's entangled with it: the Tantivy writer, the body store writer, and the inline-image store writer. This is the first phase where the Service writes to four durable stores in one operation, which makes cross-store crash consistency a real concern; the *minimal* invariant pass lands here as a result, not deferred to Phase 8.

The split is again surgical:

- **UI keeps**: search read path (`SearchReadState` over a Tantivy `IndexReader`), body read path (`BodyStoreReadState`), inline-image read path (`InlineImageStoreReadState`), the sidebar account list, the `SyncTick` cadence, and (transitionally for one phase) the JMAP WebSocket push subscriber.
- **Service gets**: `sync_delta_for_account` and the runner that owns it, all four write halves (DB writes have been Service-side since Phase 2; this phase brings the other three), the cancellation-token map keyed by `account_id`, the boot-time minimal cross-store invariant pass, and the `index.committed` emission cadence.
- **UI's reader reload**: driven by `index.committed` (MustDeliver) notifications, debounced ~200 ms so a heavy initial sync emitting one `index.committed` per 100-message batch doesn't cause a reload-storm.

The phase ships as one milestone with a clean commit-level split: wire types -> read/write split of body store -> read/write split of inline image store -> read/write split of `SearchState` -> sync handler scaffolding -> cancellation checkpoints -> minimal invariant pass -> UI sync rewiring -> push transitional state. A regression should bisect to the right commit.

JMAP first because the JMAP push subscriber forces the migration vehicle: the WebSocket lives UI-side today and stays UI-side for one transitional phase; Phase 4 lifts it. Other providers' sync paths are simpler ports once the IPC shape is settled here.

## Scope

### In scope

1. **`service-api` sync surface.** New methods: `sync.start_account { account_id }` returning `SyncStartAck { account_id, already_in_flight: bool }` (5 s timeout - just enqueue + spawn the runner; the actual sync runs to completion in the worker), `sync.cancel_account { account_id }` returning `SyncCancelAck { account_id, was_in_flight: bool }` (5 s timeout). New notifications: `sync.completed { account_id, generation, result: SyncResult }` (`MustDeliver` - a dropped completion leaves the UI's pending future hanging forever), `index.committed { generation }` (`MustDeliver` - a dropped commit notification leaves the reader stale until the next commit), `sync.progress { account_id, phase, current, total, folder, generation }` (`Coalesce { key: SyncProgress(account_id) }` - already exists from Phase 2 staging, but no production emit site exists today; Phase 3 wires the first one).

2. **`BodyStoreState` read/write split.** Today's `BodyStoreState` (`crates/stores/src/body_store.rs`) holds an `Arc<Mutex<Connection>>` and exposes both `get`/`get_batch`/`get_batch_sync` (read) and `put_batch` (write). Phase 3 splits:
   - `crates/stores/src/body_store.rs::BodyStoreReadState` - read-side only. Keeps `get`, `get_batch`, `get_batch_sync`, `conn()` for the existing read patterns. UI constructs this.
   - `crates/service-state/src/body_store_write.rs::BodyStoreWriteState` - write-side only. Hosts `put_batch`. Service constructs this.
   - Both wrap separate `Arc<Mutex<Connection>>` instances opened against the same on-disk SQLite file (`bodies.db`). SQLite's WAL handles multi-reader-single-writer; both halves are correct independently. The lockdown is type-level: the `app` crate cannot reach `BodyStoreWriteState` because it does not depend on `service-state`.

3. **`InlineImageStoreState` read/write split.** Same shape: `InlineImageStoreReadState` in `crates/stores/src/inline_image_store.rs` (keeps `get_batch_sync` and the rest of the read API); `InlineImageStoreWriteState` in `crates/service-state/src/inline_image_store_write.rs` (hosts `put_batch`).

4. **`SearchState` read/write split with a writer-task handle.** Today's `SearchState` (`crates/search/src/lib.rs:184`) holds both a `tantivy::IndexReader` and an `Arc<Mutex<IndexWriter>>`. Phase 3 splits:
   - `crates/search/src/lib.rs::SearchReadState { reader: IndexReader, fields: Fields }`. Keeps `search_with_filters`, `reload()`, and the `Fields` accessor. UI constructs this.
   - `crates/service-state/src/search_write.rs::SearchWriteHandle { tx: mpsc::Sender<WriterCommand> }`. Cheap to clone; the actual `IndexWriter` lives inside the writer task (see § "Search writer task" in Architecture). Public methods (`index_messages_batch`, `delete_messages_batch`, `delete_message`, `clear_index`, `flush_now`) send commands over the mpsc and await an oneshot ack. **No `Arc<Mutex<IndexWriter>>` in the public type; no `on_commit` callback.**
   - `tantivy::Index` is a singleton on disk; the read half (`SearchReadState`) and the writer-task open independently against the same `<app_data>/search_index/` directory. Tantivy enforces single-writer via its lock file (`.tantivy-writer.lock`); Tantivy >=0.21 recovers a stale lock from an uncleanly-killed Service.
   - The writer task owns commit cadence: it commits when N docs are queued OR M ms elapsed since the first uncommitted doc OR a `FlushNow` command arrives (`sync.completed`/`sync.cancelled`/`Shutdown` paths). `index.committed` notifications fire from the task's async context after each commit; no `block_on`, no spawn_blocking gymnastics.

5. **`sync_delta_for_account` relocation - `ProviderCtx` shape change, dispatch site moves.** Today's `crates/core/src/sync_dispatch.rs::sync_delta_for_account` constructs a `ProviderCtx` from a `&ReadDbState` plus references to the unified body / inline / search states. The function itself stays in `core` (provider-agnostic; the sync code in `crates/sync/` and `crates/jmap/sync/` doesn't need to know about the process boundary). What changes is the `ProviderCtx` shape and the dispatch site:
   - **`ProviderCtx` shape change**: takes `&BodyStoreWriteState`, `&InlineImageStoreWriteState`, `&SearchWriteHandle` instead of the unified types. Mirrors Phase 2's narrowed `ActionProviderCtx` pattern.
   - **Dispatch site moves**: today `crates/app/src/handlers/provider.rs::dispatch_sync_delta` calls `sync_delta_for_account` directly through `Task::perform`. After Phase 3, the dispatch site is `crates/service/src/handlers/sync.rs::handle_start_account` -> spawns a Service-owned runner that calls `sync_delta_for_account`. The UI's `dispatch_sync_delta` becomes `client.start_sync(account_id)`.

6. **Service-side per-account sync runner + cancellation tokens.** New `crates/service/src/sync.rs::AccountSyncMap`:
   ```rust
   pub struct AccountSyncMap {
       inner: Mutex<HashMap<String, AccountSyncEntry>>,
   }
   struct AccountSyncEntry {
       cancellation_token: CancellationToken,
       join_handle: JoinHandle<()>,
   }
   ```
   `start_account` looks up the entry; if present and `join_handle` is not finished, returns `already_in_flight: true` without spawning a duplicate. Otherwise spawns a fresh runner with a fresh `CancellationToken`. `cancel_account` flips the existing entry's token; the runner observes it at the next checkpoint, returns `SyncResult::Cancelled`, drops out of the spawned task. The `JoinHandle` finishing is the "done" signal; `start_account` cleans up finished entries opportunistically.

7. **Cancellation checkpoints in JMAP sync.** `tokio_util::sync::CancellationToken` flows through `SyncCtx<'_>`. Insertion sites:
   - `crates/jmap/src/sync/mod.rs` - top of the per-mailbox loop (currently around the per-mailbox dispatch in `sync_delta`).
   - `crates/jmap/src/sync/mod.rs` - top of the per-batch loop within `sync_mailbox_changes` and the email-fetch batches.
   - `crates/jmap/src/sync/storage.rs` - top of the persist-batch loop (so cancel after the network round-trip but before the DB write returns promptly).
   - `discover_shared_accounts` and `mailbox_changes` entry points - before the network round-trip.
   The contract: cancel-mid-sync returns within 5 s on a healthy connection (a pathological provider that takes 60 s to respond to a single in-flight request runs that one to completion - documented as accepted limit). The checkpoint pattern: `if ctx.cancellation_token.is_cancelled() { return Err(SyncError::Cancelled); }`.

8. **Tantivy index initialization in the boot handshake.** Today's `SearchState::init` creates `<app_data>/search_index/` if it doesn't exist. After Phase 3 the Service is the only process that does this; if the UI runs first against a fresh data dir, `IndexReader::open()` would race the writer's directory creation. The fix: `spawn_search_writer` runs in the Service's boot sequence as a new boot phase `BootPhase::OpeningSearchIndex`, before `boot.ready`. The UI's `SearchReadState::open` defers until after the boot handshake completes, so the directory is guaranteed to exist (and has at least the initial empty segment).

9. **Boot phase additions.** New `BootPhaseKind` variants on the Service side (each emits `BootProgress` notifications):
   - `OpeningBodyAndInlineStores` - writer halves of body store + inline image store init. Fast (<100 ms typical).
   - `OpeningSearchIndex` - Tantivy writer init. Fast unless the index is huge and tantivy needs to scan segments (rare, <500 ms typical even on a 200 GB mailbox).
   - `RunningInvariantPass` - the minimal cross-store scan. Conditional on missing clean-shutdown sentinel. Slow on big mailboxes (10 s on 100k messages, 5 minutes on 1M; bounded but not tight).

10. **Cross-store invariant pass via clear-history-id + per-account sync markers.** New `crates/service/src/startup_invariants.rs`. Runs in `BootPhase::RunningInvariantPass` only if `<app_data>/clean_shutdown` is missing.
    - **Per-account sync markers.** The Service writes `<app_data>/sync_markers/<account_id>.json` (containing `{ run_id, started_at, kind, status }`) at the start of every sync run with `status: "in_progress"`; the runner updates `status` to `"completed" | "cancelled" | "failed"` on exit (atomic temp-file-then-rename). Drain unlinks only `completed` markers; `failed` / `cancelled` / `in_progress` survive. On dirty boot, every surviving non-`completed` marker becomes a `DirtyAccount`. **Boot-side parse-failure rule:** an unparseable marker is treated as fully-dirty for that account.
    - **Per-account repair: clear the JMAP cursor.** For each dirty account, call `clear_account_history_id` (existing helper at `crates/sync/src/pipeline.rs:154`). The next sync becomes initial-style and re-fetches the cached window from the provider; this repopulates body / inline / search regardless of which leg was partial. Why nuke rather than per-row repair: the schemas the per-row design needed (`messages.updated_at`, `bodies.account_id`, an `inline_present` flag) don't exist; clearing flags doesn't make a JMAP delta cursor re-fetch (cursor only returns *changed* IDs); store-write errors after DB commit can be silently swallowed today, so per-row scans have no signal to act on. See § "Minimal cross-store invariant pass" in Architecture for the full reasoning.
    - **Defense-in-depth: orphan-drop scans.** For each dirty account, drop Tantivy / body / inline orphans scoped to that account. Cheap; redundant with the cursor-clear (next sync would clean these up too) but avoids leaving stale data visible during the gap.
    - All scans are idempotent. Each logs a summary: `{ history_ids_cleared, tantivy: ScanResult, body_store: ScanResult, inline: ScanResult }`. Boot blocks until the pass completes for dirty accounts; clean accounts skip entirely. Phase 8 refines with bounded re-scan windows + visible status reporting.

11. **Clean-shutdown sentinel writer extension.** `crates/service/src/lifecycle.rs` already writes `<app_data>/clean_shutdown` at the end of the drain. Phase 3 adds: at boot, if the sentinel exists, *consume it* (delete) before signaling `boot.ready`. If it does not exist, run the invariant pass first. The consume-on-boot semantic is what makes a SIGKILL on the next launch correctly trigger the scan.

12. **Service-side progress reporter.** New `crates/service/src/progress.rs::ServiceProgressReporter` impl of `db::ProgressReporter`. The trait method `emit_json(event_name, json)` keeps its existing string-keyed shape for compatibility with non-sync emit sites (and to avoid a Phase-3 trait surgery), but the Service impl pattern-matches on `event_name == "jmap-sync-progress"` to extract `account_id` and reshape into `Notification::SyncProgress`. The class-aware enqueue (`boot_progress::emit_classified`) delivers it to the UI with the correct `Coalesce { key: SyncProgress(account_id) }` policy.

13. **`index.committed` emission from inside the writer task.** The writer task owns the `IndexWriter` and runs in a regular `tokio::spawn` async context. After every successful `commit()` (executed via `tokio::task::block_in_place` since `IndexWriter::commit` is sync), the task awaits `notification_tx.send(Notification::IndexCommitted(...))` - a normal awaited `MustDeliver` send, no `Handle::current().block_on(...)`. See § "Search writer task" in Architecture for the full task body and cadence triggers (size threshold, time threshold, `FlushNow` command).

14. **UI-side `IndexReader` reload (debounced).** `App` holds a single `Arc<SearchReadState>`; on `Notification::IndexCommitted`, schedule a debounced reload via `App.pending_reader_reload: Option<Instant>`. A periodic `iced::time::every(Duration::from_millis(200))` subscription emits a `Message::ReaderReloadTick`; on tick, if `pending_reader_reload.is_some()` and at least 200 ms has elapsed since the last `IndexCommitted`, call `reader.reload()` and clear. This collapses 50 commits/sec under heavy initial sync into ~5 reloads/sec; reader reload itself is cheap (cooperative; no work happens until the next searcher acquires) but the debounce protects search call sites that may pin a `Searcher`.

15. **UI sync dispatch rewiring with `SyncRunId` correlation.** `crates/app/src/handlers/provider.rs::dispatch_sync_delta` becomes `client.start_sync(account_id)` returning a future that resolves on the matching `sync.completed` notification. `App.sync_handles: HashMap<String, iced::task::Handle>` is retired. Correlation is by `run_id`, not `account_id`, so multiple waiters per run (user-initiated `start_sync` + an `account-delete cancel_and_await` racing it + a duplicate tick-driven kick) all resolve cleanly:
    ```rust
    pub struct SyncRunId(pub uuid::Uuid);  // service-api; Service-generated UUIDv7

    pending_syncs: Mutex<HashMap<SyncRunId, broadcast::Sender<SyncResult>>>,
    ```
    `SyncStartAck` carries `{ account_id, run_id, already_in_flight }`. The `SyncResult` wire enum is `Completed | Cancelled | Failed(String)` only - `ServiceCrashed` is a UI-side `ClientError`, never on the wire. The `pending_syncs` map uses a `PendingSync { Pending(Sender) | Completed(SyncResult) }` enum so a fast `SyncCompleted` arriving before the subscriber inserts is latched for the late subscriber instead of being dropped (see Architecture § "UI sync dispatch rewiring" for the full design). `SyncCompleted { account_id, run_id, result }` notification is routed by `run_id`.
    Cross-respawn handling: on `ClientError::ServiceCrashed`, every `Pending` entry's `broadcast::Sender` is dropped (subscribers receive `RecvError::Closed` -> `Err(ServiceCrashed)`); `Completed` entries are discarded. The respawned Service starts fresh; the next `SyncTick` re-issues `start_sync` against the new incarnation, generating a fresh `run_id`.

16. **Account-deletion cancellation via `cancel_and_await`.** `crates/app/src/handlers/core.rs::handle_delete_account` (today calls into `update.rs:321`'s sync-handle abort path; existing race is documented at `handlers/core.rs:631`) calls `client.cancel_and_await(account_id).await` before dispatching the row delete. The helper wraps "issue `sync.cancel_account`, look up the active run's `PendingSync` entry by `run_id` from the cancel ack (latched-completed-aware), await result":
    ```rust
    set_account_is_deleting(db, &account_id, true).await?;  // hides row immediately

    match client.cancel_and_await(account_id).await {
        Ok(SyncResult::Cancelled) | Ok(SyncResult::Completed) | Ok(SyncResult::Failed(_)) => { /* sync is provably done; proceed */ }
        Err(ClientError::ServiceCrashed) => { /* Service is dead; sync is dead with it; proceed */ }
        Err(e) => { /* surface error to user; clear is_deleting and abort */ }
    }
    delete_account_row(...).await
    ```
    `is_deleting` is also checked Service-side at `start_account` entry (defense-in-depth) and UI-side in `SyncTick`'s account list filter (so the next tick can't kick a sync against the about-to-be-deleted row). See Architecture § "Account-deletion cancellation" for the both-ends gate.

17. **JMAP push transitional state.** `JmapPushReceiver` channel + `jmap_push_subscription` in `crates/app/src/handlers/provider.rs` stays UI-side for Phase 3. **Today's mapping (`crates/app/src/subscription.rs:50` maps push events to `Message::SyncComplete` directly) does not survive Phase 3 - it must be replaced with a `client.start_sync(account_id)` IPC kick.** The new dispatch: iced subscription emits `Message::JmapPushKick(account_id)`; handler calls `client.start_sync(account_id)` and discards the future (fire-and-forget; the next tick will also drive sync). Documented round-trip in this plan and in `problem-statement.md`'s "transitional state" section. Phase 4 lifts the WebSocket subscriber into the Service and removes the transit hop entirely.

18. **Generation tagging for new notifications.** `WithGeneration` impls on `SyncCompleted` and `IndexCommitted` payloads. `Notification::service_generation` and `Notification::set_service_generation` arms extended exhaustively. The catalog test (existing, from Phase 1.5 commit 30) automatically covers the new variants once the arms are added.

19. **Removal of UI-side write call sites.** After Phase 3, no `app` crate source file constructs a `BodyStoreWriteState`, `InlineImageStoreWriteState`, or `SearchWriteHandle`. The action service (Phase 2) already routes through Service-side write halves; sync (this phase) does the same. The `crates/core/src/search_pipeline.rs` paths use `SearchReadState`'s `search_with_filters` only - it has no writer dependency and stays UI-side.

### Out of scope

- **Other providers' sync paths** (Phase 5 ports Gmail, Graph, IMAP, Calendar one at a time once the IPC shape is settled here).
- **Push subscriber relocation** (Phase 4 - the WebSocket and IDLE listeners move into the Service).
- **Optimised invariant pass** (Phase 8 - marker-file gating + bounded re-scan windows + visible status reporting).
- **Full `WriteDbState` lockdown** (Phase 6 - the rest of the UI write surfaces relocate then).
- **Attachment text extraction & indexing** (Phase 7).
- **Per-account sync concurrency >1**. Today's policy is one sync at a time per account, parallel across accounts; Phase 3 keeps this exactly as-is.
- **Per-row DB→store gap repair.** The first revision's plan to scan `messages` for rows whose schema flag asserted body/inline/search presence is dropped; replaced by clear-history-id (see § "Minimal cross-store invariant pass" for why). Per-row repair re-emerges only if Phase 7 / 8 tightens the cost of full re-syncs.
- **The other six write surfaces in the inventory** (preferences, accounts, signatures, drafts, pinned searches, calendar mutations) - all Phase 6.

## Architecture

### `BodyStoreState` / `InlineImageStoreState` read/write split

Today's `BodyStoreState` (`crates/stores/src/body_store.rs:30-75`) is `Clone` and exposes both read methods (`get`, `get_batch_sync`, `get_batch`, `conn`) and the write method (`put_batch`). The same shape applies to `InlineImageStoreState` (`crates/stores/src/inline_image_store.rs`).

The split mirrors Phase 2's `db::DbState -> ReadDbState + WriteDbState`:

```rust
// crates/stores/src/body_store.rs
#[derive(Clone)]
pub struct BodyStoreReadState {
    conn: Arc<Mutex<Connection>>,
}

impl BodyStoreReadState {
    pub fn init(app_data_dir: &Path) -> Result<Self, String> { /* opens read-only conn pool */ }
    pub fn conn(&self) -> Arc<Mutex<Connection>> { Arc::clone(&self.conn) }
    pub fn get_batch_sync(&self, ids: &[&str]) -> Result<Vec<MessageBody>, String> { /* ... */ }
    pub async fn get_batch(&self, ids: Vec<String>) -> Result<Vec<MessageBody>, String> { /* ... */ }
    // No put_batch.
}

// crates/service-state/src/body_store_write.rs
#[derive(Clone)]
pub struct BodyStoreWriteState {
    conn: Arc<Mutex<Connection>>,
}

impl BodyStoreWriteState {
    pub fn init(app_data_dir: &Path) -> Result<Self, String> { /* opens writer-capable conn */ }
    pub async fn put_batch(&self, bodies: Vec<MessageBody>) -> Result<(), String> { /* ... */ }
}
```

Both halves open their own SQLite connection against the same `bodies.db` file. SQLite WAL handles multi-reader-single-writer; the writer's `BEGIN IMMEDIATE` blocks until any in-flight read finishes. The on-disk file is the synchronization point; the Rust types just enforce *which side* of the API surface the consumer sees.

The UI's `App` holds `body_store: Option<BodyStoreReadState>` (renamed from `BodyStoreState`); the Service holds the writer half in `BootContext`. Inline image stores follow the same pattern.

**Construction timing.** Today the UI constructs `BodyStoreState` synchronously at `App::boot` time (`crates/app/src/db/threads.rs:240::init_body_store`). After Phase 3 the UI constructs `BodyStoreReadState` after the boot handshake completes (the Service has already initialized the DB, so the on-disk SQLite file exists by the time the UI tries to open it). The Service constructs `BodyStoreWriteState` in `BootPhase::OpeningBodyAndInlineStores`.

### `SearchState` read/write split

Today's `SearchState` (`crates/search/src/lib.rs:183-242`) holds both an `IndexReader` and an `Arc<Mutex<IndexWriter>>`. Phase 3 unwinds the entanglement and the writer's mutex with one move: the writer goes inside a Service-internal task; the public types are read-only on the UI side and a cheap mpsc handle on the Service side. The `IndexWriter` itself never escapes the writer task. Implementation details for the task live in § "Search writer task" below.

```rust
// crates/search/src/lib.rs
#[derive(Clone)]
pub struct SearchReadState {
    reader: IndexReader,
    fields: Fields,
}

impl SearchReadState {
    pub fn open(app_data_dir: &Path) -> Result<Self, String> {
        // Requires the index dir + initial segment to already exist
        // (created by the Service in BootPhase::OpeningSearchIndex).
        // Open MmapDirectory + Index::open_in_dir; build reader.
    }
    pub fn reload(&self) -> Result<(), String> { self.reader.reload().map_err(|e| e.to_string()) }
    pub fn search_with_filters(&self, params: &SearchParams) -> Result<Vec<SearchResult>, String> { /* ... */ }
    pub fn message_indexed(&self, message_id: &str) -> Result<bool, String> { /* invariant-pass helper */ }
}

// crates/service-state/src/search_write.rs - cheap handle, mpsc sender only
#[derive(Clone)]
pub struct SearchWriteHandle {
    tx: mpsc::Sender<WriterCommand>,
}

impl SearchWriteHandle {
    pub async fn index_messages_batch(&self, docs: Vec<SearchDocument>) -> Result<(), String> { /* send + ack */ }
    pub async fn delete_messages_batch(&self, ids: Vec<String>) -> Result<(), String> { /* send + ack */ }
    pub async fn delete_message(&self, id: String) -> Result<(), String> { /* send + ack */ }
    pub async fn clear_index(&self) -> Result<(), String> { /* send + ack */ }
    pub async fn flush_now(&self) -> Result<(), String> { /* send + ack; forces commit */ }
}

pub fn spawn_search_writer(
    app_data_dir: &Path,
    notification_tx: NotificationSender,
    generation: u32,
) -> Result<SearchWriteHandle, String> { /* see § "Search writer task" */ }
```

Tantivy enforces single-writer at the directory level via its `.tantivy-writer.lock` file. Tantivy 0.21+ recovers a stale lock from an uncleanly-killed Service process - verified in the kill-mid-sync test. The version pin lives in `crates/search/Cargo.toml`.

The `Fields` struct (built from the schema) is duplicated across the read state and the writer task; it's small and immutable, so the duplication is fine. Schema changes require both halves to be rebuilt - the boot handshake serializes this naturally (the writer task initializes the schema; the reader opens against it).

### Service-side sync handler shape

```rust
// crates/service/src/sync.rs
pub struct SyncRuntime {
    accounts: Mutex<HashMap<String, AccountEntry>>,
    db: WriteDbState,
    body_write: BodyStoreWriteState,
    inline_write: InlineImageStoreWriteState,
    search_write: SearchWriteHandle,
    encryption_key: SecretKey,
    progress: Arc<dyn ProgressReporter>,
    notification_tx: NotificationSender,
}

struct AccountEntry {
    cancellation_token: CancellationToken,
    join_handle: JoinHandle<SyncResult>,
}

impl SyncRuntime {
    pub async fn start_account(&self, account_id: String) -> SyncStartAck { /* ... */ }
    pub async fn cancel_account(&self, account_id: &str) -> SyncCancelAck { /* ... */ }
}

// crates/service/src/handlers/sync.rs
pub async fn handle_start_account(
    runtime: &SyncRuntime,
    params: SyncStartAccountParams,
) -> Result<SyncStartAck, ServiceError> {
    Ok(runtime.start_account(params.account_id).await)
}
```

`start_account` is non-blocking from the handler's perspective: it acquires the per-account map lock, checks for an existing in-flight runner, spawns one if needed via `tokio::spawn`, and returns within microseconds. The actual sync work runs in the spawned task; per the handler/worker split established in Phase 2, the JSON-RPC ack is sent before the runner does any work.

The runner task structure (with panic supervisor):

```rust
async fn spawn_runner_with_supervisor(
    runtime: Arc<SyncRuntime>,
    account_id: String,
    run_id: SyncRunId,
    cancellation_token: CancellationToken,
) {
    let runner = tokio::spawn(run_sync(
        runtime.clone(),
        account_id.clone(),
        run_id,
        cancellation_token,
    ));

    // Supervisor: observe the runner's JoinHandle. If it panicked,
    // emit a synthetic SyncCompleted so subscribers and cancel_and_await
    // are not stranded forever. Marker is left in `in_progress` state;
    // the next dirty-boot invariant pass will repair (clear history_id).
    match runner.await {
        Ok(()) => {
            // Normal completion: run_sync already emitted SyncCompleted,
            // updated the marker to `completed`/`cancelled`/`failed`,
            // and removed itself from the AccountSyncMap.
        }
        Err(join_err) if join_err.is_panic() => {
            log::error!("sync runner for {account_id} panicked: {join_err:?}");
            runtime.emit_completed(
                &account_id,
                run_id,
                SyncResult::Failed(format!("runner panicked: {join_err}")),
            ).await;
            runtime.write_marker_status(&account_id, MarkerStatus::Failed).await;
            runtime.remove_account_entry(&account_id).await;
        }
        Err(join_err) => {
            // task aborted - shouldn't happen in this design; treat as failed.
            log::warn!("sync runner for {account_id} aborted: {join_err:?}");
            runtime.emit_completed(
                &account_id,
                run_id,
                SyncResult::Failed(format!("runner aborted: {join_err}")),
            ).await;
            runtime.write_marker_status(&account_id, MarkerStatus::Failed).await;
            runtime.remove_account_entry(&account_id).await;
        }
    }
}

async fn run_sync(
    runtime: Arc<SyncRuntime>,
    account_id: String,
    run_id: SyncRunId,
    cancellation_token: CancellationToken,
) {
    let result = sync_dispatch::sync_delta_for_account(
        &runtime.db,
        &account_id,
        runtime.encryption_key.as_bytes(),
        &runtime.body_write,
        &runtime.inline_write,
        &runtime.search_write,
        runtime.progress.as_ref(),
        cancellation_token.clone(),
    ).await;

    let (sync_result, marker_status) = match result {
        Ok(_) => (SyncResult::Completed, MarkerStatus::Completed),
        Err(_) if cancellation_token.is_cancelled() => (SyncResult::Cancelled, MarkerStatus::Cancelled),
        Err(e) => (SyncResult::Failed(e), MarkerStatus::Failed),
    };

    runtime.write_marker_status(&account_id, marker_status).await;
    runtime.emit_completed(&account_id, run_id, sync_result).await;
    runtime.remove_account_entry(&account_id).await;
}
```

Note: the supervisor pattern is required because release-profile `panic = "abort"` doesn't help here - in debug it's a real stranding bug, and in release the whole Service dies (which `ServiceCrashed` *does* cover) but debug behavior diverges silently. The supervisor closes the divergence.

### `ProviderCtx` shape change

The current `common::types::ProviderCtx` (`crates/common/src/types.rs`) takes references to the unified `BodyStoreState`, `InlineImageStoreState`, `SearchState`. Phase 3 narrows this for sync paths the same way Phase 2 narrowed it for action paths:

```rust
pub struct SyncProviderCtx<'a> {
    pub account_id: &'a str,
    pub db: &'a WriteDbState,                       // sync writes through writer half
    pub body_write: &'a BodyStoreWriteState,
    pub inline_write: &'a InlineImageStoreWriteState,
    pub search_write: &'a SearchWriteHandle,
    pub progress: &'a dyn ProgressReporter,
    pub cancellation_token: &'a CancellationToken,
}
```

The cancellation token rides on the ctx so every leaf call has access without threading an extra parameter. `sync_delta_for_account`'s signature changes to take `SyncProviderCtx`; its body builds the `ProviderOps` impl-specific ctx (the JMAP `SyncCtx` already exists and gets a token field added).

The `core::actions::ActionContext` (Phase 2 narrowed) stays as-is - actions don't need search/body/inline write halves (the action skips-search-index policy from Phase 2 § "Architecture deltas as shipped").

### Cancellation propagation

`tokio_util::sync::CancellationToken` is the right primitive: cheap clone, observable from any await point via `.is_cancelled()` or `.cancelled().await`. The token flows from the Service's `AccountEntry` through `SyncProviderCtx` into the JMAP `SyncCtx`.

Cancellation checkpoint sites in JMAP sync (`crates/jmap/src/sync/mod.rs`, `storage.rs`, `mailbox.rs`, plus the broader sync surfaces named below):

| Location | Why |
|----------|-----|
| **Sync entry points** | |
| Top of `sync_delta` | First gate; a cancel during init returns immediately. |
| Top of `discover_shared_accounts` | Account discovery runs at sync start; cancel before discovery completes is a fast exit. |
| Top of `mailbox_changes` | Mailbox metadata fetch runs at sync start; same reasoning. |
| Top of each shared-account sync iteration in `shared_mailbox_sync.rs` | Each shared account is its own sync loop; cancel between iterations. |
| Top of contacts sync (`contacts_sync.rs`) | Contacts entry runs as part of overall sync; cancel between sections. |
| Top of calendar sync (`calendar_sync/mod.rs`) | Same. |
| **Network call sites (pre-await)** | |
| Before each `client.email_get` / `client.email_query` | Avoids burning a network round-trip when cancellation is already requested. |
| Before each `client.mailbox_get` / `client.mailbox_changes` | Same. |
| Before token refresh / WebSocket reconnect (`crates/jmap/src/client.rs`) | Slow path; cancel before reconnect avoids a long-tail wait. |
| Before principal resolution (`mailbox_mapper.rs`) | Per-account, per-sync metadata fetch. |
| Before share-notification polling (`push.rs` polling path, if reached during sync) | Polling loops should observe cancel. |
| **Persist-batch sites** | |
| Top of per-batch loop in `sync_mailbox_changes` (email-fetch batches of ~100) | Cancel mid-mailbox returns within one batch round-trip. |
| In `storage.rs::persist_messages_batch` between batch chunks | DB write batches of 100; cancel between chunks. |
| Between body/inline persistence and search index calls | The three writes happen sequentially per batch (today concurrently; sequencing is part of this phase's reshape). Cancel between them. |

**`select!` vs poll-only.** Cancellation checkpoints today are poll-only (`if cancellation_token.is_cancelled() { return Cancelled; }`). For most call sites this is fine - the next checkpoint is at most one batch (~1 s) away. For long-running operations that *don't* return to a checkpoint promptly (a stalled SQLite WAL contention; a body-store batch flushing many MB; a slow-response from JMAP), a poll-only check can't interrupt. Where the checkpoint is around an awaited call, use `tokio::select!` against `cancellation_token.cancelled()`:

```rust
tokio::select! {
    biased;
    _ = ctx.cancellation_token.cancelled() => return Err(SyncError::Cancelled),
    result = client.email_get(account_id, ids) => result?,
}
```

The `biased` branch is critical here so cancellation wins ties.

**The contract.** A cancellation request fires within 5 seconds on a healthy connection. A pathological provider (single in-flight request takes 60 s to respond) runs that one to completion - but the `select!` arms abort the *await*, so even pathological providers see cancellation observed at the next yield point. Manual matrix entry "cancel during heavy WAL contention" verifies the worst case.

### Search writer task

The first draft of this plan wrapped `IndexWriter` in `Arc<Mutex<_>>` and fired `index.committed` from a synchronous `on_commit` callback that called `Handle::current().block_on(send)`. Both reviewers flagged this as a launch-blocker: `index_messages_batch` is `pub async fn` reached from `crates/sync/src/persistence.rs:58` on plain `.await`, not through `spawn_blocking`, so `block_on` would have deadlocked on the same tokio worker. Replaced with a Service-internal writer task.

```rust
// crates/service-state/src/search_write.rs - the public handle
#[derive(Clone)]
pub struct SearchWriteHandle {
    tx: mpsc::Sender<WriterCommand>,
}

impl SearchWriteHandle {
    pub async fn index_messages_batch(&self, docs: Vec<SearchDocument>) -> Result<(), String> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.tx.send(WriterCommand::Index { docs, ack: ack_tx }).await
            .map_err(|_| "search writer task gone".to_string())?;
        ack_rx.await.map_err(|_| "search writer ack dropped".to_string())?
    }
    pub async fn delete_messages_batch(&self, ids: Vec<String>) -> Result<(), String> { /* ... */ }
    pub async fn delete_message(&self, id: String) -> Result<(), String> { /* ... */ }
    pub async fn clear_index(&self) -> Result<(), String> { /* ... */ }
    pub async fn flush_now(&self) -> Result<(), String> { /* ... */ }
}

// crates/service/src/search_writer.rs - the task
enum WriterCommand {
    Index { docs: Vec<SearchDocument>, ack: oneshot::Sender<Result<(), String>> },
    Delete { ids: Vec<String>, ack: oneshot::Sender<Result<(), String>> },
    Clear { ack: oneshot::Sender<Result<(), String>> },
    FlushNow { ack: oneshot::Sender<Result<(), String>> },
}

pub async fn run_writer_task(
    mut writer: IndexWriter,
    fields: Fields,
    mut rx: mpsc::Receiver<WriterCommand>,
    notification_tx: NotificationSender,
    generation: u32,
) {
    let mut pending_docs: u64 = 0;
    let mut first_uncommitted: Option<Instant> = None;
    const COMMIT_DOC_THRESHOLD: u64 = 1000;
    const COMMIT_TIME_THRESHOLD: Duration = Duration::from_millis(2000);

    loop {
        let deadline = first_uncommitted
            .map(|t| t + COMMIT_TIME_THRESHOLD)
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(3600));

        tokio::select! {
            cmd = rx.recv() => match cmd {
                Some(c) => apply(&mut writer, &fields, c, &mut pending_docs, &mut first_uncommitted, &notification_tx, generation).await,
                None => { let _ = commit_and_notify(&mut writer, &notification_tx, generation).await; break; }
            },
            _ = tokio::time::sleep_until(deadline.into()) => {
                if pending_docs > 0 {
                    let _ = commit_and_notify(&mut writer, &notification_tx, generation).await;
                    pending_docs = 0;
                    first_uncommitted = None;
                }
            }
        }
    }
}
```

Inside `apply`, `add_document` / `delete_term` calls happen in `tokio::task::block_in_place(...)` (multi-thread runtime cooperative bridge from async to sync); after each command the task checks `pending_docs >= COMMIT_DOC_THRESHOLD`, and if so issues a commit. `commit_and_notify` runs the sync `IndexWriter::commit()` inside `block_in_place`, then sends the `MustDeliver` `IndexCommitted` notification with a 30 s timeout:

```rust
async fn commit_and_notify(...) {
    let commit_result = tokio::task::block_in_place(|| writer.commit().map_err(...));
    if commit_result.is_ok() {
        let notif = Notification::IndexCommitted(IndexCommitted { generation });
        match tokio::time::timeout(Duration::from_secs(30), notification_tx.send(notif)).await {
            Ok(Ok(())) => {} // delivered
            Ok(Err(_)) => log::warn!("notification queue closed; UI is probably gone"),
            Err(_) => log::warn!("IndexCommitted send timed out after 30 s; UI consumer wedged. Dropping; the next IndexCommitted will catch up."),
        }
    }
    pending_docs = 0;
    first_uncommitted = None;
}
```

**Why a timeout on a `MustDeliver` send?** The IPC backpressure taxonomy in `problem-statement.md` § "IPC" treats `MustDeliver` as "must reach the UI; never coalesced or dropped." That contract holds for state-change notifications like `action.completed`, where dropping would desynchronize generation tracking. `IndexCommitted` is *advisory*: its only effect is to trigger a UI-side reader reload, and a missed reload is correctable - the next `IndexCommitted` will fire it, and the writer's natural cadence ensures another one arrives within `COMMIT_TIME_THRESHOLD`. A 30 s deadline + warn-and-drop preserves writer forward progress when the UI consumer is wedged. Without the deadline, the writer parks on `send.await`, mpsc fills, `index_messages_batch` awaits, and *cancellation* can no longer return in 5 s because the runner is suspended inside the search write call rather than at a checkpoint - that's the bug H5 caught. The plan should reclassify `IndexCommitted` from `MustDeliver` to a fourth class (provisionally `BestEffort` or `Coalesce { key: IndexCommitted }`) so the wire-level taxonomy reflects the design, but for Phase 3 the deadline-on-send is the load-bearing fix.

**Commit cadence parameters (initial; tunable):**
- `COMMIT_DOC_THRESHOLD = 1000` (about ten JMAP `SYNC_BATCH_SIZE = 100` batches).
- `COMMIT_TIME_THRESHOLD = 2000 ms` since first uncommitted doc.
- `FlushNow` command from `sync.completed` / `sync.cancelled` / `Shutdown` paths.

**`pending_docs` accounting rules.**
- `Index { docs }`: `pending_docs += docs.len()`.
- `Delete { ids }`: `pending_docs += ids.len()` (a delete contributes to the threshold; otherwise a delete-only workload wouldn't commit on size).
- `Clear`: commit immediately (no batching); `pending_docs = 0`; `first_uncommitted = None`.
- `FlushNow`: commit immediately; same reset.
- After any commit (size, time, `Clear`, `FlushNow`): `pending_docs = 0`, `first_uncommitted = None`.
- `first_uncommitted = Some(Instant::now())` set on the first `Index`/`Delete` after each commit.

**mpsc capacity: 256 commands.** A producer that fills the queue (e.g., sync flooding the writer with batches) blocks on `send.await` - intended backpressure. The 256 ceiling lets a single sync run buffer ~25 batches' worth of commands without stalling, which is more than enough for the steady-state cadence.

A 50,000-message cold sync now produces ~50 commits instead of ~500. fsync cost drops ~10x. `IndexCommitted` IPC volume drops ~10x. UI reader reload (still 200 ms debounced) sees ~0.5 reloads/sec under sustained pressure rather than ~5/sec.

**Runtime flavor: multi-threaded only.** `tokio::task::block_in_place` panics on a `current_thread` runtime. The Service's main runtime is constructed `multi_thread` (per `crates/service/src/lib.rs:110`), and `spawn_search_writer` asserts the current runtime supports `block_in_place` at construction:

```rust
pub fn spawn(...) -> Result<SearchWriteHandle, String> {
    if tokio::runtime::Handle::try_current()
        .ok()
        .and_then(|h| h.runtime_flavor() == /* multi_thread */ ...)
        .is_none() {
        return Err("search writer requires multi-threaded tokio runtime");
    }
    // ...
}
```

Tests using the writer task use `#[tokio::test(flavor = "multi_thread")]` (the unit test suite for the writer already does; the in-process integration harness must too).

**Why not `Arc<Mutex<IndexWriter>>` plus the caller wraps `block_in_place`?** Three reasons. (1) Tantivy's `add_document` parallelises internally (its own thread pool); a mutex serialises adds across concurrent producers needlessly, which becomes a Phase 5 multi-account scaling cliff. (2) The cadence policy belongs in one place; "every caller commits" forces repetition. (3) A writer task with bounded queue applies natural backpressure: if sync produces faster than tantivy commits, the mpsc fills, `index_messages_batch` awaits, sync slows naturally.

**Crate ownership: writer task lives in `service`, not `service-state`.** First revision had `service-state::spawn_search_writer` taking a `NotificationSender` from the `service` crate - dep inversion. Resolution: `service-state` exports `SearchWriteHandle` (the mpsc sender) and `WriterCommand` (the enum). The construction site (`service::search_writer::spawn`) and the runner body (`service::search_writer::run_writer_task`) live in `service`, where notification senders are already in-scope. The `app` crate sees only `SearchWriteHandle`, never the task body or its dependencies.

### `index.committed` notification & UI reader reload

```rust
// service-api
pub struct IndexCommitted {
    pub generation: u32,
}

impl WithGeneration for IndexCommitted { /* ... */ }

// app
struct ReadyApp {
    pending_reader_reload: Option<Instant>,
    // ...
}

fn handle_notification(&mut self, n: Notification) -> Task<Message> {
    match n {
        Notification::IndexCommitted(_) => {
            self.pending_reader_reload = Some(Instant::now());
            Task::none()
        }
        // ...
    }
}

fn handle_reader_reload_tick(&mut self) -> Task<Message> {
    let Some(since) = self.pending_reader_reload else { return Task::none(); };
    if since.elapsed() >= Duration::from_millis(200) {
        if let Some(reader) = self.search_state.as_ref() {
            let _ = reader.reload();
        }
        self.pending_reader_reload = None;
    }
    Task::none()
}
```

The 200 ms threshold is conservative; reader reload itself is microseconds (cooperative; no actual segment reload until the next searcher acquires). The debounce is not for performance - it's to avoid pinning a `Searcher` across rapid reload requests, which would force tantivy to keep stale segments mapped.

### Boot phase additions

The existing `BootPhase` enum (Phase 1.5) covers: `LoadingKey`, `OpeningDb`, `RunningMigrations`, `RecoveringPendingOps`, `SweepingQueuedDrafts`, `BackfillingThreadParticipants`. Phase 3 adds:

- `OpeningBodyAndInlineStores` - between `BackfillingThreadParticipants` and `OpeningSearchIndex`.
- `OpeningSearchIndex` - between body/inline store init and the invariant pass.
- `RunningInvariantPass` - between search index init and `boot.ready`. Conditional on missing sentinel.

Each new phase emits `BootProgress` notifications via the existing `boot_progress::emit_classified` path. The phase ordering is fixed; the UI splash renders progress with the existing per-phase labels.

### Minimal cross-store invariant pass

```rust
// crates/service/src/startup_invariants.rs

pub struct DirtyAccount {
    pub account_id: String,
    pub run_id: SyncRunId,
    pub status: MarkerStatus,  // !=Completed by definition (Completed markers are unlinked at drain)
    pub started_at: i64,        // diagnostic only; not used in repair logic
}

pub async fn run_invariant_pass(
    db: &WriteDbState,
    body_write: &BodyStoreWriteState,
    inline_write: &InlineImageStoreWriteState,
    search_write: &SearchWriteHandle,
    search_read: &SearchReadState,
    dirty_accounts: &[DirtyAccount],
    progress: &dyn ProgressReporter,
) -> Result<InvariantPassStats, String> {
    let mut stats = InvariantPassStats::default();

    for account in dirty_accounts {
        // 1. Force next sync to be initial-style by clearing the JMAP cursor.
        //    Provider-side delta semantic only re-fetches *changed* IDs since
        //    the cursor; clearing forces re-fetch of the full cached window,
        //    which cleanly repopulates body / inline / search for any rows
        //    affected by partial writes from the failed sync.
        clear_account_history_id(db, &account.account_id).await?;
        stats.history_ids_cleared += 1;

        // 2. Drop store→DB orphans scoped to the account. Cheap; the account
        //    bound on each scan keeps the work proportional to one account's
        //    state, not the whole mailbox.
        stats.tantivy.merge(scan_tantivy_orphans(db, search_write, search_read, &account.account_id).await?);
        stats.body_store.merge(scan_body_orphans(db, body_write, &account.account_id).await?);
        stats.inline.merge(scan_inline_orphans(db, inline_write, &account.account_id).await?);

        // 3. Delete the marker once repair is complete for this account.
        //    Subsequent boots without further sync activity see no marker.
        unlink_marker(&account.account_id).await?;
    }

    log::info!("Invariant pass: {:?}", stats);
    Ok(stats)
}

#[derive(Debug, Default)]
pub struct InvariantPassStats {
    pub history_ids_cleared: u64,
    pub tantivy: ScanResult,
    pub body_store: ScanResult,
    pub inline: ScanResult,
}

#[derive(Debug, Default)]
pub struct ScanResult {
    pub scanned: u64,
    pub orphans_dropped: u64,
    pub elapsed_ms: u128,
}
```

**Why nuke `history_id` rather than per-row repair?** First-revision plan scanned `messages` for rows whose schema flag asserted a body/inline/search doc was written, then cleared the flag if the store was missing. Both reviewers flagged this approach as broken:

- *Schema mismatch.* `messages` has no `updated_at`; `bodies` has no `account_id` or `updated_at`; inline state is keyed on `attachments.content_hash` not a flag column (verified against `crates/db/src/db/schema/01_core.sql` and `crates/stores/src/body_store.rs`). The per-row predicates were unimplementable without schema additions.
- *Cursor semantic.* Even with the schema in place, clearing `body_cached = 0` does not make a JMAP delta sync re-fetch the row. The provider's delta state only returns *changed* IDs since the cursor; a row that already passed the cursor stays bodyless until the provider next mutates the message. Per-row flag-clearing is a no-op for the next sync.
- *Error swallowing.* Today's persistence (`crates/sync/src/persistence.rs`) logs and continues on body/inline/search write errors after the DB transaction commits. A disk-full or corrupted-index event produces a clean `SyncCompleted`, the marker would be cleared, and the next boot's invariant pass would skip. Per-row repair has no signal to act on.

The "nuke history_id" approach sidesteps all three:

- *No schema dependency.* `clear_account_history_id` already exists (`crates/sync/src/pipeline.rs:154`).
- *Forces re-fetch.* Provider-side initial sync re-fetches everything in the cached window, so all four stores get repopulated regardless of which leg was partial.
- *Coarse but correct.* The unit of repair is "one account, full re-fetch on next sync." A user who crashes during a sync of a 5 GB account pays a re-sync of that account; the typical case (small delta) is cheap.

**Scan implementations** (still cheap; redundant with the cursor-clear but defense-in-depth):

- **Tantivy store→DB orphans**: iterate documents via `SearchReadState`'s searcher filtered by `account_id`. For each `message_id`, batch-check existence via `SELECT id FROM messages WHERE id IN (?, ?, ...)` (chunks of 500). Delete orphans via `SearchWriteHandle::delete_messages_batch` + `flush_now`.
- **Body store→DB orphans**: `SELECT message_id FROM bodies` (no `account_id` column today; the scan walks all bodies but filters in-memory by joining to `messages.account_id` for the dirty-account match). Same batch-existence check. Delete via `BodyStoreWriteState::delete_batch`.
- **Inline store→DB orphans**: same pattern keyed on `message_id`; the inline store has no `account_id` either, so the join-to-`messages` filter applies.

All scans are idempotent. Cost on a 1M-message mailbox where one account is dirty: ~10 s for the three orphan scans (full body+inline+tantivy walk filtered to one account), plus the cursor-clear (microseconds). Phase 8 refines with bounded re-scan windows + per-store dirty markers.

### Sentinel file lifecycle

```rust
// crates/service/src/lifecycle.rs - existing
pub fn write_clean_shutdown_sentinel(app_data_dir: &Path) -> Result<(), io::Error> { /* ... */ }

// new
pub fn consume_clean_shutdown_sentinel(app_data_dir: &Path) -> bool {
    let path = app_data_dir.join("clean_shutdown");
    match fs::remove_file(&path) {
        Ok(()) => true,
        Err(e) if e.kind() == io::ErrorKind::NotFound => false,
        Err(e) => {
            log::warn!("Failed to remove clean_shutdown sentinel: {e}");
            false  // Treat as "shutdown was unclean"; safer to scan.
        }
    }
}
```

**Boot flow.** Open all four writers (DB, body, inline, search-writer-task); *consume* the sentinel; if consume returned `false`, list `sync_markers/` and build the `DirtyAccount` set from markers whose `status` is *not* `completed` (i.e., `in_progress`, `cancelled`, or `failed`). Run the invariant pass against that set. Signal `boot.ready`.

**Drain flow** (corrected for H4 - first revision flushed before runners stopped):

1. **Cancel + await runners first.** `SyncRuntime::shutdown()` flips every `CancellationToken` and awaits every supervisor `JoinHandle`. After this, no more `Index` / `Delete` commands will arrive at the search writer.
2. **`FlushNow` to writer task.** The writer commits any docs queued by the runners during their cancellation drain.
3. **Drop `SearchWriteHandle`s.** The mpsc sender count drops to zero; the writer task's `recv()` returns `None`; it commits any straggler docs (defense-in-depth) and exits.
4. **Await writer task `JoinHandle`.**
5. **Unlink `sync_markers/` files where `status == completed`.** `failed` / `cancelled` / `in_progress` markers survive into the next boot's invariant pass. (This is the H10 fix: first revision unlinked the entire directory, erasing repair signal for the failed-then-clean-shutdown case.)
6. **Write `<app_data>/clean_shutdown` sentinel.** Very last step; fsync; close.

**Consume-on-boot pattern.** Consuming the sentinel before the invariant-pass decision is deliberate: a SIGKILL between the check and the deletion would leave a stale sentinel. Consuming first means the next boot cannot mistake a leftover sentinel for "yes, last shutdown was clean."

**Marker file write atomicity.** Markers are written via temp-file-then-rename: `<app_data>/sync_markers/<account_id>.json.tmp` is fully written + fsync'd, then renamed atomically to `<account_id>.json`. Status updates also go through temp-rename. On Windows, the rename uses `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` which is atomic on NTFS. **Boot-side parse failure rule:** any marker file that fails to deserialize is treated as "fully dirty" - the account's `history_id` is cleared and the orphan-drop scans run. Conservative; corrupt markers are vanishingly rare and the cost of being wrong (re-syncing one account) is small.

**Marker status states.**
| Status | Meaning | Cleared at drain? |
|--------|---------|-------------------|
| `in_progress` | Runner is actively syncing | No (runner died before update) |
| `completed` | Sync finished successfully | Yes |
| `cancelled` | Sync responded to cancellation | No (defense; cancel cuts short, repair anyway) |
| `failed` | Runner returned `Err` | No |
| (unparseable) | Torn write or stale format | No (treated as fully-dirty) |

The `cancelled` case is debatable - cancellation is a known-good outcome where partial writes are bounded. But cheap to repair and harder to reason about edge cases (e.g., cancellation-during-persist), so the simple rule is "only `completed` clears."

**`started_at` clock-skew sidestep.** First revision's marker carried `started_at: i64` (wall-clock unix-millis) and the invariant pass scanned `messages WHERE updated_at >= started_at`. Both reviewers flagged: `messages` has no `updated_at`; the predicate was unimplementable. Phase 3's repair design (nuke `history_id` for non-`completed` markers; let next sync re-fetch) doesn't depend on a per-row predicate, so `started_at` is no longer load-bearing. Kept in the marker file for human-readable diagnostics only.

### UI sync dispatch rewiring

```rust
// crates/app/src/handlers/provider.rs
pub(crate) fn dispatch_sync_delta(&mut self, account_id: String) -> Task<Message> {
    let aid = account_id.clone();
    let client = self.service_client.clone();
    Task::perform(
        async move {
            client.start_sync(aid.clone()).await
        },
        move |result| Message::SyncComplete(account_id.clone(), result.map_err(|e| e.to_string()).map(|_| ())),
    )
}
```

The `Task::perform` is still in iced - we're not introducing a new subscription; the `client.start_sync` future resolves naturally and the iced runtime maps it to the `Message::SyncComplete` variant the UI already handles. The notification-driven correlation lives inside `client.start_sync`, keyed on `SyncRunId` so multiple waiters per run resolve cleanly:

```rust
// service-api
pub struct SyncStartAck {
    pub account_id: String,
    pub run_id: SyncRunId,
    pub already_in_flight: bool,
}

pub struct SyncCompleted {
    pub account_id: String,
    pub run_id: SyncRunId,
    pub result: SyncResult,
    pub generation: u32,
}

// crates/app/src/service_client.rs
pub struct ServiceClient {
    pending_syncs: Mutex<HashMap<SyncRunId, PendingSync>>,
    // ...
}

enum PendingSync {
    Pending(broadcast::Sender<SyncResult>),
    /// SyncCompleted arrived before any caller subscribed. The result
    /// is latched here until the late subscriber arrives or the entry
    /// is GC'd. Closes the subscribe-after-completion race.
    Completed(SyncResult),
}

impl ServiceClient {
    pub async fn start_sync(&self, account_id: String) -> Result<SyncResult, ClientError> {
        let ack = self.request(RequestParams::SyncStartAccount { account_id }).await?;
        let mut subs = self.pending_syncs.lock().await;
        match subs.entry(ack.run_id) {
            Entry::Occupied(mut e) => match e.get() {
                PendingSync::Completed(r) => {
                    let result = r.clone();
                    e.remove();  // consumed
                    Ok(result)
                }
                PendingSync::Pending(tx) => {
                    let mut rx = tx.subscribe();
                    drop(subs);
                    rx.recv().await.map_err(|_| ClientError::ServiceCrashed)
                }
            },
            Entry::Vacant(v) => {
                let (tx, mut rx) = broadcast::channel(8);
                v.insert(PendingSync::Pending(tx));
                drop(subs);
                rx.recv().await.map_err(|_| ClientError::ServiceCrashed)
            }
        }
    }

    pub async fn cancel_and_await(&self, account_id: &str) -> Result<SyncResult, ClientError> {
        let ack = self.request(RequestParams::SyncCancelAccount { account_id: account_id.into() }).await?;
        let Some(run_id) = ack.run_id else {
            return Ok(SyncResult::Completed);  // No active run; nothing to await.
        };
        // Same shape as start_sync: check for latched Completed first, otherwise subscribe.
        let mut subs = self.pending_syncs.lock().await;
        match subs.entry(run_id) {
            Entry::Occupied(mut e) => match e.get() {
                PendingSync::Completed(r) => { let result = r.clone(); e.remove(); Ok(result) }
                PendingSync::Pending(tx) => {
                    let mut rx = tx.subscribe();
                    drop(subs);
                    rx.recv().await.map_err(|_| ClientError::ServiceCrashed)
                }
            },
            Entry::Vacant(v) => {
                let (tx, mut rx) = broadcast::channel(8);
                v.insert(PendingSync::Pending(tx));
                drop(subs);
                rx.recv().await.map_err(|_| ClientError::ServiceCrashed)
            }
        }
    }
}

// On notification arrival (reader task; fully async context):
async fn route_sync_completed(&self, completed: SyncCompleted) {
    let mut subs = self.pending_syncs.lock().await;  // awaited; never panics on contention
    match subs.entry(completed.run_id) {
        Entry::Occupied(mut e) => match e.get_mut() {
            PendingSync::Pending(tx) => {
                let _ = tx.send(completed.result);
                e.remove();
            }
            PendingSync::Completed(_) => {
                log::warn!("duplicate SyncCompleted for run_id {:?}; dropping", completed.run_id);
            }
        },
        Entry::Vacant(v) => {
            v.insert(PendingSync::Completed(completed.result));
            // Late subscribers will find this and consume immediately.
        }
    }
}
```

**Subscribe-after-completion race closed.** First revision had a window between request-ack-returned and subscriber-inserted in which a fast `SyncCompleted` could arrive, find no entry, and be dropped on the floor. The `PendingSync::Completed` variant latches the result for the late subscriber. A subscriber that arrives finds either a live `Pending` channel or a latched `Completed` value and consumes the entry.

**Latched-Completed GC.** A `Completed` entry that no subscriber ever consumes (rare; would require the caller to drop the start_sync future before the post-ack lock acquire) sits in the map. A periodic sweep in the reader task drops `Completed` entries older than 30 s (a sub-second race window means 30 s is a wide safety margin).

Cross-respawn handling: on `ClientError::ServiceCrashed`, every entry in `pending_syncs` is drained; `Pending` senders are dropped (subscribers get `RecvError::Closed` -> `Err(ServiceCrashed)`); `Completed` entries discarded. The respawned Service starts fresh; the UI's next `SyncTick` re-issues `start_sync` against the new incarnation, generating a fresh `run_id`.

### Account-deletion cancellation

```rust
// crates/app/src/handlers/core.rs (or wherever delete_account lives)
async fn delete_account_with_cancel(
    client: &ServiceClient,
    db: &ReadDbState,
    account_id: String,
) -> Result<(), String> {
    // UI affordance: hide the account row immediately; destructive delete waits for cancel.
    set_account_is_deleting(db, &account_id, true).await?;

    // Issue cancel; await the active run's broadcast resolution (SyncResult::Cancelled normally).
    match client.cancel_and_await(&account_id).await {
        Ok(SyncResult::Cancelled)
        | Ok(SyncResult::Completed)
        | Ok(SyncResult::Failed(_)) => {}                   // sync provably done; proceed
        Err(ClientError::ServiceCrashed) => {}              // sync is dead with the Service; proceed
        Err(other) => {
            // Non-crash IPC error during cancel; surface to user, do not delete.
            set_account_is_deleting(db, &account_id, false).await?;
            return Err(format!("cancel_sync failed: {other}"));
        }
    }

    // Now safe to delete the account row.
    db.with_conn(move |c| crates_db_helpers::delete_account(c, &account_id)).await
}
```

The order matters: cancel before delete, await the completion notification before the delete, otherwise a sync still in `persist_messages_batch` could write into the deleted account's row. The `is_deleting` flag (UI-side affordance) hides the row from the sidebar immediately so the user perceives the click as taking effect; the actual destructive delete is the slow-path step. On `ClientError::ServiceCrashed` the delete still proceeds because a dead Service provably has no in-flight sync writing to the account.

**Both ends gate on `is_deleting`.** Reviewer flagged a race: between `is_deleting = true` and the row actually being deleted, the UI's next `SyncTick` is free to call `client.start_sync(account_id)` against the row about to disappear. Two gates close this:

1. **UI-side**: `SyncTick`'s account list filter excludes `is_deleting = 1` rows. The sidebar's `accounts` list (the source of truth for which accounts get tick-driven sync kicks) skips them, so the tick never issues `start_sync` for an account mid-delete.
2. **Service-side**: `SyncRuntime::start_account` checks `accounts.is_deleting` before spawning a runner. If set, returns a `SyncStartAck` with `result_hint: AccountDeleting` (a new ack variant; UI logs and discards). Defense-in-depth - a UI bug that bypasses the sidebar filter can't actually start a sync.

The `is_deleting` column lives in `accounts` (verify schema; if reusing `is_active` is cleaner, do that instead). Open question 8 covers the schema decision.

### JMAP push transitional state

```
WebSocket (UI) -> JmapPushReceiver channel (UI)
              -> jmap_push_subscription (UI, iced subscription)
              -> Message::JmapPushKick(account_id) (UI)
              -> handler: client.start_sync(account_id) (UI -> Service IPC)
              -> sync runs in Service
              -> sync.completed notification (Service -> UI)
              -> Message::SyncComplete (UI)
```

The hop count is the cost of keeping the WebSocket UI-side for one phase. Phase 4 collapses it: WebSocket moves into the Service, dispatches sync directly in-process, and only `sync.completed` crosses the IPC boundary.

Documented in this plan and in `problem-statement.md` under "transitional state." Removed in Phase 4.

### Action latency under sync writer pressure

Today's `action.execute_plan` worker writes to DB + body store + inline image store + Tantivy. After Phase 3, sync's writer halves contend with the action worker for the same SQLite WAL writer lock. SQLite serialises writers naturally; the action's `BEGIN IMMEDIATE` blocks until the sync's transaction commits.

Concretely: a heavy sync writing 100 messages/sec means ~10 ms per batch transaction. An action arriving during the batch waits 5-10 ms before its `BEGIN IMMEDIATE` succeeds. This is small but visible in the p99 latency budget (5-15 ms target from Phase 2).

Mitigation options:
- **Accept it**: 5-15 ms p99 with sync running is still acceptable for a star-toggle. Manual matrix verifies during Phase 3 close-out.
- **Sync write batching**: lower contention by batching DB writes within sync more aggressively (50→200 messages per transaction). Trades sync throughput for action latency. Out of scope unless manual-matrix p99 regresses past 50 ms.

Recommended: accept it for Phase 3, add a manual-matrix item to monitor.

## Detailed task list

In recommended commit order. Each item is one focused commit unless noted.

1. **`service-api` sync wire types.** New `crates/service-api/src/sync.rs` with `SyncRunId(uuid::Uuid)`, `SyncStartAccountParams`, `SyncStartAck { account_id, run_id, already_in_flight }`, `SyncCancelAccountParams`, `SyncCancelAck { account_id, run_id: Option<SyncRunId>, was_in_flight }`, `SyncResult { Completed | Cancelled | Failed(String) }`, `SyncCompleted { account_id, run_id, result, generation }`, `IndexCommitted { generation }`. Serde derives + serde round-trip tests. `WithGeneration` impls on `SyncCompleted` and `IndexCommitted`. `Notification::SyncCompleted` and `Notification::IndexCommitted` variants added; `Notification::class()` arms return `MustDeliver` for both; **explicit catalog test cases** (the existing catalog enumerates manually; we add `SyncCompleted` and `IndexCommitted` lines, not relying on auto-coverage); `service_generation` / `set_service_generation` arms added exhaustively. `RequestParams::SyncStartAccount` (5 s timeout), `RequestParams::SyncCancelAccount` (5 s timeout). Type-only commit.

2. **`BodyStoreState` -> `BodyStoreReadState` + `BodyStoreWriteState`.** Rename `BodyStoreState` to `BodyStoreReadState` in `crates/stores/src/body_store.rs`; remove `put_batch` from the read state. New `crates/service-state/src/body_store_write.rs::BodyStoreWriteState` with `init` + `put_batch` only. Update `crates/sync/src/persistence.rs::store_message_bodies` to take `&BodyStoreWriteState`. Cargo check fails at every UI call site that constructs `BodyStoreState` for write purposes; the next commit fixes them.

3. **`InlineImageStoreState` -> read/write split.** Same pattern. Move `put_batch` to `service-state::InlineImageStoreWriteState`. Update `crates/sync/src/persistence.rs::store_inline_images`.

4. **`SearchState` -> `SearchReadState` (UI) + `SearchWriteHandle` + writer task (Service).** Rename `SearchState` to `SearchReadState` in `crates/search/src/lib.rs`; keep only `reader`, `fields`, `search_with_filters`, `reload`. Add `message_indexed(message_id) -> Result<bool, String>` helper for the invariant pass. New `crates/service-state/src/search_write.rs::SearchWriteHandle { tx: mpsc::Sender<WriterCommand> }` with public methods that forward via mpsc + oneshot ack (capacity 256): `index_messages_batch`, `delete_messages_batch`, `delete_message`, `clear_index`, `flush_now`. New `crates/service/src/search_writer.rs::spawn_search_writer(app_data_dir, notification_tx, generation) -> Result<SearchWriteHandle, String>` (constructor; asserts multi-thread runtime; spawns `run_writer_task`) + `run_writer_task` (`tokio::spawn` runner; `tokio::task::block_in_place` for sync `IndexWriter` ops; `notification_tx.send_timeout(30s, IndexCommitted)` for the post-commit notification; commit cadence: 1000-doc threshold + 2000ms threshold + `FlushNow`). **No `Arc<Mutex<IndexWriter>>` in any public type, no `on_commit` callback, no `block_on` from inside the writer.**

5. **`common::types`: `SyncProviderCtx` shape.** Add `SyncProviderCtx<'a>` carrying writer-half references + `cancellation_token: &'a CancellationToken`. Phase 2's `SyncProviderCtx` scaffold (currently unused per Phase 2 architecture deltas) is replaced with this. The existing `ProviderCtx` (sync method signatures still take it through Phase 2) gets retired; sync methods now take `SyncProviderCtx`. Update `ProviderOps::sync_delta` (and any sync-side methods) accordingly. Action methods stay on `ActionProviderCtx`.

6. **Cancellation token propagation through JMAP sync (expanded coverage).** Add `cancellation_token: CancellationToken` to `crates/jmap/src/sync/mod.rs::SyncCtx<'_>`. Insert checkpoint calls at the sync-entry, network-call, and persist-batch sites enumerated in Architecture § "Cancellation propagation" (covers shared accounts, contacts, calendar, token refresh, principal resolution, share-notification polling - not just per-mailbox/per-batch). Use `tokio::select! { biased; _ = ctx.cancellation_token.cancelled() => ... }` around long-running awaited calls so cancellation can interrupt mid-await, not just at poll points. Add a `SyncError::Cancelled` variant.

7. **`crates/core/src/sync_dispatch.rs`: signature change.** `sync_delta_for_account` now takes `SyncProviderCtx`-shaped args (writer-half references + cancellation token) instead of unified store states. Update `crates/sync/src/persistence.rs` callers similarly.

8. **`crates/service/src/sync.rs`: `SyncRuntime` + sync markers + panic supervisor.** New file. Holds the per-account map (`HashMap<String, AccountEntry>` keyed by `account_id`, value carries `run_id` + cancellation token + supervisor `JoinHandle`), writer halves, encryption key, progress reporter, notification sender. `start_account(account_id) -> SyncStartAck` checks `accounts.is_deleting` (early exit if set), writes `<app_data>/sync_markers/<account_id>.json` with `status: "in_progress"` via temp-rename *before* spawning the runner; spawns the runner via the panic-supervisor pattern (see Architecture § "Service-side sync handler shape"). `cancel_account(account_id) -> SyncCancelAck` returns the active `run_id` so the UI's `cancel_and_await` can subscribe.

9. **`crates/service/src/handlers/sync.rs`: handler.** Wires `RequestParams::SyncStartAccount` and `RequestParams::SyncCancelAccount` into `SyncRuntime::start_account` / `cancel_account`. Returns the appropriate ack.

10. **`crates/service/src/progress.rs`: `ServiceProgressReporter`.** Implements `db::ProgressReporter`. `emit_json("jmap-sync-progress", value)` reshapes the JSON into `Notification::SyncProgress` and enqueues via the class-aware path. Other event names log a debug warning and drop (catches typos before they go silent).

11. **`crates/service/src/startup_invariants.rs`: invariant pass via clear-history-id + orphan drops.** `run_invariant_pass(db, body_write, inline_write, search_write, search_read, dirty_accounts)`. Reads `sync_markers/` and builds the dirty-account list from markers whose `status != completed`. Per-account: (a) `clear_account_history_id` to force re-fetch on next sync; (b) Tantivy / body / inline orphan-drop scans scoped to `account_id`; (c) unlink the marker. **Drops the per-row DB→store gap scan from the first revision** - the schema additions it needed don't exist; clearing flags doesn't make JMAP delta re-fetch anyway. Clear-history-id is coarser but provably correct.

12. **`crates/service/src/boot.rs`: phase additions + marker-driven invariant pass gating.** Add `OpeningBodyAndInlineStores`, `OpeningSearchIndex`, `RunningInvariantPass` to the boot sequence. Construct writer halves; spawn the search writer task (and pass back the handle); emit `BootProgress` per phase. Consume the clean-shutdown sentinel; if absent, list `sync_markers/` to build the dirty-account list and run the invariant pass against just that set; signal `boot.ready`. **Boot-side parse-failure rule:** any marker that fails to deserialize is treated as fully-dirty (its `account_id` is added to the dirty list with synthetic `MarkerStatus::Unparseable`).

13. **`crates/service/src/lifecycle.rs`: clean-shutdown sentinel + marker cleanup (corrected drain order).** Drain order: (1) `SyncRuntime::shutdown()` - cancel + await every runner's supervisor `JoinHandle`; (2) `SearchWriteHandle::flush_now()`; (3) drop `SearchWriteHandle`s; (4) await search writer task `JoinHandle`; (5) unlink only `sync_markers/<id>.json` files where `status == completed` (failed/cancelled/in_progress survive); (6) write `<app_data>/clean_shutdown` sentinel (very last step; fsync; close). Boot path: `consume_clean_shutdown_sentinel(app_data_dir)` returns `bool`.

14. **`crates/app/src/service_client.rs`: `start_sync`, `cancel_and_await` methods with latched-completed map.** `pending_syncs: Mutex<HashMap<SyncRunId, PendingSync>>` where `PendingSync` is `Pending(broadcast::Sender<SyncResult>) | Completed(SyncResult)` - the `Completed` variant latches a result that arrives before any caller subscribes (closes the subscribe-after-completion race). Reader-task `route_sync_completed` uses `pending_syncs.lock().await` (not `now_or_never`); routes by `run_id`; latches if no subscriber yet. Periodic sweep drops `Completed` entries older than 30 s. Cross-respawn drain: on `ServiceCrashed`, drain the map; `Pending` senders dropped (subscribers receive `RecvError::Closed` -> `Err(ServiceCrashed)`); `Completed` discarded.

15. **`crates/app/src/handlers/provider.rs`: rewire `dispatch_sync_delta`.** Becomes a thin `Task::perform(client.start_sync(account_id))` wrapper. Drop `App.sync_handles: HashMap<_, iced::task::Handle>`. The "already in flight" guard is now Service-side (`already_in_flight: true` in `SyncStartAck`).

16. **`crates/app/src/handlers/core.rs`: account-deletion via `cancel_and_await` (both-ends gated).** `handle_delete_account` flips `is_deleting = true` on the sidebar account immediately (UI-perceived instant); calls `client.cancel_and_await(account_id).await`; on success or `Err(ServiceCrashed)` proceeds to delete the row; on other IPC errors, surfaces to the user and clears `is_deleting`. **UI-side gate**: `SyncTick`'s account-list filter excludes `is_deleting = 1` rows, so the next tick does not kick a sync against the disappearing account. **Service-side gate**: `SyncRuntime::start_account` checks `accounts.is_deleting` and returns an early-out ack if set (defense-in-depth). Schema: adds an `accounts.is_deleting` column (or repurposes `is_active` if its semantics fit - verify against `crates/db/src/db/schema/01_core.sql:13`). See open question 8.

17. **`crates/app/src/...`: `index.committed` reader reload.** Add `pending_reader_reload: Option<Instant>` to `ReadyApp`. Subscription tick at 200 ms; on tick, reload if pending. Notification handler sets the timestamp.

18. **`crates/app/src/...`: replace push-event mapping.** Today's `subscription.rs:50` maps `JmapPushReceiver` events to `Message::SyncComplete` directly. Phase 3 replaces with `Message::JmapPushKick(account_id)` -> handler calls `client.start_sync(account_id)` and discards the future. Module comment notes the round-trip and TODO for Phase 4.

19. **`crates/app/src/app.rs`: wire `SearchReadState`.** Replace `Option<Arc<SearchState>>` with `Option<Arc<SearchReadState>>`. `SearchReadState::open(data_dir)` runs after the boot handshake completes; the Service's `OpeningSearchIndex` phase guarantees the directory exists. Drop the `search_state` field from `ActionContext` (Phase 2 already broke the action-side dependency on search; this commit makes the `app` crate stop constructing the writer half at all).

20. **Test cohort.** The Phase 3 unit + integration + real-subprocess tests below. Lands incrementally with the commits above where natural; this task is the close-out commit that catches anything missed.

21. **Doc updates.** Update `problem-statement.md`'s "Phase 3 status (as landed)" section. Update `implementation-roadmap.md`'s Phase 3 entry to reflect any settled deltas. Bundle with the close-out implementation commit per CLAUDE.md's "no markdown-only commits" rule.

## File-by-file changes

**New files:**
- `crates/service-api/src/sync.rs` - wire types: `SyncRunId`, `SyncStartAccountParams`, `SyncStartAck`, `SyncCancelAccountParams`, `SyncCancelAck`, `SyncResult { Completed | Cancelled | Failed(String) }`, `SyncCompleted`, `IndexCommitted`. (No `ServiceCrashed` / `AlreadyInFlight` variants on the wire - those are `ClientError` / ack-flag conditions.)
- `crates/service-state/src/body_store_write.rs` - `BodyStoreWriteState`.
- `crates/service-state/src/inline_image_store_write.rs` - `InlineImageStoreWriteState`.
- `crates/service-state/src/search_write.rs` - `SearchWriteHandle` (mpsc sender) + `WriterCommand` enum. **No `spawn` function here** - that lives in `service`.
- `crates/service/src/search_writer.rs` - `spawn_search_writer` (constructor) + `run_writer_task` (the runner; the actual `IndexWriter` lives here, never escapes); commit cadence (size + time triggers + `FlushNow`); emits `Notification::IndexCommitted` via `notification_tx.send_timeout(30s, ...)`. Multi-thread runtime asserted at construction.
- `crates/service/src/sync.rs` - `SyncRuntime`, per-account map, runner spawn with panic supervisor, sync-marker file lifecycle (write at start with `status: in_progress`, status-update on exit, atomic temp-file-then-rename).
- `crates/service/src/handlers/sync.rs` - `handle_start_account`, `handle_cancel_account`. Both check `accounts.is_deleting` before doing work.
- `crates/service/src/startup_invariants.rs` - dirty-account discovery (parse `sync_markers/`); per-dirty-account: clear `history_id` + run three orphan-drop scans (Tantivy / body / inline) scoped to that account; unlink the marker after repair.

**Modified files:**
- `crates/service-state/src/lib.rs` - re-export the new write halves.
- `crates/service-api/src/notification.rs` - add `SyncCompleted`, `IndexCommitted` variants; `WithGeneration` impls; `Notification::class()`, `service_generation()`, `set_service_generation()` arms.
- `crates/service-api/src/request.rs` - add `SyncStartAccount`, `SyncCancelAccount` variants and timeouts.
- `crates/service-api/src/lib.rs` - re-export `sync` types.
- `crates/stores/src/body_store.rs` - rename `BodyStoreState` to `BodyStoreReadState`; remove `put_batch`.
- `crates/stores/src/inline_image_store.rs` - rename + remove `put_batch`.
- `crates/search/src/lib.rs` - rename `SearchState` to `SearchReadState`; remove writer methods (move into the writer task in `crates/service/src/search_writer.rs`; the public handle is `service-state::SearchWriteHandle`); `reader.reload()` stays; add `message_indexed(message_id) -> Result<bool>` helper for the invariant pass.
- `crates/sync/src/persistence.rs` - `store_message_bodies`, `store_inline_images`, `index_search_documents` take writer-half references.
- `crates/common/src/types.rs` - replace the unused `SyncProviderCtx` scaffold with the real shape; remove the unified `ProviderCtx` from sync paths.
- `crates/jmap/src/sync/mod.rs` - `SyncCtx` adds `cancellation_token: CancellationToken`; insert checkpoints; add `SyncError::Cancelled`.
- `crates/jmap/src/sync/storage.rs` - per-batch checkpoint inside the persist loop.
- `crates/core/src/sync_dispatch.rs` - signature change: `SyncProviderCtx`-shaped args + cancellation token.
- `crates/service/src/lib.rs` - construct `SyncRuntime` in boot; wire it through to handlers; consume sentinel; run invariant pass.
- `crates/service/src/boot.rs` - add `OpeningBodyAndInlineStores`, `OpeningSearchIndex`, `RunningInvariantPass` phases.
- `crates/service/src/boot_progress.rs` - extend `BootPhaseKind` (or whatever the phase enum is named today).
- `crates/service/src/lifecycle.rs` - sentinel consume helper.
- `crates/service/src/dispatch.rs` - register sync handlers.
- `crates/service/src/handlers/mod.rs` - export the sync handler module.
- `crates/service/src/progress.rs` - `ServiceProgressReporter` impl.
- `crates/service/src/actions/worker.rs` - the worker's existing inline `BodyStoreState::init` / `InlineImageStoreState::init` / `SearchState::init` calls (`worker.rs:427-432`) become construction of the *write* halves; consume from the `SyncRuntime`-shared writers (single instance per Service incarnation).
- `crates/app/src/service_client.rs` - `start_sync`, `cancel_sync` methods; `pending_syncs` map; reader-task routing.
- `crates/app/src/handlers/provider.rs` - rewrite `dispatch_sync_delta`; rewrite `Message::JmapPushKick` arm; drop `sync_handles`.
- `crates/app/src/handlers/core.rs` (or wherever account delete lives) - `delete_account_with_cancel` flow.
- `crates/app/src/app.rs` - `search_state: Option<Arc<SearchReadState>>`; reader reload state; `body_store: Option<BodyStoreReadState>`; `inline_image_store: Option<InlineImageStoreReadState>`; drop `sync_handles`; drop `search_state` from `ActionContext` (already partially gone in Phase 2).
- `crates/app/src/update.rs` - new message arms for `Notification::SyncCompleted`, `Notification::IndexCommitted`; `ReaderReloadTick`.
- `crates/app/src/message.rs` - new variants.
- `crates/app/src/subscription.rs` - 200 ms tick subscription for reader reload.
- `crates/app/src/db/threads.rs` - `init_body_store` returns `BodyStoreReadState`; signature change.
- `Cargo.toml` (workspace) - `tokio-util` already present? If not, add for `CancellationToken`.

**No deletions** - the rename pattern means existing files survive; nothing is removed.

## Test plan

### Unit tests

- `service-api`: serde round-trip for `SyncRunId`, `SyncStartAck`, `SyncCancelAck`, `SyncResult`, `SyncCompleted`, `IndexCommitted`. `RequestParams::SyncStartAccount.timeout()` returns 5 s. **Explicit catalog cases** for `SyncCompleted` and `IndexCommitted` (the existing catalog enumerates manually; auto-coverage was the original plan's mistaken assumption). Static assert that `SyncResult` round-trips through JSON unchanged.
- `service-state`: `BodyStoreWriteState::put_batch` writes bodies; opening a second `BodyStoreReadState` against the same dir reads them back. Same for inline. `SearchWriteHandle::index_messages_batch` queues docs and the oneshot ack fires (the commit may or may not have run yet, depending on cadence triggers - tested separately). `flush_now` forces a commit before returning.
- `service::search_writer`: writer task commits when `COMMIT_DOC_THRESHOLD` exceeded (queue 1001 docs in one batch; assert exactly one commit fires before ack). Writer task commits when `COMMIT_TIME_THRESHOLD` elapsed (queue 50 docs; advance tokio time 2 s; assert commit fires). `FlushNow` forces immediate commit even with 0 pending docs. Writer task on `mpsc rx` close: drains pending, commits, exits cleanly. Each commit emits exactly one `IndexCommitted` notification; `service_generation` matches the writer task's captured generation.
- `service::startup_invariants`: synthesise a marker for one account with `status: in_progress` + orphan body row + orphan tantivy doc + orphan inline image row scoped to the same account; run `run_invariant_pass`; assert `clear_account_history_id` was called for the account; assert all three orphans are dropped; assert `history_ids_cleared == 1`. **Marker survival**: synthesise a marker with `status: failed`; run pass; assert it's processed (history_id cleared, marker unlinked at end). **Unparseable marker**: write garbage to a marker file; assert pass treats it as fully-dirty for that account. Idempotency: run pass twice; second run is a no-op (no markers to process). Empty-marker-set fast path: pass with no dirty accounts is a ~1 ms no-op.
- `jmap::sync`: cancellation token propagation. Construct a fake `SyncCtx` with a cancellation token; cancel before `sync_delta` runs; assert it returns `SyncError::Cancelled` immediately. Cancel mid-batch (token cancellation between batch boundaries); assert the next batch boundary returns `SyncError::Cancelled`.
- `service::sync`: `SyncRuntime::start_account` writes `<app_data>/sync_markers/<account_id>.json` before spawning; runner removes the marker on `Completed` or `Cancelled`. `start_account` for an in-flight account returns `already_in_flight: true` with the existing `run_id`. `cancel_account` returns the active `run_id` so the UI can subscribe.
- `service::progress`: `ServiceProgressReporter::emit_json("jmap-sync-progress", { accountId: "a", phase: "p", current: 1, total: 10 })` enqueues a `Notification::SyncProgress` with the right shape. Unknown event names log debug + drop.
- `app::service_client`: `start_sync` and `cancel_and_await` for the same account subscribe to the same `run_id` broadcast channel; both receive the result on completion. Multiple parallel `start_sync` calls for the same account each receive a `SyncResult` (cloned via broadcast). **Subscribe-after-completion race**: simulate the race - have `route_sync_completed` insert a `PendingSync::Completed` entry; assert a subsequent `start_sync` returns the latched result without parking. **Latched-Completed GC**: insert a `Completed` entry; advance time 31 s; assert the entry is dropped by the periodic sweep. **`now_or_never` removed**: assert `route_sync_completed` uses `lock().await` and does not panic on contention (synthesise contention by issuing `start_sync` while a notification is being routed). Cross-respawn: `ServiceCrashed` drops every broadcast `Sender`; subscribers receive `RecvError::Closed` -> `Err(ServiceCrashed)`.
- `service::sync` panic supervisor: spawn a runner that panics; assert the supervisor emits a synthetic `SyncCompleted { result: Failed("runner panicked: ...") }`; assert any subscribers receive it instead of parking forever. Marker survives in `in_progress` state (not promoted to `failed` because the runner died before updating).
- `service::lifecycle` drain order: spawn a sync runner; trigger drain; assert the order is "cancel runner" -> `JoinHandle.await` -> `FlushNow` -> drop handle -> writer-task `JoinHandle.await` -> selective marker unlink (only `completed`) -> sentinel write. Synthesise a `failed` marker; assert it survives drain. Synthesise a `completed` marker; assert it's unlinked.
- `service::search_writer` notification timeout: stub a notification consumer that never reads; commit and assert the writer task does NOT park on `notification_tx.send` indefinitely (logs warn after 30 s and continues). Test uses `tokio::time::pause()` to advance past the timeout.
- Compile-check: an `app/src/...` source file that tries `use service_state::BodyStoreWriteState` (or `SearchWriteHandle`, `InlineImageStoreWriteState`) fails to build. Document this as the lockdown's enforcement; cite the missing-dep error pattern.

### Integration tests (in-process)

- `crates/service/tests/dispatch_in_process.rs::sync_start_acks_immediately_then_emits_completed` - submit `sync.start_account` against a JMAP test fixture; assert `SyncStartAck { already_in_flight: false }` returns within 5 ms; assert at least one `sync.progress` notification arrives during sync; assert `sync.completed { result: Completed }` arrives last. Existing in-process harness.
- `crates/service/tests/dispatch_in_process.rs::sync_cancel_returns_within_5s` - start sync, immediately cancel; assert `SyncCancelAck { was_in_flight: true }` arrives within milliseconds; assert `sync.completed { result: Cancelled }` arrives within 5 s.
- `crates/service/tests/dispatch_in_process.rs::sync_already_in_flight_returns_same_run_id` - submit two `sync.start_account` calls back-to-back; assert the second returns `already_in_flight: true` with the same `run_id`; assert exactly one `sync.completed` arrives but both UI futures (subscribed to the same `run_id` broadcast) resolve.
- `crates/service/tests/dispatch_in_process.rs::index_committed_emitted_per_writer_commit` - submit a sync that writes 3 batches of 100 messages; with `COMMIT_DOC_THRESHOLD = 1000` (default) the writer task does not commit until the threshold or `FlushNow`; the runner sends `FlushNow` before `sync.completed`; assert exactly one `index.committed` arrives (or one per cadence trigger if multiple fire). Cadence-tuned variant: with threshold lowered to 100 in the test, assert 3 commits.
- `crates/service/tests/dispatch_in_process.rs::sync_progress_coalesces_per_account` - drive a heavy sync emitting 1000 progress events for one account; assert the UI-side queue holds at most one entry for that `CoalesceKey::SyncProgress(account_id)` at any time (queue inspection helper required).
- `crates/service/tests/dispatch_in_process.rs::invariant_pass_clears_history_id_for_dirty_accounts` - spin up Service against a data dir containing: account A with marker `status: in_progress` + account B with marker `status: completed` + account C with no marker + missing sentinel; observe `BootProgress(RunningInvariantPass)`; observe `boot.ready`; verify `history_id` is cleared for A only; verify the next sync against A is initial-style (re-fetches the cached window).
- `crates/service/tests/dispatch_in_process.rs::invariant_pass_drops_orphans_per_account` - spin up against a data dir with synthetic Tantivy / body / inline orphans scoped to account A (marker present); verify orphans are dropped during invariant pass.
- `crates/service/tests/dispatch_in_process.rs::invariant_pass_skipped_when_sentinel_present` - sentinel present + non-`completed` markers present; assert invariant pass does NOT run (they survive into the next dirty boot); assert sentinel is consumed at boot start.
- `crates/service/tests/dispatch_in_process.rs::failed_marker_survives_clean_shutdown` - simulate a runner that exits with `Failed`; clean-shutdown the Service; spawn fresh Service against the same dir; assert the `failed` marker is still present and triggers invariant pass.
- `crates/service/tests/dispatch_in_process.rs::cancel_during_persist_returns_within_one_batch` - start sync that has finished its network fetch and is mid-persist; cancel; assert `sync.completed { Cancelled }` arrives within one batch boundary (~1 s) via the `select!`-based interrupt rather than poll.
- `crates/service/tests/dispatch_in_process.rs::sync_runner_clears_marker_on_completion` - run sync to completion; assert marker status flips from `in_progress` to `completed`; assert drain unlinks it.
- `crates/app/tests/sync_through_ipc.rs::account_deletion_via_cancel_and_await` - start sync, dispatch account-delete; assert `is_deleting = true` flips immediately (UI affordance); assert `cancel_and_await` resolves with `SyncResult::Cancelled`; assert account row is gone after; assert no errors logged about writes against a deleted account.
- `crates/app/tests/sync_through_ipc.rs::account_deletion_proceeds_on_service_crash` - start sync, dispatch account-delete, SIGKILL Service mid-cancel; assert `cancel_and_await` resolves with `Err(ServiceCrashed)`; assert account row is still deleted (the dead Service can't write).
- `crates/app/tests/sync_through_ipc.rs::multiple_waiters_same_run_id_all_resolve` - issue two `start_sync` for the same account in quick succession (second gets `already_in_flight: true`); assert both futures receive a `SyncResult` from the broadcast channel.
- `crates/app/tests/sync_through_ipc.rs::reader_reload_debounces_on_index_committed` - emit 5 `IndexCommitted` notifications within 100 ms; assert reader reload fires exactly once at the next 200 ms tick (not 5 times).

### Real-subprocess smoke tests

- `crates/app/tests/service_subprocess.rs::sync_round_trips_through_subprocess` - spawn against a seeded data dir + JMAP test fixture; submit `sync.start_account`; observe progress + completion notifications across the real pipe; verify thread rows updated.
- `crates/app/tests/service_subprocess.rs::kill_mid_sync_recovers` - start sync that takes >2 s; SIGKILL the subprocess mid-sync; UI respawns; on respawn, observe `BootProgress(RunningInvariantPass)` (sentinel missing); observe `boot.ready`; UI's next `start_sync` resumes from the JMAP delta state. Verify any orphans introduced by the kill are dropped.
- `crates/app/tests/service_subprocess.rs::first_run_search_index_does_not_race` - spawn against a fresh data dir with no `search_index/`; observe `BootProgress(OpeningSearchIndex)`; observe `boot.ready`; UI constructs `SearchReadState::open` against the now-existing index without error.
- `crates/app/tests/service_subprocess.rs::clean_shutdown_writes_sentinel` - graceful shutdown via `client.shutdown()`; verify `<app_data>/clean_shutdown` exists; spawn fresh Service against same dir; assert no `RunningInvariantPass` boot phase emitted.
- `crates/app/tests/service_subprocess.rs::sigkill_omits_sentinel` - SIGKILL the Service; verify sentinel is absent; spawn fresh Service; assert `RunningInvariantPass` boot phase emitted.

### Manual matrix updates

- "Trigger a delta sync of 5,000 new messages on a JMAP account. Observe `sync.progress` cadence in the status bar - smooth, not laggy. Observe UI scroll responsiveness during sync - no jank."
- "Kill the Service mid-sync via `kill <pid>`. Observe respawn within Phase 1.5's bounds. Observe boot progress including `RunningInvariantPass`. Observe sync resumed automatically by the next `SyncTick`."
- "Search for a phrase that was indexed in the last batch of a heavy sync. Result should appear within ~200 ms (debounced reader reload) of the final `index.committed` for that batch."
- "Star-toggle action latency during a heavy sync (manual stopwatch). Should remain <50 ms p99 even with sync writing 100 messages/sec. Compare against Phase 2 baseline (no sync running)."
- "Delete an account whose sync is in flight. Verify cancel ack arrives, sync.completed{Cancelled} observed, account row deleted, no errors logged about writing to a deleted account."
- "JMAP push event arrives during an idle UI. Observe `Message::JmapPushKick` -> `client.start_sync` IPC -> sync runs. (Documented as transitional - Phase 4 collapses this path.)"

## Open questions

Resolve in implementation:

1. **Writer-task commit cadence parameters.** Initial: `COMMIT_DOC_THRESHOLD = 1000`, `COMMIT_TIME_THRESHOLD = 2000 ms`, mpsc capacity 256, broadcast capacity 8. Final calibration via the manual matrix: a 50k-msg cold sync should produce ~50 commits, ~5 reloads/sec UI-side after debouncing. If the manual run shows fsync-bound disk time dominating sync wall clock, push thresholds higher (2000 docs / 5000 ms). If interactive search staleness during sync feels off, push lower.

2. **Sync runner JoinHandle lifecycle.** A finished supervisor leaves a `JoinHandle` in the `AccountSyncMap` until the next `start_account` cleans it up. If the user never re-syncs that account, the handle accumulates. Default proposal: periodic sweep every 60 s. Confirm during commit 8; cheap enough that keeping the handles around indefinitely is acceptable.

3. **Latched-Completed GC interval.** Plan says 30 s. The window for the subscribe-after-completion race is sub-second in practice; 30 s is a wide safety margin. If a future call pattern surfaces longer races (e.g., a UI thread that delays its post-ack subscribe by hundreds of ms), revisit. Confirm during commit 14.

4. **`IndexCommitted` notification class.** Plan currently classifies as `MustDeliver` with a 30 s send-deadline degrade. The principled answer is to add a `BestEffort` (or `Coalesce { key: IndexCommitted }`) class to the notification taxonomy and reclassify there - that puts the policy on the wire-spec rather than the writer task. Revisit during the wire-types commit (1) or in Phase 8 alongside the cross-store invariant pass polish.

5. **`SearchWriteHandle::clear_index` semantics post-relocation.** Today's `clear_index` is callable from any code path. After Phase 3, only Service-side code can reach it. No UI-side caller exists today; surface as an IPC method only when one emerges (Phase 7 reindexing is the likely trigger).

6. **JMAP sync's `SyncCtx` field vs threading.** Default proposal: field on `SyncCtx` (mirrors `progress`). Confirm during commit 6.

7. **`BodyStoreReadState` and `BodyStoreWriteState` opening separate connections vs sharing.** Today the unified `BodyStoreState` opens one pool; UI and Service after Phase 3 will each open their own pool against `bodies.db`. SQLite WAL handles this fine, but it doubles the file descriptor count. Default: separate pools.

8. **`is_deleting` schema column vs reusing `is_active`.** `crates/db/src/db/schema/01_core.sql:13` has `accounts.is_active`; we need a "soft-deleting in-progress" state that hides from UI. Default: new `is_deleting` boolean (clearer intent than re-purposing `is_active`). Schema migration lands with commit 16.

9. **Service-side writer-half sharing across action worker + sync runtime.** The action worker (Phase 2) currently constructs its own writer halves at `worker.rs:427-432`; after Phase 3 those should be the same instances the sync runtime uses. Default proposal: a shared `Arc<BodyStoreWriteState>` + `SearchWriteHandle` constructed in `SyncRuntime` and passed to both. Confirm during commit 8 / 12.

10. **Service-state crate split trigger.** Phase 3 keeps one `service-state` crate. Trigger to split per subsystem: if Phase 6's six additional write halves push `service-state`'s dep surface across rusqlite + tantivy + zstd + provider-specific deps such that incremental compile exceeds ~10 s, split per subsystem. Recheck during the Phase 6 plan.

## Verification (end-to-end)

1. Fresh data dir + seeded JMAP test fixture. Trigger a delta sync from the UI. Both UI and Service processes visible in `ps`. UI is parent, Service is child. Sync runs in the Service; CPU usage shows up in the Service process.
2. Status bar shows sync progress in real time. Heavy sync: progress updates appear smooth (the coalescing collapses chatter into one update per ~100 ms tick).
3. Cancel the sync mid-flight (e.g. by deleting the account). Cancel ack returns within 100 ms; sidebar account row hides immediately (`is_deleting`); `sync.completed{Cancelled}` arrives within 5 s; account row destructive-deleted with no errors.
4. Search for a phrase known to be in a recently-synced message. Result appears within ~250 ms of the writer task's commit (which is itself triggered by the runner's `FlushNow` before `sync.completed`, OR by `COMMIT_DOC_THRESHOLD` / `COMMIT_TIME_THRESHOLD`).
5. Kill the Service via `kill <pid>` mid-sync. UI detects lost heartbeat (Phase 1.5 respawn), respawns the Service. Boot phases visible: existing 1.5 phases, then `OpeningBodyAndInlineStores`, then `OpeningSearchIndex`, then `RunningInvariantPass` (sentinel missing, dirty markers present for the killed sync's account); pass scoped to that account; ~30 s on a 100k-msg account, not 5 minutes; then `boot.ready`. Next `SyncTick` resumes sync from the last checkpoint.
6. Graceful shutdown via UI quit. Service writes `<app_data>/clean_shutdown` after the drain (last step). Next start: phase progression is the same as (5) but `RunningInvariantPass` is *skipped* (sentinel was present + consumed; no markers in `sync_markers/`).
7. Star-toggle action during a heavy sync. Latency stays under ~50 ms p99 (manual stopwatch sanity check). Note: writer-task batching keeps the action's transaction off contention with per-batch sync commits.
8. JMAP push arrives during idle UI. Round-trip: WebSocket -> UI subscription -> `Message::JmapPushKick` -> `client.start_sync` IPC -> sync runs in Service. Documented as transitional; Phase 4 collapses this path.
9. Compile-time check: a UI source file that injects `use service_state::SearchWriteHandle;` (or `BodyStoreWriteState`, `InlineImageStoreWriteState`) fails to build with a missing-dep error.
10. Cursor-clear recovery: pre-seed a sync marker with `status: failed` for an account; spawn Service with sentinel missing; observe `BootProgress(RunningInvariantPass)`; assert `clear_account_history_id` was called for that account; trigger `start_sync`; observe the next sync runs as initial-style (re-fetches the cached window) and repopulates body / inline / search.
11. `brokkr check` clean.

## Promotion criteria

This phase is done when:

- All items in `In scope` are implemented and wired - JMAP sync runs Service-side; the UI no longer constructs the writer task, `BodyStoreState::put_batch`, or `InlineImageStoreState::put_batch`; cancellation flows through the JMAP sync's checkpoints; `index.committed` drives the UI reader reload.
- The `service-state` crate boundary holds: a UI source file that tries `use service_state::SearchWriteHandle` (or the body / inline write halves) fails to build with a missing-dep error.
- `BodyStoreReadState`, `InlineImageStoreReadState`, `SearchReadState` exist; their write methods do not.
- The `tantivy::IndexWriter` lives only inside the Service-side writer task spawned by `spawn_search_writer` in `BootPhase::OpeningSearchIndex`. No `Arc<Mutex<IndexWriter>>` exists in any public type. No `on_commit` callback exists. `index.committed` notifications are emitted from the writer task's async context.
- The cross-store invariant pass runs Service-side, gated on the missing clean-shutdown sentinel + scoped to dirty accounts via `sync_markers/`; covers store→DB orphans (drop) AND DB→store gaps (clear cached-flags).
- The clean-shutdown sentinel is consumed (not just checked) at boot start; written at the end of clean shutdown after marker-directory unlink.
- `App.sync_handles: HashMap<_, iced::task::Handle>` is gone; account-deletion goes through `client.cancel_and_await` and tolerates `Err(ServiceCrashed)` as proceed-with-delete.
- `pending_syncs` is keyed by `SyncRunId` with broadcast channels for multiple waiters per run.
- The JMAP push subscription remains UI-side with `Message::JmapPushKick` -> `client.start_sync` (replaces today's `subscription.rs:50` direct `SyncComplete` mapping); the round-trip is removed in Phase 4.
- Catalog test has explicit `SyncCompleted` and `IndexCommitted` cases (no auto-coverage assumption).
- The Phase 3 test cohort lands: in-process integration tests for sync ack-then-stream + cancel + writer-task cadence + invariant pass with marker gating + DB→store gap repair + multiple waiters per run_id; real-subprocess smoke tests for kill-mid-sync recovery + clean-shutdown sentinel + first-run index init.
- All `Exit criteria` from `implementation-roadmap.md` § Phase 3 are satisfied.
- Reviewer signoff on the revised plan + the delivered code (a follow-up `review arch --session d4b103c9-4133-4425-a4dc-1a53597883f8` against the revisions, and `review bugs --oneshot` for the now-substantial cancellation + crash-path surfaces).
