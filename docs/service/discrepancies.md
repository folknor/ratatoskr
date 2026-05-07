# Phase 7 discrepancies

Findings from the 2026-05-07 multi-archetype review (claude + codex × security/bugs/perf/arch). Items already acknowledged in `phase-7-plan.md` § "Phase 7 known-gaps" or as Phase 8 carry-forwards in `implementation-roadmap.md` are not repeated here. Severity reflects user-visible damage; "agreement" indicates how many reviewer sessions independently surfaced the finding (out of 8) - higher is higher-confidence.

## Critical

## High

## Medium

## Low

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
