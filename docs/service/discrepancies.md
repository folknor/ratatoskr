# Phase 1.5 Review Discrepancies

Findings from a fan-out review (`bugs`, `arch`, `perf`, `security` archetypes,
two providers each). Items with strong cross-reviewer agreement appear first.
Each finding cites file:line and the plan / scope item it relates to.

This list has been trimmed as fixes land; what remains below is still open.
The arch+security follow-up review prioritized five remaining items; all
five plus the two missing-from-list items (crashloop-threshold, dev-seed
zero-key release-build hard-fail) have now landed. The remaining items are
Phase-2 / Phase-8 deferrals or genuinely cosmetic.

---

## Race spawn_inner's HealthPing against child-exit observation (deferred)

**Flagged by `bugs` review (claude+codex) when investigating an intermittent
test failure.**

`ServiceClient::spawn_inner` sends `health.ping` with a 5 s timeout. If the
child has already exited (e.g., boot's `LoadingKey` hit a missing key file),
the reader_task's EOF detection drops the pending request and returns
`ServiceCrashed` quickly - typically. Under heavy parallel-test scheduling
pressure the reader_task can be starved long enough that the 5 s ceiling
becomes the actual wall time before the request fails.

The implementation should race the ping against direct child-exit
observation: spawn a background task that awaits `child.wait()` once and
broadcasts the result via watch/oneshot, then `tokio::select!` between the
ping future and the child-exit signal in `spawn_inner`. On
child-exit-first, jump straight to `elevate_initial_boot_error`. The same
pattern would apply to `boot.ready`'s 600 s timeout.

Phase 1.5 ships with two pragmatic mitigations instead:

1. `elevate_initial_boot_error` runs the three abort-handle joins
   concurrently via `tokio::join!` (200 ms total instead of 600 ms
   sequential) and shrinks `wait_with_kill_watchdog` from 2 s to 1 s -
   the dying child has typically already exited so this is a reap, not
   a timeout-against-running-child.
2. The two no-key tests use `await_terminal_with_deadline` with a 30 s
   overall deadline rather than per-event timeouts, removing the
   structural ambiguity between "the impl hit its ping timeout" and "the
   test budget expired" that produced the intermittent flake.

The structural fix in `spawn_inner` is the right long-term answer; defer
to Phase 2 or whenever the next test-flake against this path forces it.

---

## Smaller items and nits

- **`from_boot_ready` does heavy synchronous init on the iced runtime
  thread.** `crates/app/src/app.rs:156-382` opens the DB, loads stores,
  parses bootstrap snapshots, restores pop-out windows - all synchronously
  on the iced runtime thread. On a slow disk this momentarily blocks
  rendering, and the splash has already been replaced with a frozen view.
  Worth measuring before v1 ship. (Phase 2 will rework `ActionContext`.)
- **`Drop` watchdog and `wait_with_kill_watchdog` have separate
  escalation policies.** `Drop` / `async_drop_wait`: 200ms abort budget
  for handles -> 1s wait -> SIGKILL -> 500ms poll.
  `wait_with_kill_watchdog`: 5s wait -> `start_kill` -> 1s second wait.
  The budgets are intentionally different (Drop is the user-quit path
  with ~1.7s patience; respawn has ~6s) and unifying through a helper
  would lose the distinction. Worth a contract comment naming the
  rationale.
- **`apply_standard_pragmas` is applied per-connection and the Service's
  `BootContext.db_conn` is held but unused.** Three connections to the
  same WAL DB (Service idle + UI read + UI write) is fine but wasteful;
  the Service's connection holds a WAL checkpoint slot it never uses.
  Phase 2 cleanup; flagging because if Phase 2 slips, the Service's
  `BootContext.db_conn` becomes a long-lived resource leak that scales
  with respawn count.
- **Duplicate boot work on respawn.** Each respawn re-runs the entire
  boot sequence including `reconcile_velo_rename`. Idempotent; the
  combined effect on a tight crashloop is that the lockfile race is
  amplified. The 1-second cooldown is the only mitigation. Adequate;
  worth a watch.
- **`App::scale_factor` for Booting reads `DEFAULT_SCALE` rather than the
  plan's stated 1.0.** `crates/app/src/app.rs:719-724`. The system-
  detected default is more correct than a hardcoded 1.0 (high-DPI
  displays). Improvement on the plan; mention in the next plan revision.
- **`#[cfg(test)]` `test.echo` notification variant.** Exists to verify
  wire round-trip but not actively used; can stay or go.
- **Phase 2 / `--test-fake-schema=N` test for `SchemaVersionChanged`
  end-to-end.** The mismatch path in `service_client.rs::respawn` is
  unit-tested via the Display contract, but the full subprocess path
  (where the schema actually changes across a respawn) is not. Defer
  until Phase 2 or the next schema bump introduces a real reason to
  exercise it; the smallest fake-schema hook then is cheaper than
  building it speculatively now.
