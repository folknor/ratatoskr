# The Service - Implementation Roadmap

Companion to `problem-statement.md`. Each phase below is intended as a **separate `EnterPlanMode` session** that produces a focused implementation plan, lands as one or a small handful of commits, and unblocks the next phase. Nothing here is a complete plan in itself - the goal is to chart the order of attack and keep us from accidentally building things in the wrong sequence.

This document is a sketch. Phase scope, interfaces, and risks will firm up when each phase enters its own planning session.

**Cross-document dependency.** The attachment work (`docs/attachments/`) consumes this work. Specifically: attachments Phase 2 needs the Service hosting sync (Phase 3 here); attachments Phase 3 needs the `attachment.fetch` IPC method (lands as part of Phase 6 here); attachments Phase 7 (text extraction + indexing) is essentially Phase 7 here.

## How to read this

- **Goal** - one-sentence outcome.
- **Entry criteria** - what must already exist for this phase to start cleanly.
- **In scope / Out of scope** - hard boundaries for the phase.
- **Touchpoints** - files / modules likely to change. Indicative, not exhaustive.
- **Exit criteria** - observable evidence the phase is complete.
- **Risks / open questions** - unknowns to resolve during the planning session.

---

## Phase 1 - Process boundary scaffolding

**Goal.** A second process exists. The UI spawns it at start, exchanges `health.ping` over JSON-RPC stdio, kills it cleanly on shutdown, and detects + handles its disappearance. No real work moves across the boundary yet; this phase is the empty scaffold every later phase plugs into.

**Entry criteria.**
- Problem statement approved.
- Decision pinned on single-binary-multi-mode (one `ratatoskr` binary, `--service` flag selects mode) vs separate binaries. Default proposal: single binary with mode flag.

**In scope.**
- New crate `crates/service-api/` defining `Request`, `Response`, `Notification` enums + framing helpers shared between UI and Service. Phase 1 surface: just `health.ping` and `Shutdown`. `PROTOCOL_VERSION` constant; first ping asserts UI's constant matches Service's response or boot fails.
- New crate `crates/service/` with `run_service()` async entry point + `run_service_with_io()` generic over `AsyncRead`/`AsyncWrite` for testability.
- `crates/app/src/main.rs` dispatches based on `--service` flag.
- New module `crates/app/src/service_client.rs`: spawns subprocess, pipes stdio, manages request/response correlation, dedicated stdout-writer task with bounded queue.
- Per-request timeouts (default 5 s; `Shutdown` gets 30 s); expired requests evict their pending entry.
- Bounded notification channel (cap 1024) with progress-event coalescing for the small set we have (just heartbeat round-trips in Phase 1; framework ready for sync/index in later phases).
- Frame size cap (4 MiB) enforced at the framing layer; oversize frames rejected.
- Panic safety: every handler wrapped in `catch_unwind`; panics return `ServiceError::Panic`, dispatch loop continues.
- File-based logging: Service writes to `<app_data>/logs/service.log` with simple size-based rolling (~10 MB cap, keep 3). stderr stays for `cargo run` debugging.
- Heartbeat: 30 s interval. Logs missed beats only - no respawn here (lands in Phase 1.5).
- Parent-death detection (race-free per platform):
  - Linux: `pre_exec` + `prctl(PR_SET_PDEATHSIG, SIGTERM)` + post-prctl `getppid()` check at startup (closes the "parent died before hook" race).
  - macOS: `kqueue` with `EVFILT_PROC` + `NOTE_EXIT` registered against parent PID at start.
  - Windows: `OpenProcess` against parent PID + `WaitForSingleObject` on the HANDLE.
- Clean shutdown: `Shutdown` is a **request** (not a notification); UI awaits the response with a 30 s timeout, then SIGTERM, then SIGKILL after another 5 s.

**Out of scope.**
- Any actual functionality moving across the boundary.
- Respawn-on-crash (lands in Phase 1.5).
- Tray icon, autostart, daemon promotion.
- Schema versioning of the JSON-RPC protocol (pin format-version-1 in v1; UI/Service shipped as a coupled pair).

**Touchpoints.**
- New crates: `crates/service-api/`, `crates/service/`.
- `crates/app/Cargo.toml` - dep on the two new crates.
- `crates/app/src/main.rs` - mode dispatch.
- `crates/app/src/app.rs` - boot launches `ServiceClient`, stores it on `App`.
- `crates/app/src/service_client.rs` - new module.
- Workspace `Cargo.toml` - register the two new crates.

**Exit criteria.**
- `cargo run -p app` spawns the subprocess; `ps` shows two processes.
- UI logs "Service ready (pid=...)" on start.
- Quitting the UI cleanly exits the Service via the request/ack handshake (no orphan in `ps`).
- SIGKILLing the UI exits the Service within seconds on all three platforms (Linux: PR_SET_PDEATHSIG; macOS: kqueue; Windows: WaitForSingleObject).
- `<app_data>/logs/service.log` exists and contains the boot + heartbeat lines.
- Integration tests cover: happy-path ping, EOF-during-pending-request, malformed JSON, concurrent ping fan-out (id correlation), version mismatch, spawn failure, panicking handler returns `ServiceError::Panic` and Service stays up.

**Risks / open questions.**
- Stdio framing helper LOC: realistic estimate is 200-300 LOC including all the failure-mode handling (parse errors, frame-size rejection, EOF, partial reads, write timeouts, panic catching). The earlier "~50 LOC" was undersold.
- `ResponseResult` is *not* a unified untagged enum. `pending` map holds `oneshot::Sender<Result<serde_json::Value, ServiceError>>`; the typed `request<R, P>()` wrapper deserializes the value into `R` after correlating by id. Avoids the silent-misroute trap of untagged-with-many-variants.
- `ServiceClient::Drop` impl drains the pending map and rejects every outstanding sender so dropped clients don't leave hung waiters.

---

## Phase 1.5 - Minimal respawn + UI degraded state

**Goal.** A Service crash during Phases 2-7 doesn't take the app down. This is *not* the polished crash-recovery story (that's Phase 8) - just enough to keep development livable.

**Entry criteria.**
- Phase 1 landed.

**In scope.**
- If the Service exits unexpectedly: `ServiceClient` spawns a new one. No backoff, no crashloop detection, no UI status indicator yet.
- Pending requests at the time of crash are immediately failed with `ClientError::ServiceCrashed`. UI surfaces this per call site (probably as a "Try again" toast once the toast system lands; until then, log).
- Two real subsystems land here for the same reason: schema migrations relocate to Service boot (so UI's `ReadDbState` construction depends on a Service handshake), and encryption-key load relocates to Service boot (must happen before any token-bearing IPC).
- Spawn-failure policy: spawn failure at boot is **fatal** (UI refuses to boot). Consistent across phases - avoids a "this used to boot without the Service" regression in later phases.

**Out of scope.**
- Backoff, crashloop detection, status indicator (Phase 8).
- In-flight request replay (Phase 8).

**Touchpoints.**
- `crates/service_client.rs` - respawn loop, pending-request cleanup on respawn.
- `crates/service/src/main.rs` (or equivalent boot path) - run schema migrations Service-side; load encryption key.
- `crates/app/src/app.rs` - boot waits for Service `health.ping` response (which now implies "schema migrated, key loaded, ready") before constructing `ReadDbState`.

**Exit criteria.**
- `kill <service-pid>`; UI logs the crash, spawns a new Service, app continues to function.
- Pending requests at crash time fail with a clear error, not a hang.
- `cargo test -p service` includes a respawn integration test.

**Risks / open questions.**
- Migration policy: this phase introduces the first "UI defers until Service ready" coupling. Need to be careful that this doesn't regress UI startup time visibly for the no-pending-migration case (the common one).

---

## Phase 2 - Move the Action service into the Service

**Goal.** The Action service (the existing in-process mutation gate) runs inside the Service. UI dispatches actions via IPC; the Service executes them and reports outcome.

**Goal.** Action *execution* moves into the Service. Resolution + planning + completion-effects stay UI-side. UI sends a resolved plan; Service executes; outcome streams back as notifications. Plus: the type-level write/read split lands here so the invariant becomes mechanical.

**Entry criteria.**
- Phase 1 + 1.5 landed.
- A clear inventory of every Action service entry point in the current code (the planning session enumerates these).

**In scope.**
- **Type-level write/read split.** `DbState` -> `ReadDbState` + `WriteDbState`. Same for `BodyStoreState`, `InlineImageStoreState`, `SearchState`. UI boot constructs read-only halves; only Service binary entry can construct the write halves. This makes the invariant compile-enforced.
- **Action service: execution-only relocates.**
  - UI keeps: `MailActionIntent`, `resolve_intent`, `build_execution_plan`. These read selection state, sidebar scope, completion-behavior policy - all UI-owned. They produce a list of `MailOperation`.
  - Service gets: `batch_execute(plan: Vec<MailOperation>) -> outcomes`, plus `MutationLog` writes.
  - UI keeps: completion-effects (toast, auto-advance, undo eligibility, optimistic thread-list updates) - driven by `action.completed` notifications.
- `service-api` new methods: `action.execute_plan { plan }` -> per-operation `OperationOutcome` notifications, then a final `action.completed { ... }` notification.
- **Pending-ops worker relocates.** The retry queue (`db_pending_ops_*` drainer) runs inside the Service since it dispatches actions and the action execution layer is now Service-side.
- **Progress reporter shim.** `&dyn ProgressReporter` impl in the Service serializes events into IPC notifications; UI's existing `IcedProgressReporter` keeps consuming them as it does today. This is real refactoring work - the relocation is not "no shape change" as previously framed.
- UI's existing call sites currently calling `core::actions::*` directly become: build the plan UI-side, then `service_client.execute_plan(plan)`.
- The existing `ActionContext` (`core::actions::ActionContext`) is reconstructed on the Service side from Service-owned state (`WriteDbState`, encryption key, write halves of stores).

**Out of scope.**
- Sync. Sync still happens in the UI process - it'll move in Phase 3.
- Push. Same.
- Streaming progress for long-running actions (e.g. bulk archive of 500 threads). The notification model supports this naturally; tuning the cadence is a follow-up.

**Touchpoints.**
- `crates/db/src/db/...` - introduce the read/write state-type split.
- `crates/stores/src/...` - same for body / inline-image / search states.
- `crates/service-api/` - `action.execute_plan` method + `OperationOutcome` / `action.completed` notifications.
- `crates/service/src/handlers/action.rs` - new.
- `crates/service/src/pending_ops.rs` - new (retry queue worker).
- `crates/app/src/handlers/commands.rs` - dispatch goes through the service client; planning stays here.
- `crates/app/src/action_resolve.rs` - unchanged in shape; just no longer calls the executor directly.
- `crates/core/src/actions/context.rs` - decouple `ActionContext` from `App` state references.

**Exit criteria.**
- All user-triggered actions (archive, delete, label, snooze, etc.) build the plan UI-side and execute Service-side.
- UI compilation fails if anyone tries to construct a `WriteDbState` outside the Service crate.
- `MutationLog` entries continue to land correctly (logged from the Service side).
- Undo continues to work.
- Pending-ops queue continues to drain (now Service-side).

**Risks / open questions.**
- The interaction with the UI's `nav_generation` / `thread_generation` counters (which gate stale UI updates). Generations stay UI-side; Service notifications carry no generation; UI bumps generations on receiving `action.completed`.
- Error type serialization across the boundary - decide which `ActionError` variants survive intact and which collapse into a generic `RemoteError`.
- The `ActionContext` decoupling from `App` state may force some other extractions in `core::actions::context`. Scope to keep on the radar but not block the phase.

---

## Phase 3 - Move sync into the Service (JMAP first), including Tantivy writer relocation

**Goal.** JMAP delta sync runs inside the Service, including all of its write-side interactions (DB, body store, inline image store, Tantivy writer). UI gets sync progress + completion via notifications. Tantivy reader stays UI-side, driven by `index.committed` notifications.

**Entry criteria.**
- Phase 1 + 1.5 + 2 landed.
- The Action service migration validated the IPC pattern under realistic load.

**In scope.**
- **Tantivy writer relocates here, not in Phase 7.** Sync today indexes via `SearchState`, which always opens an `IndexWriter`. Sync moving Service-side means the writer must come with it - they're entangled. This phase splits `SearchState` into a reader half (UI) and a writer half (Service-internal), and adds the `index.committed { generation }` notification that UI uses to drive `IndexReader::reload()`. Phase 7 then layers attachment text-extraction *on top of* the already-Service-side writer; it does not relocate it.
- `service-api` new methods: `sync.start_account { account_id }`, `sync.cancel_account { account_id }`. New notifications: `sync.progress`, `sync.completed`, `index.committed`.
- Service owns sync dispatch: `sync_delta_for_account` runs Service-side using Service-owned `WriteDbState` / write halves of body store / inline image store / Tantivy writer.
- UI's `dispatch_sync_delta` -> `Task::perform(...)` becomes `service_client.start_sync(account_id)` returning a future that resolves on `sync.completed`.
- `App.sync_handles` (the `iced::task::Handle` map from the recent sync-cancellation work) replaced by Service-side cancellation tokens; UI's cancel call becomes IPC.
- UI search reader subscribes to `index.committed` notifications and calls `reader.reload()` on each.

**Out of scope.**
- Other providers (Phase 5 ports them).
- Push notifications (Phase 4).
- Re-tuning per-account concurrency limit (4) - stays the same.
- New extractors / attachment indexing (Phase 7).

**Touchpoints.**
- `crates/search/src/lib.rs` - split `SearchState` into reader/writer halves; the writer becomes Service-only.
- `crates/service-api/` - sync methods + `index.committed` notification.
- `crates/service/src/handlers/sync.rs` - new.
- `crates/sync/src/persistence.rs` - now writes through Service-owned writer halves.
- `crates/app/src/handlers/provider.rs` - rewire `dispatch_sync_delta` to talk to the Service.
- `crates/app/src/update.rs` - `Message::SyncComplete` arrives via IPC notification rather than `Task::perform` callback.
- `crates/app/src/...` (search reader sites) - one shared reader; reload on `index.committed`.

**Exit criteria.**
- A JMAP sync triggered from the UI runs in the Service process (visible in `top` / `htop`).
- Sync progress events reach the UI status bar in real time.
- Cancel mid-sync works.
- The "abort sync on account deletion" wiring (recent work) continues to function via IPC.
- Search results returned from the UI reader reflect Service-side writes within milliseconds of `index.committed`.
- UI compilation fails if anyone tries to construct a Tantivy `IndexWriter` outside the Service crate.

**Risks / open questions.**
- Tantivy writer lock recovery on uncleanly-killed Service. Tantivy ≥0.21 recovers stale writer locks; verify with a kill-mid-write test in this phase. Document the version bound in `crates/search/Cargo.toml`.
- Currently `Message::SyncComplete` triggers a navigation reload + thread list refresh. That side effect stays UI-side; Service just notifies.

---

## Phase 4 - Move push notifications into the Service

**Goal.** JMAP push receivers run inside the Service. Push events become Service-to-UI notifications that trigger Service-side sync.

**Entry criteria.**
- Phase 3 landed (push triggers sync, which now lives in the Service).

**In scope.**
- JMAP push WebSocket receiver moves into the Service.
- The existing `JmapPushReceiver` channel collapses - the UI no longer subscribes to push directly. Push events arriving at the Service trigger the Service-internal sync path.
- UI gets a `push.event { account_id }` notification for visibility (status bar updates) but the actual response (sync) happens entirely in the Service.

**Out of scope.**
- IMAP IDLE (still pending; comes when IMAP IDLE itself lands in the codebase).
- Cross-platform OS-level notification surfacing (toast on new mail). Separate work.

**Touchpoints.**
- `crates/service/src/push.rs` - new.
- `crates/app/src/handlers/provider.rs` - delete the JMAP push subscription wiring.
- `crates/app/src/subscription.rs` - drop the `jmap_push_subscription` recipe.

**Exit criteria.**
- A change pushed to a JMAP mailbox triggers a sync in the Service without the UI being on the call path.
- Status bar still surfaces "new mail arrived" indicators.

**Risks / open questions.**
- WebSocket lifetime: today the receiver lives as long as the iced subscription. Service-side, it lives as long as the Service. This is strictly more durable - good.

---

## Phase 5 - Port sync to other providers

**Goal.** Gmail, Graph, IMAP sync paths run inside the Service.

**Entry criteria.**
- Phase 3 landed for JMAP. The pattern is proven.

**In scope.**
- Same Service-side hosting pattern applied to `gmail`, `graph`, `imap` provider sync entry points.
- Per-provider concurrency policies preserved (Gmail/Graph: 4 per account; IMAP: 1 per folder via session reuse).

**Out of scope.**
- Provider-specific protocol improvements (CONDSTORE/QRESYNC, batch APIs, etc.) - those are tracked in their own roadmap docs.

**Touchpoints.**
- `crates/service/src/handlers/sync.rs` - dispatch by provider type.
- The four `crates/{gmail,graph,imap,jmap}/src/sync/...` paths - no functional change, just where they're called from.

**Exit criteria.**
- All four provider sync paths run Service-side.
- UI lifecycle has no remaining sync code on the hot path.

**Risks / open questions.**
- IMAP session pooling needs to live Service-side; the existing per-folder-session state moves with sync.

---

## Phase 6 - Settings + remaining UI write surfaces relocate

**Goal.** The "Service is the only writer" invariant is fully realized. Every UI write path enumerated in the problem-statement inventory has moved across the boundary; the type-level read/write split (introduced in Phase 2) catches anything left behind at compile time. Plus: the BlobStore writer relocates and `attachment.fetch` IPC lands.

(Phase 6 used to be just blob writes; the previous split was wrong because sync brings the BlobStore writer with it in Phase 3 anyway. Combining "remaining write surfaces" + the small attachment.fetch IPC into one phase reflects the actual shape of work.)

**Entry criteria.**
- Phase 5 landed (all sync runs Service-side; the BlobStore writer is already relocated as a sync dependency).
- Attachments roadmap Phase 1a + 1b landed.

**In scope.**
- **Remaining write surfaces** from the problem-statement inventory: preferences, account create/update/delete, signature CRUD, local draft auto-save, pinned searches, calendar mutations, OAuth token persist. Each becomes a Service-side handler reachable via IPC.
- **`attachment.fetch` IPC** for cache-miss reads. Returns `{ content_hash, size }` not `Vec<u8>` (per backpressure policy); UI re-reads positionally from the pack file.
- **Eviction policy + GC** from attachments roadmap Phase 6 lands here, since the BlobStore writer is now Service-side.
- **OAuth two-step coordination.** UI captures the redirect (it's the visible app); ships the auth code to Service via IPC; Service exchanges + persists the token.

**Out of scope.**
- Settings UI changes for attachment caching policy (attachments Phase 4, lives UI-side; just makes IPC calls).
- Calendar attachments (separate work).

**Touchpoints.**
- `crates/service-api/` - `attachment.fetch`, `prefs.set`, `account.upsert`, `account.delete`, `signature.upsert`, `signature.delete`, `draft.save`, `pinned_search.upsert`, `pinned_search.delete`, `calendar.mutate`, `oauth.exchange_code`. (Final names settle in planning; this is the surface.)
- `crates/service/src/handlers/{attachment,prefs,account,signature,draft,pinned_search,calendar,oauth}.rs` - new.
- `crates/app/src/handlers/...` - replace direct DB writes with service-client calls. Type system catches anything missed.

**Exit criteria.**
- `git grep` for `with_write_conn` / similar in `crates/app/` returns nothing.
- The `WriteDbState` constructor is unreachable from any UI call site (compile-enforced).
- Cache-miss Open / Save calls succeed via IPC.

**Risks / open questions.**
- Tombstone visibility across processes: Service tombstones a blob, UI tries to read it before the index commit propagates. Service holds the write lock; UI reads see the post-commit state via SQLite WAL - verify with a stress test.
- Concurrent reads of the currently-being-written-to pack must never read past the last fsync'd offset. The pack store API enforces this; verify it survives the IPC boundary.
- OAuth coordination introduces a UI-Service round-trip during the redirect window. Need to verify it doesn't blow timeouts on slow OAuth servers.

---

## Phase 7 - Attachment text extraction + Tantivy indexing

**Goal.** The forcing function. Cached attachments get text-extracted (per mime-type extractors) and indexed into Tantivy. Search results disambiguate "matched in body" vs. "matched in attachment X." Layers on top of the already-Service-side Tantivy writer (relocated in Phase 3).

**Entry criteria.**
- Phase 3 landed (Tantivy writer is Service-side).
- Phase 6 landed (BlobStore writer is Service-side; cached attachments exist).

**In scope.**
- `crates/service/src/text_extract/` - per-mime extractor dispatch. Initial extractors: PDF (Rust crate TBD - `pdf-extract` for v1, with explicit "best effort, skip the weird ones" caveat; `pdfium-render` or `mupdf-rs` evaluated as later upgrades), OOXML (`.docx`/`.xlsx`/`.pptx` - zip + xml text extraction), plain text. Skip lists for opaque binaries (mp4, zip, exe, etc.).
- Pipeline: pre-fetch -> extract -> add to Tantivy doc with `attachment_*` field tags -> commit batched.
- Tantivy schema migration: add `attachment_text`, `attachment_filename`, `attachment_mime` fields.
- Re-index command (`index.rebuild`) for one-shot full re-extraction. Multi-hour acceptable; reports progress via notification.
- Search results carry "match in attachment" annotations.

**Out of scope.**
- OCR for scanned PDFs (substantial separate work).
- Language detection / per-language analyzers (defer until users complain).
- Attachment preview rendering (still out of scope per the attachments problem statement).

**Touchpoints.**
- New: `crates/service/src/text_extract/{mod,pdf,ooxml,plain}.rs`.
- `crates/service/src/handlers/attachment.rs` - extraction triggered after `BlobStore::put`.
- Tantivy schema migration: add `attachment_text` field, optionally `attachment_filename`, `attachment_mime`.
- `crates/search/...` - reader-side adjustments to surface attachment matches.
- `crates/app/src/ui/...` - search result rendering shows the "match in attachment X" annotation.

**Exit criteria.**
- A search query for a phrase known to be inside a cached PDF returns the parent message with an "attachment match" annotation.
- A re-index of an existing mailbox completes successfully (UI must stay open; visible progress).
- Indexing CPU stays Service-side; UI rendering remains responsive during heavy indexing.

**Risks / open questions.**
- PDF extraction crate choice. `pdf-extract` exists but is incomplete; `pdfium-render` requires shipping pdfium binary; `mupdf-rs` ditto. May need to settle for "good for most PDFs, skip the weird ones" in v1.
- Tantivy commit cadence under indexing pressure - too frequent slows things down, too rare loses recent work on crash. Probably commit every N docs or M minutes.
- Indexing memory footprint for very large attachments (extracting a 200 MB PDF). Need a streaming-ish approach or a hard skip threshold.

---

## Phase 8 - Crash recovery polish + cross-store reconciliation

**Goal.** The Service surviving / failing / being respawned is fully handled (Phase 1.5 was the minimal version). UI shows visible state when the Service is restarting; queued work is preserved across a Service crash. Plus: the cross-store invariant pass (orphan reconciliation) lands here.

**Entry criteria.**
- Phases 1-7 landed. Real crashes are happening (or being induced) so we know what hurts.

**In scope.**
- Respawn with exponential backoff (Phase 1.5 was no-backoff).
- Crashloop detection: if respawn fails N times in M seconds, surface a permanent error state in the UI ("Service can't start - check logs").
- UI status indicator for Service health (small banner or status bar element).
- In-flight requests are either (a) replayed if idempotent, (b) failed back to the caller with a clear error if not. Schema decision per-method, recorded in `service-api`.
- Persistence of the retry queue across Service restarts (already on disk in `pending_ops` table; verify).
- **Cross-store invariant pass** at Service startup: every `attachments.content_hash` resolves in the pack store; every Tantivy doc references an existing message; every body-store entry references an existing message. Orphans dropped, logged.

**Out of scope.**
- Hot-restart / live state migration of the Service. Crash + cold restart is the model.

**Touchpoints.**
- `crates/app/src/service_client.rs` - backoff + crashloop detection + status reporting.
- `crates/app/src/ui/status_bar.rs` - new "Service degraded" indicator.
- `crates/service/src/startup_invariants.rs` - new (orphan reconciliation pass).

**Exit criteria.**
- Killing the Service mid-sync results in a respawn within a few seconds (Phase 1.5 already), backoff prevents tight crashloops (new), status indicator surfaces the degraded state (new).
- A persistently failing Service surfaces a clear UI error rather than silent breakage.
- Startup invariant pass runs in <5s on a typical mailbox; logged stats let us see how often crashes leave us reconciling.

**Risks / open questions.**
- Distinguishing "Service crashed" from "Service is just slow under load" in the heartbeat. Generous timeouts; longer for the first heartbeat after a sync starts.
- Invariant pass cost on a 200 GB mailbox - need to bound it (probably "scan only what's been written since last clean shutdown" using a marker file).

---

## Phase 9 (optional) - Tray-resident promotion

**Goal.** Closing the UI window doesn't quit the app or kill the Service. Tray icon offers reopen / quit. Push notifications continue to run when the window is closed.

**Entry criteria.**
- Phases 1-8 landed and running well in real use.
- Demand exists from users for "background sync without keeping a window open."

**In scope.**
- Cross-platform tray icon (probably `tray-icon` crate or iced's tray support if available by then).
- "Close button minimizes to tray" preference (off by default; users opt in).
- Tray menu: Open, Quit, possibly Compose.
- The Service lifecycle stays exactly the same - it's still a child of the UI process. The UI process just doesn't exit when the window closes.

**Out of scope.**
- True system-daemon mode. Still rejected.
- Auto-start at user-session login - separate optional follow-up.
- Native OS notification toasts (e.g. for new mail). Separate work.

**Touchpoints.**
- New: `crates/app/src/tray.rs`.
- `crates/app/src/app.rs` - lifecycle changes around window close.

**Exit criteria.**
- App can be configured to minimize-on-close.
- Push notifications continue with window closed; reopening is fast (Service was already running).

**Risks / open questions.**
- Cross-platform tray APIs are uneven; `tray-icon` is the most established Rust crate but has its quirks.
- Quit-vs-minimize disambiguation is a known UX trap.

---

## Out of phases (deliberately deferred)

- **Full system daemon mode** (systemd unit / launchd / Windows Service). Explicit non-goal.
- **Multi-UI** (multiple windows of the app sharing one Service). Conceivable; not a target.
- **OS notification toasts** for new mail / completed actions. Separate work; depends on platform APIs.
- **Schema versioning of the IPC protocol.** UI and Service ship as a tightly coupled pair. If we ever want to support cross-version, that's its own design exercise.
- **Service-as-library** for embedding in other apps. The Service is a Ratatoskr internal; not a reusable building block.
