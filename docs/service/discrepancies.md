# Phase 7 discrepancies

Findings from the 2026-05-07 multi-archetype review (claude + codex × security/bugs/perf/arch). Items already acknowledged in `phase-7-plan.md` § "Phase 7 known-gaps" or as Phase 8 carry-forwards in `implementation-roadmap.md` are not repeated here. Severity reflects user-visible damage; "agreement" indicates how many reviewer sessions independently surfaced the finding (out of 8) - higher is higher-confidence.

## Critical

## High

## Medium

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
