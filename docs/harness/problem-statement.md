# The Service test harness

## The problem

Ratatoskr's Service is a child worker process spawned by the UI. It owns
all writes (sync, action execution, DB writes, body/inline/blob stores,
Tantivy, push, attachment extraction). UI and Service talk JSON-RPC over
stdio. The failing test shape is broader than "real subprocess" alone:
it is any test that starts a Service runtime behind an IO boundary and
then waits on boot, dispatch, drain, crash, or protocol behaviour under
libtest.

Two existing families have that shape:

- `crates/app/tests/service_subprocess.rs` spawns the `app --service`
  binary, drives JSON-RPC over real stdio, and asserts on subprocess
  lifecycle behaviour: boot handshake, shutdown drain, drop behaviour,
  parent-death, signal handling, crash recovery, schema-version respawn,
  and JSON-RPC framing defense.
- `crates/service/tests/dispatch_in_process.rs` uses
  `spawn_harness_with_suffix` to run `service::run_service_with_io`
  behind `tokio::io::duplex` pipes. It is not an OS-subprocess test, but
  it has the same operational failure mode: the test drives JSON-RPC
  over an async IO boundary and waits on Service boot/dispatch/drain
  transitions.

Before the harness landed, both families were `#[tokio::test]`
functions with internal `tokio::time::timeout` or libtest/brokkr
watchdogs around protocol waits. As of the M1/M2 wedge, five flaky
`service_subprocess.rs` lifecycle tests have authoritative Lua
replacements under `crates/app/tests/service-harness/`, while the
broader `spawn_harness_with_suffix` cohort remains to migrate.

That shape doesn't work. The diagnosis is structural, not a matter of
fixing this or that flake.

**Wall-clock timeouts inside the test race against the implementation's
own ceilings.** A `#[tokio::test]` that issues a `health.ping` with a
1 s timeout is racing against the Service's own 5 s `health.ping`
timeout, the spawn-side `request_or_observe_child_exit` 50 ms polling
interval, the OS scheduler, and the disk's behaviour during a hot
migration. The old libtest wedge grew to five ignored subprocess
flakes. They now exist as Lua scripts and 200-iteration soaks pass, but
the lesson remains: running a flaky libtest shape 200 times averages
the noise rather than eliminating it.

**Failure destroys the diagnostic state.** `DataDirGuard::Drop` in the
subprocess tests and `TestDataDir::Drop` in the in-process dispatch
tests clean up the test's app-data directory unconditionally - including
on test failure. SQLite WAL state, the lockfile, the key file, the
`clean_shutdown` sentinel: all gone the moment the test fails. There is
no `/proc` snapshot for subprocesses, no preserved frame log, no
preserved Service stderr, and no structured dump of the in-process task
state. A failure today produces "test timed out at line N" or brokkr's
per-test watchdog report with little else to attach to a bug report.
Re-running the test in a debugger is the only path forward, and the bug
may not reproduce twice.

**`kill_on_drop(false)` orphans the Service when the test itself is
killed.** A test that hits its libtest timeout, or that the developer
Ctrl-C's mid-run, leaves a Service subprocess attached to a now-dead
parent. The parent-death machinery on Linux (PR_SET_PDEATHSIG) and
Windows (Job Object) handles real parent-death cleanly, but
test-process death isn't real parent-death from the Service's
perspective.

**The cohort facing this problem is large.** Phase 8 of the Service
roadmap names ~15+ similarly-shaped real-subprocess tests planned across
Phases 2-7 (`pre_ack_crash_rolls_back_subprocess`,
`journal_replays_after_respawn`, `compose_send_50mb_attachment`,
`bulk_archive_200_threads_under_budget`,
`retry_queue_persists_across_respawn`, the `--test-fake-schema=N` e2e,
the whole T1 cohort) that hadn't landed because the old framework
couldn't carry them. The existing `spawn_harness_with_suffix` cohort in
`dispatch_in_process.rs` is already in the same danger zone; a hung
`boot.ready` test under `brokkr check` currently yields a process-level
watchdog dump, not the protocol and boot-state trace needed to diagnose
the deadlock. Phase 8's planning notes are explicit: "building T1
against today's framework would mean rebuilding it once Phase 8's work
lands."

**Manual-only items already exist where automation should have
reached.** `docs/service/manual-test-matrix.md` (relocates here as
`docs/harness/manual-test-matrix.md`) carries 14 items across Linux
parent-death, Windows Job Object, clean-shutdown handshake, stdio
corruption defense, heartbeat-detects-killed-Service,
SIGTERM-triggers-shutdown-drain, cold-boot bootstrap, draft WAL,
account.delete cancellation, OAuth re-auth, calendar create/update/
delete, and the Phase 7 search/extraction round-trip. Items 4 and 5
(heartbeat, SIGTERM) are explicitly noted as "too noisy to assert
reliably from automation." That phrasing is correct against the
existing framework. A deterministic harness pulls them in.

## Goal

A test runtime that makes Service IO-boundary tests **deterministic by
construction**: every positive wait races a predicate against the
Service terminating, plus a named safety backstop. First transition to
fire wins; the runtime records which one fired in the test trace.
Backstops remain wall-clock - the runtime cannot escape physical time
entirely - but they are explicit, named, generous, and only fire when a
determinism bug elsewhere leaves the runtime with nothing else to wait
on. A safety-backstop firing is a test-design bug, not a flake.
Absence assertions use a separate `expect_quiet` observation-window
verdict.

The runtime is **generic enough to drive a JSON-RPC service runtime
behind an IO boundary.** Ratatoskr's Service is the first user. The v1
spawn path is subprocess-only (`app --service` through
`ServiceClient`). The existing `spawn_harness_with_suffix` boot/dispatch
lifecycle tests migrate onto that real-subprocess path; an in-process
Lua mode around `run_service_with_io` is deferred until a future test
names coverage that subprocess mode cannot preserve.

The runtime is **decoupled from ratatoskr at the source level.**
Adding a ratatoskr test does not require rebuilding the orchestrator
(brokkr); changing the orchestrator does not require rebuilding the
runtime.

## Failure mode the harness exists to fix

When a test fails today, the developer learns that "the test timed
out" and little else. The data dir is gone. For subprocess tests the
`/proc` state is usually gone unless brokkr's outer libtest watchdog
catches the process at exactly the right moment. The frame log was
never captured. The Service stderr was either inherited by the test
runner or mixed into the harness binary's stderr. The in-process
dispatch tests lose the same protocol chronology even though there is
no child process to snapshot. The only path forward is to re-run with
extra `dbg!` calls and hope the failure reproduces. For drain-ordering
bugs (the most common Service-correctness class), hangs reproduce once
in twenty runs.

Under the harness, a failed test produces a self-contained artefact
directory. The M1/M2 wedge already writes the core files below; some
schema fields are intentionally minimal until later cohorts need richer
readers.

- `frames.jsonl` - every JSON-RPC frame, both directions, timestamped,
  with `raw_redacted`, frame length, and SHA-256. M1/M2 leaves
  `parsed` null; structural parsed redaction is future hardening before
  credentialed scripts land.
- `events.jsonl` - every spawn/runtime event observed (`ChildSpawned`,
  `BootReady`, `Terminal`).
- `steps.jsonl` - the test's step trace: which step was active, what
  operation ran, and which coarse transition fired.
- `proc-{wchan,status,syscall,stack}.txt` - `/proc/<pid>/` snapshot at
  failure-declaration time on Linux, best-effort. Distinguishes
  "blocked on futex" from "blocked on closed pipe" without re-running
  when the kernel permits the reads.
- `service.stderr` - Service's stderr verbatim, per-run, not
  race-mingled; v1 captures it through a harness-only Service spawn
  path that pipes stderr to this file.
- `data-dir/` - copy of the test's app-data dir at failure time.
  SQLite WAL state, lockfile, key file, `clean_shutdown` sentinel.
- `runtime-outcome.json` - child/runtime exit reason, wait time, and
  whether the harness killed anything on a safety backstop.
- `run.toml` - brokkr-owned script path, env vars, brokkr version, git
  commit, sweep label, process exit code/signal.

Scripts can set frontmatter for harness-level run behavior:
`-- ceiling: 60s` for the whole-script runaway guard. Brokkr-side
artefact retention flags still control whether successful artefacts are
kept.

That dump is the difference between "the test hung, re-run with
verbose logging" and "the writer task exited at frame 47 while the
shutdown handler was awaiting an `ack` that the dispatch loop's
in-flight counter shows is still parked at 1." Recovering it is the
largest single jump in debug ergonomics.

## Load-bearing constraints

The architecture decisions that the architecture document and the
roadmap depend on:

**No source-level dependency between the orchestrator and ratatoskr.**
Brokkr does not depend on ratatoskr; ratatoskr does not depend on
brokkr. Cross-process communication is by subprocess spawn + env vars
only. This frees brokkr from the tokio dependency it would otherwise
need (the harness's async stdio + child-exit + timeout dance lives
entirely inside ratatoskr) and frees ratatoskr from any churn in
brokkr's release cadence.

**Script-visible primitives live in the ratatoskr runtime.** Brokkr may
own helper implementations for orchestrator-side cleanup (signal, PID
polling, sentinel watch, `/proc` snapshot), but Lua scripts execute
inside `app --test-harness`. Without a source dependency or an RPC
control channel back to brokkr, they cannot call brokkr Rust functions.
V1 does not add that control channel; any primitive a script invokes is
implemented in ratatoskr's harness module.

**The Lua VM lives in ratatoskr's `app` crate, not in brokkr.** The
runtime needs `ServiceClient`'s typed classification (boot exit codes,
`ClientError` variants, `SchemaVersionChanged { was, now }`,
generation-tag tracking on notifications) - hundreds of lines of
stateful protocol logic. Embedding it in brokkr would force tokio in
and either a heavy `app`-crate dep or a parallel JSON-RPC client
implementation. Hosting the VM in ratatoskr keeps the protocol logic
where the protocol is and lets the Lua bindings sit one file over from
`ServiceClient` itself.

**Tests are Lua scripts, not Rust.** Adding a test means adding a
`.lua` file in `crates/app/tests/service-harness/`. No brokkr rebuild;
no harness-module rebuild either unless the new test exercises a Lua
API surface that does not exist yet. The Lua VM is `dellingr` (pure
Rust, no FFI, no system Lua dep; M1/M2 uses `0.2.0`). dellingr's
per-opcode cost accounting is **not** used for runaway-script abort;
runaway scripts
are bounded by per-script wall-clock backstop, same mechanism the
harness uses for every wait.

**Backstops are explicit, named, generous, and never the primary
signal.** Positive waits have the shape `predicate | child_terminated |
named_safety_backstop`. The first transition to fire wins; the trace
records which one fired. Wall-clock is never the primary signal -
child-exit observation is. A safety-backstop firing names the
test-design or implementation-determinism bug. Absence assertions use a
separate observation-window shape where "nothing happened before the
window expired" can be the expected success verdict.

## What the harness is not

- **Not a replacement for ratatoskr's correctness assertions.** The
  Lua scripts drive `ServiceClient` methods and assert on returned
  values; they don't bypass ratatoskr's invariants.
- **Not a JSON-RPC server.** Brokkr never speaks JSON-RPC.
  `ServiceClient` lives in ratatoskr; brokkr spawns the binary that
  embeds it.
- **Not a CI-only tool.** First user is a local developer
  root-causing a flake. CI integration follows once the local story
  works.
- **Not a benchmark framework.** Track 2 (provider mocks + sync
  benchmarks) reuses the runtime but is its own concern.
- **Not a cargo-test wrapper.** Cargo's single-test runner (`brokkr
  test`) stays for unit tests and tests that don't need deterministic
  Service IO-boundary orchestration. The harness is for tests where the
  Service lifecycle, boot/dispatch protocol, or process/IO boundary is
  the thing under test.
- **Not a blanket migration target for all existing tokio tests.**
  Existing unit-style tests stay where they are. The migration target is
  the Service IO-boundary cohort: the M2 `service_subprocess.rs` wedge
  now represented by Lua scripts, the `spawn_harness_with_suffix`
  tests that still need migration, and new Phase 8/T1 tests that need
  the same deterministic wait and artefact model.

## Companion documents

- `docs/harness/architecture.md` - the technical shape of the
  runtime, what brokkr provides vs what ratatoskr provides, the
  cohort coverage table, the failure model.
- `docs/harness/roadmap.md` - milestone-by-milestone implementation
  plan.
- Brokkr-side companion: `notes/ratatoskr-service-harness.md` in the
  brokkr repo. Architecture is mirrored across the two; roadmap is
  this document.
