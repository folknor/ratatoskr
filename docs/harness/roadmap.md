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
coverage.

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
- `brokkr service-list` discovery + frontmatter parse.
- `ratatoskr::artefacts::ArtefactDir` lifecycle helper.
- `ratatoskr::process` primitives: `send_signal`, `pid_is_alive`,
  `wait_for_sentinel`, `snapshot_proc`.
- Sweep-aware build (`src/ratatoskr/build.rs`) reading
  `[ratatoskr.harness] sweep / binary` against `[[check]]`.

**Ratatoskr-side, to land:**

- `dellingr` workspace dep (currently `0.2.0` on crates.io; pin
  whatever's current at land time, accept potential follow-up bumps
  per the pre-1.0 release policy).
- `crates/app/src/harness/` module: VM bootstrap, `RustFunc`
  wrappers for `ServiceClient` / `SpawnEvent` / `ClientError` /
  `NotificationQueue`, the `wait_for { predicate, child, backstop }`
  combinator that races against `observe_child_exit`, sentinel
  watch, frame-log tap, `/proc` snapshot writer, artefact-dir
  writers (`frames.jsonl` / `events.jsonl` / `steps.jsonl` /
  `data-dir/` copy / `exit.txt` / `run.toml`).
- `app --test-harness <script.lua>` CLI flag, gated behind the
  existing `test-helpers` feature so production builds never carry
  the Lua VM.
- Per-script wall-clock backstop for runaway scripts. (Not via
  dellingr's cost budget - that's structurally unable to bound
  wall-clock; see `architecture.md`.)
- `crates/app/tests/service-harness/` directory.
- `brokkr.toml` additions:
  ```toml
  [[check]]
  name = "harness"
  features = ["test-helpers"]
  build_packages = ["app", "parent_death_helper"]

  [ratatoskr.harness]
  sweep = "harness"
  binary = "app"
  ```

**Exit criteria:**

- `brokkr service-test crates/app/tests/service-harness/smoke.lua`
  runs end-to-end: brokkr builds the `app` binary with
  `test-helpers`, spawns it with `--test-harness smoke.lua`, the
  script spawns a Service via `harness.spawn(...)`, issues a
  `health.ping`, asserts a response, exits cleanly. Artefact dir
  deletes on success.
- A failing variant (e.g. `harness.spawn(...)` against a bogus
  binary) preserves the artefact dir with `frames.jsonl`,
  `service.stderr`, `exit.txt`, `run.toml`.
- `brokkr service-list` discovers the smoke script and renders its
  frontmatter.

**Open question to settle in M1:** ServiceClient slim sub-crate
carve-out vs stay in `app`. Either layout works for the bindings; the
slim crate is a compile-time-hygiene call. Decide before the harness
module's `RustFunc` wrappers land so the imports point at the right
crate.

---

### M2 - Wedge

**Status:** BLOCKED on M1.

The Service's Phase 8 close-out depends on this milestone passing.

Re-express the two `#[ignore]`'d `service_subprocess.rs` tests as
`.lua` scripts:

- `crates/app/tests/service-harness/ping_and_shutdown.lua` -
  `harness.spawn`, `client:request("HealthPing")`, assert ack,
  `client:shutdown()`, assert clean exit. Replaces
  `service_subprocess_ping_and_shutdown` (which has been hanging
  intermittently since Phase 2).
- `crates/app/tests/service-harness/terminal_on_missing_key.lua` -
  `harness.spawn_with_events` against a keyless data dir, expect
  `Terminal { BootFailure { KeyLoadFailure } }`. Replaces
  `spawn_with_events_emits_terminal_on_missing_key`.

**Exit criteria:**

- Both scripts pass under `brokkr service-test <script>`.
- A 200-iteration soak (`brokkr service-test <script> -N 200`) shows
  zero hangs across both scripts.
- When the underlying writer-task drain-ordering bug is forced
  (test by manually reverting one of the Phase 4 fixes), the
  resulting failure produces a self-contained artefact dump:
  `proc-wchan.txt` showing the writer task blocked on the mpsc;
  `frames.jsonl` showing the shutdown response was never sent;
  `events.jsonl` showing what got past `BootReady`. Reproducing the
  diagnosis from artefacts alone - no re-run needed.
- The `#[ignore]` markers in `service_subprocess.rs` for the two
  tests are removed (the tests themselves can stay or be deleted;
  the new Lua scripts are authoritative).

---

### M3 - Test-helper `RequestParams`

**Status:** BLOCKED on M1.

Add the Service-side test-helper `RequestParams` variants the cohort
needs. Each new variant is automatically usable from Lua (the binding's
`request<R>` wrapper covers the full enum).

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
`crates/app/tests/service-harness/t1/`.

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
  `wait_for_sentinel("clean_shutdown")` + `wait_exit`.
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
+ `service-list --json` deferred per `notes/ratatoskr-service-harness.md`.

- `brokkr service-suite [--filter X]` - walks
  `crates/app/tests/service-harness/`, runs every script (or every
  script matching `--filter`), aggregates pass/fail stats.
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

- **Lua API surface naming and shape.** The capabilities are pinned
  in `architecture.md`'s cohort table; function names, argument
  conventions, and return shapes are open. Settle as the binding is
  implemented; the M1 exit criteria do not require API stability,
  only end-to-end correctness.
- **Marker emission protocol.** `BROKKR_MARKER_FIFO` (existing,
  stable) vs own log entries the harness reads back. The sidecar
  protocol is the conservative call.
- **Sentinel-watch addressing.** Path relative to data dir? Absolute?
  File-glob support? Lean data-dir-relative with optional globs.
- **Backstop policy granularity.** Per-call or per-script ceiling?
  Leaning per-call (every wait takes its own backstop arg) plus a
  per-script wall-clock ceiling enforced around the whole run.
- **Trace format stability.** `frames.jsonl` / `events.jsonl` /
  `steps.jsonl` schemas need to be stable enough for scripts and
  failure-triage tooling to consume across versions. Owned by
  ratatoskr (writer); brokkr-side tooling will read them.
- **Concurrency between scripts.** Default no - subprocess tests
  touch real files and ports. Add `--jobs N` later if a class of
  scripts opts in. (Brokkr-side decision.)
- **Data-dir preservation policy on success vs failure.** Default
  cleanup on success; preserve always on failure. Open whether some
  classes of script (long-running, expensive seed) want a per-script
  override.
- **Fixture-setup home.** Should fixture setup live in
  `crates/dev-seed`, a new test-fixtures crate, or `service` test
  helpers? Settle when M3's `TestSeedAccount` lands.

## Non-goals

- The harness is not a CI-only tool. First user is a local developer
  root-causing a flake.
- The harness is not a benchmark framework on its own. Benchmarks
  ride on the harness in M9.
- The harness does not migrate the existing `#[tokio::test]` cohort.
  New tests start in the harness; old tests stay until authors
  choose to migrate them.
- Brokkr does not become an IMAP / JMAP / OAuth / calendar server.
  Mock servers are a separate concern that the harness consumes.
