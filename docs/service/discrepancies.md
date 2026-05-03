# Phase 1.5 Review Discrepancies

Findings from a fan-out review (`bugs`, `arch`, `perf`, `security` archetypes,
two providers each). Items with strong cross-reviewer agreement appear first.
Each finding cites file:line and the plan / scope item it relates to.

This list has been trimmed as fixes land; what remains below is still open.

---

## Plan-required tests are missing or weakened

**Flagged across all four archetypes; arch M-4 / M-5 / M-6 are the most
detailed enumeration.**

The classification-asserting tests (`spawn_with_events_emits_terminal_on_
missing_key`, `terminal_failure_at_initial_boot_does_not_respawn`, and the
new `spawn_with_events_classifies_another_instance_running`) and the
partial-velo-rename unit tests have landed. The crashloop tracker and the
stale-notification race fix carry their own tests. The remaining gaps:

**Integration tests (in-process):**

- `boot_ready_blocks_until_sequence_completes` - missing as named. The
  closest test (`boot_ready_returns_after_sequence_completes` at
  `crates/service/tests/dispatch_in_process.rs`) only asserts the eventual
  response; it never inserts an artificial delay to prove the request
  actually parks on `Notify`. A regression that returned immediately from
  `BootSharedState::wait_for_ready` would still pass.
- `boot_progress_notifications_emitted_in_order` - missing. The
  `read_response` helper *skips* notifications. Phase ordering is asserted
  nowhere at the integration level. A regression collapsing all phases
  under a single `CoalesceKey::BootProgress` would pass every existing
  test.
- `health_ping_succeeds_during_long_migration` - weakened.
  `health_ping_works_concurrently_with_boot_ready` runs against a fresh
  DB whose v100 migration takes ~1ms; both requests effectively complete
  instantly. Does not actually demonstrate that `spawn_blocking` doesn't
  starve the dispatch task. Scope item 18 unverified.
- `stale_notifications_dropped_after_generation_bump` - covered as unit
  tests in `service_client.rs`; the full reader -> queue -> subscription
  -> BootingApp pipeline is uncovered at the integration level.

**Unit tests:**

- `service: corrupt DB exits MigrationFailure (72)` - missing.
  `BootFailure::as_exit_code` has no callers exercised by tests; a
  refactor that inverts a match arm there would not be caught.
- `service: migration progress callback fires per step` -
  `run_all_with_progress` has no test asserting the callback fires per
  step. Phase 1.5's single v100 migration cannot exercise the multi-step
  path.
- `service: per-phase coalesce keeps Migrating collapsed in the queue` -
  covered indirectly by `service-api` unit tests, but no test exercises
  the actual queue with real `BootProgress` payloads.
- `app: BootingApp::view per BootPhase` - no test renders the splash for
  each phase.
- `app: BootingApp::update whitelist drops` - implicit via the catch-all,
  but no test asserts dropped variants log at debug rather than panic.
- `service: instance_lock panic-survival` - the kernel-managed-on-process-
  exit guarantee is the entire reason fs2 was chosen; an end-to-end test
  spawning a subprocess, panicking it, and verifying the next instance
  acquires would lock the property in.

**Real-subprocess tests:**

- `pending_request_fails_at_respawn` - partial. The existing
  `pending_request_fails_with_service_crashed_when_child_killed` covers
  the failure half but uses `spawn_for_test` (no respawn).
  `respawn_after_sigkill_succeeds` covers respawn but doesn't have a
  long-running request in flight at kill time. The plan asked for both in
  the same test.
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

## UI still loads the encryption key independently

**Flagged by claude security M1, claude arch M-2, codex arch low.**

Plan scope item 3 + task list item 6: *"Successful load stashes in
`OnceCell<[u8; 32]>` for Phase 2's `ActionContext`. Remove the UI's
key-load call site."*

`crates/app/src/app.rs:215-221` (`ReadyApp::from_boot_ready`) still calls
`rtsk::load_encryption_key(data_dir)` and falls back to `None` on error.
`crates/app/src/app.rs:263` then derives `let snapshot_key =
encryption_key.unwrap_or([0u8; 32]);` - the very silent zero-key fallback
the plan called out as a risk worth fixing.

The Service's fatal-on-missing exit ordering makes this unreachable for
the missing-key case (Service would exit first), but a UI-only readability
error (permissions race, transient I/O glitch) would silently degrade to a
zero key and continue. The Service's `BootContext.encryption_key` field is
held but `#[allow(dead_code)]` with no getter, so there's no IPC path to
plumb the validated key out.

Either:

- Plumb the Service's stashed key out via a new IPC method (most in line
  with the plan).
- Keep the UI-side load but `expect(...)` it since the Service already
  validated the file (one-line change).

Phase 2 will move the action service Service-side and remove the need
entirely; this is the interim correctness gap.

---

## Migration progress emitted only after each migration commits

**Flagged by claude security L5, codex perf low, codex arch+bugs medium,
claude arch L-2 indirectly.**

`crates/db/src/db/migrations.rs:179` calls `progress(current, total)`
*after* the COMMIT. The justification is sound (don't claim progress that
may roll back), but it means a long single migration leaves the splash on
`OpeningDatabase` rather than `Migrating`. Phase 1.5's verification step 1
implies "Migrating (1/1)" is visible; on a fresh DB the `Migrating`
notification fires after the migration is already done - the splash
flickers it briefly or not at all if `RecoveringPendingOps` arrives first
via the writer queue.

Acceptable for the single-migration v100 case, but undercuts scope items
9/18 for long migrations unless every future long migration is split into
smaller migration records. Worth a contract comment in
`crates/db/src/db/migrations.rs` calling out the post-commit timing as a
known UX gap.

---

## `BootingApp::subscription` exceeds the plan's intended set

**Flagged by claude perf L1.**

`crates/app/src/app.rs:628-641` includes `appearance::subscription`,
`iced::window::resize_events`, and `iced::window::close_requests`. Plan
scope item 21 said: *"`subscription()` is the service-notifications recipe
only (no `SyncTick`, `SnoozeTick`, etc., since those need DB state)."*

The added subscriptions are sensible and don't need DB state; the doc-
comment in `message.rs` lists window/appearance as `handle`, so the plan's
"service-notifications recipe only" statement looks like an oversight.
Either update the plan retroactively or trim the subscriptions. Note that
Booting still doesn't subscribe to *moved* events, so even within the
expanded scope the coverage is inconsistent.

---

## `service-api` wire format nits

- `service_generation` is always 0 from the Service side
  (`crates/service/src/boot_progress.rs`); the UI overwrites at the
  reader. Per-frame overhead is ~22 bytes; trivial, but the wire format
  carries a field that's never the Service's view. Either
  `#[serde(skip_serializing)]` on the Service-side serialization or
  document on `BootProgress` that the field is reserved for the UI's
  reader-task to populate.
- `BootProgress.message` is always `None` from the Service. The optional
  field exists on the wire and on `SplashState` but nothing populates it.
  Either populate (e.g., for the migration step: `Some(format!("Applying
  migration {current} of {total}"))`) or drop.

---

## Boot-failure exit + concurrent Shutdown returns wrong-looking ack

**Flagged by claude security L7.**

`crates/service/src/dispatch.rs`. If the dispatch loop's `select!` lands
on `HandleOutcome::Shutdown(id)` while `boot_failure_rx` was simultaneously
ready, the loop sets `pending_shutdown_id = Some(id)`, breaks, computes
`flushed_ok`, and sends `ShutdownResponse { flushed_ok: true }` - even
though the boot failed. Kernel exit code is still `boot_exit_code`, so the
UI sees the right exit code, but the Service emits "shutdown ok" while
exiting non-zero. Cosmetic but confusing in log triage. If
`boot_exit_code.is_some()`, skip the Shutdown ack send.

Related: `boot_handle.abort()` runs *after* `drain_in_flight`, so a
Shutdown that arrives during a 60-second migration takes >=60s to process
(the boot task is in `spawn_blocking` and cannot be aborted). The 30s IPC
timeout fires SIGTERM, which fires `lifecycle.request_shutdown()` again
(no-op), and the Service stays blocked until SIGKILL. Acceptable per plan
but undocumented; worth a comment in `dispatch.rs` since the natural fix
(abort *before* drain) would be wrong (the boot.ready handler would never
unblock).

---

## `handle_crash` cooldown vs `Drop` race

**Flagged by claude perf M6.**

`crates/app/src/service_client.rs`. After the 1s sleep, `handle_crash`
re-checks `is_shutting_down`. Between that check and `respawn()`, `Drop`
could fire. `respawn()` itself checks `is_shutting_down` *after* calling
`launch_subprocess`, then tears the new spawn down. That works, but it
briefly forks a new Service process that immediately gets killed, taking
the file lock for ~ms before teardown. Cleaner to take an early exit just
before `launch_subprocess` runs.

---

## `Drop` watchdog and `wait_with_kill_watchdog` have separate escalation policies

**Flagged by claude security L8.**

`Drop` / `async_drop_wait`: 200ms abort budget for handles -> 1s wait ->
SIGKILL -> 500ms poll. `wait_with_kill_watchdog`: 5s wait -> `start_kill`
-> 1s second wait. Two separate kill-escalation paths with different
budgets. Probably fine, but unifying through a helper would prevent drift.

Related: `Drop` on a `current_thread` runtime defeats the point of
`kill_on_drop(false)`. The fallback path is `poll_for_exit_blocking(&mut
child, 1200ms)` without giving the writer task a chance to drain - this
guarantees a SIGKILL after 1.2s. The doc-comment acknowledges this. Unit
tests in `service_client.rs::tests` use `flavor = "current_thread"`; if
any of those ever exercises Drop with a real subprocess, they'd hit the
SIGKILL path. Footgun for future test authors.

---

## Logger init is allowed to fail silently

**Flagged by claude security M10.**

`crates/service/src/lib.rs:43` uses `let _ = logging::init(&app_data_dir);`
and proceeds. If logging fails (e.g., logs directory unwritable), the
panic hook still installs but the rolling log file is absent. For Phase
1.5 this means a Service that fails its boot sequence has nowhere to write
the failure cause - the UI sees the exit code but the log file diagnosing
*why* doesn't exist. Stderr inheritance partially mitigates on dev.
Consider failing the Service start if logging init fails *and* the logs
directory was specified.

Also: `crates/service/src/lib.rs:35` uses `eprintln!` for the stdio-claim
failure. After `claim_stdio()` succeeds, eprintln is safe (stderr is
preserved). But if `claim_stdio` fails, we may be in a state where stderr
was partially redirected; eprintln could still corrupt. Probably benign
because the Service exits immediately, but use `std::io::stderr().lock().
write_all(...)` directly to be defensive in line with the rest of the
stdio-discipline policy.

---

## `dev-seed` writes a zero-key file; no debug log when the zero key is in use

**Flagged by codex bugs medium (M6).**

`crates/dev-seed/src/lib.rs:236-249` writes base64-encoded 32 zero bytes
to `ratatoskr.key`. This is sufficient to pass the Service's boot
key-load (returns `Ok`), but it means every dev launch silently uses a
key of all zeros for AES-256-GCM. Plan acknowledges this is fine for dev
(ephemeral data), but worth a `debug!("loaded zero key")` warning in
`key_load.rs` so a release build accidentally shipping with this file
produces a visible signal.

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

## `db_pending_ops_recover_on_boot_sync` API split is non-obvious

**Flagged by claude security verification, claude arch nit, codex arch
low.**

`crates/db/src/db/pending_ops.rs:372-398`'s
`db_pending_ops_recover_on_boot_sync` does both 'executing' reset *and*
'sending' draft resurfacing, while `db_mark_queued_drafts_failed_sync`
handles 'queued' drafts (a different sync_status, called separately for
`BootPhase::SweepingQueuedDrafts`). The two phases overlap on
draft-status repair but with distinct sync_status values. The split is
non-obvious from the phase names. A comment in `boot.rs` explaining what
each phase actually does would help.

Also: `crates/core/src/actions/pending.rs:404-406`'s
`recover_on_boot_db_only` is a one-line wrapper that no production caller
uses (the Service calls the canonical function in `db` directly). Either
delete the wrapper or have the boot sequence call it for consistency with
the "rtsk re-exports" pattern.

---

## Smaller items and nits

- **`from_boot_ready` does heavy synchronous init on the iced runtime
  thread.** `crates/app/src/app.rs:156-382` opens the DB, loads stores,
  parses bootstrap snapshots, restores pop-out windows - all synchronously
  on the iced runtime thread. On a slow disk this momentarily blocks
  rendering, and the splash has already been replaced with a frozen view.
  Worth measuring before v1 ship. (Phase 2 will rework `ActionContext`.)
- **`BootingApp::update`'s catch-all uses `discriminant` and loses the
  variant name.** `log::debug!("BootingApp dropped message variant {:?}
  ...", std::mem::discriminant(&other))` prints `Discriminant(...)` which
  is opaque. `format!("{other:?}")` would help, though many `Message`
  variants carry large payloads (e.g., `ThreadDetailLoaded`) - the full
  Debug print could be noisy.
- **Test naming.** `boot_ready_returns_after_sequence_completes` doesn't
  match the plan's `boot_ready_blocks_until_sequence_completes`. Current
  name describes the outcome, not the property.
- **`BootingApp::update`'s redundant guard.**
  `Message::WindowCloseRequested(id) if id == self.main_window_id` handles
  the main window. The catch-all also matches `WindowCloseRequested(_)`
  for non-main-window cases - unreachable in Booting (only one window).
  Minor over-specification.
- **`BootingApp::daemon_theme` ignores the stashed `appearance_mode`.**
  `crates/app/src/app.rs:712-717` returns `Theme::custom("Dark", DARK)`
  unconditionally for Booting; `appearance_mode` is captured during
  Booting and applied only after transition. The splash flashes Dark and
  then settles. Cosmetic.
- **`BootingApp::view` exit on user-closes-splash.** If the user closes
  the splash window during boot, `iced::exit()` fires immediately. No log
  line; the Service is not asked to shut down. Kernel-managed lock release
  saves us, but a `log::info!("user closed splash; exiting")` would be a
  small operability win.
- **`open_db_and_migrate` returns `(Connection, u32, u32)` magic tuple.**
  A struct named like `MigrateOutcome { conn, schema_version,
  migrations_applied }` would survive the next refactor better.
- **`wait_for_ready` loop is defensive but unnecessary.**
  `crates/service/src/boot.rs:77-91`. `signal_ready` only ever populates
  `result` once (guarded), and `notify.notify_waiters()` wakes everyone -
  the loop is single-iteration in practice.
- **`signal_ready` is silently no-op on second call.**
  `crates/service/src/boot.rs:93-111`. Defense-in-depth; a `debug!` when
  this happens would help future debugging if a refactor accidentally
  double-signals.
- **`SchemaVersionChanged` `None` branch defensive capture.**
  `crates/app/src/service_client.rs`. The `None` branch logs warn and
  captures `*guard = Some(response.clone())`. If a subsequent respawn
  happens, the comparison is now against the newly-captured value, not
  the original - a real binary-swap bug could be masked if the
  defense-in-depth path is ever entered.
- **Heartbeat task post-crash cleanup is eventual, not synchronous.**
  `handle_crash` aborts handles via 200ms timeouts but doesn't `.abort()`
  them - a still-running heartbeat with a stale `weak_client.upgrade()`
  idles for up to 30s before its next interval tick fails-and-exits.
  Harmless but could be tightened with explicit `.abort()` calls.
- **Stale module comment.** `crates/service/src/boot.rs:3` says the
  current implementation only covers key load and future commits will add
  DB/recovery/backfill. The file now implements the full sequence.
- **`boot_progress::emit` swallows `try_send` failures.**
  `crates/service/src/boot_progress.rs:45-47`. Phase 1.5 emits ~6 frames
  during boot, queue capacity 1024 - effectively unreachable. Worth a
  comment naming the assumption ("OUTBOUND_QUEUE_CAP >> total Phase-1.5
  boot.progress frames"); revisit if Phase 2+ adds notification emissions
  during boot.
- **`crates/service/src/lib.rs:42`** silently falls back to
  `default_app_data_dir` if `--app-data-dir` is missing. Production UI
  always passes it, but a debug-session invocation without it would
  silently use the dev-data dir. At least an info log would help.
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
- **`AtomicU32` for generation is larger than necessary.** Plan calls for
  `service_generation: u32`, implementation matches. With a 1s respawn
  floor and Phase 8 crashloop guard, hitting 2^32 incarnations is
  impossible. Leave as-is for forward compatibility.
- **`RequestParams::Shutdown` does not bypass admission**, but is
  intercepted before the admission check (handle_line catches Shutdown
  first). Worth a one-line comment on the timeout function noting that
  "shutdown is dispatch-loop-intercepted; admission check below is not
  reached."
- **`boot_failure_rx.recv()` channel has capacity 1 with `let _ = ...
  send(...).await` discarding errors.** If the dispatch loop's `select!`
  already broke out before the boot task could send, the failure send
  returns Err and is silently dropped. `boot_exit_code` stays None and
  the Service exits 0. In practice the only ways to break out early
  (Shutdown request, stdin EOF) want exit code 0 anyway, so safe; worth a
  comment.
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
  scaffold; a TODO pointing at "Phase 2 reads these via [path-to-future-
  getter]" would help the next reader. Consider a `#[cfg(test)]` smoke
  that constructs and reads from `BootContext` so a future Phase 2 PR
  doesn't quietly find that a field was dropped.
