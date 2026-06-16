# Technical implementation specification

The single document from which an open TODO item is built to completion without
re-deriving its design. Two implementers working from it independently produce
the same artifact.

## What it is

1. **Every brick.** It lays each step on the road from the current code to the
   finished item. No step is left to discover during implementation.
2. **Obstacles resolved inline.** Anything blocking the road is solved in the
   document, as part of it. An unresolved obstacle is a missing brick.
3. **No deferral.** Nothing in the originating TODO is pushed to "later" -
   deferred work is a hole in the road. (Work that belongs to a genuinely
   separate TODO is named and excluded; that is not deferral.)
4. **No shoehorning.** We do not fit the work into existing abstractions,
   structures, or conventions because they already exist. The structure that
   best serves the end goal is the one we build; whatever stands in its way is
   ripped out and rebuilt. Pre-1.0, breaking any internal API is legal.

## What it must also pin (or it is aspiration, not a spec)

5. **Verification per brick.** Every change names its gate, matched to what the
   change can break, drawn from ratatoskr's full gate surface:
   - a named `brokkr test -p <crate> <NAME>` case for any behavior a
     deterministic unit or in-process integration test can pin (parser,
     encoder, type-level check, serde round-trip, error classification, DB
     query, action-pipeline resolution);
   - a named `brokkr service-test <script>` (or `brokkr service-suite
     [--filter X]`) Lua harness script for Service IO-boundary behavior - boot,
     dispatch, drain, crash, framing - that only a real subprocess exercises;
   - a named `brokkr sync-bench <script> --gate <name>` run, held against its
     recorded `brokkr.toml` baseline, for provider-sync behavior and for any
     hot path carrying a performance, provider-request-count, or memory budget.
     Ratatoskr DOES measure performance: these baselines (elapsed, provider
     requests, peak RSS) are part of the contract and gate landings;
   - and `brokkr check` (gremlins + clippy + the changed-files test sweep) as
     the universal green-tree gate every landing must hold.

   These integration, harness, and perf gates are the norm here, not the
   exception: most bricks that touch sync, providers, the Service boundary, or
   a hot path are gated by a harness script or a sync-bench baseline, not by a
   unit test alone. If a behavior genuinely cannot be pinned by any of these,
   the spec says so explicitly and names the `brokkr check` outcome that stands
   in. A brick whose load is unproven is not laid. Per gate, the spec contains
   the EXACT command to run - copy-pasteable, flags and all, not "run the
   relevant tests". If no instrument exists that can verify a gate (no path
   exercises it, no test or harness script pins the behavior), building that
   instrument - the smallest deterministic unit test, the Lua harness script,
   or the sync-bench gate that pins the behavior - is itself a brick of the
   spec, specified to the same standard and laid before the brick it gates.
6. **A keep/revert path.** The implementation unit is one coherent, fully
   intrusive change that lands and is then kept or reverted on its gate
   results - never a tiny gated probe or an env-var experiment switch. The
   sequence of such landings is ordered so `brokkr check` stays green at every
   boundary between them. Complete-but-unorderable is a failed spec.
7. **The target as concrete artifacts.** "The ideal structure" is pinned to
   exact types, signatures, ownership, and data flow - buildable, not merely
   directional.
8. **A survey of the ground.** The current structure and everything depending on
   it is inventoried before the teardown, so the rip is precise and drops no
   load-bearing work. Specs authored as a batch reconcile their surveys against
   siblings covering the same ground before any is implemented; a sibling's
   survey may already state the fact that refutes this spec's premise.
9. **A stopping rule.** The rebuild has a bounded blast radius. Where the
   teardown stops, and what is out of scope, is stated explicitly.
10. **The standing references.** Every spec MUST cite, by path: this document
    (`reference/technical-implementation-spec.md`) as the contract it is
    written against; `reference/architecture.md`, the cross-cutting
    architecture contract - ALWAYS required reading regardless of what the spec
    targets, because crate boundaries, the `MailActionIntent -> resolve_intent
    -> build_execution_plan -> batch_execute` action pipeline, the
    `OperationResult` taxonomy, generation counters, and scope wiring bind any
    structural change; the document the spec was spawned from (the TODO source
    naming the item - e.g. a `TODO.md` entry or a `docs/` plan); AND every
    area-specific required-reading doc from AGENTS.md's required-reading map
    that the spec touches - `reference/glossary/folders-labels.md` for
    folders/labels/`label_kind`/system-folder work, `reference/glossary/harness.md`
    for any harness, `--test-harness`, service-test, or sync-bench work,
    `UI.md` for UI work, and the relevant `docs/<area>/` design doc for a
    feature area. A spec citing these references must direct its reviewers and
    implementers to READ them, not merely name them - they are the ground the
    work is built on and judged against. A spec missing any of these is
    incomplete. Unlike the bifrost dependency, ratatoskr DOES keep performance
    baselines: a spec touching a sync, provider, storage, or Service hot path
    owes the relevant `brokkr sync-bench` gate recorded against its
    `brokkr.toml` baseline (elapsed, provider-request count, peak RSS), so
    correctness AND the named performance budgets are measured axes - gated by
    `brokkr check`, named `brokkr test` / `brokkr service-test` cases, and the
    sync-bench baselines.

## Stance

- **Structural over micro.** The spec pursues the structural change that
  materially moves the goal - real capability for feature work, real
  correctness for fix work - not local tweaks. Full rewrites are labeled
  as such, distinct from local changes.
- **Cleanliness is a deliverable.** No env-var scaffolding, benchmark knobs, or
  temporary routing switches left as the way forward.
- **Unlimited resources, aggressive internal rewrites assumed.** Old
  abstractions earn no protection from age; shared writer abstractions and
  generic reuse are not goals. Correctness and maintainability of the *result*
  still hold.
