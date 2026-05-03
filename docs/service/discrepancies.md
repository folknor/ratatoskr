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
