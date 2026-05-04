# Phase 2 review discrepancies

Open gaps surfaced by the post-Phase-2 review (arch + bugs archetypes). Each item below is a present-tense gap against `phase-2-plan.md`; nothing here is historical. Resolve or rehome to a later phase, then delete the entry.

The original review surfaced 5 blockers (B1-B5), 3 high-severity items (H1-H3), 5 drift items (D1-D5), and 2 test gaps (T1-T2). All blockers, high-severity items, and four of the drift items are closed (most via code, the rest via "Phase 2 architecture deltas (as shipped)" in `phase-2-plan.md`). T2 closed via a mail-side mirror exhaustiveness test. What remains:

---

## Test gap

### T1. Plan-specified integration tests don't exist

The plan's test section (phase-2-plan.md § "Integration tests (in-process)" + "Real-subprocess smoke tests") names these tests:

- `journal_replays_after_respawn`
- `post_ack_crash_does_not_roll_back` / `post_ack_crash_replays_subprocess`
- `pre_ack_crash_rolls_back_subprocess`
- `mark_chat_read_emits_only_action_completed`
- `action_skips_search_index_write`
- `compose_send_50mb_attachment` / `send_wire_attachment_validation` / `send_wire_oversize_payload_handler_path`
- `handler_does_not_drive_batch_execute`
- `stale_outcomes_dropped_after_respawn`

None exist as named in `crates/service/tests/dispatch_in_process.rs` or `crates/app/tests/service_subprocess.rs`. The action-side unit tests in `crates/service/src/actions/tests.rs` cover individual action handlers but never spin up the worker or exercise the journal-replay path.

Foundational unit tests landed alongside the close-out fixes:

- `recover_stale_leases_resets_active_jobs_and_ops` and `recover_stale_leases_is_idempotent` (B1's core SQL).
- `unfinalized_mail_plan_jobs_finds_orphans_after_partial_finalize`, `_skips_send_jobs`, `_handles_leased_status` (B4's SQL helper that finalizes orphaned jobs on the next drain pass).
- `insert_quiet_job_rejects_unknown_account_id` (B5's FK constraint that motivated the empty-affected early-return).
- `mail_side_mirror_is_exhaustive` (T2: bidirectional `MailOperation` ↔ `WireMailOperation` mirror).

These pin the SQL and conversion contracts at the unit level, but the end-to-end behavioral tests (kill Service mid-execution, respawn, observe journal replay; submit a plan, observe per-op outcomes streaming, observe final `ActionCompleted`; etc.) don't exist. The blockers fixed during close-out were validated by reading code paths, not by exercising them.

The infrastructure investment is real: the existing in-process harness doesn't seed accounts (the FK constraint requires real `accounts(id)` rows), doesn't have a "shut down service then respawn against the same data dir" pattern, and doesn't read action notifications. A first-class test cohort needs these helpers built first.

Estimated half-day of work for the cohort. The next person who touches boot recovery, the worker drain loop, or the IPC dispatch path should land it then; without it, every change in those areas has to be re-validated by reading.
