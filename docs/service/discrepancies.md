# Phase 1.5 Review Discrepancies

Findings from a fan-out review (`bugs`, `arch`, `perf`, `security` archetypes,
two providers each). Items with strong cross-reviewer agreement appear first.
Each finding cites file:line and the plan / scope item it relates to.

This list has been trimmed as fixes land; what remains below is still open.

---

## Plan-required tests still missing

**Flagged across all four archetypes; arch M-4 / M-5 / M-6 are the most
detailed enumeration.**

The classification-asserting tests, partial-velo-rename unit tests, the
crashloop tracker, the stale-notification race fix, the artificial-delay
`boot_ready_blocks_until_sequence_completes` integration test,
`boot_progress_notifications_emitted_in_order`,
`health_ping_succeeds_during_long_migration` (real artificial migration
delay), corrupt-DB `MigrationFailure`, multi-step migration progress
callback, `BootingApp::view`/`SplashState` per-phase, instance_lock
panic-survival, and `pending_request_fails_at_respawn` (combined with the
follow-up succeeds half) have all landed. The remaining gaps:

**Integration tests (in-process):**

- `stale_notifications_dropped_after_generation_bump` - covered as unit
  tests in `service_client.rs`; the full reader -> queue -> subscription
  -> BootingApp pipeline is uncovered at the integration level.

**Unit tests:**

- `service: per-phase coalesce keeps Migrating collapsed in the queue` -
  covered indirectly by `service-api` unit tests, but no test exercises
  the actual queue with real `BootProgress` payloads.
- `app: BootingApp::update whitelist drops` - implicit via the catch-all,
  but no test asserts dropped variants log at debug rather than panic.

**Real-subprocess tests:**

- `deadlocked_service_drop_escalates_to_kill` - missing entirely. Plan
  called for a `--test-hang` flag in test-helpers; neither the helper nor
  the test exists. `wait_with_kill_watchdog` is the only line of defense
  against a deadlocked Service hanging the respawn or a deadlocked drop
  holding stdio open. Shipping it untested on a respawn-machinery commit
  is a real gap.
- `SchemaVersionChanged` post-respawn - the path in `service_client.rs`'s
  `respawn` is reachable but unverified. Would require a
  `--test-fake-schema=N` flag analogous to `--test-fake-version`.

The deadlock-escalation test is the most actionable next gap (the watchdog
code path is untested end-to-end).

---

## `key_load.rs` is a deliberate duplicate of `common::crypto::load_encryption_key`

**Flagged by claude security N2 / claude arch N-5.**

`crates/service/src/key_load.rs:1-12` justifies the duplication on
dependency-graph grounds (avoids pulling `common`'s `store`, `search`,
`seen`, `reqwest`, `lol_html`, `ammonia`, `aes-gcm` into `service`).
Sound argument. But the two implementations have already diverged subtly:
the Service version logs at `debug!("using legacy key file velo.key")`
(lowercase), the common version logs `"Using legacy key file velo.key"`
(capitalized); error messages similarly diverge ("failed to read key
file" vs "Failed to read key file"). When the canonical implementation in
`common` changes (e.g., adds `zeroize` on the loaded buffer, validates
file owner UID), the Service silently keeps the old behavior. Consider
extracting a tiny `crypto-key-load` crate (no other deps) so both call
sites share code.

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
  Two separate kill-escalation paths with different budgets. Probably
  fine, but unifying through a helper would prevent drift.
- **`BootingApp::daemon_theme` ignores the stashed `appearance_mode`.**
  `crates/app/src/app.rs:712-717` returns `Theme::custom("Dark", DARK)`
  unconditionally for Booting; `appearance_mode` is captured during
  Booting and applied only after transition. The splash flashes Dark and
  then settles. Cosmetic.
- **`SchemaVersionChanged` `None` branch defensive capture.**
  `crates/app/src/service_client.rs`. The `None` branch logs warn and
  captures `*guard = Some(response.clone())`. If a subsequent respawn
  happens, the comparison is now against the newly-captured value, not
  the original - a real binary-swap bug could be masked if the
  defense-in-depth path is ever entered.
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
- **`BootClassification` test for `code == 0 AND BootReady already
  observed` not present.** Plan calls out that pre-BootReady `code == 0`
  is `UnexpectedExit { code: Some(0) }` (broken), and post-BootReady is
  "no classification produced" - only the doc-comment encodes the latter.
- **`#[cfg(test)]` `test.echo` notification variant.** Exists to verify
  wire round-trip but not actively used; can stay or go.
- **`BootContext` fields `#[allow(dead_code)]` for Phase 2.** Marked as
  scaffold; the TODO comment now points at "Phase 2 reads these via
  ActionContext". Consider a `#[cfg(test)]` smoke that constructs and
  reads from `BootContext` so a future Phase 2 PR doesn't quietly find
  that a field was dropped.
