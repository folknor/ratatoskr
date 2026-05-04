# Brokkr + Ratatoskr Phase 8 scaffolding

Status: **pre-planning problem statement.** This is not the Phase 8
technical implementation plan. It exists so that, after Phase 7 lands,
Ratatoskr developers can turn the Brokkr collaboration into a concrete
Phase 8 plan without first reading Brokkr internals or the full set of
Service planning docs.

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

Cargo/libtest remains the place for assertions. Brokkr should become the
thing that repeatedly runs those assertions under controlled process,
fixture, timeout, logging, and measurement conditions.

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

Phase 8 then becomes the right moment to switch gears and make Brokkr
part of the Service quality story.

## The two Brokkr tracks

There are two related features. They share harness infrastructure, but
they should be planned separately.

### Track 1: Service subprocess harness

Goal: make Ratatoskr's Service lifecycle behavior repeatable,
diagnosable, and measurable.

The immediate target is the current real-subprocess test cohort,
especially the ignored tests in `crates/app/tests/service_subprocess.rs`:

- `service_subprocess_ping_and_shutdown`
- `spawn_with_events_emits_terminal_on_missing_key`

Phase 8 already owns root-causing and re-enabling these. Brokkr should
help by running them many times, killing hangs, collecting Service logs,
and preserving enough artifacts that failures are inspectable.

Candidate future Brokkr commands:

```text
brokkr ratatoskr service-smoke
brokkr ratatoskr service-soak service_subprocess_ping_and_shutdown -N 200
brokkr ratatoskr service-soak spawn_with_events_emits_terminal_on_missing_key -N 200
brokkr ratatoskr service-test --all-subprocess
```

The exact CLI shape belongs in the Brokkr implementation plan. The
Ratatoskr-side requirement is that the tests and hooks are stable enough
for Brokkr to drive them.

### Track 2: Deterministic provider mock servers

Goal: make provider sync correctness and performance testable against
local deterministic IMAP/JMAP fixtures.

This track is related to the Service harness because sync runs inside
the Service and uses the same app-data, process, logging, timeout, and
artifact machinery. It is still a separate problem: the hard part is the
fixture and protocol model, not repeating a cargo test.

Candidate future Brokkr commands:

```text
brokkr ratatoskr mock-serve --imap --jmap --fixture small
brokkr ratatoskr sync-smoke --fixture jmap-small
brokkr ratatoskr sync-bench --fixture imap-100k
brokkr ratatoskr sync-bench --fixture jmap-incremental --bench 10
```

Brokkr should orchestrate these servers. It should not itself become the
IMAP or JMAP implementation.

## What Brokkr would provide

Ratatoskr developers should think of Brokkr as the outer runner.

For Service tests, Brokkr can provide:

- build selection for the correct `app` binary;
- repeated test execution with `--include-ignored`;
- hard per-run and global timeouts;
- process-tree cleanup on timeout;
- failure artifact preservation;
- pass/fail/hang summaries;
- timing and resource measurements;
- optional sidecar `/proc` profiling;
- eventual comparison against a stored baseline.

For provider sync workloads, Brokkr can provide:

- isolated app-data directories;
- local port allocation;
- mock-server process startup and teardown;
- fixture selection;
- passing endpoint credentials into Ratatoskr;
- sync workload execution;
- result storage;
- wall-time, RSS, I/O, request-count, and DB-size tracking;
- threshold-based regression failures.

Brokkr should not provide:

- Ratatoskr correctness assertions;
- direct knowledge of Ratatoskr's DB schema;
- Service IPC semantics beyond stable public test/workload entrypoints;
- the provider mock implementation itself.

## What Ratatoskr must provide

The Ratatoskr side of this collaboration is to expose stable, narrow
surfaces that Brokkr can run.

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

More Phase 8 hooks may be needed, but they should remain explicitly
test-scoped, ideally behind the existing `test-helpers` feature surface.

### Deterministic app-data fixtures

Brokkr needs to create or ask Ratatoskr to create valid app-data
directories for subprocess tests and sync workloads.

Ratatoskr should provide a fixture setup path that can create:

- `ratatoskr.key`;
- migrated main SQLite schema;
- required accounts and labels;
- rows needed by FK-constrained action tests;
- empty or seeded body/inline/blob/search stores as needed.

This should not rely on the dev app's "wipe and seed on every launch"
behavior. Some tests need to shut down and respawn against the same data
directory.

`crates/dev-seed/` may be a useful source of deterministic data
generation patterns, but test fixtures should be a deliberate API, not
an accidental dependency on dev startup behavior.

### Machine-readable lifecycle markers

Brokkr can measure much more accurately if Ratatoskr emits stable phase
markers or counters.

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

These could be emitted through Brokkr's marker FIFO when present, while
continuing to emit normal IPC notifications for the UI. The exact
emission surface should be planned later. The important requirement is
that timings do not require Brokkr to scrape human log lines.

### Artifact-friendly logging

Service logs already belong under `<app_data>/logs/`. For Brokkr, those
logs should reliably include:

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
best surface is a future planning decision. Plausible options:

- a Service IPC method used only by tests/benchmarks;
- a small headless benchmark binary;
- a cargo integration test that Brokkr drives with fixture environment
  variables.

The requirement is that Brokkr can start local mock servers, point
Ratatoskr accounts at them, trigger sync, wait for completion, and get a
machine-readable summary.

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

Ratatoskr or a companion crate should own the mock server and fixture
model. Brokkr should start it, choose fixtures, set ports, and collect
results.

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

When Phase 8 planning starts, do not begin with mock servers. Start with
the Service harness because it is smaller and directly addresses current
test debt.

Suggested order:

1. Stabilize and re-enable the two ignored subprocess tests.
2. Add or clean up the Ratatoskr test hooks Brokkr will need.
3. Define the app-data fixture setup API.
4. Define the artifact/log retention contract.
5. Add lifecycle markers or JSON summaries where timing needs them.
6. Let Brokkr implement `ratatoskr service-soak` against the existing
   subprocess tests.
7. Use Brokkr to repeat and prove the tests stable.
8. Add the remaining Phase 8 subprocess/action-service integration
   tests on top of the same harness.
9. Only then design provider mock orchestration and sync benchmarks.

## Non-goals

- Do not move Service correctness into Brokkr.
- Do not require Brokkr to parse Ratatoskr SQLite tables.
- Do not require the iced UI for first-generation sync benchmarks.
- Do not make mock providers production code.
- Do not generalize the entire Service IPC protocol for external users.
- Do not block Phase 8 on comprehensive IMAP/JMAP mocks. The Service
  harness is useful on its own.

## Open questions for the real Phase 8 plan

- Should fixture setup live in `crates/dev-seed`, a new test-fixtures
  crate, or `service` test helpers?
- Should headless sync be an IPC method, a binary, or a cargo test?
- Should Brokkr marker FIFO emission live in Service code directly, or
  behind a small trait like `ProgressReporter`?
- Which app-data directories should be preserved by default?
- Which subprocess tests must be serial because they involve process
  locks or fixed ports?
- Which provider mock should come first: IMAP, JMAP, or a shared fixture
  model before either protocol?
- What is the smallest JSON summary that sync benchmarks should emit?

## Exit criteria for this scaffold

This scaffold has done its job when, after Phase 7, the Phase 8 planning
session can answer these questions without additional document
spelunking:

- What does Brokkr provide beyond `cargo test`?
- Why is Brokkr relevant to Ratatoskr's Service phase?
- Which parts belong in Brokkr and which belong in Ratatoskr?
- What must Ratatoskr expose before Brokkr can help?
- Why should the Service harness land before provider mock servers?
