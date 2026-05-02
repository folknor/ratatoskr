# The Service

## The problem

Several things Ratatoskr needs to do well are fundamentally incompatible with running on the UI runtime, and the list keeps growing:

- **Full-text indexing of attachment content.** Non-negotiable for an enterprise client - users have to be able to search inside the PDFs, Office documents, and plain-text attachments they receive. Text extraction is CPU-heavy (multi-second for a real-world PDF) and the indexing pass that follows it isn't a one-off - every newly cached attachment goes through it.
- **One-shot re-indexing of multi-year mailboxes.** Schema migrations or extractor-version bumps require re-extracting and re-indexing every cached attachment. At realistic mailbox sizes (the project targets 150 GB+) this is a multi-hour batch job that today would freeze the UI for the duration *and* die instantly if the UI crashed.
- **Sync, push, and the retry queue.** Already long-running, already produce visible UI jank during heavy sessions, already live on the iced runtime by accident rather than by design.

Each of these is CPU-heavy or duration-unbounded write-side work that has no business competing with rendering, input, or UI state updates - and there is no satisfying answer to "how do we do this on the UI thread" that doesn't end with a frozen UI, a crashed sync, or both.

## What this document proposes

Splitting Ratatoskr into two cooperating processes:

- A **UI process** - the existing iced app, owning rendering, input, all UI state, and read-side queries against shared on-disk state.
- A **child worker process called the Service** - owning the long-running operations listed above, plus everything write-side they imply (the action mutation gate, Tantivy writer, blob-store writes, push receivers).

The two communicate over JSON-RPC stdio. The Service is a child of the UI process: spawned at start, killed at exit. It is explicitly **not** a system daemon; there is no autostart, no tray-resident lifetime, no surviving the UI being closed. That promotion is possible later but out of scope for v1.

## Why decide this now

The attachment caching work (`docs/attachments/`) was the proximate trigger for this conversation, but the architectural decision affects multiple in-flight design areas - attachments, search, sync, the action service - all of which need to know which side of the process boundary they land on. Recording the decision now, before any implementation, lets each of those areas plan against a known model rather than discover the boundary mid-build.

## What goes in v1, what doesn't

**In v1:**
- Subprocess lifecycle: UI spawns Service at start; Service dies when UI dies (clean exit on SIGTERM, hard kill if it doesn't respond).
- IPC channel: JSON-RPC 2.0 over stdio (request/response + notifications). Small surface to start; grow as call sites move across the boundary.
- The Action service (existing in-process mutation gate) moves into the Service.
- Sync moves into the Service.
- Tantivy writer moves into the Service. UI keeps its own Tantivy *reader* (multi-reader is native to Tantivy).
- Blob-store writes move into the Service. UI reads positionally from immutable pack files directly (concurrent-reader-safe).
- Attachment text extraction + Tantivy indexing happen inside the Service.
- Push notification receivers (JMAP push, eventually IMAP IDLE) live inside the Service - **but only while the Service is running, which means only while the UI is running.**

**Not in v1:**
- Tray-resident UI ("close window keeps Service alive"). This is the one knob away from option 1; we leave it deliberately undecided.
- True background sync (Service survives UI exit). Same.
- Autostart at user-session login. Same.
- System daemon (systemd / launchd / Windows Service). Explicitly rejected; cross-platform daemon management is not worth the engineering cost for the marginal benefit.
- Multi-UI: multiple UI processes sharing one Service. Conceivable, not in scope.

## Process model

```
            +------------------------+         +-------------------------+
            |     UI process         |         |    Service process      |
            |     (iced app)         |  stdio  |    (child of UI)        |
            |                        | <-----> |                         |
            |  - Rendering           | JSON-RPC|  - Sync (all providers) |
            |  - Input               |         |  - Action service       |
            |  - Tantivy READER      |         |  - Tantivy WRITER       |
            |  - BlobStore READER    |         |  - BlobStore WRITER     |
            |  - Settings UI         |         |  - Attachment fetch+ext |
            |  - Pop-out windows     |         |  - Push subscribers     |
            |  - Tray (deferred)     |         |  - Retry queue          |
            +------------------------+         +-------------------------+
                       |                                   |
                       +--------- shared on disk ----------+
                              ratatoskr.db (SQLite)
                              bodies.db (SQLite)
                              attachment_packs/data-*.pack
                              search_index/
                              app_data/...
```

**Key invariant: the Service is the only writer.** UI never writes to SQLite, the blob store, or the Tantivy index. Reads go through the same in-process code paths the UI already uses. Writes go through IPC to the Service. This invariant is enforced at the type level (see `Type-level enforcement` below) - it cannot be silently broken by adding a new call site.

### Type-level enforcement

Convention isn't enough. The state types the UI process holds today (`DbState`, `BodyStoreState`, `InlineImageStoreState`, `SearchState`) are all `Clone` and writer-capable; nothing prevents a future call site in the UI from writing. We close this with a read/write split at the type level:

- `DbState` -> `ReadDbState` (UI) + `WriteDbState` (Service). The two share the same underlying `Connection`-pool wrapper internally, but only `WriteDbState` exposes write methods.
- Same split for `BodyStoreState`, `InlineImageStoreState`, `SearchState`, the inline-image SQLite store, and any future content store. UI sees the read half; Service sees the write half.
- The Tantivy writer becomes Service-internal and is never handed out as a value; the UI gets a `tantivy::IndexReader` and a notification-driven `reload()` trigger (see `Tantivy reader reload` below).

**Enforcement mechanism: crate boundary, not visibility.** The bare types are not enough - `pub(crate)` constructors get poked at by future contributors. The write halves are constructed in a new `service-state` crate that the `app` crate does not depend on. The `service` crate does. Compile-time prevention then comes from the Cargo dependency graph: a UI call site that writes to `WriteDbState` doesn't fail due to an access check, it fails because the type isn't reachable. `crates/db/src/db/mod.rs`'s `DbState::conn()` and `from_arc()` (which today expose raw `Connection` access) similarly disappear from the UI-visible API.

**Phase staging matters and is detailed in the roadmap.** The full lockdown is not a single phase. Phase 2 introduces the read/write types and uses them for the relocated `ActionContext`. Phases 3-5 relocate sync (which carries the body/inline/Tantivy writer halves with it). Phase 6 closes out the remaining UI write surfaces (preferences, account CRUD, signatures, drafts, pinned searches, calendar, OAuth, etc.) and is the phase at which the global "WriteDbState is unreachable from UI code" invariant becomes true. Reading any one phase as "the lockdown phase" is wrong; the invariant accumulates.

Even so: the moment a write surface relocates, its construction is removed from the UI's reachable API. By Phase 6 there is no escape hatch, no feature flag - adding a new write surface in UI code is a type error, not a runtime contract violation.

### Tantivy reader reload

`tantivy::IndexReader` is snapshot-based. A reader that opens at start and never reloads sees no Service-side commits. Pattern:

- Service emits `index.committed` notifications after every Tantivy commit batch.
- UI owns one shared `IndexReader`; on `index.committed` it calls `reader.reload()`.
- All UI search call sites query through this single reader, so reload is observable globally.

The reload is cheap (cooperative; no work happens until the next searcher acquires) and avoids the lag-behind-arbitrary problem.

## What the UI owns

- All rendering, all iced runtime tasks.
- All user input, including selection state, command-palette state, undo stack, optimistic in-flight UI updates.
- Action *resolution and planning*: `MailActionIntent -> resolve_intent -> build_execution_plan` stays UI-side. These steps read selected threads, sidebar selection, and completion-behavior policy - all UI-owned state. The Service receives a *resolved plan* (a list of `MailOperation` values) to execute.
- Tantivy *readers* (search queries are fast, in-process, no IPC). Reload triggered by Service's `index.committed` notifications.
- Blob store *readers* (positional reads against immutable pack files; OS page cache does the heavy lifting).
- Direct SQLite reads via `ReadDbState` for everything the UI queries (navigation, thread lists, message bodies, etc.). SQLite supports many concurrent readers + one writer; the writer lives in the Service.
- Settings UI (calls into the Service for any state that needs to be written).
- Pop-out windows.

## What the Service owns

- The Action service execution layer (mutation gate). Resolution + planning stay UI-side; the Service receives a `MailOperation` list and executes via `batch_execute`. Completion outcomes stream back as notifications.
- All sync paths (JMAP, Gmail, Graph, IMAP). Pre-fetch of attachments runs here.
- The Tantivy writer + the Tantivy commit cadence.
- All blob-store writes (`PackStore::put`, `unref`, `evict_lru`, `gc`).
- Attachment text extraction (`extract_text(blob_hash, mime_type) -> Option<String>`) and the indexing pipeline that follows.
- Push notification receivers and the JMAP push channel.
- The retry queue (`db_pending_ops_*`).
- Boot-time pending-ops recovery (`recover_on_boot`) - resets stranded "executing" rows back to "pending" before signaling boot-handshake readiness.
- Any periodic background work (cache GC, stale-cache cleanup, contact-photo refreshes, etc.).

(The current `MutationLog` is structured `log::info!` formatting, not a durable subsystem - no replay table exists. Phase 8's "in-flight requests are either replayed if idempotent or failed back" relies on the per-method idempotency contract and the `pending_ops` retry queue, not on a mutation log.)

## Lifecycle

**Start.** UI launches; UI spawns the Service as a child process via `tokio::process::Command`. UI passes the app data directory and runtime config via command-line args. Service initializes its handles to SQLite / blob store / Tantivy writer, then announces readiness via a stdio handshake (response to the UI's first `health.ping` includes `PROTOCOL_VERSION`; mismatch with the UI's compile-time constant is a fatal boot error).

**Health.** Service heartbeats every 30 s. UI logs missed beats; minimal respawn (cold restart, no in-flight replay) lands in Phase 1.5 so any Service crash during Phases 2-7 doesn't take the app down. Polish (backoff, crashloop detection, UI status indicator) lands in Phase 8.

**Shutdown.** UI quit -> UI sends `Shutdown` *request* (not a notification - we need an explicit ack) -> Service stops accepting new work, drains in-flight handlers, flushes Tantivy writer + closes pack files cleanly + writes a clean-shutdown sentinel file -> Service responds -> Service exits -> UI exits. UI awaits the response with a 30 s timeout (large Tantivy commits + pack-file fsync can take real time). On timeout: SIGTERM, then SIGKILL after another 5 s.

**`kill_on_drop` is disabled on the UI's `tokio::process::Child` handle.** The default `kill_on_drop(true)` interacts badly with the SIGTERM-then-SIGKILL policy: if `service_client.shutdown()` returns `Err(Timeout)` and the calling code is in the middle of escalating to SIGTERM, dropping the `ServiceClient` (panic in the caller, normal teardown order) would have `kill_on_drop` SIGKILL the process the SIGTERM was supposed to gracefully terminate. The shutdown sequence (SIGTERM, wait, SIGKILL, then drop) is explicit; the default isn't safe.

**The Service installs a SIGTERM handler** that triggers the same shutdown flow as the request-driven path. Without it, `kill <service-pid>` (or the UI's escalation path) terminates without flushes; with it, an external SIGTERM is treated as a polite "please shut down" with the same drain + flush + sentinel sequence. Worst case (SIGKILL, OOM, hard machine power-off): torn writes detected on next start by the per-store recover paths each subsystem owns, plus the cross-store invariant pass gated on the missing sentinel file.

**Crash.** If the UI crashes, the Service must exit cleanly. v1 ships for Linux and Windows. Linux closes the parent-died-before-registration race with registration *plus* a post-registration "is the parent still alive?" recheck; Windows avoids the race entirely via Job Object semantics.

- **Linux**: `pre_exec` hook calling `prctl(PR_SET_PDEATHSIG, SIGTERM)`. After the hook runs, the Service code re-checks `getppid()` once at startup; if `getppid() == 1` the parent already died before the hook took effect, exit immediately.
- **Windows**: a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` is the documented Windows idiom for child-lifetime binding. The UI creates the Job before spawning the Service, sets the kill-on-close flag, and assigns the Service to the Job. When the UI dies (handle released), the OS terminates every process in the Job. No PID lookup means no PID-reuse race. Side benefit: any further grandchildren the Service spawns (e.g. PDF/OOXML extractor subprocesses in Phase 7) inherit the Job and are killed with the parent.

PID-polling is rejected: PID reuse is fast on busy systems, and the Service would attach to a stranger if the parent died between polls. `OpenProcess(parent_pid)` after startup has the same flaw on Windows (TOCTOU between exec and OpenProcess) and is also rejected; the Job Object pattern avoids the PID lookup entirely.

**Grandchildren** (Phase 7 PDF/OOXML extractor subprocesses) inherit Service-death detection as follows:

- **Linux**: `PR_SET_PDEATHSIG` only covers a process's *immediate* parent. The cascade still fires (UI dies -> Service gets SIGTERM -> drains -> exits -> extractor's parent (Service) just died -> extractor gets *its* `PR_SET_PDEATHSIG` signal), but only if every Service-spawned child applies the same `pre_exec` PR_SET_PDEATHSIG + post-prctl `getppid() == 1` recheck the Service uses. Phase 7's extractor-spawn helper must apply the pattern; this is a per-child contract, not a one-time setup.
- **Windows**: Job Object inheritance handles this for free. Any child the Service spawns is automatically assigned to the same Job (default behavior unless `CREATE_BREAKAWAY_FROM_JOB` is set, which we don't), so the UI's handle drop terminates the entire process tree.

**macOS (deferred to post-1.0).** Design retained for when macOS becomes a target: `kqueue` with `EVFILT_PROC` and `NOTE_EXIT` registered against the parent PID at start. Registration fires only against *that specific process* (immune to PID reuse), but the parent can die between fork and registration. Same race-close as Linux: after `kqueue` registration, recheck `getppid() == 1` (parent reparented to launchd) and exit if so. Not validated in v1; Phase 1 exit criteria do not require it.

**No persistent process.** Quit the app, the Service is gone. Restart the app, both come back. This is deliberate v1 simplicity. Tray-residency promotion (Phase 9) requires no Service-lifecycle changes - only a UI-side change to not call `iced::exit()` on close. The Service's lifecycle is keyed on the UI *process*, not the visible window, so the parent-death machinery still works correctly.

**Single-instance guard.** Once schema migrations move Service-side (Phase 1.5), a second app instance trying to migrate the same data dir would race. SQLite migrations wrapped in `BEGIN EXCLUSIVE` would fail in the second Service; the second UI would see "Service crashed" - a misleading error for "another instance is running." The Service takes an OS-level file lock on a dedicated lock file under the app data directory at boot.

The lock is *not* a JSON-RPC `ServiceError`: the Service exits before stdio is established as a JSON-RPC channel. The UI distinguishes Service exit codes - clean / handshake-failure / instance-already-running / migration-failure / fatal-key-load - via a small `BootExitCode` enum, and surfaces "Ratatoskr is already running" on the corresponding code rather than treating it as a crash. Cross-platform file lock library: `fs2` and `fd-lock` are the candidates; both work on Linux and Windows. Pick in the Phase 1.5 plan.

Lock release semantics:
- **Linux**: kernel releases the lock on process exit (clean, panic, SIGKILL all OK).
- **Windows**: `LockFileEx`-style locks release on handle close, which the kernel does on process exit. Same coverage.

The respawn loop must `wait()` on the dying child *before* spawning a replacement; otherwise the new Service races the old one for the lock and bounces with "another instance," which is indistinguishable in the UI from a real second-instance attempt.

**Service log file naming.** The rolling log file is multi-writer-unsafe: a respawning Service writes to the same file while the dying Service might still hold a handle. The Service writes to `service.<pid>.log` (PID in the filename) and rotates at boot only.

A pointer to "the current Service's log" is a `service.log` *symlink* on Linux. On Windows, symlinks require Developer Mode or admin since Win10 1703 and are not a default-acceptable dependency. On Windows the equivalent is a small `service.log.txt` text file containing the current Service's PID (rewritten at each boot); operators run `type service.<pid>.log` against that PID. (`Windows shortcuts (.lnk) are GUI-shell-only and aren't useful from a terminal.`)

## IPC

**Transport.** JSON-RPC 2.0 over stdio. Newline-delimited JSON per message. UI writes to the Service's stdin; the Service writes to its own stdout (read by the UI as the child's stdout).

**Framing constraint.** Every message is exactly one line of compact JSON terminated by a single `\n`. No `to_string_pretty`, no embedded literal newlines in payloads. The wire-format crate (`service-api`) exposes a single `write_message` helper that enforces this; direct `serde_json::to_writer` is forbidden at the public API. One careless pretty-print would desync the framing.

**Why stdio + JSON-RPC.** Universally available, no socket files / port allocation / firewall concerns, well-supported by Rust crates, debuggable by piping to a file. Schema can grow without protocol changes.

**Why not gRPC / Cap'n Proto / shared memory.** All add machinery we don't need at this scope. The hot path is intentionally not crossing the IPC boundary - reads are direct against on-disk state. Writes are infrequent enough (sync per N seconds, action per user click) that JSON serialization cost is invisible.

**Backpressure and resource bounds.** First-class concern, not a polish item.

- **Notification class taxonomy.** Not all notifications are interchangeable. Each `service-api` notification declares one of three classes:
  - `Coalesce { key }` - duplicates collapse on the enqueue side. Latest-wins. Use for `sync.progress`, `index.progress`, `attachment.fetch_progress`.
  - `Drop` - drop oldest under queue pressure. Use for advisory events (heartbeat ticks, debug telemetry).
  - `MustDeliver` - must reach the UI; never coalesced or dropped. Use for state-change events: `action.completed`, `index.committed`, `push.event`, `attachment.cached`. If the UI consumer of `MustDeliver` notifications stalls, the right outcome is end-to-end backpressure: the UI reader stops draining the pipe, OS pipe buffers fill, the Service's writer task blocks, the producing handler blocks. Slow UI does not silently drop `action.completed`; it slows the Service.
  Without this taxonomy, dropping `action.completed` silently desynchronizes generations and `index.committed` silently lags the search reader.
- **Single ordered notification channel at the UI client.** The reader task on the UI side parses lines off the pipe in wire order and enqueues to **one** ordered `mpsc` channel (separate from the response pending-map). Per-class policy is enforced at *enqueue*: `Coalesce` overwrites the last entry with the same key, `Drop` evicts oldest on full, `MustDeliver` uses awaited send (`Sender::send().await`, not `try_send`). One channel preserves cross-class FIFO order, so `action.completed` (MustDeliver) can never be observed before the per-operation `OperationOutcome` events that preceded it on the wire. (An earlier draft used two channels and `try_send` for both lanes; that violated `MustDeliver` semantics and lost cross-class ordering. Settled here as the one-channel design.) Channel capacity ~1024.
- **Inbound framing cap is enforced *during* read, not after.** The Service's reader uses a bounded line decoder (`tokio_util::codec::LinesCodec::new_with_max_length(MAX_FRAME_BYTES)` or equivalent `read_until` against a `Take`-wrapped reader) that rejects once `MAX_FRAME_BYTES + 1` bytes have been seen *without* having allocated the whole oversized line. A 1 GiB no-newline payload must not OOM the Service before the cap fires.
- **Bounded in-flight requests.** The dispatch loop spawns at most N (default 64) concurrent handlers; further requests wait on a semaphore rather than ballooning Service memory under a pathological client. **Acquire the permit *inside* the spawned handler task, not in the dispatch loop**: acquiring before `tokio::spawn` would stall the dispatch loop's stdin read whenever 64 slow handlers are in-flight, blocking fast methods (e.g. `health.ping` heartbeats survive only because pipe buffers exceed ping size, but other fast methods would queue behind slow ones). Acquiring inside means dispatch keeps reading; queued tasks contend for the semaphore on their own.
- **Outbound `MustDeliver` and the in-flight semaphore interact.** A handler that emits `MustDeliver` notifications mid-flight (e.g. per-operation `OperationOutcome` from a bulk action) holds its semaphore permit while awaiting outbound enqueue. If the outbound pipeline is backpressured (slow UI consumer), the handler blocks holding the slot, starving further dispatch. Either size the outbound queue large enough that this is not the bottleneck, or release the permit before the final `MustDeliver` send. The Phase 2 plan owns this decision since `OperationOutcome` is the first `MustDeliver` use site; tuning is not Phase 1.
- **Service stdout writes:** the dispatch loop never blocks on stdout. A dedicated writer task drains a bounded queue and writes to stdout. Queue full *for* `MustDeliver` *items* applies backpressure on the producer (awaited send into the queue); for `Coalesce`/`Drop` items the queue applies the per-class policy.
- **Requests:** every method declares its timeout at the API definition site, not at the call site. The defaults table:
  | Method | Timeout |
  |--------|---------|
  | `health.ping` | 5 s |
  | `Shutdown` | 30 s |
  | `action.execute_plan` | 60 s (bulk operations) |
  | `sync.start_account` | 600 s (large initial syncs) |
  | `oauth.exchange_code` | 30 s |
  | `attachment.fetch` | 60 s |
  | `index.rebuild` | infinite |

  Expired requests evict their pending entry so leaked oneshots don't accumulate.
- **Large blobs:** `attachment.fetch` doesn't return `Vec<u8>` over JSON. It returns `{ content_hash, size }`; the UI re-reads positionally from the pack file. Same for any future "give me bytes" method - JSON carries the location, not the content.
- **Per-line frame size cap:** 4 MiB. Anything larger is a contract violation and gets rejected at the framing layer.

**Stdio discipline (corruption defense).** The Service uses stdout exclusively for JSON-RPC frames. A transitive dependency calling `println!`, a `tracing-subscriber` defaulting to stdout, an interactive panic handler reading stdin, or any `eprintln!` accidentally redirected during dev all break the framing irrecoverably. The Service therefore claims its real stdin/stdout at the top of `run_service()` and replaces the standard slots with sinks before any other code runs. Per-platform mechanism:

- **Linux**: `dup` `STDIN_FILENO` and `STDOUT_FILENO` to saved FDs; open `/dev/null` and `dup2` it onto `STDIN_FILENO`/`STDOUT_FILENO`. Reader/writer tasks operate on the saved FDs (wrapped in `tokio::fs::File` or `OwnedFd` -> `tokio::io::AsyncFd`).
- **Windows**: `DuplicateHandle` the standard input and output handles; open `NUL` (`CreateFileW("NUL", ...)`) and `SetStdHandle(STD_INPUT_HANDLE | STD_OUTPUT_HANDLE)` to point at it. Reader/writer tasks operate on the duplicated handles. CRT fd table is updated via `_open_osfhandle` / `_dup2` if any C-runtime call site might write to fd 0/1. `AllocConsole` and `AttachConsole` create new console buffers outside the std handle slots; transitive use is rare but worth flagging in the Phase 1.5 plan if any dep calls them.

The defense covers the standard FDs/handles. It does *not* intercept direct writes to `/dev/tty` (Linux) or `CONOUT$` (Windows) - those are vanishingly rare in libraries but possible in panic-handler tooling; we accept the residual risk and check with a stress test before Phase 1 ships.

**Pipe binary mode (Windows).** `tokio::process::Command` on Windows opens piped stdio in binary mode by default - explicitly verify before Phase 1 ships. CRLF translation on the pipe would silently corrupt JSON-RPC framing the moment any embedded `\r` shows up in a payload.

**Stdio inheritance for grandchildren.** Children the Service spawns later (Phase 7 PDF/OOXML extractors) inherit the redirected (post-defense) stdio by default. That's almost always what we want - a runaway extractor's stdout lands on `/dev/null`/`NUL`, not in our JSON-RPC stream. If the Service ever needs to capture an extractor's stdout (e.g. for log forwarding), it must explicitly pipe it via `Stdio::piped()`; inheritance gives empty output rather than the original stdout.

The framing layer's `write_message` helper enforces compact serialization (no `to_string_pretty`, no embedded literal newlines); direct `serde_json::to_writer` is forbidden at the public API.

**Sensitive-value logging policy.** The Service handles message bodies, OAuth bearer tokens, encryption-keyed payloads, search queries (which contain user PII), draft auto-save content, and attachment text. The rolling log file in `<app_data>/logs/service.<pid>.log` must never contain these:

- **Loggable**: request method names, request IDs, account IDs, folder IDs, message IDs, thread IDs, error codes, timing measurements.
- **Not loggable**: any `params` or `result` payload contents; OAuth auth codes (one-shot bearer credentials); message bodies; search queries; draft content; encryption-key bytes; attachment bytes or extracted text.

Wire types whose serialization would otherwise reach the logger wrap sensitive fields in a redacting `Debug` impl (e.g. `RedactedString`, `RedactedBytes`). The framing layer's logging hook records method + id + timing, never the payload. Any handler that needs to log diagnostic detail logs an aggregate (size, hash) rather than content.

**Message shape (sketch, settled in implementation):**

- `Request { id, method, params }` -> `Response { id, result | error }`. Synchronous-feeling, fits the action service / settings write surface.
- `Notification { method, params }` for one-way streams (sync progress, push events, indexer progress, `index.committed`). UI dispatches as iced messages.

**Panic safety.** Every handler runs inside `AssertUnwindSafe(...).catch_unwind()` (or as a `tokio::task::spawn` whose `JoinError` is treated as a service error). On debug + the default panic strategy, a panicking handler returns `ServiceError::Panic { method, message }` to the caller; the dispatch loop continues.

**However, the workspace release profile sets `panic = "abort"` (`Cargo.toml:117`).** In release builds, `catch_unwind` does not catch panics - the process aborts. The contract on the wire therefore varies by build profile: in debug, callers can observe `ServiceError::Panic` and the Service stays up; in release, a panicking handler crashes the Service and the UI sees `ClientError::ServiceCrashed` followed by a respawn (Phase 1.5). The doc previously claimed panic-as-error in production - that was wrong.

We accept this for v1: panics in mature handler code are rare, and the Phase 1.5 respawn loop is the production safety net. If a future phase needs production catch-and-continue (e.g. one bad PDF should not crash the Service even in release), revisit by adding a release profile override on the `service` and `service-api` crates with `panic = "unwind"`. Out of scope for v1.

**Surface scope.** Start small: `health.ping`, `sync.start_account`, `sync.cancel_account`, `action.execute_plan`, `attachment.fetch`, `index.rebuild`, plus notifications (`sync.progress`, `push.event`, `action.completed`, `index.committed`, `attachment.cached`). Grow per phase as call sites move.

## The Action service / The Service: clarifying naming

**The Action service** is the existing in-process mutation gate (`docs/architecture.md` § "Action service as mutation gate"). Today it lives in the UI process. After this work, it lives inside **the Service** (the new subprocess). The Action service is one of the things the Service hosts - it is not the Service itself.

In code, lowercase `service` may already be used in module paths (e.g. `crates/core/src/actions/`); we don't need to rename anything. In docs and conversation, "the Service" (capitalized, with definite article) refers specifically to the subprocess worker. Where the distinction matters in prose, write "Action service" or "action mutation gate" for the existing concept and "the Service" for the new subprocess.

## Search and attachment indexing

The forcing function. Pipeline (all inside the Service):

1. Sync writes attachment metadata (existing). Pre-fetch policy (per-account toggle + size threshold) decides whether to fetch bytes.
2. Pre-fetch fetches bytes via `ProviderOps::fetch_attachment`, runs `squeeze::compress`, stores via `BlobStore::put`. Updates `attachments.content_hash`. (See `docs/attachments/problem-statement.md`.)
3. Text extractor reads bytes from the blob store, dispatches by mime type to the appropriate extractor (PDF, OOXML, plaintext, etc.). Returns `Option<String>` of extracted text. Failures are logged; non-extractable types (mp4, zip, opaque binaries) skip.
4. Indexer adds the extracted text to the Tantivy document for the parent message, with field tags so search results can disambiguate "matched in body" vs. "matched in attachment X."
5. Periodic Tantivy commit batches the writes.

Search queries are UI-side (Tantivy multi-reader): fast, no IPC. Indexing is Service-side: never blocks the UI.

**Re-index** (one-shot, runs entirely in the Service): on schema migration or on user demand, the Service walks every cached attachment, re-extracts, re-indexes. Multi-hour for a large mailbox. The user keeps the app open; progress reported via the existing progress channel.

## Settings considerations

Three new settings entries surface from this work:

- **"Index attachment content for search"** (toggle, default on). When off, attachments still get cached but text extraction + indexing is skipped. Mostly an opt-out for users who don't want CPU spent on it.
- **"Maximum attachment size to index"** (slider, MB, default 50 MB). Above this, skip text extraction even if the attachment is cached. Protects against multi-hundred-MB PDFs eating CPU.
- **"Service crash recovery"** (status, not a setting per se): UI shows a small status indicator if the Service has had to be respawned recently, so users have visibility when something's off.

## Out of scope (v1)

- **Tray-resident UI.** Closing the window quits the app. The change to enable tray residency is one decision (don't `iced::exit()` on close), but it brings real complexity (icon assets, tray menu, notifications surface) and we'd rather ship without it first.
- **True background sync.** Same - requires either tray residency or a daemon, both out.
- **Autostart at login.** Per-platform registration; deferred until tray residency lands.
- **System daemon.** Explicitly rejected. The "always-on" experience can be approximated with tray residency later if it proves necessary.
- **Multiple UI processes per Service.** Not a target.
- **Service restart in place during a long-running sync.** If the Service crashes mid-sync, the next start re-syncs from the last checkpoint. No live-migration of in-flight work.
- **Schema migrations of the IPC protocol.** v1 ships one protocol version. Bump the JSON-RPC method names if the contract changes; treat the UI and Service as a tightly coupled pair shipped together.

## Migration policy

Each cross-boundary migration of a subsystem (sync, blob writes, Tantivy writer, etc.) is a **single atomic commit**. No straddling: the subsystem is either entirely in the UI's address space or entirely in the Service's. Validated mechanically: before each phase lands, the Service-bound subsystem's writer-constructor is moved into a service-only crate. The UI's Cargo dependency graph cannot reach it, so any UI call site that hasn't been migrated yet fails to compile. This forces every call-site move to land with the migration commit instead of being discovered later.

Note that "WriteDbState is unreachable from the UI" is not a Phase 2 invariant. It accumulates phase-by-phase as each writer relocates, and is fully realized only at Phase 6. See `Type-level enforcement` above.

The "exists but not wired" failure mode flagged in CLAUDE.md's multi-agent rules applies here in spades. The atomic-commit policy is the structural defense against it.

**Forward-only data directory.** Once schema migrations move to the Service (Phase 1.5), the data dir is forward-compatible only. A user who downgrades to an older binary may find the schema unrecognized; rollback across schema bumps is unsupported. The v1 commitment that "UI and Service ship as a coupled pair" extends to the on-disk state: ship them together, and don't downgrade past a schema version.

**Single-binary cost.** v1 ships one `ratatoskr` binary in two modes (UI default; `--service` flag selects Service mode). The Service mode therefore links the full UI dependency graph (iced, fonts, all four provider client crates, etc.). Acceptable for v1 simplicity; binary size will be >100 MB. A future feature gate (`#[cfg(feature = "service_only")]`) could strip iced from the Service link path - explicitly out of scope for v1.

## Write-surface inventory (UI -> Service migration)

Every place the UI process writes to durable state today, mapped to the phase that relocates it. This list anchors the migration policy above; if a write surface isn't on this list, it hasn't been considered. Reviewer scans against current code surfaced several entries the original draft missed - they are included below.

| Write surface | Today (UI-side) | Relocates in |
|---------------|-----------------|--------------|
| Action service mutations (archive, label, snooze, etc.) | `core::actions` invoked from `handlers/commands.rs` and many UI sites | Phase 2 (execution only; planning stays UI-side) |
| Pending-ops retry queue (`db_pending_ops_*`) | Background task in UI; drains queue, re-dispatches actions | Phase 2 (rides with the action service) |
| Pending-ops boot recovery (`recover_on_boot` resets stranded executing rows) | Runs at app boot in UI process | Phase 1.5 (gates the boot handshake; the periodic drainer relocates separately in Phase 2 with the action service) |
| Undo (compensating-action dispatch in `handlers/commands.rs`) | Builds a reverse plan, invokes core::actions directly | Phase 2 (undo follows the action pipeline) |
| Compose send (uses `ActionContext` for SMTP submit + DB updates) | `handlers/pop_out/compose_send.rs` | Phase 2 (send is a Service-side action; large attachments mean its IPC timeout is generous - see timeout table) |
| Snooze resurfacing (timer-driven) | `handlers/commands.rs` SyncTick path | Phase 2 (becomes a Service-internal periodic task; UI receives `action.completed` notifications) |
| Local-draft "queued failed" sweep (`db_mark_queued_drafts_failed_sync` at app boot) | Synchronous DB write in `App::boot` | Phase 1.5 (rides with schema-migration relocation; runs Service-side, gated on the boot handshake) |
| Sync writes (DB metadata + body store + inline image store + Tantivy) | `core::sync_dispatch::sync_delta_for_account` per account | Phase 3 (JMAP), Phase 5 (other providers) |
| Body store writer (`bodies.db`) | Inside sync persistence | Phase 3 (entangled with sync; moves with it) |
| Inline-image store writer | Inside sync persistence | Phase 3 (entangled with sync; moves with it) |
| Tantivy writer + commit cadence | Inside sync persistence; reader shares the writer's state | Phase 3 (writer relocation lands with sync; reader split happens here, reader stays UI-side and reloads on `index.committed` notifications) |
| Blob store writes (`PackStore::put / unref`) | Library lands in attachments Phase 1a; pre-fetch path is sync-side | Phase 3 (the writer rides with sync because pre-fetch is in the sync path) |
| Blob store eviction / GC (`evict_lru`, `gc`, tombstone propagation) + `attachment.fetch` IPC for cache-miss reads | n/a today | Phase 6 |
| Push state (JMAP push state, IDLE state) | `crates/jmap/src/sync/...` push tables | Phase 4 |
| Preferences (`set_setting` via `handlers/core.rs`) | Direct DB write from UI handler | Phase 6 |
| Account create / update / delete (`handlers/core.rs`, `update_account_sync`) + account reorder | Direct DB write | Phase 6 |
| Signature CRUD + reorder (`handlers/signatures.rs`) | Direct DB write | Phase 6 |
| Local draft auto-save (`handlers/pop_out/compose_draft.rs`) | Sync DB write on window close, async write on tick | Phase 6 (with explicit ordering against `iced::exit()` - see Phase 6 plan) |
| Pinned searches (`db/pinned_searches.rs`) | Direct DB write | Phase 6 |
| Calendar mutations (`handlers/calendar.rs`) including series/occurrence + RSVP | Direct DB write | Phase 6 (series/occurrence + RSVP semantics may not fit a flat `MailOperation` list - flagged as Phase 6 risk) |
| Calendar provider sync writes | UI-side via `handlers/provider.rs` | Phase 5 (rides with sync ports) |
| Contacts / GAL refresh writes | `handlers/contacts.rs`, GAL refresh in `handlers/provider.rs` | Phase 6 |
| Attachment collapse-state preferences | UI-side | Phase 6 (with the rest of preferences) |
| Chat read-on-view side effect (`mark_chat_read_local_sync` on entering a chat) | `handlers/chat.rs` calls `db.read_db_state()` then writes via `crates/db/src/db/queries_extra/chat.rs:218` | Phase 2 (this is a `MailOperation`-shaped action; the read-on-view trigger fires from UI but the mutation goes through the action service) |
| Thread-participants backfill at boot (`handlers/core.rs`) writing to `thread_participants` via `crates/db/src/db/queries_extra/thread_persistence.rs:670` | Synchronous DB write at app boot | Phase 1.5 (rides with the boot-side relocations) |
| Schema migrations | Run at boot in the UI process | Phase 1.5 - relocate to Service boot, UI defers `ReadDbState` construction until the Service signals "schema OK" via the boot handshake |
| Velo->Ratatoskr DB rename migration | Runs in `ReadWriteDb::init` UI-side | Phase 1.5 - moves with schema migrations; Service is the only process that should rename |
| Encryption-key load | Read from disk at boot in the UI process | Service reads it itself at boot. Not via IPC - the key is needed before any token-bearing IPC can happen. Phase 1.5. **Missing/unreadable key on Service boot is a fatal exit, not a silent zero-key fallback** - the auto-respawn machinery would otherwise widen the window where data gets written under the zero key. |
| OAuth flow (redirect listener + token exchange + token persist) | UI process owns the redirect listener (it's the visible app); token persist is a DB write | Phase 6: UI captures redirect; ships code to Service; Service exchanges + persists. Two-step IPC. (Phase 4's Service-side push needs a token-refresh path before Phase 6 lands - addressed in Phase 4 plan.) |
| Global `Db` / state handles (`OnceLock<Arc<Db>>` populated synchronously at app start) | `crates/app/src/main.rs` opens DB before `App::boot` | Phase 1.5 - initialization defers until Service handshake; many sync `crate::DB.get().expect(...)` call sites need to flip to async-init or accept a not-yet-ready state |

Anything not in this table is either (a) read-only from the UI's perspective and stays UI-side, or (b) we missed it - in which case each phase planning session has explicit homework to grep for `with_write_conn` / `db.execute` / similar in UI code and reconcile. **Grep alone is not sufficient.** Some UI handlers obtain a `read_db_state()` and then call into a function that internally opens a transaction and writes (the chat read-on-view path is the canonical example: `handlers/chat.rs` -> `read_db_state()` -> `mark_chat_read_local_sync` -> `tx.execute`). Each phase planning session must also walk the call graph from any UI-side `read_db_state()` user looking for downstream `Transaction::execute` / `with_write_conn` calls.

## Cross-store crash consistency

The Service writes to four durable stores: SQLite (main + `bodies.db`), pack files, Tantivy, inline-image store. A crash mid-sequence can leave inter-store inconsistencies: `attachments` row written but pack append not fsync'd; Tantivy doc references a body the body store hasn't durably committed; etc. Each store has its own crash-safe recovery (SQLite WAL, append-only pack scan, Tantivy uncommitted-segment cleanup) but cross-store reconciliation is not free.

The reconciliation work splits across two phases. **The minimal pass lands with the writer relocation that introduces the cross-store risk** - i.e. Phase 3, the moment the Service first writes four stores. The optimized invariant pass (with marker-file gating and bounded re-scan windows) stays in Phase 8.

**Clean-shutdown sentinel.** The Service writes a `clean_shutdown` sentinel file (in `<app_data>/`) at the end of its shutdown drain, and removes it at boot once it has acquired all writer handles. On boot, if the sentinel is missing (i.e. last process did not shut down cleanly), the Service runs the per-store recovery pass before signaling boot-handshake readiness.

**Exit-path matrix.** The teardown story has five interacting mechanisms (`kill_on_drop` disabled on the child handle, explicit `ServiceClient::Drop` ordering, Service-side SIGTERM handler, sentinel writer at the end of the drain, and Windows Job Object kill-on-close). They produce different outcomes per teardown trigger; explicit table:

| Trigger | Sentinel written? | Recovery scan next boot? | UI-observed result |
|---------|-------------------|--------------------------|---------------------|
| Graceful UI quit (Shutdown request -> ack) | yes | no | clean exit |
| UI quit but Service unresponsive (30 s timeout -> SIGTERM Linux / TerminateProcess Windows) | Linux: yes (SIGTERM handler runs the drain); Windows: **no** (TerminateProcess is not catchable) | Linux: no; Windows: yes | clean exit on Linux; "last shutdown was unclean" scan on next Windows boot |
| UI quit + Service still unresponsive 5 s after escalation (SIGKILL Linux / handle drop Windows) | no | yes | scan on next boot; UI logs the timeout |
| UI panic / OOM-kill | no (Linux: pdeathsig delivers SIGTERM but parent is already gone, the drain may run; Windows: Job Object kills before any handler runs) | yes | scan on next boot |
| Service panic in handler (debug profile) | not reached - Service stays up via `catch_unwind` | n/a | UI sees `ServiceError::Panic` |
| Service panic (release profile, `panic = "abort"`) | no | yes | UI sees `ClientError::ServiceCrashed`; respawn (Phase 1.5) |
| External SIGTERM to Service (Linux) | yes (SIGTERM handler runs the drain) | no | UI sees the channel close after the drain; logs missed heartbeat; respawn (Phase 1.5) |
| External SIGKILL to Service | no | yes | UI sees the channel close immediately; respawn |
| External TerminateProcess to Service (Windows) | no | yes | same as SIGKILL |
| Hard machine power-off / kernel panic | no | yes | torn-write recovery via per-store + cross-store passes |

The Windows asymmetry - 2 of the 3 "shutdown" exit paths and the panic/abort path land without a sentinel - means abnormal Windows exits routinely trigger the recovery scan. That's by design: Phase 8's marker-file gating shrinks the scan to "what changed since last clean shutdown," not "everything," so the cost on a 200 GB mailbox is bounded. Phase 3 / Phase 6 ship the slow scan; users on Windows will pay it more often than Linux users until Phase 8 lands.

A future enhancement (out of scope for v1) is a UI-side "pre-kill" sentinel write before `TerminateProcess`, so the Windows clean-exit path matches Linux. Costs a UI->disk write in the shutdown hot path; not worth the complexity until Phase 8 lands and we measure how often the recovery scan actually fires in practice.

**Phase 3 (minimal pass, lands with sync relocation).** Naive but correct, full-table scans, no optimization:

- For every Tantivy doc: assert the message id still exists in `messages`. Drop orphans.
- For every body store entry: assert the message id still exists. Drop orphans.
- For every inline-image store entry: assert the message id still exists. Drop orphans.

**Phase 6 (extends with blob-store reconciliation).** Lands when the blob store eviction/GC moves Service-side:

- For every `attachments` row with `content_hash IS NOT NULL`: assert the pack store can resolve the hash. If not (post-crash orphan), null the column and let the next sync re-fetch.
- For every pack-file orphan blob with refcount > 0 but no referring `attachments` row: schedule for `gc`.

**Phase 8 (optimization).** Replaces the full-table scans with marker-file gating ("scan only what's been written since last clean shutdown"), bounded windows, and visible status reporting. The Phase 3 / 6 passes are correctness-preserving but slow on a 200 GB mailbox; Phase 8 makes them fast.

All passes are idempotent. Logged with stats so we can see how often crashes leave us reconciling.

## Cross-cutting impact

This decision touches a lot. Listed for visibility, addressed in detail in the implementation roadmap:

- **Sync architecture.** All four provider sync paths move from "tokio task in the UI process" to "tokio task in the Service process." `Sync` IPC notifications replace today's direct `Message::SyncComplete` dispatch.
- **Search.** Tantivy writer relocates as part of Phase 3 (sync owns the writer today). Readers stay UI-side, driven by `index.committed` notifications. Attachment text extraction wires in as a Phase 7 pipeline step on top of the already-Service-side writer.
- **Action service.** Execution-side relocates; planning + completion-effects stay UI-side. The IPC surface is `action.execute_plan { plan }` -> notifications.
- **Push notifications.** JMAP push subscriber moves into the Service. Push events become Service-internal triggers for sync; UI gets `push.event { account_id }` notifications for status-bar visibility.
- **Attachments.** All write-side operations (`BlobStore::put`, `unref`, `evict_lru`, `gc`) become Service-internal. Read-side stays UI-direct against on-disk pack files. The attachment problem statement's Phase 1a (`PackStore` library) is unchanged; Phases 1b/2 (orchestration + sync wiring) move into Service phases.
- **Progress reporting.** Today the UI provides the `&dyn ProgressReporter` impl. After Phase 2, the Service constructs an impl that serializes events into IPC notifications. This is a real refactor (the "no shape change" framing was wrong), addressed as a sub-step of Phase 2. The notification class taxonomy applies: `sync.progress`/`index.progress` are `Coalesce`; `index.committed`/`action.completed` are `MustDeliver`. This shapes the channel topology, not just the cadence.
- **Optimistic UI under IPC latency.** Today, optimistic UI updates roll back from the same call stack that issued the action. After Phase 2, rollback depends on the round-trip - and on `ServiceCrashed` errors when the Service dies mid-action. The rollback path must trigger from both `action.completed` (success/failure) and `ClientError::ServiceCrashed` (peer disappeared). Generation counters bump *pre-dispatch* (on plan submission), not post-completion - otherwise the IPC delay creates a window where stale loads can land between dispatch and ack.
- **Read-after-write coherence.** Service emits `action.completed` only after WAL fsync. Without that contract, the UI's natural pattern of "got `action.completed`, now refresh thread list" can return pre-commit data on a slow disk.
- **Action latency budget.** Toggle-star is sub-millisecond today. After Phase 2 it crosses IPC: process boundary + JSON serialize + scheduler gap + Service dispatch + DB write + notification round-trip. Realistic budget ~5-15 ms p99 for action submit-to-ack under a healthy Service. Phase 2 lands an action latency benchmark so regressions are observable.
- **Tests.** Most existing tests are in-process and work unchanged. The Phase 1 integration test uses an in-process dispatch harness (`tokio::io::duplex` driving `run_service_with_io`) - covers the dispatch contract. Phase 1 also adds a small set of real-subprocess tests (spawn + ping, spawn + shutdown, Linux parent-death) to exercise things in-process duplex by definition cannot test: `--service` flag dispatch, real stdio pipe wiring, child cleanup on Drop, parent-death detection. Per-platform parent-death matrices stay manual.
- **Crash recovery.** Each subsystem owned by the Service has a per-store recovery story (lands with the relocation, not deferred). Cross-store invariant pass lands incrementally: minimal Phase 3 (Tantivy/body/inline orphans), extended Phase 6 (blob orphans), optimized Phase 8 (marker-file gating + bounded windows).

## Verification

End-to-end behavior to test once the v1 phases land:

1. Start the app. Both processes are visible in `ps` / Task Manager. UI is parent, Service is child.
2. Issue a search. Result returns from UI-side Tantivy reader; Service is not involved on the read path.
3. Trigger a sync. Service does the work; UI gets sync progress notifications (coalesced under load). UI rendering remains responsive throughout.
4. Send a kill signal to the Service mid-sync. UI detects the lost heartbeat, respawns the Service, the boot handshake re-runs (schema check + key load + per-store recovery if sentinel missing), sync resumes from the last checkpoint.
5. Quit the UI. Service receives shutdown request, drains in-flight work, flushes Tantivy writer + closes pack files, writes the clean-shutdown sentinel, exits cleanly. No orphan process in `ps`.
6. SIGKILL the UI. Service detects parent death (Linux: PR_SET_PDEATHSIG + getppid recheck; Windows: Job Object kill-on-close), exits within seconds. No orphan process.
7. Start a second app instance against the same data dir. Second instance sees `ServiceError::AnotherInstanceRunning` and surfaces a clear error rather than racing on schema migrations.
8. Open a thread with a cached PDF attachment that has been indexed. Search for a phrase known to be in the PDF body. Result returns the message with a "match in attachment" annotation.
9. Trigger a re-index. Indexer runs in the Service, progress reported via notification (coalesced). UI remains responsive.
10. Submit a bulk action (200 threads). Action latency p99 stays within the budget; UI updates optimistically and rolls back per-operation as `OperationOutcome` notifications arrive.
