# Brokkr + Ratatoskr Phase 8 scaffolding

Status: **architectural commitment, pre-Phase 8.** This is not the
Phase 8 technical implementation plan. It records the shape Brokkr
and Ratatoskr have agreed on for the Service-test work, so when
Phase 8 planning starts the structural choices are not relitigated.

Replaces an earlier draft that framed Brokkr's role as "a smarter
cargo test runner around the existing `#[tokio::test]` cohort."
That framing was downgraded after review (see
`~/Programs/brokkr/notes/ratatoskr-service-harness.md` for the
detailed design): the existing tests in
`crates/app/tests/service_subprocess.rs` are structurally racy by
construction - wall-clock timeouts inside the test race against
the implementation's own timeout ceilings - and re-running them
under a soak loop averages the noise rather than eliminating it.
The replacement architecture lifts the test shape into a
deterministic predicate-or-backstop runtime hosted inside
Ratatoskr's `app` crate, with Brokkr providing build / lockfile /
artefact / history orchestration around it.

## Why this exists

In Ratatoskr today, Brokkr is mostly treated as a better `cargo test` /
`cargo clippy` wrapper. That is a useful baseline, but it is not what
Brokkr is in the projects it was built for.

In pbfhogg, elivagar, nidhogg, litehtml-rs, and sluggrs, Brokkr is a
project-aware command orchestrator. It knows how to build the right
binary, run project-specific commands, enforce locks and timeouts,
record timings, attach `/proc` sidecar profiling, preserve results, and
make failures repeatable. It is the outer harness around the project,
not just a cargo wrapper.

Ratatoskr's Service architecture creates exactly the kind of work that
needs an outer harness:

- real subprocess lifecycle tests;
- boot handshakes and terminal boot failures;
- crash, respawn, and stale-notification behavior;
- hung child cleanup;
- long-running sync workloads;
- deterministic provider fixtures;
- performance regression thresholds.

Phase 8 is the right moment to switch gears and make Brokkr part of the
Service quality story.

## Assumed Ratatoskr state

This document assumes the Service plan through Phase 7 has landed.

The intended architecture at that point:

- The iced UI process owns rendering, input, UI state, action planning,
  read-side SQLite queries, search reads, and content-store reads.
- The Service process is a child process launched by the UI.
- UI and Service communicate over JSON-RPC stdio.
- The Service is the only writer.
- Service owns action execution, sync, pending-ops recovery, push
  receivers, DB writes, body/inline/blob store writes, Tantivy writes,
  attachment extraction, and long-running background work.
- Service boot is load-bearing. UI uses the two-phase spawn flow:
  `ChildSpawned` after version ping, then `BootReady` after schema, key,
  recovery, and writer initialization.
- Deterministic boot failures such as `AnotherInstanceRunning`,
  `MigrationFailure`, and `KeyLoadFailure` are terminal and must not
  respawn.
- Runtime crashes after `BootReady` are eligible for respawn.
- Service-generation tags prevent stale notifications from a dying
  incarnation reaching live UI state.

## The Brokkr/Ratatoskr split

Two related but separable features. Track 1 lands first; Track 2
re-uses the artefact / lockfile / history machinery but is otherwise
independent.

The load-bearing rule across both tracks: **Brokkr does not depend on
Ratatoskr at the source level, and Ratatoskr does not depend on
Brokkr.** Both directions are off the table. Cross-process
communication is by subprocess spawn + env vars only. This frees
Brokkr from a tokio dependency it would otherwise need (the harness's
async stdio + child-exit + timeout dance lives entirely inside
Ratatoskr) and frees Ratatoskr from any churn in Brokkr's release
cadence.

### Track 1: Deterministic Service-subprocess harness

Goal: replace the structurally-racy `#[tokio::test]` shape with a
deterministic runtime where every wait races a predicate against the
child process exiting plus a named backstop. First transition to fire
wins; the harness records which one fired in the test trace. Backstops
remain wall-clock - the harness cannot escape physical time entirely -
but they are explicit, named, generous, and only fire when a
determinism bug elsewhere leaves the harness with nothing else to wait
on. A backstop firing is a test-design bug, not a flake.

The runtime lives inside Ratatoskr's `app` crate, exposed via a new
CLI flag:

    app --test-harness <script.lua>

The harness module:

- embeds the `dellingr` Lua VM (pure Rust, currently 0.1.0 on
  crates.io; no FFI, no system Lua dep);
- registers `ServiceClient`, `SpawnEvent`, `ClientError`, and
  `NotificationQueue` as Lua userdata, one method per Rust method,
  with case-discriminating accessors on the enum types;
- exposes process-tree primitives (`kill`, `pid_is_alive`,
  `wait_for_sentinel`) and the `wait_for { predicate, child,
  backstop }` combinator that races predicates against
  `client:observe_child_exit()` internally;
- writes per-run diagnostic artefacts (frame log, event log, step
  trace, `/proc/<pid>/{wchan,status,syscall,stack}` snapshot,
  data-dir copy) to a Brokkr-supplied artefact directory;
- aborts runaway scripts via dellingr's instruction-cost ceiling.

Tests are `.lua` files in `crates/app/tests/service-harness/`. The
first cohort:

- the two currently-`#[ignore]`'d real-subprocess tests
  (`service_subprocess_ping_and_shutdown`,
  `spawn_with_events_emits_terminal_on_missing_key`), re-expressed as
  Lua scripts. Re-enabling these stable is the wedge that proves the
  architecture works.
- the Phase-8-named additions
  (`pre_ack_crash_rolls_back_subprocess`,
  `journal_replays_after_respawn`, `compose_send_50mb_attachment`,
  `bulk_archive_200_threads_under_budget`, the schema-fake e2e, the
  T1 cohort, etc.).
- manual matrix items 4 and 5 (heartbeat-detects-killed-Service,
  SIGTERM-triggers-shutdown-drain), which today sit in manual-only
  because they are "too noisy to assert reliably from automation" - a
  deterministic harness pulls them in.

Brokkr's commands (top-level, project-gated to `Project::Ratatoskr`):

    brokkr service-test <SCRIPT>
    brokkr service-test <SCRIPT> -N 200       # soak
    brokkr service-suite [--filter X]
    brokkr service-list

Brokkr orchestrates: project gating, sweep-aware build of the `app`
binary via `[ratatoskr.harness]` in `brokkr.toml`, lockfile, per-run
artefact directory naming
(`.brokkr/ratatoskr/<test>/run-N/`, preserve-on-failure,
delete-on-success-unless-`--keep-artefacts`), history-DB recording,
soak loops, suite filtering. Brokkr does not embed the Lua VM or
`ServiceClient`; it never speaks JSON-RPC over the wire.

Cargo / libtest remains the place for unit tests and for tests that
do not need the real-subprocess shape. The new harness is specifically
for tests where the subprocess lifecycle is the thing under test.

### Track 2: Deterministic provider mock servers

Goal: make provider sync correctness and performance testable against
local deterministic IMAP/JMAP fixtures.

This track is related to the Service harness because sync runs inside
the Service and uses the same app-data, process, logging, timeout, and
artifact machinery. It is still a separate problem: the hard part is the
fixture and protocol model, not repeating a cargo test.

Candidate future Brokkr commands:

```text
brokkr mock-serve --imap --jmap --fixture small
brokkr sync-smoke --fixture jmap-small
brokkr sync-bench --fixture imap-100k
brokkr sync-bench --fixture jmap-incremental --bench 10
```

Brokkr orchestrates these servers. It does not itself become the IMAP
or JMAP implementation. The mock-server design lives in a sibling
note (`~/Programs/brokkr/notes/ratatoskr-mock-server.md`) and may
move to its own repository.

## What Brokkr provides

For Service tests:

- `Project::Ratatoskr` gating + `[ratatoskr]` command tag in
  `brokkr --help`.
- Sweep-aware build of the `app` binary via `[ratatoskr.harness]` in
  `brokkr.toml`. `sweep` references a `[[check]]` entry, `binary`
  names the cargo package whose binary the harness spawns. Same
  feature contract `brokkr check` enforces, so a script can never run
  against a feature combination the rest of the toolchain has not
  validated.
- Per-run artefact directory lifecycle: collision-incrementing
  `run-N`, preserve-on-failure, delete-on-success-unless-keep,
  preserve-on-panic (Drop default). The directory path is exported to
  the harness binary via `BROKKR_HARNESS_ARTEFACT_DIR`.
- Process-tree primitives: signal, pid_is_alive, sentinel-watch with
  named backstop, /proc snapshot tolerant of missing files
  (`stack` often needs CAP_SYS_PTRACE).
- Script discovery: walks `crates/app/tests/service-harness/*.lua`,
  parses a top-of-file `-- key: value` frontmatter (`description`,
  `expected = pass | ignored`), prints a sorted table.
- Soak (`-N`) and suite (`--filter`) runners on top of the
  single-script run path.
- History-DB recording, optional sidecar `/proc` profiling, eventual
  comparison against stored baselines.

For provider sync workloads:

- isolated app-data directories;
- local port allocation;
- mock-server process startup and teardown;
- fixture selection;
- passing endpoint credentials into Ratatoskr via env vars;
- sync workload execution;
- result storage;
- wall-time, RSS, I/O, request-count, and DB-size tracking;
- threshold-based regression failures.

Brokkr does NOT provide:

- Ratatoskr correctness assertions;
- direct knowledge of Ratatoskr's DB schema or Service IPC types;
- the Lua VM (lives in `app` via `dellingr`);
- a JSON-RPC protocol implementation (`ServiceClient` lives in
  `app`);
- a tokio dependency in Brokkr (sync subprocess spawn + wait
  suffices for orchestration);
- the provider mock implementation itself.

## What Ratatoskr provides

The Ratatoskr side of this collaboration is to expose stable, narrow
surfaces that Brokkr can run, plus the harness module itself.

### Harness module in `app` crate

Phase 8's largest single Ratatoskr-side build. The module:

- depends on `dellingr` (pure Rust Lua VM, currently 0.1.0 on
  crates.io);
- adds an `app --test-harness <script.lua>` CLI flag, gated behind
  the existing `test-helpers` feature so production builds never
  carry the runtime;
- registers `ServiceClient` and friends as Lua userdata via
  `RustFunc` wrappers (`spawn_for_test`,
  `spawn_with_events_for_test`, `request<R>`, `notifications`,
  `current_generation`, `child_pid`, `shutdown`, explicit `drop`);
- registers `SpawnEvent`, `ClientError`, `Notification` as Lua
  userdata with case-discriminating accessors so scripts can
  pattern-match the way the existing `#[tokio::test]` functions do;
- registers process-tree primitives and the `wait_for { predicate,
  child, backstop }` combinator that races against
  `client:observe_child_exit()`;
- writes per-run diagnostic artefacts to the directory pointed at by
  `BROKKR_HARNESS_ARTEFACT_DIR`:
  - `frames.jsonl` - every JSON-RPC frame, both directions,
    timestamped from spawn;
  - `events.jsonl` - every `SpawnEvent` observed, timestamped;
  - `steps.jsonl` - the test's step trace: which step was active,
    what condition was awaited, which transition fired;
  - `proc-{wchan,status,syscall,stack}.txt` - `/proc/<pid>/`
    snapshot at failure-declaration time;
  - `service.stderr` - Service's stderr verbatim, per-run;
  - `data-dir/` - copy of the test's app-data dir at failure time;
  - `exit.txt` - exit code, signal, wait time, exit reason;
  - `run.toml` - script path, env vars, brokkr version, git commit;
- aborts runaway scripts via `dellingr`'s instruction-cost ceiling.

Adding or changing a test means adding a `.lua` file in
`crates/app/tests/service-harness/`. Changing the Lua API surface (the
userdata methods exposed) is the only thing that requires recompiling
the harness module.

### Stable Service entrypoints

`app --service --app-data-dir <dir>` must remain a stable subprocess
entrypoint.

Test-only flags should be kept intentional and documented. Existing
examples include:

- fake protocol version;
- fake schema version;
- slow request;
- hang-on-stdin-EOF;
- println/framing canary.

More Phase 8 hooks may be needed (`--test-fake-schema=N`,
fault-injection variants, counter probes); they should remain
explicitly test-scoped, ideally behind the existing `test-helpers`
feature surface, and should expose `RequestParams` variants the harness
can pick up automatically (no harness recompile per new variant).

### Deterministic app-data fixtures

Brokkr does not create app-data directories itself - the Lua scripts
do, via `RequestParams::TestSeedAccount` (or whatever the Phase-8 name
ends up being). Ratatoskr should provide a fixture setup path that can
create:

- `ratatoskr.key`;
- migrated main SQLite schema;
- required accounts and labels;
- rows needed by FK-constrained action tests;
- empty or seeded body/inline/blob/search stores as needed.

This must not rely on the dev app's "wipe and seed on every launch"
behavior. Some tests need to shut down and respawn against the same
data directory. `crates/dev-seed/` may be a useful source of
deterministic data generation patterns, but test fixtures should be a
deliberate API, not an accidental dependency on dev startup behavior.

### Machine-readable lifecycle markers

Brokkr can measure much more accurately when Ratatoskr emits stable
phase markers or counters. The sidecar already supports a marker FIFO
(`BROKKR_MARKER_FIFO`); the harness module can either route through
it or write its own log entries.

Useful lifecycle spans:

```text
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

Timings should not require Brokkr to scrape human log lines.

### Artifact-friendly logging

Service logs already belong under `<app_data>/logs/`. For Brokkr-driven
runs, those logs should reliably include:

- Service PID;
- app-data dir;
- protocol and schema version;
- boot phase transitions;
- request ids and method names;
- terminal boot failure classification;
- crash and respawn reason;
- shutdown path and escalation reason.

Logs must not include sensitive payloads, OAuth codes, tokens, message
bodies, or attachment contents.

### Headless sync trigger

Provider benchmarks should not require rendering the iced UI.

Ratatoskr should expose one stable way to trigger sync headlessly. The
plausible options:

- a Service IPC method used only by tests/benchmarks;
- a small headless benchmark binary;
- a `.lua` script invoked through `app --test-harness` (the harness
  binary from Track 1) that calls a test-helper `RequestParams`
  variant on the Service.

Brokkr's plan 3
(`~/Programs/brokkr/notes/ratatoskr-sync-orchestration.md`) has
converged on the third option - it reuses Track 1's harness binary,
`ServiceClient` userdata, and artefact-dir machinery, so the only
new ratatoskr-side surface needed is the sync-triggering /
state-querying `RequestParams` variants themselves (e.g.
`TestStartSync`, `TestQueryDbState`; final names are Phase-8 design
work). Phase 8 can revisit if a need for the other two options
surfaces.

The requirement is that Brokkr can start local mock servers, point
Ratatoskr accounts at them, trigger sync, wait for completion, and get
a machine-readable summary.

### Correctness assertions stay in Ratatoskr

Brokkr should not know Ratatoskr's schema. Ratatoskr tests or helper
code should assert final state, such as:

- account count;
- folder and label count;
- message and thread count;
- body-store coverage;
- attachment and inline-image coverage;
- read/star/archive/delete state;
- pending-op state;
- sync cursor state.

Lua scripts in the harness drive these assertions through
`ServiceClient` methods - they speak the same Rust-level API the
existing `#[tokio::test]` bodies do, just from a script rather than a
test function.

For benchmark-style workloads, Ratatoskr should return a compact JSON
summary that Brokkr can store alongside timing metrics.

## Provider mock expectations

The first provider-mock milestone should be intentionally small.

IMAP first is probably easier for sync realism:

- login/auth;
- list/select folders;
- UID fetch;
- flags;
- UIDVALIDITY;
- deterministic folder state;
- deterministic changes between sync passes.

JMAP is broader but still feasible as a bounded subset:

- session object;
- Mailbox/get;
- Email/query;
- Email/get;
- Email/changes;
- Email/set where needed;
- Thread/get if Ratatoskr's path requires it.

Initial fixtures should favor coverage over perfect server realism:

- small smoke mailbox;
- medium mailbox;
- large mailbox;
- huge thread;
- many folders/labels;
- duplicate Message-ID;
- malformed MIME;
- slow/paged responses;
- incremental new/change/delete/move sequence.

The mock server and fixture model live in a separate repository (or a
sibling crate, TBD); Brokkr starts it, chooses fixtures, sets ports,
and collects results. See
`~/Programs/brokkr/notes/ratatoskr-mock-server.md`.

## Metrics and regression gates

Once Brokkr has stable Ratatoskr commands, useful metrics include:

- Service boot time;
- schema migration time;
- respawn time;
- shutdown time;
- cold sync wall time;
- incremental sync wall time;
- messages per second;
- provider request count;
- peak RSS;
- disk read/write bytes;
- final DB and store sizes;
- hang rate over repeated subprocess tests.

Future Brokkr gates could fail a run when:

- a previously stable Service test hangs;
- boot or respawn time regresses past a threshold;
- sync wall time regresses by more than a configured percentage;
- peak RSS regresses past a configured percentage;
- provider request count increases unexpectedly;
- final correctness assertions fail.

## Suggested Phase 8 planning order

Do not begin with mock servers. Track 1 (the harness module) lands
first because it has a wedge: re-enabling the two ignored subprocess
tests proves the architecture works.

Suggested order:

1. Decide whether `ServiceClient` and friends move out of `app` into a
   slim sub-crate before the harness module lands. Either layout works
   for the Lua bindings; a slim crate is cleaner for compile-time
   hygiene but is not on the critical path.
2. Add the `harness` module in `app`: `dellingr` dep, `RustFunc`
   wrappers for `ServiceClient`, the wait combinator, sentinel watch,
   frame-log tap, /proc snapshot writer, artefact-dir writer.
3. Wire `app --test-harness <script.lua>` behind `test-helpers`.
4. Add an entry in Ratatoskr's `brokkr.toml`:

   ```toml
   [[check]]
   name = "harness"
   features = ["test-helpers"]
   build_packages = ["app", "parent_death_helper"]

   [ratatoskr.harness]
   sweep = "harness"
   binary = "app"
   ```

5. Re-express the two ignored tests as Lua scripts. Run them under
   `brokkr service-test`. Confirm: when the Service deadlock fires,
   the artefact dump explains it (`proc-wchan.txt` for the writer
   task, `frames.jsonl` for whether the shutdown response was sent,
   `events.jsonl` for what got past `BootReady`).
6. Add the Phase 8 Ratatoskr-side hooks the cohort needs:
   `--test-fake-schema=N`, `TestSeedAccount`, fault-injection /
   counter-probe `RequestParams` variants. Each new variant is
   automatically usable from Lua.
7. Express the T1 cohort as Lua scripts.
8. Add manual-matrix items 4 and 5 (heartbeat / SIGTERM drain).
9. Add `brokkr service-suite` and soak loops once the single-script
   path is solid.
10. Only then design provider mock orchestration and sync benchmarks
    (Track 2).

## Non-goals

- Do not move Service correctness into Brokkr.
- Do not introduce a Brokkr -> Ratatoskr or Ratatoskr -> Brokkr
  source-level dependency. Brokkr orchestrates; Ratatoskr hosts the
  harness module. Cross-process communication is by subprocess spawn +
  env vars only.
- Do not require Brokkr to host the Lua VM or embed `ServiceClient`;
  either choice would force a tokio dependency in Brokkr and either a
  heavy `app`-crate dep or a parallel JSON-RPC client implementation.
- Do not require Brokkr to parse Ratatoskr SQLite tables.
- Do not require the iced UI for first-generation sync benchmarks.
- Do not make mock providers production code.
- Do not generalize the entire Service IPC protocol for external users.
- Do not block Phase 8 on comprehensive IMAP/JMAP mocks. The Service
  harness is useful on its own.
- Do not migrate the existing tokio-tests. The new harness coexists.
  Tests that work today as `#[tokio::test]` stay there. New tests in
  the cohort start in the new harness; old tests migrate only if their
  authors choose to.

## Open questions for the real Phase 8 plan

- Should `ServiceClient` and friends carve into a slim sub-crate
  (e.g. `crates/service-client`) before the harness module lands, or
  stay in `app`? Both work for the Lua bindings; the slim crate is a
  compile-time-hygiene call, not a correctness call.
- Should `crates/app/tests/service-harness/` be the discovery root, or
  should the location be configurable in `brokkr.toml` via an
  optional `[ratatoskr.harness] script_dir` override?
- Lua API surface naming and shape - the capabilities are pinned in
  the brokkr-side note's cohort table; function names, argument
  conventions, and return shapes are open.
- Should fixture setup live in `crates/dev-seed`, a new test-fixtures
  crate, or `service` test helpers?
- Headless sync surface: Brokkr's plan 3 has converged on a Lua
  script via Track 1's `app --test-harness` (see "Headless sync
  trigger" above). Phase 8 can ratify or revisit; the question is
  not fully closed until the first sync script lands.
- Should the harness module's marker emission go through the existing
  `BROKKR_MARKER_FIFO` sidecar protocol, or write its own log entries
  the harness reads back? The sidecar protocol exists and is stable;
  using it is the conservative call.
- Which app-data directories should be preserved by default vs cleaned
  on script success?
- Which subprocess tests must be serial because they involve process
  locks or fixed ports?
- Which provider mock should come first: IMAP, JMAP, or a shared
  fixture model before either protocol?
- What is the smallest JSON summary that sync benchmarks should emit?
- What is the cost-budget default for a Lua script before dellingr
  aborts it as runaway?

## Exit criteria for this scaffold

This scaffold has done its job when, after Phase 7, the Phase 8
planning session can answer these questions without additional
document spelunking:

- What does Brokkr provide beyond `cargo test`?
- Why is Brokkr relevant to Ratatoskr's Service phase?
- Which parts belong in Brokkr and which belong in Ratatoskr?
- What must Ratatoskr expose before Brokkr can help?
- Why does the harness module live in Ratatoskr's `app` crate rather
  than in Brokkr?
- Why should the Service harness land before provider mock servers?
