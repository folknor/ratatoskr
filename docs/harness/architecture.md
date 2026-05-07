# Service test harness — architecture

The motivation is in `problem-statement.md`. The milestone-by-milestone
implementation plan is in `roadmap.md`. This document records the
technical shape of the harness: what brokkr provides, what ratatoskr
provides, the dependency direction, the determinism rule, the cohort
coverage, and the failure model.

The architecture is mirrored on the brokkr side as
`notes/ratatoskr-service-harness.md` in the brokkr repository. Both
documents stay in sync; this one is authoritative for the ratatoskr
side.

## The brokkr / ratatoskr split

The harness is split across two repositories. Ratatoskr's `app` crate
hosts the Lua VM and the `ServiceClient` userdata bindings (the
runtime); brokkr orchestrates the runtime from outside (build, spawn,
artefact dir, history). Concretely:

```
brokkr service-test foo.lua
    |
    +-- builds [ratatoskr.harness].binary via the configured [[check]] sweep
    +-- allocates .brokkr/ratatoskr/<test>/run-N/ as artefact dir
    +-- spawns: <project_root>/<target>/<profile>/app --test-harness foo.lua
    |       env: BROKKR_HARNESS_ARTEFACT_DIR=<artefact dir>
    |            BROKKR_TEST_BIN_DIR=<bin dir>
    +-- waits for child exit (sync, std::process::Command-level)
    +-- preserves artefact dir on failure / non-zero exit
```

The runtime inside `app --test-harness`:

```
+-- dellingr Lua VM
+-- ServiceClient / SpawnEvent / ClientError / NotificationQueue
|   exposed as Lua userdata, one method per Rust method
+-- wait_for { predicate, child, backstop } combinator that races
|   against ServiceClient::observe_child_exit
+-- process primitives (kill, pid_is_alive, sentinel watch)
+-- artefact writers (frames.jsonl, events.jsonl, steps.jsonl,
    proc-*.txt, data-dir/, exit.txt, run.toml) into
    BROKKR_HARNESS_ARTEFACT_DIR
```

## Dependency direction

**Both directions are off the table for source-level deps:**

- ratatoskr **must not** depend on brokkr.
- brokkr **must not** depend on ratatoskr.

Cross-process communication is by subprocess spawn + env vars only.
The Lua VM and `ServiceClient` userdata bindings live in ratatoskr's
`app` crate (already where `ServiceClient` is defined); ratatoskr
takes a `dellingr` dep, exposes the runtime via the new
`app --test-harness <script.lua>` CLI flag gated behind the existing
`test-helpers` feature. Brokkr orchestrates only: project gating,
sweep-aware build via `[ratatoskr.harness]`, lockfile, per-run
artefact-dir lifecycle, history-DB recording, soak/suite. Brokkr ships
zero ratatoskr or dellingr deps; brokkr stays sync (no tokio).

The harness needs `ServiceClient`'s typed classification (boot exit
codes, `ClientError` variants, `SchemaVersionChanged { was, now }`,
generation-tag tracking on notifications), which is hundreds of lines
of stateful protocol logic. Embedding it in brokkr would force tokio
in (the wait combinator, notification routing, and child-exit polling
all need concurrent stdio + timeout handling) and either a heavy
`app`-crate dep or a parallel JSON-RPC client implementation. Hosting
the VM in ratatoskr keeps the protocol logic where the protocol is,
keeps brokkr small, and lets the Lua bindings sit one file over from
`ServiceClient` itself.

## What brokkr provides

- **Project gating + CLI surface.** `Project::Ratatoskr` first-class
  variant; `[ratatoskr]`-tagged commands (`service-test`,
  `service-list`, eventual `service-suite`) in `brokkr --help`.
- **Sweep-aware build.** Reads `[ratatoskr.harness] sweep / binary`
  out of `brokkr.toml`, matches `sweep` to a `[[check]]` entry,
  builds every `build_packages` entry with the sweep's feature flags,
  returns the path to the `binary`-package executable. Same feature
  contract `brokkr check` enforces — a script can never run against a
  feature combination the rest of the toolchain has not validated.
  Cross-checked at parse time so a typo errors before cargo runs.
- **Subprocess spawn + capture.** `std::process::Command::output()`
  level of concurrency: spawn the harness binary with
  `--test-harness <script>` plus the artefact-dir env var, wait for
  exit, capture stdout / stderr / exit code / signal. No tokio, no
  JSON-RPC parsing on brokkr's side.
- **Per-run artefact directory lifecycle.**
  `.brokkr/ratatoskr/<test>/run-N/` with collision-incrementing N,
  preserve-on-failure, delete-on-success-unless-`--keep-artefacts`,
  preserve-on-panic (Drop default).
- **Process-tree primitives** (signal, pid_is_alive, sentinel watch
  with named backstop, /proc snapshot tolerant of CAP_SYS_PTRACE).
  Available to scripts via the harness module; also used directly by
  brokkr for hang cleanup on the orchestrator side.
- **Script discovery.** `crates/app/tests/service-harness/*.lua`,
  top-of-file `-- key: value` frontmatter (`description`,
  `expected = pass | ignored`).
- **Soak (`-N`) and suite (`--filter`) runners** on top of the
  single-script run path.
- **History-DB recording, optional sidecar /proc profiling.**

## What ratatoskr provides (in `app`'s harness module)

- **Embedded Lua VM** (`dellingr`) running test scripts.
- **`ServiceClient` userdata** — Lua scripts construct one via
  `harness.spawn(args)` or `harness.spawn_with_events(args)` and call
  the same methods the existing `#[tokio::test]` functions call:
  `client:request("HealthPing")`, `client:request("Shutdown")`,
  `client:notifications()`, `client:current_generation()`,
  `client:child_pid()`, `client:shutdown()`, `drop(client)`.
- **`SpawnEvent` receiver userdata** — `events:next(timeout_secs)`
  returns one of `ChildSpawned { client }`, `BootReady { response }`,
  `Terminal { error }`. The classification logic stays inside
  `ServiceClient`; the binding does not synthesise events.
- **`NotificationQueue` userdata** — `queue:recv(timeout)` /
  `queue:drain_for(duration)` return `Notification` userdata that
  scripts inspect for `service_generation`, `method`, etc.
- **Deterministic wait combinator** — exposed as
  `harness.wait_for { predicate, child = client, backstop = "30s" }`.
  Every wait races the predicate against
  `client:observe_child_exit()` internally; failure verdicts name
  which fired.
- **Process orchestration not covered by `ServiceClient`** —
  process-group spawn for non-`ServiceClient` children (the
  `parent_death_helper` binary, future stub helpers); SIGKILL/SIGTERM
  to a named PID; data-dir snapshotting; sentinel-file watch
  (`harness.wait_for_sentinel(path, backstop)`); JSON-RPC frame log
  for diagnostic purposes (taps the wire underneath `ServiceClient`,
  not the primary test surface).
- **Artefact-dir writers** — the frame log, event log, step trace,
  `/proc` snapshot, data-dir copy, and `exit.txt` / `run.toml` are
  populated by the harness module into the directory pointed at by
  `BROKKR_HARNESS_ARTEFACT_DIR`. Brokkr only owns the directory's
  *lifecycle* (allocate, preserve, delete); the *contents* come from
  the runtime.
- **Per-script wall-clock backstop** — runaway scripts are bounded
  by a per-script wall-clock ceiling enforced around the whole run.
  Note: dellingr's per-opcode cost accounting is **not** used for
  this. dellingr's cost budget is structurally unable to bound
  wall-clock execution (`while true do end` is free, by design — the
  budget is for not penalising users-who-write-more-code in
  game-script consumers, not for catching infinite loops). Wall-clock
  is the right mechanism for runaway abort, same shape as every
  other backstop in the harness.
- **`app --test-harness <script.lua>` CLI flag** — the runtime's
  entry point, gated behind the existing `test-helpers` feature so
  production builds never carry the Lua VM.

## Determinism rule

Every wait has the shape:

```
wait(condition) until (condition_satisfied
                     | child_terminated
                     | declared_backstop_expires)
```

The first transition to fire wins. The harness records which one fired
in the test trace. Tests assert on the transition that should have
fired; failure messages name the transition that actually did.

This is the Phase 1.6 `request_or_observe_child_exit` pattern lifted
from "one helper inside ratatoskr" to "the default wait shape exposed
through the Lua binding." `ServiceClient` already provides the
underlying mechanism (`observe_child_exit` polls `Child::try_wait` on
a 50 ms interval); the harness's wait combinator just wires that
polling into every wait the script can express. Wall-clock is never
the primary signal.

Backstops are still wall-clock — the harness can't escape physical
time entirely — but they are explicit, named, generous, and only fire
when a determinism bug elsewhere (a missing sentinel, an unmatched
event) leaves the harness with nothing else to wait on. A backstop
firing is a test-design bug, not a flake.

## Test scripts

Tests are Lua scripts. Ratatoskr's `app` crate embeds the `dellingr`
Lua VM and exposes its existing `crates/app/src/service_client.rs` API
as Lua userdata in the harness module. Brokkr never embeds dellingr;
it spawns the ratatoskr-side runtime via
`app --test-harness <script.lua>`. Adding a test means adding a
`.lua` file in ratatoskr's tree; no brokkr rebuild, and no
harness-module rebuild either unless the new test exercises a Lua API
surface that does not exist yet.

Why Lua via `dellingr`:

- Pure Rust, no FFI, no system Lua dep.
- `HostCallbacks` redirects `print()` to per-test capture and hooks
  errors for the failure dump.
- `RustFunc` is the existing pattern for exposing Rust functions to
  Lua; `ServiceClient` methods plug in directly as userdata methods.
- Variable capture, loops, conditionals come from the language.

This document does not specify the Lua API in syntax-accurate form.
What it specifies is the **required capabilities** scripts must be
able to express, derived from the existing tests in
`crates/app/tests/service_subprocess.rs`, the Phase 8 named cohort,
and the manual-matrix items moving into automation. The capabilities
below name the `ServiceClient` (and friends) methods that the Lua
binding has to surface.

### `ServiceClient` methods exposed to Lua

The existing test bodies call these. The Lua binding wraps each
one-for-one. Names below are Rust; Lua spelling is whatever the
binding picks.

- `spawn_for_test(binary, data_dir, extra_args) -> Arc<ServiceClient>`
- `spawn_with_events_for_test(binary, data_dir, extra_args) -> mpsc::Receiver<SpawnEvent>`
- `request::<R>(params: RequestParams) -> Result<R, ClientError>` —
  including the typed `RequestParams` variants (`HealthPing`,
  `Shutdown`, `TestPrintln`, `TestSlow`, `ExecutePlan`, `JobStatus`,
  `MarkChatRead`, etc.).
- `notifications() -> Arc<NotificationQueue>` — returns a queue
  userdata with `recv(timeout)` and drain helpers.
- `current_generation() -> u32`
- `child_pid() -> Option<u32>`
- `shutdown() -> Result<(), ClientError>`
- `drop(client)` — explicitly invocable from Lua to test the Drop
  teardown path.

`SpawnEvent` becomes Lua userdata with three case constructors:
`ChildSpawned { client }`, `BootReady { response }`,
`Terminal { error }`. `ClientError` becomes Lua userdata with
case-discriminating accessors (`is_service_crashed()`,
`boot_classification()`, `schema_version_changed()` returning
`{ was, now }`, etc.) so scripts can pattern-match the way the
existing tests do.

### Process and connection control

- **`drop(client)`** — exercises the explicit Drop teardown path.
- **PID-existence polling** — `harness.pid_is_alive(pid)` mirroring
  ratatoskr's `pid_is_alive` test helper.
- **Stub-parent helper invocation** — the `parent_death_helper`
  binary already exists in ratatoskr (registered as
  `CARGO_BIN_EXE_parent_death_helper`). The harness builds it
  alongside `app` and exposes
  `harness.spawn_parent_death_helper(service_binary, data_dir)
  -> { service_pid, helper_handle }`. Required for
  `linux_parent_sigkill_terminates_service_within_two_seconds`.
- **Send signal** — `harness.kill(pid, signal)` for SIGKILL/SIGTERM
  to a captured PID.
- **Respawn** — already a `ServiceClient` capability via
  `spawn_with_events_for_test` + SIGKILL + waiting for follow-up
  events on the same receiver. Lua scripts express it the same way
  the existing tests do; the harness provides nothing new.

### Assertion shapes

- **Pattern-match on `SpawnEvent`** — distinguish `ChildSpawned` vs
  `BootReady` vs `Terminal`, with `Terminal` carrying a
  `BootClassification` the script can compare against named
  constants (`BootExitCode::KeyLoadFailure`,
  `BootExitCode::AnotherInstanceRunning`, etc.).
- **Pattern-match on `ClientError`** — `Io`, `Service`,
  `ServiceCrashed`, `Timeout`, `VersionMismatch { ui, service }`,
  `BootFailure { classification }`,
  `SchemaVersionChanged { was, now }`.
- **Arc identity across respawn** —
  `harness.same_client(a, b) -> bool` exposes `Arc::ptr_eq`. Used by
  `respawn_after_sigkill_succeeds` to assert the in-place state
  swap.
- **Notification-queue introspection** —
  `queue:drain_for(duration) -> [Notification]`, with
  `Notification:service_generation()` exposing the tag.
- **Absence over window** — "no event received in N seconds, after a
  known transition." The wait combinator returns "expired" as a
  first-class verdict so the absence assertion is structural.
- **Cardinality** — "exactly N notifications matching predicate."
  Lua expresses with `drain_for` + `#table` + `assert`.
- **Counter probe with delta** — `client:request("TestCounterRead",
  ...)` called before and after, with Lua subtraction. No new harness
  primitive needed beyond the test-helper RPC.
- **Resource budget** — peak RSS, IO bytes from sidecar samples
  during a script's lifetime. Reuse brokkr's existing sidecar; expose
  `harness.resource_summary(client) -> { rss_kb, io_bytes, ... }`.
- **Child exit** — `harness.wait_exit(client, backstop) -> ExitStatus`
  with `code()`, `signal()`, `wall_time_ms()`.
- **Time-floor assertion** — "drop took at least N ms before
  escalating to kill." A simple `os.time` delta in Lua.

### Determinism scaffolding

- **Wait combinator** — `harness.wait_for { ... }`. Composes a
  predicate against `client:observe_child_exit()` so any Service
  death short-circuits the wait with a "child exited while awaiting
  X" verdict.
- **Sentinel-file watch** — `harness.wait_for_sentinel(path,
  backstop)`. Required for the `clean_shutdown` sentinel in the
  manual-matrix items moving into automation; available for any
  future test that benefits from a non-clock readiness signal.
- **Frame log** — captured under the hood via `ServiceClient`'s wire
  layer (the harness taps stdin/stdout). Diagnostic only; emitted to
  the artefact dir on failure. Tests do not pattern-match on raw
  frames — they pattern-match on `ServiceClient` return values and
  `Notification` userdata.
- **Backstop policy** — explicit, named, generous. Backstop firing
  is a test-design bug, not a flake.

### Cohort coverage

| Test | Surface used |
| --- | --- |
| `service_subprocess_ping_and_shutdown` (existing, ignored) | direct subprocess + raw frames; rewrite to `ServiceClient`-based once stable. |
| `dropping_client_terminates_child_within_one_second` | `spawn_for_test`, `child_pid`, `drop(client)`, `pid_is_alive` poll. |
| `spawn_failure_against_missing_binary_returns_io_error` | `spawn_for_test` against bogus path, expect `ClientError::Io`. |
| `linux_parent_sigkill_terminates_service_within_two_seconds` | `parent_death_helper` invocation, PID handoff via stdout, `kill(helper, SIGKILL)`, `pid_is_alive` poll on Service PID. |
| `println_from_handler_does_not_corrupt_json_rpc_framing` | `spawn_for_test`, `request("TestPrintln")`, `request("HealthPing")`, two-step round-trip. |
| `version_mismatch_surfaces_during_handshake` | `spawn_for_test` with `--test-fake-version=999`, expect `ClientError::VersionMismatch { ui, service }`. |
| `pending_request_fails_with_service_crashed_when_child_killed` | `request("TestSlow", 60_000)` in background, `kill(pid, SIGKILL)`, expect `ClientError::ServiceCrashed`. |
| `spawn_with_events_emits_child_spawned_then_boot_ready_on_healthy_boot` | `spawn_with_events_for_test`, ordered events, `BootReady` field assertions. |
| `spawn_with_events_emits_terminal_on_missing_key` (existing, ignored) | `spawn_with_events_for_test` against keyless dir, expect `Terminal { BootFailure { KeyLoadFailure } }`. |
| `missing_key_file_exits_with_key_load_failure_code` | direct subprocess, hold stdin, `wait_exit` with code 73. |
| `second_instance_against_same_data_dir_exits_with_already_running` | two parallel children, drive A's IPC, B's `wait_exit` with code 71. |
| `spawn_with_events_classifies_another_instance_running` | A direct, B via events, expect `Terminal { BootFailure { AnotherInstanceRunning } }`. |
| `respawn_after_sigkill_succeeds` | `spawn_with_events_for_test`, `kill`, follow-up events, `same_client(a, b)`, ping after respawn. |
| `pending_request_fails_at_respawn_then_subsequent_succeeds` | combined `pending_request_fails_*` + `respawn_after_sigkill_*`. |
| `terminal_failure_at_initial_boot_does_not_respawn` | `spawn_with_events`, `Terminal`, post-Terminal absence-over-window. |
| `crashloop_threshold_emits_terminal_after_third_crash` | loop of `kill` + observe `ChildSpawned + BootReady`, third kill expects `Terminal`. |
| `stale_notifications_dropped_after_generation_bump_end_to_end` | `notifications()`, `current_generation()`, drain across kill+respawn, generation-tag check. |
| `deadlocked_service_drop_escalates_to_kill` | `spawn_for_test --test-hang-on-stdin-eof`, `drop`, `pid_is_alive` poll with time-floor + ceiling. |
| `pre_ack_crash_*` / `post_ack_crash_*` (Phase 8 cohort) | `request("ExecutePlan")`, fault-inject via test-helper RPC, kill, respawn, follow-up `request`, `Notification` drain. |
| `compose_send_50mb_attachment` | `request("ComposeSend", payload_from_file)`, `wait_exit` budget. |
| `bulk_archive_200_threads_under_budget` | Lua loop dispatching 200 `request` calls in parallel, wall-clock budget assertion via `os.time`. |
| `mark_chat_read_emits_only_action_completed` | `request("MarkChatRead")`, `notifications():drain_for`, cardinality-1 assertion. |
| `action_skips_search_index_write` / `handler_does_not_drive_batch_execute` | `request("TestCounterRead", ...)` before/after, Lua subtraction. |
| `journal_replays_after_respawn` / `stale_outcomes_dropped_after_respawn` | `request` + `kill` + respawn + `notifications` drain. |
| `test_fake_schema_propagates_via_terminal` | `spawn_with_events_for_test` first run + `kill` + respawn with `--test-fake-schema=N`, expect `Terminal { SchemaVersionChanged { was, now } }`. |
| Manual matrix #4 (heartbeat detects killed Service) | `spawn_with_events_for_test`, `kill(service_pid, SIGKILL)`, `wait_for_sentinel("logs/heartbeat-exiting")` or follow-up event. |
| Manual matrix #5 (SIGTERM triggers shutdown drain) | `spawn_for_test`, `kill(pid, SIGTERM)`, `wait_for_sentinel("clean_shutdown")`, `wait_exit`. |

If a capability above doesn't have a Lua binding, the binding is
incomplete; extend it. The harness binding never reimplements
`ServiceClient` behaviour — it forwards.

## Stable Service entrypoints (ratatoskr-side contract)

`app --service --app-data-dir <dir>` must remain a stable subprocess
entrypoint.

Test-only flags should be kept intentional and documented. Existing
examples include:

- fake protocol version (`--test-fake-version=N`);
- slow request (`TestSlow { millis }`);
- hang-on-stdin-EOF (`--test-hang-on-stdin-eof`);
- println/framing canary (`TestPrintln { message }`).

More hooks will come (`--test-fake-schema=N`, fault-injection
variants, counter probes) as the cohort grows; they should remain
explicitly test-scoped, ideally behind the `test-helpers` feature
surface, and should expose `RequestParams` variants the harness can
pick up automatically (no harness recompile per new variant).

## Deterministic app-data fixtures

Brokkr does not create app-data directories itself — the Lua scripts
do, via `RequestParams::TestSeedAccount` (or equivalent). Ratatoskr
provides a fixture setup path that creates:

- `ratatoskr.key`;
- migrated main SQLite schema;
- required accounts and labels;
- rows needed by FK-constrained action tests;
- empty or seeded body/inline/blob/search stores as needed.

This must not rely on the dev app's "wipe and seed on every launch"
behavior. Some tests need to shut down and respawn against the same
data directory. `crates/dev-seed/` may be a useful source of
deterministic data generation patterns, but test fixtures are a
deliberate API, not an accidental dependency on dev startup behavior.

## Machine-readable lifecycle markers

Brokkr can measure much more accurately when ratatoskr emits stable
phase markers or counters. The sidecar already supports a marker FIFO
(`BROKKR_MARKER_FIFO`); the harness module can either route through
it or write its own log entries.

Useful lifecycle spans:

```
SERVICE_BOOT_START
SERVICE_BOOT_END
SCHEMA_MIGRATION_START
SCHEMA_MIGRATION_END
PENDING_RECOVERY_START
PENDING_RECOVERY_END
SYNC_START
SYNC_END
ACTION_PLAN_START
ACTION_PLAN_END
SHUTDOWN_START
SHUTDOWN_END
```

Timings should not require brokkr to scrape human log lines.

## Failure model

When a test fails, brokkr writes a self-contained artefact directory
to `.brokkr/ratatoskr/<test-name>/<run-N>/` containing:

- **`frames.jsonl`** — every JSON-RPC frame, both directions,
  timestamped from spawn. Single most useful artefact for
  drain-ordering / framing bugs.
- **`events.jsonl`** — every spawn event observed (`ChildSpawned`,
  `BootReady`, `Terminal`), timestamped.
- **`steps.jsonl`** — the test's step trace: which step was active,
  what condition was awaited, which transition fired.
- **`service.stderr`** — Service's stderr verbatim. Captured
  per-run, not race-mingled with test stdout.
- **`proc-at-failure.txt`** — snapshot of `/proc/<pid>/status`,
  `/proc/<pid>/wchan`, `/proc/<pid>/syscall`, `/proc/<pid>/stack` at
  the moment failure was declared. Distinguishes "blocked on futex"
  from "blocked on closed pipe" without re-running.
- **`data-dir/`** — copy of the test's app-data dir at failure time.
  SQLite WAL state, lockfile presence, key file, `clean_shutdown`
  sentinel.
- **`exit.txt`** — exit code, signal, wait time, exit reason
  (clean / harness-killed-on-backstop / signal / etc.).
- **`run.toml`** — the test script, env vars, brokkr version, git
  commit. Reproducibility metadata.

On success, the artefact directory is deleted unless
`--keep-artifacts` is passed.

The data dir copy and `/proc` snapshot are the two pieces of state
that today's tokio-test pattern destroys at failure (`DataDirGuard::Drop`
unconditional cleanup; no `/proc` capture at all). Recovering them is
the largest single jump in debug ergonomics.

## Brokkr CLI surface

Top-level commands tagged `[ratatoskr]`, project-gated to
`Project::Ratatoskr` (a brokkr-side enum variant). Same convention as
brokkr's other project-gated tags.

```
brokkr service-test <SCRIPT>
brokkr service-test <SCRIPT> -N 200       # soak
brokkr service-suite [--filter X]
brokkr service-list
```

Brokkr does not embed the Lua VM or `ServiceClient`; it never speaks
JSON-RPC over the wire.

Cargo / libtest remains the place for unit tests and for tests that
do not need the real-subprocess shape. The new harness is specifically
for tests where the subprocess lifecycle is the thing under test.

## Out of scope

- **Replacing `brokkr test`.** The cargo single-test runner stays.
  Subprocess-lifecycle tests use the new harness; everything else
  uses `brokkr test`.
- **Fixing the underlying Service bugs.** The harness exists to make
  those bugs deterministic and diagnosable, not to hide them. The
  Service-side Phase 8 work owns the drain-ordering / class-aware
  emit / crashloop fixes.
- **Migrating the existing tokio-tests.** The new harness coexists.
  Tests that work today as `#[tokio::test]` stay there. New tests in
  the cohort start in the new harness; old tests migrate only if
  their authors choose to.
- **Track 2 (provider mocks + sync benchmarks).** Reuses the
  artefact / lockfile / history machinery and the harness binary.
  Separate planning note in the brokkr repo. Lands as harness
  roadmap M8+ once Track 1 is solid.
- **CI-only features.** First user is a local developer
  root-causing a flake. CI integration follows once the local story
  works.

## Open questions

- **`ServiceClient` slim sub-crate carve-out.** Should `ServiceClient`
  and friends carve into `crates/service-client` before the harness
  module lands, or stay in `app`? Either layout works for the Lua
  bindings; the slim crate is a compile-time-hygiene call, not a
  correctness call.
- **Marker emission protocol.** Should the harness module's marker
  emission go through the existing `BROKKR_MARKER_FIFO` sidecar
  protocol, or write its own log entries the harness reads back? The
  sidecar protocol exists and is stable; using it is the
  conservative call.
- **Fixture-setup home.** Should fixture setup live in
  `crates/dev-seed`, a new test-fixtures crate, or `service` test
  helpers?
- **Concurrency between scripts.** Does the suite runner run scripts
  in parallel? Default no — subprocess tests touch real files and
  ports. Add `--jobs N` later if a class of scripts opts in.
  (Brokkr-side decision.)
- **Sentinel-watch addressing.** Path relative to data dir?
  Absolute? File-glob support? Lean data-dir-relative with optional
  globs.
- **Backstop policy.** Per-call or per-script ceiling? Leaning
  per-call (every wait takes its own backstop arg) plus a per-script
  wall-clock ceiling enforced around the whole run.
- **Trace format stability.** `frames.jsonl` / `events.jsonl` /
  `steps.jsonl` schemas need to be stable enough for scripts and
  failure-triage tooling to consume across versions.
- **Data-dir preservation policy.** Default cleanup on script
  success; preserve always with `--keep-artefacts`. Open whether
  some classes of script (long-running, expensive seed) want a
  per-script override.
