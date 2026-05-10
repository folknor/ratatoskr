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
  `harness.pid_is_alive`, `harness.sleep`, `harness.now_ms`,
  `harness.path_exists`, `harness.dir_has_prefix`,
  `harness.read_json`, `harness.read_text`, `harness.read_base64`,
  `harness.write_text`, `harness.assert`,
  `harness.assert_eq`, `harness.same_client`,
  `harness.expect_quiet(events, seconds)`, `harness.http_get(url)`,
  `harness.http_post_json(url, body)`,
  `harness.http_json({ method, url, body })`,
  `harness.http_delete(url)`, `harness.env(name)`, and
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

  [ratatoskr]
  mock_server_binary = "/home/folk/.cargo/bin/saehrimnir"
  fixtures_dir = "crates/app/tests/sync-fixtures"
  test_endpoint_env_jmap = "RATATOSKR_TEST_JMAP_ENDPOINT"
  test_endpoint_env_imap = "RATATOSKR_TEST_IMAP_ENDPOINT"
  test_endpoint_env_smtp = "RATATOSKR_TEST_SMTP_ENDPOINT"
  test_endpoint_env_graph = "RATATOSKR_TEST_GRAPH_ENDPOINT"
  test_endpoint_env_gmail = "RATATOSKR_TEST_GMAIL_ENDPOINT"
  test_endpoint_env_caldav = "RATATOSKR_TEST_CALDAV_ENDPOINT"
  test_endpoint_env_people = "RATATOSKR_TEST_PEOPLE_ENDPOINT"
  test_endpoint_env_gcal = "RATATOSKR_TEST_GCAL_ENDPOINT"
  sync_script_dir = "crates/app/tests/sync-harness"
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

**Status:** LANDED for the boot/dispatch lifecycle migration.

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
2026-05-08. A 200-cycle directory soak passed 1000/1000 runs on
2026-05-09:
`brokkr service-test crates/app/tests/service-harness/m2_5 -N 200`.

The forced-hang artefact drill was revalidated on 2026-05-09 with a
temporary `boot.ready` hang script using `--test-boot-delay-ms=60000`.
The preserved failure artefacts included `steps.jsonl`,
`frames.jsonl`, `data-dir/`, `service.stderr`, and child `/proc`
snapshots. `frames.jsonl` showed the outbound `boot.ready` request
with no response before failure, and `proc-wchan.txt` captured the
child blocked in `futex_do_wait`.

The old libtest bodies are ignored pointers to the authoritative Lua
scripts.

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

**Status:** LANDED for the T1 cohort.

Express the Phase 2 plan-specified integration cohort as `.lua`
scripts. The "T1" cohort:

- `journal_replays_after_respawn`
- `post_ack_crash_does_not_roll_back` /
  `post_ack_crash_replays_subprocess`
- `pre_ack_crash_rolls_back_subprocess`
- `mark_chat_read_emits_only_action_completed`
- `retry_queue_persists_across_respawn` - Phase 8-1 carry-forward
  verify for `pending_ops` persistence across Service respawn.
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
- `retry_queue_persists_across_respawn`
- `unbroken_crashes_trip_persistently_failing`
- `compose_send_50mb_attachment`
- `send_wire_attachment_validation`
- `send_wire_oversize_payload_handler_path`

This slice also adds the M4-specific helper surface:
`test.seed_thread`, `test.thread_read`, `test.pending_ops_read`,
`test.delay_next_write`, Lua bindings for `action.execute_plan`,
`action.job_status`, `action.mark_chat_read`, and `action.send`,
`client:notification_should_dispatch`, plus notification fields for
action plan IDs and service generations. Compose-send coverage uses
`harness.stage_attachment`, `harness.repeat_byte`, and mock SMTP
submission-log helpers (`harness.env`, `harness.http_get`,
`harness.http_delete`) to verify attachment staging, captured wire
metadata, and oversize-frame rejection. The crashloop script combines
the existing Service-side
`--test-boot-delay-ms` helper with `SIGKILL` to keep pre-`BootReady`
respawn failures deterministic.

The `harness-offline` account provider is a test-helper provider that
constructs successfully and returns transient network errors for
provider operations. This keeps planned actions and quiet remote
propagation on the same retry policy: unknown providers are permanent
setup errors, while `harness-offline` is a deliberate offline network
provider for `pending_ops` coverage.

The full T1 directory soak is now green with brokkr directory support.
The pre-compose slice passed 550/550 runs across 50 cycles on
2026-05-08. The full 14-script cohort, including the compose-send
scripts, passed 700/700 runs across 50 cycles on 2026-05-09 with
manual mock endpoint wiring. After brokkr gained fixture-aware
`service-test` orchestration for service-harness frontmatter, the
plain command also passed 700/700 on 2026-05-09:
`brokkr service-test crates/app/tests/service-harness/t1 -N 50`.

**Exit criteria:**

- Every test in the list passes individually.
- A 50-iteration soak across the whole T1 directory is clean.
- The Phase 2 plan-doc reference to "T1 deferred to Phase 8" is
  resolved.

---

### M5 - Phase 7 integration cohort

**Status:** LANDED for the planned automated cohort.

The Phase 7 plan called for `crates/service/tests/extract_in_process.rs`
to cover end-to-end fetch -> extract -> re-index -> search annotation,
status-aware idempotency, eviction-during-extract, cross-attachment
phrase non-match (position-gap working), body+attachment co-match,
backfill-kick semantics, and rebuild cancellation. Lands as `.lua`
scripts in `crates/app/tests/service-harness/extract/`.

Real-world fixture corpus lands here: `.pdf` / `.docx` / `.xlsx` /
`.pptx` files plus a malicious zip-bomb `.docx`, checked into the
repo at `crates/app/tests/service-harness/fixtures/extract/`.

Landed slices:

- `crates/app/tests/service-harness/extract/backfill_kick_indexes_cached_text_attachment.lua`
  seeds a cached `text/plain` attachment, sends the real
  `extract.backfill_kick` client notification, and asserts the Service
  writes `attachment_extracted_text`, marks `attachments.text_indexed_at`,
  and advances `extract.status`. The helper surface added for this
  slice is `client:notify(...)`, `TestSeedCachedAttachment`, and
  attachment extraction fields in `TestQueryDbState`.
- `crates/app/tests/service-harness/extract/backfill_kick_is_status_aware.lua`
  repeats `extract.backfill_kick` after a cached text attachment has
  resolved and asserts the second kick does not re-index the row or
  advance `extract.status` counters.
- `crates/app/tests/service-harness/extract/backfill_kick_marks_new_reference_to_resolved_hash.lua`
  seeds a second cached attachment with the same content hash after the
  first one has already resolved and asserts backfill marks the new
  reference indexed from existing extracted text without advancing
  extraction counters.
- `crates/app/tests/service-harness/extract/backfill_kick_skips_missing_cached_bytes.lua`
  removes a seeded cached blob while leaving the DB cache metadata in
  place, sends `extract.backfill_kick`, and asserts the worker records
  retryable `skipped:bytes_gone` without marking `text_indexed_at`.
- `crates/app/tests/service-harness/extract/cross_attachment_phrase_non_match.lua`
  seeds two cached text attachments on one message, verifies phrase
  searches inside each extracted attachment hit the search index, and
  asserts a phrase whose terms straddle the attachment boundary does
  not match.
- `crates/app/tests/service-harness/extract/body_attachment_co_match.lua`
  seeds a message body and a cached text attachment with the same term,
  verifies the extracted attachment reaches the search index, and
  asserts search attribution keeps the body as the primary match while
  reporting the attachment in `alsoMatched`.
- `crates/app/tests/service-harness/extract/attachment_only_search_annotation.lua`
  seeds a cached text attachment whose unique search term does not occur
  in the message body and asserts search attribution reports the
  attachment as the primary match.
- `crates/app/tests/service-harness/extract/index_rebuild_force_preempts_in_flight_wipe.lua`
  drives `index.rebuild` through the Lua harness, holds the search
  writer's `search.clear` command in flight with the test delay hook,
  asserts a duplicate non-forced rebuild is rejected, and verifies
  `force=true` preempts the first rebuild and completes a fresh one.
- `crates/app/tests/service-harness/extract/index_rebuild_wipe_roundtrip.lua`
  drives a normal non-forced Wipe rebuild, waits for
  `index.rebuild_completed`, then verifies a seeded message body is
  searchable through a fresh `TestSearchIndex` reader.
- `crates/app/tests/service-harness/extract/schema_mismatch_triggers_rebuild.lua`
  seeds a stale `search_index/.version`, boots the Service, waits for
  the post-ready schema rebuild to complete, asserts `.version` is
  updated, then reboots against the same data dir and verifies the
  matching version does not trigger another rebuild.
- `crates/app/tests/service-harness/extract/real_fixture_cache_miss_roundtrip.lua`
  seeds uncached attachment rows backed by provider bytes registered
  through the harness-only offline provider, drives real
  `attachment.fetch` cache misses, waits for extraction, and asserts
  search attribution for checked-in PDF, DOCX, XLSX, and PPTX fixtures.
  The same script verifies the hostile archive-shaped DOCX fixture is
  classified as `skipped:zipbomb` without poisoning the extraction
  queue.
- `crates/app/tests/service-harness/fixtures/extract/` now contains the
  small known-content fixture corpus used by the real cache-miss script:
  `known-content.pdf`, `known-content.docx`, `known-content.xlsx`,
  `known-content.pptx`, and `zipbomb-shaped.docx`.

**Exit criteria:**

- All seven test classes from the original plan list pass.
- The fixture corpus catches regression classes beyond the synthetic
  byte-literal fixtures: MIME-specific PDF / OOXML extraction, cache-
  miss provider fetch registration, and zip central-directory bomb
  rejection.
- The Phase 7 plan-doc reference to "integration tests deferred to
  Phase 8" is resolved.

**Remaining actionable work:**

- None. Future extraction formats can extend the same fixture corpus
  and script pattern.

---

### M6 - Manual-matrix automation

**Status:** PARTIAL - the former manual matrix has been folded into
this roadmap. Items 4 through 14 have landed for Linux/non-Windows
coverage; the remaining action items are listed below.

The old `docs/harness/manual-test-matrix.md` file has been deleted so
this roadmap is the single work queue. Every former manual item is now
represented here as landed automation, remaining work, or an explicit
retirement decision.

Sequencing:

- **M6.4 + M6.5 (LANDED):** heartbeat-detects-killed-Service and
  SIGTERM-triggers-shutdown-drain now live in
  `crates/app/tests/service-harness/m6/`. The heartbeat script kills
  the Service with `SIGKILL` and observes respawn through
  `ChildSpawned` + `BootReady`. The SIGTERM script verifies the
  unrequested drain contract: the Service exits and does not write the
  `clean_shutdown` sentinel. Verified on 2026-05-09 with focused
  `brokkr service-test` runs for both scripts.
- **M6.1, M6.2, M6.3 (READY when cross-platform CI exists):**
  Linux / Windows parent-death + clean shutdown handshake + stdio
  defense. Linux items already automate; Windows items need a real
  Windows host (cross-platform CI runner, dev box, or paid test
  service). The harness scripts are platform-agnostic; the gate is
  the test environment.
- **M6.6 (LANDED):** cold-boot bootstrap snapshots now lives in
  `crates/app/tests/service-harness/m6/`. The script persists
  settings via `settings.set`, shuts down cleanly, respawns against
  the same data dir, and asserts `internal.read_bootstrap_snapshots`
  returns the persisted UI/settings snapshot values. Verified on
  2026-05-09 with a focused `brokkr service-test` run.
- **M6.7 (LANDED):** draft WAL replay now lives in
  `crates/app/tests/service-harness/m6/`. The script seeds an
  account, writes `drafts.wal` with one valid draft entry and one
  partial trailing line, boots the Service against the same data dir,
  and asserts the row replayed plus the WAL rotated to
  `drafts.wal.replayed.*`. Verified on 2026-05-09 with a focused
  `brokkr service-test` run.
- **M6.8 (LANDED):** account.delete cancels in-flight sync now lives
  in `crates/app/tests/service-harness/m6/`. The script uses a
  test-helper `harness-slow-sync` provider that parks until its
  cancellation token fires, then asserts `account.delete` writes a
  cancelled sync marker and removes the account-scoped rows. Verified
  on 2026-05-09 with a focused `brokkr service-test` run.
- **M6.9 (LANDED):**
  OAuth re-auth is no longer blocked on the fake
  provider. `crates/app/tests/service-harness/m6/oauth_reauth_uses_mock_provider.lua`
  drives `oauth.exchange_code` against saehrimnir's mock OAuth
  provider, asserts the re-auth ack omits token bytes, and verifies the
  account row gets new encrypted access / refresh token hashes without
  changing identity or provider columns.
  `crates/app/tests/sync-harness/jmap-oauth-revoked-fails.lua`
  seeds an OAuth-enforced JMAP account with an invalid bearer token and
  asserts `client:start_sync` returns a failed sync result carrying the
  expected 401. `crates/app/tests/sync-harness/jmap-oauth-recovery.lua`
  seeds a JMAP OAuth account, re-authenticates through saehrimnir's
  token route, and verifies the refreshed tokens can import mail from
  the OAuth-enforced fixture.
- **M6.10 (LANDED):**
  `crates/app/tests/sync-harness/graph-calendar-initial.lua`
  runs the Graph calendar fixture through the real calendar runtime
  and asserts local calendar/event state plus Graph request-log
  coverage. `crates/app/tests/service-harness/m6/calendar_actions_graph_crud.lua`
  drives `cal_action.execute_plan` create/update/delete against the
  Graph fixture and asserts local state plus POST/PATCH/DELETE request
  bodies, then follows the action triplet with a calendar delta sync
  to prove the action-side fixture mutations replay through normal
  Graph `calendarView/delta`. `crates/app/tests/sync-harness/graph-calendar-remote-delta.lua`
  mutates the mock Graph fixture directly with POST/PATCH/DELETE, then
  verifies calendar delta sync imports the created, updated, and
  tombstoned events. CalDAV now has initial-sync, action CRUD,
  remote-mutation import, and shared-fixture mutation proof through
  Graph delta. JMAP now has
  `crates/app/tests/sync-harness/jmap-calendar-initial.lua` plus
  `crates/app/tests/service-harness/m6/calendar_actions_jmap_crud.lua`,
  covering `Calendar/get`, `CalendarEvent/get`, and
  `CalendarEvent/set` create/update/delete through the Service worker.
  Google Calendar now has
  `crates/app/tests/sync-harness/gcal-calendar-initial.lua` plus
  `crates/app/tests/service-harness/m6/calendar_actions_gcal_crud.lua`,
  covering calendarList/events sync and POST/PATCH/DELETE through
  the Google Calendar v3 listener. The Google action slice also
  normalizes provider-neutral action timestamps into Google
  `dateTime` objects and hardens Google tombstone parsing so
  cancelled delta events without start/end can delete local rows.
  `crates/app/tests/service-harness/m6/calendar_create_provider_failure_local_only.lua`
  covers the create-on-provider-failure lane: a JMAP calendar account
  imports calendars with a valid OAuth token, the script invalidates
  that token, and `cal_action.execute_plan` CreateEvent returns
  `LocalOnly` while keeping the locally-created row unsynced.
- **M6.12 (LANDED):** backfill kick on boot.ready now lives in
  `crates/app/tests/service-harness/m6/`. The script seeds a cached
  but unindexed text attachment, restarts the Service against the same
  data dir, and asserts the post-ready extract startup indexes it
  without an explicit harness `extract.backfill_kick`.
- **M6.13 (LANDED):** normal search-index Wipe rebuild now lives in
  `crates/app/tests/service-harness/extract/index_rebuild_wipe_roundtrip.lua`.
  It covers the Service-side rebuild request that backs the palette
  command and proves search is usable after completion.
- **M6.14 (LANDED):** schema-version mismatch rebuild now lives in
  `crates/app/tests/service-harness/extract/schema_mismatch_triggers_rebuild.lua`.
  The script exercises the post-ready PreserveExisting rebuild lane,
  uses `harness.read_text` to assert the `.version` sentinel is
  rewritten only after a successful rebuild, and verifies the next boot
  is quiet.
- **M6.11 (LANDED):** attachment extraction cache-miss round trip now
  lives in
  `crates/app/tests/service-harness/extract/real_fixture_cache_miss_roundtrip.lua`.
  The script seeds provider-backed uncached attachment rows, fetches
  them through the real `attachment.fetch` path, waits for extraction,
  asserts attachment-only search attribution for PDF, DOCX, XLSX, and
  PPTX fixtures, and verifies the hostile archive-shaped DOCX is
  skipped as `skipped:zipbomb`.

**Exit criteria:**

- Every former manual-matrix item has a corresponding `.lua` script,
  OR has been explicitly retired here with a note naming why
  automation is not worth the effort.
- This roadmap has no remaining M6 actionable items.
- This milestone marks the user-visible promise: "the user just runs
  `brokkr check` and everything is tested autonomously."

**Remaining actionable work:**

- M6.1-M6.3 Windows lifecycle validation:
  run the parent-death Job Object check, clean shutdown handshake, and
  stdio-corruption defense on a real Windows host. If the checks need
  to become permanent automation, add Windows-capable Lua or libtest
  coverage and keep the Linux-only SIGTERM script separate.
---

### M7 - Brokkr-side polish

**Status:** PARTIAL - `service-list`, single-script soak,
directory-cohort `service-test`, and `service-suite -N` landed;
`service-list --json` is deferred.

- `brokkr service-suite [--filter X]` - walks
  `crates/app/tests/service-harness/`, runs every script (or every
  script matching `--filter`), aggregates pass/fail stats. V1 suite
  execution is serial and does not expose `--jobs`.
- `brokkr service-test <DIR> -N <COUNT>` - directory form is sugar
  for the suite path scoped to that cohort. `-N` means cohort cycles,
  so `-N 50` over the 11-script T1 directory runs 550 invocations.
- `brokkr service-list --json` - machine-readable script discovery
  for failure-triage tooling and editor integrations.

**Exit criteria:**

- Suite runs across the M4 + M5 + M6 cohort cleanly.
- The JSON shape of `service-list --json` is documented stable.

---

### M8 - Provider mock servers (Track 2)

**Status:** LANDED for the non-Windows M8 scope - ratatoskr's
test-only endpoint override, sync-trigger, DB-query, first
sync-harness script, OAuth re-auth persistence harness path, JMAP
remote-mutation and scripted incremental delta scripts, Graph calendar
mutation scripts, Graph contact coverage, CalDAV calendar coverage,
JMAP/Google calendar initial/action coverage, People API
initial/incremental contact coverage, People API write-back/delete
coverage, and the deeper JMAP fixture scripts have landed.
Saehrimnir now exposes the remaining major mock surfaces for
JMAP Calendar, Google Calendar v3, Google People API, attachment
raw-bytes, and slow-paged fixture recipes. Ratatoskr now owns its
sync fixture corpus under `crates/app/tests/sync-fixtures/`; the
sibling saehrimnir repository owns the fixture DSL and loader.
Brokkr can now spawn saehrimnir for fixture-frontmatter scripts and
inject provider endpoints; its `sync-bench --gate` / `--as-baseline`
path now records per-host baselines in `gate.db` and evaluates
scalar, `sidecar.*`, and `meta.*` thresholds from `brokkr.toml`.
Recent saehrimnir support adds admin
request/reset routes,
OAuth token / userinfo routes, steady-state JMAP changes, JMAP
`Email/set` / `Mailbox/set` changesets, persistent IMAP `UID STORE`,
IMAP `UID COPY` / `UID EXPUNGE`, Graph calendar mutation deltas,
cursor-driven change scripts through `POST /test/fixture/step`,
fixture-image rewind on reset, stable request logs, fixture snapshots,
per-protocol latency injection, Graph contact fixture/read/delta
surfaces with contact change scripts, and a CalDAV listener covering
discovery, PROPFIND/REPORT/GET, PUT, and DELETE. The newest
saehrimnir surfaces also add JMAP Calendar
(`Calendar/get` / `Calendar/changes`, `CalendarEvent/get` /
`CalendarEvent/changes` / `CalendarEvent/set`), cross-protocol
`Email::raw_bytes` for JMAP `Email/get` and Gmail `threads.get`, a
slow-paging fixture recipe, a Google People API listener, and a Google
Calendar v3 listener. The sentinel now reports PEOPLE and GCAL ports;
ratatoskr now has matching `RATATOSKR_TEST_PEOPLE_ENDPOINT` and
`RATATOSKR_TEST_GCAL_ENDPOINT` overrides so live binaries can use those
listeners under `sync-bench` / `service-test`.

Mock provider servers and fixture sets (small smoke / medium / large /
huge thread / many folders / duplicate Message-ID / malformed MIME /
slow-paged responses / incremental new+change+delete+move sequence /
raw attachment bytes / Graph contacts / Google People contacts /
Graph+CalDAV+Google+JMAP calendar events), and the brokkr-side
commands to start/stop them and collect results.

The mock-server design lives in a sibling brokkr-side note
(`notes/ratatoskr-mock-server.md`); this milestone tracks the
ratatoskr-side integration. IMAP first is probably easier for sync
realism; JMAP is broader but feasible as a bounded subset.

Headless sync trigger has converged on the Lua-script-via-harness
path (per the brokkr-side note's Plan 3 resolution): tests use the
existing `app --test-harness` binary and `ServiceClient` userdata
plus new sync-triggering / state-querying `RequestParams` variants
(`TestStartSync`, `TestQueryDbState`).

Ratatoskr-side M8 surface now in tree:

- `RATATOSKR_TEST_{JMAP,IMAP,SMTP,GRAPH,GMAIL,CALDAV,PEOPLE,GCAL}_ENDPOINT`
  are read under the `test-helpers` feature and redirect provider
  clients to the mock endpoints supplied by brokkr.
  `RATATOSKR_TEST_PEOPLE_ENDPOINT` covers Gmail contact sync, Google
  otherContacts sync, GAL lookup, and Google contact write-back/delete
  URLs. `RATATOSKR_TEST_GCAL_ENDPOINT` covers Google Calendar v3 sync
  and action URLs. `brokkr.toml` now lists `test_endpoint_env_people`
  and `test_endpoint_env_gcal`, so brokkr injects all eight
  saehrimnir endpoint variables for fixture-frontmatter scripts.
- The Lua service harness now parses the contact IPC methods
  `contacts.contact_save`, `contacts.contact_save_with_writeback`,
  and `contacts.contact_delete`, so sync-harness scripts can drive the
  real Service contact write-back pipeline while saehrimnir is running.
- `test.start_sync` starts the real Service sync runtime. The
  Service sync dispatcher now runs provider initial sync when
  `accounts.initial_sync_completed = 0`, then delta sync afterwards.
- `client:start_sync` is the Lua path for scripts that need the
  terminal result; it routes through `ServiceClient::start_sync`
  because raw `sync.completed` notifications are consumed by the
  client's waiter map before `client:notifications()`.
- `test.query_db_state` returns account, label, thread, message,
  unread-message, attachment, calendar, contact, contact-group,
  credential-summary, and small row snapshots for sync-harness
  assertions.
- Ratatoskr-owned sync fixtures live in
  `crates/app/tests/sync-fixtures/`. Saehrimnir still owns the fixture
  DSL and loader; ratatoskr owns this regression corpus because the
  scripts and assertions are ratatoskr behavior tests.
- `crates/app/tests/sync-harness/jmap-initial.lua` is the first
  sync-harness script. It targets the `jmap-small.toml` fixture and
  asserts the two fixture messages land in the local DB. It now emits
  sync markers and writes `summary.json` with message/thread counts
  and JMAP request counters.
- `crates/app/tests/sync-harness/jmap-bulk-initial.lua` targets
  `jmap-bulk.lua` and proves a 10,001-message fixture imports through
  the normal JMAP initial-sync path. It emits sync markers and writes
  `summary.json` with bulk message/thread counts and JMAP request
  counters.
- `crates/app/tests/sync-harness/jmap-bulk-steady-state-delta.lua`
  runs the same 10,001-message fixture twice and asserts the measured
  steady-state pass uses `Mailbox/changes` and `Email/changes` without
  falling back to `Email/query`. It emits sync markers around the
  measured delta and writes `summary.json`.
- `crates/app/tests/sync-harness/jmap-attachment-initial.lua` targets
  `jmap-attach.toml` and asserts JMAP `Email/get` raw-byte projection
  imports attachment metadata for `sample.txt`. It emits sync markers
  and writes `summary.json` with attachment counts and request metrics.
- `crates/app/tests/sync-harness/jmap-many-folders-initial.lua`
  targets `jmap-many-folders.lua` and verifies JMAP mailbox import at
  many-folder scale. It emits sync markers and writes `summary.json`
  with message, label, and provider-request metrics.
- `crates/app/tests/sync-harness/jmap-duplicate-message-id-initial.lua`
  targets `jmap-duplicate-message-id.lua` and verifies that three
  separate messages sharing one `Message-ID` header survive initial
  import as three local rows. It emits sync markers and writes
  `summary.json` with message, thread, and provider-request metrics.
- `crates/app/tests/sync-harness/jmap-malformed-mime-initial.lua`
  targets `jmap-malformed-mime.lua` and verifies that malformed raw
  body bytes do not abort JMAP initial sync. It emits sync markers and
  writes `summary.json` with message and provider-request metrics.
- `crates/app/tests/sync-harness/jmap-slow-paged-initial.lua` targets
  `jmap-slow-paged.lua` and verifies that initial sync completes when
  a middle `Email/query` page is delayed by the fixture hook. It emits
  sync markers and writes `summary.json` with message, thread, and
  provider-request metrics.
- `crates/app/tests/sync-harness/jmap-steady-state-delta.lua`
  runs the same fixture twice, asserts the first run marks
  `initial_sync_completed`, and uses saehrimnir's request log to
  prove the second run goes through `Mailbox/changes` and
  `Email/changes` without falling back to `Email/query`. It now
  emits `SYNC_START` / `SYNC_END` around the measured delta sync and
  writes `summary.json` with scalar correctness, DB-count, and
  provider-request metrics for `brokkr sync-bench` ingestion.
- `crates/app/tests/sync-harness/jmap-email-set-delta.lua`
  mutates the mock fixture through a direct JMAP `Email/set`, then
  runs ratatoskr delta sync and asserts the updated read state is
  imported via `Email/changes` plus `Email/get` without falling back
  to `Email/query`. The mutation now uses
  `harness.http_json({ method, url, body })` with a Lua table body so
  later PATCH/DELETE provider-admin calls can reuse the same helper.
  It emits sync markers around the measured delta sync and writes
  `summary.json` with scalar correctness, message-count, and JMAP
  request metrics.
- `crates/app/tests/sync-harness/jmap-incremental-steps.lua`
  targets the `jmap-incremental.lua` fixture, walks saehrimnir's
  cursor-driven `POST /test/fixture/step` script through new, change,
  delete, and move steps, runs ratatoskr delta sync after each step,
  and asserts local DB convergence plus `Email/changes` / `Email/get`
  usage without falling back to `Email/query`. It emits sync markers
  around each measured delta sync and writes `summary.json` with final
  message counts and aggregate JMAP request counters.
- `crates/app/tests/sync-harness/jmap-latency-smoke.lua` configures
  saehrimnir global and JMAP latency knobs, proves a direct JMAP probe
  is delayed, then runs ratatoskr initial sync against the delayed
  mock endpoint and clears the knobs again.
- `crates/app/tests/sync-harness/imap-initial.lua` targets the
  `imap-small.toml` fixture, asserts the two fixture messages land in
  the local DB, verifies `$seen` / `$flagged` import into read /
  starred state, and uses saehrimnir's request log to prove the sync
  listed folders, searched UIDs, and fetched messages through IMAP.
  It now emits sync markers and writes `summary.json` with message,
  unread, thread, and IMAP request counters.
- `crates/app/tests/sync-harness/imap-steady-state-delta.lua` runs
  the same fixture twice, asserts state stays stable, and uses the
  request log to prove the second run lists/selects/searches without
  issuing body-fetching `UID FETCH` calls for unchanged messages. It
  emits `SYNC_START` / `SYNC_END` around the measured delta sync and
  writes `summary.json` with final DB counts and IMAP request counters.
- `crates/app/tests/sync-harness/imap-writeback-flags.lua` drives
  real `ActionExecutePlan` SetRead and SetStarred operations against
  an IMAP-synced thread, asserts saehrimnir records `UID STORE`, then
  runs follow-up syncs to prove the persisted fixture flags do not
  revert. This also hardens the action local-write path so SetRead and
  SetStarred update per-message flags as well as thread flags.
- `crates/app/tests/sync-harness/imap-writeback-move-delete.lua`
  drives a real `MoveToFolder` action into the fixture Archive mailbox
  and a real `PermanentDelete` action after a follow-up sync. It
  asserts saehrimnir records the expected `UID COPY`, `UID STORE`, and
  `EXPUNGE` / `UID EXPUNGE` traffic, then proves later syncs preserve
  the moved state and final deletion. This slice also hardens IMAP
  `MoveToFolder` to resolve canonical label IDs to provider mailbox
  paths, records UIDPLUS `COPYUID` mappings so moved rows track their
  destination mailbox UID, uses `UID EXPUNGE` when available before
  falling back to plain `EXPUNGE`, teaches IMAP sync to reuse the
  existing message row for a repeated `(folder, uid)` fetch instead of
  creating duplicate folder-derived IDs or placeholder threads, and
  makes permanent delete provider-first while preserving retry-queue
  insertion when provider dispatch fails.
- `crates/app/tests/sync-harness/imap-incremental-new-change.lua`
  targets the shared scripted `jmap-incremental.lua` fixture through
  IMAP, applies saehrimnir's new, flag-change, delete, and move steps,
  runs ratatoskr IMAP delta sync after each step, and asserts local DB
  convergence across message creation, flag import, tombstone import,
  and mailbox-label movement. It emits sync markers around each
  measured mutation-import sync and writes `summary.json` with final
  message counts and aggregate IMAP request counters. This also fixes
  IMAP flag-only fetches to
  request `UID` alongside `FLAGS`, treats a pinned `HIGHESTMODSEQ = 1`
  as an untrusted seed value that requires a UID search plus full flag
  diff while retaining the trusted CONDSTORE fast path for real
  advancing modseq values, and tightens deletion detection so folders
  whose server message count dropped can force a server-UID comparison
  instead of waiting for the normal ten-minute janitor throttle. The
  flag-change path now also mirrors aggregate IMAP read/starred updates
  back into `UNREAD` / `STARRED` `thread_labels`, so thread-detail and
  sidebar-style projections stay consistent with `messages`.
- `crates/app/tests/sync-harness/imap-jmap-shared-state.lua`
  seeds both an IMAP and a JMAP account against the same fixture, moves
  a message through the real IMAP `MoveToFolder` action, then runs JMAP
  delta sync and asserts the move arrives through `Email/changes` /
  `Email/get` without falling back to `Email/query`. This is the mail
  sibling of the CalDAV-write to Graph-delta shared-fixture proof. It
  emits sync markers around the measured JMAP convergence sync and
  writes `summary.json` with scalar correctness, message-count, and
  JMAP request metrics.
- `crates/app/tests/service-harness/m6/oauth_reauth_uses_mock_provider.lua`
  targets the `jmap-small.toml` fixture's mock OAuth routes and
  automates the M6.9 re-auth persistence check.
- `crates/app/tests/sync-harness/jmap-oauth-recovery.lua` targets the
  `jmap-oauth.toml` fixture, seeds a JMAP OAuth account, runs
  `oauth.exchange_code`, then verifies the refreshed encrypted tokens
  can drive a JMAP sync against the OAuth-enforced fixture.
- `crates/app/tests/sync-harness/jmap-oauth-revoked-fails.lua` targets
  the same OAuth-enforced fixture and locks in the failed-sync return
  shape for an invalid bearer token (`result.result == "failed"` and a
  401-bearing error string).
- `crates/app/tests/sync-harness/graph-calendar-initial.lua`
  targets the `graph-calendar-small.toml` fixture, drives
  `client:start_calendar_sync`, asserts the Work/Personal calendars
  and two Work events land in the local DB, and uses saehrimnir's
  request log to prove Graph calendar list and per-calendar delta
  endpoints were exercised. It emits sync markers and writes
  `summary.json` with calendar/event counts and Graph request counters.
- `crates/app/tests/service-harness/m6/calendar_actions_graph_crud.lua`
  targets the same Graph calendar fixture, drives
  `client:execute_calendar_plan`, and verifies create/update/delete
  flow through the Service worker into Graph POST/PATCH/DELETE
  requests and local calendar-event state. It then runs a follow-up
  calendar sync and asserts the normal Work-calendar delta endpoint
  sees the action-side mutation log.
- `crates/app/tests/sync-harness/graph-calendar-remote-delta.lua`
  targets the same Graph calendar fixture, mutates the remote mock
  directly through Graph POST/PATCH/DELETE calls, then drives
  `client:start_calendar_sync` and verifies the local calendar DB
  imports the created event, updated event fields, and deleted-event
  tombstone through normal `calendarView/delta`. This slice also
  hardens Graph delta parsing so tombstone objects with only `id` and
  `@removed` deserialize far enough for the deletion path to handle
  them. It emits sync markers and writes `summary.json` with final
  calendar/event counts and Graph delta request counters.
- `crates/app/tests/sync-harness/graph-contacts-initial.lua`
  targets `graph-contacts-small.toml`, drives the normal
  `client:start_sync` Graph initial-sync path, and asserts the synced
  contact rows include fixture contacts from multiple contact folders,
  skip no-email contacts, carry `source = "graph"` and Graph
  `server_id`, and bootstrap contact delta endpoints for follow-up
  syncs. It emits sync markers and writes `summary.json` with final
  contact counts and Graph contact request counters.
- `crates/app/tests/sync-harness/graph-contacts-incremental.lua`
  targets `graph-contacts-incremental.lua`, applies saehrimnir's
  scripted contact create/update/delete steps, runs Graph delta sync
  until the production twentieth-cycle contact cadence fires, and
  asserts contact rows converge after each step. This also fixed the
  Graph delta cycle counter to persist in sync state instead of living
  only on the per-run `GraphClient`, so the twentieth-cycle contact,
  label, group, and folder-tier work is reachable across separate
  `start_sync` requests. It emits sync markers around the measured
  delta-cadence loops and writes `summary.json` with final contact
  counts and aggregate Graph contact-delta request counters.
- `TestSeedAccount` now accepts `caldav_url`, `caldav_username`, and
  `caldav_password`, so scripts can seed a real CalDAV account without
  a UI account-create flow.
- `crates/app/tests/sync-harness/caldav-calendar-initial.lua`
  targets the shared `graph-calendar-small.toml` calendar/event
  fixture through saehrimnir's CalDAV listener, drives
  `client:start_calendar_sync`, and asserts the same Work/Personal
  calendars and Work events land through PROPFIND plus
  calendar-multiget REPORT. It emits sync markers and writes
  `summary.json` with calendar/event counts and CalDAV request counters.
- `crates/app/tests/service-harness/m6/calendar_actions_caldav_crud.lua`
  targets the same fixture through CalDAV, drives
  `client:execute_calendar_plan`, and verifies create/update/delete
  flow through the Service worker into CalDAV GET/PUT/DELETE requests
  and local calendar-event state. This slice also normalizes the
  provider-neutral calendar action payload into CalDAV's `startTime` /
  `endTime` iCal write shape, so numeric action timestamps produce
  dated VEVENTs.
- `crates/app/tests/sync-harness/caldav-calendar-remote-delta.lua`
  mutates the CalDAV fixture directly with raw PUT/DELETE calls, then
  drives `client:start_calendar_sync` and verifies the local calendar
  DB imports the created event, updated event fields, and deleted
  resource through the normal CalDAV ctag plus REPORT path. It emits
  sync markers and writes `summary.json` with final calendar/event
  counts and CalDAV request counters.
- `crates/app/tests/sync-harness/graph-calendar-caldav-mutation-delta.lua`
  writes create/update/delete calendar mutations through CalDAV and
  verifies a subsequent Graph `calendarView/delta` imports the same
  shared fixture changes. This proves saehrimnir's calendar mutation
  log is shared across the Graph and CalDAV protocol surfaces. It
  emits sync markers and writes `summary.json` with final
  calendar/event counts and Graph delta request counters.
- `crates/app/tests/sync-harness/jmap-calendar-initial.lua`
  targets the shared calendar fixture through saehrimnir's JMAP
  Calendar surface, drives `client:start_calendar_sync`, and asserts
  the Work/Personal calendars plus Work events land through
  `Calendar/get` and `CalendarEvent/get`. It emits sync markers and
  writes `summary.json` with calendar/event counts and JMAP request
  counters.
- `crates/app/tests/service-harness/m6/calendar_actions_jmap_crud.lua`
  targets the same fixture through JMAP, drives
  `client:execute_calendar_plan`, and verifies create/update/delete
  through `CalendarEvent/set`, local calendar-event state, and a
  follow-up `CalendarEvent/changes` sync.
- `crates/app/tests/sync-harness/jmap-calendar-remote-delta.lua`
  mutates the shared calendar fixture directly through JMAP
  `CalendarEvent/set`, then drives `client:start_calendar_sync` and
  verifies local calendar-event state imports the create/update/delete
  delta through the normal calendar-list refresh plus
  `CalendarEvent/changes` and `CalendarEvent/get`. It emits sync
  markers and writes `summary.json`. The delete assertion also pins
  the account-scoped JMAP calendar tombstone path, which had previously
  passed an account ID into a calendar-ID scoped delete helper.
- `crates/app/tests/sync-harness/gcal-calendar-initial.lua`
  targets the shared calendar fixture through saehrimnir's Google
  Calendar v3 listener, drives `client:start_calendar_sync`, and
  asserts calendarList plus per-calendar events requests import the
  Work/Personal calendars and Work events. It emits sync markers and
  writes `summary.json` with calendar/event counts and Google Calendar
  request counters.
- `crates/app/tests/service-harness/m6/calendar_actions_gcal_crud.lua`
  targets the same fixture through Google Calendar v3, drives
  `client:execute_calendar_plan`, and verifies create/update/delete
  through POST/PATCH/DELETE requests, local calendar-event state, and
  a follow-up events sync. This slice also normalizes Google action
  bodies and accepts cancelled delta tombstones without start/end.
- `crates/app/tests/sync-harness/gcal-calendar-remote-delta.lua`
  mutates the shared calendar fixture directly through Google Calendar
  v3 POST/PATCH/DELETE calls, then drives
  `client:start_calendar_sync` and verifies the local calendar DB
  imports the created event, updated event fields, and deleted-event
  tombstone through normal events sync. It emits sync markers and
  writes `summary.json`.
- `crates/app/tests/sync-harness/people-contacts-initial.lua`
  targets `graph-contacts-small.toml` through saehrimnir's People API
  listener, drives Gmail initial sync, and asserts Google contact rows
  import from `/v1/people/me/connections` while empty
  `/v1/otherContacts` still bootstraps successfully. It emits sync
  markers and writes `summary.json` with final contact counts and
  People request counters.
- `crates/app/tests/sync-harness/people-contacts-incremental.lua`
  targets `graph-contacts-incremental.lua` through the People API,
  applies saehrimnir's contact create/update/delete change steps, runs
  Gmail delta sync until the production twentieth-cycle contact
  cadence fires, and asserts Google contact rows converge after each
  step. This also persists Gmail's delta-cycle counter in sync state,
  matching the Graph contact cadence fix so separate `start_sync`
  requests can reach the twentieth-cycle contact tier. It emits sync
  markers around the measured cadence loops and writes `summary.json`.
- `crates/app/tests/sync-harness/people-contacts-writeback-delete.lua`
  targets `graph-contacts-small.toml` through saehrimnir's People API,
  drives `contacts.contact_save_with_writeback` against a synced Google
  contact, verifies the local phone/company/notes update and the
  provider-side `PATCH /v1/people/{id}:updateContact`, then drives
  `contacts.contact_delete`, verifies the provider-side
  `DELETE /v1/people/{id}:deleteContact`, and runs a follow-up People
  contact-delta cadence loop to prove the deleted contact does not
  return. It emits sync markers and writes `summary.json`.
- `crates/app/tests/sync-harness/gmail-attachment-initial.lua`
  targets `jmap-attach.toml` through the Gmail listener and asserts
  Gmail `threads.get` imports attachment metadata for `sample.txt`.
  It emits sync markers and writes `summary.json` with attachment and
  Gmail request counters.
- `crates/app/tests/sync-harness/graph-attachment-initial.lua`
  targets `jmap-attach.toml` through the Graph listener and asserts
  Graph mail payloads import attachment metadata for `sample.txt`. It
  emits sync markers and writes `summary.json` with attachment and
  Graph request counters. The attachment scripts also hardened JMAP,
  Gmail, and Graph message persistence so provider sync creates the
  placeholder thread and message rows before inserting attachment rows;
  attachment-bearing fixtures now catch FK-order regressions.
- Sync-harness request-log helper cleanup has landed. The Lua harness
  now exposes `harness.join_url`, `harness.mock_requests(endpoint)`,
  `harness.clear_mock_requests(endpoint)`, and
  `harness.request_count(requests, protocol, command)`, plus
  `harness.request_count_prefix(requests, protocol, command_prefix)`
  for requests with generated resource names. The Lua harness also
  exposes `harness.http_json({ method, url, body })`,
  which supports table bodies and arbitrary HTTP methods while keeping
  the older `http_get` / `http_post_json` / `http_delete` helpers for
  compatibility. `harness.http({ method, url, body, content_type,
  if_match })` returns `{ status, ok, body }` for raw text protocols
  such as CalDAV iCalendar PUT/DELETE. `harness.mock_requests(endpoint,
  { stable = true })`
  requests saehrimnir's deterministic request-log shape. Existing
  JMAP, IMAP, Graph calendar, CalDAV calendar, and OAuth fixture
  scripts use the shared helpers instead of copy-pasting local URL and
  request-count utilities.
- `harness.snapshot_state(endpoint)` wraps saehrimnir's
  `GET /test/snapshot-state` fixture projection. The scripted JMAP
  incremental test uses it after each change step to prove the remote
  fixture image changed before ratatoskr imports the delta.
- `harness.latency(endpoint)` and
  `harness.set_latency(endpoint, { global_ms, per_protocol })` wrap
  saehrimnir's `GET/POST /test/latency` controls. The setter parses
  Lua numeric fields as non-negative JSON integers so saehrimnir's
  validation sees the same shape as a raw JSON client.
- `harness.marker(name)` emits sidecar phase markers when
  `BROKKR_MARKER_FIFO` is present and otherwise no-ops, so the same
  script can run under `service-test`, `sync-smoke`, and
  `sync-bench`. `harness.write_summary(table)` writes a JSON object to
  the run artefact's `summary.json` for brokkr's `meta.*` ingestion.
- Lua `ActionCompleted` notifications now expose the action plan
  summary counters (`summary_total`, `summary_remote_succeeded`,
  `summary_remote_failed`, `summary_local_only`, and
  `summary_conflicts`) so scripts can assert remote-dispatch success
  without reverse-engineering it from provider request logs alone.

Resolved saehrimnir dependency for IMAP remote-mutation scripts:

- `POST /test/fixture/step` delete/move coverage through IMAP has
  landed on top of saehrimnir commit `46ded29`. The mock now keeps
  `Fixture::mailbox_uid_history`, with per-mailbox UID slots,
  tombstones for retired messages, monotonically growing UIDNEXT, and
  stable sequence-number projection at FETCH time. UIDVALIDITY remains
  pinned at 1; tests that need a UIDVALIDITY-bump simulation can still
  use `POST /test/fixture/reset`.

Lua helper cleanup backlog:

- Do not add another extract/search script that copy-pastes the
  backfill, attachment polling, search polling, or attachment lookup
  helpers. First hoist them into shared harness helpers or a supported
  Lua include path.

**Exit criteria:**

- A small-mailbox IMAP fixture syncs end-to-end against a fake
  IMAP server, with assertions on final account/folder/message
  counts. Initial import and steady-state delta coverage have landed;
  flag writeback persistence, move/delete writeback persistence, and
  out-of-band remote new/flag-change/delete/move import have landed.
- A small-mailbox JMAP fixture does the same. Initial import,
  steady-state delta, raw `Email/set` mutation, and scripted
  new/change/delete/move incremental coverage have landed; deeper
  JMAP fixture cases for bulk, attachment-bearing, many-folder,
  duplicate-Message-ID, malformed-MIME, and slow-paged-response
  fixtures have landed.
- M6.9's OAuth-enforced sync recovery slice now verifies revoked-token
  failure reporting, manual re-auth persistence, and successful
  follow-up sync.
- M6.10 (calendar) has Graph read/sync, Graph create/update/delete
  action coverage, Graph remote-mutation delta import coverage,
  Graph action-to-delta confirmation, CalDAV initial-sync coverage,
  CalDAV create/update/delete action
  coverage, CalDAV remote-mutation import coverage, and a
  CalDAV-write to Graph-delta shared-fixture proof. Google and JMAP
  now have initial-sync, create/update/delete action coverage, and
  remote-mutation delta import coverage. JMAP also covers
  create-on-provider-failure returning `LocalOnly` while preserving
  the unsynced local event row.

**Remaining actionable work:**

- None for the non-Windows M8 scope. Additional marker/summary adoption
  belongs to optional M9 gate expansion when another script is selected
  as a benchmark gate.

---

### M9 - Sync benchmarks

**Status:** LANDED for the first checked-in gate. M8 has enough
mock-provider coverage for useful benchmarks, and brokkr command
support has landed. Brokkr
`sync-bench --gate <name>` records gated runs in
`.brokkr/ratatoskr/gate.db`, `--as-baseline` prints the per-host
baseline pin to add under `[ratatoskr.gate.<name>.baseline]`, and gate
rules can compare top-level scalars plus `sidecar.*` and `meta.*`
summary fields. Ratatoskr has a checked-in
`jmap_steady_state_delta` gate in `brokkr.toml`, pinned to the
clean-tree `plantasjen` baseline UUID
`5399393bfded447aba4176b586a5905c` (best-of-10, 22 ms). The dirty
bootstrap UUID was retired on 2026-05-10. The gate evaluates 8/8
rules cleanly against that baseline, covering elapsed timing,
sidecar `rss_peak_kb`, provider request count, and the script's
`meta.correct` / `meta.message_count` summary scalars.
Saehrimnir's latency knob now has ratatoskr Lua helpers and a JMAP
smoke script; stable request logs and `GET /test/snapshot-state` also
exist. Ratatoskr now has Lua helpers for `BROKKR_MARKER_FIFO` markers
and `summary.json`, with JMAP steady-state, JMAP Email/set delta, JMAP
scripted incremental, IMAP steady-state, IMAP scripted incremental,
IMAP-to-JMAP shared-state delta, Graph calendar initial, Graph
calendar remote-delta, Graph calendar CalDAV-mutation delta, Graph
contacts initial, Graph contacts incremental, CalDAV calendar initial,
CalDAV calendar remote-delta, JMAP initial, JMAP calendar initial,
JMAP calendar remote-delta, JMAP bulk initial, JMAP bulk steady-state,
JMAP attachment initial, JMAP many-folder initial, JMAP duplicate
Message-ID initial, JMAP malformed-MIME initial, JMAP slow-paged
initial, IMAP initial, Google Calendar initial, Google
Calendar remote-delta, People contacts initial, People contacts
incremental, People contacts writeback/delete, Gmail attachment
initial, and Graph attachment initial scripts using them.
The remaining work is broader ratatoskr-side gate coverage and
additional per-host baseline promotion as needed.

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

Brokkr gates can now fail a run when configured thresholds catch:

- a previously stable Service test hangs;
- boot or respawn time regresses past a threshold;
- sync wall time regresses by more than a configured percentage;
- peak RSS regresses past a configured percentage;
- provider request count increases unexpectedly;
- final correctness assertions fail through `meta.correct` or another
  script-emitted summary scalar.

**Exit criteria:**

- At least one checked-in `[ratatoskr.gate.<name>]` block points at a
  stable sync script, has a clean-tree per-host baseline UUID recorded
  in `.brokkr/ratatoskr/gate.db`, and `brokkr sync-bench <script>
  --gate <name> --bench 10` records timings, compares against that
  baseline, and exits non-zero on regression. Satisfied on 2026-05-10
  by the clean-tree `jmap_steady_state_delta` baseline
  (`5399393bfded447aba4176b586a5905c`).

**Optional follow-ups:**

- Record per-host baselines for any other contributor or CI hosts that
  should run `jmap_steady_state_delta`; the checked-in baseline map is
  currently single-host (`plantasjen`).
- Add more checked-in gates for the next stable benchmark scripts once
  they matter to CI or release decisions. Good candidates are JMAP
  scripted incremental, IMAP steady-state, Graph calendar remote-delta,
  and CalDAV calendar remote-delta.

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
       +-- M6.10 Calendar (unblocks via M8 calendar fake)
```

The Service Phase 8 close-out depends only on M2.

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
