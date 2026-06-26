# The spec-loop orchestration workflow

The standing procedure for working a TODO document down to landed commits. A
future orchestrator session reads this document at invocation and runs it
identically. The artifacts carry the intelligence; the prompts are minimal by
design - there is no hidden instruction shaping any stage's output, so every
stage is auditable after the fact.

## Roles

- **Orchestrator: Claude Opus, the main session.** Owns the
  budget ledger, the spec documents, the git history, and all validation.
  Never delegates the commit.
- **Claude Opus agents (Agent tool)** author and repair: they write the spec,
  co-review it, and review-and-fix the implementation.
- **Codex xhigh** (`scripts/codex-review.py`) is the deepest reasoner in the
  system. That depth is spent where an error is cheapest to catch: critiquing
  the spec document, before any code exists.
- **Codex medium** (`scripts/codex-implement.py`) implements. This is not a
  cost compromise - it is the falsifiability test of
  `reference/technical-implementation-spec.md` itself. The contract promises
  "two implementers working from it independently produce the same artifact"
  and "no step is left to discover during implementation." If the spec is real,
  a medium-effort implementer just lays bricks. Needing a brilliant implementer
  means the spec failed, not the implementer.

### Codex invocation

Two scripts wrap the canonical `codex exec` call, one per role. Launch in the
background, one Bash call, nothing before or after it:

- Critique: `python3 scripts/codex-review.py '<prompt>'` (gpt-5.5 at xhigh).
- Implement: `python3 scripts/codex-implement.py '<prompt>'` (gpt-5.5 at
  medium; the script adds the `/goal` prefix).

The single argument is the prompt: one line, no linebreaks, plain ascii, no
escapes or quoting tricks, single-quoted; substitute X and Y with plain paths.
The script owns the model, the reasoning effort, the workspace-write sandbox,
and `/goal`. The caller owns only the prompt.

The sandbox is also network-isolated: no outbound connections, no git fetch,
no cargo download. The load-bearing consequence is that codex cannot add a
crate that is not already present in Cargo.lock and the local cargo registry
cache - cargo cannot fetch it offline, so the build fails. The same applies
to any other network operation (remote fetch, git push, external API call).
If a spec brick requires a brand-new external dependency, the orchestrator
must add it - via a Claude side-step (Opus or Sonnet in the main session) -
BEFORE the step-4 codex implement run. Alternatively, write the spec so it
builds on crates already in the lockfile. A codex run that hits a missing
crate will fail at compile time with no path to recovery inside that run;
the mitigation belongs in the spec review (steps 2-3) or a pre-step-4
side-step, not inside the codex run itself.

The script keeps the raw NDJSON inside its own process and prints a clean
digest at exit: the final agent message in full, the token usage, and any
plain-text log lines codex emitted (surfaced, not dropped). The final message
is captured via `--output-last-message`, so it survives even when a mid-run
codex error halts the stream. There is nothing to peek at and nothing to flood
context: a run is opaque until it exits, and its exit IS the signal.

We never resume a codex thread. A run that ends with its goal unmet - a gate
honestly reported unpassed, bricks left unbuilt, victory wrongly declared - is
replaced by a FRESH `codex-implement.py` run pointed at what remains, never
`codex exec resume`. Fresh-from-spec is the methodology: the spec is the only
communication channel, so a second implementer reads it exactly as the first.

The `/goal` prefix (added by codex-implement.py) is mechanical, not
motivational: whenever the agent yields - a status report, a question, a
premature wrap-up - the harness auto-replies "that is not what the user said,
continue work" and the agent resumes. It structurally cannot hand back control
until it declares the goal achieved. Consequences the orchestrator must hold:

- A question asked mid-run is answered by nobody. Every ambiguity in Y is
  resolved by the implementer alone, from the spec. An underspecified spec
  does not draw a clarifying question; it draws hours of confidently building
  the wrong thing. This is why steps 2-3 exist.
- The run ends when the agent declares the goal done, and not before. Long is
  normal, hours are normal, "still going" is the expected state and never by
  itself evidence of a problem. The orchestrator NEVER kills a run - not for
  slowness, not for apparent thrash, not for budget. (Orchestrators reliably
  talk themselves into "this one is clearly stuck"; that judgment is not
  available to you.) A run six hours in is indistinguishable from one about to
  finish, so never judge a stall by elapsed time.
- A `/goal` run burns whatever budget it takes, without asking. That is the
  deal made at launch.

## Input

A goal, owned by the user. Before launching anything - before step 1 of the
first item - state the goal back to the user in one or two sentences and get
explicit confirmation that this is the goal. Do not infer it: the user hands
over TODO files, design docs, and specs as supporting context, and none of
them is the goal by default - a document the user called "the end goal" is;
a TODO file given "because it has relevant information in it" is not.
Misalignment here is the most expensive mistake available to the
orchestrator: every step after it builds the wrong thing with full
discipline.

From the confirmed goal the orchestrator derives the work items - what the
goal still lacks - processed serially, one commit each, tree green at every
boundary between them. The loop ends when the goal is met; report it met and
stop deriving. Adjacent work the goal does not require is never started
uninvited.

Nothing defers. An item either lands with its gates met, or fails a gate and
is reverted - there is no third state where work parks half-done for later.
Every review finding is fixed (step 5), every stale document reconciled
(step 6). A defect discovered while landing an item is not chased inside
that item: it becomes a TODO entry immediately, evidence captured while
fresh, ordered ahead of any item that depends on it, and enters the loop as
its own item. The single exception to "nothing parks": an artifact produced
by AIMING error - a step 1 launched against the wrong item - is parked in
the tree with a pointer at its TODO item rather than thrown away; that item
later resumes from step 2. Produced work is never discarded.

## Per item

Run the seven steps in order. Never skip a step, never reorder, never merge two
steps into one launch. Use the prompts below verbatim - substitute X and Y,
change nothing else, add nothing. Do not enrich a prompt with context from
the conversation, the TODO document, or a previous step's output; the
documents carry all context by design. One step's artifact must exist and be
complete before the next step launches. If a step fails, rerun that step -
do not improvise a recovery that bypasses it.

Each file-touching prompt below carries `Do not commit.` That guard is
load-bearing, not boilerplate: the orchestrator owns the commit - step 7,
exclusively - so an agent that commits its own work lands it before
review-and-fix (step 5) has run and outside the orchestrator's git control. The
failure mode is model-shaped - a codex agent does not commit unless asked, but a
Claude agent will commit, push, and touch git at the first opportunity - and
step 4 (like 1, 3, 5 and 6) may be run by either, so every file-touching prompt
carries the guard to stay model-agnostic. Keep it verbatim. If an agent commits
anyway it has skipped steps 5 through 7: soft-reset the tree back to uncommitted
with a mixed `git reset HEAD~1` before the loop resumes, then continue from
step 5.

Every prompt to a Claude Agent (any type) also ends with `Do not launch any
sub-agents.` This guard is load-bearing too: the loop runs exactly one Agent at
a time, and the anti-nesting flush (see the harness-bug section) depends on it.
An Agent that spawns its own sub-agents puts a second Agent in flight, which
reintroduces the lingering-window nesting the flush exists to prevent. Codex
prompts omit this guard (codex is a separate process, not a Claude Agent);
step 2's prompt is the one Claude-Agent prompt also handed to codex, which
harmlessly ignores it.

### 1. Spec

Launch one Agent(opus), background:

> Read X and reference/technical-implementation-spec.md and write a new
> implementation spec document. Do not commit. Do not launch any sub-agents.

where X is a reference to where the TODO item lives, not the item's text -
e.g. "item N in TODO.md", in whatever form the TODO document named at
invocation makes addressable. The agent reads the item at the source. Nothing
more.

### 2. Critique - two reviewers, both, every time

Launch simultaneously, both background:

- Agent(opus)
- codex `gpt-5.5` at `xhigh`

Both get the same prompt:

> Please critically review Y and report back your findings. Do not launch any
> sub-agents.

where Y is the spec document from step 1. The Opus reviewer shares the
author's priors and catches Claude-shaped gaps; the xhigh codex reviewer
brings deeper reasoning and foreign priors and catches what both would miss.
Neither substitutes for the other.

### 3. Consolidate

Hard barrier: wait until **both** reviews are in. The orchestrator writes
each report verbatim to a file beside the spec document (R1, R2), then
launches one Agent(opus), background:

> Read Y, and the two review reports at R1 and R2. Validate each finding,
> consolidate the two reports, and fold every valid finding regardless of
> severity into Y. Note the findings you rejected and why. Do not commit. Do
> not launch any sub-agents.

The orchestrator does not do this itself: validating findings means reading
code, and code readings must not accumulate in the orchestrator's context -
it has to survive the whole loop. The orchestrator deletes R1 and R2 once
the consolidated Y is in.

### 4. Implement

Launch `scripts/codex-implement.py`, background, with the prompt (the script
adds `/goal`):

> Please implement Y from beginning to end. If you hit a gate, please try
> honestly to overcome it. Do not commit.

When the run ends, read the digest - do not take "done" on faith. If
`final_message_captured: false`, the run ended without a final report
(crashed, killed, or yielded out); if it is `true` but the message says the
goal is unmet (a gate honestly reported unpassed, bricks left unbuilt, victory
wrongly declared), it is likewise not done. In either case launch a FRESH
`codex-implement.py` run whose prompt names what remains. Never resume. Repeat
until the implementation is whole, then go to step 5.

### 5. Review and fix

Launch one Agent(opus), background:

> Read Y. The uncommitted changes in this tree are an implementation of it.
> Critically review the implementation against the spec and fix what you
> find - bugs, gaps, smells, and nits alike, not just the serious ones. Where
> the implementation deviates from the spec deliberately and the deviation is
> sound, keep it and note it. Report every finding with its severity and the
> fix you applied. Do not commit. Do not launch any sub-agents.

Design notes on this prompt:

- The contract is the spec, not taste. A fix is "the implementation deviates
  from Y," never "I would have designed Y differently." Step 5 does not
  relitigate the design steps 1-3 settled.
- But the spec can be wrong too. The implementer may have hit a real obstacle
  the spec missed and honestly overcome it by deviating. A sound deliberate
  deviation is kept and reported, not reverted to match a broken spec.
- Fix means fix, not rewrite. Surgical repair of findings; wholesale rework
  would throw away the medium-implementer test result.
- All severities are fixed, not just reported. There is no human between step
  5 and the commit, so an unfixed nit is not deferred to review - it is landed
  debt. Severity labels survive purely as reporting vocabulary for the
  orchestrator's triage read; they never gate action. The only thing that
  escapes fixing is the sound deliberate deviation.
- Reviewers defer anyway ("left as-is", "latent smell", "scope creep",
  "fold in later"). The orchestrator's triage read of the step-5 report
  exists to catch exactly this: any finding reported but not fixed, other
  than a sound deliberate deviation, gets a follow-up Agent(opus,
  background) launched with the deferred findings quoted verbatim and the
  instruction to fix them. Repeat until a step-5 report defers nothing.
  A reviewer's "low severity, no consumer relies on it" is a severity
  label, and severity labels never gate action.

### 6. Update relevant documents

Launch one Agent(opus), background:

> Read Y. The uncommitted changes in this tree are its landed implementation.
> Update every document the landing makes stale: close or remove the
> originating TODO item at its source per repo convention (completed items
> are removed entirely, never marked done), and reconcile any other document
> that still describes the pre-landing state. Do not touch the spec document
> itself, and never cite it from any document - it is deleted at landing, so
> point durable references at git history instead. Report each
> document you changed and why. Do not commit. Do not launch any sub-agents.

The reviewers and the implementer fix documents only when they happen to
notice them; this step is the guarantee. A landed item whose TODO entry
still reads as open re-enters the loop as a ghost.

### 7. Land

The orchestrator, in the main session:

1. `brokkr fmt` (run before every commit per repo policy - the loop owns it so
   a codex implementer's stray formatting never surfaces as a surprise diff at
   commit time)
2. `brokkr check`
3. Every harness gate the spec named in its verification section, run with the
   EXACT command the spec gave: `brokkr service-test <script>` /
   `brokkr service-suite [--filter X]` for Service IO-boundary bricks, and
   `brokkr sync-bench <script> --gate <name>` held against its recorded
   baseline for sync/provider/hot-path bricks. A spec that named no harness
   gate runs only steps 1-2. These gates are routine here, not exceptional:
   ratatoskr is where the integration, mock-server, and performance coverage
   lives, so most landings hold at least one.
4. Delete the spec document (the orchestrator manages spec-doc lifecycle;
   durable findings settle into TODO.md and git history per repo convention)
5. Commit (per the repo git commit rules - `Cargo.lock` if changed, on the main
   branch, no push)

No further validation - fmt, check, and the spec's named gates, then commit.

Then loop from step 1 on the next item.

## Waiting discipline

These are rules, not guidance:

- Launch every Agent call and every codex call in the background. No
  exceptions.
- Flush before every Agent launch that follows an Agent's rest. The moment any
  Agent comes to rest, fire scripts/prevent-harness-bug.sh as a background
  task, wait for its exit, and only then launch the next Agent. This is the
  load-bearing fix for the sub-agent nesting bug (see the section below); it
  has no exemptions and applies between items as well as between steps. Codex
  launches need no flush (separate OS process, cannot nest).
- Never set a timeout on anything. Never kill a run for being slow.
- Never block waiting on a launched task. Launch, then schedule the wakeup.
- While anything is in flight, keep the heartbeat firing: call ScheduleWakeup
  for 270 seconds out, every turn, until the task returns. The heartbeat is NOT
  polling for completion - do not reason about it as "checking on the task." A
  harness-tracked Agent auto-notifies you the instant it finishes, and a codex
  run re-invokes you when its process exits, so completion always arrives on its
  own; you never need to wake to look for it. The heartbeat exists for ONE
  reason: to keep the prompt cache warm. The Anthropic prompt cache has a
  5-minute TTL, so a wake every 270 seconds is a touch inside that window that
  keeps this (very long) conversation cached. Skip a beat and the cache lapses;
  the next turn then pays a full-context cold re-read - real money. So do NOT
  "optimize" the heartbeat into a single long fallback (1800s, "the harness will
  notify me anyway") - that reasoning is the trap: the notification is not the
  point, cache warmth is, and a long gap is exactly what drops it. The 270s
  cadence is identical whether you wait on codex or a harness-tracked Agent; the
  auto-notification changes nothing about the cache and so changes nothing about
  the cadence.
- ScheduleWakeup's prompt is plain continuation text and must NEVER begin with
  a slash. The harness reads a leading slash as a slash-command or skill
  invocation; if that command is absent or disabled the wakeup silently fails
  to fire AND never wakes you - the worst failure mode, because the loop looks
  alive while it is dead, with no signal to you or the user. Plain text
  ("continue: spec C, step 4 in flight") wakes you cleanly; the heartbeat does
  not need to re-enter any skill to do its job.
- On each wake: check whether the task returned. "Returned" means exactly one
  thing - the background process exited (for codex, the script then printed its
  digest). A codex run is opaque until then: there is no live stream to read
  and no mid-run completion claim to misjudge. Process running = step running.
- While the task has not returned, schedule the next wakeup and nothing else.
  Never kill a run (see Codex invocation).
- On every wake, write something visible in the session - one short line is
  enough ("step 4 still running, 12 min in"). Never wake silently: the user
  reads the session to see the loop is alive, and a silent wake is
  indistinguishable from a dead one.
- The orchestrator runs the loop to the end on its own. How long a step takes
  is not the orchestrator's problem to solve. If you believe something is
  genuinely wrong - stuck, thrashing, anything - you do not act on that
  belief: write what you see in the session, bring the user in, and let the
  user decide. Escalation is a report, never an action.
- Escalating never pauses the loop. The user may not be there to read the
  report for hours; you keep waking every 270 seconds, keep reporting, keep
  advancing whatever can advance. Never idle waiting for the user to answer
  an escalation - the loop stops only when the work is done or the user says
  stop.
- An item can block on something only the user can produce (TV fieldwork, a
  credential, a ruling). That wait begins ONLY at a clean checkpoint: bring
  the item to a green boundary first - land what is landable as a committed,
  gate-passing unit; revert what is not - so the tree is clean before the
  loop goes quiet. The user may be back in eighteen hours; idling a dirty
  tree across that gap turns the resume into a full-context cold re-read
  costing hundreds of thousands of tokens for nothing. Once clean: surface
  the exact ask (what to produce, how, what arrives where), drop from 270s
  beats to long idle beats, and resume the moment the dependency lands.

## Claude harness bug: sub-agent nesting at a step hand-off

Recent Claude harness versions leave a completed background Agent (the Agent
tool) lingering ~20-30 seconds before it fully tears down. If the orchestrator
launches the next Agent during that window, the new Agent is mis-parented
INSIDE the lingering one: it runs to completion there, the lingering Agent's
lifetime swallows the inner Agent's entire runtime, and the inner work bubbles
back as a confusing SECOND return on the WRONG (outer) Agent's id - while the
inner Agent's own clean return never arrives. Left unhandled it compounds:
every later launch nests deeper inside the same outer Agent, which can never
tear down because it reacquires a live child before its window closes, so one
Agent becomes the immortal container for the entire item.

Codex runs are separate OS processes and are immune - they neither linger nor
nest. That is why the bug fired LESS with codex in the loop: an interleaved
codex step is itself a gap that lets the prior Agent tear down. An all-Opus
loop has an Agent at every hand-off, so without the flush it fires every time.

The fix is PREVENTION, not recovery: never launch into the lingering window.
The moment any Agent comes to rest, fire scripts/prevent-harness-bug.sh as a
background task, wait for it to exit, and only then launch the next Agent. The
script is a 60-second wall-clock sleep - well clear of the measured 20-30s
window - and its exit reliably re-invokes the orchestrator. Teardown is
wall-clock, so idling through the flush is enough; no activity is required of
the orchestrator during it.

The rule has NO exemptions. It applies between steps within an item AND between
items: the last step of item N to step 1 of item N+1 is a hand-off like any
other, and "the start of the next item" is not a fresh start. A flush before a
codex launch is unnecessary (codex cannot nest) but harmless; the load-bearing
requirement is a flush before every Agent launch that follows an Agent's rest.

This was characterised empirically: 0s, 10s and 20s flushes all still nested;
30s cleared it on repeated runs; 60s is the production value, chosen with
margin. The window does not scale with agent size, so 60s holds for the
heaviest Claude Agent in the loop (step 5's review-and-fix) as well as the
lightest. (Step 4, implement, is codex and immune - it never enters this.)

If the confusing-second-return signature ever appears DESPITE the flush, treat
it as a flush that was missed, not as a result to accept: quiesce to a clean
state (no Agent left idling), then resume with the flush in place. Never fold a
nested second return into the work, and never reconstruct a contract document
(a spec) from one - that launders corruption into the loop.

## Telemetry

`codex-review.py` and `codex-implement.py` print a digest at exit: the final
agent message in full, the summed token usage, and any plain-text log lines
codex emitted. Read the final message in full when a run ends - never a
truncated excerpt. The closing report is where deferred work, honest gate
failures, and wrongly-declared victories surface; a capped read is how they get
missed. Each codex call's usage folds into the per-item cost ledger alongside
the Agent calls, so each landed item gets a true cost figure.
