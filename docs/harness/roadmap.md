# Service test harness - roadmap

The motivation is in `problem-statement.md`. The architectural shape
is in `architecture.md`. This document tracks the milestone-by-
milestone implementation plan.

The harness work is a long arc - at least a year of incremental work
to fully absorb the manual test matrix and add Track 2 (provider mock
servers + sync benchmarks). It is **not** a single phase of any
service-relocation roadmap. The Service's Phase 8 depends only on M2
of this roadmap (the wedge passing); everything past M2 unblocks
later phases of broader project work or improves the harness's
coverage. The existing `spawn_harness_with_suffix` tests are part of
the same migration arc, but are tracked separately: v1 rewrites their
boot/dispatch lifecycle coverage onto the real-subprocess
`ServiceClient` path. In-process Lua mode around `run_service_with_io`
is deferred unless a future test names coverage the subprocess path
cannot preserve.

## Status legend

- **LANDED** - committed, tests pass, exit criteria met.
- **PARTIAL** - some work in tree; remaining scope listed.
- **READY** - predecessors landed; work can start.
- **BLOCKED** - predecessors not landed.
- **DEFERRED** - explicitly held back; reason in the milestone body.

## Milestones

### M1 - Foundation

**Status:** LANDED for the Phase 8 wedge.

Ratatoskr now ships the executable harness foundation needed by M2:
`app --test-harness <script.lua>` behind `test-helpers`, a dellingr
`0.2.0` VM in `crates/app/src/harness/`, harness-only Service spawn
and trace capture, brokkr sweep wiring, and the
`crates/app/tests/service-harness/` script directory.

The original M1 notes below also named future general-purpose harness
surface (`wait_for`, `NotificationQueue`, sentinel watch, parent-death
helper bindings, complete request registry). Those are not needed by
the M2 wedge and remain future capability work for M2.5/M3+.

**Brokkr-side, in tree** (per `notes/ratatoskr-service-harness.md` in
the brokkr repo):

- `Project::Ratatoskr` first-class enum variant; `[ratatoskr]`-tagged
  commands grouped in `brokkr --help`.
- `brokkr service-test <SCRIPT>` end-to-end: project gating, script
  validation, lockfile, sweep-aware build via `[ratatoskr.harness]`,
  per-run artefact dir allocation, sync `std::process::Command`
  spawn + capture, `BROKKR_HARNESS_ARTEFACT_DIR` and
  `BROKKR_TEST_BIN_DIR` env, `run.toml` + `binary-stdout.log` +
  `binary-stderr.log`, success-cleanup unless `--keep-artefacts`,
  failure-preserve, history-DB recording.
- `brokkr service-test <SCRIPT> -N <COUNT>` soak loop with per-iter
  status line, per-iter artefact dir, `--keep-going`, exit-code
  aggregation.
- `brokkr service-list` discovery + frontmatter parse for
  `service-harness` scripts. M2's landed scripts are top-level under
  `crates/app/tests/service-harness/`; deeper cohort directories
  start at M2.5/M4.
- `ratatoskr::artefacts::ArtefactDir` lifecycle helper.
- `ratatoskr::process` primitives: `send_signal`, `pid_is_alive`,
  `wait_for_sentinel`, `snapshot_proc`.
- Sweep-aware build (`src/ratatoskr/build.rs`) reading
  `[ratatoskr.harness] sweep / binary` against `[[check]]`.

**Ratatoskr-side, in tree:**

- `dellingr = "0.2.0"` optional dependency, enabled only through the
  existing `test-helpers` feature.
- `crates/app/src/harness/` module: VM bootstrap, wall-clock script
  ceiling, global `harness` table, resource tables for clients,
  event streams, and async request handles, and runtime-owned
  artefact writers.
- Landed Lua API surface:
  `harness.data_dir`, `harness.spawn`,
  `harness.spawn_with_events`, `harness.kill`,
  `harness.pid_is_alive`, `harness.sleep`, `harness.assert`,
  `harness.assert_eq`, `harness.same_client`,
  `harness.expect_quiet(events, seconds)`, and
  `harness.protocol_version`.
- Landed client/event/request methods:
  `client:request`, `client:request_async`, `client:shutdown`,
  `client:child_pid`, `client:current_generation`, `client:drop`,
  `events:next(timeout_seconds)`, and `request:await(timeout_seconds)`.
- M1/M2 request registry entries:
  `HealthPing`, `Shutdown`, `BootReady`, `TestSlow`, and
  `TestPrintln`.
- Harness-only Service spawn/capture path for `service.stderr`,
  plus frame tracing in `frames.jsonl` and event/step tracing in
  `events.jsonl` / `steps.jsonl`.
- Failure preservation of runtime-owned diagnostics:
  `runtime-outcome.json`, copied `data-dir/`, and best-effort Linux
  `/proc` snapshots (`proc-status.txt`, `proc-wchan.txt`,
  `proc-syscall.txt`, `proc-stack.txt`).
- `app --test-harness <script.lua>` CLI flag, gated behind
  `test-helpers` so production builds never carry the Lua VM.
- Per-script wall-clock backstop. Scripts can set frontmatter
  `-- ceiling: 60s`; omitted scripts use the default ceiling.
- `crates/app/tests/service-harness/` directory.
- `brokkr.toml` additions:
  ```toml
  [[check]]
  name = "harness"
  features = ["test-helpers"]
  build_packages = ["app"]

  [ratatoskr.harness]
  sweep = "harness"
  binary = "app"
  ```

  `parent_death_helper` is a bin target inside the `app` package
  (`crates/app/src/bin/parent_death_helper.rs`), not a separate cargo
  package - `cargo build -p app` builds it as a side effect and the
  proven `crates/app/tests/service_subprocess.rs` setup already runs
  off `build_packages = ["app"]`.

**Deferred from the original foundation sketch:**

- Generic `harness.wait_for { predicate, child, backstop }` that
  races arbitrary Lua predicates against child-exit observation.
- `NotificationQueue` Lua userdata and notification-payload helpers.
- Sentinel-file watch, parent-death helper bindings, generic
  `wait_exit`, resource-budget summaries, and the full request
  registry needed by M3+.
- Parsed frame payloads in `frames.jsonl`; M1/M2 currently record
  redacted raw frames and hashes, with `parsed` left null.

**Exit criteria:**

- `brokkr service-test crates/app/tests/service-harness/smoke.lua`
  runs end-to-end: brokkr builds the `app` binary with
  `test-helpers`, spawns it with `--test-harness smoke.lua`, the
  script spawns a Service via `harness.spawn(...)`, issues a
  `health.ping`, asserts a response, exits cleanly. Artefact dir
  deletes on success. Verified on 2026-05-08.
- A failing variant (e.g. `harness.spawn(...)` against a bogus
  binary) preserves the artefact dir with brokkr-owned metadata
  (`run.toml`, `binary-stdout.log`, `binary-stderr.log`) plus
  runtime-owned diagnostics that exist for the failure class
  (`steps.jsonl`, `runtime-outcome.json`, and an empty or
  partial `frames.jsonl` if no Service frame was ever exchanged).
  The runtime writers are in tree; a dedicated bogus-binary negative
  script has not been kept as part of the permanent M2 wedge.
- `brokkr service-list` discovers the smoke script and renders its
  frontmatter. Verified on 2026-05-08; it now lists the smoke script
  plus the five M2 scripts.

**Settled M1 layout call:** keep `ServiceClient` in `app`. A slim
`crates/service-client` carve-out is deferred until a second crate
genuinely needs it or compile-time profiling shows pressure.

---

### M2 - Wedge

**Status:** LANDED for the Phase 8 close-out gate; one hardening drill
remains open.

The Service's Phase 8 close-out depends on this milestone passing.

Re-express the `#[ignore]`'d `service_subprocess.rs` tests as `.lua`
scripts. Five old libtest-subprocess flakes of the same shape (passes
solo, hangs in the suite or under `-N`) now have authoritative Lua
coverage:

- `crates/app/tests/service-harness/ping_and_shutdown.lua` -
  `harness.spawn`, `client:request("HealthPing")`, assert ack,
  `client:shutdown()`, assert clean exit. Replaces
  `service_subprocess_ping_and_shutdown`.
- `crates/app/tests/service-harness/two_phase_spawn.lua` -
  `harness.spawn_with_events`, observe `ChildSpawned` then
  `BootReady` in order, validate the two-phase contract. Replaces
  `spawn_with_events_emits_child_spawned_then_boot_ready_on_healthy_boot`.
- `crates/app/tests/service-harness/terminal_on_missing_key.lua` -
  `harness.spawn_with_events` against a keyless data dir, expect
  `Terminal { BootFailure { KeyLoadFailure } }`. Replaces
  `spawn_with_events_emits_terminal_on_missing_key`.
- `crates/app/tests/service-harness/respawn_after_sigkill.lua` -
  spawn, `harness.kill(pid, SIGKILL)`, observe `ServiceCrashed`,
  observe respawn `ChildSpawned + BootReady`, ping, assert pid
  changed. Replaces `respawn_after_sigkill_succeeds`.
- `crates/app/tests/service-harness/pending_at_respawn.lua` - spawn,
  issue `TestSlow`, SIGKILL, assert pending resolves as
  `ServiceCrashed`, then ping respawned Service. Replaces
  `pending_request_fails_at_respawn_then_subsequent_succeeds`.

**Exit criteria:**

- All five scripts pass under `brokkr service-test <script>`.
  Verified on 2026-05-08.
- A 200-iteration soak (`brokkr service-test <script> -N 200`) shows
  zero hangs across all five scripts. Verified on 2026-05-08:
  `ping_and_shutdown`, `two_phase_spawn`,
  `terminal_on_missing_key`, `respawn_after_sigkill`, and
  `pending_at_respawn` each passed 200/200.
- When the underlying writer-task drain-ordering bug is forced
  (test by manually reverting one of the Phase 4 fixes), the
  resulting failure produces a self-contained artefact dump:
  `proc-wchan.txt` showing the writer task blocked on the mpsc;
  `frames.jsonl` showing the shutdown response was never sent;
  `events.jsonl` showing what got past `BootReady`. Reproducing the
  diagnosis from artefacts alone - no re-run needed. **Not yet
  manually revalidated after the ratatoskr M1/M2 landing.**
- The `#[ignore]` markers in `service_subprocess.rs` for the five
  tests no longer hide untracked coverage. The old libtest bodies
  have been replaced by tiny ignored stubs pointing at the
  authoritative Lua scripts.

---

### M2.5 - `spawn_harness_with_suffix` cohort

**Status:** PARTIAL; the boot/dispatch lifecycle scripts have landed,
with soak and forced-hang artefact validation still open.

Migrate the `crates/service/tests/dispatch_in_process.rs` tests that
use `spawn_harness_with_suffix`. These tests are not OS-subprocess
tests, but they share the same failure mode: a Service runtime behind
an async IO boundary, protocol waits inside libtest, unconditional data
dir cleanup, and poor failure artefacts. `brokkr check` has already
caught `boot_ready_returns_after_sequence_completes` hanging under the
outer per-test watchdog.

V1 migrates this cohort by rewriting the boot/dispatch lifecycle tests
onto the real-subprocess `ServiceClient` path. Do not add
`harness.spawn_in_process(...)` in M2.5; defer in-process Lua mode
unless a future test needs coverage that subprocess mode cannot
preserve.

Initial migration list:

- `crates/app/tests/service-harness/m2_5/boot_ready_returns_after_sequence_completes.lua`
  - replaces `boot_ready_returns_after_sequence_completes`
- `crates/app/tests/service-harness/m2_5/health_ping_succeeds_during_long_migration.lua`
  - replaces `health_ping_succeeds_during_long_migration`
- `crates/app/tests/service-harness/m2_5/health_ping_works_concurrently_with_boot_ready.lua`
  - replaces `health_ping_works_concurrently_with_boot_ready`
- `crates/app/tests/service-harness/m2_5/boot_ready_blocks_until_sequence_completes.lua`
  - replaces `boot_ready_blocks_until_sequence_completes`
- `crates/app/tests/service-harness/m2_5/boot_progress_notifications_emitted_in_order.lua`
  - replaces `boot_progress_notifications_emitted_in_order`

All five scripts pass individually under `brokkr service-test` as of
2026-05-08. The old libtest bodies remain ignored pointers until the
M2.5 soak and forced-hang artefact drill close out.

The remaining `spawn_harness()` tests in the same file should be
reviewed after the boot/dispatch subset lands; many may stay as
ordinary libtest cases if they are simple request/response codec tests
that do not need lifecycle artefacts.

**Exit criteria:**

- The migrated boot/dispatch scripts pass under `brokkr service-test`.
- `brokkr check` no longer runs the flaky `spawn_harness_with_suffix`
  versions by default.
- A forced hang in `boot.ready` produces `steps.jsonl`,
  `frames.jsonl`, the preserved data dir, and either child `/proc`
  state plus `service.stderr`.

---

### M3 - Test-helper `RequestParams`

**Status:** LANDED for the initial M3 helper surface.

Add the Service-side test-helper `RequestParams` variants the cohort
needs. Each new variant should become usable from Lua through the
request-binding strategy chosen in M1 (raw JSON, typed helpers, or a
request/response registry).

Required for M4 / M5:

- `TestSeedAccount { ... }` - FK-constrained writes need real
  `accounts(id)` rows. Creates an account with credentials, label
  set, and any required adjacent rows.
- `TestCounterRead { counter }` - counter probe for "before / after"
  delta assertions in `action_skips_search_index_write`,
  `handler_does_not_drive_batch_execute`. Service-side counters that
  accumulate per-class write counts; the test-helper RPC reads the
  current value.
- Fault-injection: `TestCrashAfterNWrites { n, kind }` (or
  equivalent) - the action service / sync runner panics or exits
  after the Nth write of the given class. Needed for `pre_ack_crash_*`
  / `post_ack_crash_*` cohort.
- `--test-fake-schema=N` CLI flag (analog of the existing
  `--test-fake-version`). Used by
  `test_fake_schema_propagates_via_terminal`.

**Exit criteria:**

- Each variant has a wire-shape round-trip test in `service-api`.
- Each variant is exercised by at least one Lua script.
- `--test-fake-schema=N` flag present; existing
  `--test-fake-version` test pattern extended for schema.

Implementation note: the initial M3 slice adds
`test.seed_account`, `test.counter_read`,
`test.crash_after_n_writes`, `--test-fake-schema=N`, and Lua coverage
under `crates/app/tests/service-harness/m3/`.

---

### M4 - T1 cohort

**Status:** PARTIAL; deterministic action/journal, stale-generation,
bulk-action, and crashloop slices have landed.

Express the Phase 2 plan-specified integration cohort as `.lua`
scripts. The "T1" cohort:

- `journal_replays_after_respawn`
- `post_ack_crash_does_not_roll_back` /
  `post_ack_crash_replays_subprocess`
- `pre_ack_crash_rolls_back_subprocess`
- `mark_chat_read_emits_only_action_completed`
- `action_skips_search_index_write`
- `compose_send_50mb_attachment` / `send_wire_attachment_validation`
  / `send_wire_oversize_payload_handler_path`
- `handler_does_not_drive_batch_execute`
- `stale_outcomes_dropped_after_respawn`
- `test_fake_schema_propagates_via_terminal`
- `bulk_archive_200_threads_under_budget` (depends on `os.time`
  budget assertion; Lua-level loop dispatching 200 concurrent
  requests)
- `unbroken_crashes_trip_persistently_failing` - Phase 8-1 carry-
  forward. Replaces `crashloop_threshold_emits_terminal_after_third_crash`
  (now `#[ignore]`'d in `service_subprocess.rs`). Forces 3 unbroken
  crashes by killing the Service before BootReady on each spawn,
  asserts `ServiceHealth::PersistentlyFailing` surfaces and respawn
  stops. Also asserts the inverse: 3 kill-respawn-BootReady cycles
  do NOT trip (regression for the Phase 8-1 reset-on-success fix).

Each script lands as a separate file in
`crates/app/tests/service-harness/t1/`. Brokkr discovery recurses under
`service-harness/**/*.lua`.

Current in-tree slice:

- `handler_does_not_drive_batch_execute`
- `action_skips_search_index_write`
- `journal_replays_after_respawn`
- `post_ack_crash_replays_subprocess`
- `pre_ack_crash_rolls_back_subprocess`
- `mark_chat_read_emits_only_action_completed`
- `test_fake_schema_propagates_via_terminal`
- `stale_outcomes_dropped_after_respawn`
- `bulk_archive_200_threads_under_budget`
- `unbroken_crashes_trip_persistently_failing`

This slice also adds the M4-specific helper surface:
`test.seed_thread`, `test.thread_read`, `test.delay_next_write`, Lua
bindings for `action.execute_plan`, `action.job_status`, and
`action.mark_chat_read`, `client:notification_should_dispatch`, plus
notification fields for action plan IDs and service generations. The
crashloop script combines the existing Service-side
`--test-boot-delay-ms` helper with `SIGKILL` to keep pre-`BootReady`
respawn failures deterministic.

Remaining M4 scope:

- compose-send attachment and oversize validation scripts.
- 50-iteration soak across the full T1 directory.

**Exit criteria:**

- Every test in the list passes individually.
- A 50-iteration soak across the whole T1 directory is clean.
- The Phase 2 plan-doc reference to "T1 deferred to Phase 8" is
  resolved.

---

### M5 - Phase 7 integration cohort

**Status:** BLOCKED on M3.

The Phase 7 plan called for `crates/service/tests/extract_in_process.rs`
to cover end-to-end fetch -> extract -> re-index -> search annotation,
status-aware idempotency, eviction-during-extract, cross-attachment
phrase non-match (position-gap working), body+attachment co-match,
backfill-kick semantics, and rebuild cancellation. Lands as `.lua`
scripts in `crates/app/tests/service-harness/extract/`.

Real-world fixture corpus lands here: `.pdf` / `.docx` / `.xlsx` /
`.pptx` files plus a malicious zip-bomb `.docx`, checked into the
repo at `crates/app/tests/service-harness/fixtures/extract/`.

**Exit criteria:**

- All seven test classes from the original plan list pass.
- The fixture corpus catches at least one regression class beyond
  what the synthetic byte-literal fixtures already cover (verified by
  intentionally regressing one extractor and observing fixture-driven
  test failure).
- The Phase 7 plan-doc reference to "integration tests deferred to
  Phase 8" is resolved.

---

### M6 - Manual-matrix automation

**Status:** PARTIAL - items 4 and 5 are ready after M2; the rest unblocks
incrementally as harness capability grows.

The manual test matrix relocates from `docs/service/manual-test-matrix.md`
to `docs/harness/manual-test-matrix.md` as part of the Service Phase 8
close-out. It is the **deletable artefact**: when M6 completes,
`docs/harness/manual-test-matrix.md` is empty and gets deleted from
the repo. Every item it contains has either been automated or
explicitly retired.

Sequencing:

- **M6.4 + M6.5 (READY since M2):** heartbeat-detects-killed-Service,
  SIGTERM-triggers-shutdown-drain. Today flagged "too noisy to
  assert reliably from automation"; the deterministic harness pulls
  them in. Lua scripts: `harness.kill(service_pid, SIGKILL)` +
  follow-up event observation; `harness.kill(pid, SIGTERM)` +
  `wait_for_sentinel { path = "clean_shutdown", backstop = "5s" }` +
  `wait_exit`.
- **M6.1, M6.2, M6.3 (READY when cross-platform CI exists):**
  Linux / Windows parent-death + clean shutdown handshake + stdio
  defense. Linux items already automate; Windows items need a real
  Windows host (cross-platform CI runner, dev box, or paid test
  service). The harness scripts are platform-agnostic; the gate is
  the test environment.
- **M6.6, M6.7 (READY when fixture-setup API lands in M3):**
  cold-boot bootstrap snapshots, draft WAL replay. Both need
  deterministic data-dir state across multiple runs.
- **M6.8 (READY when M4 lands):** account.delete cancels in-flight
  sync. Same shape as the `respawn_after_sigkill_succeeds` pattern;
  needs `TestSeedAccount` + `TestSlow`-style sync stub.
- **M6.9 (BLOCKED on Track 2 OAuth fake server):** OAuth re-auth.
  Currently manual because there's no fake OAuth provider; M8 (mock
  servers) provides one.
- **M6.10 (BLOCKED on Track 2 calendar fake server):** calendar
  create/update/delete. Same as M6.9 - needs a fake CalDAV /
  Google Calendar / Graph fixture.
- **M6.11-M6.14 (READY when M5 lands):** Phase 7 attachment
  extraction round-trip, backfill kick on boot.ready, palette
  rebuild, schema-version mismatch rebuild. All have Lua-script
  shapes already sketched in `docs/service/manual-test-matrix.md`
  entries 11-14.

**Exit criteria:**

- Every item in `docs/harness/manual-test-matrix.md` has a
  corresponding `.lua` script, OR has been explicitly retired with a
  doc-comment naming why automation isn't worth the effort.
- The manual-test-matrix doc is empty.
- The doc is deleted; this milestone marks the user-visible
  promise: "the user just runs `brokkr check` and everything is
  tested autonomously."

---

### M7 - Brokkr-side polish

**Status:** PARTIAL - `service-list` and soak landed; `service-suite`
and `service-list --json` deferred per
`notes/ratatoskr-service-harness.md`.

- `brokkr service-suite [--filter X]` - walks
  `crates/app/tests/service-harness/`, runs every script (or every
  script matching `--filter`), aggregates pass/fail stats. V1 suite
  execution is serial and does not expose `--jobs`.
- `brokkr service-list --json` - machine-readable script discovery
  for failure-triage tooling and editor integrations.

**Exit criteria:**

- Suite runs across the M4 + M5 + M6 cohort cleanly.
- The JSON shape of `service-list --json` is documented stable.

---

### M8 - Provider mock servers (Track 2)

**Status:** DEFERRED - gated on M2+M4 stable, and on the team having
appetite for the protocol-modeling work (which is not small).

Mock IMAP and JMAP servers, fixture sets (small smoke / medium /
large / huge thread / many folders / duplicate Message-ID / malformed
MIME / slow-paged responses / incremental new+change+delete+move
sequence), and the brokkr-side commands to start/stop them and
collect results.

The mock-server design lives in a sibling brokkr-side note
(`notes/ratatoskr-mock-server.md`); this milestone tracks the
ratatoskr-side integration. IMAP first is probably easier for sync
realism; JMAP is broader but feasible as a bounded subset.

Headless sync trigger has converged on the Lua-script-via-harness
path (per the brokkr-side note's Plan 3 resolution): tests use the
existing `app --test-harness` binary and `ServiceClient` userdata
plus new sync-triggering / state-querying `RequestParams` variants
(`TestStartSync`, `TestQueryDbState`).

**Exit criteria:**

- A small-mailbox IMAP fixture syncs end-to-end against a fake
  IMAP server, with assertions on final account/folder/message
  counts.
- A small-mailbox JMAP fixture does the same.
- M6.9 (OAuth re-auth) and M6.10 (calendar) unblock as a
  side-effect (M8's fixture infrastructure provides the fake
  servers they need).

---

### M9 - Sync benchmarks

**Status:** DEFERRED - gated on M8.

Once mock servers are in place, sync workloads can run
deterministically against them and produce comparable timings. The
brokkr-side `notes/ratatoskr-sync-orchestration.md` covers the
orchestration; this milestone tracks the ratatoskr-side `RequestParams`
variants for emitting machine-readable sync summaries
(`TestQueryDbState`, possibly a dedicated `TestSyncSummary`).
When marker timing is needed, use the existing `BROKKR_MARKER_FIFO`
sidecar protocol; v1 harness work does not ship lifecycle markers and
does not invent a second marker protocol.

Useful metrics: cold sync wall time, incremental sync wall time,
messages per second, provider request count, peak RSS, disk read /
write bytes, final DB and store sizes.

Future brokkr gates could fail a run when:

- a previously stable Service test hangs;
- boot or respawn time regresses past a threshold;
- sync wall time regresses by more than a configured percentage;
- peak RSS regresses past a configured percentage;
- provider request count increases unexpectedly;
- final correctness assertions fail.

**Exit criteria:**

- `brokkr sync-bench --fixture imap-small --bench 10` runs
  10 iterations, records timings, compares against a stored baseline,
  exits non-zero on regression.

---

## Dependency graph

```
M1 Foundation
 |
 +-- M2 Wedge ----------------+--> M4 T1 cohort
 |                            |
 +-- M2.5 spawn_harness cohort
 |                            |
 +-- M3 Test-helper variants -+--> M5 Phase 7 cohort
 |                            |
 |                            +--> M6 Manual matrix
 |                                  (per-item readiness varies)
 |
 +-- M7 Brokkr-side polish (independent)

M2 + M4 + harness stable
 |
 +-- M8 Provider mocks
       |
       +-- M9 Sync benchmarks
       |
       +-- M6.9 OAuth re-auth (unblocks via M8 OAuth fake)
       +-- M6.10 Calendar (unblocks via M8 calendar fake)
```

The Service Phase 8 close-out (`docs/service/phase-8-plan.md`)
depends only on M2.

## Open questions deferred from M1 to "design as we implement"

- **Fixture-setup home.** Should fixture setup live in
  `crates/dev-seed`, a new test-fixtures crate, or `service` test
  helpers? Settle when M3's `TestSeedAccount` lands.

## Non-goals

- The harness is not a CI-only tool. First user is a local developer
  root-causing a flake.
- The harness is not a benchmark framework on its own. Benchmarks
  ride on the harness in M9.
- The harness does not migrate every existing `#[tokio::test]`.
  Unit-style tests stay under libtest. The Service IO-boundary cohort
  (`service_subprocess.rs` wedge, `spawn_harness_with_suffix`, and new
  Phase 8/T1 scripts) moves when it needs deterministic waits and
  artefacts.
- Brokkr does not become an IMAP / JMAP / OAuth / calendar server.
  Mock servers are a separate concern that the harness consumes.
