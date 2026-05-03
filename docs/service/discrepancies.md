# Phase 1.5 Review Discrepancies

Findings from a fan-out review (`bugs`, `arch`, `perf`, `security` archetypes,
two providers each). Items with strong cross-reviewer agreement appear first.
Each finding cites file:line and the plan / scope item it relates to.

---

## Initial-boot terminal failures lose their `BootClassification`

**Flagged by every reviewer (security H1, bugs C1, perf H1, arch H-1, codex
critical/high in multiple lanes).**

When the Service exits with a deterministic `BootExitCode` *before* the UI
ever observed `BootReady`, the structured classification is silently
discarded and the user sees a generic "Service boot failed: service crashed"
string instead of the plan's headline messages ("Ratatoskr is already
running.", "Encryption key missing or unreadable.", "Database migration
failed.").

The race:

1. Service exits with code 71 / 72 / 73.
2. Reader hits EOF; `fail_pending` resolves the in-flight `health.ping`
   (or `boot.ready`) with `ClientError::ServiceCrashed`.
3. `handle_crash` (`crates/app/src/service_client.rs:629-640`) captures the
   exit status via `wait_with_kill_watchdog`, **then bails before
   classification because `first_boot_ready.is_none()`** ("defer to
   run_spawn_flow"). The captured exit status is dropped on the floor.
4. `run_spawn_flow` (`crates/app/src/service_client.rs:1057, 1087`) emits
   `SpawnEvent::Terminal(error)` with the raw `ClientError`, never
   inspecting the dying child's exit code.
5. `BootFailureReason::from_client_error`
   (`crates/app/src/service_client.rs:117-124`) only preserves the
   classification for `ClientError::BootFailure { .. }`; everything else
   collapses into `Other(detail)`.
6. `terminal_failure_user_message` renders the generic string.

This violates plan scope items 7, 14, 15, 16 and detailed tasks 11, 14, 16.
The friendly per-code messages in `terminal_failure_user_message` are
effectively dead code on the canonical "second instance / missing key /
migration crash" cases that scope item 7 was designed for; they only
surface on post-Ready respawn deterministic failures.

The KeyLoadFailure case has slightly better luck because the boot.ready
handler may complete with `ServiceError::Internal("boot sequence failed:
KeyLoadFailure (exit code 73)")` before the Service exits, but the UI
still string-formats this as `Other(...)`. AnotherInstanceRunning has no
such path because the Service exits before the dispatch loop is alive.

**Suggested fixes (reviewers proposed several):**

- In `run_spawn_flow`'s `Err` arm, wait briefly on the dying child and
  consult `wait_with_kill_watchdog`'s captured exit status; if it matches a
  `BootExitCode`, emit `Terminal(ClientError::BootFailure { classification })`
  instead of the raw error.
- Or: invert `handle_crash`'s `first_boot_ready=None` branch so a
  deterministic `BootExitCode` always emits `Terminal(BootFailure { code })`
  before deferring to `run_spawn_flow`. Needs coordination so only one
  Terminal event reaches the UI.
- Or (cheaper): introduce a structured `ServiceError::BootFailure {
  code: BootExitCode }` wire variant and have the boot.ready handler emit
  it in place of the current `ServiceError::Internal(format!(...))`. Closes
  the KeyLoadFailure / MigrationFailure path; AnotherInstanceRunning still
  needs the exit-code inspection because the Service never answers IPC at
  all.

The existing tests do not catch this because they pattern-match
`SpawnEvent::Terminal(_) => {}` and discard the payload
(`crates/app/tests/service_subprocess.rs:420, 463, 735`). The plan
explicitly required asserting the variant: *"replace the binary with one
whose key file is missing and assert `Terminal(BootFailure {
KeyLoadFailure })` reaches the App."*

---

## Partial velo-rename recovery can silently lose WAL data

**Flagged as Critical by codex security and codex perf, M3 by claude
security, M1 by claude perf, M-4 / M-5 / Critical by arch.**

`crates/db/src/db/mod.rs:45-96` (`reconcile_velo_rename`) has a doc-comment
at lines 27-35 that explicitly states opening `ratatoskr.db` without a
matching `ratatoskr.db-wal` can silently lose WAL-only transactions. The
implementation then proceeds to do exactly that: the main `velo.db ->
ratatoskr.db` rename is fatal (`?`), but WAL/SHM renames only `log::warn!`
and continue (lines 59, 67-75, 77, 85-93). If the WAL rename fails
(permissions, disk full, racing process), boot continues, the DB opens
without its WAL, and any uncheckpointed WAL transactions are lost.

This violates plan scope item 19 ("must add explicit recovery for the
partial-rename case ... complete the rename before opening the DB") and
detailed task 8.

The function should return `Err` on WAL/SHM rename failure and let the
boot sequence map it to `BootExitCode::MigrationFailure`. The destructive
branches (`fs::remove_file` on orphan WAL/SHM at lines 72, 90) are also
completely untested - the plan called for an integration test
(`partial-velo-rename recovery: stage ratatoskr.db + velo.db-wal +
velo.db-shm in a temp dir; assert open succeeds and the WAL/SHM files are
renamed`) that did not land.

A side concern: when `velo.db-wal` AND `ratatoskr.db-wal` both exist, the
code logs warn and removes the velo-named orphan. Plan scope item 19 is
silent on this case; removing the orphan is the safest choice but
theoretically loses an old un-checkpointed WAL. Worth a comment naming the
trade-off.

---

## UI still runs schema migrations via `Db::open` / `ReadWriteDb::init`

**Flagged by every claude reviewer (M1/M1/H2/M-1) and codex perf+arch
high+medium.**

`crates/app/src/app.rs:168` calls `Db::open(data_dir)` from
`ReadyApp::from_boot_ready`. `Db::open`
(`crates/app/src/db/connection.rs:11-19`) calls `ReadWriteDb::init`, which
(`crates/db/src/db/mod.rs:188-211`) runs `reconcile_velo_rename` *and*
`migrations::run_all` on a fresh writer connection - both of which the
Service has already done.

This violates plan scope item 2: *"the UI's `ReadWriteDb::init` no longer
runs at app boot; instead the UI waits for the Service's `boot.ready`
response, then constructs read-side state."* The implementation does the
second part (gates on handshake) but skips the first part (still runs
full read+write init).

Behavioral consequences:

- The velo-rename runs twice. Idempotent, but the UI re-acquires the
  lockfile race window after Service has finished its rename; this
  contradicts "the Service is the only writer."
- `migrations::run_all` runs twice. Idempotent for v100, but means the UI
  is also a writer on `_migrations`. Future migration authors who add v101
  will silently have it run twice.
- Three SQLite connections to the same WAL DB (Service idle in
  `BootContext` + UI read with `query_only=ON` + UI write).
- The stale-version validation in `migrations.rs:135-148` only catches
  pre-collapse versions, not future versions. If the Service ever bumps
  `_migrations` to a version the UI binary doesn't know, the UI's
  `migrations::run_all` happily runs against it.
- The comment at `app.rs:90-92` says "The DB is no longer opened here"
  which is misleading - it's just been deferred.

**Suggested fix:** add `ReadWriteDb::open_existing` (or equivalent) that
opens connections without rename or migration, and have `Db::open` call
that. Phase 2 will move the writer entirely Service-side; this is the
Phase 1.5 stub that gets us to "UI doesn't migrate" without waiting.

---

## Boot recovery steps fail silently while `boot.ready` reports success

**Flagged by claude bugs H3, codex perf+arch+bugs medium, claude arch M-3
overlap.**

`crates/service/src/boot.rs:227-255` runs pending-ops recovery, queued-drafts
sweep, and thread-participants backfill via `run_boot_recovery`
(`boot.rs:272-289`). Each failure logs at warn and returns `Ok(())`. Boot
proceeds. The Service then answers `boot.ready` with `ready: true` even if
all three recovery steps failed.

The plan's wording on this is mixed:

- Scope item 4 says *"Stranded 'executing' rows reset to 'pending' before
  the UI thinks the Service is ready."*
- Scope item 5/5a similarly say these complete before handshake readiness.
- Scope item 13 enumerates the readiness contract.

But the `run_boot_recovery` doc-comment explicitly says *"a failure leaves
the DB in the same state the previous boot left it in"* - i.e., log and
continue is intentional.

If "log and continue" is the intent, the doc should say so explicitly.
Either way, `BootReadyResponse` carries no field signaling partial
recovery, so the UI cannot surface "boot ok but recovery had warnings."
At minimum, add a `recovery_warnings: Vec<&'static str>` (or
`partial_recovery: bool`) so the status bar can flag it. A failure in
`db_pending_ops_recover_on_boot_sync` leaves `executing` rows stranded -
Phase 2's periodic drainer skips those (it only processes `pending`), so
"log and continue" has actual user-visible consequences down the line.

---

## `BootingApp` window-resize/move events are silently dropped, contradicting the doc table

**Flagged by all four claude reviewers (security L1, bugs M2, perf M2,
arch covered indirectly) and codex bugs+perf low.**

`crates/app/src/message.rs:34-38` declares:

```
WindowResized(id, size)  - handle (apply to single main window if id matches)
WindowMoved(id, point)   - handle (same)
```

But `BootingApp::update` (`crates/app/src/app.rs:582-586`) groups both
into the no-op catch-all. `WindowCloseRequested` *is* handled (line 575).
The other two are silently dropped.

Practical effect: a user who resizes/moves the splash window during a long
migration has those changes ignored. `ReadyApp::from_boot_ready` reloads
`WindowState::load(data_dir)` from disk, so the during-boot move/resize is
lost. `BootingApp` doesn't carry a `WindowState` field to apply them to.

This violates plan scope item 21 and contradicts the in-source audit
table. Either route the events through a held `WindowState` and pass it
to `ReadyApp` on transition, or update the doc-comment table to "drop".
Picking inconsistently between the two is the worst of both worlds.

Booting also doesn't subscribe to move events (`crates/app/src/app.rs:628`
vs `crates/app/src/subscription.rs:17` for Ready), so even fixing the
update arm wouldn't surface the events.

---

## Plan-required tests are missing or weakened

**Flagged across all four archetypes; arch M-4 / M-5 / M-6 are the most
detailed enumeration.**

Comparing the plan's "Test plan" section against the delivered tests:

**Integration tests (in-process):**

- `boot_ready_blocks_until_sequence_completes` - missing as named. The
  closest test (`boot_ready_returns_after_sequence_completes` at
  `crates/service/tests/dispatch_in_process.rs:317-337`) only asserts the
  eventual response; it never inserts an artificial delay to prove the
  request actually parks on `Notify`. A regression that returned
  immediately from `BootSharedState::wait_for_ready` would still pass.
- `boot_progress_notifications_emitted_in_order` - missing. The
  `read_response` helper at `dispatch_in_process.rs:412-438` *skips*
  notifications. Phase ordering is asserted nowhere at the integration
  level. A regression collapsing all phases under a single
  `CoalesceKey::BootProgress` (the design the plan specifically warned
  against) would pass every existing test.
- `health_ping_succeeds_during_long_migration` - weakened.
  `health_ping_works_concurrently_with_boot_ready` runs against a fresh
  DB whose v100 migration takes ~1ms; both requests effectively complete
  instantly. Does not actually demonstrate that `spawn_blocking` doesn't
  starve the dispatch task. Scope item 18 unverified.
- `stale_notifications_dropped_after_generation_bump` - covered only as a
  unit test in `service_client.rs:1474-1497, 1601-1627`; the full reader
  -> queue -> subscription -> BootingApp pipeline is uncovered.

**Unit tests:**

- `service: corrupt DB exits MigrationFailure (72)` - missing.
  `BootFailure::as_exit_code` (`boot.rs:122-129`) has no callers exercised
  by tests; a refactor that inverts a match arm there would not be caught.
- `service: partial-velo-rename recovery` - missing (see the velo-rename
  finding above; `reconcile_velo_rename` has zero tests at any level).
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
  the test exists. `wait_with_kill_watchdog`
  (`service_client.rs:910-931`) is the only line of defense against a
  deadlocked Service hanging the respawn or a deadlocked drop holding
  stdio open. Shipping it untested on a respawn-machinery commit is a real
  gap.
- `terminal_failure_does_not_respawn` and
  `spawn_with_events_emits_terminal_on_missing_key` - present but ignore
  the `Terminal` payload (see the headline initial-boot finding); they
  pass against the broken classification.
- `SchemaVersionChanged` post-respawn - the path at
  `service_client.rs:744-776` is reachable but unverified. Would require
  a `--test-fake-schema=N` flag analogous to `--test-fake-version`.

The non-trivial gaps are the partial-velo-rename test (the headline
silent-failure finding has no safety net), the deadlock-escalation test
(the watchdog code path is untested end-to-end), and the
classification-asserting tests that would have caught the headline
initial-boot bug.

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

## `boot.ready` handler returns unstructured `ServiceError::Internal` for boot failures

**Flagged by claude security M6, claude bugs M5 indirect, claude perf M4.**

`crates/service/src/handlers/boot.rs:23-31`:

```rust
let response = state.wait_for_ready().await.map_err(|failure| {
    ServiceError::Internal(format!(
        "boot sequence failed: {failure:?} (exit code {})",
        failure.as_exit_code().as_i32()
    ))
})?;
```

There is no structured `BootClassification` field in the wire shape. If
the boot.ready response actually arrives at the UI before the Service
exits (a tight race), the UI sees `ClientError::Service(ServiceError::
Internal(...))` and loses the structured classification. Combined with the
headline initial-boot finding, the UI has no avenue to recover the
structured classification on initial boot regardless of which path the
failure takes.

Adding a `ServiceError::BootFailure { code: BootExitCode }` wire variant
(or similar) and having the handler emit it would let this case flow
through `BootFailureReason::Classified`. Closes the
KeyLoadFailure / MigrationFailure leg of the headline finding cheaply.

---

## `BootClassification::UnexpectedExit` is treated as runtime-respawnable, not terminal

**Flagged by arch M-3.**

Plan scope item 15: *"`BootClassification::UnexpectedExit { .. }` is also
terminal. Only 'Service was running and crashed' (reader-EOF after
`BootReady` was observed, or heartbeat hard-error) triggers respawn."*

`crates/app/src/service_client.rs:664-678` (`handle_crash`) - the
deterministic-exit-code branch fires only when
`BootExitCode::from_i32(code).is_some()`. Signal-killed (`status.code() ==
None`) and unknown numeric codes both fall through to `respawn(...)` at
line 680.

`service-api/src/boot.rs:67-79`'s `BootClassification::from_exit_code`
correctly returns `UnexpectedExit { code }` for unknown codes, but **this
function is never called from production code** - it's only used in unit
tests. The respawn path inlines the equivalent logic.

The implementation chose user-friendly (respawn rather than terminate the
app on signal-kill) over the plan's anti-loop guarantee. The risk is that
the 1-second sleep is the only crashloop bound for runtime crashes;
combined with per-PID log naming and 24h cleanup threshold, a session-long
SIGKILL crashloop accumulates one log file per second per respawn.

Either: update the plan to acknowledge the runtime-respawn-on-unknown-exit
behavior, or gate respawn on a small in-memory crashloop counter (e.g.,
3 respawns in 30s -> terminate) without bringing in Phase 8 machinery.
The respawn-after-SIGKILL test pins the user-friendly behavior; reverting
to plan would break it.

---

## `clean_shutdown` sentinel is written but not removed at boot

**Flagged by arch M-9.**

Problem-statement.md says: *"The Service writes a `clean_shutdown` sentinel
file (in `<app_data>/`) at the end of its shutdown drain, **and removes it
at boot once it has acquired all writer handles**."*

`crates/service/src/lifecycle.rs:42-61` writes the sentinel during drain.
There is no boot-time removal - it persists across reboots. Phase 3+
depends on absence-at-boot to trigger cross-store recovery; if the
sentinel always exists from a prior clean shutdown, recovery never fires.

For Phase 1.5 this has no behavioral consequence (no recovery pass exists
yet), but it's a buried trap: when Phase 3's recovery lands, the trigger
condition will be wrong on every boot until someone notices. A one-liner
`let _ = tokio::fs::remove_file(&app_data_dir.join("clean_shutdown")).await;`
early in the boot sequence prevents that, with the side-effect that the
sentinel becomes a meaningful crash indicator for diagnostics.

---

## Stale-notification dispatch race: coalescing happens before the generation check

**Flagged by codex bugs high.**

`BootProgress` coalesces only by phase
(`crates/service-api/src/notification.rs:55-66`), and the queue replaces
the existing slot before dispatch
(`crates/app/src/notification_queue.rs:70`). A late gen-1 `Migrating`
notification can replace a fresh gen-2 `Migrating` notification in the
coalesce slot, and then get dropped by `notification_should_dispatch`
(`crates/app/src/service_client.rs:1186`) - losing the fresh update.

Violates scope item 20 / task 15: stale notifications should never affect
fresh ones.

Fix: do the generation check at enqueue time (in the reader task), before
the coalesce key is used. Drop stale notifications before they touch the
queue.

---

## Heartbeat decode errors are treated as soft warnings

**Flagged by codex bugs medium.**

`crates/app/src/service_client.rs:1272`. Plan scope item 16: non-timeout
heartbeat failures are hard errors that should trigger respawn. A
malformed `health.ping` response currently logs and continues, leaving the
Service considered healthy. The contract enumerated `Timeout`,
`stdin_tx.send` Err, reader EOF, and "Anything else" - decode failure
should fall under the catch-all hard-error case.

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

## `BootingApp::view` doesn't render migration progress visually

**Flagged by codex bugs medium (M4).**

`SplashState` (`crates/app/src/app.rs:464-495`) holds the current phase but
`view()` (line 597-622) renders only the label string and a fallback
detail line. When `BootPhase::Migrating { current, total }` arrives, the
rendered detail is "Migration {current} of {total}" *only if*
`splash.message` is `None` - otherwise the human-readable message takes
precedence and the count is hidden.

Plan verification step 1 explicitly calls for "ordered `Migrating {
current, total }` updates" being visible. The data is reaching the UI; it
just isn't being rendered as a progress indicator. Reasonable for "v1
splash is functional plaintext" (out of scope: branded splash visuals)
but the count should always show for `Migrating`, even if `message` is
present.

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
expanded scope the coverage is inconsistent (related to the
window-resize/move drop finding above).

---

## `service-api` wire format and `BootClassification` plumbing nits

- `service_generation` is always 0 from the Service side
  (`crates/service/src/boot_progress.rs:36`); the UI overwrites at
  `crates/app/src/service_client.rs:1166-1170`. Per-frame overhead is ~22
  bytes; trivial, but the wire format carries a field that's never the
  Service's view. Either `#[serde(skip_serializing)]` on the Service-side
  serialization or document on `BootProgress` that the field is reserved
  for the UI's reader-task to populate.
- `BootClassification::from_exit_code`
  (`crates/service-api/src/boot.rs:67-79`) is only used in unit tests;
  production inlines the logic at
  `crates/app/src/service_client.rs:664-678`. Either call the named helper
  from `handle_crash` (and the suggested H1 fix) or remove it.
- `BootProgress.message` is always `None` from the Service
  (`crates/service/src/boot.rs:172, 197, 226, 235, 248`). The optional
  field exists on the wire and on `SplashState`, but nothing populates it.
  Either populate or drop.
- `crates/service/src/handlers/boot.rs:24-29` formats the failure with
  `{failure:?}` Debug format - the Debug repr is human-tolerable but not a
  stable wire contract. If the structured-error variant lands per the
  earlier finding, this evaporates.

---

## Boot-failure exit + concurrent Shutdown returns wrong-looking ack

**Flagged by claude security L7.**

`crates/service/src/dispatch.rs:151-157, 167-176`. If the dispatch loop's
`select!` lands on `HandleOutcome::Shutdown(id)` while `boot_failure_rx`
was simultaneously ready, the loop sets `pending_shutdown_id = Some(id)`,
breaks, computes `flushed_ok`, and sends `ShutdownResponse { flushed_ok:
true }` - even though the boot failed. Kernel exit code is still
`boot_exit_code`, so the UI sees the right exit code, but the Service
emits "shutdown ok" while exiting non-zero. Cosmetic but confusing in
log triage. If `boot_exit_code.is_some()`, skip the Shutdown ack send.

Related: `boot_handle.abort()` runs *after* `drain_in_flight`
(`crates/service/src/dispatch.rs:151-157`), so a Shutdown that arrives
during a 60-second migration takes >=60s to process (the boot task is in
`spawn_blocking` and cannot be aborted). The 30s IPC timeout fires SIGTERM,
which fires `lifecycle.request_shutdown()` again (no-op), and the Service
stays blocked until SIGKILL. Acceptable per plan but undocumented; worth a
comment in `dispatch.rs` since the natural fix (abort *before* drain)
would be wrong (the boot.ready handler would never unblock).

---

## `handle_crash` cooldown vs `Drop` race

**Flagged by claude perf M6.**

`crates/app/src/service_client.rs:653-656`. After the 1s sleep,
`handle_crash` re-checks `is_shutting_down`. Between that check and
`respawn()`, `Drop` could fire. `respawn()` itself checks
`is_shutting_down` (line 712) *after* calling `launch_subprocess`, then
tears the new spawn down. That works, but it briefly forks a new Service
process that immediately gets killed, taking the file lock for ~ms before
teardown. Cleaner to take an early exit just before `launch_subprocess`
runs.

---

## `Drop` watchdog and `wait_with_kill_watchdog` have separate escalation policies

**Flagged by claude security L8.**

`crates/app/src/service_client.rs:786-855` (`Drop` /
`async_drop_wait`: 200ms abort budget for handles -> 1s wait -> SIGKILL ->
500ms poll) vs `crates/app/src/service_client.rs:910-931`
(`wait_with_kill_watchdog`: 5s wait -> `start_kill` -> 1s second wait).
Two separate kill-escalation paths with different budgets. Probably fine,
but unifying through a helper would prevent drift.

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
  `Message::WindowCloseRequested(id) if id == self.main_window_id` (line
  575) handles the main window. The catch-all also matches
  `WindowCloseRequested(_)` for non-main-window cases - unreachable in
  Booting (only one window). Minor over-specification.
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
  `crates/app/src/service_client.rs:752-770`. The `None` branch logs warn
  and captures `*guard = Some(response.clone())`. If a subsequent respawn
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
- **`cleanup_stale_logs` runs after `logging::init`.** Order is `init`
  -> open `service.<pid>.log` -> `cleanup_stale_logs` filters out current
  PID. Correct end state; on a freshly-installed system where
  `<app_data>/logs/` doesn't exist, `init` creates it then `cleanup_stale_
  logs` reads it via `fs::read_dir` returning empty. Fine.
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

---

## Items the implementation settled correctly

For completeness, items the plan handwaved or under-specified that the
implementation settled well:

- **`BootClassification` mapping for code 0 pre-BootReady.** Settled
  correctly in `BootClassification::from_exit_code` -
  `Some(0)` maps to `UnexpectedExit { code: Some(0) }`, distinct from
  clean shutdown. (Caveat: the function isn't called from production -
  see the nit above.)
- **Concurrent boot.ready and boot.progress notification ordering.** The
  plan said "concurrently with the dispatch loop." Implementation
  serializes through one writer task and `out_tx` mpsc, so response and
  notifications interleave in wire order. Tests correctly skip
  notifications when waiting for a response.
- **Reuse of `ProcessGuard` across respawns.** No-op on Linux; re-assigns
  to the same Job on Windows. Settled correctly.
- **`SchemaVersionChanged` policy on respawn.** Plan open question 2 said
  "default proposal: hard-fail." Implementation hard-fails via
  `ClientError::SchemaVersionChanged` -> `Terminal` -> `iced::exit()`.
  Correct.
- **`signal_ready` ordering.** The boot task signals BEFORE sending
  `boot_failure_tx`, so any in-flight `boot.ready` handler unblocks before
  the dispatch loop sees the failure. Subtle but correct.
- **`recover_on_boot_db_only` extraction.** Split into the canonical
  `db::db::pending_ops::db_pending_ops_recover_on_boot_sync` with
  `rtsk::actions::pending::recover_on_boot_db_only` as a thin wrapper.
  The Service calls the db-crate function directly, avoiding a transitive
  dep on `rtsk` (which would pull in all four provider crates). Better
  than the plan's working title.
- **`BootProgress` shape split between Service-emitted (gen=0) and
  reader-tagged.** Plan said the field is "tagged on the UI side at
  reader-task enqueue time, not by the Service." Honored.
- **`bypasses_admission` rename** applied at the trait-method level and
  at every callsite (`service-api/src/request.rs:81-83`,
  `service/src/dispatch.rs:217, 301`).
- **Per-phase `CoalesceKey::BootProgress(BootPhaseKind)`** so
  `Migrating(1, 10)` and `Migrating(5, 10)` collapse but `LoadingKey` and
  `OpeningDatabase` do not.
- **`crate::DB` OnceLock deletion + `crate::APP_DATA_DIR` retention.**
  Applied throughout the app crate; `Arc<Db>` is constructed in
  `ReadyApp::from_boot_ready` and threaded through.
- **Wait-then-cooldown-then-respawn ordering for lock release.**
  `wait_with_kill_watchdog` (5s, escalating to `start_kill`) -> 1s
  cooldown -> respawn.
- **Stale-notification dispatch:** reader tags every notification with
  its captured generation; `notification_should_dispatch` drops
  mismatches at debug; `current_generation` is bumped before respawn.
  (Caveat: see the coalesce-vs-generation-check race finding above.)
- **Heartbeat hard-error vs Timeout taxonomy:** `Timeout` warns and
  continues, anything else triggers crash handler. (Caveat: see the
  decode-error finding above.)
- **`kill_on_drop(false)` on the child handle** so explicit shutdown
  ordering is preserved.
- **Boot-time stale-log cleanup:** parses `service.<pid>.log[.<n>]`
  filenames, preserves current-PID files, preserves the `service.log`
  symlink and `service.log.txt` pointer. Well-tested.
- **Single-instance lock file** at `<app_data>/ratatoskr.lock`, fs2,
  kernel-managed release on exit. The 1s respawn cooldown covers the
  documented Windows "lock release may not be immediate" caveat.
- **Booting whitelist** implemented via explicit `match` with a catch-all
  that logs `std::mem::discriminant` at debug rather than panicking.
- **Two-phase spawn flow:** `ChildSpawned` after version-check ping;
  `BootReady` after `boot.ready`; `Terminal` on failure. The receiver is
  mpsc; the App task adapts via `spawn_event_stream`.
- **Boot exit codes 70-73** picked outside clap=2 / panic=101 /
  signal-137/143 ranges. Round-trip tested.
- **`BootReadyResponse { ready, schema_version, migrations_applied }`**
  matches the plan shape.
- **fs2 lock release on Windows.** `instance_lock.rs:11-14` documents
  that Windows lock release "may not become available immediately"; the
  respawn algorithm's wait-then-spawn ordering plus the 1s cooldown
  covers the typical case.
- **Doc updates in commit 17** (`docs/service/implementation-roadmap.md`
  and `problem-statement.md`) accurately reflect the settled design.
  "Extended first-ping timeout" wording correctly replaced with
  `boot.ready` references; `BootExitCode` codes 70-73 documented;
  two-phase spawn and generation tag integrated into Health / Start /
  Single-instance-guard sections.
