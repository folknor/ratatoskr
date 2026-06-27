# Service test harness

Lua-driven test runtime for Service IO-boundary tests. Lives in ratatoskr's `app` crate; orchestrated from outside by brokkr. Remaining harness work is in the root `TODO.md`.

The architecture is mirrored on the brokkr side as `notes/ratatoskr-service-harness.md` in the brokkr repository. Both documents stay in sync; this one is authoritative for the ratatoskr side.

The target cohort is Service tests that start the Service behind an **OS-level IO boundary** - a subprocess - and then wait on boot, dispatch, drain, crash, or framing behaviour. The wall-clock-timeout and child-exit-race failure modes that motivate the harness only apply when there's a real child process to race against.

The libtest cohort that originally lived in `crates/app/tests/service_subprocess.rs` is fully migrated to Lua scripts under `crates/app/tests/service-harness/`.

The `spawn_harness_with_suffix` family in `crates/service/tests/dispatch_in_process.rs` is **not** a migration target. It uses in-memory `tokio::io::duplex` pipes against `service::run_service_with_io` - no subprocess, no PID, no race against the OS scheduler. Wire-level parser tests (malformed JSON, oversized frames, invalid UTF-8) and protocol-level concerns (panicking handler, in-flight semaphore + heartbeat bypass) live there and are simpler to drive as in-process libtest. The boot-failure paths (`boot_sequence_returns_key_load_failure_when_key_file_is_missing`, `boot_sequence_returns_migration_failure_when_db_is_corrupt`) follow the same pattern.

## Motivation

Four failure modes break Service IO-boundary testing under a plain `#[tokio::test]` shape.

**Wall-clock timeouts inside the test race the implementation's own ceilings.** A 1 s `health.ping` timeout in libtest races the Service's own 5 s timeout, the spawn-side `request_or_observe_child_exit` 50 ms poll, the OS scheduler, and the disk during a hot migration. Running a flaky shape 200 times averages noise rather than eliminating it.

**Failure destroys diagnostic state.** `DataDirGuard::Drop` and `TestDataDir::Drop` clean up the test's app-data directory unconditionally, including on failure. SQLite WAL state, the lockfile, the key file, the `clean_shutdown` sentinel are all gone the moment the test fails. No `/proc` snapshot, no preserved frame log, no preserved Service stderr. Re-running with `dbg!` calls is the only path forward, and the bug may not reproduce.

**`kill_on_drop(false)` orphans the Service when the test itself is killed.** `PR_SET_PDEATHSIG` and Windows Job Objects handle real parent-death cleanly, but a libtest timeout or `Ctrl-C` is not real parent-death from the Service's perspective.

**The cohort facing this problem is large.** Phase 8 named ~15+ similarly-shaped real-subprocess tests planned across Phases 2-7. The `spawn_harness_with_suffix` cohort in `dispatch_in_process.rs` was in the same danger zone. Phase 8's planning notes put it directly: "building T1 against today's framework would mean rebuilding it once Phase 8's work lands."

The harness fixes these by making waits deterministic by construction and by preserving a self-contained artefact directory on failure.

## Brokkr context

Brokkr is a single-binary Rust dev tool, **external to ratatoskr** - its source lives in a separate repository and it is installed via `cargo install --path ~/Programs/brokkr`. It is not a ratatoskr crate, not a workspace member, and not a build dependency. From ratatoskr's side, brokkr is a binary on `$PATH` invoked from any project root. Brokkr reads `./brokkr.toml` to detect the active project and exposes project-gated commands tagged `[ratatoskr]`. The harness commands (`service-test`, `service-list`, `service-suite`, `sync-bench`) are an extension of that surface.

## Brokkr / ratatoskr split

Two repositories. Ratatoskr's `app` crate hosts the Lua VM and the `ServiceClient` userdata bindings (the runtime); brokkr orchestrates from outside (build, spawn, artefact dir, history). Concretely:

```
brokkr service-test foo.lua
    |
    +-- builds [ratatoskr.harness].package (defaults: binary=package, dev profile per debug=true)
    +-- allocates .brokkr/ratatoskr/<test>/run-N/ as artefact dir
    +-- spawns: <project_root>/<target>/<profile>/<binary> --test-harness foo.lua
    |       env: BROKKR_HARNESS_ARTEFACT_DIR=<artefact dir>
    |            BROKKR_TEST_BIN_DIR=<bin dir>
    +-- waits for child exit (sync, std::process::Command-level)
    +-- preserves artefact dir on failure / non-zero exit
```

Inside `app --test-harness`:

```
+-- dellingr Lua VM
+-- ServiceClient / SpawnEvent / ClientError userdata
+-- harness.* primitives (kill, pid_is_alive, http_*, marker, summary, ...)
+-- artefact writers (frames.jsonl, events.jsonl, steps.jsonl,
    proc-*.txt, data-dir/, service.stderr, runtime-outcome.json) into
    BROKKR_HARNESS_ARTEFACT_DIR
```

Brokkr's responsibilities at a glance: project gating + CLI surface, self-contained build of `[ratatoskr.harness].package` (no `[[check]]` cross-reference - bare `brokkr check` is blind to orchestration blocks), subprocess spawn + capture at `std::process::Command::output()` concurrency, per-run artefact directory lifecycle, script discovery (recursive over `crates/app/tests/service-harness/**/*.lua` with frontmatter parsing), soak (`-N`) and serial suite runners, history-DB recording, and an optional sidecar `/proc` profiler. None of this machinery is part of the harness contract - see Contract surface below.

## Dependency direction

**Both directions are off the table for source-level deps.** Ratatoskr must not depend on brokkr; brokkr must not depend on ratatoskr. Cross-process communication is by subprocess spawn + env vars only.

The Lua VM and `ServiceClient` userdata bindings live in ratatoskr's `app` crate (already where `ServiceClient` is defined); ratatoskr takes an unconditional `dellingr` dep and exposes the runtime via `app --test-harness`. Brokkr orchestrates only: project gating, self-contained orchestration build, lockfile, per-run artefact-dir lifecycle, history-DB recording, soak/suite. Brokkr ships zero ratatoskr or dellingr deps; brokkr stays sync (no tokio).

The harness needs `ServiceClient`'s typed classification (boot exit codes, `ClientError` variants, `SchemaVersionChanged { was, now }`, generation-tag tracking on notifications) - hundreds of lines of stateful protocol logic. Embedding it in brokkr would force tokio in and either a heavy `app`-crate dep or a parallel JSON-RPC client implementation. Hosting the VM in ratatoskr keeps the protocol logic where the protocol is.

`ServiceClient` stays in `app`. A slim `crates/service-client` carve-out is a compile-time-hygiene refactor, not a correctness requirement; revisit only if a second crate genuinely needs the client API or compile-time profiling shows pressure.

## Contract surface

**What brokkr passes the harness binary at spawn:**

- Argv: `app --test-harness <script.lua>` (script path; brokkr resolves it relative to the project root before passing).
- Env var `BROKKR_HARNESS_ARTEFACT_DIR` - absolute path to the per-run artefact directory (brokkr creates it before spawning).
- Env var `BROKKR_TEST_BIN_DIR` - absolute path to the directory containing the built `app` binary plus sibling helpers (`parent_death_helper`, future stub binaries).
- The harness binary's stdout/stderr are piped by brokkr; brokkr writes them to `binary-stdout.log` and `binary-stderr.log` in the artefact dir after the process exits. The runtime can `println!` for human-readable progress, but that is not a structured protocol.

**What brokkr expects in return:**

- Exit code zero on test pass; non-zero on failure. Brokkr preserves the artefact dir on non-zero (or signal) and deletes it on zero unless the user passed `--keep-artefacts`. There is no other out-of-band signaling - no JSON stdout protocol, no shared memory, no inotify on the artefact dir.
- Brokkr-owned artefacts (`run.toml`, copied script, `binary-stdout.log`, `binary-stderr.log`, `spawn-error.txt` on spawn failure) are written by brokkr. Runtime-owned artefacts (`frames.jsonl`, `events.jsonl`, `steps.jsonl`, `data-dir/`, `service.stderr`, `runtime-outcome.json`, Service `/proc` snapshots) are written by the harness module. Brokkr preserves but does not parse them in v1.

**What the harness module does NOT need to interact with:**

- Brokkr's history DB (`.brokkr/results.db`) - brokkr records the run outcome there automatically; the runtime is unaware of it.
- Brokkr's sidecar `/proc` profiler - reusable later if a script opts into it, but not part of the v1 contract.
- Brokkr's worktree machinery - worktrees aren't used for harness runs.
- Brokkr's lockfile coordination - the lock acquires before spawn and releases after wait; the runtime sees neither.
- Brokkr's source code - the contract is process-level, not source-level.

If you find yourself wanting more from brokkr than argv, env vars, stdout/stderr capture, brokkr-owned artefacts, and exit status, **stop and surface it as an explicit design change** rather than reaching across the contract. Both this document and `notes/ratatoskr-service-harness.md` in the brokkr repo would need to change in lockstep.

## Determinism rule

Every wait has the shape:

```
wait(condition) until (condition_satisfied
                     | child_terminated
                     | declared_backstop_expires)
```

The first transition wins. The harness records which one fired in the test trace. Tests assert on the transition that should have fired; failure messages name the transition that actually did. Wall-clock must never become the primary signal - child-exit observation is.

Backstops are still wall-clock - the harness can't escape physical time entirely - but they are explicit, named, generous, and only fire when a determinism bug elsewhere leaves the runtime with nothing else to wait on. A safety-backstop firing names a test-design or implementation-determinism bug, not a flake. Absence assertions (`expect_quiet`) use a separate observation-window verdict where the window expiring without the predicate firing is success.

## Test scripts and the Lua API

Tests are Lua scripts under `crates/app/tests/service-harness/` (and `crates/app/tests/sync-harness/` for provider-sync coverage). The VM is `dellingr` (pure Rust, no FFI, no system Lua dep). Adding a test means adding a `.lua` file; no brokkr rebuild, and no harness-module rebuild unless the new test exercises a Lua API surface that does not yet exist.

Why dellingr: pure Rust, no system Lua dep; `HostCallbacks` redirects `print()` to per-test capture and hooks errors for the failure dump; `RustFunc` exposes `ServiceClient` methods directly as userdata; variable capture, loops, and conditionals come from the language. Dellingr's per-opcode cost accounting is **not** used for runaway-script abort - `while true do end` is free by design, so wall-clock is the right mechanism. Scripts may set frontmatter `-- ceiling: 60s`; omitted scripts use a sane default.

Coverage claims live in the same initial frontmatter comment block. Use one line per contract ID:

```
-- @covers: architecture.folder_vs_label_semantics_are_explicit
```

The current coverage parser accepts repeated `-- @covers: id` lines, validates the ID grammar, and reports missing or unknown claims. It is read-only for now: missing claims are not harness-loader errors until a pilot area has been backfilled and strict mode is enabled for that area.

The `harness` global exposes:

- spawn helpers (`spawn`, `spawn_with_events`, `data_dir`),
- process primitives (`kill`, `pid_is_alive`, `sleep`, `now_ms`),
- assertions (`assert`, `assert_eq`, `same_client`, `expect_quiet`),
- filesystem helpers (`path_exists`, `dir_has_prefix`, `read_json`, `read_text`, `read_base64`, `write_text`),
- HTTP helpers (`http`, `http_get`, `http_post_json`, `http_delete`, `http_json`, `join_url`),
- env access (`env`, `protocol_version`),
- benchmarking helpers (`marker`, `write_summary`, `mock_requests`, `clear_mock_requests`, `request_count`, `request_count_prefix`, `snapshot_state`, `latency`, `set_latency`),
- large-payload helpers (`stage_attachment`, `repeat_byte`).

Client tables expose `request`, `request_async`, `shutdown`, `child_pid`, `current_generation`, `drop`, `notify`, `start_sync`, `start_calendar_sync`, `execute_calendar_plan`. Event streams expose `events:next(timeout_seconds)`. Async request handles expose `request:await(timeout_seconds)`.

The `request` binding is registry-backed: Rust owns a request/response registry that maps Lua method names and argument tables onto `RequestParams` variants, decodes the typed response, and returns a plain Lua table. Bad method names, malformed argument tables, and mismatched response shapes fail in Rust with a structured harness error. New `RequestParams` variants become script-visible by adding entries to the registry. The harness binding never reimplements `ServiceClient` behaviour - it forwards.

`SpawnEvent` is Lua userdata with three case constructors - `ChildSpawned { client }`, `BootReady { response }`, `Terminal { error }`. `ClientError` is Lua userdata with case-discriminating accessors so scripts can pattern-match without parsing strings.

Notification payloads are the exception to typed request/response decoding: they expose a `serde_json::Value`-backed Lua view for `params` so scripts can filter on `notif.method == "X"`.

Capability backlog (generic `wait_for`, full `NotificationQueue` userdata, sentinel-file watch, parent-death helper bindings, generic `wait_exit`, resource-budget summaries, parsed `frames.jsonl` payloads) is in TODO.md and lands when a future test names coverage those capabilities unblock.

## Stable Service entrypoints (ratatoskr-side contract)

`app --service --app-data-dir <dir>` is the canonical Service subprocess entrypoint. `app --test-harness <script.lua>` is its sibling.

Test-only flags are intentional and unconditionally compiled in (ratatoskr is pre-release; there is no production binary surface to guard):

- fake protocol version (`--test-fake-version=N`),
- fake schema (`--test-fake-schema=N`),
- slow request (`TestSlow { millis }`),
- hang-on-stdin-EOF (`--test-hang-on-stdin-eof`),
- println/framing canary (`TestPrintln { message }`),
- boot-delay (`--test-boot-delay-ms=N`).

Provider mock endpoints are env vars consulted on every provider boot; unset in normal use, set by the harness to redirect traffic to `saehrimnir`-hosted mocks:

```
RATATOSKR_TEST_JMAP_ENDPOINT
RATATOSKR_TEST_IMAP_ENDPOINT
RATATOSKR_TEST_SMTP_ENDPOINT
RATATOSKR_TEST_GRAPH_ENDPOINT
RATATOSKR_TEST_GMAIL_ENDPOINT
RATATOSKR_TEST_CALDAV_ENDPOINT
RATATOSKR_TEST_PEOPLE_ENDPOINT
RATATOSKR_TEST_GCAL_ENDPOINT
```

JMAP endpoints are passed as base URLs and the JMAP client discovers `/.well-known/jmap`; Graph origins map to `/v1.0` and `/beta`; Gmail origins map to `/gmail/v1/users/me`; IMAP and SMTP expect host:port. This lets brokkr pass per-run mock ports without changing persisted account config.

The sync-harness request surface includes `test.start_sync` / `TestStartSync` (kicks the real Service sync runtime - initial when `accounts.initial_sync_completed = 0`, then delta), `test.query_db_state` / `TestQueryDbState` (returns account, label, thread, message, attachment, calendar, contact, contact-group, credential, and bounded small-row snapshots for assertions), and the calendar-action counterparts. Lua scripts that need the terminal sync result call `client:start_sync({ account_id = ... })`, which routes through `ServiceClient::start_sync` and consumes raw `sync.completed` frames in the waiter map (they are not delivered to `client:notifications()`).

A second sync-harness request family - `test.bifrost_attach` / `test.bifrost_inject_batch` / `test.bifrost_arm_hook` / `test.bifrost_probe` (the `TestBifrost*` requests) - drives the additive bifrost change-stream consumer (`crates/service/src/bifrost/consumer/`) directly, with NO real provider fetch: `bifrost_attach` stands up the engine + consumer over a fixture account and returns a `session_id`, `bifrost_inject_batch` feeds one SYNTHETIC `SyncEvent::Batch` through the real hydrate -> write -> post-persist -> search-flush -> ack pipeline, `bifrost_arm_hook` arms a one-shot stall/crash for the durability gates, and `bifrost_probe` reads back the durable cursor / seen-counter / searchable-doc / marker state. The `bifrost-consumer-*` scripts (`lag-recovery`, `crash-before-ack`, `search-durability`, `checkpoint-resume`, `completion-cadence`, `hot-path`) under `crates/app/tests/sync-harness/` use this family. The `TestBifrost*` family itself drives no production fetch (it injects synthetic batches), but the consumer it exercises is now a PRODUCTION path for JMAP, Graph, Gmail, and IMAP production sync. The production JMAP path is gated by its own `jmap-*` sync-harness scripts (which drive real provider sync against `saehrimnir`), `jmap-production-lag-backoff.lua`, and `jmap-contacts-initial.lua`; the production Graph path by `graph-initial.lua`, the `graph-steady-state-delta.lua` sync-bench (the `graph_steady_state_delta` gate), the `graph_consumer_membership_equals_legacy` golden + `hydrate_change_graph_category_and_importance_mapping` / `graph_drive_reconciles_move`-`purge` service tests, and the existing `graph-attachment-*` / `graph-master-category-label-sync` / `graph-contacts-*` scripts; the production Gmail path by `gmail-initial.lua`, the `gmail-steady-state-delta.lua` sync-bench, `gmail-incremental-steps.lua`, `gmail-production-lag-backoff.lua`, the `gmail_consumer_membership_equals_legacy` golden + the Gmail hydrate-mapping service test, and the existing `gmail-attachment-*` / `gmail-oauth-multi-account` / `gmail-send-as-multi-account-import` scripts. For the consumer's durability contract read the bifrost-migration B3a-infra landing commit, and for the per-provider production wiring read the B3a-cut-jmap, B3a-cut-graph, B3a-cut-gmail, and B3a-cut-imap landing commits.

The bifrost B3b push landing then inverted this lifecycle to keep-attached (the engine stays `attach`ed across kicks with one resident `account_changes_stream` consumer per slot) and lit up push across all four providers, adding new sync-harness gates: the per-transport push scripts `jmap-push-websocket.lua` / `imap-push-idle.lua` (in-process forwarder + reconciler) and `gmail-push-pubsub.lua` / `graph-push-webhook.lua` (the Service-owned out-of-process `push_ingress/` module) each deliver a mock push and assert the resident consumer persists the change at push latency rather than waiting a poll interval, and `bifrost-consumer-sustained-push-bound.lua` pins the Graph move-vs-purge / IMAP threading / deferred-ack accumulator and ack-deferral bound under continuous push with no quiescence window. These required new `saehrimnir` push affordances - JMAP WebSocket `StateChange` push frames, a Gmail Pub/Sub source plus the `users.watch` / `users.stop` endpoints, and Graph `POST /subscriptions` (echo `validationToken`) + a notification POST to the registered loopback `notification_url` - and a harness-mode ingress validation bypass keyed to the mock's signing material, so no real Google/Microsoft endpoint is needed. For the keep-attached lifecycle and push wiring read the B3b landing commit.

The bifrost B3c control/recovery landing then wired the resident slot's pause/recovery surface, adding new sync-harness gates plus a test hook. `jmap-terminated-mid-sync-fails.lua` seeds a JMAP account that is valid at attach, then drives a mid-sync `SyncEvent::Terminated`, asserting the in-flight `start_sync` resolves `result == "failed"` (not a hang) and a follow-up kick fails fast while the slot is parked. `jmap-pause-resume.lua` drives a REAL engine `RetryBudgetExhausted` pause (saehrimnir budget exhaustion via the `jmap-changes-budget-exhaustion.lua` fixture, forcing every account reopen to fail so the three-attempt budget exhausts) so an `AccountControl::Pause` actually transits the engine's `account_control_stream` and the new `resident_control_loop` subscription - it asserts a `sync.account_paused` (`Notification::AccountPaused`) frame is observed and the account is parked, then resumes (re-auth or the new `sync.resume_account` request) and asserts a subsequent kick reaches `completed`. The `TestBifrost*` family gained a `ForceTerminated { recovery }` hook (`TestBifrostHook::ForceTerminated`, mirroring `ForceLag`) that injects a mapped terminal failure on the next event without provider/mock timing; it may SUPPLEMENT but never solely drive a pause gate, because a consumer hook bypasses the real `account_control_tx` broadcast and the subscription. `bifrost_probe` gained `resident_redrive_total` / `resident_redrive_attempt` fields read off the live `ResidentEngine` slot, and `bifrost-consumer-lag-recovery.lua` + `jmap-production-lag-backoff.lua` are extended to assert the bounded exponential backoff re-drives without hot-looping and that a clean caught-up edge resets the attempt counter. The new `sync.resume_account` request (`SyncResumeAccount`) clears a paused-account latch and resumes the attached boundary. For the control / pause / recovery wiring read the B3c landing commit.

The B4a action-pipeline landing then rewired the email-action mutation dispatch off the per-provider `ProviderOps` surface and onto the resident `SyncEngine`, so the action-write gates now exercise the engine path rather than a per-batch provider client. Three new per-provider round-trip gates - `jmap-action-writeback.lua`, `graph-action-writeback.lua`, and `gmail-action-writeback.lua` - drive the real action pipeline (`ActionExecutePlan` -> resident-engine mutation passthrough) against `saehrimnir` and verify each action by SERVER ROUND-TRIP, not by matching the provider's internal wire-op string: each asserts the `action.completed` summary shows the op dispatched REMOTELY (`remote_succeeded >= 1`, `remote_failed == 0`, `local_only == 0` - so a silent local-only degrade that never reached the provider is caught), then resyncs the account and asserts the server-side state reflects the mutation. The existing IMAP write-back scripts are held green across the cut: `imap-writeback-flags.lua` still pins the flag write-back (now via the engine), and `imap-writeback-move-delete.lua` was rewired to verify move/delete by the same server round-trip - dropping its old `UID COPY` / `UID STORE` / `EXPUNGE` request-count needles, because bifrost performs moves via the atomic RFC 6851 UID MOVE and a wire-op needle would be both brittle and wrong. The IMAP write-back path depends on a schema prerequisite this cut also landed: an `imap_uidvalidity` column on the `messages` table, populated at consumer hydrate time (the consumer now keeps the `uidvalidity` it previously dropped out of the decoded IMAP object id), so the action service can reconstruct a valid IMAP `ObjectId` (`imap1:<len>:<folder>:<uidvalidity>:<uid>`) for write-back; Gmail/JMAP/Graph object ids round-trip from the stored provider message id and need no schema change. For the action-dispatch rewire read the B4a landing commit.

## Deterministic app-data fixtures

Brokkr does not create app-data directories itself - Lua scripts do. `harness.data_dir(suffix, with_key)` creates a per-run app-data directory and, by default, a deterministic non-zero `ratatoskr.key`. The missing-key wedge passes `false`.

`TestSeedAccount` (and friends) creates accounts, labels, and FK-constrained adjacent rows. This must not rely on the dev app's "wipe and seed on every launch" behavior - some tests need to shut down and respawn against the same data directory.

## Trace schema

Trace files are JSONL with versioned records. Each top-level record carries `schema = 1`.

`frames.jsonl`:

```
{ "schema": 1, "ts_ms": 123, "direction": "in" | "out",
  "raw_redacted": "...", "raw_len": 456, "raw_sha256": "...",
  "parsed": { ... } | null }
```

The writer emits `raw_redacted`, never `raw`. Structural redaction is the default posture for strings above a chosen threshold (`<redacted len=N>`). `parsed` is currently always `null`; structural parsed redaction (per-`RequestParams` field allowlist) is future hardening before any credentialed script lands.

`events.jsonl`:

```
{ "schema": 1, "ts_ms": 123, "event": { ... } }
```

`event` is the typed `SpawnEvent` serialization.

`steps.jsonl`:

```
{ "schema": 1, "ts_ms": 123, "step": "...",
  "kind": "spawn" | "request" | "expect_quiet" | "...",
  "transition": "started" | "ok" | "error" | "quiet" | "event" | "..." }
```

When failure-triage tooling starts reading these files, factor the schemas into serde structs and keep readers tolerant of unknown fields for forward compatibility.

## Failure model

When a test fails, brokkr preserves a self-contained artefact dir at `.brokkr/ratatoskr/<test-name>/<run-N>/`. Brokkr writes the run metadata (`run.toml`, copied script, `binary-stdout.log`, `binary-stderr.log`, `spawn-error.txt`); the runtime writes Service diagnostics:

- `frames.jsonl` - every JSON-RPC frame, both directions, timestamped from spawn. Single most useful artefact for drain-ordering / framing bugs.
- `events.jsonl` - every spawn/runtime event observed (`ChildSpawned`, `BootReady`, `Terminal`).
- `steps.jsonl` - the test's step trace.
- `service.stderr` - Service stderr verbatim, per-run, not race-mingled with test stdout.
- `proc-{status,wchan,syscall,stack}.txt` - Linux best-effort `/proc` snapshot at failure-declaration time. Distinguishes "blocked on futex" from "blocked on closed pipe" without re-running.
- `data-dir/` - copy of the test's app-data dir at failure time (SQLite WAL state, lockfile, key file, `clean_shutdown` sentinel).
- `runtime-outcome.json` - runtime-side exit reason (clean, harness-killed-on-backstop, child-exited, signal, etc.).

On success, the artefact directory is deleted unless `--keep-artefacts` is passed.

The data dir copy, protocol/step trace, and `/proc` snapshot for real subprocesses are the pieces of state that today's tokio-test pattern destroys or never records (`DataDirGuard::Drop` / `TestDataDir::Drop` unconditional cleanup; no structured frame/step capture). Recovering them is the largest single jump in debug ergonomics: the difference between "the test hung, re-run with verbose logging" and "the writer task exited at frame 47 while the shutdown handler was awaiting an `ack` that the dispatch loop's in-flight counter shows is still parked at 1."

## Brokkr CLI surface

```
brokkr service-test <SCRIPT>
brokkr service-test <SCRIPT> -N 200       # single-script soak
brokkr service-test <DIR> -N 50           # cohort cycles
brokkr service-suite [--filter X] [-N 50]
brokkr service-list
brokkr sync-bench <SCRIPT> [--gate <name>] [--as-baseline]
```

Brokkr does not embed the Lua VM or `ServiceClient`; it never speaks JSON-RPC over the wire.

## Out of scope

- **Replacing ratatoskr's correctness assertions.** The Lua scripts drive `ServiceClient` methods and assert on returned values; they don't bypass ratatoskr's invariants.
- **Speaking JSON-RPC from brokkr.** `ServiceClient` lives in ratatoskr; brokkr spawns the binary that embeds it and never parses the wire.
- **Replacing `brokkr test`.** The cargo single-test runner stays. Service IO-boundary tests use the new harness; everything else uses `brokkr test`.
- **Fixing the underlying Service bugs.** The harness exists to make bugs deterministic and diagnosable, not to hide them. The Service side owns the drain-ordering / class-aware emit / crashloop fixes.
- **Migrating every existing tokio test.** The new harness coexists with libtest. Unit-style `#[tokio::test]` functions stay there. The migration target is the Service IO-boundary cohort.
- **Owning the mock servers.** Sync-bench reuses the artefact / lockfile / history machinery and the harness binary, but protocol mocks live in `../sæhrimnir` and orchestration lives in brokkr. Ratatoskr owns only the test-only endpoint overrides, sync-trigger requests, and sync-harness scripts.
- **CI-only features.** First user is a local developer root-causing a flake. CI integration follows once the local story works.
