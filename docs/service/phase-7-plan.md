# The Service - Phase 7 Plan: attachment text extraction + Tantivy indexing

Companion to `phase-6a-plan.md`, `phase-6b-plan.md`, `phase-6c-plan.md`, and `phase-6d-plan.md`. Implements Phase 7 of `implementation-roadmap.md`.

> **Best-effort second draft.** Authored after Phase 6 (a/b/c/d) has fully landed and the post-Phase-6 review's discrepancies have closed. Phase 7 is the first phase since Phase 1.5 that is **not** a relocation - it adds a brand-new feature surface (attachment text extraction) layered on top of the now-Service-side Tantivy writer (relocated in Phase 3) and the Service-side flat attachment cache (`attachment_cache/<content_hash>`, landed in Phase 6b). Treat this as a structural skeleton; detailed extractor edge cases will be revised after the per-mime fixture corpus lands in 7-2.

## Revision history

**2026-05-07 - initial draft.** Captured the four locked-in design decisions from the planning Q&A: per-message Tantivy doc, deferred ExtractRuntime, SQLite text store keyed by content_hash, auto-rebuild on schema mismatch.

**2026-05-07 - post-implementation reconciliation (partial landing).** Phase 7 partially implemented in 11 commits (e2738ab7 -> e04eb34c). What follows reconciles the plan against what actually landed; sections below describe the original intent and are kept for context, but the per-7-N "Implementation order" section is the authoritative state-of-play.

**Landed:**

- **7-1**: schema (`02_mail.sql` v100 in place) + `attachment_extracted_text` table + `AttachmentCacheInfo.text_indexed_at` / `extraction_status` extension + `INDEX_SCHEMA_VERSION = 1` constant + `boot::check_schema_version_and_dispatch` stub.
- **7-2a**: `text_extract` module skeleton + dispatch + skip-list + char-boundary truncation + `plain.rs` (encoding_rs sniff + HTML stripper).
- **7-2b**: `pdf.rs` with `/Encrypt` head-inspection pre-flight + `pdf-extract` dispatch.
- **7-2c**: `ooxml.rs` covering `.docx` / `.xlsx` / `.pptx` with two-layer zip-bomb defense (claimed + actual decompressed-size cap).
- **7-3a**: 4 new Tantivy fields (`attachment_text` / `attachment_filename` / `attachment_mime` / `attachment_id`) + `INDEX_SCHEMA_VERSION` bump 1->2 + `SearchDocument.attachments: Vec<AttachmentDocFragment>` + `SearchResult.match_kind` / `also_matched` + boundary-padding (`ATTACHMENT_BOUNDARY_TOKEN` repeated 32x) + cross-attachment-phrase non-match verification test.
- **7-4a**: `ATTACHMENT_SWEEP_LOCK` relocated to `crates/service/src/attachment_lock.rs`.
- **7-4b**: extract IPC wire types (`ExtractStatusParams/Ack`, `RebuildPolicy`, `IndexRebuildParams/Ack`, 4 notifications with `service_generation`) + `ClientNotification::ExtractBackfillKick` + `RequestParams::ExtractStatus` / `IndexRebuild` + handler stubs + dispatch wiring + UI BootingApp/ReadyApp drop arms.
- **7-4c**: `ExtractRuntime` + worker + extraction pipeline (status-aware idempotency, `SWEEP_LOCK` read guard, `spawn_blocking` + `tokio::time::timeout(30s)`, persistence to `attachment_extracted_text`, `text_indexed_at` UPDATE) + panic supervisor + `Arc<Mutex<HashSet<content_hash>>>` enqueue dedupe + lifecycle unit tests.
- **7-4d** (partial in `8a63c127`, fully landed in post-7-8 follow-up): `BootSharedState.extract_runtime` slot + `install_extract_runtime` + `extract_runtime` + `take_extract_runtime` accessors. `BootSharedState.search_write` slot (single-use: `install_search_write` + `take_search_write`) so the post-ready producer can grab a `SearchWriteHandle` clone without storing it on `SyncRuntime` only. `spawn_post_ready_extract_startup` in `dispatch.rs` waits for `boot.ready`, opens a `BodyStoreReadState` against `app_data_dir`, constructs `ExtractRuntime`, installs it. `run_shutdown_drain` gains a step between sync drain and search-writer await that drains the runtime + clears the `search_write` slot. `ExtractRuntime::shutdown()` is now async, driven by a `CancellationToken` + stored worker `JoinHandle`.
- **7-5**: `attachment.fetch` cache-miss + cache-hit enqueue hooks (defensive no-op when runtime not installed) + `should_enqueue_extraction` status-aware gate.
- **7-7**: `ExtractRuntime` worker emits `WriterCommand::Index` after every successful extraction, with full `attachments: Vec<AttachmentDocFragment>` populated from DB at extraction time. Three new queries in `db::queries_extra::extract_reindex` (`find_message_ids_referencing_content_hash`, `select_messages_for_index_batch`, `select_attachment_fragments_batch`).
- **7-8**: `SearchReadState::enrich_match_kinds` per-field SnippetGenerator scoring + per-attachment attribution. `core::search_pipeline` batches DB attachment fragments and calls enrichment before grouping. `UnifiedSearchResult` + app-side `Thread` carry `match_kind` + `also_matched`. `thread_card` swaps the snippet slot to "in *<filename>*: <fragment>" for attachment matches.

**Dropped (design changed during implementation):**

- **`WriterCommand::ReindexByContentHash` variant** + writer-task apply-time DB enrichment (planned in 7-3). Concluded the writer-staleness race the variant guarded against is moot given sync's DB-write-before-Index ordering: by the time sync's `Index` command lands, the attachments table reflects sync's commit; ExtractRuntime reads canonical state when it builds Index commands. Provider crates emit thin docs with `attachments: Vec::new()`; ExtractRuntime emits `Index` commands with full attachment data populated from DB.

**Deferred / pending:**

- **7-6 (post-boot backfill)**, **7-9 (rebuild IPC + dual-index PreserveExisting)**, **7-10 (integration tests + architecture doc + roadmap LANDED)**: not yet implemented. With 7-4d's production producer now wired, the cache-miss/cache-hit enqueue hooks from 7-5 actively drive extractions, the per-message re-emit from 7-7 lands attachment text in the search index, and 7-8 surfaces "matched in *<filename>*" annotations - so the user-visible feature works end-to-end pending only manual verification.

**Plan-shape notes:**

- The plan's "Implementation order" treated each 7-N as one commit. Implementation sub-split 7-2 into a/b/c, 7-3 into 7-3a (no 7-3b landed since the writer-enrichment was dropped), and 7-4 into a/b/c/d. The bullets below mark each landed slice with its commit SHA.
- Workspace-wide `brokkr check` continues to trip the pre-existing `boot_progress_notifications_emitted_in_order` ignored-but-running hang. Per-package `brokkr check -p service` is clean across all landed commits.

**2026-05-07 - post-arch+bugs-review revision (large).** Two reviewers (claude on arch + bugs, codex on bugs) returned a sweep of correctness and policy findings. Major changes:

- **Per-message doc shape kept, but with per-attachment `add_text` + giant position increment.** The original "concatenate all attachments into one `add_text` separated by `\n\n`" allowed phrase queries to match across attachment boundaries (whitespace produces no tokens, so positions are continuous). Per-attachment `add_text` calls with `set_position_inc(usize::MAX)` between values suppress the cross-boundary match class. Attribution algorithm pinned: per-attachment `SnippetGenerator` reconstructs the matching attachment by scoring against each attachment's text segment, with explicit tiebreak rules.
- **Body+attachment co-match policy explicit.** `SearchResult` now carries `match_kind: MatchKind` (primary) plus `also_matched: Vec<MatchKind>` (secondary). UI can render "matched in body + report.pdf" rather than collapsing to one.
- **Central DB-backed reindex builder.** The original plan claimed `crates/sync/src/persistence.rs::index_search_documents` would JOIN `attachment_extracted_text` at construction time; that fn is a thin pass-through and `SearchDocument` is built in four provider crates with no DB handle. Phase 7-3 introduces a Service-side enrichment step on the writer-task side: providers continue to build their thin SearchDocument shells, the writer task enriches `attachment_*` fields by joining `attachment_extracted_text` at apply time using its `&ReadDbState`. Same enrichment is reused by the ExtractRuntime fan-out and the IndexRebuild path.
- **Status-aware idempotency.** Original "skip if row exists at current schema_version" permanently poisoned `bytes_gone` and `failed:transient` rows. New rule: skip only on `status='indexed'` or `status='skipped:<permanent>'`. `bytes_gone`, `failed:transient`, and `skipped:timeout` are eligible for retry on next enqueue. Backfill scan filters on `cached_at IS NOT NULL` (eviction nulls `cached_at`/`local_path`/`cache_size` but keeps `content_hash`), not just `content_hash IS NOT NULL`.
- **Auto-mismatch path: PreserveExisting via dual-index.** First-launch wipe is unacceptable on enterprise mailboxes (multi-minute-to-hours search-returns-nothing window). Schema mismatch on boot opens a parallel `search_index_next/` directory with the new schema and runs catch-up reindex into it; reads serve the legacy index until catch-up completes, then atomically swap. Explicit "Rebuild search index" palette command keeps the wipe path - the user asked for it. Phase 7-9 splits into two flavors: `RebuildPolicy::Wipe` (palette) vs `RebuildPolicy::PreserveExisting` (auto on schema mismatch).
- **Heap math rewritten.** `WRITER_HEAP_BYTES` 64 MB is a segment-build budget that auto-flushes on overflow, not an OOM gate. Real pressure is the `WriterCommand::Index` mpsc holding `Vec<SearchDocument>` payloads. Per-attachment text cap drops from 1 MB to 100 KB; mpsc batches chunked by total bytes (8 MB ceiling per command), not by message count.
- **Migration policy.** Pre-release: extend `02_mail.sql` v100 in place; no v101 migration row (`migrations.rs:65-70` is explicit).
- **Schema-wipe location split.** Wipe lives in a Service-only `boot::check_schema_version_and_wipe()` called inside `BootPhase::OpeningSearchIndex` before the writer spawn. `open_or_create_search_index` (used by both Service writer and UI reader) loses the wipe responsibility.
- **Wipe-version sentinel ordering pinned.** Sequence is: detect mismatch -> wipe -> `Index::create_in_dir` succeeds -> only then write `.version`. Crash mid-rebuild leaves a wiped-but-versionless directory, which the next boot treats as a fresh install and writes the current version. First-ever-boot (no `.version` file, no existing index) writes the current version without wiping.
- **Cancellation honesty.** `spawn_blocking` is uncancellable. Drain abandons in-flight extractions (worker thread continues until process exit); idempotent backfill resumes from the dropped work next boot. Drain budget is for queue-receiver drop + sender-drop awaits, not for in-flight extraction wait.
- **Panic supervisor.** `ExtractRuntime` mirrors `CalendarRuntime`'s `terminal_failure` notification on worker panic + `JoinError` from `spawn_blocking`. Generic dispatch panic-wrapper covers request handlers only; background runtimes need their own.
- **Dedupe at enqueue.** ExtractRuntime maintains an `Arc<Mutex<HashSet<String>>>` of in-flight `content_hash`es; enqueue checks-and-inserts; worker removes on completion. Suppresses the 4x-redundant-extract worst case for viral hashes.
- **`ATTACHMENT_SWEEP_LOCK` relocates.** Moves from `handlers/attachment.rs` (private static) to a new `crates/service/src/attachment_lock.rs` module with `pub(crate) static`. Both `handlers/attachment.rs` and `extract.rs` use it. Mechanical move, no behavior change.
- **`service_generation` on all new notifications.** `notification.rs:245` contract requires it; original wire types missed it. Added to `ExtractProgress`, `ExtractCompleted`, `IndexRebuildProgress`, `IndexRebuildCompleted`.
- **Backfill cadence event-driven, not always-on.** Drop the `Message::SyncTick` always-on fan-out. Triggers: (a) one shot on `boot.ready`, (b) hourly safety net (not 5-min). Cache-miss fetches enqueue directly via the handler; no kick needed for that path.
- **Extractor specs tightened.** Two distinct caps named separately (`MAX_INPUT_BYTES = 50 MB`, `MAX_EXTRACTED_TEXT_BYTES = 100 KB`); UTF-8 char-boundary truncation; PDF `/Encrypt` head-inspection pre-flight; OOXML decompressed-size cap + `quick-xml` entity-resolution explicitly off; `text/plain` encoding sniff via `encoding_rs`; `text/calendar` explicitly skipped (privacy); `text/html` attachment indexed knowing it likely duplicates body (v1 noise).
- **AttachmentCacheInfo extension.** Struct (`provider_sync_writes.rs:88-93`) gains `text_indexed_at` + `extraction_status` fields; the `find_attachment_cache_info` SELECT extends. Cache-hit re-enqueue path now compiles.
- **IndexRebuild lifecycle slot.** Rebuild runs as a tracked spawned task (CancellationToken-registered), not as the IPC handler's await. Drain aborts the token; partial progress survives across respawn via the same idempotency rules.
- **`local_drafts` included in IndexRebuild.** Drafts are search-visible today via SQL; rebuild reindexes both `messages` and `local_drafts`.
- **Encryption-at-rest stance documented.** `attachment_extracted_text.extracted_text` is plaintext SQLite TEXT. Consistent with `attachment_cache/` (plaintext on disk). Release notes flag the gap; future encryption work uses the existing `common::crypto` AES-256-GCM wrap. Not Phase 7 scope.
- **Dropped speculative surfaces.** `MatchKindFilter` plumbing (no UI in Phase 7 - CLAUDE.md no-speculative-features), "Subprocess extractors as a future contract" doc-comment (premature; revisit when an actual subprocess extractor lands).
- **Schema-bump cost honest.** Risks bullet rewritten: 50 GB / 70k-thread mailbox reindex is realistically tens of minutes (body-only) to hours (with attachment extraction); existing-mail search remains available throughout via the dual-index PreserveExisting path.

## Context

After Phase 6 closed, the workspace state relevant to Phase 7 is:

- **Tantivy writer** is Service-side (`crates/service/src/search_writer.rs`), driven via the `service-state::SearchWriteHandle` mpsc. Cadence: 1000 docs or 2 s, whichever first; `Index` / `Delete` / `Clear` / `FlushNow` commands; emits `Notification::IndexCommitted` per commit.
- **Attachment cache** is flat under `<app_data>/attachment_cache/<content_hash>` (Phase 6b). `attachment.fetch` IPC handler does cache-miss provider fetch + `<hash>.tmp` -> commit-row -> rename. `attachment.eviction_kick` handler runs an orphan-first + LRU sweep on the 5-min `SyncTick`.
- **Per-message Tantivy doc** has thirteen body / metadata fields (`subject`, `from_name`, `from_address`, `to_addresses`, `body_text`, `snippet`, `message_id`, `thread_id`, `account_id`, `date`, `is_read`, `is_starred`, `has_attachment`). No attachment-content fields.
- **`attachments` table** (`crates/db/src/db/schema/02_mail.sql`) carries `id`, `message_id`, `account_id`, `filename`, `mime_type`, `size`, `content_id`, `is_inline`, `local_path`, `cached_at`, `cache_size`, `content_hash`. Eviction nulls `local_path` / `cached_at` / `cache_size` but retains `content_hash`. No extraction state.
- **`AttachmentCacheInfo`** (`crates/db/src/db/queries_extra/provider_sync_writes.rs:88-93`) carries `id`, `content_hash`, `mime_type` only. Phase 7-1 extends.
- **`SearchDocument`** is built in four provider crates - `gmail/src/sync/storage.rs:308`, `jmap/src/sync/storage.rs:467`, `graph/src/sync/stores.rs:51`, `imap/src/sync_pipeline.rs:350` - none of which hold a `&ReadDbState`. `crates/sync/src/persistence.rs::index_search_documents` is a thin pass-through to `SearchWriteHandle::index_messages_batch`. Phase 7's attachment-field enrichment cannot land at the construction site without a much larger refactor; instead Phase 7-3 enriches at the writer-task apply step, where a `&ReadDbState` is already available.
- **Migration policy** (`crates/db/src/db/migrations.rs:65-70`): pre-release, schema changes extend the v100 migration in place. No new migration rows.
- **Notification contract** (`crates/service-api/src/notification.rs:245`): every state-changing notification must expose `service_generation` + `set_service_generation`. New variants must comply or post-respawn dispatch may surface stale notifications to a fresh UI.
- **`CalendarRuntime` panic supervisor** (`crates/service/src/calendar.rs:386`) is the canonical pattern for background-runtime panic-handling; `dispatch.rs::panic-wrapper` covers request handlers only.
- **No extraction infrastructure exists.** `crates/squeeze/` carries `lopdf = "0.40"` for compression but no read-side text extraction. There is no `text_extract` module anywhere.

The implementation-roadmap entry's intent: cached attachments get text-extracted via per-mime extractors, indexed into Tantivy alongside the message body, and search results disambiguate "matched in body" vs. "matched in attachment X." This is the "search beyond email body" forcing function for the Service architecture - the work is CPU-bound, must not block UI, and benefits from the writer-half lockdown that Phase 6 closed.

## Scope

### Entry criteria

- **Phase 6 fully landed.** `attachment.fetch` IPC + flat attachment cache + `attachment.eviction_kick` are in place; `WriteDbState` and the body / inline-image / search write halves are unreachable from `app/` at the Cargo direct-dep level.
- **`brokkr check` is clean** at the start of 7. The `boot_progress_notifications_emitted_in_order` test is expected to remain `#[ignore]` (Phase 8 carry-forward) and is unaffected by Phase 7 surfaces.
- **`pdf-extract`, `quick-xml`, `encoding_rs` workspace deps land in 7-2** (not pre-7); no entry-side dep work is required.

### In scope

#### Storage + schema

- **`attachment_extracted_text` table**, keyed by `content_hash`. One row per unique content; survives attachment-cache eviction. Schema in **§ Schema migration**. Extends the v100 migration in place per the pre-release policy.
- **`attachments.text_indexed_at INTEGER`** column. Per-row pointer to its extracted-text row's `extracted_at`; nullable. Backfill scan filters on `cached_at IS NOT NULL AND text_indexed_at IS NULL` so evicted-but-still-referenced rows don't churn the queue.
- **`AttachmentCacheInfo` extension**: add `text_indexed_at: Option<i64>` and `extraction_status: Option<String>` to the struct; extend the `find_attachment_cache_info` SELECT. Cache-hit path in `attachment.fetch` uses these to decide whether to enqueue.
- **Tantivy schema fields**: `attachment_text` (text indexed, not stored, multi-value with position-gap), `attachment_filename` (text indexed + stored, multi-value, ordered same as attachment_text), `attachment_mime` (string stored, multi-value, ordered same), `attachment_id` (string stored, multi-value, ordered same - lets attribution map a snippet position back to a specific attachment). Schema-version constant `INDEX_SCHEMA_VERSION = 2` in `crates/search/src/lib.rs`. Persisted at `<app_data>/search_index/.version`.
- **Encryption-at-rest stance.** `extracted_text` stores as plaintext SQLite TEXT - same posture as the `attachment_cache/` files on disk. Phase 7 release notes flag this for users storing text from contracts/medical/legal PDFs. Encrypting via the existing `common::crypto` AES-256-GCM wrap is a future-phase exercise; not in Phase 7 scope.
- **Auto schema-mismatch handling: dual-index PreserveExisting.** Service detects schema mismatch on boot. Opens `search_index_next/` adjacent to legacy `search_index/` with the new schema. ExtractRuntime + sync writes hit `next` only; reads continue against `legacy` until catch-up completes; rebuild background task drives messages + drafts into `next`. On catch-up, atomic swap (legacy -> archived, next -> primary), UI reader rebinds via `IndexCommitted`. Wipe-on-launch is **not** the v1 default. Explicit user-triggered "Rebuild search index" (palette) takes the wipe path - they asked for it.

#### Extractors

- **New module `crates/service/src/text_extract/{mod,pdf,ooxml,plain}.rs`.** Per-mime dispatch surface; pure functions (`fn extract(bytes: &[u8], mime: &str, filename: &str) -> ExtractionOutcome`). No I/O inside extractors; the runtime owns reading bytes from disk and persisting results.
- **Two distinct caps named separately**: `MAX_INPUT_BYTES = 50 MB` (skip-before-extract; bytes are never read into memory beyond this). `MAX_EXTRACTED_TEXT_BYTES = 100 KB` (post-extract; truncate to a UTF-8 char boundary via `floor_char_boundary`). The 1 MB output cap from the first draft was wrong by an order of magnitude given the mpsc heap-pressure analysis - 100 KB is the new ceiling.
- **PDF extractor** via `pdf-extract` crate. Pure Rust; runs inside `tokio::task::spawn_blocking`. **`/Encrypt` head-inspection pre-flight** before full extract: scan the first 4 KB for `/Encrypt` keyword; if present, return `Skipped { reason: Encrypted }` without ever calling `pdf-extract`. Some `pdf-extract` versions panic on encrypted PDFs; the pre-flight avoids the panic class entirely. Per-extraction wallclock cap 30 s (enforced via `tokio::time::timeout` over the spawn_blocking handle); timeout abandons the in-flight thread - same idempotent-retry contract as `bytes_gone`.
- **OOXML extractor** for `.docx` / `.xlsx` / `.pptx`. Inline implementation using the existing `zip` workspace dep + `quick-xml` (new). Walks `word/document.xml` (docx), `ppt/slides/*.xml` (pptx), `xl/sharedStrings.xml` + `xl/worksheets/*.xml` (xlsx); collects text content from `<w:t>` / `<a:t>` / `<si><t>` nodes. **Decompressed-size cap**: per-archive sum of decompressed sizes capped at `2 * MAX_INPUT_BYTES = 100 MB`; exceeding triggers `Skipped { reason: ZipBomb }`. **`quick-xml` entity-resolution explicitly off**: configure `Reader` to refuse DOCTYPE / external entities (the default-off behavior is documented as a load-bearing fact, not assumed). Skips tables-only / shape-only documents (no false-positive empty extracts).
- **Plain text extractor** for `text/plain`, `text/csv`, `text/markdown`. **Encoding sniffer via `encoding_rs`**: BOM-detect UTF-16/UTF-32; otherwise probe Windows-1252 / ISO-8859 / UTF-8 by validity ratio. Real-world `text/plain` arrives as UTF-8, UTF-16LE, Windows-1252, ISO-8859-* roughly evenly. Naive UTF-8-with-replacement skip-rates ~30% on Windows-1252 forwards.
- **HTML extractor** for `text/html`. Strips via `crates/common/src/html_sanitize.rs`. Will frequently duplicate the message body when the attachment IS the message body in `multipart/alternative`. v1 accepts the duplication as noise; deduplicating against `body_text` is a follow-up.
- **`text/calendar` (.ics) explicitly skipped.** iCalendar payloads expose attendee emails, organizer names, addresses - privacy-relevant. `Skipped { reason: PrivacyExempt }`. Documented in the dispatch table.
- **Skip lists.** By extension (`.exe`, `.dll`, `.zip`, `.tar`, `.gz`, `.7z`, `.mp3`, `.mp4`, `.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, `.heic`) and by mime (`application/x-executable`, `audio/*`, `video/*`, `image/*`). Skip-list match -> `Skipped { reason: OpaqueMime }` without bytes ever being read into memory.

#### Runtime

- **`ExtractRuntime`** (`crates/service/src/extract.rs`). Lifecycle surface mirrors `CalendarRuntime`: `closed: AtomicBool`, per-runtime semaphore (cap 4 concurrent extractions), mpsc work queue. Holds an `Arc<Mutex<HashSet<String>>> in_flight_hashes` for enqueue-dedupe. `enqueue(ExtractWork { content_hash, account_id, message_id, attachment_id })` is the only producer entry point.
- **Status-aware idempotency.** Pre-flight check at the worker (not at enqueue): skip if `attachment_extracted_text.status` is `'indexed'` OR `skipped:<permanent>` (where permanent = `OpaqueMime`, `UnknownMime`, `EmptyContent`, `OcrUnavailable`, `EncodingInvalid`, `PrivacyExempt`, `Encrypted`, `OversizeFile`). Eligible for retry: `failed:transient`, `bytes_gone`, `Timeout`, `ZipBomb`, no-row.
- **Worker body**: pop work item (releases enqueue-dedupe slot in `in_flight_hashes` only at completion, not at dequeue) -> acquire `attachment_lock::SWEEP_LOCK.read()` -> read bytes from `attachment_cache/<content_hash>` -> dispatch to mime-routed extractor inside `spawn_blocking` with `tokio::time::timeout(30s)` -> upsert `attachment_extracted_text` row -> set `attachments.text_indexed_at` for every row referencing this `content_hash` -> trigger fan-out re-index.
- **`ATTACHMENT_SWEEP_LOCK` relocates.** Moved from `handlers/attachment.rs` (private static) to a new `crates/service/src/attachment_lock.rs` with `pub(crate) static SWEEP_LOCK: tokio::sync::RwLock<()> = ...`. `handlers/attachment.rs` re-imports; `extract.rs` imports. Mechanical move, no behavior change. Lock still serializes evict-vs-fetch and now evict-vs-extract.
- **ENOENT semantics rephrased.** `SWEEP_LOCK.read()` is acquired *per worker dequeue*, not held across the queue lifetime. The window between `attachment.fetch` ack and the worker pulling its enqueued item is uncovered by the lock - eviction can run to completion in that window and unlink the bytes. The `bytes_gone` skip-reason exists for that ordering, not for an in-flight read race. With the lock held, ENOENT during `read()` is a genuine bug-class event (file unlinked under a held read guard); log and skip.
- **Re-index fan-out** (revised post-implementation). The original plan invoked a `WriterCommand::ReindexByContentHash` variant + writer-task apply-time DB enrichment; both were dropped during 7-3 (see Revision history > "Dropped"). Implemented shape: when ExtractRuntime's production producer lands per the 7-4d follow-up, the worker will emit existing `WriterCommand::Index` calls with `attachments: Vec<AttachmentDocFragment>` populated from DB at the time of extraction. The writer task is unchanged from Phase 3.
- **Byte-based mpsc batching.** `WriterCommand::Index` back-pressures on `COMMAND_QUEUE_CAPACITY = 256`. Sync's chunking continues at 100 messages per command; per-attachment extracted text is capped at `MAX_EXTRACTED_TEXT_BYTES = 100 KB` (truncated at the extractor in `text_extract::truncate_on_char_boundary`). Worst-case per-command payload: 100 messages * 4 attachments * 100 KB = 40 MB (absolute ceiling - typical is far less).
- **Drain order**: `PushRuntime -> CalendarRuntime -> SyncRuntime -> ExtractRuntime -> RebuildTask -> search-writer -> sentinel`. Extract drains after Sync because Sync's last batch may have queued attachments worth indexing; RebuildTask drains after Extract because rebuild can be the largest in-flight work-unit (a tracked spawned task carrying its own CancellationToken). search-writer drains after both because they hold `SearchWriteHandle` clones; only after every clone drops does the writer task exit, and only after that does the sentinel write.
- **Cancellation + drain budget honesty.** `spawn_blocking` is uncancellable. Drain abandons in-flight extractions: `ExtractRuntime::shutdown()` flips `closed`, drops the queue receiver, drops the semaphore. In-flight `spawn_blocking` threads complete on their own and write their results (which won't be observed - the runtime is shutting down). On next boot, the backfill scan re-discovers any unfinalized rows via the status-aware idempotency. Drain budget is **5 seconds for queue-receiver drop + sender drops**, not for waiting on in-flight extract threads.
- **Panic supervisor.** Worker task wraps the dispatch loop in `catch_unwind`; on panic, emits `Notification::ServiceTerminalFailure { reason: "extract_runtime_panic" }` (the existing terminal-failure channel from Phase 1) and exits. `JoinError` from `spawn_blocking` is also a panic class - same handling. Mirror of `crates/service/src/calendar.rs:386`.

#### IPC surface

- **`crates/service-api/src/extract.rs`** new module. Wire types (concrete shapes in **§ IPC wire types**):
  - `ExtractStatusParams` / `ExtractStatusAck` - polling read for status-bar summaries.
  - `IndexRebuildParams { policy: RebuildPolicy, force: bool }` / `IndexRebuildAck { rebuild_id: String }` - triggers rebuild. `RebuildPolicy::Wipe` (palette command) vs `RebuildPolicy::PreserveExisting` (auto on schema mismatch). `force: false` no-ops if a rebuild is in flight; `force: true` aborts and starts fresh.
  - Notifications all carry `service_generation: u32`: `ExtractProgress`, `ExtractCompleted`, `IndexRebuildProgress`, `IndexRebuildCompleted`.
- **Client kicks** (UI -> Service):
  - `extract.backfill_kick` (Drop class). **Not** fanned out from `Message::SyncTick` always-on. Triggered: (a) one-shot on `boot.ready` from the UI, (b) hourly safety-net subscription. Cache-miss fetches enqueue directly via the `attachment.fetch` handler; no kick needed for that path.
- **`Notification` enum** gains the four variants above; `production_notification_catalog` extends to cover them; method-name + class assignments per the existing pattern.
- **`RebuildTask` lifecycle**: not a request handler. The `index.rebuild` IPC handler immediately spawns a tracked `tokio::task` (registered with the Service's existing CancellationToken pattern), stores its `JoinHandle` in `BootSharedState`, and acks the IPC with the `rebuild_id`. The rebuild progresses asynchronously, emitting `IndexRebuildProgress` notifications. Drain aborts the token; partial state survives via idempotent reindex-by-content-hash.

#### UI integration

- **Search result disambiguation.** `SearchResult` grows two fields:
  - `match_kind: MatchKind` - the primary (highest-scoring) match.
  - `also_matched: Vec<MatchKind>` - secondary matches, ordered by score descending, deduped against `match_kind`.
  - `MatchKind` variants: `Body`, `Subject`, `From`, `Attachment { attachment_id, filename, mime, snippet }`.
- **Per-attachment attribution.** Service-side searcher per result: build per-text-field `SnippetGenerator`s; the `attachment_text` field's snippets get reconstructed against each attachment's individual extracted text. For each attachment in the message, run a per-attachment `SnippetGenerator` against just that attachment's text segment from `attachment_extracted_text`; the highest-scoring per-attachment generator wins. Tiebreak: highest term frequency in the matching segment, then alphabetical by `filename` for determinism.
- **Batched DB lookup for attribution.** Top-N query results -> single `SELECT a.id, a.filename, a.mime_type, a.content_hash, t.extracted_text FROM attachments a JOIN attachment_extracted_text t ON t.content_hash = a.content_hash WHERE a.message_id IN (...) AND a.account_id IN (...)`. One query per search call, not one per result.
- **Body+attachment co-match.** `match_kind` is the highest-scoring single field; `also_matched` lists everything else above a small threshold. UI renders "matched in body + report.pdf" by joining annotations.
- **Status-bar indicator.** During `ExtractProgress.remaining > 0` or `IndexRebuildProgress` in flight, status bar surfaces "Indexing N attachments" / "Rebuilding search index (X / Y)".
- **Palette command.** "Rebuild search index" entry -> `client.rebuild_index(RebuildPolicy::Wipe, force: false)`. Confirmation modal: "This will clear and rebuild the local search index from scratch. New mail searches will be unavailable for several minutes." Automatic mismatch path uses `RebuildPolicy::PreserveExisting` with no UI prompt - existing-mail search keeps working.
- **Search result rendering.** Result row shows: subject + sender + date as today, plus when `match_kind == Attachment { filename, ... }` (or `also_matched` contains an attachment) a second-line "matched in *<filename>*" annotation with the attachment's snippet (italicized, dimmed). Existing reading-pane jump-to-message behavior unchanged.

### Out of scope

- **OCR for scanned PDFs / images.** Image extractors return `Skipped { reason: OcrUnavailable }`. Adding an OCR backend (tesseract / cloud-OCR) is its own roadmap entry.
- **Language detection / per-language analyzers.** Tantivy's default tokenizer handles all-Latin-script extracted text adequately for v1. Per-language analyzers (CJK, Arabic, etc.) require schema fields per language and are deferred.
- **Attachment preview rendering.** Phase 7 is search-only; rendering a PDF in-app remains the existing `attachment.fetch` -> open-with-system-app flow.
- **Encrypted PDFs.** Detected and skipped via head-inspection pre-flight. Decryption requires a passphrase prompt UX that does not exist.
- **Calendar attachments** (CalDAV `ATTACH`, Microsoft `fileAttachment`, Google calendar attachments). Today the calendar action pipeline does not write these; Phase 7 leaves the surface alone. If the calendar pipeline starts caching attachments later, dispatch is purely mime-driven, so they extract automatically without code change.
- **`attachment.fetch` lease semantics** (Phase 1a forward-reference in architecture.md). Eviction-during-read uses today's `SWEEP_LOCK` model. Lease IDs land when the pack store does.
- **`attachment_extracted_text` orphan sweep.** When an attachment row's `content_hash` becomes nil (account deletion cascades the row away), the corresponding `attachment_extracted_text` row may become unreferenced. The cross-store invariant pass extension lands in Phase 8 alongside the existing optimization work; Phase 7 just adds a TODO comment in `startup_invariants.rs`.
- **Encryption-at-rest for `extracted_text`.** Plaintext SQLite TEXT in v1 (consistent with `attachment_cache/` plaintext files). Future encryption is a separate phase; Phase 7 release notes flag the gap.
- **Body+attachment dedup for HTML attachments.** v1 indexes both, accepts the duplicate scoring as noise.
- **`SearchParams.match_kind` filter** for "attachment-only" searches. Not plumbed - speculative API surface without a UI consumer (CLAUDE.md no-speculative-features). Lands when the UI for it lands.
- **`extract.cancel` IPC.** Rebuild cancellation goes through the existing CancellationToken on the rebuild task; per-attachment cancellation isn't needed (extraction is bounded by the 30 s wallclock).

## Architecture

### Pipeline

```
┌─────────────────┐                         ┌────────────────────────────┐
│ UI: open attach │                         │ boot.ready (one-shot)      │
└────────┬────────┘                         │ + hourly safety-net        │
         │ attachment.fetch IPC             └────────────┬───────────────┘
         ▼                                               │ extract.backfill_kick
┌──────────────────────────────────┐                     │
│ handlers/attachment.rs           │                     ▼
│  - cache hit:  bump cached_at    │         ┌─────────────────────────────┐
│                ack               │         │ handle_extract_backfill_kick│
│                if !text_indexed: │         │  SELECT id, content_hash,...│
│                  enqueue ─────┐  │         │  WHERE cached_at IS NOT NULL│
│  - cache miss: provider fetch │  │         │   AND text_indexed_at IS    │
│                stage .tmp     │  │         │       NULL LIMIT 1000       │
│                commit row     │  │         │  enqueue each ──────────┐   │
│                rename → final │  │         └─────────────────────────┼───┘
│                ack            │  │                                   │
│                enqueue ───────┼──┘                                   │
└───────────────────────────────┼──────────────────────────────────────┘
                                ▼
                  ┌──────────────────────────────────────────────────────┐
                  │  ExtractRuntime::enqueue                             │
                  │   - in_flight_hashes HashSet dedupes viral content   │
                  │   - bounded mpsc                                     │
                  └────────────┬─────────────────────────────────────────┘
                               ▼
                  ┌──────────────────────────────────────────────────────┐
                  │  ExtractRuntime worker (semaphore=4, panic supervised)│
                  │   - status-aware idempotency check                   │
                  │   - acquire attachment_lock::SWEEP_LOCK.read         │
                  │   - read attachment_cache/<hash>                     │
                  │   - spawn_blocking + tokio::time::timeout(30s)       │
                  │     - PDF: head-inspect /Encrypt; pdf-extract        │
                  │     - OOXML: zip + quick-xml (entities off)          │
                  │     - text: encoding_rs sniff                        │
                  │   - char-boundary truncate to MAX_EXTRACTED_TEXT_BYTES│
                  │   - upsert attachment_extracted_text                 │
                  │   - set attachments.text_indexed_at for content_hash │
                  │   - emit WriterCommand::ReindexByContentHash         │
                  │   - in_flight_hashes.remove                          │
                  └────────────┬─────────────────────────────────────────┘
                               ▼
                  ┌──────────────────────────────────────────────────────┐
                  │  search_writer.rs (extended)                         │
                  │   On ReindexByContentHash:                           │
                  │     SELECT message_ids referencing content_hash      │
                  │     For each, build full SearchDocument from current │
                  │     DB state (subject, from, body, attachment_*)     │
                  │     using &ReadDbState the writer now holds          │
                  │   delete_term(message_id) + add_document             │
                  │   cadence: 1000 docs / 2 s / FlushNow                │
                  │   emit Notification::IndexCommitted                  │
                  └──────────────────────────────────────────────────────┘
```

### Per-mime dispatch (`text_extract/mod.rs`)

```rust
pub enum ExtractionOutcome {
    Indexed { text: String },
    Skipped { reason: SkipReason },
    Failed { error: String },
}

pub enum SkipReason {
    OpaqueMime,        // image/audio/video/archive/executable - permanent
    Encrypted,         // PDF /Encrypt detected pre-flight - permanent
    OversizeFile,      // > MAX_INPUT_BYTES (50 MB) - permanent
    Timeout,           // > 30 s wallclock - retry-eligible
    EncodingInvalid,   // text/* with no detectable encoding - permanent
    EmptyContent,      // extractor produced no text - permanent
    OcrUnavailable,    // image with no OCR backend - permanent
    BytesGone,         // attachment_cache file ENOENT - retry-eligible
    UnknownMime,       // mime not in dispatch table - permanent
    PrivacyExempt,     // text/calendar - permanent
    ZipBomb,           // OOXML decompressed > 2*MAX_INPUT_BYTES - permanent
}

const MAX_INPUT_BYTES:          usize = 50  * 1024 * 1024;  // 50 MB
const MAX_EXTRACTED_TEXT_BYTES: usize = 100 * 1024;         // 100 KB

pub fn extract(bytes: &[u8], mime: &str, filename: &str) -> ExtractionOutcome {
    if is_opaque_by_mime_or_extension(mime, filename) {
        return ExtractionOutcome::Skipped { reason: SkipReason::OpaqueMime };
    }
    if bytes.len() > MAX_INPUT_BYTES {
        return ExtractionOutcome::Skipped { reason: SkipReason::OversizeFile };
    }
    let outcome = match canonicalize_mime(mime, filename) {
        Mime::Pdf       => pdf::extract(bytes),
        Mime::Docx      => ooxml::extract_docx(bytes),
        Mime::Xlsx      => ooxml::extract_xlsx(bytes),
        Mime::Pptx      => ooxml::extract_pptx(bytes),
        Mime::PlainText => plain::extract(bytes),
        Mime::Html      => plain::extract_html(bytes),
        Mime::Calendar  => ExtractionOutcome::Skipped { reason: SkipReason::PrivacyExempt },
        Mime::Unknown   => ExtractionOutcome::Skipped { reason: SkipReason::UnknownMime },
    };
    match outcome {
        ExtractionOutcome::Indexed { text } => ExtractionOutcome::Indexed {
            text: truncate_on_char_boundary(text, MAX_EXTRACTED_TEXT_BYTES),
        },
        other => other,
    }
}

fn truncate_on_char_boundary(mut text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes { return text; }
    let mut cut = max_bytes;
    while cut > 0 && !text.is_char_boundary(cut) { cut -= 1; }
    text.truncate(cut);
    text.push_str(" ... [truncated]");
    text
}
```

`canonicalize_mime` collapses provider-reported variants (`application/pdf`, `application/x-pdf`, missing-mime + `.pdf` extension, etc.) into a single enum tag.

### Schema migration

Extend `crates/db/src/db/schema/02_mail.sql` v100 in place. **No new migration row in `migrations.rs`.**

```sql
-- Append to crates/db/src/db/schema/02_mail.sql

-- Phase 7: per-attachment extraction state.
ALTER TABLE attachments ADD COLUMN text_indexed_at INTEGER;
CREATE INDEX IF NOT EXISTS idx_attachments_text_indexed_at
    ON attachments(text_indexed_at)
    WHERE cached_at IS NOT NULL AND text_indexed_at IS NULL;

CREATE TABLE IF NOT EXISTS attachment_extracted_text (
    content_hash    TEXT PRIMARY KEY,
    mime_type       TEXT,
    extracted_text  TEXT,
    status          TEXT NOT NULL,
    extracted_at    INTEGER NOT NULL,
    schema_version  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_attachment_extracted_text_schema_version
    ON attachment_extracted_text(schema_version);
```

**Status taxonomy** (string tags so future-extensible without enum migration):

- Permanent (no retry): `'indexed'`, `'skipped:opaque'`, `'skipped:encrypted'`, `'skipped:oversize'`, `'skipped:encoding'`, `'skipped:empty'`, `'skipped:ocr'`, `'skipped:unknown_mime'`, `'skipped:privacy'`, `'skipped:zipbomb'`.
- Retry-eligible: `'failed:transient'`, `'skipped:bytes_gone'`, `'skipped:timeout'`.

**Status-aware idempotency at worker pre-flight**: skip if row exists at current `schema_version` AND status is in the permanent set.

**`AttachmentCacheInfo` extension** in `crates/db/src/db/queries_extra/provider_sync_writes.rs`:

```rust
pub struct AttachmentCacheInfo {
    pub id:                 String,
    pub content_hash:       Option<String>,
    pub mime_type:          Option<String>,
    pub text_indexed_at:    Option<i64>,        // new
    pub extraction_status:  Option<String>,     // new
}
```

The `find_attachment_cache_info` SELECT extends to LEFT JOIN `attachment_extracted_text` on `content_hash` and projects the two new fields. Cache-hit path reads `extraction_status`; only enqueues if status is null or retry-eligible.

### Tantivy schema fields

```rust
// crates/search/src/lib.rs (post-7-3)

pub const INDEX_SCHEMA_VERSION: u32 = 2;

pub fn build_schema() -> Schema {
    let mut builder = Schema::builder();

    // ... existing 13 fields ...

    // Phase 7: attachment text indexing.
    // Multi-value with explicit position-gap to suppress cross-attachment phrase matches.
    builder.add_text_field("attachment_text",     text_indexed());          // searchable, not stored
    builder.add_text_field("attachment_filename", text_indexed_stored());   // searchable + stored, multi-value
    builder.add_text_field("attachment_mime",     STRING | STORED);         // string, multi-value
    builder.add_text_field("attachment_id",       STRING | STORED);         // string, multi-value, ordered = filename/mime/text

    builder.build()
}
```

**Per-attachment `add_text` with position-gap**, not concatenated. `build_search_doc` populates:

```rust
fn build_search_doc(fields: &Fields, msg: &SearchDocument) -> tantivy::TantivyDocument {
    let mut doc = tantivy::TantivyDocument::default();
    // ... existing fields ...

    // Phase 7: per-attachment add_text. Each call's positions start
    // POSITION_GAP higher than the previous to prevent phrase queries
    // from straddling attachment boundaries.
    //
    // Tantivy's per-value position increment for multi-value text fields
    // defaults to 1; we use TextOptions::set_position_inc to enforce a
    // gap of POSITION_GAP between values.
    const POSITION_GAP: u32 = 1_000_000;
    // (the default tokenizer caps position deltas; configure the
    //  field's position_inc via set_position_inc on the TextFieldIndexing.
    //  See § "Position-gap configuration" below for the exact API form.)

    for att in &msg.attachments {
        doc.add_text(fields.attachment_text,     &att.extracted_text);
        doc.add_text(fields.attachment_filename, &att.filename);
        doc.add_text(fields.attachment_mime,     &att.mime);
        doc.add_text(fields.attachment_id,       &att.attachment_id);
    }

    doc
}
```

`SearchDocument` grows a `Vec<AttachmentDocFragment>` instead of three parallel vecs (keeps the per-attachment fields naturally aligned):

```rust
pub struct AttachmentDocFragment {
    pub attachment_id:    String,
    pub filename:         String,
    pub mime:             String,
    pub extracted_text:   String,    // already truncated to MAX_EXTRACTED_TEXT_BYTES
}

pub struct SearchDocument {
    // ... existing fields ...
    pub attachments: Vec<AttachmentDocFragment>,
}
```

**Position-gap configuration**: Tantivy's `TextFieldIndexing` carries a position-increment-between-values setting (`set_index_option` controls token freqs/positions; the per-value gap is `IndexingPositions` default of 1). To enforce a giant gap, the field is configured at `build_schema` time with a custom `TextFieldIndexing` builder. If Tantivy's API doesn't expose this directly (some versions don't), the fallback is to inject a sentinel token between values via a custom tokenizer wrapper - documented as a 7-3 implementation detail for the developer to verify against the Tantivy version pinned by `crates/search/Cargo.toml`.

### Match-kind disambiguation

`SearchResult` grows two fields:

```rust
pub enum MatchKind {
    Body,
    Subject,
    From,
    Attachment {
        attachment_id: String,
        filename:      String,
        mime:          String,
        snippet:       String,
    },
}

pub struct SearchResult {
    // ... existing fields ...
    pub match_kind:   MatchKind,
    pub also_matched: Vec<MatchKind>,  // secondary matches above threshold, score-desc
}
```

The Service-side searcher (`crates/search/src/lib.rs::SearchReadState::search`):

1. Parses the query with `QueryParser` against all text fields.
2. Top-N collector returns scored docs.
3. **Single batched DB SELECT** for all top-N results' attachments + extracted text:
   ```sql
   SELECT a.id, a.message_id, a.filename, a.mime_type, a.content_hash,
          t.extracted_text
   FROM attachments a
   LEFT JOIN attachment_extracted_text t ON t.content_hash = a.content_hash
   WHERE a.message_id IN (...) AND a.account_id IN (...)
   ```
4. Per result, build per-text-field `SnippetGenerator`s.
5. **Per-attachment attribution**: for each attachment in this message, run a `SnippetGenerator` against just its `extracted_text` segment. Score the snippet. The highest-scoring per-attachment generator wins.
   - **Tiebreak**: highest term frequency in the segment, then alphabetical by `filename` for determinism.
6. Compute `match_kind` (highest-scoring single field across body / subject / from / per-attachment) and `also_matched` (every other field/attachment with a snippet score above a threshold, e.g. 50% of the top score), score-descending.

This algorithm gives the user "matched in *report.pdf*" with a real snippet, plus "+ body" if the body also matched, while preserving per-attachment correctness via the position gap (cross-attachment phrase matches don't occur at the index level, so no per-attachment snippet generator can score on a cross-attachment fake match).

### SearchDocument construction relocation

> **DROPPED during 7-3 implementation.** Reconsidered: the writer-staleness race this section guards against does not occur in practice because sync writes the attachments-table row to DB *before* sending the `Index` command, so any later read by ExtractRuntime sees canonical state. Provider crates were updated to emit thin docs with `attachments: Vec::new()` (7-3a), and ExtractRuntime (when its production producer lands per 7-4d follow-up) will emit `Index` commands with the full `attachments: Vec<AttachmentDocFragment>` populated from DB at the time of extraction. The writer task remains unchanged from Phase 3.

The original plan put attachment-field enrichment in `crates/sync/src/persistence.rs::index_search_documents`. That fn is a thin pass-through (`crates/sync/src/persistence.rs:59-72`); `SearchDocument` is built upstream in four provider crates that don't hold a `&ReadDbState`. Two viable shapes:

- **Option A (chosen)**: enrich at the **writer-task apply step**. `SearchWriteHandle` clones now require a `&ReadDbState` available to the writer task itself (already true post-Phase-3, since the writer task holds shared boot state). On `WriterCommand::Index { docs }`, the writer iterates docs, JOINs `attachment_extracted_text` for each `message_id`'s `content_hash`es, populates `attachments` field, then runs `delete_term + add_document`. Provider crates pass through unchanged thin docs.
- **Option B (rejected)**: relocate `SearchDocument` construction to a new central `crates/service/src/reindex_builder.rs` and refactor four provider crates to call it. Larger blast radius; doesn't buy correctness over Option A.

Option A's apply-time enrichment also enables the **writer-side staleness guard**: when extract and sync race on the same `message_id`, both go through the same DB-read-at-apply step, so the doc reflects current canonical state regardless of arrival order.

### Schema-wipe Service-only path

The original plan put wipe logic in `open_or_create_search_index` - which is also called by the UI's `SearchReadState::init` (`crates/search/src/lib.rs:291`). The UI must not destroy the writer's directory.

- **`crates/search/src/lib.rs::open_or_create_search_index`**: pure open. No wipe. No `.version` reads.
- **`crates/service/src/boot.rs::check_schema_version_and_dispatch`** (new, called inside `BootPhase::OpeningSearchIndex` *before* the writer spawn):
  1. Read `<search_index>/.version`.
  2. If absent: write current version. (First-ever boot; no wipe needed.)
  3. If present and matches current: no-op.
  4. If present and mismatches: invoke the dual-index PreserveExisting path. Open `<search_index_next>/`, persist its `.version`, register both indexes with the writer task (writes go to `next` only, reads in the UI continue against `legacy` until swap), spawn the rebuild task.
- **Sentinel-write ordering pinned**: only write `.version` *after* the corresponding `Index::create_in_dir` succeeds. Crash mid-rebuild leaves a half-built `next/` without a `.version`, which next boot treats as a fresh `next/` to recreate. No orphan-version-file failure mode.

UI reader (`SearchReadState`) opens against the primary `search_index/` only. After dual-index swap, the rebuild task writes a `Notification::IndexRebuildCompleted`; the UI rebinds its reader by calling `SearchReadState::init` afresh on the new primary directory.

### Writer-staleness guard

> **DROPPED during 7-3 implementation.** The race this section invokes does not occur in practice. Sync's `Index` command is built from in-memory parsed-message state and dispatched after the per-message DB rows are committed; ExtractRuntime reads DB state when it builds its own `Index` command. The two writers may both write the same `message_id` but with non-conflicting payloads (sync writes body fields with empty attachments; extract writes body fields read from DB plus populated attachments). Last-writer-wins is correct because the second write's body fields equal the first's. `WriterCommand::ReindexByContentHash` was not implemented; the existing `WriterCommand::Index` carries the full doc shape.

Two writers can target the same `message_id`: ExtractRuntime (after extraction completes) and Sync (when delta sync touches the message). Without coordination, last-writer-wins on `delete_term + add_document` produces a stale doc.

Apply-time DB enrichment (per § SearchDocument construction relocation, Option A) is the guard: regardless of which writer's command lands at the writer task, the writer reads canonical DB state at apply time, builds the doc fresh, and writes. The race becomes "two writers issue the same canonical doc," which is idempotent.

The contract that makes this work:
- ExtractRuntime emits **`WriterCommand::ReindexByContentHash { content_hash }`** (a *trigger*, not a payload).
- Sync emits `WriterCommand::Index { thin_docs }` (current path).
- The writer task, on either, derives the canonical doc from DB state.

This eliminates the need for generation tokens or version checks at the writer level.

## IPC wire types

```rust
// crates/service-api/src/extract.rs (new)

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ExtractStatusParams {}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ExtractStatusAck {
    pub queue_depth:    u64,
    pub indexed_total:  u64,
    pub skipped_total:  u64,
    pub failed_total:   u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub enum RebuildPolicy {
    /// Auto, on schema mismatch. Dual-index; legacy serves reads until catch-up.
    PreserveExisting,
    /// Explicit user-triggered (palette command). Wipe + rebuild.
    Wipe,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct IndexRebuildParams {
    pub policy: RebuildPolicy,
    pub force:  bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct IndexRebuildAck {
    pub rebuild_id: String,
}

// All notifications carry service_generation per the notification.rs:245 contract.

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ExtractProgress {
    pub service_generation:   u32,
    pub remaining:            u64,
    pub indexed_in_session:   u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ExtractCompleted {
    pub service_generation: u32,
    pub indexed:            u64,
    pub skipped:            u64,
    pub failed:             u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct IndexRebuildProgress {
    pub service_generation: u32,
    pub rebuild_id:         String,
    pub processed:          u64,
    pub total:              u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct IndexRebuildCompleted {
    pub service_generation: u32,
    pub rebuild_id:         String,
}
```

`Notification` enum gains four variants, each implementing `service_generation()` + `set_service_generation()`:

- `ExtractProgress(ExtractProgress)` -> `Coalesce { key: () }`, method `extract.progress`
- `ExtractCompleted(ExtractCompleted)` -> `MustDeliver`, method `extract.completed`
- `IndexRebuildProgress(IndexRebuildProgress)` -> `Coalesce { key: rebuild_id }`, method `index.rebuild_progress`
- `IndexRebuildCompleted(IndexRebuildCompleted)` -> `MustDeliver`, method `index.rebuild_completed`

`ClientNotification` (UI -> Service) extends with:

- `ExtractBackfillKick` -> `Drop`, method `extract.backfill_kick`

`RequestParams` enum extends with:

- `ExtractStatus { params: ExtractStatusParams }`
- `IndexRebuild { params: IndexRebuildParams }`

`production_notification_catalog` adds round-trip cases for all six new notification + request shapes.

## Implementation order

Follows the `phase 7-N:` commit-tagging convention prior phases use.

### phase 7-1: schema + AttachmentCacheInfo + version sentinel (LANDED `e2738ab7`)

- Extend `crates/db/src/db/schema/02_mail.sql` v100 in place: `attachments.text_indexed_at` column + index; `attachment_extracted_text` table + indexes. **No new migration row** in `migrations.rs`.
- Extend `AttachmentCacheInfo` (`crates/db/src/db/queries_extra/provider_sync_writes.rs`) with `text_indexed_at` + `extraction_status` fields. Update `find_attachment_cache_info` SELECT.
- Add `INDEX_SCHEMA_VERSION = 2` constant in `crates/search/src/lib.rs`.
- Add `crates/service/src/boot.rs::check_schema_version_and_dispatch` (stubbed - returns "no mismatch" until 7-9 implements the dual-index path; for 7-1 a mismatch just writes the new version).
- `open_or_create_search_index` is **untouched** (pure open).
- Test: schema migration runs cleanly on a v100 DB (confirms ALTER TABLE works); first-boot writes `.version`; second-boot is a no-op.

### phase 7-2: `text_extract` module + extractors + fixture corpus (sub-split, 7-2a/b/c LANDED)

**Sub-split during implementation:**

- **7-2a LANDED** (`6ceee751`): module skeleton (`mod.rs` types + dispatch + skip-list + char-boundary truncation), `plain.rs` (encoding_rs BOM detect + UTF-8/Windows-1252 fallback + control-char-ratio guard + HTML stripper). PDF/OOXML stubs return `Failed { error: "not yet wired" }` until 7-2b/c.
- **7-2b LANDED** (`4f67bba6`): `pdf.rs` with `/Encrypt` head-inspection pre-flight (scan first 64 KB + last 4 KB) + `pdf-extract` dispatch. Replaces 7-2a's PDF stub.
- **7-2c LANDED** (`fb1b9fe1`): `ooxml.rs` covering `.docx` / `.xlsx` / `.pptx`. Two-layer zip-bomb defense (claimed CD size cap + `Read::take(byte_budget)` per entry). `quick-xml` entity-resolution explicitly off.

In-tree fixtures kept minimal: synthetic byte literals + zip-built docs in tests rather than checked-in `.pdf` / `.docx` binaries. Real-world fixture corpus (per the original plan) deferred to 7-10's integration test cohort.

- New workspace deps: `pdf-extract`, `quick-xml`, `encoding_rs`, `zip` (if not already in workspace).
- `crates/service/src/text_extract/{mod,pdf,ooxml,plain}.rs`. Pure functions; no I/O; no async.
- Two-cap policy: `MAX_INPUT_BYTES = 50 MB`, `MAX_EXTRACTED_TEXT_BYTES = 100 KB`. Char-boundary truncation via `floor_char_boundary` semantics.
- PDF: `/Encrypt` head-inspection pre-flight; `pdf-extract` dispatch; pure-Rust.
- OOXML: `zip` open + `quick-xml` walk; entity-resolution explicitly off; per-archive decompressed-size cap = `2 * MAX_INPUT_BYTES`.
- Plain text: `encoding_rs` BOM + heuristic sniff; UTF-16/Windows-1252/ISO-8859-/UTF-8 coverage.
- Test fixtures under `crates/service/tests/fixtures/text_extract/`: valid PDF, encrypted PDF, oversized PDF (synthetic - never the 220 MB CLAUDE.md banned `5.pdf`), corrupt PDF, `.docx`, `.xlsx`, `.pptx`, malicious zip-bomb-shaped `.docx`, `.txt` (UTF-8), `.txt` (Windows-1252), `.csv`, `.html`, `.png` (skipped), `.zip` (skipped), `.ics` (skipped:privacy).
- Per-extractor unit tests assert outcomes for each fixture.
- No Service integration yet; module is pure-function callable from tests.

### phase 7-3: Tantivy schema fields + per-attachment `add_text` (7-3a LANDED `fca94930`)

**Sub-split during implementation:**

- **7-3a LANDED** (`fca94930`): four schema fields + resolver + boundary-padding (32 `rtskbnd` tokens between values; default Tantivy `POSITION_GAP=1` already blocks slop=0 phrase queries from straddling, so the padding is belt-and-suspenders against slop>=2). `INDEX_SCHEMA_VERSION` bump 1->2. `SearchDocument.attachments: Vec<AttachmentDocFragment>` replaces three parallel Option/Vec fields. `build_search_doc` emits per-attachment `add_text` calls in order with boundary padding inserted before all-but-the-first attachment_text value. `SearchResult.match_kind` / `also_matched` fields land with default `MatchKind::Body` / empty until 7-8. Provider crates updated to emit `attachments: Vec::new()`. Verification test (`phase_7_3_attachment_boundary_blocks_cross_attachment_phrase`) constructs a multi-attachment doc with text spanning the boundary; phrase query "brown fox" does NOT match while within-attachment phrases and single tokens do.

- **7-3b NOT LANDED (DROPPED)**: writer-task apply-time DB enrichment + `WriterCommand::ReindexByContentHash` variant. Reconsidered during 7-3a: the writer-staleness race is moot under sync's DB-write-before-Index ordering. Provider crates pass thin docs with empty `attachments`; ExtractRuntime in 7-7 (when wired) emits `WriterCommand::Index` with full attachment data populated from DB at extraction time.

### phase 7-4: `ExtractRuntime` + IPC wire types + drain integration + panic supervisor (4 sub-slices, 7-4a/b/c/d LANDED in full)

**Sub-split during implementation:**

- **7-4a LANDED** (`52a13d2a`): `ATTACHMENT_SWEEP_LOCK` relocated from `handlers/attachment.rs` (private `static`) to `crates/service/src/attachment_lock.rs` (`pub(crate) static SWEEP_LOCK`). Mechanical move; semantics preserved.
- **7-4b LANDED** (`772d170d`): `crates/service-api/src/extract.rs` wire types (with `service_generation` on every state-changing notification). `Notification` enum gains 4 variants + 2 `CoalesceKey` variants. `ClientNotification::ExtractBackfillKick` (Drop class). `RequestParams::ExtractStatus` + `RequestParams::IndexRebuild`. `production_notification_catalog` extends. Handler stubs in `crates/service/src/handlers/extract.rs` + dispatch wiring. UI BootingApp/ReadyApp drop arms exhaustive.
- **7-4c LANDED** (`a47e76a6`): `crates/service/src/extract.rs` with `ExtractRuntime` mirroring `CalendarRuntime` shape. `Arc<Mutex<HashSet<String>>> in_flight_hashes` for enqueue-dedupe. Worker dispatches to `text_extract::extract` via `spawn_blocking + tokio::time::timeout(30s)`; persists to `attachment_extracted_text`; UPDATEs `attachments.text_indexed_at`. Panic supervisor: per-item processing wrapped in `tokio::spawn` so JoinError captures panics; failed_count increments + finalize (no synthetic notification - the per-item completion granularity differs from CalendarRuntime's per-account run model). Cancellation honesty doc-comment in source. **Re-index emission deferred to 7-7** (worker updates DB state but does not yet emit `WriterCommand::Index` for affected messages). Two lifecycle unit tests cover enqueue-after-shutdown + concurrent-same-hash dedup.
- **7-4d LANDED partial** (`8a63c127`): `BootSharedState.extract_runtime` slot + `install_extract_runtime` + `extract_runtime` accessor scaffolded with `#[allow(dead_code)]`. **Production producer originally DEFERRED**: an earlier attempt at `spawn_post_ready_extract_startup` deadlocked `boot_ready_blocks_until_sequence_completes` and was reverted before the root cause was found.
- **7-4d revival LANDED** (post-7-8 follow-up): `spawn_post_ready_extract_startup` now constructs `ExtractRuntime` (with the 7-7-added `SearchWriteHandle` + `BodyStoreReadState` parameters) after `wait_for_ready` resolves and installs it on `BootSharedState`. The deadlock was real: the post-ready spawn needs a `SearchWriteHandle` clone to construct the runtime, but if that clone lived in a `BootSharedState` slot that nothing dropped during shutdown, `search_writer_handle.await` blocked forever on the orphan. Fix: the `search_write` slot is single-use - `take_search_write` consumes it from inside `spawn_post_ready_extract_startup` on success and is also called defensively from `run_shutdown_drain` before the search-writer await, so no `SearchWriteHandle` clone leaks past the writer-task EOF observation. ExtractRuntime gained a `CancellationToken` + stored worker `JoinHandle`; `shutdown()` is now `async`, cancels the token, awaits the worker. Drain step added in `run_shutdown_drain` between sync drain and search-writer await.

**Drain order** is now `Push -> Calendar -> Sync -> Extract -> search-writer -> sentinel`. Rebuild slot still pending (7-9).

### phase 7-5: `attachment.fetch` cache-miss + cache-hit hooks (LANDED `e04eb34c`)

- `handle_fetch` enqueues on cache-miss (after `commit_cached_tmp`) and on cache-hit when `info.extraction_status` is null OR retry-eligible. `should_enqueue_extraction(status: Option<&str>)` is the gate; it pins the schema's status taxonomy.
- `enqueue_extraction_if_runtime_installed` is a defensive no-op when `boot_state.extract_runtime()` returns None - relevant until the 7-4d production producer is revived.
- Worker's status-aware idempotency check inside the worker (per 7-4c) covers DB-level dedup as belt-and-suspenders.
- Tests deferred to 7-10's integration suite: the unit-test surface for handle_fetch is small and the runtime is not constructed in tests today.

### phase 7-6: post-boot backfill (event-driven) + UI fan-out (LANDED)

- `find_unindexed_cached_attachments` in `db::queries_extra::extract_reindex` - SELECT against the partial `idx_attachments_text_indexed_at` index, returns `(attachment_id, message_id, account_id, content_hash)` tuples up to a caller-supplied limit. Two unit tests (filter correctness + limit respect).
- `handle_backfill_kick` in `service::handlers::extract` resolves the installed `ExtractRuntime` via `boot_state.extract_runtime()`, runs the query against `boot_state.db_conn()`, and enqueues each row whose `content_hash` is `Some`. NULL hash rows are skipped (the worker can't extract without one). Defensive no-op when the runtime is not yet installed (boot still in progress) or has been taken (drain in progress).
- `crates/app/src/handlers/provider.rs::kick_extract_backfill` mirrors `kick_calendar_sync`'s shape - sends `ClientNotification::ExtractBackfillKick` and discards the result.
- `crates/app/src/subscription.rs` adds a 1-hour `iced::time::every` ticker that emits `Message::ExtractBackfillTick`, which `update.rs` forwards to `kick_extract_backfill`. The same kick is fired once from the `Message::ServiceBootReady` arm to catch up after a Service crash mid-extraction.
- Idempotency comes from three layers: the SELECT returns 0 rows after the backlog drains, the runtime's `in_flight_hashes` dedupe rejects duplicates while extraction is in progress, and the worker's status-aware skip handles already-extracted rows. Drop class - missed kicks self-heal on the next hour.
- Handler integration tests deferred to 7-10's `extract_in_process.rs` cohort (same setup as the other planned end-to-end tests).

### phase 7-7: re-index propagation (handler-side wiring complete)

- Apply-time DB enrichment for `WriterCommand::Index` and `WriterCommand::ReindexByContentHash` lands in 7-3. Phase 7-7 wires up the actual fan-out trigger.
- ExtractRuntime worker, on successful extraction, emits `WriterCommand::ReindexByContentHash { content_hash }` to its `SearchWriteHandle` clone.
- Writer task already (via 7-3) handles the variant: SELECTs `message_id`s referencing the hash, dedupes, builds canonical docs from DB, runs `delete_term + add_doc`.
- `ExtractProgress` notification fired per drained item; `ExtractCompleted` fired when queue empties (all `in_flight_hashes` clear).
- Test: extracting one attachment of a message with three attachments produces a Tantivy doc whose `attachment_text` has only the indexed one's text segment and aligned `attachment_filename` / `attachment_mime` / `attachment_id` arrays; re-extracting another produces a doc with both attachments aligned in order.

### phase 7-8: search-result disambiguation + per-attachment attribution

- `SearchReadState::search`: top-N results -> single batched DB SELECT for all results' attachments + extracted text -> per-result `match_kind` + `also_matched` computation.
- Per-attachment `SnippetGenerator` runs against each attachment's segment; tiebreak by term frequency, then filename alphabetical.
- `also_matched` populated with secondary fields/attachments above 50%-of-top-score threshold, score-descending.
- UI search-result rendering surfaces `match_kind` + `also_matched`; attachment annotations are second-line italicized snippets with filename.
- Tests: phrase-match in a fixture PDF returns `MatchKind::Attachment { filename: "<fixture>.pdf", ... }`; phrase that matches both body and attachment returns body as primary + attachment in `also_matched` (or vice versa, score-dependent); cross-attachment phrase fake-match (positions span boundary) does NOT match (position-gap working).

### phase 7-9: `index.rebuild` IPC + palette command + schema-version dispatch (Wipe-only; PreserveExisting deferred)

**Sub-slices:**

- **7-9a LANDED**: handler skeleton + Wipe path + tracked-task lifecycle.
  - `handlers/extract.rs::handle_rebuild` mints a UUIDv4 rebuild_id, rejects concurrent rebuilds via `boot_state.rebuild_in_flight_id()` (or pre-empts when `force=true` by cancel + abort of the previous handle), resolves runtime deps (db_conn, search_write, body_read, notification_sender), spawns the task, installs `RebuildTaskState` (rebuild_id + JoinHandle + CancellationToken) on `BootSharedState`, and acks the IPC with the rebuild_id.
  - `service::rebuild::run_wipe_rebuild` sends `WriterCommand::Clear`, runs `reset_extracted_text_for_rebuild` (UPDATE `attachments.text_indexed_at = NULL` + DELETE `attachment_extracted_text`), iterates every message via `select_all_message_ids_for_rebuild` in 200-row chunks, builds `SearchDocument`s via `select_messages_for_index_batch` + `select_attachment_fragments_batch` + `body_read.get_batch` in parallel, sends `WriterCommand::Index` per chunk, emits `IndexRebuildProgress` per chunk (Coalesce by rebuild_id), emits `IndexRebuildCompleted` (MustDeliver), then triggers `extract.backfill_kick` so attachment text re-extracts.
  - Cancellation respected between chunks; on cancel the task exits without `IndexRebuildCompleted` and the next boot can re-trigger.
  - Drain step in `run_shutdown_drain` between sync drain and search-writer await: `take_rebuild_task` -> cancel -> abort -> await; defensively clears `search_write` + `out_tx` slots.
  - `local_drafts` re-emit deferred to a follow-up.
- **7-9c LANDED**: schema-version mismatch dispatcher.
  - `check_schema_version_and_dispatch` (stub in 7-1) now sets a `pending_schema_rebuild` flag on `BootSharedState` when the persisted `.version` differs from `INDEX_SCHEMA_VERSION`. The post-ready dispatcher (`spawn_post_ready_schema_rebuild`) clears the flag, calls `handle_rebuild` with `RebuildPolicy::Wipe`, polls `rebuild_in_flight_id` until clear, then writes the current `.version`. Sentinel-write ordering is preserved: on a mid-rebuild crash, the OLD `.version` stays on disk and the next boot re-fires.
  - v1 only handles additive schema changes (new fields). Tantivy can open the existing index against a superset schema; the rebuild backfills the new fields. Non-additive bumps would require deleting the index dir during boot pre-writer-spawn; documented as a future need.
- **7-9d LANDED**: palette command + IPC client methods.
  - `cmdk::CommandId::AppRebuildSearchIndex` ("Rebuild Search Index", category App). Dispatch routes to `Message::RebuildSearchIndex` -> `handlers::provider::dispatch_rebuild_search_index` -> `client.rebuild_index(Wipe, force=false)`.
  - `ServiceClient::rebuild_index` and `extract_status` IPC methods.
  - `IndexRebuildProgress` / `IndexRebuildCompleted` notification arms in `update.rs` log progress; visual status-bar surface deferred to a follow-up (would store `Option<RebuildProgressState>` on `ReadyApp` and render in `status_bar.rs`).

**DEFERRED**: true PreserveExisting (search-stays-live during rebuild). The plan called for opening `search_index_next/` adjacent with a parallel writer + atomic directory swap + UI reader rebind. The honest scope is a substantial rework of how `SearchWriteHandle` is plumbed through `SyncRuntime` (currently moved at construction, not consulted via `boot_state`). v1 ships the simpler "search briefly unavailable while Wipe rebuild runs" UX; status bar progress softens the gap. Re-introduce when the index size + rebuild duration become user-pain.

**DROPPED** from the original plan: the `RebuildAlreadyInFlight` `ServiceError` variant. The handler returns `ServiceError::Internal` with a clear message instead - cheaper to add and the error surface is internal to the IPC error pipeline anyway.

Tests: handler-level unit tests + integration tests deferred to 7-10's `extract_in_process.rs` cohort. The DB-side `select_all_message_ids_for_rebuild` and `reset_extracted_text_for_rebuild` lack dedicated tests; the existing extract_reindex test schema covers the relevant tables and the queries are short.

### phase 7-10: integration tests, manual matrix, architecture doc

- Integration tests in `crates/service/tests/extract_in_process.rs`:
  - End-to-end: seed a message + a PDF attachment fixture; trigger fetch; await `ExtractCompleted`; query Tantivy for a phrase known to be in the PDF; assert `MatchKind::Attachment { ... }`.
  - Eviction-during-extract: hold the read lock; trigger eviction-kick concurrently; assert serialization (no torn reads, no double-indexes).
  - Schema-version mismatch dual-index: write fake `.version`; restart Service; assert legacy serves reads while `next/` populates; swap completes; UI reader rebinds.
  - `extract.backfill_kick`: seed N unindexed cached attachments; fire kick; assert all enqueue and complete; second kick is no-op; evicted rows skipped.
  - Status-aware idempotency: seed a `bytes_gone` row; re-enqueue same hash; assert worker re-extracts. Seed a `skipped:opaque` row; re-enqueue; assert worker skips.
  - Cross-attachment phrase non-match: seed a message with two attachments where text "foo" ends one and "bar" begins the next; query "foo bar"; assert no match.
  - Body+attachment co-match: seed; query; assert `match_kind` + `also_matched` populated correctly.
  - Rebuild cancellation: trigger `Wipe` rebuild; mid-flight, drain Service; restart; assert backfill resumes.
- Manual matrix entries (`docs/service/manual-test-matrix.md`):
  - "Open a PDF whose contents include a known phrase. Wait for status bar to clear. Search the phrase. Verify result row carries 'matched in *<filename>*' annotation."
  - "Trigger 'Rebuild search index' from palette. Verify confirmation modal warns about temporary unavailability. Verify status bar shows progress; verify search returns expected results post-rebuild."
  - "Schema-mismatch path: simulate a schema-version bump. Verify search remains functional throughout rebuild (PreserveExisting); verify status bar shows progress; verify `IndexRebuildCompleted` arrives; verify reader rebinds."
- `docs/architecture.md` updates:
  - Add a "Text extraction pipeline" paragraph alongside the "Action service as mutation gate" + "Calendar action pipeline" paragraphs.
  - Note the per-attachment `add_text` + position-gap shape and the apply-time DB enrichment as the v1 contract.
  - Bump the "Service-side write surfaces" paragraph to mention `attachment_extracted_text` as a Service-only writer.
  - Add the dual-index PreserveExisting pattern to the "Settled patterns" section.
- `docs/service/implementation-roadmap.md` Phase 7 entry: change status from "future" to "LANDED" with a per-7-N retrospective bullet list (matches the Phase 5 / 6c / 6d post-landing structure).

## Critical files

Modified or created in Phase 7. Lean on existing patterns where possible.

- `crates/db/src/db/schema/02_mail.sql` - extend v100 in place; new column + new table (7-1). **No `migrations.rs` change.**
- `crates/db/src/db/queries_extra/provider_sync_writes.rs` - extend `AttachmentCacheInfo` + `find_attachment_cache_info` (7-1).
- `crates/db/src/db/queries_extra/...` - new queries: `find_unindexed_cached_attachments`, `upsert_attachment_extracted_text`, `set_text_indexed_at_for_content_hash`, `find_message_ids_referencing_content_hash`, `find_attachments_with_extracted_text_batch` (7-1, 7-7, 7-8).
- `crates/search/src/lib.rs` - new fields, `INDEX_SCHEMA_VERSION`, `Fields::from_schema` extension, `MatchKind` + `SearchResult.also_matched` extension, per-field snippet generation, position-gap configuration (7-1, 7-3, 7-8).
- `crates/service/src/text_extract/{mod,pdf,ooxml,plain}.rs` - new (7-2). `crates/service/tests/fixtures/text_extract/` - fixture corpus (7-2).
- `crates/service/src/attachment_lock.rs` - new module hosting relocated `SWEEP_LOCK` (7-4).
- `crates/service/src/extract.rs` - `ExtractRuntime` + panic supervisor (7-4).
- `crates/service/src/handlers/extract.rs` - request handlers + backfill kick (7-4, 7-6, 7-9).
- ~~`crates/service/src/search_writer.rs` - extended with `ReindexByContentHash` apply path; holds `&ReadDbState`; apply-time enrichment (7-3).~~ **DROPPED post-7-3a**: writer-task enrichment shape was reconsidered; ExtractRuntime emits `WriterCommand::Index` directly with full attachment data, writer task is unchanged from Phase 3.
- ~~`crates/service-state/src/search_write.rs` - `WriterCommand::ReindexByContentHash` variant (7-3).~~ **DROPPED post-7-3a**: variant not added; existing `WriterCommand::Index` carries full doc shape.
- `crates/service-api/src/extract.rs` - wire types with `service_generation` (7-4).
- `crates/service-api/src/{notification,request,client_notification}.rs` - new variants (7-4).
- `crates/service-api/src/lib.rs` - re-exports.
- `crates/service/src/handlers/attachment.rs` - cache-miss + cache-hit enqueue; relocate `SWEEP_LOCK` import (7-4, 7-5).
- `crates/service/src/boot.rs` - `check_schema_version_and_dispatch`; `ExtractRuntime` + `RebuildTask` slots (7-1, 7-4, 7-9).
- `crates/service/src/lifecycle.rs` - drain order extension (7-4).
- `crates/service/src/dispatch.rs` - request dispatch arms (7-4).
- `crates/app/src/service_client.rs` - `rebuild_index`, `extract_status`, four new `Notification::*` arms (7-4, 7-9).
- `crates/app/src/subscription.rs` - `extract.backfill_kick` boot.ready + hourly subscription (7-6).
- `crates/app/src/handlers/...` - palette command + status-bar integration (7-9).
- `crates/app/src/ui/...` - search result row rendering with `also_matched` annotations (7-8).
- `crates/service/src/startup_invariants.rs` - TODO comment for `attachment_extracted_text` orphan sweep (Phase 8 carry-forward).
- `docs/architecture.md`, `docs/service/manual-test-matrix.md`, `docs/service/implementation-roadmap.md` (7-10).

## Reused patterns

- **Runtime lifecycle** mirrors `crates/service/src/calendar.rs::CalendarRuntime` (closed: AtomicBool, semaphore, mpsc); panic supervisor mirrors `calendar.rs:386`.
- **Drain integration** mirrors the Phase 5 sequence in `lifecycle::run_drain`. Insert point: between `Sync` and `search-writer`.
- **Coalesce / MustDeliver / Drop notification classes** mirror the existing `notification.rs` pattern; per-class behavior is the reader-task `try_send` / `await send` split that Phase 1 established. **`service_generation` on every state-changing notification** is the contract from `notification.rs:245`.
- **Search writer task** (`SearchWriteHandle`) is the only path that writes into Tantivy; ExtractRuntime fans through it. Apply-time DB enrichment in the writer eliminates the writer-staleness race class.
- **Tracked-task lifecycle for IndexRebuild** mirrors how Phase 6c handled long-running calendar bulk operations (CancellationToken-registered spawned task, not request handler).
- **Wire-type module split** mirrors `service-api/src/{cal_action,attachment,oauth}.rs` from Phase 6.
- **Migration-policy** (extend v100 in place, no new rows) per `migrations.rs:65-70`.

## Testing strategy

### Unit tests (per extractor)

- `text_extract::pdf::extract`, `ooxml::extract_*`, `plain::extract` against the fixture corpus.
- Specific assertions: encrypted-PDF skipped pre-flight (no `pdf-extract` invocation); zip-bomb skipped at decompressed-cap; UTF-16 plain text decoded correctly via `encoding_rs`; char-boundary truncation does not panic on multi-byte input near the cap.

### Lifecycle tests (`ExtractRuntime`)

- Enqueue after shutdown returns Err.
- Shutdown safe on empty runtime.
- Drop of last sender exits worker cleanly.
- Panic in worker emits `ServiceTerminalFailure`.
- In-flight `spawn_blocking` extraction abandoned on drain (no deadlock waiting on uncancellable thread).
- `in_flight_hashes` dedupes concurrent enqueues of same hash.

### Wire / catalog tests

- `production_notification_catalog` round-trips `ExtractProgress`, `ExtractCompleted`, `IndexRebuildProgress`, `IndexRebuildCompleted`, `ExtractBackfillKick`.
- Method-name + class assignments on each new variant.
- `service_generation` field round-trips on every state-changing variant; mismatch is detected and stale notifications are dropped.

### Integration tests (`extract_in_process.rs`)

- End-to-end: fetch -> extract -> re-index -> search match-kind annotation.
- Status-aware idempotency: re-enqueue with permanent-skipped row is no-op; re-enqueue with retry-eligible row re-extracts.
- Eviction-during-extract: lock serializes; bytes_gone path on race-without-lock window.
- Schema mismatch dual-index: legacy serves reads, swap completes, UI rebinds.
- Cross-attachment phrase non-match (position gap working).
- Body+attachment co-match populates `also_matched`.
- Backfill kick: cached-but-unindexed enqueue; evicted-but-with-content_hash skipped.
- Rebuild cancel: drain mid-rebuild; restart; backfill resumes.

### Manual matrix updates

- "Open then search" PDF round trip.
- "Rebuild search index" palette flow.
- Schema-mismatch PreserveExisting flow (search remains live).
- Eviction-during-search persistence: search match for evicted attachment still returns the result (text in `attachment_extracted_text` survives eviction).

## Risks / open questions

- **PDF extractor quality.** `pdf-extract` is incomplete; some PDFs (ICC color profiles, custom fonts, CMap-heavy CJK) extract empty text or garbled output. Skip-rate is hard to predict pre-corpus. Ship v1 with `pdf-extract` and accept misses; if the fixture corpus shows skip-rate > 30%, pivot to `pdfium-render` (binary dep) in a follow-up. The roadmap entry calls this out as TBD.
- **OOXML format coverage.** Real-world `.docx` files include comments, footnotes, headers/footers, tracked changes, embedded objects. v1 extracts `<w:t>` body text only. Document the limitation; iterate post-7. Same for `.xlsx` - we extract sharedStrings + cell text but not formulas-as-strings or named ranges.
- **Tantivy position-gap API.** The plan assumes Tantivy's `TextFieldIndexing` exposes a per-value position-increment knob. If the pinned version doesn't (need to verify against `crates/search/Cargo.toml`'s tantivy version), the fallback is a sentinel-token tokenizer wrapper - documented as a 7-3 verification step. If neither path works, the cross-attachment phrase-match class returns; mitigation would be per-attachment subdocs (significantly larger refactor). **This is the highest-impact verification step in 7-3.**
- **Heap pressure on viral content.** A content_hash referenced by 1000 messages, post-extraction, fans out 1000 doc rebuilds via `WriterCommand::ReindexByContentHash`. The writer's apply-time DB enrichment chunks this; mpsc backpressures naturally on 256-cap. Worst-case writer-task lag during viral-fan-out: a few seconds. `WRITER_HEAP_BYTES = 64 MB` stays as-is; the writer's segment-build budget auto-flushes on overflow. The original "256 MB bump for OOM avoidance" was wrong direction.
- **Schema-bump cost.** A 50 GB / 70k-thread / ~300k-message mailbox: body-only reindex at Tantivy's typical 5-15k docs/sec is ~30-60 minutes; with attachment extraction (4 workers, 30 s p95 per PDF, 100k cached attachments) is ~10 hours. The dual-index PreserveExisting path keeps existing-mail search live throughout - the user-visible cost is the rebuild progress indicator, not search downtime. Mitigation works only if Tantivy's per-value position-gap is available; if it isn't and we fall back to wipe, this becomes a hard-cost UX issue.
- **Encryption-at-rest gap.** `attachment_extracted_text.extracted_text` is plaintext SQLite TEXT. Same posture as `attachment_cache/` plaintext on disk. Acceptable for v1 (consistent with existing cache); release notes should flag for users storing extracted text from sensitive PDFs (contracts, medical records, legal). Future encryption work via `common::crypto` AES-256-GCM is a separate phase.
- **`attachment_extracted_text` orphan disk-leak between Phase 7 and Phase 8 invariant pass.** Worst case: 100 KB per orphan content_hash. Typical 1000-msg/day mailbox with 5% attachment turnover = ~150 orphans/year = ~15 MB/year accumulation. Acceptable until Phase 8.
- **`extract.backfill_kick` thundering herd post-respawn.** Service crash mid-extraction; on next boot, the boot.ready kick finds every still-unindexed row and enqueues. ExtractRuntime semaphore (cap 4) bounds the burst; mpsc grows by N. 1000-row LIMIT per kick caps the burst per boot; subsequent hourly kicks chip away. A 100k-attachment mailbox catches up over ~100 kicks (~100 hours at hourly cadence) - acceptable steady-state behavior, not a ceiling concern.
- **Subprocess extractors.** All Phase 7 extractors are pure Rust + `spawn_blocking`. No subprocess concerns. The "future contract" doc-comment was dropped from this draft as premature; re-introduce when an actual subprocess extractor lands.

## Phase 7 carry-forwards (anticipated, not load-bearing)

These are deferred at landing time and feed into Phase 8 / future phases:

- **Cross-store invariant pass extension** for `attachment_extracted_text` orphans (Phase 8).
- **Per-language analyzers** for non-Latin-script attachments (post-7).
- **OCR backend** for image attachments + scanned PDFs (post-7, separate roadmap).
- **HTML attachment dedup** against message body (post-7 cleanup).
- **Encrypted `extracted_text`** via `common::crypto` (separate encryption-at-rest phase).
- **`extract.cancel`** IPC for user-cancellable extraction (deferred; rebuild cancellation already covered via drain).
- **Per-attachment subdocs** as a Tantivy-shape pivot if position-gap verification in 7-3 fails (large refactor, only if needed).

## Verification

End-to-end manual flow:

1. Run `brokkr check` - clean.
2. Run `crates/app/seed-db.py` to seed dev DB, then `cargo run -p app --features dev-seed`.
3. Wait for status bar to clear.
4. Open a PDF attachment in the seeded mailbox (the dev-seed corpus needs at least one PDF; add one to `dev-seed.toml` if absent).
5. Search for a phrase known to be in that PDF.
6. Verify the result row shows "matched in *<filename>*" annotation with a snippet.
7. Trigger schema-mismatch by manually editing `<app_data>/search_index/.version` to a different value, restart Service. Verify search remains functional during rebuild (PreserveExisting); verify reader rebinds on `IndexRebuildCompleted`.
8. From the command palette, run "Rebuild search index"; confirm the modal. Verify the wipe path produces a temporary search-unavailable window with progress; verify search returns expected results post-rebuild.
9. Run the integration-test cohort: `brokkr test -p service extract_in_process` - all passing.
10. Run lifecycle tests: `brokkr test -p service extract_runtime` - all passing.
11. Run the per-extractor unit tests: `brokkr check -p service` covers them in the test sweep.
12. Visually inspect the manual matrix entries (added in 7-10) and walk the `docs/service/manual-test-matrix.md` Phase 7 section.
