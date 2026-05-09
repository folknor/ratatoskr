# Service - Phase 8 close-out audit

This file is the post-close-out audit of the Service-relocation phase plans.
The phase plans themselves were deleted in commit `41c18c87` ("service: close
out phase 8 docs") as part of the planned 8-9 close-out step. This audit was
produced one day later by restoring the deleted tree at `1a936620^`,
verifying every claim in `phase-8-plan.md` against current code on `main`,
and keyword-scanning every doc under `docs/service/` (restored), `docs/
architecture.md`, `docs/harness/`, and `docs/roadmap/` for `DEFERRED` /
`PARTIAL` / `not landed` / `carry-forward` / `BLOCKED` / `TODO` / `FIXME`,
plus a source-side `TODO` / `FIXME` scan across `crates/*/src/`.

## Verdict

The Phase 8 implementation work is genuinely landed. The retrospective at
commit `322a1c9d` (24 h before close-out) correctly identified deferrals;
the rebuild-surfaces commit `1a936620` then closed each one. Every
implementation claim verifies against current code.

The close-out is mostly clean, with two real gaps and a handful of stale
references. Details below.

---

## Phase 8 scope items - verified

| Item | Claim | Verified by |
|---|---|---|
| 8-1 A | Exponential backoff (1,2,4,8,16,30s cap), unbroken-crash counter, N=3 heartbeat misses, 60 s elongation under SyncProgress | `crates/app/src/service_client.rs:48,2332,4041` (`crashloop_tracker_fires_on_third_unbroken_crash`) |
| 8-1 B | `ServiceHealth` enum, `SpawnEvent::HealthChanged`, `Idempotency` enum, `RequestParams::idempotency()` | `service_client.rs:552,561,581` + `service-api/src/request.rs:613,849` |
| 8-1 B (UI) | Status-bar visual surface (was deferred in retrospective, claimed landed in `1a936620`) | `crates/app/src/ui/status_bar.rs:83,428,591,703` (resolves all four `ServiceHealth` variants) |
| 8-1 C | `DROP_ABORT_DEADLINE`, `DROP_EXIT_DEADLINE_HEALTHY`, `DROP_EXIT_DEADLINE_BOOTING`, `POST_KILL_WAIT`, `BOOT_PROGRESS_RECENT_WINDOW` constants shared between watchdogs | `service_client.rs:88,93,101,107,115,2739,2830` |
| 8-1 D | Async store init via `BodyStoreReady` / `InlineImageStoreReady` / `SearchStateReady` Message variants | `crates/app/src/app.rs:510,521,532` + `update.rs:222,230,238` |
| 8-1 E (was DEFERRED in retro) | Class-aware `boot_progress::emit` - `MustDeliver` awaits queue capacity | `crates/service/src/boot_progress.rs:44,61,126` (`NotificationSender::send` + `try_send_with_class_check` refusing `MustDeliver`) |
| 8-1 F | Retry-queue persistence verify | Harness script `crates/app/tests/service-harness/t1/retry_queue_persists_across_respawn.lua` (per `docs/harness/roadmap.md:332`) |
| 8-2 | `clean_shutdown_cursors` cursor gating + Tantivy orphan sweep + `attachment_extracted_text` orphan sweep | `crates/service/src/startup_invariants.rs:71,104,425` + `dispatch.rs:480,501` |
| 8-3 | `fresh_start: bool` knob, `discover_dirty_accounts`, push re-arm | `crates/jmap/src/push.rs:156,162,180,308` + `service/src/dispatch.rs:1031,1044,1049` |
| 8-4 sub-1 (was DEFERRED in retro) | PreserveExisting dual-index path | `service-api/src/extract.rs:68` + `service/src/rebuild.rs:97` + `service/src/dispatch.rs:1250` (`spawn_post_ready_schema_rebuild` now dispatches PreserveExisting) + `crates/search/src/lib.rs:332,357,377` |
| 8-4 sub-2 | `IndexRebuildProgress`/`Completed` consumption + reader rebind | `app/src/app.rs:57,112,402` (`RebuildProgressState`) + `update.rs:377,391,404` (rebind via `Message::SearchStateReady`) + `status_bar.rs:728` |
| 8-5 | `accounts.is_deleting` column + Service-side gate + UI SyncTick filter | `db/src/db/schema/01_core.sql:54` + `service/src/sync.rs:185,202` + `app/src/handlers/provider.rs:53` (`.filter(|a| !a.is_deleting)`) |

## Phase 8 scope items - legitimately deferred

| Item | Status | Reason |
|---|---|---|
| 8-4 sub-3 (`local_drafts` re-emit) | DEFERRED | No-op against current code: drafts are not in the search index. Will revisit if drafts get indexed. |
| Compose-send 50 MB attachment / oversize harness scripts | DEFERRED to harness M4 backlog | Blocked on saehrimnir mock SMTP path (`docs/harness/architecture.md:563`). The action-pipeline code is fully in tree. |
| macOS parent-death | Deferred to post-1.0 | Documented in `crates/process-lifetime/src/lib.rs:44` and the (now-deleted) `phase-1-plan.md:54`. |
| Phase 9 tray-resident mode | Descoped | Phase 8 plan: "no Phase 9 - the 'tray-resident' entry that sat in the original roadmap is dropped (no plans for a tray icon)." |

## Close-out (8-6 / 8-7 / 8-8 / 8-9) - verified

- 8-6: `docs/architecture.md` "Service process model" (lines 49-117) covers
  IPC contract, two-phase boot, generation counters, drain order,
  parent-death platform table, stdio discipline, log redaction policy,
  three notification classes. Cross-store crash consistency at line 118.
- 8-7: `docs/harness/manual-test-matrix.md` exists.
- 8-8 + 8-9: `docs/service/` was deleted in `41c18c87`. (This file is the
  intentional re-introduction; the rest of the tree stays gone.)

---

## Real gaps - silently dropped from the plan tree

> **Resolution (same session):** both gaps below were closed by recording
> the descope decisions in `docs/architecture.md` "Current Exceptions",
> rewriting the stale forward-references in code, and updating
> `docs/harness/roadmap.md`. The text in this section is preserved as
> the original audit finding.

### Plan: physical relocation of provider sync into `provider-sync`

The audit-time recording of "Current Exception" makes the gap visible
but does not close it. The plan to actually close it:

**Choice.** Physical relocation into `provider-sync`, not a
`write-handles` type-split. The architecture document is built on
strict structural boundaries; the closure must be structural too. The
constructor-visibility argument that motivates the type-split is
real but secondary to the principle.

**Current state, post-reconnaissance:**

- `crates/provider-sync/` already exists, holds `SyncProviderCtx`,
  the `ProviderSyncOps` trait, and four orphan-impls
  (`{gmail,jmap,graph,imap}_impl.rs`). It depends on each provider
  crate today (the orphan impls call `jmap::sync::jmap_initial_sync`
  etc.), so the dep direction is already correct.
- Five crates still name `service_state::*` types in function
  signatures, forcing them to keep the Cargo dep:
  - `sync` - `persistence.rs` (helpers like `store_message_bodies`).
  - `jmap` - `sync/{mod,storage}.rs`, `shared_mailbox_sync.rs`.
  - `gmail` - `sync/{mod,storage}.rs`.
  - `graph` - `sync/{mod,storage,stores}.rs`,
    `shared_mailbox_sync.rs`.
  - `imap` - `imap_initial.rs`, `imap_delta.rs`,
    `imap_delta_janitor.rs`, `sync_pipeline.rs`.
- The non-sync code in each provider crate (clients, ops, parsers,
  converters) does not appear to call into its own sync subdir, so
  the moves should not create reverse-dep cycles. Reconnaissance
  was interrupted before this was fully verified.

**Move shape:**

- `crates/{gmail,jmap,graph,imap}/src/sync/` -> `crates/provider-sync/src/{gmail,jmap,graph,imap}/`.
- `crates/{jmap,graph}/src/shared_mailbox_sync.rs` -> matching
  subdir under `provider-sync`.
- `crates/imap/src/{imap_initial,imap_delta,imap_delta_janitor,sync_pipeline}.rs` ->
  `crates/provider-sync/src/imap/` (folded into the imap subdir).
- `crates/sync/src/persistence.rs` (or its writer-using bits) ->
  `crates/provider-sync/src/persistence.rs`.
- Drop `service-state` from the five Cargo.toml files. Add it to
  `provider-sync` (already there).
- Add a strict-transitive lockdown test alongside
  `app_crate_must_not_transitively_depend_on_cal` in
  `crates/service-state/tests/lockdown.rs`: same Cargo-graph BFS,
  target `service-state`, blessed set `{service}`. Strike the
  "open architectural exception" note in `docs/architecture.md`
  "Current Exceptions" and the matching note in `lockdown.rs`
  module doc when the test passes.

**Risk shape:**

- Bulk file moves + import rewrites. Best landed as a single
  mostly-mechanical commit so a regression bisect lands on it.
- `imap` is the messiest because the sync files live at the crate
  root rather than in a `sync/` subdir.
- Verify no cycle is created by `provider-sync -> {provider} -> provider-sync`
  before each move.

**Status:** plan recorded; implementation deferred. The "Current
Exception" entry in `docs/architecture.md` stands as the visible
tracker until the relocation lands.

### 1. Phase 6d-B / 6d-C strict transitive `service-state` lockdown - orphaned

`docs/service/phase-6d-plan.md:13` (when it existed) explicitly deferred the
four `app -> ... -> {gmail,jmap,graph,imap} -> service-state` Cargo edges
and the `app -> ... -> sync -> service-state` edge to Phase 8 alongside
"the rest of the structural lockdown work." Phase 8 plan-doc never listed
this in scope - neither implementation nor descope decision was recorded.
Verified open today:

```
crates/sync/Cargo.toml:24:    service-state = { path = "../service-state" }
crates/jmap/Cargo.toml:29:   service-state = { path = "../service-state" }
crates/gmail/Cargo.toml:27:  service-state = { path = "../service-state" }
crates/graph/Cargo.toml:31:  service-state = { path = "../service-state" }
crates/imap/Cargo.toml:29:   service-state = { path = "../service-state" }
```

Only `common`'s edge was closed (`crates/common/Cargo.toml:29: # Phase 6d-B:
service-state dep removed`).

The close-out preserved the "future goal" framing - `docs/architecture.md:47`
("Phase 6d-C extends to the strict `app -> ... -> service-state` blackout
once 6d-B's structural moves close the remaining edges") and
`crates/service-state/tests/lockdown.rs:14` ("the Phase 6d-C goal") both
still reference work that no plan now tracks. The `docs/service/` deletion
was the last index where this was visible.

This is most likely an intentional descope (the 6d plan itself called it
"its own multi-phase project"), but the descope decision was never
recorded. Either fold a one-line note into `docs/architecture.md`
"Current Exceptions" naming the open edges, or strike the "Phase 6d-C"
forward references in code+docs.

### 2. Compose-send network coverage - flagged then silently dropped

The restored `docs/service/phase-8-plan.md:520` (current completion ledger)
said:

> Compose-send network coverage is still not in tree. If this remains a
> Phase 8 close-out gate, land a focused SMTP send smoke or record the
> explicit deferral.

Neither happened. The harness scripts (`compose_send_50mb_attachment`,
`send_wire_attachment_validation`,
`send_wire_oversize_payload_handler_path`) are still in M4's "remaining
scope" list (`docs/harness/roadmap.md:392`). The action-pipeline code
itself is in tree, so this is a test-coverage gap, not an implementation
gap - but the plan asked for an explicit decision and the decision was
elided.

---

## Stale references and minor doc drift

> **Resolution (same session):** the three doc-drift items below were
> rewritten in place. The two open hardening drills further down are
> tracked in the harness roadmap and not closed here.

- `crates/service/src/boot.rs:38` - `TODO(phase-2): the action service
  handler reads encryption_key and db_conn from this struct ...
  #[allow(dead_code)] markers come off then.` Phase 2 (and 6a, 6d-A)
  shipped long ago; this comment is from before any of that. Either the
  markers should come off and the TODO with them, or the comment should
  be rewritten. **Resolved:** comment rewritten to describe the actual
  consumer pattern (accessors on `BootSharedState`).
- `crates/app/src/subscription.rs:135` - `TODO(phase-9): when tray-resident
  mode lands ...`. Phase 9 was descoped. Comment should be either deleted
  or rewritten as "if tray-resident ever ships, ...". **Resolved:**
  rewritten to record the descope and frame the relocation as conditional.
- `docs/harness/roadmap.md:408` - M5 marked BLOCKED on M3, but M3's
  initial slice has LANDED (`crates/app/src/harness/` +
  `--test-fake-schema=N` + `test.seed_account` etc.). Status line is
  stale - M5 is now READY, not BLOCKED. **Resolved:** flipped to READY.
- `docs/harness/roadmap.md:217` - M2's third exit criterion ("forced
  writer-task drain bug produces a self-contained artefact dump") is
  still flagged "Not yet manually revalidated after the ratatoskr M1/M2
  landing." Open hardening drill.
- `docs/harness/roadmap.md:228` - M2.5 marked PARTIAL; the soak +
  forced-hang artefact validation are still open. Phase 8 close-out
  gated only on M2 (line 628 confirms this), so M2.5 PARTIAL is fine -
  but it's a real outstanding item for the broader harness arc.

---

## What this audit did not find

- No "exists but not wired" cases of the kind `CLAUDE.md` warns about.
  Every Phase 8 type / function / notification spot-checked is reachable
  from the dispatch path, UI, or boot path.
- No regressions in `docs/architecture.md` content; the promoted
  Service-architecture material is coherent and consistent with current
  code.
- No suspicious deletions in the close-out commit beyond the docs
  themselves (the code-side changes in `41c18c87` are warning fixes +
  minor tidies, per its commit message and stat).

---

## Recommendation

Phase 8 was not prematurely closed. The implementation work is real and
verifiable. The close-out has two real omissions:

1. The Phase 6d-B/6d-C structural lockdown carry-forward needs an
   explicit descope note (or a re-scope into a future entry). Right now
   it is invisible.
2. The compose-send network harness gate needs the explicit
   "deferred to post-Phase-8 / blocked on saehrimnir" annotation that the
   plan asked for.

The stale TODOs and the M5 BLOCKED status are minor doc drift - easy to
fix in passing.
