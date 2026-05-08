# Service - Phase 8 Plan: crash recovery polish + invariant pass optimization + close-out

**Status:** planning, pre-implementation. This is the final phase of
the Service-relocation arc that started in Phase 1. When Phase 8
ships, `docs/service/` is empty and gets deleted. This file deletes
with it.

The plan has two halves of work:

- **Implementation** (8-1 through 8-5) - the Service-architecture work
  the original Phase 8 entry in `implementation-roadmap.md` named:
  crash recovery polish, cross-store invariant pass optimization,
  JMAP push hardening, surviving Phase 7 carry-forwards, and the
  account-deletion `is_deleting` gate.
- **Close-out** (8-6 through 8-9) - fold the durable architectural
  content from `problem-statement.md` into `docs/architecture.md`,
  relocate `manual-test-matrix.md` to the harness directory, retire
  the per-phase plans, retire the implementation roadmap, delete the
  Service docs directory.

## Companion documents

- `docs/harness/roadmap.md` - Phase 8's test cohort items (the wedge,
  T1, the Phase 7 extract cohort, manual-matrix automation, the
  `--test-fake-schema=N` e2e) live in the harness roadmap, not here.
  Phase 8's close-out depends on harness **M2** (the wedge) passing.
  The implementation work itself does not gate on the harness - code
  changes can land independently of test coverage shape.
- `docs/harness/architecture.md`, `docs/harness/problem-statement.md`
  - context for the harness move.
- `docs/architecture.md` - the durable architecture document. The
  close-out promotes content from `docs/service/problem-statement.md`
  into it.

## Why Phase 8 is the close-out

Phase 1 split the app into UI + Service processes. Phases 1.5 through
7 progressively relocated every write surface from the UI into the
Service. Phase 7 closed the last user-visible feature surface
(attachment text extraction + Tantivy indexing). What remains in the
original Phase 8 scope is **polish on the architecture that's already
landed**: making crashes recoverable, making the boot-time invariant
pass fast enough for 200 GB mailboxes, hardening push, and resolving
carry-forwards from earlier phases. There is no Phase 9 - the
"tray-resident" entry that sat in the original roadmap is dropped (no
plans for a tray icon).

Once these are in tree the Service is fully realized. The Service
docs (the per-phase plan files, the implementation roadmap, the
problem statement) document a relocation that's done; their
information either migrates to the durable architecture doc (for
content that future readers will need) or rides with git history (for
content about how the work proceeded). Either way it doesn't need a
home in `docs/service/`.

---

## Implementation

### 8-1 Crash recovery polish

The largest cluster. Surfaces the user sees when the Service is
unhealthy, plus the drain-ordering and emit-class work that was
entangled with the two flaky `service_subprocess` tests pre-harness.

- **Exponential backoff respawn.** Replaces Phase 1.5's fixed 1-second
  cooldown + sliding-window crashloop guard. The "duplicate boot work
  on respawn" cost (each respawn re-runs the entire boot sequence
  including `reconcile_velo_rename`) amplifies the lockfile race
  under tight crashloop; backoff + crashloop detection together
  remove the amplification. Lands in
  `crates/app/src/service_client.rs`.
- **Crashloop detection refinement.** If respawn fails N times in M
  seconds, surface a permanent error state in the UI ("Service can't
  start - check logs"). Phase 1.5's flat
  `CRASHLOOP_THRESHOLD = 3 / CRASHLOOP_WINDOW = 30s` policy gets
  replaced. Test for the "3 crashes → 3 successful recoveries → 3
  more crashes within window should NOT trip" case (Phase 1.5
  carry-forward, flagged by the arch review) lands in
  harness M4 against the new shape, not against the
  sliding-window placeholder.
- **UI status indicator for Service health.** Small banner or
  status-bar element distinguishing "respawning" from "persistently
  failing" from "healthy." Lands in
  `crates/app/src/ui/status_bar.rs`.
- **In-flight request idempotency contract.** In-flight requests are
  either replayed (idempotent) or failed back to the caller (not).
  Per-method idempotency contract recorded in `service-api`.
- **Retry-queue persistence verify.** The `pending_ops` retry queue
  already persists across Service restarts; this entry is a verify
  pass + a real-subprocess test (lands as a harness M4 script).
- **Heartbeat policy refinement.** Distinguish "dispatch loop alive"
  from "no progress on a long-running task." Generous timeout on
  first heartbeat after a sync starts; require N consecutive misses
  before respawning rather than 1.
- **Drop-watchdog and `wait_with_kill_watchdog` escalation policies
  unified.** Phase 1.5 ships two kill-escalation paths whose budgets
  are intentionally different (Drop / `async_drop_wait` is the
  user-quit path with ~1.7s patience; `wait_with_kill_watchdog` is
  the respawn path with ~6s). Phase 8 extracts a shared helper that
  takes the budget shape as a parameter, with a doc-comment naming
  why each call site picks its budget; loses the rationale-drift
  risk without losing the distinction.
- **Soft-cancel signal for `boot.ready` to avoid mid-COMMIT SIGKILL.**
  Phase 1.5 carry-forward, flagged by the bugs review.
  `crates/service/src/dispatch.rs::run_dispatch_loop` orders
  `drain_in_flight` before `boot_handle.abort()`; a `boot.ready`
  parked on `wait_for_ready` keeps drain awaiting until
  `spawn_blocking` migration completes. UI Drop's `wait_with_kill_watchdog` is 1s
  before SIGKILL; on a mid-`COMMIT` Service, SQLite WAL recovers and
  the next boot redoes the migration - the same "duplicate boot work"
  cost the backoff bullet flags. The fix has `boot.ready` respect a
  soft-cancel signal so the Drop watchdog doesn't escalate at all on
  big migrations.
- **Class-aware `boot_progress::emit` helper.** Phase 2 carry-forward.
  The first attempt to make the helper pick `try_send` for
  Coalesce/Drop and awaited `send` for MustDeliver introduced a hang
  in the `service_subprocess` cohort and was reverted; today's helper
  uses `try_send` only, which is structurally incompatible with
  MustDeliver semantics. The contract noted in
  `crates/service/src/boot_progress.rs` ("`OUTBOUND_QUEUE_CAP=1024`
  must remain >> Phase-1.5 boot frame count") is doc-only - no
  per-phase regression test bounds total emit count. Phase 8 owns
  re-attempting the helper *after* harness M2 makes the underlying
  drain bug deterministic. Either ship the class-aware helper with
  the underlying drain fix, or replace the contract with per-emitter
  regression tests bounding emit count per boot phase. Coalesce-class
  `try_send` remains correct for `boot.progress` / `sync.progress`
  regardless.
- **`from_boot_ready` async store init.** Phase 2 carry-forward. Body
  / inline / search store init still runs synchronously inside
  `crates/app/src/app.rs::from_boot_ready` after `boot.ready`. On a
  slow disk this momentarily blocks the splash transition with a
  frozen view. Pure UI surgery: relocate the store init to async
  tasks dispatched from `BootingApp::update`; the `Booting → Ready`
  transition fires earlier; async store-init tasks fire
  `Message::ReadyStoreReady(...)` events that finalize the `ReadyApp`
  field set incrementally.

**Touchpoints:**
- `crates/app/src/service_client.rs` - backoff + crashloop detection
  + status reporting + heartbeat policy refinement.
- `crates/app/src/ui/status_bar.rs` - Service-health indicator.
- `crates/app/src/app.rs` - async store init, new
  `Message::ReadyStoreReady(...)` arms.
- `crates/service/src/boot_progress.rs` - class-aware emit helper.
- `crates/service/src/dispatch.rs` - soft-cancel signal for
  `boot.ready`; Drop-watchdog unification.
- `crates/service-api/src/` - idempotency contract per method.

---

### 8-2 Cross-store invariant pass optimization

The minimal pass landed in Phase 3 (body / inline orphan scans;
Tantivy iteration was deferred) and Phase 6 (blob orphan scan);
both are full-table walks gated only on the missing `clean_shutdown`
sentinel. On a 200 GB
mailbox the cost is multi-minute boot delay every time the previous
shutdown was unclean (which on Windows is most non-graceful exits per
the exit-path matrix). Phase 8 makes them fast.

- **Marker-file gating.** Track a "last clean shutdown" cursor per
  store; scan only what's been written since. Bounded to a known
  budget on a 200 GB mailbox via per-store cursors. Implementation:
  prefer a small per-store SQLite row over a flat file; SQLite gives
  atomicity for free.
- **Tantivy orphan iteration.** Phase 3 carry-forward. The Phase 3
  invariant pass clears `history_id` per dirty account and drops
  body / inline orphans; Tantivy orphan iteration was deferred. Add
  the Tantivy scan (per dirty account: iterate index, drop docs
  whose `message_id` is no longer in `messages`) alongside the
  marker-file gating so they share the per-account scan loop.
- **`attachment_extracted_text` orphan sweep.** Phase 7 carry-forward.
  Folds into the same per-account scan loop. Worst-case 100 KB per
  orphan content_hash; typical 1000-msg/day mailbox with 5%
  attachment turnover ~150 orphans/year ~15 MB/year accumulation.

**Touchpoints:**
- `crates/service/src/startup_invariants.rs` - extend with marker-
  file gating + bounded windows; the minimal pass scaffolding
  already exists from Phases 3 and 6.

**Exit criteria:**
- Invariant pass runs in <5s on a typical mailbox; <30s on a 200 GB
  mailbox. Logged stats let us see how often crashes leave us
  reconciling.
- Tantivy orphan scan integrated; observable via the same logged
  stats.

---

### 8-3 JMAP push hardening

Two Phase 4 carry-forwards. Both are correctness-preserving without
the fix; Phase 8 makes them robust under crash.

- **Push re-auth re-arm.** UI-side re-auth
  (`AddAccountWizard::new_reauth` at
  `crates/app/src/handlers/accounts.rs`) updates the existing account
  row in place and does NOT trigger `PushRuntime::start_account`. So
  a JMAP token-revocation kills push for that account until Service
  restart, even after the user re-authorizes - the dead `PushRuntime`
  entry has no path to re-arm. Phase 8 wires push re-arm to a
  token-refresh-success event (or to a UI-side
  `account.reauthorized { account_id }` IPC). Manual workaround
  today: restart the Service.
- **Push state hardening.** Today's `start_push` unconditionally
  loads `jmap_push_state.push_state` and sends it in
  `WebSocketPushEnable`. On crash, Phase 3's invariant pass clears
  `history_id` so a stale `StateChange` resolves correctly via
  re-fetch; the resume path is correctness-preserving. Phase 8
  hardens it: detect crashed accounts (via the same Phase 3
  sync-marker signal) and force a fresh-start by clearing
  `push_state` before `start_push`. Adds an explicit fresh-start knob
  on `start_push` rather than a pre-call `save_push_disabled`
  workaround.

**Touchpoints:**
- `crates/service/src/push.rs::PushRuntime::start_account` - push
  state hardening.
- `crates/service/src/push.rs::PushRuntime` + an event emission
  point in the OAuth refresh path - re-auth re-arm.

---

### 8-4 Phase 7 architectural carry-forwards

Phase 7's plan-doc named six carry-forwards that survived close-out.
Two fold into other Phase 8 clusters (orphan sweep → 8-2;
real-world fixture corpus → harness M5). Four remain here.

- **PreserveExisting dual-index path.** v1 of Phase 7 ships
  Wipe-only - search is briefly unavailable while a rebuild runs.
  Phase 8 reintroduces the originally-planned dual-index path: open
  `<search_index_next>/` adjacent with a parallel writer; route
  writes there; atomic-swap the directory; UI reader rebinds.
  Requires plumbing rework: today's `SearchWriteHandle` is moved at
  construction into `SyncRuntime`, not consulted via `boot_state`,
  so the dual-writer scaffolding has to thread through the runtime.
  This is also the moment the
  `RebuildPolicy::PreserveExisting` wire variant comes back -
  Phase 7 close-out (M12) deleted the variant since the
  implementation didn't ship; Phase 8 restores it.
- **Status-bar visual surface for `IndexRebuildProgress` +
  `IndexRebuildCompleted` reader rebind.** Phase 7's
  `IndexRebuildProgress` / `IndexRebuildCompleted` notification arms
  in `update.rs` log at `info` level only; future work stores
  `Option<RebuildProgressState>` on `ReadyApp` and renders in
  `status_bar.rs`. Same-PR work: re-run `SearchReadState::init` on
  `IndexRebuildCompleted` so the new index is reachable in-session
  (today the UI keeps the stale reader handle until the next app
  launch). Both are pure UI surgery; the wire path is verified
  end-to-end via logs.
- **`local_drafts` re-emit during Wipe rebuild.**
  `run_wipe_rebuild` iterates `messages` only; `local_drafts` rows
  that were in the search index pre-rebuild vanish until a draft
  auto-save touches them. Acceptable v1 because draft rows are rare
  and the next save round-trips them; Phase 8 cleans up alongside
  the visual-surface work. (Lands in
  `crates/service/src/rebuild.rs::run_wipe_rebuild_inner`.)

(Real-world fixture corpus - checked-in `.pdf` / `.docx` / `.xlsx` /
`.pptx` corpus + the malicious zip-bomb `.docx` - moves to harness
**M5**, not Phase 8. The fixtures are test infrastructure; they
belong with the integration cohort, not with Service architecture.)

**Touchpoints:**
- `crates/search/src/lib.rs` + `crates/service/src/search_writer.rs`
  + `crates/service/src/rebuild.rs` - PreserveExisting plumbing.
- `crates/service-api/src/extract.rs` - restore
  `RebuildPolicy::PreserveExisting` variant.
- `crates/service/src/dispatch.rs::spawn_post_ready_schema_rebuild` -
  switch from hardcoded `Wipe` to PreserveExisting where the
  plumbing supports it.
- `crates/app/src/update.rs` - `Option<RebuildProgressState>` on
  `ReadyApp`, `IndexRebuildProgress` / `IndexRebuildCompleted`
  consumption, reader rebind on completion.
- `crates/app/src/ui/status_bar.rs` - rebuild-progress rendering.
- `crates/service/src/rebuild.rs::run_wipe_rebuild_inner` -
  `local_drafts` re-emit.

---

### 8-5 Account-deletion `is_deleting` gate

Phase 3 carry-forward. The plan called for an `accounts.is_deleting`
schema column + UI-side `SyncTick` filter (skip deleting accounts) +
Service-side defense-in-depth check in `SyncRuntime::start_account`.
The load-bearing `cancel_and_await` flow shipped without it, so a
`SyncTick` firing between the cancel-ack and the row-delete can
re-kick a sync against the disappearing account. The cancel races the
start; either the new run gets the cancel (correct outcome) or runs
to completion against a half-deleted account (briefly inconsistent
until the row delete finalizes).

The fix adds the column + both gates so the deletion flow is
monotonic.

**Touchpoints:**
- `crates/db/src/db/schema/01_core.sql` - `accounts.is_deleting`
  column.
- `crates/app/src/...` - `SyncTick` account-list filter.
- `crates/service/src/sync.rs::SyncRuntime::start_account` -
  defense-in-depth gate.

---

## Close-out

The close-out work runs as the final commits of Phase 8. Order:

1. Run 8-6 (promote durable content into `docs/architecture.md`) -
   independent of implementation order; can land first.
2. Run 8-7 (relocate `manual-test-matrix.md`) - depends on harness M1
   landed (so `docs/harness/` exists with companion docs).
3. Run 8-8 (per-file disposition) - verify each
   `docs/service/*` file has a target.
4. Run 8-9 (delete the directory) - final commit; this plan deletes
   with it.

### 8-6 Promote durable content into `docs/architecture.md`

`docs/service/problem-statement.md` carries durable architectural
content not yet in `docs/architecture.md`. The promotion target is a
new section (or set of sections) in `docs/architecture.md`. Specific
content:

- **IPC contract.** JSON-RPC 2.0 over stdio, newline-delimited
  framing constraint, the wire-format crate (`service-api`) +
  `write_message` helper, the **notification class taxonomy**
  (`Coalesce { key }` / `Drop` / `MustDeliver`), the single ordered
  notification channel design, the inbound framing cap, the
  bounded-in-flight-requests semaphore + handler-side acquire, the
  outbound `MustDeliver` interaction, the writer-task drain, the
  per-method timeout table, the large-blob policy, the per-line
  frame size cap.
- **Process model.** UI process + Service process, parent-child
  spawn via `tokio::process::Command`, `kill_on_drop` disabled
  rationale, the explicit shutdown handshake, parent-death machinery
  (Linux PR_SET_PDEATHSIG + getppid recheck; Windows Job Object
  kill-on-job-close), the deferred macOS kqueue design, single-
  instance OS file lock + `BootExitCode` taxonomy, log file naming.
- **Boot handshake.** Two-phase from Phase 1.5 onward: `health.ping`
  with `PROTOCOL_VERSION` first, then `boot.ready` with
  `{ ready, schema_version, migrations_applied }`. UI splits on
  `SpawnEvent::ChildSpawned` and `SpawnEvent::BootReady`.
- **Cross-store crash consistency.** The `clean_shutdown` sentinel
  contract, the exit-path matrix (graceful UI quit, UI-quit-but-
  unresponsive on Linux/Windows, panic in handler debug vs release,
  external SIGTERM/SIGKILL/TerminateProcess, hard power-off - and
  whether each writes the sentinel + triggers the recovery scan),
  the rationale for full-table scans being correctness-preserving,
  the marker-file gating that 8-2 implements.
- **Service-generation contract.** UI-side counter, bumped on every
  respawn; reader task tags every notification at enqueue; dispatch
  drops notifications whose tag does not match the live generation.
  Closes the cross-respawn race for stale `BootProgress` /
  `action.completed` / etc.
- **Stdio discipline (corruption defense).** The Service claims its
  real stdin/stdout at the top of `run_service()` and replaces the
  standard slots with sinks before any other code runs. Per-platform
  mechanism (Linux dup + dup2 to /dev/null; Windows DuplicateHandle
  + SetStdHandle). Inheritance for grandchildren.
- **Sensitive-value logging policy.** Loggable / not-loggable lists.
  `RedactedString` / `RedactedBytes` wire-type pattern.

The `docs/architecture.md` "Settled patterns" section is the natural
home for the policy-shaped items (notification class taxonomy,
sensitive-value logging, stdio discipline). The lifecycle / IPC /
crash-consistency content gets its own sibling section, probably
"Service process model" or similar.

**Not promoted** (rides with git history):
- The Phase-by-Phase status retrospectives in problem-statement.md
  (Phase 2 / 3 / 4 / 5 / 6a / 6b / 6c / 6d / 7 status sections).
  These document how the work proceeded; the durable lessons are
  already in `docs/architecture.md` and the per-component docs.
- The "Why decide this now" framing - context only, the decision is
  made and the architecture is shipped.
- The "What goes in v1" list - by Phase 8 ship, "v1" means current
  ratatoskr.
- The migration policy (atomic-commit, single-binary-cost) - the
  migration is done; the policy was load-bearing during the work
  but isn't an ongoing concern.
- The write-surface inventory table - the table tracked which UI
  write surface relocated when; by Phase 8 ship, every entry is
  LANDED. The architectural shape (Service-side write surfaces) is
  already in `docs/architecture.md`.

### 8-7 Relocate `manual-test-matrix.md`

Move `docs/service/manual-test-matrix.md` to
`docs/harness/manual-test-matrix.md`. Update the references in:

- `docs/service/problem-statement.md` (will be retired in 8-9 anyway,
  but during the close-out window the link should go to the new
  path).
- `docs/harness/roadmap.md` and `docs/harness/problem-statement.md`,
  which already prefigure the relocation in five places and link
  `docs/service/manual-test-matrix.md` directly.
- Any `// MANUAL TEST REQUIRED` comments in
  `crates/process-lifetime/src/{linux,windows}.rs` (the parent-death
  code was extracted to its own crate post-Phase-1) or similar
  source-side pointers.

The matrix itself doesn't change in content; it just relocates. The
harness roadmap M6 then absorbs items into automation incrementally;
when M6 completes, the matrix is empty and gets deleted entirely.

### 8-8 Per-file disposition

| File | Disposition |
| --- | --- |
| `docs/service/implementation-roadmap.md` | RETIRE. The Service-relocation arc is done; every phase entry is LANDED. The doc was a planning artefact, not a reference. |
| `docs/service/problem-statement.md` | RETIRE after 8-6. Durable content has been promoted to `docs/architecture.md`. |
| `docs/service/manual-test-matrix.md` | RELOCATE to `docs/harness/manual-test-matrix.md` (8-7). |
| `docs/service/phase-1-plan.md` | DELETE. Process-boundary scaffolding is shipped; the durable architectural decisions are in `docs/architecture.md`. The implementation log is git history. |
| `docs/service/phase-1.5-plan.md` | DELETE. Same. |
| `docs/service/phase-2-plan.md` | DELETE. Same. |
| `docs/service/phase-3-plan.md` | DELETE. Same. |
| `docs/service/phase-4-plan.md` | DELETE. Same. |
| `docs/service/phase-5-plan.md` | DELETE. Same. |
| `docs/service/phase-6a-plan.md` | DELETE. Same. |
| `docs/service/phase-6b-plan.md` | DELETE. Same. |
| `docs/service/phase-6c-plan.md` | DELETE. Same. |
| `docs/service/phase-6d-plan.md` | DELETE. Same. |
| `docs/service/phase-7-plan.md` | DELETE. Same. |
| `docs/service/phase-8-plan.md` | DELETE. This file. The implementation work is shipped, the close-out is shipped, the doc has done its job. |
| `docs/service/brokkr-phase-8-scaffolding.md` | DELETE. Already relocated to `docs/harness/architecture.md` during the harness roadmap M1; this is the final cleanup. |

Anyone needing the historical detail about how Phase N landed reads
the git log of the relevant commits.

### 8-9 Delete `docs/service/`

Final commit. After 8-6, 8-7, and 8-8 land:

- `git rm -r docs/service/`
- Verify `docs/service/` is absent from the repo.
- Verify no surviving link points at `docs/service/<anything>`. The
  audit must cover **source code** as well as docs - the relocation
  arc seeded ~35 module-doc / Cargo.toml / SQL-comment pointers of
  the form `// Phase N of \`docs/service/phase-N-plan.md\`` across
  `crates/service-api/`, `crates/service/`, `crates/service-state/`,
  `crates/action-types/`, `crates/db/`, `crates/jmap/`,
  `crates/graph/`, `crates/imap/`, `crates/calendar/`,
  `crates/process-lifetime/`, and a handful of `crates/app/` sites.
  Each becomes a dangling pointer the moment the directory
  disappears. (One is dangling already: `crates/calendar/src/jmap.rs`
  references the long-deleted `docs/service/discrepancies.md`.)
  Doc-side references outside the close-out flow that need rewrites:
  `docs/architecture.md` (the `WriteDbState` bullet referencing
  `docs/service/problem-statement.md`) and
  `docs/attachments/{problem-statement,implementation-roadmap}.md`
  (cross-document dependencies on the Service problem statement and
  roadmap). Replacement strategy: for source-side pointers, either
  inline the rationale into the comment or drop it; for the
  architecture / attachments docs, redirect to the new Service
  sections in `docs/architecture.md` from 8-6.

---

## Exit criteria

Implementation:
- ✓ All 8-1 sub-items LANDED. Killing the Service mid-sync results in
  a respawn within a few seconds; backoff prevents tight crashloops;
  status indicator surfaces degraded state. A persistently failing
  Service surfaces a clear UI error rather than silent breakage.
- ✓ 8-2 LANDED. Invariant pass <5s typical, <30s on 200 GB. Tantivy +
  `attachment_extracted_text` orphan scans included.
- ✓ 8-3 LANDED. Push survives token revocation + re-auth without a
  Service restart; crashed accounts force-clear `push_state` on
  next start.
- ✓ 8-4 LANDED. PreserveExisting dual-index path operational; user
  search stays live during a rebuild. Status-bar progress + reader
  rebind on completion. `local_drafts` re-emitted across Wipe rebuilds.
- ✓ 8-5 LANDED. Account deletion is monotonic; no SyncTick re-kick
  against a deleting account.
- ✓ Heartbeat false-positive rate (load-induced miss interpreted as
  crash) goes to zero.

Test coverage (lives in harness roadmap, listed here as gating):
- ✓ Harness M2 LANDED. All five wedge scripts
  (`ping_and_shutdown.lua`, `two_phase_spawn.lua`,
  `terminal_on_missing_key.lua`, `respawn_after_sigkill.lua`,
  `pending_at_respawn.lua`) pass consistently, including under
  200-iteration soak.

Close-out:
- ✓ `docs/architecture.md` contains the IPC contract, process model,
  boot handshake, cross-store crash consistency, service-generation
  contract, stdio discipline, and sensitive-value logging policy
  sections.
- ✓ `docs/harness/manual-test-matrix.md` exists; the
  `docs/service/manual-test-matrix.md` path no longer resolves.
- ✓ `docs/service/` is deleted from the repo.
- ✓ No link in any surviving doc points at `docs/service/<anything>`.
- ✓ This file is gone with the directory.

## Suggested implementation order

The implementation clusters (8-1 through 8-5) are independent at the
code level; their internal sequencing is up to whoever picks them
up. Some natural pairings:

1. **8-2 first if invariant-pass cost is hurting users.** It's
   purely Service-internal work, no UI plumbing, no harness
   gating. Most measurable user-visible improvement.
2. **8-1 second.** The recovery + boot polish cluster is the most
   substantial work and the most user-visible. Several sub-items
   benefit from harness M2 being available for verification (the
   class-aware emit re-attempt particularly), so harness M2 ideally
   lands in parallel.
3. **8-3 in parallel** with 8-1 and 8-2 - different code area,
   different reviewers.
4. **8-4 after 8-1 / 8-2 / 8-3.** PreserveExisting is the largest
   single sub-item and benefits from the rest of Phase 8 being
   solid first. The status-bar + `local_drafts` work is small and
   can slot anywhere.
5. **8-5 anywhere.** Smallest cluster.
6. **Close-out (8-6 through 8-9) last.** Gated on all
   implementation landed and on harness M1+M2 landed (so
   `docs/harness/manual-test-matrix.md` has a home).

Each sub-slice that lands gets a per-slice retrospective bullet here
in this plan as it goes (mirroring the convention from
`phase-7-plan.md`'s § "Phase 7 status (as landed)") so the close-out
documents what shipped before the file deletes.

## Phase 8 status (as landed)

- **8-1** LANDED in four commits (A-D); two carry-forwards (E, F)
  deferred behind harness work, called out per-bullet below.
  - **A** (respawn backoff + heartbeat policy). Replaced the Phase
    1.5 fixed 1 s cooldown + sliding-window crashloop guard with
    exponential backoff (1, 2, 4, 8, 16, 30 s cap) and a
    reset-on-success unbroken-crash counter that closes the
    "3-crash, 3-recovery, 3-crash within window" false positive.
    Heartbeat now requires N=3 consecutive misses before tripping
    and elongates to a 60 s deadline when a `SyncProgress`
    notification was observed within the last 30 s. The Phase 1.5
    `crashloop_threshold_emits_terminal_after_third_crash`
    subprocess test (which encoded the OLD sliding-window
    semantics) joined the four other libtest-subprocess flakes
    under `#[ignore]`; harness M4 covers the new unbroken-crash
    semantics via `unbroken_crashes_trip_persistently_failing.lua`.
  - **B** (ServiceHealth + idempotency contract). New
    `ServiceHealth` enum + `SpawnEvent::HealthChanged` variant;
    emit on respawn / persistent-failure transitions. New
    `Idempotency` enum + `RequestParams::idempotency()` mapping
    every variant to `Idempotent` / `Mutating` / `Conditional`.
    Status-bar visual surface for the health indicator stays
    deferred as a small UI follow-up; the data plumbing is in.
  - **C** (drop-watchdog unification + soft-cancel for boot.ready).
    Named-constant kill-escalation budgets (`DROP_ABORT_DEADLINE`,
    `DROP_EXIT_DEADLINE_HEALTHY`, `DROP_EXIT_DEADLINE_BOOTING`,
    `POST_KILL_WAIT`) shared between `async_drop_wait` and
    `wait_with_kill_watchdog`. The Drop watchdog elongates to 60 s
    when a `BootProgress` notification is recent (within
    `BOOT_PROGRESS_RECENT_WINDOW`), avoiding mid-COMMIT SIGKILL
    during long migrations. The plan-doc's `KillEscalationPolicy`
    struct is intentionally not extracted - the two paths' work
    shapes diverge enough that a struct would force a less natural
    layout; named constants + doc-comments capture the rationale.
  - **D** (async store init in `from_boot_ready`). Body / inline /
    search store init now runs as `Task::perform` post-`BootReady`
    so the splash-to-Ready transition fires immediately rather than
    blocking on slow-disk file I/O. Three new `Message` variants
    (`BodyStoreReady`, `InlineImageStoreReady`, `SearchStateReady`)
    populate the `Option<...>` fields on `ReadyApp` as each init
    completes. UI surfaces gracefully render their existing
    "loading..." placeholder while the fields are None.
  - **E** (class-aware `boot_progress::emit`) DEFERRED behind
    harness M2. The earlier attempt was reverted because of
    subprocess-cohort flakes; once M2 lands the wedge scripts as
    `.lua` (and the `#[ignore]`'d libtest tests are retired), the
    class-aware emit re-attempt has deterministic verification.
  - **F** (retry-queue persistence verify) DEFERRED behind harness
    M4. The `pending_ops` retry queue already persists across
    restarts; the verify is a real-subprocess test that lands in
    the M4 T1 cohort.

- **8-2** LANDED. Cross-store invariant pass now bounds its scans
  via per-store cursors (`clean_shutdown_cursors` table in main
  DB, advanced on graceful shutdown drain) plus `inserted_at`
  column on `bodies`. Tantivy orphan iteration added (per-account
  scope via `find_orphan_message_ids_for_account`).
  `attachment_extracted_text` orphan sweep folded into the
  per-account loop. Per-helper elapsed-ms logging in
  `InvariantPassStats` makes the <5 s typical / <30 s 200 GB exit
  criterion observable in production.

- **8-3** LANDED. JMAP push hardening: re-arm on re-auth (OAuth +
  password paths), `fresh_start: bool` knob threaded through
  `start_account` -> `start_push`, dirty-account discovery via
  `discover_dirty_accounts` orchestrated in
  `dispatch::spawn_post_ready_push_startup`. Pre-existing reactive
  `save_push_disabled` path unchanged.

- **8-4** PARTIAL.
  - Sub-item 2 LANDED. `Notification::IndexRebuildProgress` /
    `IndexRebuildCompleted` now drive `ReadyApp::index_rebuild_progress`;
    completion triggers a `SearchReadState::init` rebind so the new
    index is reachable in-session. Status-bar visual render is a
    small follow-up; the data is wired.
  - Sub-item 1 (PreserveExisting dual-index path) DEFERRED. Largest
    remaining piece of Phase 8 implementation; reintroduces the
    dual-writer + atomic-swap plumbing deleted in Phase 7's M12
    close-out (commit `b2638752`). Threads through `SyncRuntime`
    because today's `SearchWriteHandle` is moved at construction.
    Genuinely a meaty follow-up commit; not gated on harness.
  - Sub-item 3 (`local_drafts` re-emit) DEFERRED. No-op against
    current code: drafts are not in the search index today (the
    plan-doc reference predates the current draft-indexing state).
    Revisit when drafts join the search index.

- **8-5** LANDED. `accounts.is_deleting` column + Service-side
  defense-in-depth gate in `SyncRuntime::start_account` rejecting
  starts against accounts with the flag set. UI-side SyncTick
  filter not landed; the Service-side gate is the
  correctness-preserving piece, the UI filter is a small UX
  optimization for follow-up.

- **8-6 through 8-9** (close-out) BLOCKED on harness M1 + M2
  landing on the ratatoskr side. The brokkr-side scaffolding for
  M1 + M2 is ready per `notes/ratatoskr-service-harness.md`; the
  ratatoskr-side `crates/app/src/harness/` module + the wedge
  scripts unblock the close-out.
