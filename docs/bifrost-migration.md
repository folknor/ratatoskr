# Bifrost migration: governing plan

The strategic map for replacing ratatoskr's hand-rolled provider stack with
dependencies on the bifrost workspace. This document is the source-of-record
the spec-loop consumes: every work item below becomes one
technical-implementation-spec, run through the orchestrate.md seven steps. The
loop is running: Track B has begun (B1 has landed; see § 7).

`../bifrost` and `./research/bifrost` are at the same git commit. We keep both
because they serve two distinct purposes: `../bifrost` is the Cargo dependency
path the build resolves, and `./research/bifrost` is a reading-reference that
exists so agents can read bifrost source freely without tripping up the harness.
See § 11 for the full distinction.

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
ratatoskr would benefit from a rewrite or refactor of something in bifrost - a
missing capability, an awkward surface, a wart that would otherwise force a
ratatoskr workaround - that becomes a side-quest, never a ratatoskr
contortion. The orchestrator brings the tree to a clean boundary (landing what
is landable, reverting the blocked in-flight work so nothing parks dirty),
pauses the orchestration loop, and surfaces the brick to the user. The user
implements it in the bifrost repo and re-syncs the two folders (`../bifrost`
and `./research/bifrost` advanced together to a single new shared commit). Only
then does the loop resume, against the updated surface, with the new commit as
the frozen reference for the item.

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
scoping.

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

- B2. CheckpointStore plus cursor schema. Implement bifrost's `CheckpointStore`
  over a new opaque cursor table; migrate off `jmap_sync_state` /
  `folder_sync_state` / `graph_*_delta_tokens`. Needs B1.

  Those three are not the full set. Because bifrost owns the sync engine, push,
  and cursor state, B2 must account for the lifecycle of every protocol/sync
  state table, not just the three named above. Per `reference/architecture.md`
  and `migrations.rs`, the broader set includes at least `jmap_push_state`,
  `graph_subscriptions`, `graph_shared_mailbox_delta_tokens`,
  `graph_contact_delta_tokens`, `shared_mailbox_sync_state`, and
  `public_folder_sync_state`. B2 owns a table-by-table decision for each one:
  retired into the opaque checkpoint envelope, moved into bifrost's ownership, or
  explicitly retained app-side with a stated reason. (Some - e.g. the
  shared-mailbox/public-folder cursors and the push-subscription state - land
  with their feature specs B12 / B3b rather than B2 itself; B2's job is to
  enumerate the full set and pin where each retires, so none is left orphaned.)
- B3. The bifrost-sync consumer (center of gravity). Stand up the `SyncEngine`;
  build the change-stream-to-DB writer (Change / Inventory / hydration to
  `ProviderParsedMessage`-equivalent to body store, search index, messages
  table); wire ack, control/pause handling, and the invalidation sink for
  out-of-process push (Gmail Pub/Sub, Graph webhooks). Feed the unchanged
  application sync layer. Cut sync over for all providers at once; delete the
  `provider-sync` sync impls. Likely splits into B3a (engine plus change
  translation), B3b (push plus invalidation), B3c (control / pause / recovery).
  Needs A1-A3, B1-B2.
- B4. Action pipeline rewire. Dispatch onto `Account` conveniences plus bulk
  mutations over `MutationTarget`; thread-to-message expansion; map
  `AccountError` to `OperationResult`; rebuild the pending-ops / retry journal
  on `RecoveryClass`. Likely splits into B4a (mutation dispatch) and B4b (error
  plus retry). Needs B1, A1-A2.
- B5. Send plus drafts. Rewire onto `send_message` / `draft_*` plus scheduled
  send. Needs A2, A4.
- B6. Folders, labels, containers. Rewire onto `container_*` / `apply_label`
  plus folder/label sync. Needs B3.
- B7. Calendar. Replace the `calendar` crate's per-provider sync (the largest
  single auxiliary; Graph alone is ~41k LOC today) with the bifrost calendar
  surface. Needs B1; A7 for DAV.
- B8. Contacts. Replace Google People, Graph contacts, JMAP contacts, and
  Google other-contacts sync with the bifrost contact surface. Needs B1; A7 for
  DAV.
- B9. Attachments plus cloud attachments. Rewire `fetch_attachment` onto blobs
  and the cloud-storage surface. Needs B1, A6.
- B10. Search. Drive bifrost server-side search/filters where used; the local
  tantivy search stays app-level. Needs B1.
- B11. Server-side filters / Sieve. Rewire onto `filter_*`. Needs B1.
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
- Calendar (B7) is the largest single rewire and depends on bifrost calendar
  maturity (high on native providers; A7 for DAV).
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

- Reading-reference: `./research/bifrost`. This is where agents inspect bifrost
  source - to verify a Track A item against bifrost's current shape, to read the
  `Account` / `AccountError` / `SyncEngine` surface a Track B spec is written
  against, or to confirm a type signature before speccing. Spec authors and
  reviewers read here; it is the ground a bifrost-facing spec is judged against.
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

Track A is complete: the reading-reference `./research/bifrost` is at commit
`ff56478` (the A8-closing commit), in sync with the `../bifrost` dependency
checkout at the same commit. Each Track B spec records, in its ground
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
`bifrost-jmap` dep (§ 7 B15), the incomplete B2 cursor-table set (§ 7 B2), and the
compile-only-replacement gate gap (§ 9 B3 and § 10).

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
