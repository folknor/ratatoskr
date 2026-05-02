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
- New crate `crates/service-api/` defining `Request`, `Response`, `Notification` enums shared between UI and Service. Phase 1 surface: just `Ping`/`Pong`.
- New crate `crates/service/` with `run_service()` async entry point: reads JSON-RPC from stdin, writes responses to stdout, handles `health.ping`, exits on `Shutdown` notification.
- `crates/app/src/main.rs` dispatches based on a `--service` flag: with the flag, calls `service::run_service()`; without, boots iced as today.
- New module `crates/app/src/service_client.rs`: spawns the subprocess (`tokio::process::Command::new(env::current_exe()?).arg("--service")`), pipes stdio, manages request/response correlation, background reader for notifications.
- Heartbeat: UI sends `health.ping` every 5 s, logs missed beats. No respawn yet (Phase 8); just visibility.
- Parent-death detection on Linux via `pre_exec` + `prctl(PR_SET_PDEATHSIG, SIGTERM)`. macOS/Windows: poll parent PID every N seconds, exit if it's gone.
- Clean shutdown handshake: UI sends `Shutdown` notification, waits up to 5 s for Service exit, escalates SIGTERM -> SIGKILL.

**Out of scope.**
- Any actual functionality moving across the boundary. Sync, action service, Tantivy writer, blob store writes, etc. all stay in the UI process.
- Respawn-on-crash with backoff (Phase 8).
- Tray icon, autostart, daemon promotion.
- Schema versioning of the JSON-RPC protocol (UI and Service ship as a tightly coupled pair; pin format-version-1 in v1, bump method names if anything changes later).

**Touchpoints.**
- New crates: `crates/service-api/`, `crates/service/`.
- `crates/app/Cargo.toml` - dep on the two new crates.
- `crates/app/src/main.rs` - mode dispatch.
- `crates/app/src/app.rs` - boot launches `ServiceClient`, stores it on `App`.
- `crates/app/src/service_client.rs` - new module.
- Workspace `Cargo.toml` - register the two new crates.

**Exit criteria.**
- `cargo run -p app` spawns the subprocess; `ps` shows two processes (UI + child).
- UI logs "Service ready (pid=...)" on start.
- Heartbeat ticks visibly in logs.
- Quitting the UI cleanly exits the Service (no orphan in `ps`).
- SIGKILLing the UI exits the Service within seconds (Linux: PR_SET_PDEATHSIG triggers; macOS/Windows: parent-poll triggers).
- Integration test: `crates/service/tests/spawn_and_ping.rs` spawns the service, pings it, asserts the response, shuts down cleanly.

**Risks / open questions.**
- Stdio framing under load (long messages, partial reads). Use `tokio::io::AsyncBufReadExt::read_line` for newline-delimited JSON; document any payload-size cap.
- Logging: Service writes logs to its stderr, which is inherited by the UI's stderr. Works for `cargo run` but may swallow output in packaged builds. Probably fine for v1; revisit if it's painful.
- Test harness for stdio-spawned subprocess. May need a test helper that spawns the service binary in a subprocess from the test process.

---

## Phase 2 - Move the Action service into the Service

**Goal.** The Action service (the existing in-process mutation gate) runs inside the Service. UI dispatches actions via IPC; the Service executes them and reports outcome.

**Entry criteria.**
- Phase 1 landed.
- A clear list of every Action service entry point in the current code (the planning session enumerates these).

**In scope.**
- `service-api` grows new methods: `action.dispatch { account_id, intent }` -> `ActionOutcome`, plus `action.completed` notifications for async completions and undo eligibility.
- Service hosts the action pipeline: `MailActionIntent -> resolve_intent -> build_execution_plan -> batch_execute -> handle_action_completed`.
- UI's existing call sites (currently calling `core::actions::*` directly) replaced with `service_client.dispatch_action(...)`.
- The existing `ActionContext` (`core::actions::ActionContext`) reconstructed on the Service side from Service-owned state (db, encryption key, stores).

**Out of scope.**
- Sync. Sync still happens in the UI process - it'll move in Phase 3.
- Push. Same.
- Streaming progress for long-running actions (e.g. bulk archive of 500 threads). Probably wanted; deferred to a later iteration.

**Touchpoints.**
- `crates/service-api/` - new method + outcome types.
- `crates/service/src/handlers/action.rs` - new.
- `crates/app/src/handlers/...` - replace direct action calls with service-client calls.
- Possibly `crates/core/src/actions/context.rs` - decouple `ActionContext` from `App` state references.

**Exit criteria.**
- All user-triggered actions (archive, delete, label, snooze, etc.) flow through IPC.
- `MutationLog` entries continue to land correctly (logged from the Service side).
- Undo continues to work.

**Risks / open questions.**
- The Action service's interaction with the UI's `nav_generation` / `thread_generation` counters (which gate stale UI updates). Probably: keep the counters UI-side; Service notifications carry no generation; UI bumps generations on receiving completions.
- Error type serialization across the boundary - need to decide which `ActionError` variants survive intact and which collapse into a generic `RemoteError`.

---

## Phase 3 - Move sync into the Service (JMAP first)

**Goal.** JMAP delta sync runs inside the Service. UI gets sync progress and completion via notifications instead of in-process channels.

**Entry criteria.**
- Phase 1 + 2 landed.
- The Action service migration validated the IPC pattern under realistic load.

**In scope.**
- `service-api` new methods: `sync.start_account { account_id }`, `sync.cancel_account { account_id }`. New notification: `sync.progress { ... }`, `sync.completed { ... }`.
- The Service owns sync dispatch: `sync_delta_for_account` runs Service-side using Service-owned db / body store / inline image store / search state.
- The UI's `dispatch_sync_delta` -> `Task::perform(...)` becomes `service_client.start_sync(account_id)` returning a future that resolves on `sync.completed`.
- `App.sync_handles` (the `iced::task::Handle` map from the recent sync-cancellation work) replaced by Service-side cancellation tokens; UI's cancel call becomes IPC.

**Out of scope.**
- Other providers (Phase 5 here ports them).
- Push notifications (Phase 4).
- Re-tuning the existing per-account concurrency limit (4) - stays the same.

**Touchpoints.**
- `crates/service-api/` - sync methods + notifications.
- `crates/service/src/handlers/sync.rs` - new.
- `crates/app/src/handlers/provider.rs` - rewire `dispatch_sync_delta` to talk to the Service.
- `crates/app/src/update.rs` - `Message::SyncComplete` arrives via IPC notification rather than `Task::perform` callback.

**Exit criteria.**
- A JMAP sync triggered from the UI runs in the Service process (visible in `top` / `htop`).
- Sync progress events reach the UI status bar in real time.
- Cancel mid-sync works.
- The "abort sync on account deletion" wiring (recent work) continues to function via IPC.

**Risks / open questions.**
- Sync writes touch the body store, inline image store, search index. All three become Service-owned. The UI's read paths against them stay UI-direct (multi-reader-safe stores).
- Currently `Message::SyncComplete` triggers a navigation reload + thread list refresh. That side effect stays UI-side; the Service just notifies.

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

## Phase 6 - Blob store writes into the Service

**Goal.** All `BlobStore::put` / `unref` / `evict_lru` / `gc` calls happen inside the Service. UI continues to do positional reads from immutable pack files directly. The Service exposes `attachment.fetch` over IPC for cache-miss reads.

**Entry criteria.**
- Phase 1 landed.
- Attachments roadmap Phase 1a + 1b landed (`PackStore` exists, `core::attachments::fetch_or_load` exists).
- Phase 3 landed (sync runs Service-side, so sync-time pre-fetch can wire here naturally).

**In scope.**
- `service-api` new method: `attachment.fetch { account_id, message_id, attachment_id }` -> `Vec<u8>` (or `content_hash` reference for large blobs).
- Service hosts the `BlobStore` writer instance.
- Sync-time pre-fetch (attachments roadmap Phase 2) wires here as the consumer.
- UI's hot-path reads use the existing `PackStore::get` API directly (no IPC).
- Eviction policy + GC from attachments roadmap Phase 6 lands here.

**Out of scope.**
- Settings UI for attachment caching policy (attachments Phase 4 - lives UI-side).
- Calendar attachments (separate work).

**Touchpoints.**
- `crates/service-api/` - `attachment.fetch` method.
- `crates/service/src/handlers/attachment.rs` - new.
- `crates/app/src/handlers/attachments.rs` - cold-path uses service client; hot-path is direct read.

**Exit criteria.**
- `BlobStore::put` calls only happen from inside the Service process.
- UI hot-path attachment reads bypass IPC entirely.
- Cache-miss Open / Save calls succeed via IPC.

**Risks / open questions.**
- Tombstone visibility across processes: the Service tombstones a blob, UI tries to read it before the index commit propagates. Needs careful ordering (Service holds the write lock; UI reads see the post-commit state via SQLite WAL).
- Concurrent reads of the open (currently-being-written-to) pack. Must guarantee the UI never reads past the last fsync'd offset.

---

## Phase 7 - Attachment text extraction + Tantivy indexing in the Service

**Goal.** The forcing function. Cached attachments get text-extracted (per mime-type extractors) and indexed into Tantivy. Search results disambiguate "matched in body" vs. "matched in attachment X." The Tantivy writer lives in the Service; UI keeps its multi-reader.

**Entry criteria.**
- Phase 6 landed (blob store writes are Service-side; cached attachments exist).
- Tantivy writer relocation to the Service is part of this phase.

**In scope.**
- `crates/service/src/text_extract/` - per-mime extractor dispatch. Initial extractors: PDF (via a Rust PDF text extraction crate, TBD), OOXML (`.docx`, `.xlsx`, `.pptx` - zip + xml text extraction), plain text. Skip lists for opaque binaries.
- Tantivy writer relocates to the Service. UI keeps reader.
- Pipeline: pre-fetch -> extract -> add to Tantivy doc with `attachment_*` field tags -> commit batched.
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

## Phase 8 - Crash recovery polish

**Goal.** The Service surviving / failing / being respawned is well-handled. UI shows visible state when the Service is restarting; queued work is preserved across a Service crash.

**Entry criteria.**
- Phases 1-6 landed. Real crashes are happening (or being induced) so we know what hurts.

**In scope.**
- Respawn with exponential backoff on Service crash.
- Crashloop detection: if respawn fails N times in M seconds, surface a permanent error state in the UI ("Service can't start - check logs").
- UI status indicator for Service health (small banner or status bar element).
- In-flight requests are either (a) replayed if idempotent, (b) failed back to the caller with a clear error if not. Schema decision per-method.
- Persistence of the retry queue across Service restarts (already on disk in `pending_ops` table; just verify).

**Out of scope.**
- Hot-restart / live state migration of the Service. Crash + cold restart is the model.

**Touchpoints.**
- `crates/app/src/service_client.rs` - respawn logic.
- `crates/app/src/ui/status_bar.rs` - new "Service degraded" indicator.

**Exit criteria.**
- Killing the Service mid-sync results in a respawn within a few seconds; the affected sync is restarted on the next tick.
- A persistently failing Service surfaces a clear UI error rather than silent breakage.

**Risks / open questions.**
- Distinguishing "Service crashed" from "Service is just slow under load" in the heartbeat. Generous timeouts; longer for the first heartbeat after a sync starts.

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
