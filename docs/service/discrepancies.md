# Phase 7 discrepancies

Findings from the 2026-05-07 multi-archetype review (claude + codex × security/bugs/perf/arch). Items already acknowledged in `phase-7-plan.md` § "Phase 7 known-gaps" or as Phase 8 carry-forwards in `implementation-roadmap.md` are not repeated here. Severity reflects user-visible damage; "agreement" indicates how many reviewer sessions independently surfaced the finding (out of 8) - higher is higher-confidence.

## Critical

## High

## Medium

## Low

## Open design questions

These aren't bugs, just unsettled choices flagged for follow-up.

### Q1. `MAX_EXTRACTED_TEXT_BYTES = 100 KB` is a hard cap

Sufficient for indexing-as-search-target (Tantivy doesn't store the bytes - `attachment_text` is `text_indexed()`, not stored), but the per-attachment `SnippetGenerator` runs against the truncated text. A document whose key match phrase lives past 100 KB is invisible to attribution. For long contracts or transcripts this matters. Should the cap be a config knob with a higher default for offline-search-heavy users?

### Q2. `COMMAND_QUEUE_CAPACITY=256` vs `BACKFILL_KICK_LIMIT=1000`

The backfill kick enqueues up to 1000 items in a tight loop with `await`-on-bounded-mpsc. With `WORKER_CONCURRENCY=4` and ~30 s p95 per PDF, draining 1000 items takes ~32 minutes. The kick handler is async and parked the entire window. Subsequent hourly kicks just block on the prior one if it's still running. After C3 lands and permanent-skip rows stop re-enqueueing, the steady-state backlog shrinks dramatically; this question becomes "how do we cap the kick handler's wall-clock parking" rather than "how do we cap the burst." Worth pinning a policy.
