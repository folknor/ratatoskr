# Phase 7 discrepancies

Findings from the 2026-05-07 multi-archetype review (claude + codex × security/bugs/perf/arch). Items already acknowledged in `phase-7-plan.md` § "Phase 7 known-gaps" or as Phase 8 carry-forwards in `implementation-roadmap.md` are not repeated here. Severity reflects user-visible damage; "agreement" indicates how many reviewer sessions independently surfaced the finding (out of 8) - higher is higher-confidence.

## Critical

## High

### H1. Drain race with `spawn_post_ready_extract_startup`; per-item spawn tasks not tracked

**Files:** `crates/service/src/dispatch.rs:1079` (search_write clone, not take), `:382-419` (drain order), `:490` (extract_startup_handle.abort - too late), `crates/service/src/extract.rs:217-289` (worker + per-item tokio::spawn).

Two separate problems with the same drain.

**Drain race against post-ready spawn:** the plan's 7-4d revival retro claims the fix was making the `search_write` slot single-use - `take_search_write` consumed from inside the spawn on success. The implementation calls `boot_state.search_write()` (clone), not `take_search_write`. The slot stays populated; the spawn holds a separate `SearchWriteHandle` clone in a local until `ExtractRuntime::new` consumes it. Race window: drain runs between the spawn cloning `search_write` and `install_extract_runtime`. Drain's `take_extract_runtime` returns None, drain runs `take_search_write` clearing the slot, drain awaits `search_writer_handle.await`. Concurrently the spawn finishes constructing `ExtractRuntime` and installs it. Drain is past `take_extract_runtime`; never re-takes. Search-writer can't observe EOF because the runtime's `Arc<Inner>` holds a clone. `extract_startup_handle.abort()` lives at `:490`, after the writer await - never reached. Hang.

**Per-item tasks not awaited at shutdown:** `run_worker` spawns each work item as `tokio::spawn` (`extract.rs:258`) and does not retain handles. `ExtractRuntime::shutdown()` cancels the worker's `tokio::select!` and awaits only `worker_handle`. Per-item tasks each hold an `Arc<ExtractRuntimeInner>` containing both `search_write` and `notification_tx`. After cancellation: worker exits without joining per-item tasks; per-item tasks continue running, holding live `SearchWriteHandle` clones; `take_search_write` clears the boot-state slot but per-item tasks each still hold a clone via the inner; `search_writer_handle.await` blocks on those clones until each per-item task rides out its full 30 s `spawn_blocking` timeout. Drain blocks for up to ~30 s × min(in-flight items, semaphore cap=4).

**Agreement: 4/8** (claude arch, claude perf, codex bugs, codex perf).

**Fix:** (a) `take_search_write` from inside `spawn_post_ready_extract_startup` so the slot becomes a one-time consume on success. (b) Track per-item handles in a `JoinSet` on `Inner`; `shutdown()` aborts and awaits them after cancelling the worker. Or move per-item spawn into the worker's loop with internal concurrency via JoinSet so the worker handle's await is the single drain point.

### H2. `fan_out_reindex` doesn't chunk; viral content_hash blows the writer mpsc

**Files:** `crates/service/src/extract.rs:559-651`.

`fan_out_reindex` for a content_hash referenced by N messages: `find_message_ids_referencing_content_hash` returns N pairs; the follow-up SELECTs and `body_read.get_batch` materialize all rows in memory; one `WriterCommand::Index { docs: Vec<SearchDocument> }` is sent. For N=1000 (the plan's "viral content" scenario at `phase-7-plan.md:773`) × 50 KB body × 100 KB extracted_text per attachment, the single command carries ~150 MB through the mpsc - well past the writer's 64 MB heap budget. `COMMAND_QUEUE_CAPACITY=256` backpressure does not help because each one-attachment fan-out is one command, not chunked. `rebuild::rebuild_chunk` already chunks at `REBUILD_CHUNK_SIZE = 200` (`rebuild.rs:45`); `fan_out_reindex` should mirror that.

**Agreement: 6/8** (claude security, claude perf, claude arch, codex bugs, codex perf, codex arch).

**Fix:** chunk `pairs` at 200 and emit one `WriterCommand::Index` per chunk.

### H3. OOXML decompressed-bytes accounting tracks output text, not actual decompressed input

**Files:** `crates/service/src/text_extract/ooxml.rs:83, 100-106, 140-148, 226-240`.

`budget = budget.saturating_sub(text.len() as u64)` decrements against extracted text (post-XML walk), but `Read::take(limit)` caps actual decompressed bytes per entry. A malicious xlsx/pptx with N entries whose central directory honestly claims small sizes (passing `open_with_size_check`) can decompress up to `MAX_TOTAL_DECOMPRESSED ≈ 100 MB` per entry whenever the XML is long but `<t>` text is sparse. Total decompression work is bounded only by entry count × 100 MB (CPU/IO DoS).

Compounding: `cap_hit && limit < MAX_TOTAL_DECOMPRESSED` only fires the ZipBomb signal when the budget had already been reduced. The first entry's `limit == MAX_TOTAL_DECOMPRESSED` short-circuits the check, so a single-entry bomb declaring exactly 100 MB and inflating to 100 MB passes both pre-check and read-cap.

**Agreement: 7/8** (claude security, claude bugs, claude perf, claude arch, codex bugs, codex perf, codex arch).

**Fix:** subtract `buf.len()` from budget instead of `text.len()`; tighten the cap-hit check to `cap_hit && (buf.len() as u64) >= limit` regardless of whether `limit < MAX_TOTAL_DECOMPRESSED`.

### H4. OOXML CD-size sum can overflow u64 in release mode

**Files:** `crates/service/src/text_extract/ooxml.rs:190-201`.

```rust
let claimed_total: u64 = (0..archive.len())
    .filter_map(|i| { ... f.size() })
    .sum();
```

`<u64 as Sum>::sum` uses checked `+` in debug (panics) and wrapping `+` in release. A crafted OOXML with N entries each declaring `u64::MAX/2 + 1` wraps to a small total and passes the cap check. In debug, the worker panics into the per-item supervisor (counted as transient failure). In release, the bypass is silent. `Read::take` still bounds memory, but the first-line defense the plan promised at `phase-7-plan.md:107` is bypassable.

**Agreement: 1/8** (claude arch).

**Fix:** manual fold with `checked_add`, or short-circuit when any single declared size > `MAX_TOTAL_DECOMPRESSED`.

### H5. OOXML pre-flight clones the archive per entry: O(n²) memory churn

**Files:** `crates/service/src/text_extract/ooxml.rs:191-196`.

`(0..archive.len()).filter_map(|i| { let mut clone_archive = archive.clone(); clone_archive.by_index(i).ok().map(|f| f.size()) })` clones the whole `ZipArchive` once per entry. Each clone copies central-directory metadata. For an adversarial OOXML with hundreds of thousands of entries, this is O(n²) bytes of cumulative allocations before extraction starts. DoS vector.

**Agreement: 2/8** (claude perf, claude arch).

**Fix:** iterate `&mut archive` directly: `for i in 0..archive.len() { archive.by_index(i)?.size() }`.

### H6. `spawn_blocking` thread leak under malicious payloads

**Files:** `crates/service/src/extract.rs:217-231` (shutdown), `:253` (per-item spawn), `:458-482` (timeout wrapping).

`run_extraction_pipeline` wraps `spawn_blocking` in `tokio::time::timeout(30s)`. On timeout, the JoinHandle drops but the blocking-pool thread keeps running with the up-to-50-MB `bytes` Vec moved into it. `pdf-extract` and `quick-xml` are not interrupt-safe: a crafted PDF with pathological CMap/font tables or a docx with deeply-nested groups can keep the blocking thread CPU-pegged for minutes. The worker's semaphore is released on timeout, so new work spawns more blocking threads - bounded only by tokio's default 512-thread blocking-pool ceiling. Memory: 4 simultaneous in-flight + N abandoned-but-running × up to 50 MB bytes each. The doc-comment at `extract.rs:209-216` honestly notes "thread continues to completion ... result is discarded" but doesn't quantify the ceiling.

**Agreement: 4/8** (claude bugs, claude security, codex bugs, codex perf).

**Fix surface:** hard input-size cap on PDF specifically (lower than 50 MB), or run the extractor in a child process with a SIGKILL deadline (the only correct answer for adversarial inputs). Tighter `WORKER_CONCURRENCY` lower-bound + per-class soft limit is a stop-gap.

### H7. PDF `/Encrypt` pre-flight bypassable (multiple paths)

**Files:** `crates/service/src/text_extract/pdf.rs:69-93`.

Three independent bypass paths:

1. **Hex-escape encoding:** PDF name tokens permit `/#45ncrypt`, parsed by any conformant PDF as `/Encrypt`. Literal byte search misses it. A malicious PDF references the encryption dict via `<< /#45ncrypt 1 0 R >>` and the pre-flight is blind.
2. **Mid-file XRef streams (PDF 1.5+):** the trailer + xref live inside FlateDecode-compressed object streams. Byte scan reads raw stored bytes; `/Encrypt` references inside compressed streams are invisible. Linearized PDFs and incremental-update PDFs commonly place the catalog mid-file.
3. **Head/tail gap:** `HEAD_SCAN_BYTES = 64 KB` + `TAIL_SCAN_BYTES = 4 KB`. A PDF where `/Encrypt` lives between offset 64 KB and EOF-4 KB escapes pre-flight. Also: files in the 64-68 KB range have a coverage gap.

When the pre-flight fails to detect, `pdf-extract` is invoked on encrypted bytes. The worker's panic supervisor catches resulting panics, but the outcome records as `failed:transient` (retry-eligible) rather than `skipped:encrypted` (permanent). The same encrypted file then re-runs every backfill kick.

**Agreement: 5/8** (claude security, claude bugs, claude perf, codex bugs, codex perf).

**Fix:** double the scan windows (cheap mitigation for path 3); for paths 1 and 2, parse the PDF trailer with a real PDF tokenizer rather than byte scan, or accept that `pdf-extract` itself must reject encrypted input. Permanent-vs-retry classification: when extraction fails on a file the pre-flight failed to flag, prefer the permanent classification (caller of `pdf::extract` can pattern-match on the underlying `pdf-extract` error string).

### H8. `service_generation` hardcoded to 0 in every Phase 7 notification source

**Files:** `crates/service/src/dispatch.rs:1105` (extract), `crates/service/src/handlers/extract.rs:105` (rebuild).

`ExtractRuntime::new(..., notification_tx, 0)` and `run_wipe_rebuild(..., notification_tx, 0)` pass literal `0`. Every `ExtractProgress` / `ExtractCompleted` / `IndexRebuildProgress` / `IndexRebuildCompleted` notification carries `service_generation: 0`. The plan's `notification.rs:245` contract for stale-notification rejection is decoratively present but operationally inert for these four variants - the UI's per-incarnation drop logic never fires.

**Agreement: 1/8** (claude security).

**Fix:** thread `boot_state.service_generation()` (or whatever the actual accessor is) into both spawn sites.

## Medium

### M2. App-layer attribution passes empty `body_text`

**Files:** `crates/core/src/search_pipeline.rs:223, 274`, `crates/search/src/lib.rs:795`.

The pipeline intentionally passes empty `body_text` into attribution inputs. Search scoring supports body scoring, but the app never supplies it, so body-only / body-plus-attachment co-match cases skew toward attachment attribution: a result that matched body text gets labeled as attachment when the attachment also contains the query, and `also_matched` cannot include `Body`. Diverges from the documented Phase 7 "matched in body + also_matched: [Attachment]" intent.

**Agreement: 3/8** (codex bugs, codex perf, codex arch).

**Fix:** thread the actual body text through `core::search_pipeline` into attribution. The body store already holds it; the lookup is one batched read.

### M3. `also_matched` threshold uses `div_ceil(2)` not `/2`

**Files:** `crates/search/src/lib.rs:892`.

`let threshold = top_score.div_ceil(2).max(1);` rounds up. For top_score=3 → threshold=2 (66%); top_score=5 → threshold=3 (60%). Plan documents 50%. Only even top_scores honor it; odd top_scores tighten the threshold and drop more candidates.

**Agreement: 1/8** (claude bugs).

**Fix:** `top_score / 2` (integer division floors) or `(top_score as f64 * 0.5) as u64`.

### M4. `MatchKind::Body` shown as default for non-text matches

**Files:** `crates/search/src/lib.rs:541-548, 755-759, 802-803, 807-810, 886`.

Two distinct cases:

1. **Snippet-field-only matches.** Free-text parser includes the `snippet` field. A document matched solely on `snippet` reaches `enrich_match_kinds`, where `build_field_snippet_gen` is only invoked for `body_text / subject / from_name / attachment_text`. All four field scores come back zero, the `let-else` continues, and the result keeps default `MatchKind::Body`. UI renders "matched in body" for results with no body match.
2. **Filter-only / no-text searches** (e.g. `is:starred`). When `free_text` is empty, `enrich_match_kinds` returns Ok(()) without rewriting `match_kind`. Default left as `Body`. Same UI lie.

**Agreement: 2/8** (claude bugs, claude perf).

**Fix:** include `snippet` in the per-field SnippetGenerator pool, drop `snippet` from the parser, or introduce `MatchKind::FilterOnly` / `Option<MatchKind>` for the no-text case. Most precise: introduce `Option<MatchKind>` so the UI can render no annotation when attribution couldn't determine one.

### M5. First-enqueue-wins on filename/mime poisons hash-shared attachments

**Files:** `crates/service/src/extract.rs:382-409`, `crates/service/src/text_extract/mod.rs:194-220`.

`process_one`'s metadata fetch looks up filename + mime by the *specific* `(account_id, message_id, attachment_id)` tuple from the work item. If that exact attachment row was deleted between enqueue and dequeue (account.delete, message expire, sync purge), `meta` is `None`, falling through to `("", "")`. `canonicalize_mime("","")` finds no mapping → `Mime::Unknown` → `Skipped { UnknownMime }` → permanent row. The same content_hash may be referenced by N other live attachments with valid filenames and mime types; none of them ever get extracted because the worker pre-flight short-circuits on the permanent row.

**Agreement: 1/8** (claude bugs).

**Fix:** query metadata as `SELECT filename, mime_type FROM attachments WHERE content_hash = ?1 AND filename != '' LIMIT 1` instead of pinning to the specific tuple. Hash-collision-aware naming is fine; first-enqueuer naming is not.

### M6. Index-after-Delete race during message removal

**Files:** `crates/service/src/extract.rs:559-651`, `crates/service/src/search_writer.rs`.

Racy sequence: extract reads `select_messages_for_index_batch` → returns msg. Concurrently sync deletes msg + sends `WriterCommand::Delete { ids: [msg] }`. Sync's Delete arrives first (mpsc FIFO). Extract emits `WriterCommand::Index { docs: [msg] }`. Writer applies Index → `delete_term` (no-op) + `add_document` → re-creates the deleted doc. The plan's "DB-write-before-Index" invariant doesn't address this: the DB write happens, the Index command happens AFTER its own DB read with stale-but-valid content. Writer FIFO doesn't enforce DB-vs-Index ordering across runtimes.

**Agreement: 1/8** (claude bugs).

**Fix:** at apply time, re-read DB and skip-if-deleted (overlaps with C2's writer-task DB enrichment). Or per-message generation counter that the writer rejects when older than the last Delete.

### M7. Manual palette rebuild beats schema rebuild → `.version` skipped

**Files:** `crates/service/src/dispatch.rs:1149-1154`, `crates/service/src/handlers/extract.rs:48-52`.

If the user invokes `Rebuild Search Index` after `boot.ready` but before `spawn_post_ready_schema_rebuild` calls `handle_rebuild`, the slot is occupied. The post-ready task's `handle_rebuild` returns `Err("index.rebuild already in flight ...")`. The dispatcher logs and returns *without* writing `.version`. `pending_schema_rebuild` was already swapped to false. User's manual rebuild completes successfully; no code path writes `.version` for the rest of this boot. Next boot re-marks the rebuild and runs it redundantly.

Self-healing on next boot but wasteful (one redundant full-mailbox rebuild). Folds into the C4 fix once `.version` is gated on success.

**Agreement: 1/8** (claude bugs).

### M8. Phase 7 writes shared DB tables directly from `service`, bypassing the `db` crate ownership rule

**Files:** `crates/service/src/extract.rs:352, 504, 535` (inline SQL upsert + status pre-flight + UPDATE).

Architecture rule (`docs/architecture.md` § "Shared-table SQL belongs to db"): shared tables are owned by `db`, not by `service`/`core`/provider crates. `attachment_extracted_text` upsert, status pre-flight, and `attachments.text_indexed_at` UPDATE are inline SQL in `extract.rs`. Should be `db::queries_extra` functions called by the service.

**Agreement: 1/8** (codex arch).

**Fix:** relocate to `crates/db/src/db/queries_extra/extract_reindex.rs` (which already hosts `find_unindexed_cached_attachments`, etc.).

### M9. `text/calendar` privacy exemption bypassable

**Files:** `crates/service/src/text_extract/mod.rs:194-223`.

`canonicalize_mime` routes via mime first then `.ics` extension. An ICS file with mime `text/plain` and filename `meeting.invite` (or no recognizable extension) falls through to `Mime::PlainText` and is extracted. The plan's `PrivacyExempt` skip-reason is not airtight - attendee/organizer data can land in Tantivy.

**Agreement: 3/8** (claude arch, codex arch, codex bugs).

**Fix:** widen `canonicalize_mime` to sniff `BEGIN:VCALENDAR` in the first 1 KB of plain content, or accept mis-typed ICS as out-of-scope and document.

### M10. `attachment_extracted_text` plaintext survives cache eviction

**Background:** plan acknowledges plaintext-at-rest in § "Encryption-at-rest gap"; this finding is the divergence from the existing cache posture, which the plan does not anticipate.

`attachment_cache/<content_hash>` plaintext is bounded by cache eviction. `attachment_extracted_text.extracted_text` is plaintext SQLite TEXT and is **never** evicted (per the schema design intent: text persists when bytes are evicted, so search-for-evicted-attachment still works). A user who clears their attachment cache to "shred" a sensitive PDF still has its text in SQLite indefinitely; the only path to reclaim is a Wipe rebuild. Mental-model divergence from "cache cleared = data gone."

**Agreement: 3/8** (claude security, claude bugs, codex bugs).

**Fix:** release-notes flag is acceptable for v1, but add an `attachment.shred_extracted_text` IPC (or fold into a future `attachment.shred` that handles cache + text together) so users can act on the divergence without a full rebuild.

### M11. `enqueue_dedupe` + `queue_depth` ordering can fire `ExtractCompleted` before drain finishes

**Files:** `crates/service/src/extract.rs:164-190` (enqueue), `:313-336` (finalize_item).

`enqueue` does (1) insert into `in_flight_hashes`, (2) `queue_depth.fetch_add(1)`, (3) `tx.send(work).await`, (4) on send error, undo. On a thundering-herd backfill, step (3) blocks at the bounded mpsc. While blocked, `queue_depth` already shows the new count. If the worker concurrently finalizes another item via `fetch_sub(1)` and computes `new_depth = prev - 1`, the result is racy with step (2)'s concurrent `fetch_add(1)`. The `if new_depth == 0` branch can fire prematurely with items still parked in `tx.send`. `ExtractCompleted` is then sent before the queue is actually drained - the UI's terminal "all-drained" signal lies.

**Agreement: 1/8** (claude bugs).

**Fix:** bump `queue_depth` *after* `tx.send` succeeds (move step 2 below step 3), or use a single AtomicU64 reflecting `mpsc.len() + in_flight_hashes.len()`.

### M12. `PreserveExisting` is still a public wire API but errors at runtime

**Files:** `crates/service-api/src/extract.rs:58` (RebuildPolicy enum), `crates/service/src/handlers/extract.rs:115-119` (returns `ServiceError::Internal`), `crates/service/src/dispatch.rs:1145` (schema dispatcher hardcodes Wipe).

`RebuildPolicy::PreserveExisting` is in the public IPC enum; calling it with that variant returns `ServiceError::Internal("PreserveExisting rebuild lands in phase 7-9b")`. The schema-version dispatcher hardcodes `RebuildPolicy::Wipe`. The wire surface advertises a capability the runtime refuses. Either collapse the API to Wipe-only for v1 (delete the variant) or document the runtime gap so external callers don't burn IPCs against an error path.

**Agreement: 1/8** (codex bugs).

**Fix:** delete the `PreserveExisting` variant from `RebuildPolicy` until the implementation lands. Re-introduce when Phase 8's true PreserveExisting work is in flight.

## Low

### L1. Three independent encodings of the permanent-vs-retry-eligible status taxonomy

**Files:** `crates/service/src/extract.rs:655-669` (`is_permanent_status`), `crates/service/src/handlers/attachment.rs:231-239` (`should_enqueue_extraction`), `crates/service/src/text_extract/mod.rs:106-130` (`SkipReason::status_string` + `is_retry_eligible`).

Three sources of truth for the same partition. A future addition (e.g. `SkipReason::Throttled`) compiles cleanly and silently falls into the wrong bucket in two of three places. No type-level link.

**Fix:** introduce a single `is_retry_eligible_status_str(&str) -> bool` and a single permanent-status-from-SkipReason path. Co-locate with the SkipReason enum.

### L2. `count_word_occurrences` treats non-ASCII bytes as word boundaries

**Files:** `crates/search/src/lib.rs:944-971`.

Tiebreak helper iterates bytes and tests `is_ascii_alphanumeric()`. Multi-byte UTF-8 sequences (`é = 0xC3 0xA9`) start with non-ASCII. False-positive word boundaries inflate per-attachment tiebreak `term_freq` for Latin-Extended haystacks. Tiebreak-only effect; ties fall through to filename-alphabetical anyway.

**Fix:** iterate `chars()` with `char::is_alphanumeric`.

### L3. Plain-text U+FFFD-heavy decode bypasses control-char ratio guard

**Files:** `crates/service/src/text_extract/plain.rs:39, 162-177`.

`encoding_rs::decode` returns `(text, _, had_errors)`. The skip path triggers only when decoded text is empty; `had_errors=true` with replacement-char-heavy content passes through. The control-char ratio uses `char::is_control()`, which is false for U+FFFD. A binary blob mistyped as `text/plain` decodes to mostly U+FFFD and indexes as garbage.

**Fix:** count `'\u{FFFD}'` toward the bad-char ratio, or skip when `had_errors && replacement_count > N% of total`.

### L4. `application/octet-stream` blocks extension fallback for extractable files

**Files:** `crates/service/src/text_extract/mod.rs:153, 209, 229`.

`extract()` calls `is_opaque_by_mime_or_extension` before `canonicalize_mime`. `application/octet-stream` is treated as opaque immediately. An octet-stream attachment named `.pdf` / `.docx` / etc. is skipped before extension fallback can classify it. Common case for forwarded-attachment chains where the original mime is lost.

**Fix:** check the extension first when mime is `application/octet-stream`, and only treat as opaque when the extension also doesn't match a known extractor.

### L5. `attachment.fetch` can backpressure user-facing fetch on extraction queue

**Files:** `crates/service/src/handlers/attachment.rs:123`, `crates/service/src/extract.rs:53` (`COMMAND_QUEUE_CAPACITY=256`).

Cache-miss path awaits `enqueue_extraction` while still holding the sweep read lock. Runtime queue is bounded at 256; on a thundering-herd backfill, the user's UI fetch can block on indexing-queue capacity. Drop class would be appropriate here (the fetch path doesn't need to wait for enqueue to succeed).

**Fix:** make the enqueue from the fetch handler non-blocking - `try_send` instead of `send().await`, log on full, accept the missed enqueue (the next backfill kick will catch it).

### L6. `extract.backfill_kick` from `Message::ServiceBootReady` can race runtime install

**Files:** `crates/app/src/update.rs:187`, `crates/service/src/dispatch.rs:1062` (post-ready spawn), `crates/service/src/handlers/extract.rs:144` (handler).

UI fires the kick on `boot.ready`. Service installs `ExtractRuntime` asynchronously *after* `boot.ready` via the post-ready spawn. The handler's defensive no-op when the runtime isn't installed swallows the kick silently. Backfill waits until the next hourly tick.

**Fix:** install `ExtractRuntime` before signalling `boot.ready` (would change boot semantics; needs care), or have the post-ready spawn fire the first backfill kick itself when it installs the runtime.

### L7. `lifecycle.rs` drain doc-comment is stale

**Files:** `crates/service/src/lifecycle.rs:75-95`.

Lists steps 1-5 as Push → Sync → drop → search-writer → sentinel. Phase 5 inserted Calendar between Push and Sync; Phase 7 inserted Extract before search-writer + Rebuild after Extract. Doc-rot only.

### L8. `spawn_post_ready_schema_rebuild` polls every 500 ms instead of subscribing

**Files:** `crates/service/src/dispatch.rs:1161-1167`.

Polls `rebuild_in_flight_id().is_none()` every 500 ms. The rebuild task already produces a clean `IndexRebuildCompleted` notification; subscribing to that signal would be event-driven. Today the cost is one timer per schema rebuild, so cosmetic. Folds into the C4 fix.

### L9. Single-use `search_write` slot pattern is reusable for one more producer

**Files:** `crates/service/src/boot.rs:272-303`.

The slot's first-taker-wins / defensively-cleared-by-drain semantics are invariants by convention, not by type. Reusable for exactly one more post-ready producer; if a second is added, both can race the drain in the way described in H1. Plan acknowledges; not a defect today.

**Fix when needed:** rename to `peek_search_write` / `take_search_write` to make the clone-friendly contract explicit, or convert to a `JoinSet`-of-clone-takers pattern.

### L10. Encoding fast paths and minor polish

Folded together because they're individually trivial:

- `encoding_rs` UTF-8 fast path scans bytes twice (`plain.rs:46-48`) - `std::str::from_utf8(bytes).is_ok()` then `UTF_8.decode(bytes)`.
- HTML extractor doesn't BOM-detect UTF-16 attachments (`plain.rs:69-150`).
- `fan_out_reindex` body-text fallback to `None` rewrites the doc with empty body (`extract.rs:626`); on a missing body store row, prefer skip-the-doc over rewrite-with-empty-body.
- `finalize_item` `fetch_sub` potential underflow (`extract.rs:313-314`); guard with `if prev > 0`.
- `ExtractProgress` sends fail silently when `out_tx` already taken at drain (`extract.rs:317-336`); UI may not see the terminal signal.
- `attachment_extracted_text` JOIN missing `schema_version` filter (`extract_reindex.rs:248`); moot today (Wipe truncates), blocks PreserveExisting.

## Open design questions

These aren't bugs, just unsettled choices flagged for follow-up.

### Q1. `MAX_EXTRACTED_TEXT_BYTES = 100 KB` is a hard cap

Sufficient for indexing-as-search-target (Tantivy doesn't store the bytes - `attachment_text` is `text_indexed()`, not stored), but the per-attachment `SnippetGenerator` runs against the truncated text. A document whose key match phrase lives past 100 KB is invisible to attribution. For long contracts or transcripts this matters. Should the cap be a config knob with a higher default for offline-search-heavy users?

### Q2. `COMMAND_QUEUE_CAPACITY=256` vs `BACKFILL_KICK_LIMIT=1000`

The backfill kick enqueues up to 1000 items in a tight loop with `await`-on-bounded-mpsc. With `WORKER_CONCURRENCY=4` and ~30 s p95 per PDF, draining 1000 items takes ~32 minutes. The kick handler is async and parked the entire window. Subsequent hourly kicks just block on the prior one if it's still running. After C3 lands and permanent-skip rows stop re-enqueueing, the steady-state backlog shrinks dramatically; this question becomes "how do we cap the kick handler's wall-clock parking" rather than "how do we cap the burst." Worth pinning a policy.
