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

**Status:** PARTIAL - brokkr-side scaffolding LANDED; ratatoskr-side
work READY (gated by Service Phase 8 starting; can land independently
of Phase 8's recovery work).

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
- `brokkr service-list` discovery + frontmatter parse. Existing
  scaffolding is top-level-only; update it to recursive
  `service-harness/**/*.lua` and parse `ceiling` /
  `preserve_data_dir` before M4/M5.
- `ratatoskr::artefacts::ArtefactDir` lifecycle helper.
- `ratatoskr::process` primitives: `send_signal`, `pid_is_alive`,
  `wait_for_sentinel`, `snapshot_proc`.
- Sweep-aware build (`src/ratatoskr/build.rs`) reading
  `[ratatoskr.harness] sweep / binary` against `[[check]]`.

**Ratatoskr-side, to land:**

- `dellingr` workspace dep; verify the current crates.io version at
  land time and accept potential follow-up bumps per the pre-1.0
  release policy.
- `crates/app/src/harness/` module: VM bootstrap, `RustFunc`
  wrappers for `ServiceClient` / `SpawnEvent` / `ClientError` /
  `BootClassification` / `SchemaVersionChanged` /
  `NotificationQueue`, registry-backed `client:request(...)`
  returning Lua tables, the `wait_for { predicate, child, backstop }`
  combinator that races against child-exit observation,
  `expect_quiet { predicate, child, window }` for absence assertions,
  sentinel watch, frame-log tap, `/proc` snapshot writer, artefact-dir
  writers (`frames.jsonl` / `events.jsonl` / `steps.jsonl` /
  `data-dir/` copy / `service.stderr` / `runtime-outcome.json`).
  Brokkr already owns `run.toml`, `binary-stdout.log`, and
  `binary-stderr.log`.
- Make the child-exit observation surface available to the harness
  module by bumping `ServiceClient::observe_child_exit` to
  `pub(crate)`. The wait combinator stays in the harness module so
  `ServiceClient` remains a protocol layer and does not know about Lua
  predicates or the VM.
- Implement the Lua-facing request/response registry. The Rust API is
  `request::<R>(RequestParams)`; Lua calls a registry-backed
  `client:request(method, params)` and receives a Lua table decoded by
  Rust. Notification payloads use a `serde_json::Value`-backed view
  instead of per-notification typed shells.
- Implement script-visible process primitives in ratatoskr where Lua
  needs them (`kill`, `pid_is_alive`, sentinel watch, data-dir
  snapshot, optional `/proc` snapshot). Brokkr's matching helpers are
  orchestrator-side only under the current process-level contract; v1
  does not add a brokkr/runtime control channel.
- Add a harness-only Service spawn/capture path for `service.stderr`,
  because the current `ServiceClient` subprocess path inherits stderr.
- `app --test-harness <script.lua>` CLI flag, gated behind the
  existing `test-helpers` feature so production builds never carry
  the Lua VM.
- Per-script wall-clock backstop for runaway scripts. (Not via
  dellingr's cost budget - that's structurally unable to bound
  wall-clock; see `architecture.md`.) Scripts can set frontmatter
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

**Exit criteria:**

- `brokkr service-test crates/app/tests/service-harness/smoke.lua`
  runs end-to-end: brokkr builds the `app` binary with
  `test-helpers`, spawns it with `--test-harness smoke.lua`, the
  script spawns a Service via `harness.spawn(...)`, issues a
  `health.ping`, asserts a response, exits cleanly. Artefact dir
  deletes on success.
- A failing variant (e.g. `harness.spawn(...)` against a bogus
  binary) preserves the artefact dir with brokkr-owned metadata
  (`run.toml`, `binary-stdout.log`, `binary-stderr.log`) plus
  runtime-owned diagnostics that exist for the failure class
  (`steps.jsonl`, `runtime-outcome.json`, and an empty or
  partial `frames.jsonl` if no Service frame was ever exchanged).
- `brokkr service-list` discovers the smoke script and renders its
  frontmatter.

**Settled M1 layout call:** keep `ServiceClient` in `app`. A slim
`crates/service-client` carve-out is deferred until a second crate
genuinely needs it or compile-time profiling shows pressure.

---

### M2 - Wedge

**Status:** BLOCKED on M1.

The Service's Phase 8 close-out depends on this milestone passing.

Re-express the `#[ignore]`'d `service_subprocess.rs` tests as `.lua`
scripts. Five are currently ignored - all libtest-subprocess flakes
of the same shape (passes solo, hangs in the suite or under `-N`):

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
- A 200-iteration soak (`brokkr service-test <script> -N 200`) shows
  zero hangs across all five scripts.
- When the underlying writer-task drain-ordering bug is forced
  (test by manually reverting one of the Phase 4 fixes), the
  resulting failure produces a self-contained artefact dump:
  `proc-wchan.txt` showing the writer task blocked on the mpsc;
  `frames.jsonl` showing the shutdown response was never sent;
  `events.jsonl` showing what got past `BootReady`. Reproducing the
  diagnosis from artefacts alone - no re-run needed.
- The `#[ignore]` markers in `service_subprocess.rs` for the five
  tests no longer hide untracked coverage. Acceptable resolutions:
  delete the old libtest bodies, leave tiny ignored stubs pointing at
  the authoritative Lua scripts, or re-enable them only if they no
  longer use the flaky wait pattern. The Lua scripts are authoritative.

---

### M2.5 - `spawn_harness_with_suffix` cohort

**Status:** BLOCKED on M1; can land in parallel with M3 once the M2
wedge proves the harness runner.

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

- `boot_ready_returns_after_sequence_completes`
- `health_ping_succeeds_during_long_migration`
- `health_ping_works_concurrently_with_boot_ready`
- `boot_ready_blocks_until_sequence_completes` (currently ignored)
- `boot_progress_notifications_emitted_in_order` (currently ignored)

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

**Status:** BLOCKED on M1.

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

---

### M4 - T1 cohort

**Status:** BLOCKED on M2 + M3.

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

Each script lands as a separate file in
`crates/app/tests/service-harness/t1/`. Brokkr discovery recurses under
`service-harness/**/*.lua`.

**Exit criteria:**

- Every test in the list passes individually.
- A 50-iteration soak across the whole T1 directory is clean.
- The Phase 2 plan-doc reference to "T1 deferred to Phase 8" is
  resolved.

---

### M5 - Phase 7 integration cohort

**Status:** BLOCKED on M2 + M3.

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

**Status:** PARTIAL - items 4 and 5 unblock at M2; the rest unblocks
incrementally as harness capability grows.

The manual test matrix relocates from `docs/service/manual-test-matrix.md`
to `docs/harness/manual-test-matrix.md` as part of the Service Phase 8
close-out. It is the **deletable artefact**: when M6 completes,
`docs/harness/manual-test-matrix.md` is empty and gets deleted from
the repo. Every item it contains has either been automated or
explicitly retired.

Sequencing:

- **M6.4 + M6.5 (READY at M2):** heartbeat-detects-killed-Service,
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
