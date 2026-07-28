# Side-quests: changing a dependency repo

Ratatoskr depends on two sibling repositories that we own and can change:

- **bifrost** - the provider stack. All mail, calendar, and contact protocol
  I/O, the unified `Account` surface, `AccountError` / `RecoveryClass`, and the
  sync engine. A Cargo path dependency, compiled from source.
- **saehrimnir** - the mock provider server the sync harness runs against. An
  installed binary, not a Cargo dependency.

When work here surfaces a need to change one of them - a missing capability, an
awkward surface, a wart that would otherwise force a workaround on our side -
that is a **side-quest**. This document is the procedure.

## The governing rule

The dependencies exist to serve ratatoskr. Where a dependency's shape is
sub-optimal for us, the dependency is fixed FIRST, in its own repo, before the
ratatoskr work that needs it.

Ratatoskr is never contorted around a dependency wart. Provider-reality
differences (Gmail has no separate attachment-upload endpoint, IMAP has no
native send) are absorbed by bifrost behind the uniform `Account` surface, or
expressed as `AccountCapabilities` flags we read declaratively. They never leak
in here as per-provider special-cases. A genuinely immutable provider limit
becomes a capability flag the UI consults, not a code branch.

Corollary worth stating because it comes up often: if a capability looks
mysteriously absent from bifrost, that is usually not a design gap to engineer
around. Bifrost was started by ripping ratatoskr's own provider code out and
unifying it, so an absence generally means that part was not carried over yet.
Git history is the reference for what it should already do.

## Where the repos live

Each dependency exists in two places, serving two distinct purposes. Do not
conflate them.

| Path | Role |
| --- | --- |
| `./research/bifrost`, `./research/saehrimnir` | In-tree working copies. The reading reference AND the staging area where side-quest edits are made. |
| `../bifrost`, `../sæhrimnir` | The live dependency. What Cargo `path = "..."` resolves to, and what `cargo install` builds the mock from. |

Keeping a copy in-tree is what lets agents read and edit dependency source
without tripping up the harness. `./research/bifrost/reference/` holds per-crate
and per-protocol quick-reference sheets (`net.md`, `sync.md`, `error-model.md`,
`jmap.md`, `imap.md`, `graph.md`, `google.md`, `smtp.md`, `caldav.md`,
`carddav.md`, `sasl.md`) - start there for a crate's surface, then drop into the
source.

## Reading is expected; writing is the prohibition

Side-quest work is never delegated to an implement run. Codex and other
implement agents must not EDIT `../bifrost`, `../sæhrimnir`, or the
`./research/` copies - even when the spec names the side-quest surface as a
required prerequisite - and every implement prompt that runs while a side-quest
is pending states this prohibition explicitly.

**The prohibition is on WRITES, and the distinction is load-bearing.** Reading
those trees is expected and often necessary: an implementer resolving a promoted
type signature, a field's visibility, or an accessor name must be able to
consult them. Phrase the guard as "must not edit", never as "must not read or
touch".

Both halves were learned the hard way. First an implement run edited the frozen
`../bifrost` directly, which is what prompted the rule. Then the next run
stalled mid-brick, unable to resolve a `BatchOutcome` accessor on a
freshly-promoted surface, because the guard had been over-tightened to forbid
reading as well. Those are opposite errors around the same boundary.

## The procedure

### 1. One agent, confined

Launch ONE Opus agent (the Agent tool, never codex) to do the work. Its prompt
must state, in unambiguous terms:

- It works EXCLUSIVELY inside `./research/<repo>`. It must not read, edit, or
  otherwise touch any part of ratatoskr proper, under any circumstance. If it
  finds itself blocked on something that would require a ratatoskr change, it
  STOPS WORK immediately and reports back - it never improvises a ratatoskr
  edit.
- It must `cd` into the relevant `./research/<repo>` folder before doing
  anything, and stay there. (A guardrail, not a wall - the fence is the
  instruction above, so state it plainly.)
- It must NOT commit. Committing is the orchestrator's job.
- It is told NOTHING about the bridge scripts and must never run them.
  Promotion is the orchestrator's job.
- It must not launch any sub-agents.
- It is doing a DIRECT implementation task, not an orchestration. Each
  `./research/<repo>` CLAUDE.md leads with a spec-loop cue ("when asked to
  orchestrate, read reference/orchestrate.md FIRST"); that cue is not for this
  agent. It must not orchestrate, must not read that repo's orchestrate.md, and
  must ignore the spec-loop machinery - it does the work itself, in place.

### 2. Review, validate, commit, promote - in that order

The orchestrator does all four, without pausing for the user:

1. **Review** the work in `./research/<repo>`.
2. **Validate it IN PLACE**: `cd` into `./research/<repo>` and run
   `brokkr check` (plus any focused `brokkr test`). This is the gate, and it
   runs BEFORE the commit.
3. **Commit it there.** The commit is the orchestrator's job, never the
   agent's.
4. **Promote** by running the bridge script from the main session:
   `bash scripts/bifrost.sh` or `bash scripts/saehrimnir.sh`. Each pushes the
   staged `./research/<repo>` commit to its shared remote and pulls it into the
   dependency path; `saehrimnir.sh` also reinstalls the mock binary.

The bridge scripts round-trip through GitHub, so they are **orchestrator-only**
and can never run inside a codex step - that sandbox is network-isolated. The
push and the reinstall are routine steps needing no separate approval.

Bridge-script assumptions to hold: the work is already committed in
`./research/<repo>`, and that clone is on a branch tracking origin (not a
detached HEAD), so the scripts' bare `git push` succeeds.

## A side-quest is not done at promotion

**It is done when the consumer's own harness gates pass against the promoted
surface.** This is the single most expensive lesson in the migration and it has
a specific cause: bifrost's own rules forbid integration and mock-server tests,
so a bifrost change can be unit-green and wire-broken. One item hit that four
separate times - a LIST form a conforming server answers with the wrong
namespace, a hydration door that routed in batch but not singly (twice, once per
protocol crate), and a discovery seed gated on an optional chain.

Two mitigations, both load-bearing:

- Extract each wiring decision into a pure, unit-pinnable function rather than
  leaving it inline.
- Prefer several narrow work items to one broad one where a brick spans more
  than one dependency repo.

The same failure shape recurs on the mock side, where it is even easier to miss:
a mock can serve a surface in the wrong SHAPE and every test still passes,
because the tests exercise the shape the mock implements rather than the one the
client drives. Assert the consumer's actual request form, and assert response
STATUS on batched sub-requests.

## Two validation layers

A sync-touching side-quest wants both.

- **In place**, before promotion (above): gates the change inside the research
  copy.
- **Post-promotion, ratatoskr-side** from the repo root: for bifrost - a path
  dep compiled from source - the authoritative gate is ratatoskr's own
  `brokkr check` after `bifrost.sh`. For saehrimnir - an installed binary -
  `saehrimnir.sh`'s `cargo install` compile-gates it, and the behavioral gate is
  a ratatoskr sync-harness run against the reinstalled mock.

## Freeze discipline

Whatever commit the bridge reports becomes the frozen reference for the work
item, and the dependency stays pinned there for the item's full duration -
including the hours a single implement run can take.

This is load-bearing: bifrost compiles from source, so a `../bifrost` that is
red OR merely mutating underneath an in-flight step makes every ratatoskr gate
meaningless, and a later change would silently shift the surface the work was
built against.

## Working in the research copies

The repo's bash rules (no `git -C`, one command per invocation, no chaining) are
written for ratatoskr proper. `./research/bifrost` and `./research/saehrimnir`
are separate repos the orchestrator legitimately manages, so the orchestrator is
exempt there for the review / validate / commit / discard it owns:

- **Git**: run `git -C <abs-path>/research/<repo> ...` directly. The Bash
  working directory also persists between calls, so a bare
  `cd ./research/<repo>` followed by a separate `git` / `brokkr` command works
  equally well - just `cd` back to the ratatoskr root afterward, since later
  steps assume it.
- **Validate in place**: this works because each research repo is its own
  standalone Cargo workspace root that brokkr resolves instead of walking up
  into ratatoskr's. Bifrost already is a workspace; saehrimnir needed a bare
  `[workspace]` table in its `Cargo.toml` so cargo would not adopt the parent
  manifest.
- **The `CARGO_MANIFEST_DIR` gotcha**: brokkr builds a nested research workspace
  with `CARGO_MANIFEST_DIR` anchored under brokkr's OWN install path, not the
  real source location. Any test resolving a committed fixture via
  `env!("CARGO_MANIFEST_DIR")` therefore reads a path that does not exist and
  fails. Such tests must resolve fixtures against the runtime working directory
  (`std::env::current_dir()`), which is the crate root under both `cargo test`
  and brokkr.
