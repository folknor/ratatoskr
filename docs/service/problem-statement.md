# The Service (Subprocess Worker)

## Overview

Ratatoskr runs as **two cooperating processes**: a UI process (the iced app), and a child worker process called **the Service**. The Service owns long-running, CPU-heavy, or write-side work that has no business competing with the UI runtime - sync, the action mutation gate, the Tantivy writer, blob-store writes, attachment text extraction, the retry queue. The UI owns rendering, user input, and read-side queries against shared on-disk state (Tantivy index, blob store, SQLite).

The Service is a **child process of the UI** - spawned at app start, killed when the UI exits. It is *not* a system daemon; there is no autostart, no tray-resident lifetime, no surviving the app being closed. That promotion is possible later but explicitly out of scope for v1.

## Why this matters

Three things are forcing this decision right now, with attachment content indexing as the immediate trigger:

1. **Full-text indexing of attachments is non-negotiable for an enterprise client.** PDFs, Office documents, plaintext - users need to search inside attached files, not just message bodies. Text extraction is CPU-heavy (multi-second for large PDFs); doing it on the iced runtime would make the UI unusable during sync.
2. **Re-indexing 5+ years of mail is a multi-hour batch job.** It must not crash with a UI hang, and it must not freeze the UI for hours while it runs. Today, both happen.
3. **Sync, push, and the retry queue already want to be off the UI runtime.** They live there today by accident, not by design. Every long-running async task in the app currently shares a runtime with rendering, and the symptoms (lag, jank, dropped frames during heavy sync) are visible.

The blob-store work for attachments (see `docs/attachments/problem-statement.md`) was the proximate trigger for this conversation, but the architectural decision applies to everything write-side and CPU-heavy in the app.

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
                              tantivy_index/
                              app_data/...
```

**Key invariant: the Service is the only writer.** UI never writes to SQLite, the blob store, or the Tantivy index. Reads go through the same in-process code paths the UI already uses. Writes go through IPC to the Service.

## What the UI owns

- All rendering, all iced runtime tasks.
- All user input.
- Tantivy *readers* (search queries are fast, in-process, no IPC).
- Blob store *readers* (positional reads against immutable pack files; OS page cache does the heavy lifting).
- Direct SQLite reads for everything the UI already queries (navigation, thread lists, message bodies, etc.). SQLite supports many concurrent readers + one writer; the writer happens to live in the Service.
- Settings UI (which calls into the Service for any state it needs to write).
- Pop-out windows.

## What the Service owns

- The Action service (mutation gate). Existing in-process abstraction; relocates with no shape change.
- All sync paths (JMAP, Gmail, Graph, IMAP). Pre-fetch of attachments runs here.
- The Tantivy writer + the Tantivy commit cadence.
- All blob-store writes (`PackStore::put`, `unref`, `evict_lru`, `gc`).
- Attachment text extraction (`extract_text(blob_hash, mime_type) -> Option<String>`) and the indexing pipeline that follows.
- Push notification receivers and the JMAP push channel.
- The retry queue (`db_pending_ops_*`).
- The action mutation log.
- Any periodic background work (cache GC, stale-cache cleanup, contact-photo refreshes, etc.).

## Lifecycle

**Start.** UI launches; UI spawns the Service as a child process via `tokio::process::Command` (or the equivalent platform spawn). UI passes the app data directory and runtime config via command-line args. Service initializes its handles to SQLite / blob store / Tantivy writer, then announces readiness via a stdio handshake. UI proceeds with first paint once the handshake completes.

**Health.** Service heartbeats every N seconds. UI tears down + respawns if heartbeats stop (Service died, hung, deadlocked). Re-respawn has a backoff to avoid crashloops.

**Shutdown.** UI quit -> UI sends `Shutdown` request -> Service flushes Tantivy writer + closes pack files cleanly -> Service exits -> UI exits. If Service doesn't ack within a timeout, UI sends SIGTERM, then SIGKILL. Worst case: torn writes detected on next start by the recover paths each subsystem already has (or will have).

**Crash.** If the UI crashes, the Service is orphaned. Two options: (a) Service exits when its parent dies (SIGHUP / `prctl(PR_SET_PDEATHSIG)` on Linux, equivalent watchdog on macOS/Windows), (b) Service stays alive and the next UI launch reattaches. Option (a) is simpler and matches v1 scope (no surviving UI exit anyway); pick (a).

**No persistent process.** Quit the app, the Service is gone. Restart the app, both come back. This is deliberate v1 simplicity.

## IPC

**Transport.** JSON-RPC 2.0 over stdio. Newline-delimited JSON per message. UI writes to Service's stdin; Service writes to UI's stdin (which the UI reads as the Service's stdout).

**Why stdio + JSON-RPC.** Universally available, no socket files / port allocation / firewall concerns, well-supported by Rust crates (`jsonrpsee`, hand-rolled is also fine for this scope), debuggable by piping to a file. Schema can grow without protocol changes.

**Why not gRPC / Cap'n Proto / shared memory.** All add machinery we don't need at this scope. The hot path is intentionally not crossing the IPC boundary - reads are direct against on-disk state. Writes are infrequent enough (sync per N seconds, action per user click) that JSON serialization cost is invisible.

**Message shape (sketch, settled in implementation):**

- `Request { id, method, params }` -> `Response { id, result | error }`. Synchronous-feeling, fits the action service / settings write surface.
- `Notification { method, params }` for one-way streams (sync progress, push events, indexer progress). UI dispatches as iced messages.

**Surface scope.** Start small: `health.ping`, `sync.start_account`, `sync.cancel_account`, `action.dispatch`, `attachment.fetch`, `index.rebuild`, plus a `notification` channel for sync progress, push events, action completion. Grow per phase as call sites move.

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

## Cross-cutting impact

This decision touches a lot. Listed for visibility, addressed in detail in the implementation roadmap:

- **Sync architecture.** All four provider sync paths move from "tokio task in the UI process" to "tokio task in the Service process." `Sync` IPC notifications replace today's direct `Message::SyncComplete` dispatch.
- **Search.** Tantivy writer relocates; readers stay where they are. Attachment text extraction is wired in as a new pipeline step.
- **Action service.** Relocates wholesale. The IPC surface for actions is the bulk of the new RPC schema.
- **Push notifications.** JMAP push subscriber moves into the Service. Push events become Service-to-UI notifications instead of in-process channel sends.
- **Attachments.** All write-side operations (`BlobStore::put`, `unref`, `evict_lru`, `gc`) become Service-internal. Read-side stays UI-direct against on-disk pack files. The attachment problem statement's Phase 1a (`PackStore` library) is unchanged; Phases 1b/2 (orchestration + sync wiring) effectively move into Service phases.
- **Tests.** Most existing tests are in-process and work unchanged. New IPC-boundary tests need their own harness (spawn a Service, talk to it, verify behavior).
- **Crash recovery.** Each subsystem owned by the Service needs a recovery story for "Service died mid-write." Most already do (SQLite WAL, append-only pack files with crash-safe recovery, Tantivy commit semantics) - this is mostly verification rather than new work.

## Verification

End-to-end behavior to test once the v1 phases land:

1. Start the app. Both processes are visible in `ps` / Activity Monitor / Task Manager. UI is parent, Service is child.
2. Issue a search. Result returns from UI-side Tantivy reader; Service is not involved on the read path.
3. Trigger a sync. Service does the work; UI gets sync progress notifications. UI rendering remains responsive throughout.
4. Send a kill signal to the Service mid-sync. UI detects the lost heartbeat, respawns the Service, sync resumes from the last checkpoint.
5. Quit the UI. Service receives shutdown request, flushes Tantivy writer + closes pack files, exits cleanly. No orphan process in `ps`.
6. SIGKILL the UI. Service detects parent death (PR_SET_PDEATHSIG / equivalent), exits within seconds. No orphan process.
7. Open a thread with a cached PDF attachment that has been indexed. Search for a phrase known to be in the PDF body. Result returns the message with a "match in attachment" annotation.
8. Trigger a re-index. Indexer runs in the Service, progress reported via notification. UI remains responsive.
