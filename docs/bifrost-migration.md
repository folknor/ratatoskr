# Bifrost migration: governing plan

The strategic map for replacing ratatoskr's hand-rolled provider stack with
dependencies on the bifrost workspace. This document is the source-of-record
the spec-loop consumes: every work item below becomes one
technical-implementation-spec, run through the orchestrate.md seven steps. The
loop is running: Track B has begun (B1 has landed; see § 7).

`../bifrost` and `./research/bifrost` are at the same git commit. We keep both
because they serve two distinct purposes: `../bifrost` is the Cargo dependency
path the build resolves, and `./research/bifrost` is the in-tree working copy -
both the reading-reference agents read bifrost source from AND the staging area
where side-quest edits to bifrost are made before the bridge promotes them to
`../bifrost` (see § 2). Keeping it in-tree is what lets agents read and edit
bifrost without tripping up the harness. See § 11 for the full distinction.

## 1. Goal

Rip out ratatoskr's hand-rolled provider stack - the four provider crates
(`gmail`, `jmap`, `graph`, `imap`), the `provider-sync` crate, and the
`ProviderOps` / `ProviderSyncOps` / `ProviderError` / `ProviderState` /
`create_provider` surface in `common` - and replace it with dependencies on the
bifrost workspace, so ratatoskr speaks bifrost's unified `Account` /
`AccountError` / sync language natively.

This is a feature-preserving plumbing replacement. Every capability ratatoskr
ships today survives the migration. The goal is to delete ~33k LOC of
duplicated provider logic and inherit bifrost's unified, tested surface, not to
change what the client does.

**Maximal integration.** This is a total integration, not a provider-only swap.
The four provider crates and `jmap-client` are the largest target, but the rule
is general: wherever bifrost offers an equivalent for something ratatoskr
currently hand-rolls, or for an external dependency ratatoskr currently pulls
separately, that thing is replaced by bifrost. The end state is ratatoskr
depending on bifrost for everything bifrost covers, with no parallel
hand-rolled or duplicated dependency surviving alongside it.

## 2. First principle (this project's governing rule)

Bifrost exists to serve ratatoskr. The plan is written against an IDEAL
bifrost. Wherever bifrost's current shape is sub-optimal for ratatoskr, bifrost
is fixed FIRST, in the bifrost repo, before the corresponding ratatoskr work.

- ratatoskr is never contorted around a bifrost wart.
- Provider-reality differences (Gmail has no separate attachment-upload
  endpoint; IMAP has no native send; etc.) are absorbed by bifrost behind a
  uniform `Account` surface, or expressed as clean `AccountCapabilities` flags
  that ratatoskr reads declaratively. They never leak into ratatoskr as
  per-provider special-cases.
- A genuinely immutable provider limit (e.g. Gmail cannot update filters via
  API) becomes a capability flag the ratatoskr UI consults - not a code branch.

Track A below is the concrete definition of that ideal bifrost. It is a
specification of the target, not a list of bugs.

Bifrost's origin makes this concrete. Bifrost was started by ripping
ratatoskr's existing provider code - what now lives in the provider crates -
out of ratatoskr and unifying it. So if a capability looks mysteriously absent
from bifrost, that is not a design gap to engineer around; it means that part
was simply not carried over yet. The original code still in ratatoskr (the
current provider crates plus git history) is the reference for what bifrost
should already do.

**The side-quest protocol.** When any brick along the way surfaces that
ratatoskr would benefit from a rewrite or refactor of something in bifrost or
saehrimnir - a missing capability, an awkward surface, a wart that would
otherwise force a ratatoskr workaround - that becomes a side-quest, never a
ratatoskr contortion. The orchestrator brings the tree to a clean boundary
(landing what is landable, reverting the blocked in-flight work so nothing
parks dirty), then handles the side-quest itself, in-loop, without pausing for
the user:

1. The orchestrator launches ONE Opus agent (the Agent tool, never codex) to do
   the bifrost or saehrimnir work. The agent's prompt must state, in
   unambiguous terms:
   - It works EXCLUSIVELY inside `./research/bifrost` or
     `./research/saehrimnir`. It must not read, edit, or otherwise touch any
     part of ratatoskr proper, under any circumstance. If it finds itself
     blocked on something that would require a change in ratatoskr, it STOPS
     WORK immediately and reports back - it never improvises a ratatoskr edit.
   - It must `cd` into the relevant `./research/<repo>` folder before doing
     anything, and stay there. (This is a guardrail, not a wall - the fence is
     the instruction above, so state it plainly.)
   - It must NOT commit. Committing in `./research/<repo>` is the
     orchestrator's job.
   - It is told NOTHING about the bridge scripts below and must never run them.
     Promotion to the live dependency is the orchestrator's job.
   - Per the standing rule, it must not launch any sub-agents.
   - It is doing a DIRECT implementation task, not an orchestration. The
     `./research/<repo>` CLAUDE.md leads with the spec-loop ("when asked to
     orchestrate, read reference/orchestrate.md FIRST"); that cue is not for
     this agent. It must not orchestrate, must not read that repo's
     orchestrate.md, and must ignore the spec-loop machinery - it does the
     work itself, in-place.
2. When the agent returns, the orchestrator - not pausing for the user -
   reviews, validates, commits, and promotes, in that order:
   - Review the work in `./research/<repo>` (`git -C ... diff`; see the
     mechanics note below).
   - Validate it IN PLACE: `cd` into `./research/<repo>` and run `brokkr check`
     (plus any focused `brokkr test`). This is the gate on the side-quest and
     runs BEFORE the commit, per the loop's check-before-commit discipline.
   - Commit it there (the commit is the orchestrator's job, never the agent's).
   - Promote it to the live dependency by running the bridge script from the
     main session - `bash scripts/bifrost.sh` or `bash scripts/saehrimnir.sh`. Each pushes
     the staged `./research/<repo>` commit to its shared remote and pulls it
     into the Cargo/install path (`../bifrost` / `../sæhrimnir`);
     `saehrimnir.sh` also reinstalls the mock binary. The scripts round-trip
     through GitHub, so they are orchestrator-only and can never run inside a
     codex step (the codex sandbox is network-isolated). The push and the
     reinstall are routine in-loop steps; like every other step of the loop they
     need no separate user approval.

The `../bifrost` / `../sæhrimnir` HEAD the bridge reports becomes the frozen
reference for the item (`./research/<repo>` and the dependency path now sit at a
single shared commit). Only then does the loop resume, against the updated
surface, with that commit pinned for the item's full duration.

**Orchestrator mechanics in the research working copies.** The bash rules (no
`git -C`, one command per invocation, no chaining) are written for ratatoskr
proper. `./research/bifrost` and `./research/saehrimnir` are separate repos the
orchestrator legitimately manages, so the orchestrator is exempt there for the
review / validate / commit / discard it owns:

- Git: run `git -C <abs-path>/research/<repo> ...` directly (diff, status,
  commit, branch-check, `checkout` to discard). The Bash working directory also
  persists between calls, so a bare `cd ./research/<repo>` followed by a
  separate `git` / `brokkr` command is an equivalent path - just `cd` back to the
  ratatoskr root afterward, since the cwd persists and later steps assume root.
- Validate in place: `cd ./research/<repo>` then `brokkr check` (and focused
  `brokkr test -p <pkg> <name>`). This is the side-quest's gate and runs before
  the commit. It works because each research repo is its own standalone Cargo
  workspace root that brokkr resolves instead of walking up into ratatoskr's:
  bifrost already is a workspace; saehrimnir needed a bare `[workspace]` table
  added to its `Cargo.toml` so cargo would not adopt a parent manifest (a
  committable fix in its own right).
- The CARGO_MANIFEST_DIR gotcha: brokkr builds a nested research workspace with
  `CARGO_MANIFEST_DIR` anchored under brokkr's OWN install path, not the real
  source location. Any test that resolves a committed fixture via
  `env!("CARGO_MANIFEST_DIR")` therefore reads a path that does not exist and
  fails (in saehrimnir this surfaced as the lifecycle tests' "sentinel did not
  appear"). Such tests must resolve fixtures against the runtime working
  directory (`std::env::current_dir()`), which is the crate root under both
  `cargo test` and brokkr. A side-quest that adds or touches such a test uses the
  runtime-cwd form.

Two validation layers exist, and a sync-touching side-quest wants both:

- In place (above): gates the change inside the research copy before promotion.
- Post-promotion, ratatoskr-side from the repo root (no nested-workspace issue):
  for bifrost - a path dep ratatoskr compiles from source - the authoritative
  gate is ratatoskr's own `brokkr check` after `bifrost.sh`. For saehrimnir - an
  installed mock binary - `saehrimnir.sh`'s `cargo install` compile-gates the
  binary, and the behavioral gate is a ratatoskr sync-harness run against the
  reinstalled mock.

Bridge-script assumptions the orchestrator must hold: the work is already
committed in `./research/<repo>`, and that clone is on a branch tracking origin
(not a detached HEAD), so the scripts' bare `git push` succeeds.

## 3. Target architecture (the seam, post-migration)

Bifrost is a service-side dependency. The app stays bifrost-free: it depends on
`rtsk` (core) plus `service-api` wire types only. The service-to-app IPC wire
contract (`ActionWirePlan`, `OperationResult`, `SyncStatusEvent`) is the
firewall and stays stable; `AccountError` never crosses it.

Bifrost owns:

- protocol I/O and the unified `Account` surface (all four providers plus
  CalDAV/CardDAV, behind one trait);
- `AccountError` plus `RecoveryClass` (the structured recovery taxonomy);
- the sync engine: multiplexer, cursor state machine, adaptive polling, push,
  mutation pipeline, recovery dispatch;
- cursor state, as opaque versioned envelopes.

ratatoskr keeps:

- the main DB, body store, inline-image store, attachment file cache, and
  tantivy local search (storage and local search are app-level);
- the application sync layer: JWZ threading, AI bundling, filters, smart
  labels, notifications (`crates/sync`);
- account discovery (the five-stage pipeline) and OAuth authorization (browser
  redirect plus code exchange);
- the `MailActionIntent` action pipeline and the `service`-to-`app` wire
  contract;
- the entire app and UI.

Two seams connect them:

1. Sync seam. The bifrost `SyncEngine` emits a change stream; a new ratatoskr
   consumer persists items plus checkpoints and feeds the unchanged application
   sync layer at the existing `ProviderParsedMessage` / `SyncResult` boundary.
2. Action seam. The action pipeline drives bifrost `Account` mutations;
   `AccountError` maps down to the existing `OperationResult` taxonomy.

## 4. The four structural shifts

These define the character of the migration. They are why it is a rewrite, not
a dependency bump.

1. Persistence inversion. ratatoskr's protocol-sync layer writes the DB itself
   today. Bifrost-sync persists nothing: it emits a change stream and the
   consumer owns the DB write plus a `CheckpointStore` impl. Per-provider cursor
   tables (`jmap_sync_state`, `folder_sync_state`, `graph_*_delta_tokens`)
   retire in favor of opaque checkpoint envelopes.
2. Error-model upgrade. `ProviderError` (7 variants) plus a Transient/Permanent
   binary becomes `AccountError` to `RecoveryClass` (12 variants: Retry,
   Reconcile, Engine directives, AuthLost, NeedsAdminConsent, and more)
   internally, mapped down to `OperationResult` at the wire.
3. Object-level mutations. Thread-level named ops (`archive`, `star`,
   `add_label`) become object-level bulk mutations plus capability-dispatched
   conveniences (`set_starred`, `apply_label`, `move_thread`). Thread-to-message
   expansion happens consumer-side.
4. Capability dispatch. Per-provider special-casing (star as Gmail-label vs
   IMAP-keyword vs Graph-category) moves OUT of ratatoskr and INTO bifrost,
   behind `AccountCapabilities`. ratatoskr calls the convenience; bifrost picks
   the primitive.

## 5. Inventory

Deleted (~33k LOC): the four provider crates `gmail` / `jmap` / `graph` /
`imap` (~24.7k LOC, 51 files); the `provider-sync` crate (~8.5k LOC of
per-provider sync impls); `common`'s `ProviderOps` / `ProviderSyncOps` /
`ProviderError` / `ProviderState` / `create_provider`; the external
`jmap-client` dependency.

Survives untouched: the application sync layer (threading, bundling, filters,
notifications, smart labels in `crates/sync`); discovery and OAuth
authorization in `core`; the DB, stores, and tantivy search; the app; the
service-to-app wire contract.

Rewired: provider construction in `core` / `service`; the action pipeline
bottom; the `calendar` crate; contacts sync; attachments; server-side search;
server-side filters; identities/settings; shared-mailbox and public-folder
scoping. B10 closed the server-side-search seam: B8 had already migrated its
sole live instance, GAL directory enumeration, to `engine.directory_search`.
Mail search remains the provider-free local Tantivy plus smart-folder SQL
pipeline by design; `search` manifest isolation and the `rtsk` / `service`
search-path lockdown tests gate that invariant.

## 6. Track A: making bifrost ideal (bifrost repo, lands first)

Each item is a bifrost-repo spec, filed as a bifrost TODO and run through
bifrost's own loop. Verify each against current bifrost before speccing; some
may be partially built already. A1 is the universal first domino.

- A1. Uniform token rotation. Every factory accepts an
  `Arc<dyn TokenSource>` (or every factory exposes `set_access_token`), so
  ratatoskr drives refresh plus DB write-back through one generic path. Today
  only JMAP and Graph expose rotation; Gmail, IMAP, CalDAV, CardDAV, SMTP do
  not. This blocks all of Track B.
- A2. IMAP send. `Account::send_message` / `draft_send` work for IMAP via an
  SMTP backend wired inside the IMAP account, so ratatoskr sees one uniform
  send surface with no IMAP-plus-SMTP composition leaking up.
- A3. Raw RFC822 hydration. A projection that yields real assembled MIME bytes
  uniformly across all providers (JMAP, Google, Graph included), for the body
  store, attachment dedup, and forwarding. Today raw-MIME assembly is deferred
  on the HTTP providers.
- A4. Scheduled send. A first-class `Account` capability across the providers
  that support it (Gmail, Graph, JMAP), uniform surface plus a capability flag
  where unsupported.
- A5. Shared mailboxes plus public folders. First-class bifrost scopes: Graph
  (EWS / Autodiscover for shared and public folders) and IMAP (NAMESPACE /
  ACL), surfaced through `discover_cursor_scopes` / `discover_memberships` and
  the `Account` surface, fully sync-integrated (not CRUD-only). This is the
  largest Track A unknown; size it after verifying bifrost's current story.
- A6. Cloud-storage attachments. A bifrost surface for large-attachment hosting
  plus share-link generation (Google Drive, Microsoft OneDrive), uniform across
  providers that support it, capability-flagged.
- A7. DAV as first-class synced accounts. CalDAV / CardDAV emit cursor scopes
  and sync-integrate (closing the composed-account dead-end), and compose into
  any account, not just IMAP.
- A8. Provider-wart absorption sweep. Anywhere a wart would otherwise force a
  ratatoskr special-case, bifrost absorbs it behind the uniform surface or
  expresses it as a clean capability flag: Graph `remove_from_container`,
  blob-range uniformity, Gmail attachment inlining behind `attachment_upload`,
  and the rest. Immutable provider limits become capability flags, never
  consumer branches.

## 7. Track B: the ratatoskr rip (this repo, against the ideal surface)

Written purely against the ideal bifrost from Track A. No adapters around
warts, no per-provider branches.

B1 (dependency wiring plus construction plumbing) is done and its TODO entry is
removed per repo convention; the items below that name it as a prerequisite
("Needs B1") have that dependency satisfied. For what B1 delivered - the bifrost
path deps wired into `service`, the `service`-side `build_account_factory` and
generic `DbWriteBackTokenSource` (the construction module at
`crates/service/src/bifrost/`), and the `AccountError`-to-`OperationResult`
mapping - read the B1 landing commit. It is additive: nothing live routes through
it yet (that is B3/B4) and no legacy provider surface was removed.

One load-bearing rule B1 fixed binds every later item: bifrost must not become a
dependency of `core` (`rtsk`). The app depends on `rtsk` (`crates/app/Cargo.toml`),
so any bifrost type that lands in `core` is pulled into the UI build - directly
contradicting § 3's "the app stays bifrost-free; it depends on `rtsk` plus
`service-api` wire types only." Bifrost is confined to `service` and other
writer-side crates; only ratatoskr-owned DTOs and the `service`-to-`app` wire
types cross the core/UI boundary.

B2 (CheckpointStore plus cursor schema) is done and its TODO entry is removed per
repo convention; the items below that name it as a prerequisite ("Needs B1-B2",
"B1 to B2 to B3") have that dependency satisfied. It landed additively: a new
opaque `sync_cursors` table in `crates/db/src/db/schema/10_sync.sql`, the
service-side `SqliteCheckpointStore` over bifrost's own `encode_envelope` /
`decode_envelope` codec (`crates/service/src/bifrost/checkpoint_store.rs`), and a
table-by-table disposition decision for every protocol/sync-state table. The
store and table sit dormant - written by no one, read by no one - until B3 wires
the engine to them; no legacy cursor table or writer was removed. The disposition
enumeration is the reconciliation source for the later deletion cuts: it pins,
for each of `folder_sync_state`, `jmap_sync_state`, `graph_folder_delta_tokens`,
`graph_contact_delta_tokens`, `graph_shared_mailbox_delta_tokens`,
`public_folder_sync_state`, `jmap_push_state`, `graph_subscriptions`,
`shared_mailbox_sync_state`, `public_folder_content_routing`, `pending_operations`,
and `clean_shutdown_cursors`, whether it retires into the opaque envelope (B3/B8/
B12), moves to bifrost ownership (B3b), or is retained app-side - so none is left
orphaned when its writer is dropped. Two pinned open questions ride into the
cutover: the JMAP `shared_account_id` dimension does not fold cleanly into any
`CursorScope` variant (a B3/B12 concern gated on bifrost growing a shared-mailbox
scope), and `jmap_push_state` / `graph_subscriptions` are subscription state, not
checkpoint cursors, dispositioned at B3b (the B3b done-note below carries the
corrected split, which the original B2-note conflated: `graph_subscriptions`
retired into bifrost's `SubscriptionRegistry`, while `jmap_push_state` - a JMAP
WebSocket resume cursor, not a handle - was preserved as nothing, since frozen
bifrost keeps no WebSocket resume position). For
the full disposition table, the brick-by-brick gates, and the design rationale,
read the B2 landing commit.

B3a-infra (the engine harness plus the provider-agnostic durability framework)
is done and its TODO entry is removed per repo convention; the B3a-cut-* / B3b /
B3c sub-items below that name it as a prerequisite ("Needs B3a-infra", "Needs
B3a") have that dependency satisfied. For what B3a-infra delivered - the
`service`-owned `BifrostSyncEngine` (the first live wiring of B2's
`SqliteCheckpointStore` into `SyncEngineBuilder::checkpoints`), the
change-stream-to-DB consumer module (`crates/service/src/bifrost/consumer/`) with
REAL writes to all four stores (main DB, body, inline-image, search), the
`RecvError::Lagged` detach/re-attach recovery, search `flush_now`-before-ack,
ack-last ordering, the single-txn replay-safety marker keyed `(account_id, scope,
checkpoint)`, the per-item Succeeded / Failed / Uncertain hydration taxonomy, the
baseline provider-agnostic membership write, the per-provider post-persist HOOK
SEAM with only the shared `seen_ingest` arm filled, the one-shot
attach/drive/detach driver with completion synthesis from
`backfill_registry().snapshot()` plus a fixed-2s idle cadence, the
`provider_sync::consumer_support` facade that makes the existing `provider-sync`
helpers reachable without relocating them, the test-only attach path
(`TestBifrost*` requests driven by synthetic injected batches), and the new
`bifrost-consumer-*` durability + hot-path harness instruments - read the
B3a-infra landing commit. It landed ADDITIVELY: it cuts no provider over, rewires
no production sync, deletes nothing - legacy `provider-sync` stays live and
authoritative for all four providers, and the consumer was reached only through
the test-only attach path until B3a-cut-jmap (below) routed JMAP onto it.
The HARD ordering constraint the remaining cut specs inherit: no B3a-cut-* may
splice the consumer's lag-recovery driver into production `run_sync` before B3c
lands its backoff / pause-and-surface recovery (the recovery is
correct-but-unbounded and can livelock a structurally-slow consumer), unless that
cut carries its own gated bounded-retry stopgap (B3a-cut-jmap carried one - the
minimal bounded lag-backoff in `engine_sync.rs` - so the remaining cuts can reuse
the same stopgap shape).

B3a-cut-jmap (the first per-provider cutover) is done and its TODO entry is
removed per repo convention; the per-provider cutovers that name it as
establishing the per-provider mechanics (B3a-cut-graph and B3a-cut-gmail, now
also done below, and the remaining B3a-cut-imap sub-item) have those mechanics
available to reuse. It landed as ONE coexistence cutover: JMAP accounts now sync
through `BifrostSyncEngine` + `ChangeStreamConsumer`, while Gmail, Graph, and IMAP
stay on legacy `provider-sync`. For what it delivered - the coexistence dispatch
in `run_sync` (`crates/service/src/sync.rs`) that routes JMAP accounts to the new
engine path and every other provider to the unchanged legacy
`sync_dispatch::sync_for_account`; the service-owned one-shot JMAP runner
(`crates/service/src/bifrost/engine_sync.rs`, with the minimal bounded lag-backoff
the B3a-infra HARD constraint requires); the filled JMAP arms of the consumer's
hydrate / write / post-persist hooks (real engine-driven JMAP hydration over the
`dc670ef` `SyncEngine` passthrough, the folders-only-recompute membership strategy
with keyword labels, the JMAP deletion path, thread participants + chat-state, and
the `is_important` aggregate); the relocation of the four entangled JMAP auxiliary
passes (shared-account discovery, identity resolution, contacts sync,
ShareNotification polling) into `crates/provider-sync/src/jmap/aux_sync.rs` and the
runner branch, not dropped; and the DELETION of the legacy JMAP `provider-sync`
sync impl (`crates/provider-sync/src/jmap/sync/`, `shared_mailbox_sync.rs`, and
`jmap_impl.rs`) - read the B3a-cut-jmap landing commit. The `jmap_sync_state`
`"Email"`/`"Mailbox"` change-cursor writer retires (the engine owns that cursor
via the opaque `sync_cursors` envelope); the `"ShareNotification"` writer survives
because its call site was re-homed; the table schema stays (additive-green) until
B15. The JMAP `ProviderSyncOps::{sync_initial, sync_delta}` arm is gone, but the
JMAP `ProviderOps` action methods survive (B4/B15). The coexistence dispatch is
in-tree provider-kind routing, removed by the final per-provider cut
(B3a-cut-imap). Gated against the bifrost freeze `ae73e92` (§ 11): `brokkr check`
green, `brokkr service-suite` 63/63, and every spec § 6 gate passes -
`jmap-initial`, `jmap-bulk-initial` (10001 msgs), `jmap-steady-state-delta`,
`jmap-incremental-steps`, `jmap-email-set-delta`, `jmap-contacts-initial`,
`jmap-production-lag-backoff`, `jmap-multi-account-{primary,secondary}-isolation`,
the `golden` membership-equality + `hydrate` service tests,
`bifrost-consumer-lag-recovery`, and `parent_sigkill`. The two
multi-account-isolation gates initially read as a redundant wire `Email/get`
regression, but the root cause was the harness matcher: it counted bifrost's empty
open-time `Email/get(ids=[])` `Account::open` probe (which hydrates nothing) as a
fetch. saehrimnir's per-`accountId` state was correct; the fix was a
`count_email_hydrations` helper that counts only `Email/get` calls carrying a
non-empty `ids[]`, preserving the strict "delta fetches zero emails after a
foreign-account mutation" intent while ignoring the no-op probe.

B3a-cut-graph (the second per-provider cutover) is done and its TODO entry is
removed per repo convention; the remaining B3a-cut-imap sub-item reuses
the mechanics it shares with B3a-cut-jmap and B3a-cut-gmail. It landed as ONE coexistence cutover:
Graph (Microsoft) accounts now sync through `BifrostSyncEngine` +
`ChangeStreamConsumer`, while Gmail and IMAP stay on legacy `provider-sync`. For
what it delivered - the `"graph"` arm of the `run_sync` coexistence dispatch
(`crates/service/src/sync.rs`); the service-owned one-shot Graph runner
`sync_graph_account` (`crates/service/src/bifrost/engine_sync.rs`), a near-clone
of the JMAP runner with the same bounded lag-backoff and a single connected
`GraphClient` (`aux_client`) built per kick and shared by the folder-map prepare
and the post-drive aux passes; the filled Graph arms of the consumer's
hydrate / write / post-persist hooks. The consumer machinery was generalized,
not forked: the `jmap_folder_map` field/setter became the provider-agnostic
`folder_map` / `with_folder_map`, and `is_email_scope` now also matches Graph's
per-folder `CursorScope::FolderType { ty: Email, .. }` (JMAP emits
account/type-level email scopes; Graph emits one cursor per folder). The Graph
hydration arm resolves opaque `parentFolderId` containers through the folder map,
translates bifrost's `category:<name>` flags into `cat:` `graph_category` labels
(kept OUT of the keyword set), normalizes bifrost's backslash `\seen` / `\flagged`
canonical flags into the `$`-forms the consumer reads, hydrates with
`HydrationProjection::FullWithBlobs` (Graph omits blob handles under plain `Full`,
which would drop every attachment), and surfaces importance as both the
`importance:*` undeletable labels and the `threads.is_important` aggregate - the
last enabled by a bifrost side-quest (`importance` added to Graph's typed
`hydrate_select`) that advanced the freeze to `7c576bdd` (§ 11). Graph hard-deletes
arrive as a per-folder `ScopeChange{Removed}` (Graph delta has no
created/updated/destroyed split), so a drive-level reconcile accumulates
`removed - live` ids across every per-folder scope batch and deletes only the
remainder at drive end - a move (source-folder `Removed` + destination-folder
`Updated`) survives because the destination's full-replace membership already
corrects it. The five entangled auxiliary passes were relocated into
`crates/provider-sync/src/graph/aux_sync.rs` (folder-map prepare + `importance:*`
seed, master-category labels, contacts, Exchange groups, and a reaction
seeder+refresh poll - the legacy refresher only re-polled messages that already
had reaction rows, so the poll's selection was broadened to also seed
recently-received messages), driven on the existing per-account `graph_sync_cycle`
counter (`increment_graph_sync_cycle`, repurposed from the personal path's
priority-tier scheduling to the aux cadence: master categories + Exchange groups
every 20th cycle, reactions + contacts-delta every 5th - contacts dropped from the
legacy 20th to the 5th so the `graph-contacts-incremental` gate reaches a delta
within its 120s ceiling under the one-shot per-kick connect cost, to be restored
once B3b amortizes it), not collapsed to every-kick (which would inflate the
steady-state Graph request count). The legacy
Graph `provider-sync` sync impl `graph_impl.rs` (the `ProviderSyncOps` orphan) is
DELETED and Graph removed from the `sync_dispatch` / `create_provider` provider-kind
dispatch; the personal Graph path stops writing `graph_folder_delta_tokens` (the
engine owns each `FolderType` cursor via the opaque `sync_cursors` envelope) while
the retained shared-mailbox leg still writes it until B12, and `graph_sync_cycle`
is repurposed rather than dropped. The retired-table schema stays additive-green
until B15. The `graph/sync/` tree is
RETAINED (not deleted) as a deliberate documented deviation: its only remaining
consumer is `graph/shared_mailbox_sync.rs` (B12), which calls the FULL
`graph_{initial,delta}_sync`, so re-homing a "minimal shim" would relocate ~the
whole tree for ~0 net LOC and high risk - B12 deletes/rewires it. The Graph
`ProviderOps` action methods survive (B4/B15). Gated by `brokkr check` green,
`brokkr service-suite`, the `graph-initial` / `graph-steady-state-delta` sync-bench
(the new `graph_steady_state_delta` gate, `meta.provider_requests` pinned
`max_delta = 0`), the `graph_consumer_membership_equals_legacy` membership golden,
the `hydrate_change_graph_category_and_importance_mapping` unit test, the
`graph_drive_reconciles_move/purge` in-process move-vs-purge tests, and the
existing `graph-attachment-*` / `graph-master-category-label-sync` /
`graph-contacts-*` scripts held green across the cut - read the B3a-cut-graph
landing commit.

B3a-cut-gmail (the third per-provider cutover) is done and its TODO entry is
removed per repo convention; the remaining B3a-cut-imap sub-item reuses the
mechanics it shares with B3a-cut-jmap and B3a-cut-graph. It landed as ONE
coexistence cutover: Gmail (Google) accounts now sync through `BifrostSyncEngine`
+ `ChangeStreamConsumer`, while only IMAP stays on legacy `provider-sync`. For
what it delivered - the `run_sync` coexistence-dispatch arm
(`crates/service/src/sync.rs`) that routes Gmail to the engine path, matching the
canonical provider string `gmail_api` (a deliberate sound deviation from the
literal `gmail`: `gmail_api` is the string the account row actually carries); the
service-owned one-shot Gmail runner `sync_gmail_account`
(`crates/service/src/bifrost/engine_sync.rs`), a near-clone of the JMAP/Graph
runners with the same bounded lag-backoff and a single connected legacy
`GmailClient` (`aux_client`) built per kick and shared by the label-folder-map
prepare and the post-drive aux passes; the bifrost factory Gmail arm
(`crates/service/src/bifrost/factory.rs`) honoring `RATATOSKR_TEST_GMAIL_ENDPOINT`
via `GoogleAccountFactory::from_token_source_with_api_base` (the harness redirect
added by the frozen-commit side-quest, mirroring the Graph arm); and the filled
Gmail arms of the consumer's hydrate / write hooks. The Gmail write arm
(`crates/service/src/bifrost/consumer/write.rs`) computes full-thread membership
by the per-message-rows-plus-recompute resolution - per-message `message_folders`
/ `message_labels` rows written through `replace_message_membership_and_recompute`
-> `recompute_thread_folders_from_messages`, which reads ALL of a thread's
persisted message rows on every recompute, so a partial delta batch is correct by
construction (Gmail joins Graph on this helper rather than its legacy
`replace_thread_membership_from_full_coverage`, the lowest-risk of the three
coverage options weighed at spec time). The hydrate arm
(`crates/service/src/bifrost/consumer/hydrate.rs`) maps Gmail labels onto folders
+ labels through the prepared folder map; routes label-only `ScopeChange` rows
(Gmail `labelsAdded`/`labelsRemoved` on an existing message arrive as
`ScopeChange`-only with no `ObjectChange`) through a Gmail `ScopeChange`
re-hydration that updates membership rather than acking the change into oblivion;
synthesizes an `archive` folder membership when `INBOX` is removed and no other
system container (`SENT`/`DRAFT`/`TRASH`/`SPAM`/`archive`) remains (a deliberate
sound deviation, harness-gated by `gmail-incremental-steps`); reproduces reaction
insertion (`insert_gmail_reaction`, resolving the target via `In-Reply-To` ->
`message_id_header`); and treats `STARRED`/`UNREAD` as message STATE
(`threads.is_starred` / `messages.is_read`), NOT `message_labels` rows, while
`IMPORTANT` rides the label/flag surface into the `threads.is_important`
aggregate. The three entangled auxiliary passes were relocated into
`crates/provider-sync/src/gmail/aux_sync.rs` (the label folder-map prepare
`sync_gmail_label_folder_map`; the BIDIRECTIONAL `sendAs` signature sync
`sync_gmail_signatures`, run every kick and made non-fatal-on-error -
log-and-continue rather than legacy-fatal, a deliberate sound deviation
consistent with the aux-pass non-fatal framing; and Google contacts +
other-contacts at the legacy once-on-initial / every-20th-delta cadence driven by
`increment_gmail_sync_cycle`), not dropped. The legacy Gmail `provider-sync` sync
impl is DELETED: `gmail_impl.rs` (the `ProviderSyncOps` orphan) and the entire
`gmail/sync/` subtree (`mod.rs`, `delta.rs`, `storage.rs`, `labels.rs`), with
Gmail removed from the `sync_dispatch` / `create_sync_provider` provider-kind
dispatch; the Gmail `ProviderOps` action methods (`GmailOps`) survive (B4/B15).
The `accounts.history_id` change-cursor WRITER retires (the engine owns the
history-id cursor inside the opaque `sync_cursors` envelope); the column stays
additive-green until B15. Accepted coverage gap, recorded not deferred-as-a-hole:
the multi-message-thread partial-delta sibling scenario (a label change on ONE
message of a multi-message thread, asserting the OTHER messages' membership
survives end-to-end) is NOT integration-gated. The per-message-recompute
resolution makes the partial-batch case correct by construction (the recompute
reads every persisted thread message row), and the
`gmail_consumer_membership_equals_legacy` golden covers the multi-message union
invariant; exercising the partial delta end-to-end would need external
`saehrimnir` multi-message-thread grouping plus a single-message history-delta
emitter plus a new Gmail fixture, deferred as a separate follow-up. Gated against
the bifrost freeze `002e7b9` (full `002e7b9f1b7cfe218b491520f4e1ea7efc7f7997`,
§ 11), advanced from `7c576bdd` for the Gmail mock-redirect side-quest
(`GoogleAccountFactory::from_token_source_with_api_base` /
`from_access_token_with_api_base`): `brokkr check` green, and every Gmail gate
green - `gmail-initial`, `gmail-incremental-steps`, `gmail-production-lag-backoff`,
`gmail-attachment-initial`, `gmail-attachment-prefetch`, `gmail-oauth-multi-account`,
`gmail-send-as-multi-account-import`, the `gmail_consumer_membership_equals_legacy`
membership golden, the Gmail hydrate unit test, and `parent_sigkill`;
`gmail-steady-state-delta` is behaviorally correct with its host-sensitive
`provider_requests` baseline pinned at land. This cut also required `saehrimnir`
Gmail mock extensions (a message-list + per-message get including `format=raw`, an
incremental `history.list` projection, and an oauth refresh grant that resolves
the account from the presented refresh token) - an installed external binary, not
commit-pinned here. Read the B3a-cut-gmail landing commit.

B3a-cut-imap (the fourth and FINAL per-provider cutover) is done and its TODO entry
is removed per repo convention. With it the per-provider cutover series
(B3a-cut-jmap, B3a-cut-graph, B3a-cut-gmail, B3a-cut-imap) is COMPLETE: every
personal-account provider's mail sync now runs through `BifrostSyncEngine` +
`ChangeStreamConsumer`, and only B3b (push) and B3c (control / pause / recovery)
remain open in B3. It landed as the final, fully intrusive cutover and removed the
coexistence scaffolding WHOLESALE. IMAP personal-account mail sync now routes
through the engine + consumer; the legacy IMAP `provider-sync` sync impl is DELETED
- `imap_impl.rs` (the `ProviderSyncOps` orphan) and the entire `imap/` sync subtree
(`imap_initial.rs`, `imap_delta.rs`, `imap_delta_janitor.rs`, `sync_pipeline.rs`,
`thread_store.rs`) - AND so are `sync_dispatch.rs`, the `ProviderSyncOps` trait +
`SyncProviderCtx`, and `create_sync_provider` (the sync-only factory). Because IMAP
was the last legacy provider the `run_sync` coexistence dispatch is removed ENTIRELY
- no provider routes to a legacy path now, so the former `Ok(_)` legacy
fall-through becomes an explicit unsupported-provider error. `create_provider` (the
SEPARATE action-ops factory, `actions/provider.rs`) SURVIVES: it backs ~20 action /
prefetch / attachment call sites, and the IMAP `ProviderOps` action methods
(`imap::ops::ImapOps`) live in the `imap` crate, untouched until B4/B15. The
IMAP-specific engineering this cut required, none of it shared with the HTTP cuts:
drive-end JWZ threading (`threading::build_threads` run ONCE per drive over a
post-adoption-id `ThreadableMessage` accumulator, at the legacy per-cycle boundary,
so cross-batch subject-merge stays byte-identical to legacy output) with the IMAP
change-cursor ack DEFERRED to drive end for crash-safety - a crash after per-batch
persist but before drive-end threading re-drives the un-threaded messages on the
next drive rather than stranding them on provisional thread ids past an advanced
cursor, gated by a new `crash_before_drive_end_threading` consumer hook + the
`bifrost-consumer-imap-crash-before-drive-end-threading.lua` script; identity
adoption by `(account_id, imap_folder, imap_uid)` before insert so existing
legacy-synced rows are reused, not duplicated; the bifrost IMAP CONDSTORE/QRESYNC
delta surfaced as `ScopeChange{Added|Removed}` + `ObjectChange{Updated}` with
`CursorScope::Folder` routing (`is_email_scope` extended to accept it);
`HydrationProjection::Full` for IMAP (`FullWithBlobs` is redundant - bifrost IMAP
returns empty blob handles, so the re-parsed RFC822 MIME tree is the SOLE source of
attachment + inline-image rows); and the relocated IMAP aux passes in
`crates/provider-sync/src/imap/aux_sync.rs` (folder-map prepare + a per-mailbox
`PERMANENTFLAGS` keyword-capability probe, run every kick because `PERMANENTFLAGS`
is per-mailbox and a new mailbox can appear at any time). The cross-provider
`is_important` follow-up that B3a-cut-gmail filed under this item is RESOLVED here,
not carried forward: `is_important` is recomputed as a sticky-OR of the batch
against the persisted `threads.is_important`, so a delta carrying a non-important
sibling can no longer clear a previously-important thread (applied to the JMAP /
Graph / Gmail write arms; IMAP has no importance). A deliberate, validated
DEVIATION rode along: `SyncEvent::Done(checkpoint)` persistence was GENERALIZED to
all providers (the consumer previously ignored `Done`), closing the empty-delta
cursor-durability gap for every provider - the JMAP, Gmail, and Graph steady-state
sync-bench gates were re-run and all held at delta = 0 (no request-count
regression). This cut needed NO bifrost change: the frozen `../bifrost` stays at
`002e7b9` (§ 11; bifrost's IMAP CONDSTORE/QRESYNC behavior was already correct). The
mock work was entirely in `saehrimnir`, whose IMAP mock gained real CONDSTORE/QRESYNC
support (parse `SELECT (CONDSTORE)`, return `HIGHESTMODSEQ`, honor
`FETCH ... CHANGEDSINCE`, emit QRESYNC `VANISHED`) - an installed external binary,
not commit-pinned here, recorded like the sibling cuts' mock extensions. Gated by
`brokkr check` green; the `imap_consumer_membership_equals_legacy` and
`imap_drive_threading_equals_legacy` goldens; the hydrate, identity-adoption, and
threading-reassign unit tests; all ten `imap-*` sync-harness scripts; the three
`bifrost-consumer-*` durability scripts including the new IMAP deferred-ack crash
gate; `parent_sigkill`; `brokkr service-suite` 63/63; and the `imap_steady_state_delta`
sync-bench (baseline pinned on the clean tree at land). Read the B3a-cut-imap landing
commit.

B3b (push plus invalidation - the keep-attached lifecycle) is done and its TODO entry
is removed per repo convention; the only remaining B3 sub-item is B3c (control / pause /
recovery), whose "Needs B3a, B3b" prerequisite is now satisfied. It inverted the B3a
one-shot attach/drive/detach lifecycle to a resident keep-attached engine and lit up
push for all four providers, so new mail arrives with push latency instead of poll
latency. For what it delivered - the service-owned `ResidentEngine` / `ResidentSlot`
runtime (`crates/service/src/bifrost/resident.rs`) that holds each account `attach`ed
across kicks with one long-lived `account_changes_stream` consumer task per slot; the
new `ChangeStreamConsumer::drive_resident` entry (a variant of `drive_to_caught_up` that
does NOT return on the caught-up idle edge - it resets the per-drive Graph move-vs-purge
/ IMAP threading + deferred-ack accumulators at each caught-up boundary and keeps
draining, with every per-batch hydrate / write / post-persist / search-flush-before-ack /
ack-last behavior byte-identical to B3a); the per-kick completion barrier reworked from a
single `Notify` to a monotone `run_seq` + `caught_up` watch channel so an interactive
kick awaits its OWN caught-up edge and a concurrent push reconcile cannot complete it
prematurely; the `run_sync` rewire (`crates/service/src/sync.rs`) from the four one-shot
runners to `ResidentEngine::kick_account`, with the four `sync_<provider>_account` runners
and the `drive_once` helper deleted (`crates/service/src/bifrost/engine_sync.rs` now holds
only the per-provider folder-map prepare helpers), and explicit
`cancel_account_and_await` -> `detach_account` plus `shutdown` -> `ResidentEngine::shutdown`
teardown wiring that preserves the account-delete / Service-shutdown drain invariant
(`engine.detach` 5s timeout + `Account::close()`); in-process push (JMAP WebSocket, IMAP
IDLE) coming alive for free via the engine's attach-spawned forwarder + reconciler; the
new out-of-process push ingress (`crates/service/src/bifrost/push_ingress/`: `mod.rs` +
`pubsub.rs` + `webhook.rs`) - a single Service-owned Gmail Cloud Pub/Sub pull subscriber +
loopback Graph webhook receiver that validate and route notifications to
`SyncEngine::invalidation_sink().push(..)` and perform NO DB write (a forged notification
can at worst force a redundant authenticated reconcile); the factory push-config wiring
(`crates/service/src/bifrost/factory.rs`: Gmail `with_pubsub_config`, Graph
`PushMode::GraphSubscriptions` + `PushEndpoint`) with `subscribe_push` at attach TOLERANT
of the no-config `Unsupported` / `MissingCoreCapability` case as a logged poll-fallback;
the auxiliary passes moved from a per-kick cycle counter to a per-slot wall-clock cadence
task; `Notification::PushEvent` emitted on a SPONTANEOUS changed batch (push-origin is
merged away before the broadcast, so the consumer provably cannot distinguish push from
poll - the legacy "emit if push-derived" was unimplementable against frozen bifrost); and
a bounded transitional stopgap for `Lagged` / `Terminated` / `Pause` (re-subscribe AND
re-push `Invalidated{Unknown}` so dropped changes actually re-drive, not merely reconnect)
plus a secondary size/time-cap accumulator flush that bounds the move-vs-purge / threading
/ deferred-ack state under sustained push with no quiescence window - both
owned-and-removed by B3c - read the B3b landing commit. The legacy push stack is DELETED:
`crates/service/src/push.rs` (the JMAP-only `PushRuntime`), `crates/jmap/src/push.rs` (the
legacy WebSocket driver), and the dead `crates/graph/src/webhooks.rs`; push is no longer a
standalone runtime, so the shutdown drain order drops its leading `Push ->` slot (the
resident engine owns the push bridges and is torn down inside the sync drain). The
subscription-state table disposition CORRECTS the B2-note above, which conflated the two
tables: `graph_subscriptions` retires into bifrost's in-memory per-engine
`SubscriptionRegistry` (re-`subscribe_push` on re-attach makes losing the durable rows
harmless - a surviving server subscription is renewed, a stale one recreated), while
`jmap_push_state` is preserved as NOTHING - frozen bifrost calls `enable_push_ws(.., None)`
and keeps no WebSocket resume position anywhere, so JMAP push recovery is durable
change-cursor replay, not a resume cursor. Both tables stop being written (additive-green;
row deletion is B15). This cut also realized the two B3a follow-ups: keep-attached
collapses the two provider connections per kick to one and removes the per-kick
`COMPLETION_IDLE_INTERVAL` idle tax and the `OAuthRefresher` first-read refresh from the
steady-state path, and the four steady-state sync-bench baselines were re-recorded against
the measured drop. A B3b bifrost side-quest advanced the freeze from `002e7b9` to
`db34ab4` (§ 11): the push-gate work surfaced a JMAP-WebSocket `StateChange` parser bug in
`client_ws.rs` - a double `@type` tag that rejected conformant RFC 8620/8887 frames -
fixed upstream. The mock work was a separate `saehrimnir` push side-quest (JMAP WebSocket
`StateChange` push frames, a Gmail Pub/Sub source plus `users.watch` / `users.stop`, and
Graph `POST /subscriptions` + notification POST to the registered loopback
`notification_url`), an installed external binary not commit-pinned here, with a
harness-mode ingress validation bypass keyed to the mock's signing material. Gated by
`brokkr check` green, `brokkr service-suite`, the four per-transport push scripts
(`jmap-push-websocket`, `imap-push-idle`, `gmail-push-pubsub`, `graph-push-webhook`), the
B3a regression + durability scripts held green under the longer lifetime, the new
`bifrost-consumer-sustained-push-bound` accumulator-bound gate, the re-recorded
steady-state sync-bench amortization baselines, and the resident-teardown /
no-table-writer service tests - read the B3b landing commit.

B3c (control / pause / recovery - the final B3 sub-item) is done and its TODO
entry is removed per repo convention; with it B3 (the bifrost-sync consumer) is
COMPLETE - the next open Track B item is B4 (action pipeline rewire). It closed
the control loop B3a/B3b left open: a resident slot used to react to any
structurally-terminal condition by logging and blind-re-driving every 250 ms
forever, so a token revoked mid-sync livelocked the slot AND hung the
`kick_account` future (no caught-up edge ever fired), and an engine `Pause` was
invisible because nothing subscribed to the control broadcast. For what B3c
delivered - a THIRD per-slot task (`resident_control_loop`,
`crates/service/src/bifrost/resident.rs`) that subscribes once to
`engine.account_control_stream(account)` and dispatches
`AccountControl::Pause(reason)` / `Resume`: an intervention-required reason
(`RetryBudgetExhausted` / `OperatorOverrideRequired`) latches a per-slot
`terminal` watch cell and emits the new `Notification::AccountPaused` standing
banner, the engine-resolved `TenantThrottle` is observed-and-logged (it
auto-resumes engine-side), and a bounded control-stream `Lagged` fails CLOSED
(latch a dismissible `NeedsAttention`) rather than silently dropping lifecycle
state; the consumer's `SyncEvent::Terminated` arm
(`crates/service/src/bifrost/consumer/mod.rs`) changed from log-and-swallow to
mapping the `AccountError` through the B1 `account_error_to_operation_result`
into a `TerminalFailure` carried out on `ConsumerDriveReport.terminal`, which
the resident loop latches on `slot.terminal` to UNBLOCK `kick_account` with a
structured `Err` - so a mid-sync terminal now reaches `run_sync`'s existing
`SyncResult::Failed` + `MarkerStatus::Failed` + `SyncCompleted { Failed }` path
exactly as a synchronous attach failure does, fixing the kick-hang livelock;
a MERGE discipline on the two writers of `slot.terminal` (the consumer sets
`result`/`message`, the control loop sets only `pause`) so a result-bearing
outcome is never clobbered to pause-only across the unordered `changes_tx` /
`account_control_tx` channels, with the wire `SyncPauseReason` (`NeedsReauth`
upgraded from the latched error's `AuthLost` recovery class, else
`NeedsAttention`) derived in `bifrost/error_map.rs`'s new `pause_reason_to_wire`
rather than from the bare `PauseReason`; the B3b fixed-250 ms re-subscribe/repush
stopgap REPLACED by a `RecoveryClass`-driven bounded exponential backoff (jitter,
capped, reset on a clean caught-up edge) for the `Lagged` / closed-stream /
drive-error arms - the retry-vs-terminal bool reads the mapped
`OperationResult`'s `RemoteFailure.retryable` rather than minting a third
classification, and all `Terminated` outcomes park (record + idle, no
re-drive); the size/time-cap accumulator flush RETAINED unchanged (it is a
genuine steady-state memory invariant gated by
`bifrost-consumer-sustained-push-bound.lua`, not a stopgap) and reconciled into
the finalized model in the consumer module doc (a pause self-limits because the
parked boundary stops new batches; the caps remain the backstop for the
no-pause sustained-push case); a new `service-api` wire surface
(`SyncPauseReason`, `Notification::AccountPaused(AccountPausedNotification)`
carrying `service_generation` with arms in `class()` / `method_name()` /
`service_generation()` / `set_service_generation()`, and the new
`sync.resume_account` request `SyncResumeAccount` -> `SyncRuntime::resume_account`
-> `ResidentEngine::resume_account` -> `engine().resume_account` that clears the
latch and resumes an attached boundary) plus an app-side `AccountPaused` banner
on the existing sidebar account-row sync-status surface; and the verified
end-to-end cursor-clear on `SchemaIncompatible` / `RestartScope` (the engine
owns the clear and calls `SqliteCheckpointStore::delete_change_cursor`; B3c pins
the durable side). The firewall holds: no bifrost type (`AccountControl`,
`PauseReason`, `AccountError`, `RecoveryClass`) crosses into `service-api` /
`core` / `app`; the one UI-bound type is the service-api-local `SyncPauseReason`,
mapped down inside `service`. The re-auth path needed no new call: a successful
token writeback already detaches + reattaches, and `SyncEngine::attach` mints a
fresh `Run` boundary + slot per attach, so the engine `Pause` does not survive
the cycle; the new `resume_account` is the dedicated banner-clear action for a
user dismissing a pause WITHOUT a token change. B3c required NO bifrost change -
every control/recovery surface it consumes (`account_control_stream`,
`resume_account`, the `AccountControl` + `SyncEvent::Terminated` broadcasts) was
already public at the frozen `db34ab4` (§ 11) - so the freeze holds and the
side-quest count stays at seven; its only test additions were in-tree (a
`ForceTerminated { recovery }` consumer hook + `resident_redrive_*` probe
telemetry) plus a saehrimnir budget-exhaustion affordance (an installed external
binary, not commit-pinned here). Gated by `brokkr check` green,
`brokkr service-suite`, the new `jmap-terminated-mid-sync-fails.lua` and
`jmap-pause-resume.lua` (the latter driving a real engine `RetryBudgetExhausted`
pause through `account_control_tx`, not a hook), the extended
`bifrost-consumer-lag-recovery.lua` / `jmap-production-lag-backoff.lua` bounded
re-drive gates, the `checkpoint_store_change_roundtrip` cursor-delete unit test (the § 4.5
fallback covering `delete_change_cursor`; the live engine cursor-clear wire rides
`jmap-oauth-recovery.lua`), the
`bifrost_error_map_*` + `account_paused` notification-contract + resident-teardown
unit tests, and the `bifrost-consumer-sustained-push-bound` / account-delete /
`sigint-mid-prefetch` drain regression guards held green - read the B3c landing
commit.

B4a (the action-pipeline mutation-dispatch rewire, the first half of B4) is
done; at the time it landed the B4 TODO entry below was kept and narrowed to its
then-remaining open sub-item B4b (the error-model + retry-journal cleanup, since
landed - see the B4b done-note below), per repo convention. It
rewired the BOTTOM of the email-action pipeline off the per-provider
`ProviderOps` mutation surface and onto the already-resident bifrost
`SyncEngine`, driven through a new per-account mutation handle
(`ResidentActionAccount`, `crates/service/src/bifrost/resident.rs`) - the top
half (intent resolution, planning, the `action_jobs` durable plan journal and
its crash-replay worker, the local `*_local` writes, optimistic-UI/undo/toast)
is preserved exactly. For what it delivered - a new
`crates/service/src/actions/dispatch_target.rs` that owns the single
consumer-side thread->message expansion (`resolve_thread_messages` resolves the
local `thread_id` to the thread's bifrost message `ObjectId`s from the
`messages` table, uniform across all four providers and NOT dependent on a
server thread id IMAP has none of), the declarative role->container resolution
(archive/trash/spam/move expressed object-level over the `MutationTarget`-typed
membership primitives `add_to_container` / `remove_from_container`, dest and
per-message source resolved by `FolderRole`, never a `match` on provider kind),
and the exhaustive `dispatch_mutation` / `dispatch_bulk_mutation` that replaced
`dispatch_with_provider` (`SetStarred`/`SetRead` -> `set_starred`/`set_read`;
`AddLabel`/`RemoveLabel` -> `apply_label`/`remove_label`, with
`LabelKind::GraphImportance` routed to the exclusive `set_importance` primitive
so the high/low exclusivity survives; `PermanentDelete` -> `bulk_destroy` kept
provider-first so the local rows that carry the remote refs survive a retry);
the same-account same-op coalescing (`RemoteBatchKey`) that accumulates resolved
`ObjectId`s and dispatches through the bulk surface so a 200-thread campaign
keeps the batched wire shape today's providers issue rather than regressing to N
single-object calls; the `batch.rs` dispatch and the `pending.rs` retry
re-dispatch both rewired onto the resident handle (no per-action
`create_provider`); the composite label-group fan-out
(`label_group.rs` / `label.rs`) rewired onto `apply_label`/`remove_label` while
preserving the single-composite-row retry contract (never N member rows); and
the `SetRead` action rewired onto `engine.set_read`, its read-receipt (MDN)
follow-up re-homed onto the new dispatch path (`send_mdn_for_read`,
`mark_read.rs` -> `mdn_send.rs`). One deliberate deviation from the spec's plan:
where the spec called for routing the `mark_mdn_sent` write-back through
`engine.mark_mdn_sent` as part of the `SetRead` dispatch, B4a instead LEFT the
MDN send and its `mark_mdn_sent` keyword sync on the provider send path (a
`ProviderOps` provider built on demand), because MDN composition/send is B5
territory and splitting the keyword sync from the send it pairs with buys
nothing - the engine MDN hook waits for B5. The send-path `mark_send_intent`
likewise stays B5. The IMAP-only schema prerequisite landed inside
this cut as its first step: an `imap_uidvalidity` column on `messages`
(`crates/db/src/db/schema/02_mail.sql`) populated at consumer hydrate time
(`bifrost/consumer/hydrate.rs` now keeps the `uidvalidity` it previously dropped
out of the decoded IMAP object id), so `resolve_thread_messages` can reconstruct
a valid IMAP `ObjectId` (`imap1:<len>:<folder>:<uidvalidity>:<uid>`);
Gmail/JMAP/Graph ids round-trip from the stored provider message id and need no
migration. The error half uses the EXISTING B1 `account_error_to_action_error`
map (already `RecoveryClass`-derived, not a shim), so B4a is behavior-equivalent
across the cut. B4-SQ (the additive `SyncEngine` mutation-passthrough cluster,
mirroring the B3 read-only hydration passthrough so every action mutation runs
against the freshly-installed `live_account` connection, never a stale snapshot)
landed FIRST in `./research/bifrost` per the side-quest protocol and was
promoted to the `../bifrost` dependency before any ratatoskr mutation brick - read
the B4-SQ landing commit. The error-model + retry-journal cleanup B4a deliberately left
compile-gated (the dead `classify_provider_error` heuristic, the dead
`create_provider`-for-actions arms, and the per-op retry-budget reconciliation)
landed separately as B4b - see the B4b done-note below. Gated by `brokkr check` green,
`brokkr service-suite`, the IMAP write-back scripts (`imap-writeback-flags.lua`,
`imap-writeback-move-delete.lua` - the latter rewired to verify move/delete by
server round-trip rather than IMAP wire-op needles, since bifrost moves via the
atomic RFC 6851 UID MOVE), the three new per-provider
`jmap-` / `graph-` / `gmail-action-writeback.lua` round-trip gates, the
exhaustive `dispatch_mutation` mapping unit test, the composite
`apply_label_group` contract test, and the journal/crash replay regression
scripts held green - read the B4a landing commit. The
`bulk_archive_200_threads_under_budget` throughput gate (a `service-harness`
`t1/` script; B4a's coalescing is structural - the `RemoteBatchKey` grouping
routes same-account / same-op entries through one `bulk_move` / `bulk_set_flags` /
`bulk_destroy`) held green within `service-suite` 63/63. Three lateral follow-ups
are recorded as their own separate items (NOT B4b, which is scoped to the
error-model cleanup): bulk star never coalesces (the capability-aware
`set_starred` is per-id for any set size, so a large star campaign issues N wire
ops); IMAP single-message Archive/Trash from a NON-inbox folder resolves
`source = INBOX` and so degrades to a retryable LocalOnly (bounded - resync
reconciles, the common archive-from-INBOX and advisory-source MoveToFolder cases
are fine); and a latent FROZEN-bifrost Gmail un-spam divergence (`move_patch`
adds INBOX without removing the SPAM label when the bulk destination is INBOX,
so a bulk un-spam campaign on Gmail leaves the SPAM label - the singleton path's
explicit add+remove avoids it, only the >1 bulk path hits it; a future bifrost
side-quest).

B4b (the error-model + retry-journal cleanup, the closing half of B4) is done;
with it landed B4 is COMPLETE and its TODO entry below is updated accordingly,
per repo convention. It ripped the pre-bifrost dead surface B4a left
compile-gated and unified the action pipeline on a single remote-error
classifier and a single retry authority. Concretely: the per-provider string
heuristic `classify_provider_error` (and its
`provider_creation_errors_are_classified_for_retry` test) is DELETED, leaving
the `RecoveryClass`-derived `error_map.rs::account_error_to_action_error` as the
only remote classifier in the action path; the seven dead
`create_provider`-for-actions dispatch arms (`archive` / `star` / `trash` /
`spam` / `mark_read` / `move_to_folder` / `permanent_delete` plus their
`*_dispatch` halves and `enqueue_permanent_delete_retry`) are removed along with
their now-dead `pub use` re-exports and pruned imports, keeping each module's
live `*_local` DB-mutation half; `create_provider` itself STAYS (no action
caller now, but live for the send / draft / folder-CRUD / attachment / MDN /
prefetch paths, the MDN write-back deferred to B5). The genuine correctness fix
is the `error_map.rs::recovery_to_failure_kind` Engine split: the previously
collapsed `RecoveryClass::Engine(_) -> Transient` arm is split by directive so
the operator-blocked terminal directives (`SchemaIncompatible` /
`OperatorOverrideRequired`) map to `Permanent` and no longer enqueue a doomed
retry, while the auto-recoverable directives (scope/account restart, strategy or
capability downgrade, scope disable) stay `Transient`/retryable - the same
single classifier also feeding `account_error_to_operation_result` and
`pause_reason_to_wire`, so the `SchemaIncompatible` / `OperatorOverrideRequired`
pause banner tightens too. The `batch.rs` degraded path is reclassified from the
bare `ActionError::remote` (`Unknown`) to an explicit `Transient`, and
`pending.rs::retry_policy` keeps its per-op budgets (folder 10 / label 7 / flags
5) with its doc reconciled to state the budget is the retry CEILING consulted
ONLY after `is_retryable()` (the one `RecoveryClass`-derived gate) admits the
failure, so a terminal class never reaches the budget. Gated by the
`bifrost_error_map_*` unit tests (the `SchemaIncompatible` case repointed from
Transient to Permanent, a new auto-recoverable `CapabilityChanged ->
RestartAccount` Transient case added), `pending_retry_classification` +
`retry_budget_only_for_retryable`, `action_error_retryable_classification`, the
rewritten `archive_nonexistent_thread_does_not_succeed` (now driven through the
live `batch_execute` path, not the deleted `archive::archive`), the exhaustive
`dispatch_mutation_mapping_is_exhaustive` guard, the five per-provider
action-writeback sync-harness round-trip scripts, the `pause_reason_to_wire_*` +
`jmap-pause-resume.lua` banner gates, and `brokkr service-suite` / `brokkr
check` green - read the B4b landing commit.

B6 (folders, labels, containers) is done and its TODO entry is removed per repo
convention; with it the folder/label OBJECT layer (the container objects
themselves - names, hierarchy, system roles, label colors - and their CRUD) is
off the legacy provider stack, leaving B7 (calendar) as the next open Track B
item. B6 is the container leg the membership halves left open: B3 moved
folder/label MEMBERSHIP onto the consumer and B4a moved thread-level
`apply_label` / `remove_label` onto the engine, but the container OBJECTS and
their create/rename/move/delete still spoke to `ProviderOps` / `create_provider`.
B6 rewired both surviving seams onto bifrost's `SyncEngine` and deleted the
legacy folder surface. It landed in three ratatoskr bricks on top of the B6-SQ
bifrost prerequisite (see below). B6a (the list-sync rewire): a new
provider-agnostic `crates/service/src/bifrost/containers.rs` whose
`sync_containers` calls `SyncEngine::containers_list` once and whose
`persist_containers` / `build_container_rows` partition the returned
`Vec<Container>` into `folders` / `labels` rows through the UNCHANGED
`insert_folders_batch` / `upsert_labels` seam, returning the same
`HashMap<String, FolderKind>` folder map the action dispatch and push scopes
consume; this collapsed `prepare_folder_map`'s four-arm per-provider dispatch
(`crates/service/src/bifrost/resident.rs`) to a single call and the four
hand-rolled system-role normalisers into one `FolderRole -> canonical id` table
(`role_to_system_folder_id`, pinned against the glossary Identity table by
`container_role_maps_to_canonical_id`). A container routes to the `folders`
table when it is folder-shaped (`kind == Folder`), carries a `role`, OR is a
native system container (the B6-SQ `system` flag, which reproduces Gmail's
`CATEGORY_*` / `IMPORTANT` / `CHAT` system-label-as-folder split that `role`
alone cannot - message-state ids like `STARRED` / `UNREAD` are filtered first),
otherwise to `labels`; Gmail user-label server colors flow from `Container.style`
into `server_color_*` while `user_color_*` stays a local override the sync never
clobbers (`upsert_labels` COALESCE-preserves it). `attach_account` was reordered
so `engine.attach` precedes the list sync (Obstacle A': `containers_list`
resolves through `live_account`, which would `AccountNotAttached` under the old
prepare-before-attach order), and the surviving IMAP keyword-capability aux pass
now sources its folder-path list from the already-synced `folders` table instead
of the deleted folder-map facade. B6a DELETED `prepare_jmap_mailboxes` /
`prepare_graph_folders` / `prepare_gmail_labels`, the four
`consumer_support::sync_*_folder_map` facades, and the
`{jmap,graph,gmail,imap}/aux_sync.rs::sync_*_folder_map` impls. One recorded
feature gap (feature-preserving mandate): the legacy JMAP folder-map pass
populated the `right_*` ACL columns and `is_subscribed` from `Mailbox.myRights` /
`isSubscribed`, which the `Container` shape AT B6 LANDING carried on neither
field, so those columns (consumed by shared-mailbox submit gating, B12) wrote
`None`. Since restored: a follow-up bifrost SQ (freeze `0e71226`) added
`ContainerRights` + `Container.rights` / `is_subscribed`, populated by
bifrost-jmap's `containers_list`, and `build_container_rows` now reads them into
the `right_*` / `is_subscribed` columns (golden-pinned by
`jmap_folder_rights_and_subscription_map_to_columns`). B6b (the CRUD rewire): `service::actions::folder::{create,
rename,delete}` plus a new `move_folder`, and the parallel new
`service::actions::label::{create,rename,delete,recolor}`, dispatch through
`ResidentActionAccount` onto `engine.container_create` / `container_rename`
(with the B6-SQ `style` arg for recolor) / `container_move` / `container_delete`,
keeping the provider-first ordering, `MutationLog`, and outcome taxonomy
unchanged; the vestigial folder color params were dropped (folders carry no
color). B6b DELETED `ProviderOps::{list_folders, create_folder, rename_folder,
delete_folder}` (the rest of `ProviderOps` survives to B15), their four
per-provider impls, and the `OfflineOps` stubs. Because § 5 keeps a user-facing
CRUD IPC + UI out of scope (no such request or affordance existed pre-B6), the
handlers stay callable-but-unwired exactly as before, now on bifrost; a
harness-only `TestContainerCrud` request (`test.container_crud`) was added so the
`*-container-crud.lua` gates can drive them. IMAP container CRUD is a capability
GAIN here (the legacy `ProviderOps` IMAP stub returned `Failed`; bifrost
implements mailbox CREATE/RENAME/DELETE) - a documented deviation in the
direction the migration intends, exercised by `imap-container-crud.lua` (its
rename re-key gate was carved out and has since been resolved). B6c (residue
sweep): removed `ProviderFolderMutation` and reconciled
`reference/glossary/folders-labels.md` and `reference/architecture.md` (bundled
with the code). The whole item rode on the already-promoted B6-SQ bifrost
prerequisite: `../bifrost` + `./research/bifrost` are frozen at `60e979b` (§ 11),
which added the `SyncEngine::containers_list` read passthrough, the optional
`style: Option<ContainerStyle { color_bg, color_fg }>` and `system: bool` fields
on `Container` / `Label` (Gmail-populated; Graph / JMAP / IMAP leave them unset -
the earlier "Graph category preset color" carry was struck as dead work, Graph
categories being message flags, not containers), and the optional `style` arg on
`container_create` / `container_rename`. Out of scope, named not deferred: the
new user-facing folder/label CRUD IPC + sidebar/Settings UI (a `UI.md` feature
item) and shared-mailbox / public-folder containers (B12). (The JMAP ACL /
subscription columns dropped at landing were since restored - see the `0e71226`
note above.) Two action-layer bugs surfaced at gate time and were fixed in the landing: the
B6b CRUD handlers resolved a target through the resident slot's CACHED folder map
with no refresh-on-miss (so a folder created earlier in the same session could
not be moved/renamed/deleted) - fixed with a `refresh_folder_map`-then-retry
(`resolve_container_with_refresh`) in `actions::folder`; and `move_folder` alone
skipped the local DB write-back that create/rename/delete perform, so a reparent
never landed locally - fixed with a targeted `update_folder_parent_sync`. Gated
by the `containers_persist_equals_legacy` golden (asserting the new rows match a
captured legacy snapshot per provider, including the Gmail system-label-as-folder
rows, a pre-seeded `user_color_*` override, and an `importance:*` undeletable
row), the `container_role_maps_to_canonical_id` + container-CRUD-dispatch unit
tests, the `jmap-container-crud.lua` + `graph-container-crud.lua` round-trip
gates, and `brokkr service-suite` / `brokkr check` green. Three CRUD-path gates
were initially carved out rather than block the landing, all SINCE RESOLVED (see
git history): `imap-container-crud` (the rename handler now re-keys the local
`folder-{path}` id and cascades the membership / child FKs, via a new
`rekey_imap_folder_sync` DB helper), and `gmail-container-crud` +
`gmail-label-color-roundtrip` (their scripts now mirror `gmail-initial.lua`'s
OAuth seeding). The separate `gmail-initial` zero-ingest carved alongside them
turned out to be TWO faults: a stale installed `saehrimnir` binary (which also
silently broke `jmap-initial`) plus a latent bifrost cold-start backfill
subscriber race (freeze `bea56e8`; see § 11). The `containers-attach` sync-bench
gates exist with per-host baselines to record via `--as-baseline`. Read the B6
landing commit.

- B3. The bifrost-sync consumer (center of gravity), carved into per-provider
  cutovers so no single landing carries the whole rip. The `SyncEngine` and the
  change-stream-to-DB consumer that persists each batch (messages, body store,
  inline images, search), flushes search, then acks the bifrost checkpoint LAST
  are stood up by B3a-infra (done; see the done-note above), and JMAP, Graph,
  Gmail, and IMAP are ALL routed onto it by B3a-cut-jmap, B3a-cut-graph,
  B3a-cut-gmail, and B3a-cut-imap (all done; see the done-notes above). The
  per-provider cutover series is therefore COMPLETE: every personal-account
  provider syncs through the consumer, and each provider's ACTUAL post-persist
  processing - asymmetric, not a uniform pipeline - is now replicated in it: IMAP
  runs JWZ `threading::build_threads` at drive end;
  JMAP / Graph / Gmail write the thread aggregate inline (`upsert_thread_aggregate`)
  with per-provider membership strategies;
  `seen_ingest` is the one shared pass;
  bundling, filters, smart labels, and `evaluate_notifications` have NO sync-time
  callers today and stay unwired (whether they should auto-fire on new mail is a
  separate product item, explicitly not B3's scope - feature-preserving means the
  consumer reproduces today's behavior, not that it inherits an unwired gap as a
  feature). B3 is COMPLETE: B3a-infra, the four per-provider cutovers, B3b
  (push), and B3c (control / pause / recovery) have all landed - see the
  done-notes above. The worked-out design - the per-provider seam survey,
  durability ordering, and the lag / hydration / completion policies each
  sub-spec was carved from - was produced during B3 spec review and consumed by
  each sub-spec's author.
- B4. Action pipeline rewire. COMPLETE: B4a (mutation dispatch) and B4b
  (error-model + retry-journal cleanup) have both landed - see the B4a and B4b
  done-notes above. B4a moved the bottom of the email-action pipeline off the
  per-provider `ProviderOps` mutation surface onto the resident bifrost
  `SyncEngine` (resident-engine mutation dispatch onto `Account` conveniences
  plus bulk mutations over `MutationTarget`, the consumer-side
  thread-to-message expansion, role->container resolution, and bulk coalescing),
  so the action pipeline no longer constructs a `ProviderOps` per batch; B4b
  ripped the dead `classify_provider_error` heuristic and the
  `create_provider`-for-actions dispatch arms B4a left compile-gated, split the
  `RecoveryClass::Engine` arm so an operator-blocked engine failure (and the
  already-correct `AuthLost`) does not enqueue a retry that can never succeed
  while a `Reconcile` / `Retry` one does, and reconciled the per-op retry
  BUDGETS (`retry_policy`: folder 10 / label 7 / flags 5) downstream of the one
  `is_retryable()` gate. Needed B1, A1-A2 (all satisfied).
- B5. Send plus drafts. LANDED - the send / drafts / scheduled-send / MDN action
  paths now dispatch through the resident bifrost `SyncEngine` (`engine.send_message`
  / `send_raw_message` / `draft_discard` / `mark_replied` / `mark_forwarded`), with
  `to_bifrost_send_request` mapping ratatoskr's `SendRequest` onto bifrost's
  (per-element address parse, inline-attachment `content_id` carry,
  `request_read_receipt` set to preserve the outgoing read-receipt request), the
  RFC 8098 MDN sent as raw bytes via `engine.send_raw_message` at the two `batch.rs`
  follow-up sites, the `lettre` assembler retired, Gmail header-based threading
  preserved, scheduled send wired end-to-end (`scheduled_at` IPC field, journaled
  job, worker route, cancel/reschedule verbs, capability-gated via
  `engine.account_capabilities(..).pim_methods.scheduled_send`), and a failed send
  job finalizing terminal (not re-leased). The legacy `ProviderOps` send impls stay
  live until B15. Required two side-quests: B5-SQ advanced bifrost to `8ea29b6` (the
  `SyncEngine` compose passthroughs - `send_message` / `draft_*` /
  `cancel_scheduled_send` / `reschedule_send` / `account_capabilities`, the real new
  `send_raw_message` `Account` trait method across all four mail crates, `content_id`
  on `AttachmentInline`, `request_read_receipt` on `SendRequest`); and a `saehrimnir`
  send side-quest (SMTP `SIZE` 256 MiB + submission header projection, JMAP
  `Blob/upload` + `Email/import` + `EmailSubmission/set`, Gmail `messages.send`,
  Graph send-to-Sent, scheduled-send acknowledgement) - an installed mock binary,
  not commit-pinned. Gated by `brokkr check` green, the `scheduled_send_capability_gate`
  / `send_intent_maps_to_engine_mark` / `to_bifrost_send_request` unit tests, the
  `gmail-` / `imap-scheduled-send-rejected.lua` capability gates, and 45/46
  `service-suite` (the one red gate is the deferred B5-FIX below). Two follow-ups
  ride out of B5, each its own item:
- B5-FIX. DONE. The `compose_send_50mb_attachment` failure was NOT a large-DATA /
  bifrost-smtp transport issue - that framing was a misdiagnosis. The real cause:
  the bifrost SMTP submission transport was never redirected to the `saehrimnir`
  mock, so bifrost dialed the persisted placeholder host `smtp.example.test` and
  failed at DNS resolution (`failed to lookup address information`, surfaced as
  `transport.network` ~533 ms in) BEFORE any DATA was sent - the 50 MB body and the
  EHLO SIZE stage were never reached. `build_imap_factory` already honored
  `RATATOSKR_TEST_IMAP_ENDPOINT`; the fix adds the parallel
  `RATATOSKR_TEST_SMTP_ENDPOINT` redirect in `smtp_submission`
  (`crates/service/src/bifrost/factory.rs`): host:port from the endpoint, forced
  `SubmissionTls::Plaintext` (the mock's self-signed STARTTLS cert is rejected by
  `starttls_relay`'s native-tls verifier, and the mock accepts cleartext AUTH).
  Production is unaffected - the override only fires when the env var is set. No
  bifrost or saehrimnir change was needed; the earlier `SendAttachment.data ->
  bytes::Bytes` double-buffer fix and saehrimnir's `SIZE` 256 MiB bump were
  red-herring changes that landed harmlessly (the mock handles a 69 MB DATA body
  over both plaintext and STARTTLS, confirmed in isolation). Gated by
  `compose_send_50mb_attachment` green and `service-suite` 63/63 - read the landing
  commit.
- B5-GATES. The per-provider send / MDN / scheduled-send ROUND-TRIP harness gates
  that were NOT built in B5 (the `saehrimnir` send side-quest unblocking them only
  landed at B5's tail). The three send-writeback gates are DONE:
  `jmap-` / `gmail-` / `graph-send-writeback.lua` (sync-harness) drive the real
  send action (`ActionSend` -> resident `SyncEngine` `engine.send_message`) against
  `saehrimnir` over each provider's native submission surface (JMAP `Email/set` +
  `EmailSubmission/set`, Gmail `messages.send`, Graph send), then verify two ways,
  mirroring the B4a action-writeback gates: the `action.completed` summary shows the
  send dispatched REMOTELY (`remote_succeeded >= 1`, `remote_failed == 0`,
  `local_only == 0`, `conflicts == 0` - catching a silent local-only degrade), and a
  fresh resync brings the sent message back filed under `SENT` (server round-trip).
  They run on a new `send-small.toml` sync-fixture (Inbox/Drafts/Sent + a seed
  draft). The JMAP gate required a `saehrimnir` fix (`build_email_from_create` now
  projects the structured compose properties - subject, the `from`/`to`/`cc`/`bcc`
  address lists, `messageId`, and the `bodyValues` text body - that the `Email/set`
  create carries; it previously dropped them, so a sent message round-tripped with an
  empty subject). `imap-draft-discard.lua` is also DONE: it seeds an IMAP account on
  the same fixture, syncs the seed draft, then drives the new harness-only
  `test.discard_draft` trigger (`TestDiscardDraft` -> resolve the draft's bifrost
  `ObjectId` via the action pipeline's `resolve_thread_messages` ->
  `engine.draft_discard`, the remote leg of `actions::delete_draft`) and asserts a
  follow-up resync shows the draft GONE from the server. That gate required a
  `saehrimnir` fix: the IMAP mock implements `UID EXPUNGE` but never advertised
  `UIDPLUS`, so bifrost's `delete_messages` (`UID STORE \Deleted` + `UID EXPUNGE`)
  returned `Unsupported(DraftDiscard)`; the mock now advertises `UIDPLUS`.
  `jmap-scheduled-send.lua` is also DONE: the capable-provider counterpart to the
  `*-scheduled-send-rejected.lua` gates, it drives a scheduled `ActionSend` on a JMAP
  account (which advertises a non-zero `maxDelayedSend`, so `pim_methods.scheduled_send
  = true`) and asserts the send is ACCEPTED (clean remote dispatch via FUTURERELEASE
  `holduntil`) and round-trips under `SENT`, rather than being rejected at the
  capability gate. It needed a new `harness.wall_ms()` Lua binding returning wall-clock
  UNIX-epoch milliseconds: `harness.now_ms()` is monotonic (ms since harness start), so
  a computed `scheduled_at` read as a past instant that bifrost's `validate_scheduled`
  rejected, and no hardcoded absolute timestamp stays valid against the 1-year
  `maxDelayedSend` window. The MDN (read-receipt) round-trip gates are DONE for two
  providers: `imap-mdn.lua` and `gmail-mdn.lua` (on a new `mdn-small.toml` fixture whose
  Inbox message carries a `Disposition-Notification-To` header via `body_raw_bytes`, so
  the consumer's raw re-parse sets `mdn_requested = 1`). Each seeds an account-scoped
  `always` read-receipt policy (via a new optional `read_receipt_policy` field on
  `TestSeedAccount`, since the default global policy is `never` and only `Always`
  auto-sends), marks the thread read, and asserts the RFC 8098 MDN dispatched via
  `engine.send_raw_message` reached the requesting sender - IMAP by the mock's SMTP
  submission log, Gmail by a resync finding the MDN filed under `SENT` addressed to the
  sender. The remaining two providers - `jmap-mdn.lua` and `graph-mdn.lua` - are now
  also DONE, each unblocked by a `saehrimnir` mock side-quest. The JMAP gate needed the
  mock's `/jmap/download` to serve a message's WHOLE assembled RFC822 under its own
  `blob-{id}` blob id (it served only attachment blobs before, so the consumer's
  `open_raw_rfc822` could not detect `Disposition-Notification-To` and never set
  `mdn_requested`). The Graph gate needed the mock's `POST /messages` to accept
  bifrost's raw-MIME `send_raw_message` body (`Content-Type: text/plain` carrying base64
  RFC822, imported as a `$draft` then filed into Sent by the existing send path); it
  previously modeled only the structured send. Both also required the `mdn-small.toml`
  fixture to carry a Drafts role mailbox, since the HTTP-API providers stage the raw MDN
  as a draft (`Email/import` / Graph draft create) before submitting - without it
  bifrost-jmap's `send_raw_message` failed `role_mailbox(Drafts)`-not-found, surfacing
  as `Remote(NotImplemented): unsupported`. All four MDN gates (IMAP, Gmail, JMAP,
  Graph) now pass. A latent finding
  surfaced while building these: a JMAP submitted message keeps its `$draft` keyword
  after
  `EmailSubmission/set` (bifrost's `on_success_update_email` rewrites `mailboxIds` to
  `[Sent]` but never clears `keywords/$draft`), so the sent message shows under both
  `DRAFT` and `SENT` - a bifrost send-fidelity follow-up, out of B5-GATES scope.
- B7. Calendar. Replace the `calendar` crate's per-provider sync with the
  bifrost calendar surface. Carved into TWO bricks by the read/write seam
  (B7a sync, B7b actions), NOT a per-provider cutover series like B3 - see the
  decomposition note below. Needs B1; A7 for DAV (landed). B7a and B7b are
  both done (see the done-notes below); what remains open is the B7c
  follow-up filed out of B7a.

  Decomposition (settled during B7 scoping, read against frozen bifrost
  `0e71226`). B3 was carved four ways because it built a shared change-stream
  consumer with asymmetric per-provider hydrate / write hooks that had to be
  validated one provider at a time. NEITHER condition holds for calendar.
  (1) There is no consumer to build: bifrost's `SyncEngine` (multiplexer,
  change-stream, push reconciler) is mail-only; calendar is a direct PULL
  surface on the `Account` trait (`calendars_list`, `events_in_range` paged,
  `event_get` / `event_create` / `event_update` / `event_delete` / `event_rsvp`
  / `event_search`), with `ObjectType::CalendarEvent` present only in the cursor
  codec, not multiplexed by the engine. (2) The per-provider axis COLLAPSES:
  ratatoskr's calendar today branches four ways on provider string in both
  `sync.rs` (`google_` / `graph_` / `jmap_` / `caldav_..._impl`) and `actions.rs`
  (the `CalendarProvider` enum), and bifrost has already unified all four behind
  the one `Account` calendar surface, so after the rewire there is a single call
  site per operation and nothing to cut over provider-by-provider - a B3-style
  4-way split is meaningless here, not merely unnecessary. Calendar also runs on
  its own `CalendarRuntime` (hourly-kick supervisor, separate from the mail
  resident engine), so B7 rewires the runner body and leaves mail sync untouched.
  Sizing correction: the earlier "~41k LOC Graph" figure was wrong (the whole
  Graph crate is ~8.7k); the calendar protocol layer is ~10k LOC, MOST of it
  deleted rather than rewired (`core/caldav` parse / ical / xml,
  `graph/calendar_sync.rs`, `jmap/calendar_sync/`, the per-provider
  `calendar/{google,graph,jmap}.rs` + `calendar/caldav/`), with the actual
  rewire concentrated in `calendar/sync.rs` (~1k) and `calendar/actions.rs`
  (~0.7k). The app / DB / stores / wire-contract calendar layers stay, exactly
  as the mail consumer kept the DB.

  B7a (calendar read sync) is done and its TODO entry is removed per repo
  convention; B7b's reuse of B7a's id-translation seam, and B7c's dependence
  on B7a's stale-calendar retain-and-skip policy, are both available now. It
  collapsed
  the four per-provider calendar read-sync implementations -
  `google_calendar_*_impl` / `graph_calendar_*_impl` /
  `jmap::sync_jmap_calendar_account` / the CalDAV CTag/ETag machinery in
  `sync.rs` and `caldav/` - onto bifrost's uniform `Account` pull surface
  (`calendars_list` + paged `events_in_range`), reading
  `capabilities().pim_methods.calendars_list` to no-op cleanly on an account
  with no calendar backend. For what it delivered - the new
  `crates/calendar/src/idmap.rs` id-translation seam (stripping bifrost's
  composite `calendar_id::event_id` native-provider `native_id` back to the
  legacy dedup key / write-back id, the moved-not-duplicated
  `make_google_event_id` / `href_synthetic_uid` CalDAV keying, the single
  `event_time_to_epoch` rule unifying all-day / zoned / floating / DST-gap
  epoch semantics on the tz-independent CalDAV convention, and a total mapping
  over bifrost's `#[non_exhaustive]` status/availability/visibility/RSVP
  enums); the new Service-side `build_calendar_account_factory` router
  (`crates/service/src/bifrost/factory.rs`) that makes `calendar_provider`
  genuinely win over the mail provider (fixing a legacy silent short-circuit
  that lost a configured CalDAV calendar on a gmail/graph mail account) and
  builds `CalDavAccountFactory` from the account's dedicated
  `caldav_username`/`caldav_password` columns via Basic auth, not
  bearer-from-mail-OAuth; the rewritten `calendar_sync_account_impl` on an
  injected `now_ms` clock, syncing a rolling `[-365d, +730d]` window plus a
  one-time per-calendar history backfill (four 365-day upsert-only slices to
  -5y, marked by the new `calendars.history_backfilled_at` column) so
  Google/Graph gain history versus their legacy -90d anchor while JMAP/CalDAV
  keep their 5-year fresh-setup coverage; a windowed deletion reconcile on an
  overlap + RRULE-occurrence-intersection predicate
  (`should_delete_absent_candidate` / `recurring_master_intersects_window`) so
  an ended or future recurring series is preserved rather than deleted on mere
  absence from the active window, an empty-pull transient-failure guard, and
  (via bifrost surfacing the per-resource failed set) preservation of a
  CalDAV row whose resource failed to fetch; and a retain-and-skip policy for a calendar
  the current backend no longer lists (excluded from the fetch-target set,
  not reaped - the reap-vs-hide lifecycle is the new B7c follow-up below) -
  read the B7a landing commit. Deleted: `crates/calendar/src/jmap.rs` (whole
  file, read-sync only); the read-sync bodies out of `google.rs` / `graph.rs`
  / `caldav/mod.rs` (their B7b write helpers stay until B7b cuts them); and
  the CalDAV CTag/ETag sync machinery in `sync.rs`. Required four bifrost
  prerequisite side-quests, landed as one commit `be11bbb` (uniform
  exclusive-end `EventTime` across all four backends, full multi-VEVENT
  projection with a recurrence-aware CalDAV range filter,
  `CalendarEvent.reminders` from CalDAV VALARM + JMAP JSCalendar alerts, and
  `Page.failed_ids` surfacing per-resource CalDAV failures), plus a further
  in-loop side-quest (a GCAL harness endpoint redirect + a spec-correct JMAP
  range-filter time split) that advanced the freeze to `a0a18c2` (§ 11); the
  matching `saehrimnir` calendar pull-shape mock coverage is an installed
  external binary, not commit-pinned here. Two findings ride out: a literal
  non-2xx propstat inside a CalDAV 207 still aborts bifrost's whole pull (so
  `Page.failed_ids` only surfaces a resource that fetched 200 but failed to
  tokenize; a transient whole-collection failure is caught by the empty-pull
  guard instead), and a pre-existing saehrimnir bug (CalDAV all-day emitted as
  `X-MICROSOFT-CDO-ALLDAYEVENT` rather than `VALUE=DATE`, which bifrost read
  as timed-UTC) was fixed in passing. Gated by `brokkr check` green, the
  `-p cal` / `-p service` / `-p db` unit gates (dedup-key, event-time,
  recurrence-id, reminder, and stale-unlisted-calendar cases), and the
  per-provider calendar sync-harness round-trip scripts.
  B7b (calendar actions) is done and its TODO entry is removed per repo
  convention. It rewired `crates/calendar/src/actions.rs`'s create / update /
  delete, and added a new RSVP action, onto bifrost's `Account` write surface
  (`event_create` / `event_update` / `event_delete` / `event_rsvp`), collapsing
  the four-way `enum CalendarProvider { Google | Graph | Jmap | CalDav }`
  dispatch and its per-variant `dispatch_create` / `dispatch_update` /
  `dispatch_delete` matches into one bifrost call site per operation. Deleted
  whole: `crates/calendar/src/google.rs`, `graph.rs`, `caldav/mod.rs`,
  `caldav/ical.rs` (the write-side iCal generator - bifrost's CalDAV account
  now generates its own on write), and the `create_event_remote` /
  `update_event_remote` / `delete_event_remote` functions out of
  `crates/jmap/src/calendar_sync/protocol.rs` (the module's non-write record
  types survive, narrowed). `CalendarEventDto` and its `CalendarEventInput`
  alias went with them. `idmap.rs` grew the write direction B7a's seam was
  built to host - inverse mappers (`epoch_to_event_time`, `parse_availability`
  / `parse_visibility` / `parse_rsvp`), the `input_to_event_create` /
  `input_to_event_patch` lowering functions, and `strip_event_id` - and
  `event_id_for_writeback` (landed inert by B7a) got its first caller.

  The crate-boundary problem - `cal` cannot call the Service's
  `build_calendar_account_factory` without a circular dep - is resolved by an
  injected `CalendarAccountOpener` trait: `action-types` gained a
  `bifrost-types` dependency to hold `pub opener: Arc<dyn
  CalendarAccountOpener>` on `CalendarActionContext`, and the Service's
  `ServiceCalendarAccountOpener` (`cal_actions/mod.rs`) wraps the factory,
  returning the account's `ProtocolKind` alongside the opened `Arc<dyn
  Account>` so the write path never re-parses a provider string. One adopted
  behavior change: calendar-provider routing now follows the factory's
  richer precedence rule (`calendar_provider == "caldav"` OR `provider ==
  "caldav"` with a non-empty `caldav_url`, else `MailProviderKind::parse`)
  rather than the legacy dispatcher's plain string match, which disagreed on
  the edge case of `provider == "caldav"` paired with a mail-provider
  `calendar_provider`. Capability gating mirrors the read path
  (`pim_methods.event_create` / `.event_update` / `.event_delete` /
  `.event_rsvp`), degrading to `LocalOnly` (create) or `Failed`
  (update/delete/rsvp) without a provider call when an account's calendar
  backend cannot write. The consumer-side etag/`If-Match` writeback contract
  is retired - concurrency becomes each bifrost `Account` impl's
  responsibility, advertised via `capabilities().mutation.concurrency` -
  named honestly as an intentional last-write-wins downgrade for Gmail
  (`MutationConcurrency::None`) and CalDAV delete (bifrost sends no
  `If-Match` there), not a lossless hand-off. A previously-undiscovered
  app-side bug rides along as an in-scope fix: an all-day save lowered both
  `start_time` and `end_time` from the same `draft.start_date`, producing a
  zero-day span; `build_wire_input` now derives an exclusive one-day-minimum
  end for all-day events.

  RSVP landed as a fourth `WireCalendarOperation::RsvpEvent { event_id,
  response: RsvpResponse }` wire variant (a typed, fallible `accepted` /
  `declined` / `tentative` enum, not a defaulting string parse), a new
  `cal::actions::rsvp_calendar_event` domain function, and a `run_one` arm -
  all as a one-op plan, closing this document's architecture-note question
  (`reference/architecture.md`'s calendar-action-pipeline section) of whether
  RSVP would need an N-op plan shape: it does not. RSVP updates only the
  user's own participation status and does NOT email the organizer (bifrost's
  `event_rsvp` trait has no delivery flag, and Graph posts `send_response:
  false`), so the calendar design doc's "Email organizer" checkbox stays out
  of scope pending a bifrost trait extension. The app gained RSVP
  Accept/Decline/Tentative controls in both the event-detail popover and the
  reading-pane's meeting-invite card. The previously-missing ICS `UID` parser
  (`common::email_parsing::extract_imip_uid`) now persists
  `messages.meeting_invite_uid` at hydration time, so the reading pane can
  resolve `(account_id, uid)` against the synced calendar cache and either
  dispatch the same `RsvpEvent` plan on a hit or render disabled with a "sync
  this calendar to respond" affordance on a miss (ambiguity - multiple synced
  events for one UID - treated as a miss, not a guess). Emitting a standalone
  iMIP REPLY email for an invite that never resolves to a synced event stays
  out of scope, filed as a mail-send-surface concern (B4/B5), not a calendar
  action.
  - RSVP round-trip gates: the four
    `crates/app/tests/sync-harness/{gcal,graph,jmap,caldav}-calendar-rsvp-writeback.lua`
    scripts sync an invite with the authed user as an attendee, RSVP
    Accept through the `RsvpEvent` op, and prove the change on three
    planes - the local `rsvp_status` column (now surfaced through
    `TestQueryDbState` / `TestDbCalendarEventRow`), the provider request
    log (pinned per-op counts), and a load-bearing read-back: a follow-up
    sync must show the self attendee's persisted participation status flip
    to accepted in `attendees_json`, so a mock that recorded nothing fails.
    Unblocked by the saehrimnir side-quest that added a per-attendee
    participation-status round-trip across all four providers (Graph
    `attendees[].status.response`, Google `attendees[].responseStatus`,
    JMAP `participants/{id}/participationStatus`, CalDAV `PARTSTAT` plus a
    schedule-outbox POST) and the Graph calendar-scoped event routes
    (`/v1.0/me/calendars/{cal}/events/{id}` GET/PATCH/DELETE + POST
    accept|decline|tentativelyAccept). The Graph action-writeback gate's
    update/delete request-log needles were moved onto those calendar-scoped
    routes at the same time. Two harness realities are pinned, not bugs:
    (a) the mock's `log_request` middleware records every request, so a
    mutating handler that also logs (Google RSVP PATCH) shows a log count of
    2 for a single wire PATCH; (b) bifrost-jmap resolves the RSVP self
    participant from its `self_emails` set, which saehrimnir's JMAP session
    leaves empty (no participant-identity surface), so the JMAP RSVP gate
    uses a single-attendee fixture (`calendar-rsvp-jmap.toml`) where
    bifrost's sole-non-owner-attendee fallback applies; gcal/graph/caldav
    match the self attendee by email and share the two-attendee
    `calendar-rsvp-small.toml`.
  - bifrost follow-up (CalDAV `event_rsvp` capability): bifrost-caldav's
    capabilities hardcode `pim_methods.event_rsvp = true` regardless of what
    RFC 6638 scheduling the server actually advertises. A scheduling-less
    CalDAV server (no `schedule-outbox-URL` / no calendar-user-address) would
    pass ratatoskr's capability gate and then fail `event_rsvp` at runtime
    when bifrost cannot post the iTIP REPLY. ratatoskr's `ActionOutcome::Failed`
    mapping covers the UX (the RSVP surfaces as failed, not a silent no-op),
    so this is not a ratatoskr defect; the durable fix is bifrost deriving the
    `event_rsvp` flag from discovery rather than hardcoding it.

  Gated by `brokkr check`; the `-p cal` idmap inverse-mapper and round-trip
  unit tests; the `-p service` `cal_actions` unit tests (outcome mapping, the
  `run_one` RSVP arm, the factory `Ok(None)` no-op); the `-p app`
  `build_wire_input` all-day exclusive-end unit tests; a `-p cal` unit test
  for `rsvp_calendar_event`'s unsynced-event `Failed` path; and the eight
  per-provider calendar action-writeback and RSVP sync-harness round-trip
  scripts, with per-op provider-request counts pinned in the harness
  assertions as the performance instrument (no `brokkr sync-bench` regression
  surfaced). Read the B7b landing commit for the full accounting.
  - B7c. Stale-calendar reap-vs-hide lifecycle. B7a's policy for a calendar
    the current backend no longer lists is retain-and-skip: it is kept
    and simply excluded from sync (`sync.rs::resolve_sync_targets`), so its
    cached events render indefinitely and it never disappears from the
    sidebar. B7c makes unlisting observable and eventually reaps it. Add
    `calendars.unlisted_since INTEGER` (nullable); on a SUCCESSFUL
    `calendars_list()` that omits a previously-known calendar, stamp
    `unlisted_since = now` (and clear it back to NULL when the calendar
    re-appears in a later list). While stamped, suppress the calendar in the
    sidebar (hidden, not deleted, so a transient list omission does not lose
    the user's events). After N consecutive unlisted SUCCESSFUL runs
    (suggest ~7 days' worth at the hourly cadence, i.e. ~168 runs, or a
    wall-clock `now - unlisted_since >= 7d` check to be robust against
    missed kicks), REAP the calendar: cascade-delete its `calendar_events`,
    `calendar_attendees`, `calendar_reminders`, `caldav_event_map` rows and
    the `calendars` row itself. Only successful list runs count - a failed
    `calendars_list()` must neither stamp nor advance the counter, so a
    server outage cannot reap a live calendar. This is deferred out of B7a
    because it needs a new schema column and B7a already spent its one
    adjudicated schema addition (`history_backfilled_at`); B7c is the natural
    home for the second calendar-schema change. Gated by a harness script
    that lists a calendar, drops it from a later list, and asserts hide-then
    -reap across the threshold.

  Two questions the B7 decomposition parked were resolved by B7a's ground
  survey, neither blocking the decomposition: (i) a CalDAV asymmetry -
  bifrost-caldav DOES emit `CursorScope::Type(CalendarEvent)` through a
  `changes_stream` while the HTTP providers expose calendar only via the pull
  surface; B7a drives all calendar uniformly through `events_in_range` (per
  this document's § 2 first principle) and treats the CalDAV change-stream as
  an optional future optimization or a
  bifrost side-quest, never a consumer-side special-case; (ii) iMIP scope -
  the email-embedded ICS path (`common/email_parsing.rs`: `has_meeting_invite`,
  `method` REQUEST / REPLY / CANCEL, RSVP-from-reading-pane) is parsed
  app-side today and routes through `event_rsvp`; it is a WRITE concern, so it
  stays with B7b, not B7a.
- B8. Contacts. Replace Google People, Graph contacts, JMAP contacts, and
  Google other-contacts sync with the bifrost contact surface. Needs B1; A7 for
  DAV (landed). B8 is done and its TODO entry is removed per repo convention;
  what remains open is the B8-groups follow-up filed out of it (see below).

  It collapsed the four per-provider contact READ implementations -
  `crates/gmail/src/contacts/` (People main + otherContacts),
  `crates/graph/src/contact_sync.rs`, `crates/jmap/src/contacts_sync.rs`, and
  the dormant unwired `crates/core/src/carddav/` parser - onto ONE
  provider-agnostic pull, `run_contact_pull`
  (`crates/service/src/bifrost/contacts/pull.rs`), driven off the resident
  auxiliary loop (`resident.rs::run_aux_pass`) rather than the mail
  change-stream consumer: contacts stay a pull-at-cadence surface, mirroring
  B7a's calendar decomposition. The pull pages the account-wide
  `contacts_list(None)` route (not a per-address-book loop, which would
  amplify Google's per-group fan-out into a near-full-corpus refetch per
  group), maps each `ContactCard` through a new pure `contact_write_rows`
  mapper (`crates/service/src/bifrost/contacts/map.rs`) keyed on a
  `ContactProjection` (`PackSecondEmail` for JMAP/Google/CardDAV vs
  `FanOutPerEmail` for Graph, preserving Graph's one-row-per-email shape
  across the cut), and snapshot-reconciles against a NEW unified
  `contact_claims(account_id, source, server_id, email, address_book_id,
  corpus)` remote-claim ledger that replaces the four per-provider
  `graph_contact_map` / `google_contact_map` / `google_other_contact_map` /
  `carddav_contact_map` tables (and drops `graph_contact_delta_tokens`
  outright - the cursor is now bifrost's `Page::next_cursor`) in a v100
  schema edit (`crates/db/src/db/schema/03_contacts.sql`), per the
  migrations file's pre-release no-v101 policy. The claim ledger, not
  `contacts.server_id`, is what preserves the pre-existing cross-provider
  dedup guard (`contacts.email` is globally UNIQUE; a materialized row can be
  claimed by several `(account, source, server_id)` tuples, and a row is only
  deleted once no claim references its email). The reconcile excludes a
  page's `failed_ids` from the "absent -> retire" set (a transient hydration
  failure is not a remote delete) and only honors a clean, fully-enumerated
  empty result as a real delete-all. Google `otherContacts` keeps writing
  `seen_addresses` (`source='google_other'`), not `contacts`, preserving
  `local_observed` rows. Per-source pull cadence is preserved (not
  flattened to one interval): a DB-backed `(account_id, source)` cycle
  counter in the `settings` table (surviving a Service restart, unlike an
  in-memory counter) pulls JMAP every resident-aux pass, Graph and CardDAV
  every fifth, and Google every twentieth - matching each provider's
  pre-cut effective freshness. CardDAV contacts are lit up for the first
  time (the one capability B8 adds, not merely preserves): the IMAP account
  factory (`crates/service/src/bifrost/factory.rs::build_imap_factory`) now
  calls `.with_carddav(..)` whenever the account carries CalDAV discovery
  settings (`caldav_url` + either the shared OAuth bearer source or
  `caldav_username`/`caldav_password` Basic credentials) - reusing the
  existing CalDAV configuration surface rather than adding a separate DAV
  credential path - so a plain IMAP account with no DAV config stays
  contact-`Unsupported` and the pull no-ops on it. Write-back
  (`crates/service/src/actions/contacts.rs::dispatch_write_back` /
  `dispatch_delete`) collapsed its four provider arms (including the CardDAV
  stub that used to return `ActionError::not_implemented`) into one
  `ContactPatch` call through `engine.contact_update` / `contact_delete`; GAL
  (`crates/service/src/handlers/gal.rs`, `crates/core/src/contacts/gal.rs`)
  dropped `fetch_graph_gal` / `fetch_google_gal` for one
  `engine.directory_search` call, keeping `gal_cache_age` and the `gal_cache`
  table unchanged. `crates/graph/src/group_sync.rs` (Exchange distribution
  list / M365 group sync) deliberately STAYS - bifrost's
  `address_books_list` models contact folders, not directory groups, so it
  has no bifrost equivalent and is carved out to the B8-groups item below
  rather than forced into the shared model. Gated by `brokkr check`, the
  `-p service` `contact_write_rows` and `contact_pull_reconcile` unit tests,
  and the sync-harness suite under `crates/app/tests/sync-harness/contacts/`
  (per-provider pull round-trips, delete-all-vs-transient-empty, the Google
  otherContacts corpus, cross-provider dedup, the Graph fan-out
  identity-equivalence check, per-provider write-back including CardDAV, and
  GAL) plus the `contacts_cadence` sync-bench request-budget gate, replacing
  the retired `jmap-contacts-initial.lua` / `graph-contacts-{initial,
  incremental}.lua` / `people-contacts-{initial,incremental,
  writeback-delete}.lua` scripts. Read the B8 landing commit for the full
  accounting.
  - B8-groups. Exchange distribution / M365 group sync onto a bifrost groups
    surface. Carved out of B8 (disposition (b), not attempted there):
    `crates/graph/src/group_sync.rs` enumerates `/groups`, classifies
    Unified (M365) vs distribution-list groups, and fetches transitive
    membership into `contact_groups` / `contact_group_members`, and bifrost
    has no directory-group equivalent to `address_books_list` today. Needs a
    bifrost groups surface (new bifrost work, not yet a side-quest) before
    this can rewire uniformly; until then `group_sync.rs` and
    `sync_exchange_groups` stay in place, Graph-specific.
- B9. Attachments plus cloud attachments. LANDED - both attachment seams now
  dispatch through the resident bifrost `SyncEngine` instead of per-provider
  `ProviderOps`. INCOMING: `attachment.fetch` and the prefetch worker pull
  bytes through the new `AttachmentByteSource`
  (`crates/service/src/bifrost/attachment.rs`) - HTTP providers (Gmail /
  Graph / JMAP) reconstruct a `BlobHandle` from a new `attachments.blob_id`
  column (the verbatim bifrost blob id, persisted by the consumer's
  `hydrate.rs::build_consumer_row`) and call `engine.open_blob`; IMAP has no
  server-side blob handle (only a persisted `part_id`), so it drains
  `engine.open_raw_rfc822` and MIME-re-parses the part. The prefetch
  worker's old dedicated IMAP session (LOGIN + SELECT +
  `fetch_attachment_on_selected`) is gone - `process_imap_batch` now groups
  a folder batch by message and hydrates each message's RFC822 at most
  once, satisfying every one of that message's queued attachments from the
  single parse (`imap-folder-batch-session-reuse.lua` rewritten to assert
  no dead per-batch session survives; the new
  `imap-attachment-multipart-single-download.lua` gate proves the
  once-per-message invariant on a real multi-attachment message). A Graph
  reference / non-byte-stream attachment (`SyncEvent::Warning(BlobNotByteStream)`)
  surfaces as a distinct `AttachmentByteError::NotByteStream` rather than
  being cached as an empty file. Errors carry a typed `AttachmentByteError`
  (not `ServiceError`), so the prefetch worker's `SkipReason` circuit-breaker
  classification survives the cut. OUTGOING: the hand-rolled
  `crates/gmail/src/gdrive.rs` and `crates/graph/src/onedrive.rs` (resumable
  upload + share-link code with no production caller) are DELETED along with
  the `cloud_attachments` table's outgoing upload-state writers and
  `core::cloud_attachments::UploadStatus` / `supports_cloud_upload` /
  `LARGE_ATTACHMENT_THRESHOLD`; the incoming cloud-link carve-out
  (`detect_cloud_links`, `enrich_*_link`, `CloudProvider`,
  `insert_incoming_cloud_links_sync`) is kept whole, unwired as before. A
  pinned `AttachmentByteSource::host_large_attachment` forwards to bifrost's
  capability-gated `Account::host_attachment` through a new
  `SyncEngine::host_attachment` passthrough (B9-SQ, bifrost commit
  `1769367`, advancing the freeze from `cf024ab` - § 11); it has no caller
  yet (`#[allow(dead_code)]`), pinning the surface for a future compose
  item rather than wiring a UX that was never present. Read the B9 landing
  commit for the full accounting.
- B10. Search. CLOSED - audit found no provider-side mail-search surface to
  rewire. The seam's only live occupant, GAL directory enumeration, already
  migrated onto `engine.directory_search` in B8; mail search itself is, and
  stays, the local Tantivy plus smart-folder SQL pipeline (`core::search_pipeline`),
  provider-free by design. Two invariant gates pin the seam shut:
  `local_search_links_no_provider_surface` (`crates/search/tests/`) locks the
  `search` crate's manifest against a provider/bifrost dependency, and
  `search_pipeline_routes_local_only` (`crates/core/tests/`) walks the `rtsk`
  and `service` source for any provider/`SyncEngine` search-shaped call
  outside the single allow-listed B8 GAL call. IMAP's sync-time
  `search_folder` / `uid_search` enumeration is unrelated (B3/B15 disposition,
  not a user-search surface); server-side filters/Sieve are the separate B11
  item.
- B11. Server-side filters / Sieve. CLOSED - deleted the dead hand-rolled
  `crates/jmap/src/sieve.rs` duplicate, while leaving the demo Filters settings
  tab and dormant local `sync/filters.rs` engine as app-level surfaces. The
  capability-dispatched bifrost `filter_*` surface is pinned behind the
  no-caller `ServerFilterSurface` (`#[allow(dead_code)]`), mirroring B9's
  attachment pin; `server_filters_route_through_bifrost` locks the route and
  duplicate deletion down.
- B12. Shared mailboxes plus public folders. Rewire
  `ViewScope::SharedMailbox` / `PublicFolder` onto bifrost scopes. Needs A5.
- B13. Identities, signatures, vacation, quota. Rewire onto the bifrost
  settings surface. Needs B1.
- B14. Account construction, discovery, verify. Use `AccountFactory::open` as
  the connection test; keep the five-stage discovery and OAuth authorization.
  Needs B1.
- B15. Deletion and collapse. Remove the four provider crates, `provider-sync`,
  `common`'s provider surface, the external `jmap-client` dep, and the
  workspace members; remove any transitional scaffolding. The final green cut.
  Needs all above.

  This explicit list is a floor, not the full scope. The § 1 maximal-integration
  rule (no parallel hand-rolled or duplicated dependency surviving alongside a
  bifrost equivalent) is stronger than this enumeration, so B15 must run a
  mechanical dependency-and-module audit of the whole workspace - every crate's
  `Cargo.toml` plus its module tree - and delete every bifrost-covered equivalent
  it finds, not only the named targets. Known instance to confirm in that audit:
  `crates/service/Cargo.toml` still carries
  `bifrost-jmap = { path = "/home/folk/Programs/jmap-client/crates/jmap" }` (the
  out-of-tree `jmap-client` checkout, used by the JMAP-specific contact action
  handlers). That dependency, and any others the audit surfaces, retire here -
  subject to the § 9 caveat that retiring the external `jmap-client` must not
  strand bifrost-jmap (confirm bifrost-jmap's own internal JMAP dependency first).
- B16. Reference-doc reconciliation. Update `reference/architecture.md`,
  `AGENTS.md`, and the crate map. Bundled with B15 per repo convention (never a
  standalone markdown commit).

Estimated scope: ~8 bifrost specs plus ~16-20 ratatoskr specs.

## 8. Sequencing and green-tree strategy

- Track A lands first. Each A-item lands as it unblocks its B dependents; A1
  is the literal first task.
- Within Track B, two trunk lines: B1 to B2 to B3 (sync), and B1 to B4
  (actions). B5 through B14 are branches off them. B15 closes.
- Each spec is one coherent, fully intrusive landing, kept or reverted on its
  gate results. Where a subsystem cannot cut over atomically across all four
  providers in one commit, the spec author may stage via a short-lived
  ratatoskr-internal adapter deleted within that same spec's final landing -
  never a runtime or env switch, never a probe left in the tree. Per the spec
  methodology, complete-but-unorderable is a failed spec; the exact green-tree
  ordering is pinned per spec, not here.

## 9. Risks

- The sync inversion (B3) is the highest-risk landing: persistence ownership
  moves, the cursor schema changes, and the application sync layer's input
  contract must be preserved exactly. Mitigate by freezing the
  `ProviderParsedMessage` / `SyncResult` contract as the seam and validating
  that threading and bundling outputs are unchanged across the cut. "Validate"
  here means named behavioral gates, not a compile check: B3 (and every other
  sync-touching spec) must pin explicit `brokkr service-test` / sync-harness runs
  plus the relevant `brokkr sync-bench` so a compile-only replacement cannot pass
  the gate. See § 10 for the workspace-wide gate requirement this is an instance
  of.
- Calendar (B7) is a large rewire but lower structural risk than B3: bifrost
  exposes calendar as a direct `Account` pull surface, not the mail
  change-stream, so there is no persistence inversion and no consumer to build,
  and the per-provider axis collapses to one call path. It is carved by the
  read / write seam (B7a / B7b), not per-provider (see § 7). Depends on bifrost
  calendar maturity (high on native providers; A7 for DAV, landed).
- Shared mailboxes and public folders (A5 / B12) is the largest bifrost-side
  unknown; verify bifrost's current support before sizing.
- Token rotation (A1) gates everything; it is the first task in the whole
  project.
- Retiring the external `jmap-client` dependency must not strand bifrost-jmap;
  confirm bifrost-jmap's own internal JMAP dependency first.

## 10. Methodology

This document is the source TODO the spec-loop consumes. Each B-item, and each
A-item in the bifrost repo, becomes one technical-implementation-spec, run
through the orchestrate.md seven steps. Items are processed serially, the tree
green at every boundary, nothing deferred.

Behavioral gates are mandatory, not optional. Because this migration swaps the
entire provider stack underneath unchanged application behavior, a green
`brokkr check` is necessary but not sufficient - it proves the new code
compiles and passes unit tests, not that real provider sync still behaves. Every
spec that touches sync, actions, calendar, or contacts must pin, in its gate
section, the explicit behavioral gates it has to pass: the relevant
`brokkr service-test` scripts, the sync-harness runs (real provider sync against
the `saehrimnir` mock servers - see the harness doc), and `brokkr sync-bench`
where performance is in scope. A spec the loop can satisfy with a compile-only
replacement is under-gated and must be rejected at review.

## 11. Bifrost source and dependency paths

Bifrost lives in two places relative to this repo's top-level folder, and they
serve two distinct purposes - do not conflate them:

- In-tree working copy: `./research/bifrost`. It serves two roles. First, the
  reading-reference: where agents inspect bifrost source - to verify a Track A
  item against bifrost's current shape, to read the `Account` / `AccountError` /
  `SyncEngine` surface a Track B spec is written against, or to confirm a type
  signature before speccing. Spec authors and reviewers read here; it is the
  ground a bifrost-facing spec is judged against. Second, the staging area:
  side-quest edits to bifrost (§ 2) are made HERE, by an Opus agent confined to
  this folder, then committed by the orchestrator and promoted to the dependency
  path by `bash scripts/bifrost.sh`. `./research/saehrimnir` works the same way for
  the mock server, paired with `bash scripts/saehrimnir.sh`.
  `./research/bifrost/reference/` holds per-crate and per-protocol
  quick-reference sheets (`net.md`, `sync.md`, `error-model.md`, `jmap.md`,
  `imap.md`, `graph.md`, `google.md`, `smtp.md`, `caldav.md`, `carddav.md`,
  `sasl.md`) - start there for a crate's surface, then drop into the source.
- Dependency path: `../bifrost/`. This is what Cargo `path = "..."` deps resolve
  to. The path deps on the bifrost crates (added by B1, extended by later items)
  point at `../bifrost/`, not at the reading-reference copy.

A spec that touches bifrost cites `./research/bifrost` as required reading for
its implementers and reviewers, and any spec that adds or changes a bifrost
dependency pins the `../bifrost/` path explicitly.

Track A is complete at commit `ff56478` (the A8-closing commit). The frozen
reference was `a0a18c2` through B7a; B11-SQ (below) has since advanced it to
`bc97132`, the current frozen reference. Twelve bifrost side-quests landed between the
A8-closing commit and `0e71226` (see § 2's side-quest protocol), and both
`./research/bifrost` and `../bifrost` were re-synced together to each in turn; the
B7a calendar side-quests then carried the freeze from `0e71226` on to `be11bbb`
and then to `a0a18c2` (closing notes below). First `aa9172d` ("sync:
make backfill checkpoints consumer-ack-deferred"), surfaced during B3's spec
review, made backfill checkpoints consumer-ack-deferred for at-least-once
cold-start hydration. Then `dc670ef` ("sync: expose read-only hydration
passthrough on SyncEngine"), surfaced during B3a-cut-jmap's spec review, added a
read-only hydration passthrough cluster (`get_stream` / `message_hydrate` /
`thread_hydrate` / `open_raw_rfc822` / `open_blob` / `open_blob_range`) so the
change-stream consumer can reach the engine-private `Arc<dyn Account>` to hydrate
a broadcast `Change` into real rows - the blocker B3a-cut-jmap § 4.2 named.
Then two further side-quests surfaced during B3a-cut-jmap's step-7 gate
validation, both in `run_backfill_orchestrator` (`crates/sync/src/engine.rs`):
the OpenPages loop treated a short inventory page (`seen < chunk`, the common case
when a JMAP server caps `Email/query` below the requested chunk) as
end-of-inventory and dropped the rest of a bulk backfill - fixed to terminate on
an empty page; and the orchestrator never read `get_backfill` from the
`CheckpointStore`, so a one-shot re-attach re-walked backfill from page 0 every
kick (inflating delta `Email/query` and failing the steady-state gates) - fixed to
resume/skip from the persisted backfill checkpoint. Both landed and advanced the
freeze to `ae73e92`. Two further per-cut side-quests advanced the freeze again:
B3a-cut-graph added Graph `importance` to the typed `hydrate_select` (advancing
to `7c576bdd`), and B3a-cut-gmail added the Gmail mock-redirect seam to
`GoogleAccountFactory` (`from_token_source_with_api_base` /
`from_access_token_with_api_base`, advancing to `002e7b9`). (Note: separate from
these bifrost commits, B3a-cut-jmap also
required several `saehrimnir` mock extensions - JMAP `Thread/get`, `ContactCard/get`,
and per-`accountId` `Email`/`Mailbox` state - which are an installed external
binary, not commit-pinned here.) B3a-cut-imap required NO bifrost change - the
freeze stays at `002e7b9` because bifrost's IMAP CONDSTORE/QRESYNC behavior was
already correct - so the side-quest count holds at six; its mock work was entirely
in `saehrimnir`, whose IMAP mock gained real CONDSTORE/QRESYNC support (parse
`SELECT (CONDSTORE)`, return `HIGHESTMODSEQ`, honor `FETCH ... (CHANGEDSINCE ...)`,
emit QRESYNC `VANISHED`), again an installed external binary, not commit-pinned
here. B3b advanced the freeze a seventh time, to `db34ab4`: its push-gate work
surfaced a JMAP-WebSocket `StateChange` parser bug in `client_ws.rs` (a double
`@type` tag that rejected conformant RFC 8620/8887 frames), fixed upstream; B3b's
mock work was a separate `saehrimnir` push side-quest (JMAP WebSocket push frames,
a Gmail Pub/Sub source plus `users.watch` / `users.stop`, and Graph
`POST /subscriptions` + a loopback notification POST), again an installed external
binary, not commit-pinned here. B3c required NO bifrost change - every
control/recovery surface it consumes (`account_control_stream`, `resume_account`,
the `AccountControl` + `SyncEvent::Terminated` broadcasts) was already public at
`db34ab4` - so the freeze stays at `db34ab4` and the side-quest count holds at
seven; B3c's only test additions were in-tree (a `ForceTerminated` consumer hook
+ `resident_redrive_*` probe telemetry) plus a `saehrimnir` budget-exhaustion
affordance for the real-engine pause gate, again an installed external binary,
not commit-pinned here. B4-SQ (B4a's bifrost prerequisite) advanced the freeze an
eighth time, to `75cf810`: a `SyncEngine` mutation passthrough cluster - the
single-object conveniences (`set_starred` / `set_read` / `apply_label` /
`remove_label` / `set_importance` / `add_to_container` / `remove_from_container` /
`set_keyword` / `set_label_membership` / `set_category` / `container_*`) dispatched
direct through `live_account`, plus `bulk_move` / `bulk_destroy` routed through the
same idempotency / read-back pipeline `bulk_set_flags` uses - mirroring the
`dc670ef` read-only hydration passthrough, so the action pipeline drives
object-level mutations through the resident engine without the engine-private
`Arc<dyn Account>`. B4a's mock work was a separate `saehrimnir` mutation side-quest
(Graph `@odata.etag` + `If-Match` on PATCH / move / DELETE including inside
`$batch`, Gmail `messages.modify` / `batchModify` / `batchDelete`, and IMAP
RFC 6851 `UID MOVE`), again an installed external binary, not commit-pinned here.
B5-SQ (B5's bifrost prerequisite) advanced the freeze a ninth time, to `8ea29b6`:
the `SyncEngine` compose passthrough cluster (`send_message` / `draft_create` /
`draft_update` / `draft_discard` / `draft_send` / `cancel_scheduled_send` /
`reschedule_send` + the non-async `account_capabilities` accessor), a real new
`send_raw_message` `Account` trait method (no raw-send existed) implemented across
gmail / graph / jmap / imap for the pre-assembled RFC 8098 MDN, plus two send
feature-preservation type additions - `content_id` on `AttachmentInline` and
`request_read_receipt` on `SendRequest`. B5's mock work was a separate `saehrimnir`
send side-quest (SMTP `SIZE` raise + submission header projection, JMAP
`Blob/upload` + `Email/import` + `EmailSubmission/set`, Gmail `messages.send`, Graph
send-to-Sent, scheduled-send acknowledgement), again an installed external binary,
not commit-pinned here. B6-SQ (B6's bifrost prerequisite) advanced the freeze a
tenth time, to `60e979b`: a `SyncEngine::containers_list` read passthrough
(the read companion to the existing container-mutation cluster, resolving through
`live_account` and bailing `AccountNotAttached` up front), an optional
`style: Option<ContainerStyle { color_bg, color_fg }>` field and a `system: bool`
flag on `Container` / `Label` - both Gmail-populated (the `style` carries the
Gmail user-label color, the `system` flag the `label_type == "system"` split so
the system-label-as-folder partition survives the cut; Graph / JMAP / IMAP leave
them unset) - and an optional `style` argument on `container_create` /
`container_rename` for the recolor path. B6's mock work was a separate
`saehrimnir` container-CRUD side-quest, again an installed external binary, not
commit-pinned here. Two further side-quests landed during the B5/B6 gate
cleanup. The eleventh advanced the freeze to `bea56e8`: the `gmail-initial`
sync gate ingested zero messages because the cold-start backfill orchestrator
(`crates/sync/src/engine.rs`) broadcast its first inventory page onto the
per-account `tokio::broadcast` channel BEFORE the consumer could subscribe (the
consumer can only `account_changes_stream` after `attach` inserts the slot), so
a late subscriber started at the ring tail and never saw the page - a latent
data-loss race that bit Gmail's single fast `CursorScope::Account` pass while
JMAP's multi-page walk happened to survive it. Fixed by parking the orchestrator
on `wait_for_real_subscriber` before broadcasting, exactly as
`run_deferred_inventory_establishment` already did for the fusion path. The
twelfth advanced the freeze to `0e71226` (B6-FIX): a `ContainerRights` type
(nine `Option<bool>` JMAP `myRights` members) plus `rights` / `is_subscribed`
fields on `Container`, populated by bifrost-jmap's `containers_list` from
`Mailbox.myRights` / `isSubscribed` (other providers leave them `None`),
restoring the per-folder ACL + subscription state the legacy JMAP folder sync
wrote (consumed by `navigation::rights_from_folder`, gating shared-mailbox
submit). The matching `saehrimnir` work for the B5-GATES MDN gates (JMAP
whole-message raw `/jmap/download` + Graph raw-MIME `text/plain` send) landed in
the mock, an installed external binary, not commit-pinned here. A lateral
finding surfaced during the backfill-race investigation and is NOT yet fixed:
the `LiveSupersedes` cold-start de-dup set in `crates/sync/src/engine.rs` is
constructed and threaded onto the backfill handle but nothing ever calls
`.add()`, so `filter_supersedes` is a no-op and cold-start backfill can
double-emit an object the live `changes_stream` already announced. It did not
cause the zero-ingest bug (an empty set filters nothing) and the gates pass, but
it is a real latent correctness gap worth a future bifrost side-quest.

The B7a calendar side-quests then advanced the freeze to `be11bbb`. Two
bookkeeping notes ride with it. First, the live `../bifrost` / `./research/bifrost`
checkouts had already advanced past `0e71226` to `d3f9cca` before B7a began - an
advance this narrative does not itemise (a known § 11 reconciliation gap), and the
B7 decomposition note (§ 7) had cited `0e71226` / `d3f9cca` while B7a was in
flight, since re-pinned to the final freeze below. Second, the calendar work
landed as one bifrost commit `be11bbb` ("calendar: uniform exclusive EventTime,
multi-VEVENT, reminders, failed-set"): an exclusive all-day `EventTime` contract
uniform across caldav / google / jmap / graph, full multi-VEVENT projection with a
recurrence-aware caldav range filter, `CalendarEvent.reminders` from caldav VALARM
+ jmap JSCalendar alerts, and `Page.failed_ids` surfacing per-resource caldav
failures; the matching `saehrimnir` pull-shape calendar mock coverage landed as an
installed external binary, not commit-pinned here. Two findings ride out for the
B7a consumer work: a literal non-2xx propstat inside a caldav 207 makes bifrost's
`parse_multiget_report` return `Err` and abort the whole pull, so `Page.failed_ids`
only surfaces a resource that fetched 200 but failed to tokenize (a transient
whole-collection failure is caught by the empty-pull deletion guard, not
`failed_ids`); and a pre-existing saehrimnir bug (caldav all-day emitted as
`X-MICROSOFT-CDO-ALLDAYEVENT` rather than `VALUE=DATE`, which bifrost reads as
timed-UTC) was fixed in passing. A further in-loop B7a side-quest, surfaced during
implementation, advanced the freeze a fourteenth time, to `a0a18c2` ("calendar:
GCAL harness endpoint redirect + spec-correct JMAP range-filter time"): a Google
`calendar_api_base()` honoring `RATATOSKR_TEST_GCAL_ENDPOINT` across all five
Google Calendar URL sites (calendarList, events range, create, search, event_url),
mirroring the existing Gmail/Graph harness-redirect seams; and a JMAP
`jmap_utc_filter_time` fix for the `CalendarEvent/query` `after`/`before` range
filter - per draft-ietf-jmap-calendars-26 § 5.11.1 those bounds are `LocalDateTime`
(bare wall-clock, no zone, no trailing `Z`, interpreted in the query's `timeZone`
argument), so the filter-only helper UTC-normalizes an offset-bearing RFC 3339
bound and renders it bare rather than the UTC-with-`Z` serialization an earlier
implementation pass had bent bifrost into emitting to satisfy the then-strict mock;
the matching fix lives in the `saehrimnir` mock (bare JSCalendar date-time accepted
in `after`/`before`), not commit-pinned here. `a0a18c2` is B7a's final freeze -
the fourteenth bifrost side-quest overall.

B9-SQ advanced the freeze a fifteenth time, from `cf024ab` (the B8-closing
commit the B9 item's ground survey was authored and gated against; the
intervening B7b and B8 freeze advances between `a0a18c2` and `cf024ab` are a
known § 11 reconciliation gap this landing does not close, per that item's
own note) to `1769367` ("sync: expose host_attachment passthrough on
SyncEngine"): a pure additive `SyncEngine::host_attachment(account_id, bytes,
meta)` forwarder mirroring the existing passthrough clusters, resolving
through `live_account` (bailing `AccountNotAttached` up front) and forwarding
to `Account::host_attachment` (already implemented for Google -> Drive and
Graph -> OneDrive at `cf024ab`). No `saehrimnir` mock work was needed - B9
wires no compose-side caller, so the pinned `host_large_attachment` surface
has only compile-and-capability-dispatch unit coverage, no round-trip gate.
`1769367` was the previous frozen reference.

B11-SQ advanced the freeze a sixteenth time, from `1769367` to `bc97132`
("sync: expose server-filter passthroughs on SyncEngine"): five additive,
capability-dispatched `SyncEngine::filter_*` forwarders for list, create,
update, delete, and validation. They resolve through `live_account`, return
the flattened engine `Error`, and preserve account failures inside
`Error::Account`.

Each Track B spec records, in its ground
survey, the exact `../bifrost` commit it was authored and gated against, and
`../bifrost` stays frozen at that commit for the full duration of the item -
including the hours a step-4 implement run can take. This is load-bearing: the
dependency compiles from source, so a `../bifrost` that is red OR merely
mutating underneath an in-flight ratatoskr step makes every ratatoskr gate
meaningless, and a later bifrost change would silently shift the surface the
spec was built against.

## 12. Review reconciliation

This plan was reviewed twice before the loop launched (R1 / opus, R2 / codex);
both reports live under `docs/bifrost-migration/`. Every valid finding from both
is now folded into the sections above: the stale commit pin (advanced from
`416cbd4` to `ff56478` in § 11), the core-boundary leak (the core-boundary rule
in § 7), the maximal-integration deletion audit and the out-of-tree
`bifrost-jmap` dep (§ 7 B15), the incomplete B2 cursor-table set (which expanded
B2's scope to a full table-by-table disposition and has since been satisfied by
the B2 landing - see the B2-done note in § 7), and the compile-only-replacement
gate gap (§ 9 B3 and § 10).

Findings considered but not folded, with reasons:

- R1's pre-loop status observations about the tree (and its note on the then-open
  B1 spec) were point-in-time facts, not changes to this governing plan, and have
  since been overtaken by the B1 landing (which added the bifrost deps and
  `crates/service/src/bifrost/`). No edit to the plan was needed for them.
- R1's "could not verify the `../bifrost` sibling from the sandbox" and the
  accompanying open question, plus R2's open question on whether `core` may take a
  bifrost dependency, are both resolved rather than folded as caveats: the
  user confirmed both `../bifrost` and `./research/bifrost` are at `ff56478`
  (§ 11), and the core-boundary rule (§ 7) settles that `core` stays
  bifrost-free. No residual uncertainty to record.
