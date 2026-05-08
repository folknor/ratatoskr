# Service test harness - architecture

The motivation is in `problem-statement.md`. The milestone-by-milestone
implementation plan is in `roadmap.md`. This document records the
technical shape of the harness: what brokkr provides, what ratatoskr
provides, the dependency direction, the determinism rule, the cohort
coverage, and the failure model.

The target cohort is not limited to OS-subprocess tests. It includes
any Service test that starts the Service behind an IO boundary and then
waits on boot, dispatch, drain, crash, or framing behaviour. Today that
means both `crates/app/tests/service_subprocess.rs` and the
`spawn_harness_with_suffix` family in
`crates/service/tests/dispatch_in_process.rs`.

The architecture is mirrored on the brokkr side as
`notes/ratatoskr-service-harness.md` in the brokkr repository. Both
documents stay in sync; this one is authoritative for the ratatoskr
side.

## Brokkr context (for readers new to it)

Brokkr is a single-binary Rust dev tool, **external to ratatoskr** -
its source lives in a separate repository (sibling to ratatoskr's on
the author's machine: `~/Programs/brokkr`) and is installed via
`cargo install --path ~/Programs/brokkr`. It is not a ratatoskr crate,
not a workspace member, and not a build dependency. From ratatoskr's
side, brokkr is just a binary on `$PATH` that the developer invokes
from any project root.

Brokkr reads `./brokkr.toml` to detect which project it's working in
(`pbfhogg`, `nidhogg`, `ratatoskr`, etc.) and exposes project-gated
top-level commands tagged `[ratatoskr]`, `[pbfhogg]`, etc. The CLI is
flat: `brokkr service-test foo.lua`, not `brokkr ratatoskr
service-test foo.lua`. Today it already handles ratatoskr's
`brokkr check` and `brokkr test` invocations; the harness commands
(`service-test`, `service-list`, `service-suite`, and mock/sync
orchestration commands as they land) are an extension of that surface.

Brokkr also owns lockfile coordination, build orchestration with
feature sweeps, a sidecar profiler that samples `/proc` at 100 ms, a
results DB (`.brokkr/results.db`) for benchmark history, persistent
worktrees, structured artefact retention, and visual reference
testing for HTML/glyph projects. **None of that machinery is part of
the harness contract** - see "Contract surface" below for what the
harness module actually owes brokkr. The other capabilities are
either reused implicitly (the artefact-dir lifecycle, the lockfile,
the history DB recording happen automatically when `service-test`
spawns the harness binary) or unused entirely (the sidecar profiler,
the worktrees, visual testing).

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

The target runtime inside `app --test-harness`:

```
+-- dellingr Lua VM
+-- ServiceClient / SpawnEvent / ClientError / NotificationQueue
|   exposed as Lua userdata, one method per Rust method
+-- wait_for { predicate, child, backstop } combinator that races
|   against child-exit observation
+-- script-visible process primitives (kill, pid_is_alive, sentinel watch)
+-- artefact writers (frames.jsonl, events.jsonl, steps.jsonl,
    proc-*.txt, data-dir/, runtime diagnostics) into
    BROKKR_HARNESS_ARTEFACT_DIR
```

Current landed state (2026-05-08): the M1/M2 wedge implements the
subprocess Service path, the dellingr `0.2.0` VM, client/event/request
userdata needed by the wedge scripts, redacted frame tracing,
event/step tracing, Service stderr capture, runtime outcome writing,
data-dir copy-on-failure, and best-effort Linux `/proc` snapshots.
M8's ratatoskr-side sync surface has also started: provider clients
read test-only mock endpoint env vars, sync-harness scripts can call
`test.start_sync` and `test.query_db_state`, and
`crates/app/tests/sync-harness/jmap-initial.lua` targets the
`jmap-small` fixture. Mock orchestration remains outside ratatoskr.
The broader target surface above is still incremental work:
generic `wait_for`, sentinel watch, parent-death helper bindings,
generic `wait_exit`, resource summaries, and a complete request
registry are deferred until the first migrated test needs each one.

The v1 spawn path is subprocess-only. Scripts spawn the Service through
`ServiceClient` / `app --service`; the `spawn_harness_with_suffix`
cohort migrates by rewriting boot/dispatch lifecycle coverage onto
that subprocess path. An in-process Lua mode around
`run_service_with_io` is deferred until a future test names coverage
that the subprocess path cannot preserve.

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

`ServiceClient` stays in `app` for v1. A slim
`crates/service-client` carve-out is a compile-time-hygiene refactor,
not a correctness requirement; revisit only if a second crate genuinely
needs the client API or compile-time profiling shows pressure.

## What brokkr provides

- **Project gating + CLI surface.** `Project::Ratatoskr` first-class
  variant; `[ratatoskr]`-tagged commands (`service-test`,
  `service-list`, `service-suite`, `mock-serve`) in `brokkr --help`.
- **Sweep-aware build.** Reads `[ratatoskr.harness] sweep / binary`
  out of `brokkr.toml`, matches `sweep` to a `[[check]]` entry,
  builds every `build_packages` entry with the sweep's feature flags,
  returns the path to the `binary`-package executable. Same feature
  contract `brokkr check` enforces - a script can never run against a
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
- **Orchestrator-side process-tree primitives** (signal, pid_is_alive,
  sentinel watch with named backstop, /proc snapshot tolerant of
  CAP_SYS_PTRACE). These are useful for brokkr's own hang cleanup and
  for implementation sharing later, but they are **not** directly
  callable from Lua scripts. V1 implements equivalent script-visible
  bindings in ratatoskr's harness module and does not add a
  brokkr/runtime control channel.
- **Script discovery.** `crates/app/tests/service-harness/**/*.lua`,
  top-of-file `-- key: value` frontmatter (`description`,
  `expected = pass | ignored`, `ceiling = 60s`,
  `preserve_data_dir = on_success_too`). Discovery is recursive so
  cohorts can live under `t1/`, `extract/`, etc.; non-`.lua` files
  such as fixtures are ignored by extension. `preserve_data_dir` is
  brokkr-side frontmatter because brokkr owns artefact-dir deletion.
- **Soak (`-N`) and serial suite (`--filter`) runners** on top of the
  single-script run path. `service-test <SCRIPT> -N 50` repeats that
  script 50 times. `service-test <DIR> -N 50` routes through the suite
  path and runs the cohort 50 cycles. The v1 suite is serial and does
  not expose `--jobs`; parallelism can be added later only for an
  opt-in class of isolated scripts.
- **History-DB recording, optional sidecar /proc profiling.**

## What ratatoskr provides (in `app`'s harness module)

The M1/M2 implementation is deliberately narrower than the full
target API described below. In tree today:

- `app --test-harness <script.lua>` is compiled only with
  `test-helpers`.
- The Lua global `harness` exposes `data_dir`, `spawn`,
  `spawn_with_events`, `kill`, `pid_is_alive`, `sleep`, `assert`,
  `assert_eq`, `same_client`, `expect_quiet(events, seconds)`, and
  `protocol_version`.
- Client tables expose `request`, `request_async`, `shutdown`,
  `child_pid`, `current_generation`, and `drop`.
- Event streams expose `events:next(timeout_seconds)`.
- Async request handles expose `request:await(timeout_seconds)`.
- The request registry currently covers `HealthPing`, `Shutdown`,
  `BootReady`, `TestSlow`, and `TestPrintln`.

The bullets after this paragraph are the target capability set for the
full harness arc; when they mention APIs not in the current list, those
APIs are future work, not hidden existing behavior.

- **Embedded Lua VM** (`dellingr`) running test scripts.
- **`ServiceClient` userdata** - Lua scripts construct one via
  `harness.spawn(args)` or `harness.spawn_with_events(args)` and call
  the same methods the existing `#[tokio::test]` functions call:
  `client:request("HealthPing")`, `client:request("Shutdown")`,
  `client:notifications()`, `client:current_generation()`,
  `client:child_pid()`, `client:shutdown()`, `drop(client)`.
  The `request` binding is registry-backed: Rust owns a
  request/response registry that maps Lua method names and argument
  tables onto `RequestParams` variants, decodes the typed Rust
  response, and returns a plain Lua table. Bad method names, malformed
  argument tables, and mismatched response shapes fail in Rust with a
  structured harness error.
- **`SpawnEvent` receiver userdata** - `events:next(timeout_secs)`
  returns one of `ChildSpawned { client }`, `BootReady { response }`,
  `Terminal { error }`. The classification logic stays inside
  `ServiceClient`; the binding does not synthesise events.
  `SpawnEvent`, `ClientError`, `BootClassification`, and
  `SchemaVersionChanged { was, now }` are exposed as typed userdata so
  scripts can pattern-match without parsing strings.
- **`NotificationQueue` userdata** - not landed in M1/M2.
  Target shape: `queue:recv(timeout)` /
  `queue:drain_for(duration)` return `Notification` userdata that
  scripts inspect for `service_generation`, `method`, etc.
  Notification payloads are the exception to typed request/response
  decoding: they expose a `serde_json::Value`-backed Lua view for
  `params`, so scripts can filter on `notif.method == "X"` and inspect
  varied payload details without one typed shell per notification.
- **Deterministic wait combinator** - not landed in M1/M2.
  Target shape: exposed as
  `harness.wait_for { predicate, child = client, backstop = "30s" }`.
  Every wait races the predicate against
  child-exit observation internally; failure verdicts name which fired.
  When this lands, the harness will need a child-exit observation
  surface that preserves `ServiceClient` as a protocol layer and does
  not teach it about Lua predicates or the VM.
- **Quiet observation combinator** - M1/M2 exposes
  `harness.expect_quiet(events, seconds)` for event-stream absence
  assertions. Target shape:
  `harness.expect_quiet { predicate, child = client, window = "2s" }`.
  The window expiring without the predicate firing is success; child
  termination should short-circuit with a named verdict once the
  generic form lands.
- **Process orchestration not covered by `ServiceClient`** -
  process-group spawn for non-`ServiceClient` children (the
  `parent_death_helper` binary, future stub helpers); SIGKILL/SIGTERM
  to a named PID; data-dir snapshotting; sentinel-file watch
  (`harness.wait_for_sentinel { path = "...", backstop = "5s" }` for
  data-dir-relative paths and
  `harness.wait_for_sentinel { absolute = "/...", backstop = "5s" }`
  for explicit absolute paths); JSON-RPC frame log for diagnostic
  purposes (taps the wire underneath `ServiceClient`, not the primary
  test surface). There is no leading-slash auto-detection and no glob
  support in v1.
  M1/M2 implements `kill` and `pid_is_alive`. The other primitives in
  this bullet are target capabilities. Brokkr has similar helpers for
  its own cleanup path, but there is no brokkr/runtime control channel
  in v1.
- **Artefact-dir writers** - the frame log, event log, step trace,
  Service-specific `/proc` snapshot, data-dir copy, `service.stderr`,
  and `runtime-outcome.json` are populated by the harness module into
  the directory pointed at by
  `BROKKR_HARNESS_ARTEFACT_DIR`. Brokkr owns the directory lifecycle
  and already writes brokkr-owned files (`run.toml`,
  `binary-stdout.log`, `binary-stderr.log`, copied script, spawn error
  breadcrumb). The runtime must not claim ownership of those same files
  unless the contract is changed. `service.stderr` is a v1 artefact:
  the harness uses a Service spawn path that pipes the child Service's
  stderr to that file instead of inheriting it.
- **Per-script wall-clock backstop** - runaway scripts are bounded
  by a per-script wall-clock ceiling enforced around the whole run.
  Scripts may set frontmatter `-- ceiling: 60s`; omitted scripts use a
  sane default ceiling. This is a runaway guard for infinite loops or
  missing per-call backstops, not normal test control flow.
  Note: dellingr's per-opcode cost accounting is **not** used for
  this. dellingr's cost budget is structurally unable to bound
  wall-clock execution (`while true do end` is free, by design - the
  budget is for not penalising users-who-write-more-code in
  game-script consumers, not for catching infinite loops). Wall-clock
  is the right mechanism for runaway abort, same shape as every
  other backstop in the harness.
- **`app --test-harness <script.lua>` CLI flag** - the runtime's
  entry point, gated behind the existing `test-helpers` feature so
  production builds never carry the Lua VM.

## Contract surface

The entire contract between brokkr and the ratatoskr-side harness
runtime is small. Spelling it out so the implementer knows what they
do **not** need to integrate with.

**What brokkr passes the harness binary at spawn:**

- Argv: `app --test-harness <script.lua>` (the script path; brokkr
  resolves it relative to the project root before passing).
- Env var `BROKKR_HARNESS_ARTEFACT_DIR` - absolute path to the
  per-run artefact directory. The directory exists (brokkr creates
  it before spawning); the runtime writes its diagnostic artefacts
  into it.
- Env var `BROKKR_TEST_BIN_DIR` - absolute path to the directory
  containing the built `app` binary plus any sibling helpers
  (`parent_death_helper`, future stub binaries). The runtime reads
  this when it needs to spawn helper subprocesses; brokkr guarantees
  the binaries listed in `[[check]] build_packages` are present.
- The harness binary's stdout/stderr are piped by brokkr; brokkr writes
  them to `binary-stdout.log` and `binary-stderr.log` in the artefact
  dir after the process exits. The runtime can `println!` for
  human-readable progress, but that is not a structured protocol.

**What brokkr expects in return:**

- Exit code zero on test pass; non-zero on failure. Brokkr preserves
  the artefact dir on non-zero (or on signal) and deletes it on zero
  unless the user passed `--keep-artefacts`. There is no other
  out-of-band signaling - no JSON stdout protocol, no shared memory,
  no inotify on the artefact dir.
- Brokkr-owned artefacts (`run.toml`, copied script,
  `binary-stdout.log`, `binary-stderr.log`, `spawn-error.txt` on spawn
  failure) are written by brokkr. Runtime-owned artefacts
  (`frames.jsonl`, `events.jsonl`, `steps.jsonl`, `data-dir/`,
  `service.stderr`, `runtime-outcome.json`, and Service `/proc`
  snapshots) are written by the ratatoskr harness module. Brokkr does
  not parse runtime-owned artefacts in v1, just preserves them. Future
  failure-triage tooling may read them, but that tooling is brokkr-side
  and lives outside this v1 contract.

**What the harness module does NOT need to interact with:**

- Brokkr's history DB (`.brokkr/results.db`). Brokkr records the run
  outcome there automatically; the runtime is unaware of it.
- Brokkr's sidecar `/proc` profiler. Reusable later if a script
  opts into it, but not part of the v1 contract.
- Brokkr's worktree machinery. Worktrees aren't used for harness
  runs; brokkr-side tests run against the project root directly.
- Brokkr's lockfile coordination. The lock acquires before spawn and
  releases after wait; the runtime sees neither.
- Brokkr's visual reference testing, project download/verify
  pipelines, or any other capability not listed in "What brokkr
  passes" above.
- Brokkr's source code. The runtime never imports anything from
  brokkr; the contract is process-level, not source-level.

If you find yourself wanting more from brokkr than argv, env vars,
stdout/stderr capture, brokkr-owned artefacts, and exit status, **stop
and surface it as an explicit design change** rather than reaching
across the contract. Both this document and
`notes/ratatoskr-service-harness.md` in the brokkr repo would need to
change in lockstep.

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
from "one helper inside ratatoskr" toward "the default wait shape
exposed through the Lua binding." M1/M2 does not yet expose the generic
Lua `wait_for` combinator; the wedge uses typed `ServiceClient`
requests, event-stream receives, async request handles, and generous
per-call timeouts. The future generic combinator should keep the loop
in the harness module so `ServiceClient` remains a pure protocol layer.
Wall-clock must never become the primary signal.

Backstops are still wall-clock - the harness can't escape physical
time entirely - but they are explicit, named, generous, and never the
primary signal. The API has two separate wait shapes:

- `harness.wait_for { predicate, child, backstop }` is target API for
  positive waits that should complete through a predicate or child-exit
  transition. Safety-backstop firing is a test-design or
  implementation-determinism bug, not a flake.
- M1/M2 exposes the narrower `harness.expect_quiet(events, seconds)`
  used by the wedge scripts. The target
  `harness.expect_quiet { predicate, child, window }` shape remains the
  full absence assertion API.

## Test scripts

Tests are Lua scripts. Ratatoskr's `app` crate embeds the `dellingr`
Lua VM and exposes its existing `crates/app/src/service_client.rs` API
as Lua userdata in the harness module. Brokkr never embeds dellingr;
it spawns the ratatoskr-side runtime via
`app --test-harness <script.lua>`. Adding a test means adding a
`.lua` file in ratatoskr's tree; no brokkr rebuild, and no
harness-module rebuild either unless the new test exercises a Lua API
surface that does not exist yet.

The landed dependency is `dellingr 0.2.0`.

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

The existing and planned test bodies call these. M1/M2 wraps the
subset needed by the wedge (`spawn`, `spawn_with_events`, `request`,
`request_async`, `shutdown`, `current_generation`, `child_pid`,
`drop`). Names below are Rust; Lua spelling is whatever the binding
picks as each capability lands.

- `spawn_for_test(binary, data_dir, extra_args) -> Arc<ServiceClient>`
- `spawn_with_events_for_test(binary, data_dir, extra_args) -> mpsc::Receiver<SpawnEvent>`
- `request(method, params) -> Result<table, ClientError>` - registry-
  backed Lua-facing shape. The registry covers the typed
  `RequestParams` variants (`HealthPing`, `Shutdown`, `TestPrintln`,
  `TestSlow`, `ActionExecutePlan`, `ActionJobStatus`,
  `ActionMarkChatRead`, etc.; the `Action` prefix is canonical in the
  Rust enum and the binding strips or maps it as it chooses for Lua
  spellings). Rust decodes the typed response and converts it to a Lua
  table.
- `notifications() -> Arc<NotificationQueue>` - target capability;
  returns a queue userdata with `recv(timeout)` and drain helpers.
- `current_generation() -> u32`
- `child_pid() -> Option<u32>`
- `shutdown() -> Result<(), ClientError>`
- `drop(client)` - explicitly invocable from Lua to test the Drop
  teardown path.

`SpawnEvent` becomes Lua userdata with three case constructors:
`ChildSpawned { client }`, `BootReady { response }`,
`Terminal { error }`. `ClientError` becomes Lua userdata with
case-discriminating accessors (`is_service_crashed()`,
`boot_classification()`, `schema_version_changed()` returning
`{ was, now }`, etc.) so scripts can pattern-match the way the
existing tests do.

### Process and connection control

- **`drop(client)`** - exercises the explicit Drop teardown path.
- **PID-existence polling** - `harness.pid_is_alive(pid)` mirroring
  ratatoskr's `pid_is_alive` test helper.
- **Stub-parent helper invocation** - target capability. The
  `parent_death_helper`
  binary already exists in ratatoskr (registered as
  `CARGO_BIN_EXE_parent_death_helper`). The harness builds it
  alongside `app` and exposes
  `harness.spawn_parent_death_helper(service_binary, data_dir)
  -> { service_pid, helper_handle }`. Required for
  `linux_parent_sigkill_terminates_service_within_two_seconds`.
- **Send signal** - `harness.kill(pid, signal)` for SIGKILL/SIGTERM
  to a captured PID.
- **Respawn** - already a `ServiceClient` capability via
  `spawn_with_events_for_test` + SIGKILL + waiting for follow-up
  events on the same receiver. Lua scripts express it the same way
  the existing tests do; the harness provides nothing new.

### Assertion shapes

- **Pattern-match on `SpawnEvent`** - distinguish `ChildSpawned` vs
  `BootReady` vs `Terminal`, with `Terminal` carrying a
  `BootClassification` the script can compare against named
  constants (`BootExitCode::KeyLoadFailure`,
  `BootExitCode::AnotherInstanceRunning`, etc.).
- **Pattern-match on `ClientError`** - `Io`, `Service`,
  `ServiceCrashed`, `Timeout`, `VersionMismatch { ui, service }`,
  `BootFailure { classification }`,
  `SchemaVersionChanged { was, now }`.
- **Arc identity across respawn** -
  `harness.same_client(a, b) -> bool` exposes `Arc::ptr_eq`. Used by
  `respawn_after_sigkill_succeeds` to assert the in-place state
  swap.
- **Notification-queue introspection** -
  `queue:drain_for(duration) -> [Notification]`, with
  `Notification:service_generation()` exposing the tag.
- **Absence over observation window** - "no event received in N seconds,
  after a known transition." M1/M2 scripts use
  `harness.expect_quiet(events, seconds)`. The target shape is
  `harness.expect_quiet { predicate, child, window }`; the window
  expiring is the expected success verdict, not a harness-timeout
  failure.
- **Cardinality** - "exactly N notifications matching predicate."
  Lua expresses with `drain_for` + `#table` + `assert`.
- **Counter probe with delta** - `client:request("TestCounterRead",
  ...)` called before and after, with Lua subtraction. No new harness
  primitive needed beyond the test-helper RPC.
- **Resource budget** - peak RSS, IO bytes from sidecar samples
  during a script's lifetime. Reuse brokkr's existing sidecar; expose
  `harness.resource_summary(client) -> { rss_kb, io_bytes, ... }`.
- **Child exit** - `harness.wait_exit(client, backstop) -> ExitStatus`
  with `code()`, `signal()`, `wall_time_ms()`.
- **Time-floor assertion** - "drop took at least N ms before
  escalating to kill." A simple `os.time` delta in Lua.

### Determinism scaffolding

- **Wait combinator** - target API `harness.wait_for { ... }`.
  Composes a
  predicate against `client:observe_child_exit()` so any Service
  death short-circuits the wait with a "child exited while awaiting
  X" verdict.
- **Sentinel-file watch** - target API:
  `harness.wait_for_sentinel { path = "clean_shutdown", backstop = "5s" }`
  for data-dir-relative paths, or
  `harness.wait_for_sentinel { absolute = "/var/run/foo", backstop = "5s" }`
  for explicit absolute paths. Required for the `clean_shutdown`
  sentinel in the manual-matrix items moving into automation; available
  for any future test that benefits from a non-clock readiness signal.
- **Frame log** - captured under the hood via `ServiceClient`'s wire
  layer (the harness taps stdin/stdout). Diagnostic only; emitted to
  the artefact dir on failure. Tests do not pattern-match on raw
  frames - they pattern-match on `ServiceClient` return values and
  `Notification` userdata.
- **Backstop policy** - explicit, named, generous. Safety-backstop
  firing in `wait_for` is a test-design bug, not a flake.
  Observation-window expiry in `expect_quiet` is a success verdict for
  absence assertions.

### Cohort coverage

| Test | Surface used |
| --- | --- |
| `dispatch_in_process.rs` tests using `spawn_harness_with_suffix` | Migrates as a cohort because they share the same IO-boundary wait failure mode even though they use `tokio::io::duplex` instead of an OS subprocess. V1 rewrites the boot/dispatch lifecycle coverage onto the real-subprocess `ServiceClient` path; in-process Lua mode is deferred until a future test needs it. |
| `boot_ready_returns_after_sequence_completes` | Current example of the cohort failing under `brokkr check`; needs frame/step trace around `boot.ready`, boot shared state, and shutdown drain rather than only an outer libtest timeout. |
| `health_ping_succeeds_during_long_migration` / `health_ping_works_concurrently_with_boot_ready` | Need concurrent request driving while `boot.ready` is parked; Lua API may need explicit background request / parallel request primitive, not just sequential `client:request`. |
| `boot_ready_blocks_until_sequence_completes` / `boot_progress_notifications_emitted_in_order` (currently ignored) | Existing in-process harness hangs; migrate with the same diagnostic artefact contract as the subprocess wedge. |
| `service_subprocess_ping_and_shutdown` | M2 landed `crates/app/tests/service-harness/ping_and_shutdown.lua`; the old libtest body is now an ignored pointer stub. |
| `dropping_client_terminates_child_within_one_second` | `spawn_for_test`, `child_pid`, `drop(client)`, `pid_is_alive` poll. |
| `spawn_failure_against_missing_binary_returns_io_error` | `spawn_for_test` against bogus path, expect `ClientError::Io`. |
| `linux_parent_sigkill_terminates_service_within_two_seconds` | `parent_death_helper` invocation, PID handoff via stdout, `kill(helper, SIGKILL)`, `pid_is_alive` poll on Service PID. |
| `println_from_handler_does_not_corrupt_json_rpc_framing` | `spawn_for_test`, `request("TestPrintln")`, `request("HealthPing")`, two-step round-trip. |
| `version_mismatch_surfaces_during_handshake` | `spawn_for_test` with `--test-fake-version=999`, expect `ClientError::VersionMismatch { ui, service }`. |
| `pending_request_fails_with_service_crashed_when_child_killed` | `request("TestSlow", 60_000)` in background, `kill(pid, SIGKILL)`, expect `ClientError::ServiceCrashed`. |
| `spawn_with_events_emits_child_spawned_then_boot_ready_on_healthy_boot` | M2 landed `two_phase_spawn.lua`; the old libtest body is now an ignored pointer stub. |
| `spawn_with_events_emits_terminal_on_missing_key` | M2 landed `terminal_on_missing_key.lua`; the old libtest body is now an ignored pointer stub. |
| `missing_key_file_exits_with_key_load_failure_code` | direct subprocess, hold stdin, `wait_exit` with code 73. |
| `second_instance_against_same_data_dir_exits_with_already_running` | two parallel children, drive A's IPC, B's `wait_exit` with code 71. |
| `spawn_with_events_classifies_another_instance_running` | A direct, B via events, expect `Terminal { BootFailure { AnotherInstanceRunning } }`. |
| `respawn_after_sigkill_succeeds` | M2 landed `respawn_after_sigkill.lua`; the old libtest body is now an ignored pointer stub. |
| `pending_request_fails_at_respawn_then_subsequent_succeeds` | M2 landed `pending_at_respawn.lua`; the old libtest body is now an ignored pointer stub. |
| `terminal_failure_at_initial_boot_does_not_respawn` | `spawn_with_events`, `Terminal`, post-Terminal absence-over-window. |
| `crashloop_threshold_emits_terminal_after_third_crash` | loop of `kill` + observe `ChildSpawned + BootReady`, third kill expects `Terminal`. |
| `stale_notifications_dropped_after_generation_bump_end_to_end` | `notifications()`, `current_generation()`, drain across kill+respawn, generation-tag check. |
| `deadlocked_service_drop_escalates_to_kill` | `spawn_for_test --test-hang-on-stdin-eof`, `drop`, `pid_is_alive` poll with time-floor + ceiling. |
| `pre_ack_crash_*` / `post_ack_crash_*` (Phase 8 cohort) | `request("ExecutePlan")`, fault-inject via test-helper RPC, kill, respawn, follow-up `request`, `Notification` drain. |
| `retry_queue_persists_across_respawn` | M4 landed `test.pending_ops_read`, a test-only `harness-offline` provider, and a real-subprocess `action.execute_plan` retry-queue script; 50/50 focused soak passes. Full T1 directory soak also passed 550/550 across 50 cohort cycles. |
| `compose_send_50mb_attachment` | Blocked on the mock SMTP path in `../sæhrimnir`: the harness can call `action.send`, but cannot yet exercise and assert the actual network send. |
| `bulk_archive_200_threads_under_budget` | Lua loop dispatching 200 `request` calls in parallel, wall-clock budget assertion via `os.time`. |
| `mark_chat_read_emits_only_action_completed` | `request("MarkChatRead")`, `notifications():drain_for`, cardinality-1 assertion. |
| `action_skips_search_index_write` / `handler_does_not_drive_batch_execute` | `request("TestCounterRead", ...)` before/after, Lua subtraction. |
| `journal_replays_after_respawn` / `stale_outcomes_dropped_after_respawn` | `request` + `kill` + respawn + `notifications` drain. |
| `test_fake_schema_propagates_via_terminal` | `spawn_with_events_for_test` first run + `kill` + respawn with `--test-fake-schema=N`, expect `Terminal { SchemaVersionChanged { was, now } }`. |
| `sync-harness/jmap-initial` | `test.seed_account` with provider `jmap`, `client:start_sync`, `test.query_db_state` assertion over the `jmap-small` mock fixture. |
| Manual matrix #4 (heartbeat detects killed Service) | `spawn_with_events_for_test`, `kill(service_pid, SIGKILL)`, `wait_for_sentinel { path = "logs/heartbeat-exiting", backstop = "30s" }` or follow-up event. |
| Manual matrix #5 (SIGTERM triggers shutdown drain) | `spawn_for_test`, `kill(pid, SIGTERM)`, `wait_for_sentinel { path = "clean_shutdown", backstop = "5s" }`, `wait_exit`. |

If a capability above doesn't have a Lua binding, the binding is
incomplete; extend it. The harness binding never reimplements
`ServiceClient` behaviour - it forwards.

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
surface. New `RequestParams` variants become script-visible by adding
entries to the harness request/response registry.

Provider mock endpoints are test-scoped env vars, consumed only when
the relevant provider crate is compiled with `test-helpers`:

```
RATATOSKR_TEST_JMAP_ENDPOINT=http://127.0.0.1:<jmap-port>
RATATOSKR_TEST_IMAP_ENDPOINT=127.0.0.1:<imap-port>
RATATOSKR_TEST_SMTP_ENDPOINT=127.0.0.1:<smtp-port>
RATATOSKR_TEST_GRAPH_ENDPOINT=http://127.0.0.1:<graph-port>
RATATOSKR_TEST_GMAIL_ENDPOINT=http://127.0.0.1:<gmail-port>
```

JMAP endpoints are passed as base URLs and the JMAP client discovers
`/.well-known/jmap`; Graph origins map to `/v1.0` and `/beta`; Gmail
origins map to `/gmail/v1/users/me`; IMAP and SMTP expect host:port.
This lets brokkr pass per-run mock ports without changing persisted
account config.

The sync-harness request surface is:

- `test.start_sync` / `TestStartSync` - starts the real Service sync
  runtime for an account. The Service sync dispatcher runs provider
  initial sync when `accounts.initial_sync_completed = 0`, then
  provider delta sync afterwards.
- `test.query_db_state` / `TestQueryDbState` - returns account,
  label, thread, message, unread-message, attachment, and bounded
  message-list snapshots for assertions.

Lua sync scripts that need the terminal result should call
`client:start_sync({ account_id = ... })`. That routes through
`ServiceClient::start_sync`, which uses the same waiter map as the UI;
raw `sync.completed` frames are consumed there and are not delivered
to `client:notifications()` queues.

## Deterministic app-data fixtures

Brokkr does not create app-data directories itself - Lua scripts do.
M1/M2 exposes `harness.data_dir(suffix, with_key)` for simple Service
boot fixtures; it creates a per-run app-data directory and, by default,
a deterministic non-zero `ratatoskr.key`. The missing-key wedge passes
`false`.

The later fixture setup path, likely via `RequestParams::TestSeedAccount`
(or equivalent), should create:

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
(`BROKKR_MARKER_FIFO`). V1 ships without lifecycle markers. When M9
sync benchmarks need marker timing, the harness routes through
`BROKKR_MARKER_FIFO` rather than inventing a parallel marker protocol.

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

## Trace schema

Trace files are JSONL with versioned records. M1/M2 emits these
records directly from the harness runtime and `ServiceTraceSink`;
there is not yet a separate `trace_schema.rs` module or reader crate.
When failure-triage tooling starts reading these files, factor the
schemas into serde structs and keep readers tolerant of unknown fields
for forward compatibility. Each top-level record currently carries
`schema = 1`.

`frames.jsonl` records:

```
{
  "schema": 1,
  "ts_ms": 123,
  "direction": "in" | "out",
  "raw_redacted": "...",
  "raw_len": 456,
  "raw_sha256": "...",
  "parsed": { ... } | null
}
```

Both `raw_redacted` and `parsed` follow the same redaction policy. V1's
wedge cohort uses fake credentials, but the field names are redaction-
ready from day one: the writer emits `raw_redacted`, never `raw`.
Structural redaction is the default posture for strings above a chosen
threshold (`<redacted len=N>`). A per-`RequestParams` field allowlist is
the long-term refinement before any credentialed script lands.

Current M1/M2 limitation: `parsed` is always `null`. The raw redacted
frame, length, and SHA-256 are present and are the supported diagnostic
surface for the wedge.

`events.jsonl` records:

```
{ "schema": 1, "ts_ms": 123, "event": { ... } }
```

`event` is the typed `SpawnEvent` serialization.

`steps.jsonl` records:

```
{
  "schema": 1,
  "ts_ms": 123,
  "step": "script supplied label",
  "kind": "spawn" | "request" | "expect_quiet" | "...",
  "transition": "started" | "ok" | "error" | "quiet" | "event" | "..."
}
```

The target schema will add predicate descriptions, duration fields,
and richer transition naming when generic wait combinators land.
Failure-triage tooling can provide feedback once it exists.

## Failure model

When a test fails, brokkr preserves a self-contained artefact directory
at `.brokkr/ratatoskr/<test-name>/<run-N>/`. Brokkr creates the
directory and writes brokkr-owned run metadata; the ratatoskr runtime
writes Service/runtime diagnostics into that directory. The target
contents are:

- **`frames.jsonl`** - every JSON-RPC frame, both directions,
  timestamped from spawn. Single most useful artefact for
  drain-ordering / framing bugs.
- **`events.jsonl`** - every spawn/runtime event observed
  (`ChildSpawned`, `BootReady`, `Terminal`), timestamped.
- **`steps.jsonl`** - the test's step trace: which step was active,
  what condition was awaited, which transition fired.
- **`service.stderr`** - Service's stderr verbatim. Captured
  per-run, not race-mingled with test stdout. V1 requires a
  harness-specific Service spawn path that pipes stderr to this file,
  because today's `ServiceClient::launch_subprocess` inherits stderr.
- **`proc-{status,wchan,syscall,stack}.txt`** - best-effort snapshot of
  `/proc/<pid>/status`, `/proc/<pid>/wchan`, `/proc/<pid>/syscall`,
  `/proc/<pid>/stack` at the moment failure was declared.
  Distinguishes "blocked on futex" from "blocked on closed pipe"
  without re-running.
- **`data-dir/`** - copy of the test's app-data dir at failure time.
  SQLite WAL state, lockfile presence, key file, `clean_shutdown`
  sentinel.
- **`runtime-outcome.json`** - runtime-side exit reason
  (clean / harness-killed-on-backstop / child-exited / signal / etc.).
- **`run.toml`** - brokkr-owned reproducibility metadata: test script,
  env vars, brokkr version, git commit, sweep label, exit code/signal.

On success, the artefact directory is deleted unless
`--keep-artefacts` is passed.

The data dir copy, protocol/step trace, and `/proc` snapshot for real
subprocesses are the pieces of state that today's tokio-test pattern
destroys or never records (`DataDirGuard::Drop` / `TestDataDir::Drop`
unconditional cleanup; no structured frame/step capture). Recovering
them is the largest single jump in debug ergonomics.

## Brokkr CLI surface

Top-level commands tagged `[ratatoskr]`, project-gated to
`Project::Ratatoskr` (a brokkr-side enum variant). Same convention as
brokkr's other project-gated tags.

```
brokkr service-test <SCRIPT>
brokkr service-test <SCRIPT> -N 200       # single-script soak
brokkr service-test <DIR> -N 50           # cohort cycles
brokkr service-suite [--filter X] [-N 50]
brokkr service-list
brokkr mock-serve --fixture <NAME>
```

Brokkr does not embed the Lua VM or `ServiceClient`; it never speaks
JSON-RPC over the wire.

Cargo / libtest remains the place for unit tests and for tests that
do not need deterministic Service IO-boundary orchestration. The new
harness is specifically for tests where the Service lifecycle,
boot/dispatch protocol, or process/IO boundary is the thing under test.

## Out of scope

- **Replacing `brokkr test`.** The cargo single-test runner stays.
  Service IO-boundary tests use the new harness; everything else uses
  `brokkr test`.
- **Fixing the underlying Service bugs.** The harness exists to make
  those bugs deterministic and diagnosable, not to hide them. The
  Service-side Phase 8 work owns the drain-ordering / class-aware
  emit / crashloop fixes.
- **Migrating every existing tokio test.** The new harness coexists
  with libtest. Unit-style `#[tokio::test]` functions stay there. The
  migration target is the Service IO-boundary cohort: the flaky
  `service_subprocess.rs` wedge, the `spawn_harness_with_suffix`
  dispatch tests, and new tests that need deterministic protocol waits
  plus artefact dumps.
- **Owning the mock servers.** Track 2 reuses the artefact / lockfile
  / history machinery and the harness binary, but protocol mocks live
  in `../sæhrimnir` and orchestration lives in brokkr. Ratatoskr owns
  only the test-only endpoint overrides, sync-trigger requests, and
  sync-harness scripts.
- **CI-only features.** First user is a local developer
  root-causing a flake. CI integration follows once the local story
  works.

## Open questions

- **Fixture-setup home.** Should fixture setup live in
  `crates/dev-seed`, a new test-fixtures crate, or `service` test
  helpers?
