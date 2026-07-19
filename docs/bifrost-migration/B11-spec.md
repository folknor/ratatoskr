# B11 technical-implementation-spec: server-side filters / Sieve

Closes the "server-side filters / Sieve" seam listed in
`docs/bifrost-migration.md` § 5 and enumerated as § 7 item B11 ("Server-side
filters / Sieve. Rewire onto `filter_*`. Needs B1."). The decisive survey
finding (§ 2) is that this seam, like B9's cloud-hosting surface and unlike a
normal action rewire, has NO live caller anywhere in ratatoskr today: the only
server-side-filter code in the tree is `crates/jmap/src/sieve.rs`, a
hand-rolled JMAP Sieve CRUD module that is `pub mod sieve` in the `jmap` crate
but is invoked by nothing in `core`, `service`, `app`, or `sync`. The
user-visible "Filters" settings tab (`crates/app/src/ui/settings/tabs/filters.rs`)
is bound to `state.demo_filters` - two hardcoded placeholder strings
("Auto-archive promotions", "Star from VIPs") seeded in
`crates/app/src/ui/settings/types/mod.rs:580` - with no provider, service, or
`sieve.rs` wiring behind it. Server-side filters were therefore never a live
feature in ratatoskr; they are a dead hand-rolled provider surface plus a demo
UI shell.

Because there is no live server-filter feature, B11 must not build one:
`docs/bifrost-migration.md` § 1 is a feature-preserving plumbing swap, and
wiring a working server-side-filters UX that is absent today would ADD a
capability, violating that mandate. But § 1's maximal-integration rule is
equally binding: no hand-rolled duplicate of a bifrost surface may survive
alongside the bifrost equivalent. Bifrost exposes the canonical
server-filter surface as `Account::{filters_list, filter_create,
filter_update, filter_delete, filter_validate}`
(`../bifrost/crates/types/src/account.rs:566-588`) over the unified
`ServerFilter` / `FilterRule` / `FilterScript` model
(`../bifrost/crates/types/src/filter.rs`). So `crates/jmap/src/sieve.rs` is
exactly a hand-rolled duplicate of that surface with no caller - the § 1
maximal-integration target.

B11's disposition mirrors B9 (`docs/bifrost-migration.md:1481-1487`) precisely:
delete the hand-rolled duplicate NOW (as B9 deleted `gmail/src/gdrive.rs` and
`graph/src/onedrive.rs`), and pin the bifrost `filter_*` surface behind a
capability-dispatched, `#[allow(dead_code)]` ratatoskr forwarder with no UI
caller (as B9 pinned `host_large_attachment` onto `SyncEngine::host_attachment`),
so a future filters-settings product item consumes bifrost's surface rather
than reviving a hand-rolled one. This is the honest reading of "rewire onto
`filter_*`" for a seam whose only occupants are dead code and a demo shell:
there is no live call site to move, so the rewire degenerates to
delete-the-duplicate plus pin-the-surface, exactly as B9 handled cloud hosting.

This spec is written against `reference/technical-implementation-spec.md` (the
contract it must satisfy - READ IT) and conforms to its ten clauses. It is one
item of `docs/bifrost-migration.md` (the governing plan and TODO source - READ
§ 1, § 3, § 5, § 7 B9/B11, § 8, § 11), run through the `orchestrate` procedure
(the standing spec-loop workflow; `reference/orchestrate.md` has been removed and
the repo now invokes `orchestrate` directly per the project CLAUDE.md).

## Required reading (clause 10)

Every implementer and reviewer MUST read these before laying a brick. They are
the ground this work is built on and judged against; naming them is not enough.

- `reference/technical-implementation-spec.md` - the contract this spec is
  written against. Clause 3 (no deferral; separate-TODO work is named and
  excluded), clause 8 (survey the ground; reconcile against sibling surveys),
  and clause 9 (a bounded stopping rule) are the load-bearing clauses for a
  delete-duplicate-plus-pin item.
- `reference/architecture.md` - ALWAYS required. The `core`/`app` firewall (the
  app depends on `rtsk` + `service-api` wire types only, never bifrost), the
  crate map, and the `MailActionIntent -> resolve_intent ->
  build_execution_plan -> batch_execute` action pipeline (which server-side
  filters DO NOT flow through - they are an account-settings surface, not a
  per-message action) all bind where the pinned surface may live.
- `docs/bifrost-migration.md` - the TODO source. § 1 (feature-preserving AND
  maximal-integration - both bind here, and they point in opposite directions
  the spec must reconcile), § 3 (target architecture / the seam), § 5
  ("Rewired: ... server-side filters/Sieve"), § 7 B9 (the pinned-surface
  precedent this spec mirrors, lines 1473-1487) and B11 (line 1501), § 8
  (sequencing), § 11 (the bifrost freeze; B11 advances it - see below).
- `research/bifrost/crates/types/src/filter.rs` + `.../types/src/account.rs`
  (frozen tree) - the bifrost `filter_*` surface B11 consumes: the
  `ServerFilter` / `ServerFilterCreate` / `ServerFilterPatch` / `ServerFilterId`
  / `FilterValidation` model and the five `Account` trait methods, gated by
  `capabilities().filter_rule_shape` and `capabilities().pim_methods`.
- `research/bifrost/crates/sync/src/engine.rs` (frozen tree) - the `SyncEngine`
  passthrough cluster the B11-SQ extends: `host_attachment` (`:1515`) and
  `directory_search` (`:2049`) are the exact shape the new `filter_*`
  passthroughs mirror. Note the two shapes differ: `host_attachment` is a
  non-async `pub fn` returning the NESTED
  `Result<AccountFuture<Result<_, AccountError>>, Error>`, while
  `directory_search` is `pub async fn` returning the FLATTENED `Result<_, Error>`
  (folding `AccountError` in via `?`). `filter_*` follows `directory_search` (§ 4.1).
- `research/bifrost/reference/sync.md` (frozen tree) - REQUIRED per
  `research/bifrost/AGENTS.md` (read the target crate's reference sheet). Its
  direct-passthrough cluster section (`:447`) documents the contract the new
  `filter_*` cluster joins: these methods "return the engine `Error` (the trait's
  `AccountError` folds in through `?`)". This is the authority settling the
  flattened-vs-nested error-shape question (§ 4.1, § 4.3): `filter_*` is a member
  of this flattened direct-passthrough cluster, so the account error is preserved
  NESTED inside `bifrost_sync::Error::Account`, not as a distinct top-level
  variant. The B11-SQ updates this sheet to name the `filter_*` cluster.
- `crates/service/src/bifrost/attachment.rs` - the ratatoskr-side pinned-surface
  precedent: `AttachmentByteSource::host_large_attachment` (`:105`,
  `#[allow(dead_code)]`) forwarding through `action_account.engine.host_attachment`.
  B11's `server_filters` module is a direct structural copy.
- `crates/service/src/bifrost/resident.rs` - `ResidentEngine::action_account`
  (`:355`) returns `ResidentActionAccount { engine: Arc<SyncEngine>, .. }`; the
  pinned forwarder resolves the account through it, mirroring `host_large_attachment`.
- `reference/glossary/folders-labels.md` - REQUIRED because `FilterAction`
  variants (`MoveTo(ContainerId)`, `AddLabel(ContainerId)`, `RemoveLabel(...)`)
  and `FilterCondition::InContainer(ContainerId)` reference containers that map
  to ratatoskr's folders/labels model; a reviewer judging any future mapping
  from `ServerFilter` to ratatoskr types needs the labels model. (B11 itself
  adds no such mapping - it pins the raw bifrost surface - but the required
  reading covers the ground the pinned surface will be built on.)
- `reference/glossary/harness.md` - the Service test harness, `brokkr
  service-suite`, and gate baselines; the green-tree backstop gate (§ 6) is
  defined here.

The `../bifrost` dependency checkout is frozen for the full duration of this
item per `docs/bifrost-migration.md` § 11. The current frozen reference is
`1769367` (§ 11, the B9-SQ commit). B11 consumes a bifrost surface AND adds a
bifrost side-quest (the `SyncEngine::filter_*` passthroughs, § 4.1), so the
freeze ADVANCES here for the sixteenth time; record the exact new frozen commit
in the ground survey of the landing (§ 3), as every prior Track B item did.

## 1. The goal (clause 7: the target as concrete artifacts)

Today the "server-side filters / Sieve" work of `docs/bifrost-migration.md` § 5
is distributed across the tree as follows:

- HAND-ROLLED JMAP SIEVE CRUD (dead, no caller). `crates/jmap/src/sieve.rs`
  (382 LOC) is a complete hand-rolled Sieve surface -
  `list_sieve_scripts`, `get_sieve_script`, `create_sieve_script`,
  `update_sieve_script`, `rename_sieve_script`, `delete_sieve_script`,
  `activate_sieve_script`, `deactivate_sieve_script`, `validate_sieve_script`,
  plus `SieveScript` / `SieveValidationResult` types and
  `server_supports_sieve`. It drives `bifrost_jmap::sieve::*` directly. It is
  declared `pub mod sieve;` at `crates/jmap/src/lib.rs:14` and is called by
  NOTHING in the workspace (verified in § 2.1). This is the § 1
  maximal-integration duplicate of `Account::filter_*`.
- DEMO SETTINGS UI (placeholder, no backend). `crates/app/src/ui/settings/tabs/filters.rs`
  renders `state.demo_filters` when non-empty; the list is two static strings
  seeded in `crates/app/src/ui/settings/types/mod.rs:580` and mutated only by
  local drag reordering (`crates/app/src/ui/settings/update/list_drag.rs:84`).
  No message handler calls a provider, `sieve.rs`, or any service endpoint. It
  is UI chrome, structurally identical to any other demo settings list.
- LOCAL CLIENT-SIDE FILTER ENGINE (app-level, NOT this seam). `crates/sync/src/filters.rs`
  (`FilterCriteria` / `FilterActions` / `FilterableMessage`) is ratatoskr's
  LOCAL rule engine that matches synced messages against user rules and applies
  local labels/archive/star/read/trash. It is the client-side analogue of
  Gmail-style filters run over the local mailbox, not a server-side Sieve/rule
  surface. Like local tantivy search in B10, it is app-level BY DESIGN and is
  out of B11's scope (§ 5, named not deferred).

After B11, the state is:

- The hand-rolled `crates/jmap/src/sieve.rs` and its `pub mod sieve` line are
  DELETED. No hand-rolled server-filter surface survives anywhere (proven by
  the § 4.4 invariant gate, not asserted).
- A bifrost `SyncEngine::filter_*` passthrough cluster exists (B11-SQ, § 4.1),
  mirroring `host_attachment` / `directory_search`: five additive forwarders to
  `Account::filter_*`, resolving through `live_account`.
- A ratatoskr `crates/service/src/bifrost/server_filters.rs` module (§ 4.3)
  holds a `#[allow(dead_code)]` `ServerFilterSurface` whose five methods
  forward through `action_account.engine.filter_*`, capability-dispatched and
  with NO UI caller - the pinned surface for a future filters-settings item,
  exactly as B9 pinned `host_large_attachment`.
- The demo settings tab and the local `crates/sync/src/filters.rs` engine are
  UNCHANGED.

The concrete artifacts B11 produces:

1. (bifrost, B11-SQ) five `SyncEngine` methods:

```rust
// edit research/bifrost/crates/sync/src/engine.rs (the SQ staging copy; promoted
// to ../bifrost by scripts/bifrost.sh), mirroring directory_search (async, awaited)
pub async fn filters_list(&self, account_id: &AccountId)
    -> Result<Vec<bifrost_types::ServerFilter>, Error>;
pub async fn filter_create(&self, account_id: &AccountId, filter: bifrost_types::ServerFilterCreate)
    -> Result<bifrost_types::ServerFilterId, Error>;
pub async fn filter_update(&self, account_id: &AccountId, filter: bifrost_types::ServerFilterId, patch: bifrost_types::ServerFilterPatch)
    -> Result<(), Error>;
pub async fn filter_delete(&self, account_id: &AccountId, filter: bifrost_types::ServerFilterId)
    -> Result<(), Error>;
pub async fn filter_validate(&self, account_id: &AccountId, filter: bifrost_types::ServerFilterCreate)
    -> Result<bifrost_types::FilterValidation, Error>;
```

   Each body is `Ok(self.live_account(account_id)?.<method>(args).await?)`,
   identical in shape to `directory_search` (`:2049-2060`) - the synchronous
   `live_account` surfaces `AccountNotAttached` up front, the awaited
   `AccountFuture` surfaces the trait's `AccountError` (including
   `Unsupported` for accounts whose `filter_rule_shape` is `None` or whose
   `pim_methods` filter flags are false).

2. (ratatoskr) `crates/service/src/bifrost/server_filters.rs`:

```rust
pub(crate) struct ServerFilterSurface {
    resident: ResidentEngine,
}

impl ServerFilterSurface {
    pub(crate) fn new(resident: ResidentEngine) -> Self { Self { resident } }

    #[allow(dead_code)] // pinned filters-settings surface; B11 adds no UI caller
    pub(crate) async fn list(&self, account_id: &str)
        -> Result<Vec<ServerFilter>, ServerFilterError> { /* forward */ }
    // create / update / delete / validate mirror `list`, each forwarding
    // through action_account.engine.filter_*.
}
```

   forwarding through the resident engine exactly as
   `AttachmentByteSource::host_large_attachment` resolves its account, but with
   the FLATTENED error shape of the `filter_*` engine methods (§ 4.1, modeled on
   `directory_search`), not `host_attachment`'s nested split:
   `let action_account = self.resident.action_account(account_id).await
   .map_err(ServerFilterError::Attach)?; let account = AccountId(...);
   action_account.engine.filters_list(&account).await
   .map_err(ServerFilterError::Engine)`.

   `ServerFilterError` is therefore a TWO-variant enum (`Attach(String)`,
   `Engine(bifrost_sync::Error)`), not the three-variant shape
   `AttachmentByteError` uses. The account-level classification is NOT lost: the
   directory-search-shaped `filter_*` fold `AccountError` into
   `bifrost_sync::Error::Account` via `?` (`research/bifrost/crates/sync/src/error.rs:59-63`,
   which carries the full `AccountError` with its `RecoveryClass` intact), so
   `Unsupported` and every other account error survive NESTED inside the `Engine`
   variant rather than as a distinct top-level `Account` variant. This matches the
   documented direct-passthrough cluster contract in
   `research/bifrost/reference/sync.md` (the contact / `directory_search` cluster
   "returns the engine `Error` (the trait's `AccountError` folds in through `?`)"),
   of which `filter_*` is a member.

3. Deletion of `crates/jmap/src/sieve.rs` (382 LOC) and the `pub mod sieve;`
   line at `crates/jmap/src/lib.rs:14`.

4. The § 4.4 invariant gate.

There is NO new wire message, DB table, cursor, schema, action type, or
`app`/`service-api` change. `ServerFilter` and friends do NOT cross the
`service-api` wire in B11 (there is no caller), so no wire type is minted -
that is a future settings item's job.

## 2. Survey of the ground (clause 8)

The survey must be exhaustive enough to be falsifiable: it must show that the
server-filter seam has no live caller (so B11 is a pin-and-delete, not a
rewire) and that the deletion drops no load-bearing work.

### 2.1 `crates/jmap/src/sieve.rs` has zero callers (the decisive fact)

A workspace-wide sweep splits into two token families, both of which confirm
the seam is dead. The SIEVE-family tokens the hand-rolled module actually uses
(`sieve`, `Sieve`, `SieveScript`, `server_supports_sieve`, `list_sieve_scripts`,
`create_sieve_script`) return hits ONLY inside `crates/jmap/src/sieve.rs`
(definitions + its own unit tests, where the `sieve::` references reach the
upstream `bifrost_jmap::sieve` crate, not any ratatoskr caller) and
`crates/jmap/src/lib.rs:14` (the `pub mod sieve;` declaration). The
BIFROST-family tokens (`filters_list`, `filter_create`, `ServerFilter`) - the
surface B11 pins, names the dead module never used - return ZERO hits ANYWHERE
in ratatoskr, `sieve.rs` included (an earlier draft wrongly claimed these landed
in `sieve.rs`; they do not, because that module speaks the sieve-family
vocabulary above). Either way: no `use jmap::sieve`, no external `sieve::` call,
no `SieveScript` construction appears in `core`, `service`, `app`, `sync`, or any
other crate. The module is dead: it compiles as part of the `jmap` crate but is
never invoked. Deleting it removes no reachable behavior. (The audit command and
its classification are mechanized as the § 4.4 gate so this stays true.)

### 2.2 The "Filters" settings tab is a demo shell (no backend to preserve)

`crates/app/src/ui/settings/tabs/filters.rs` renders `state.demo_filters` only.
`demo_filters: Vec<EditableItem>` (`crates/app/src/ui/settings/types/mod.rs:355`)
is seeded with two static labels (`:580-589`) and is mutated exclusively by the
generic list-drag reorder path (`crates/app/src/ui/settings/update/list_drag.rs:84-85`,
which even routes the unknown-key default back to `demo_filters`). There is no
`SettingsMessage` variant, handler, or service call that reads a real filter
from a provider or writes one back. The tab is a visual placeholder. B11 leaves
it untouched: removing it would be scope creep (UI deletion), and wiring it
would ADD the absent feature (forbidden by § 1 feature-preserving). Its
disposition is named in § 5.

### 2.3 Bifrost exposes the canonical `filter_*` surface, fully implemented

`../bifrost/crates/types/src/account.rs:566-588` declares the five `Account`
methods over the model in `../bifrost/crates/types/src/filter.rs`
(`ServerFilter::{Rule,Script}`, `FilterRule` with typed `FilterCondition` /
`FilterAction`, `FilterScript` with `ScriptLanguage::Sieve` body, and
`FilterValidation` with severity-tiered diagnostics). Every provider Account
impl in the frozen tree implements them - JMAP
(`../bifrost/crates/jmap/src/sync/account.rs:823-846`), Google
(`.../google/src/account/mod.rs:611-639`), Graph
(`.../graph/src/account/mod.rs:774-801`), IMAP-Sieve
(`.../imap/src/account/mod.rs:634-657`), plus CardDAV/CalDAV returning
`Unsupported`. The surface is complete at the frozen commit; only the
`SyncEngine` passthrough is missing, which the B11-SQ adds (§ 4.1). This is the
`filter_*` the migration line names.

### 2.4 The `SyncEngine` has no `filter_*` passthrough yet (the B11-SQ gap)

`../bifrost/crates/sync/src/engine.rs` exposes `host_attachment` (`:1515`) and
`directory_search` (`:2049`) but no filter passthrough. The pinned ratatoskr
forwarder (§ 4.3) resolves the account via `ResidentEngine::action_account`,
whose `engine` field is an `Arc<SyncEngine>` (`resident.rs:366`); it therefore
calls `engine.filter_*`, which must exist on `SyncEngine`. Hence the B11-SQ (§
4.1) is a prerequisite of the ratatoskr pin, exactly as B9-SQ's
`SyncEngine::host_attachment` was a prerequisite of `host_large_attachment`.
Going through the `SyncEngine` passthrough (rather than reaching for
`live_account` from ratatoskr) matches the B9 precedent and keeps ratatoskr off
bifrost's internal `live_account` surface.

### 2.5 Local `crates/sync/src/filters.rs` is app-level, NOT this seam

`crates/sync/src/filters.rs` (`FilterCriteria` / `FilterActions` /
`FilterableMessage`) is ratatoskr's CLIENT-SIDE filter machinery: `evaluate_filters`
is PURE computation (`filters.rs:144`, "does not touch DB or providers") that
maps user rules over `FilterableMessage` rows into per-thread `FilterResult`
actions. It is currently DORMANT, not a live pipeline stage: a workspace-wide
caller search finds no non-test invocation of `evaluate_filters`, and the only
cross-module reuse is `FilterableMessage`, imported by notification code
(`crates/sync/src/notifications.rs:4`) - not the filter evaluator itself. It
never calls a provider and is not a server-side-filter surface. (The earlier
"consumed by the sync pipeline / applies local mutations" framing overstated it;
the machinery computes actions but nothing wires it into a live mutation path
today.) This is the direct analogue of B10's local-vs-provider search disambiguation
(`docs/bifrost-migration.md:1488-1500`): local filtering stays app-level by
design and is out of B11's scope (§ 5). B11 does not touch it, and the § 4.4
gate is scoped so it does not false-positive on this module.

### 2.6 The seam does not flow through the action pipeline

Server-side filters are an account-SETTINGS surface (list/create/update/delete/
validate rules or scripts), not a per-message `MailActionIntent`. They do not
enter `resolve_intent -> build_execution_plan -> batch_execute`
(`reference/architecture.md`), carry no `CompletionBehavior`, and need no undo/
toast/auto-advance wiring. B11 adds nothing to the action pipeline; the pinned
surface is a direct `ServerFilterSurface` method group, parallel to
`AttachmentByteSource`, not an action.

### 2.7 Table / cursor / schema disposition

B11 touches no table, cursor, or schema. There is no persisted server-filter
state in ratatoskr today (the seam is dead), and B11 adds none - the pinned
surface is a live passthrough with no local store. A future filters-settings
item may add caching, but that is out of scope (§ 5).

## 3. The split (clause 6: keep/revert, ordered so the tree stays green)

Two landings, in order. Each is coherent and fully intrusive; `brokkr check` is
green at the boundary before and after each.

Record the new frozen `../bifrost` commit (the B11-SQ) in this ratatoskr
landing's commit message and ground-survey note AND in the § 11 migration
ledger, per § 11 (as every prior Track B item did). Do NOT ask the B11-SQ
commit message to carry its own final hash - a commit cannot contain the hash
it has not yet been assigned; the promoted-commit hash is recorded downstream
(the ratatoskr landing and the ledger), never inside the bifrost commit itself.

### B11-SQ (bifrost repo, lands first) - expose `filter_*` passthrough on `SyncEngine`

Add the five additive `SyncEngine::filter_*` forwarders (§ 4.1) - AND the
matching required-reading doc update (§ 4.1) - by editing
`research/bifrost/crates/sync/src/engine.rs`, the in-tree staging copy where all
side-quest edits to bifrost are made (`docs/bifrost-migration.md:9-14, 1600-1610`);
the orchestrator then commits it there and `bash scripts/bifrost.sh` promotes the
committed SQ to the `../bifrost` dependency path. Do NOT edit `../bifrost`
directly - that is the frozen dependency the ratatoskr gates build against, and
editing it in place bypasses the staging-and-promotion protocol and invalidates
the freeze. Mirror `directory_search`. Pure additive; no existing bifrost
behavior changes. Gate: bifrost `brokkr check` green PLUS the named
compile-and-dispatch unit test (§ 4.1); per `research/bifrost/AGENTS.md`, bifrost
tests are small/technical, and clause 5 requires the smallest deterministic test
where a behavior is testable, so the forwarders carry that unit gate (not
compile-alone coverage) - no round-trip mock is added, matching B9-SQ
(`docs/bifrost-migration.md:1774-1776`). This advances the § 11 freeze a
sixteenth time from `1769367`; the promoted commit is the frozen reference the
ratatoskr landing builds against.

### B11 (ratatoskr repo) - delete the duplicate + pin the surface + gate

Ordered so the tree stays green:

1. Add `crates/service/src/bifrost/server_filters.rs` (§ 4.3) and
   `pub mod server_filters;` in `crates/service/src/bifrost/mod.rs`. This
   compiles against the new frozen bifrost `SyncEngine::filter_*`. It has no
   caller (`#[allow(dead_code)]`), so it changes no runtime behavior.
2. Delete `crates/jmap/src/sieve.rs` and the `pub mod sieve;` line in
   `crates/jmap/src/lib.rs` (§ 4.2). Because § 2.1 proved zero callers, this
   drops no reachable code and cannot break a downstream crate.
3. Add the § 4.4 invariant gate (source lockdown in the `service` crate).
4. Fold the § 4.5 migration-doc reconciliation into this same commit (B11 lands
   real code, so it carries its own doc note - never a standalone markdown
   commit).

Steps 1-4 are one ratatoskr commit (the pin, the delete, the gate, and the doc
note are one coherent keep/revert unit). There is no ordering hazard within it:
the pin does not depend on the delete, and the delete does not depend on the
pin; both depend only on the B11-SQ freeze.

## 4. The bricks

### 4.1 B11-SQ: `SyncEngine::filter_*` passthrough (bifrost)

Edit `research/bifrost/crates/sync/src/engine.rs` (the SQ staging copy, promoted
to `../bifrost` by `scripts/bifrost.sh` - never edit `../bifrost` in place, § 3),
in the PIM passthrough cluster alongside `directory_search`, adding the five
methods listed in § 1 artifact 1. Each is `pub async fn`, resolves
`self.live_account(account_id)?` synchronously (surfacing `AccountNotAttached`),
then `.<Account method>(args).await?` (folding `AccountError` into
`bifrost_sync::Error::Account` via `?`, including `Unsupported` when the account's
`filter_rule_shape`/`pim_methods` deny the method - the flattened
direct-passthrough contract, § 1 artifact 2, NOT `host_attachment`'s nested
shape). Doc-comment each with the `Account::filter_*` forward and the capability
gate, mirroring `directory_search`'s doc block (`:2045-2048`). Import
`bifrost_types::{ServerFilter, ServerFilterCreate, ServerFilterPatch,
ServerFilterId, FilterValidation}` (or reference fully-qualified, as
`host_attachment` does for `CloudUploadMeta`).

Update the bifrost required-reading and the passthrough-cluster inventory:
`research/bifrost/AGENTS.md` requires reading the target crate's reference sheet,
so add `research/bifrost/reference/sync.md` to this spec's required-reading list
(clause 10, done above),
and extend that sheet's direct-passthrough cluster enumeration (the contact /
`directory_search` list, `sync.md:447`) to name the new `filter_*` cluster and
its flattened engine-`Error` contract - otherwise the reference goes stale the
moment the forwarders land. This doc edit rides in the B11-SQ commit.

Verification (bifrost repo): `brokkr check` green, PLUS a REQUIRED (not
optional) compile-and-dispatch unit test. `SyncEngine::filter_*` is
deterministically testable without any product UI - clause 5 mandates the
smallest deterministic test where a behavior can be pinned, so this test is a
brick of the SQ, not a reviewer's discretionary extra. Name it (e.g.
`filter_passthrough_forwards_and_flattens`) and cover: (a) each of the five
delegations dispatches to the matching `Account::filter_*` with its arguments
forwarded unchanged; (b) an unattached engine returns `Err(AccountNotAttached)`
up front (the synchronous `live_account` bail, mirroring any existing
`live_account`-bailing test in that file); and (c) a trait method returning
`Err(AccountError)` surfaces as `Error::Account` with the account error carried
intact (the flattened-fold contract § 1 artifact 2 relies on). No mock
round-trip gate is added, matching the B9-SQ precedent - the unit test pins the
dispatch and error-fold, which is the whole of the added behavior.

### 4.2 Delete the hand-rolled Sieve duplicate (ratatoskr)

Delete `crates/jmap/src/sieve.rs` in full and remove `pub mod sieve;` at
`crates/jmap/src/lib.rs:14`. Confirm no other `jmap` module references
`sieve::` (§ 2.1 shows none). The `jmap` crate still compiles - the module was
a leaf with no intra-crate dependents. This is the § 1 maximal-integration
deletion, discharged early for the filter slice (the `jmap` crate as a whole
retires at B15; B11 removes only the filter duplicate now, exactly as B9
deleted `gdrive.rs`/`onedrive.rs` ahead of B15's provider-crate deletion).
B11's deletion is INPUT to B15's whole-workspace audit
(`docs/bifrost-migration.md:1514-1519`), not a waiver of it.

### 4.3 Pin the bifrost `filter_*` surface (ratatoskr)

Add `crates/service/src/bifrost/server_filters.rs` with the
`ServerFilterSurface` struct (§ 1 artifact 2), holding a `ResidentEngine` and
constructed via `new(resident)` - a direct structural copy of
`AttachmentByteSource` (`attachment.rs:29-36`). Its five methods
(`list`/`create`/`update`/`delete`/`validate`) each:

- resolve `let action_account = self.resident.action_account(account_id).await
  .map_err(ServerFilterError::Attach)?;`
- build `let account = AccountId(account_id.to_string());`
- forward `action_account.engine.filter_*(&account, <args>).await
  .map_err(ServerFilterError::Engine)`

re-exporting bifrost's `ServerFilter` / `ServerFilterCreate` /
`ServerFilterPatch` / `ServerFilterId` / `FilterValidation` for the signatures.
Define a small TWO-variant `ServerFilterError` enum (`Attach(String)`,
`Engine(bifrost_sync::Error)`) - do NOT collapse to `ServiceError`. This splits
the resident-attach failure from the bifrost engine error, but does NOT
reproduce `AttachmentByteError`'s three-variant top-level split (`Attach` /
`Engine` / `Account`): the `directory_search`-shaped `filter_*` engine methods
(§ 4.1) already flatten `AccountError` into `bifrost_sync::Error::Account` via
`?` (which retains the full `AccountError` and its `RecoveryClass`), so the
structured `AccountError` / `Unsupported` classification is preserved NESTED
inside the `Engine` variant, not surfaced as a separate `Account` variant the
way `host_large_attachment` does (that forwarder's nested `host_attachment`
shape is what earns its distinct `Account` arm; the flattened `filter_*` cluster
does not, and must not pretend to). Mark the whole struct/methods
`#[allow(dead_code)]` with the comment
`// pinned filters-settings surface; B11 adds no UI caller`, matching
`attachment.rs:104`. Wire `pub mod server_filters;` into
`crates/service/src/bifrost/mod.rs`.

This adds NO caller and NO wire type. It pins the surface so a future
filters-settings item forwards through bifrost's `filter_*` rather than reviving
a hand-rolled Sieve module.

### 4.4 The invariant gate (mechanize the § 2.1 audit)

The threat this gate guards: a future change reintroducing a hand-rolled
server-filter/Sieve CRUD surface alongside bifrost's `filter_*` (violating § 1
maximal-integration), or the pinned surface being bypassed by a direct
provider-Sieve call. Following B10's Gate B precedent (a source lockdown that
mechanizes the one-time audit into a permanent test, same shape as
`crates/db-read-lockdown/tests/lockdown.rs`):

**Gate - `server_filters_route_through_bifrost` (test lives in the `service`
crate, but its SCAN is workspace-wide).** A lockdown test that walks EVERY
ratatoskr Rust source under `crates/*/src` - not just `service`/`core` - and
asserts:

1. The deleted duplicate stays deleted: `crates/jmap/src/sieve.rs` does not
   exist AND no `pub mod sieve;` line appears in `crates/jmap/src/lib.rs`. This
   is the load-bearing addition: § 1 claims "no hand-rolled server-filter
   surface survives ANYWHERE," but a gate that scanned only `service` would GREEN
   even after someone restored `crates/jmap/src/sieve.rs`. The scan must reach
   the crate that actually held the duplicate.
2. No source references the hand-rolled Sieve CRUD surface -
   `bifrost_jmap::sieve` / `SieveScript` / `SieveValidationResult` /
   `server_supports_sieve` / a `create_sieve_script`/`sieve_script_create`-style
   function - anywhere in the workspace.
3. Server-filter access appears ONLY as `engine.filter_*` inside the single
   allow-listed call site `crates/service/src/bifrost/server_filters.rs`
   (`ServerFilterSurface`). A direct `engine.filter_*` call from any other file
   FAILS the gate, since `ServerFilterSurface` is the sole intended entry point.

Scan source only, EXCLUDING `#[cfg(test)]` / test-module bodies and test fixture
strings, so a test that names `SieveScript` in an assertion does not
false-positive (`crates/jmap/src/sieve.rs`'s own unit tests are gone after § 4.2
anyway, but the exclusion also keeps the gate robust if it ever runs against a
pre-delete tree or a fixture). The `service`-crate placement of the TEST is
load-bearing: `service` links the resident engine and survives the migration
(unlike the `jmap` crate, which retires at B15), so the gate outlives the
provider crates - but the SCAN it performs is the whole workspace, not the
`service` crate alone. The gate is scoped to EXCLUDE `crates/sync/src/filters.rs`
(the local engine, § 2.5) and the demo settings tab (§ 2.2) by matching the
Sieve/`SieveScript` provider-surface tokens, not the generic word "filter".

Exact gate command (clause 5, copy-pasteable; package is `service`):

```
brokkr test -p service server_filters_route_through_bifrost
```

This name and placement are pinned intent - implement under this exact name so
the § 6 gate command resolves without substitution. (A leaf-crate manifest
guard analogous to B10's Gate A is NOT added: the `jmap` crate that held the
duplicate retires wholesale at B15, so a manifest guard on it would be
short-lived; the source lockdown in the surviving `service` crate is the durable
invariant.)

### 4.5 Migration-doc reconciliation (rides with the ratatoskr code landing)

Update `docs/bifrost-migration.md`: rewrite the § 7 B11 line
(`:1501`) from a TODO into a done-note recording that the seam is CLOSED - the
hand-rolled `crates/jmap/src/sieve.rs` deleted (maximal-integration), the demo
settings tab and local `sync/filters.rs` engine left as app-level surfaces (§
5), the bifrost `filter_*` surface pinned behind `ServerFilterSurface`
(`#[allow(dead_code)]`, no UI caller, mirroring B9's `host_large_attachment`),
and the invariant gated (§ 4.4). Record the B11-SQ freeze advance (sixteenth,
from `1769367`) in § 11 with the new commit hash and a one-line description of
the additive `SyncEngine::filter_*` passthrough, mirroring the B9-SQ § 11 entry
(`:1765-1777`). Bundle this markdown with the § 4.2-4.4 code in the same commit
(never a standalone markdown commit). The full crate-map /
`reference/architecture.md` reconciliation remains B16's job.

## 5. Stopping rule (clause 9)

- IN: the B11-SQ `SyncEngine::filter_*` passthrough (§ 4.1); deletion of the
  hand-rolled `crates/jmap/src/sieve.rs` (§ 4.2); the pinned
  `ServerFilterSurface` (§ 4.3); the invariant gate (§ 4.4); the migration-doc
  closure note + § 11 freeze advance (§ 4.5).
- OUT, named not deferred (clause 3):
  - A working server-side-filters SETTINGS FEATURE (loading real rules,
    rendering them, editing/creating/deleting against the provider): a NEW
    product item, forbidden here by § 1 feature-preserving (the feature is
    absent today). The demo tab and pinned surface are the seam of a future
    item, not B11.
  - The demo "Filters" settings tab (`crates/app/src/ui/settings/tabs/filters.rs`,
    `demo_filters`): UI chrome, left exactly as-is. Neither deleted (scope creep)
    nor wired (would add the absent feature).
  - The LOCAL client-side filter engine (`crates/sync/src/filters.rs`):
    app-level by design (the § 2.5 analogue of B10's local search). Untouched.
  - `service-api` wire types for `ServerFilter`: minted by the future settings
    item when a caller exists, not by B11 (no caller = no wire type).
  - The whole `jmap`-crate deletion and the workspace-wide maximal-integration
    audit: B15. B11 discharges only the FILTER slice early and records it as
    input to B15.
- Blast radius: one new ratatoskr module (`server_filters.rs`), one deleted
  ratatoskr module (`jmap/src/sieve.rs`) plus its `mod` line, one new invariant
  test, one bifrost additive passthrough cluster, and doc notes. No `app` /
  `service-api` / wire change; no schema / cursor / table change; no
  action-pipeline change; no change to the demo tab or the local filter engine.

## 6. Verification per brick (clause 5)

- The bifrost B11-SQ (§ 4.1), in the bifrost repo - green tree AND the required
  compile-and-dispatch unit test (clause 5; the forwarders ARE deterministically
  testable, so the unit test is mandatory, not compile-alone coverage):

```
brokkr check
brokkr test -p sync filter_passthrough_forwards_and_flattens
```

  (The unit test pins the five delegations, argument forwarding, the
  `AccountNotAttached` up-front bail, and the `AccountError -> Error::Account`
  fold, per § 4.1. No round-trip mock gate is added, matching the B9-SQ
  precedent - the added behavior is dispatch + error-fold, which the unit test
  fully covers. Adjust the `-p` package to the bifrost sync crate's name if it
  differs.)

- The ratatoskr pinned surface + deletion + gate (§ 4.2-4.4). The universal
  green-tree gate, which proves the `jmap` crate still compiles after the
  `sieve.rs` deletion and the `service` crate compiles the pinned
  `ServerFilterSurface` against the new bifrost surface:

```
brokkr check
```

- The invariant gate (§ 4.4), the durable behavior B11 adds:

```
brokkr test -p service server_filters_route_through_bifrost
```

- The Service-boundary backstop - a GENERAL green-tree gate that the Service
  boot + handler surface still assembles after the module add/delete (NOT proof
  of any server-filter behavior, since B11 wires no caller):

```
brokkr service-suite
```

Coverage splits by layer. The bifrost `SyncEngine::filter_*` forwarders ARE
deterministically testable and so carry the required unit test above (clause 5) -
dispatch and error-fold are pinned, not merely compiled. The ratatoskr pinned
`ServerFilterSurface`, like B9's `host_large_attachment`, has no live caller, so
its coverage is compile-and-capability-dispatch plus the maximal-integration
invariant gate - the spec says so explicitly, per clause 5's provision for a
behavior no round-trip instrument can yet pin (there is no product caller to
drive a round trip through the resident engine). When a future settings item adds
that caller, it owes the round-trip harness gate; B11 does not, because driving a
real end-to-end `filter_*` round trip with no product caller would be building
throwaway test scaffolding, not pinning shipped behavior.

## 7. The falsifiability challenge (why "no live caller" is a finding, not a hand-wave)

B11's pin-and-delete disposition is correct only if the server-filter seam
genuinely has no live caller. The finding is REFUTED if any of these is true at
implementation time:

1. Any crate outside `crates/jmap/src/sieve.rs` references `jmap::sieve`,
   `SieveScript`, or otherwise invokes the hand-rolled Sieve surface. (§ 2.1
   sweep. As surveyed: false - zero callers. If a caller exists, deleting
   `sieve.rs` breaks the build, and B11 becomes a real rewire that must migrate
   that caller onto `ServerFilterSurface` / `engine.filter_*` before landing.)
2. The "Filters" settings tab has a real backend path (a `SettingsMessage`
   handler that reads/writes a provider filter, not just `demo_filters` reorder).
   (§ 2.2. As surveyed: false - it is a demo shell. If true, B11 would be
   preserving a live feature and must rewire that path, not merely pin.)
3. Bifrost lacks a working `Account::filter_*` for a provider ratatoskr ships.
   (§ 2.3. As surveyed: false - all provider Account impls implement the five
   methods at the frozen commit. If a provider returned only `Unsupported`, the
   pinned surface still compiles and dispatches correctly; the gate is
   capability-driven.)
4. `docs/bifrost-migration.md` § 1 is reinterpreted to REQUIRE wiring the
   filters feature now. (That would make B11 a feature-add, contradicting the
   feature-preserving mandate - a separate product item, not a B11 reopening.)

If (1) or (2) is non-empty at land, B11 stops being pin-and-delete and becomes a
real caller migration: the found call site moves onto the bifrost `filter_*`
surface (through `ServerFilterSurface`), specified to this document's standard,
before landing. Absent that, the finding stands: the seam's only occupants are a
dead hand-rolled module and a demo shell, so the correct disposition is delete
the duplicate, pin the bifrost surface, and gate the invariant - the B9-mirror
close.

## 8. Review reconciliation

Two reviews landed: R1 (opus, `B11-R1.md`) and R2 (codex xhigh, `B11-R2.md`).
Every finding was re-validated against the frozen tree; the valid ones are
folded into the sections above. Both reviewers independently confirmed the
load-bearing survey (zero-caller, demo-tab shell, bifrost method inventory,
`1769367` freeze) - that foundation stands unchanged.

### Folded (valid)

- **Error-shape contradiction (R1 "Bug" + R2 "R1 reconciliation", consolidated).**
  Both reviewers flagged that § 4.3 claimed the ratatoskr forwarder reproduces
  `AttachmentByteError`'s three-way top-level split (`Attach`/`Engine`/`Account`)
  while § 1 artifact 2 defined only two variants. Real defect; folded. RESOLUTION
  follows R2, not R1: I kept the `directory_search`-shaped async `filter_*`
  engine methods (§ 4.1) and REJECTED R1's proposal to re-model them on
  `host_attachment`'s nested `pub fn` shape - because `research/bifrost/reference/sync.md:447`
  documents the direct-passthrough cluster contract (contacts / `directory_search`)
  as FLATTENED, "returns the engine `Error` (the trait's `AccountError` folds in
  through `?`)", and `filter_*` is a member of exactly that cluster. Inventing the
  nested shape would make `filter_*` inconsistent with its sibling passthroughs.
  R1's underlying point - the spec must stop pretending to a three-way split - is
  nonetheless correct and is fixed: § 1 artifact 2 and § 4.3 now specify a
  TWO-variant `ServerFilterError` (`Attach`, `Engine(bifrost_sync::Error)`) with
  `AccountError`/`Unsupported` preserved NESTED inside `Engine` (bifrost
  `error.rs:59-63` carries the full `AccountError` + `RecoveryClass`), and
  required reading (§ engine.rs / sync.md entries) now spells out the
  flattened-vs-nested distinction so an implementer cannot re-trip on it.
- **Invariant gate cannot catch the regression it names (R2 P1 #2, R1 § 4.4 nit).**
  Valid and important: the § 4.4 gate scanned only `service`, so a restored
  `crates/jmap/src/sieve.rs` would GREEN despite § 1's "survives nowhere" claim.
  § 4.4 rewritten: the test still lives in `service` (durable home) but its SCAN
  is workspace-wide, explicitly asserts `sieve.rs` + the `pub mod sieve;` line are
  absent, restricts `engine.filter_*` to the single `server_filters.rs` call site,
  and excludes `#[cfg(test)]`/fixture bodies (R1's non-test-scope nit).
- **Forwarding untested despite being deterministically testable (R2 P1 #3).**
  Valid per clause 5 (the smallest deterministic test is a required brick, not
  optional). § 4.1 and § 6 now mandate a named bifrost unit test covering the
  five delegations, argument forwarding, `AccountNotAttached`, and
  `AccountError -> Error::Account`; the "if a reviewer wants" hedge is gone.
- **SQ targets the wrong checkout (R2 P1 #1).** Valid: side-quest edits are made
  in `research/bifrost` and promoted by `scripts/bifrost.sh`, never in `../bifrost`
  directly (`docs/bifrost-migration.md:1600-1610`). § 1 artifact 1 comment, § 3,
  and § 4.1 now direct edits to `research/bifrost` with the promotion note.
- **Stale orchestration + self-hash (R2 P2 #4, partial).** Valid parts folded:
  the deleted `reference/orchestrate.md` reference (line 47) now points at the
  `orchestrate` procedure; § 3 no longer asks the B11-SQ commit message to carry
  its own not-yet-assigned hash (recorded in the ratatoskr landing + § 11 ledger
  instead). REJECTED part: the recommendation to move § 4.5's migration-doc note
  out of the implementer's commit into a separate step-6 pass - that contradicts
  the repo rule "never commit markdown alone, bundle it with the code" and the
  B9/B10 precedent each item followed; § 4.5's bundling stays.
- **sync.md required-reading + staleness (R2 P2 #5).** Valid:
  `research/bifrost/AGENTS.md` requires the target crate's reference sheet.
  `research/bifrost/reference/sync.md` added to required reading, and § 4.1 now
  requires the B11-SQ to extend that sheet's passthrough-cluster inventory with
  the `filter_*` cluster.
- **Local-filter survey overstated (R2 P2 #6).** Valid: `evaluate_filters` is
  pure (`filters.rs:144`) with no non-test caller; only `FilterableMessage` is
  reused (by `notifications.rs:4`). § 2.5 now describes dormant local-filter
  machinery, not a live pipeline stage.
- **§ 2.1 token overreach (R1 nit).** Valid: the bifrost-family tokens
  (`filters_list`/`filter_create`/`ServerFilter`) appear NOWHERE in ratatoskr,
  not in `sieve.rs` (which speaks the sieve-family vocabulary). § 2.1 rewritten to
  split the two token families.
- **LOC off-by-one (R1 nit).** `sieve.rs` is 382 lines, not 383; fixed both
  occurrences.

### Rejected

- **R1: re-model `SyncEngine::filter_*` on `host_attachment`'s nested `pub fn`
  shape.** Rejected. `filter_*` belongs to bifrost's documented flattened
  direct-passthrough cluster (`sync.md:447`); the nested shape is specific to the
  streaming/hydration `host_attachment` and adopting it for `filter_*` would break
  cluster consistency and invent a contract bifrost does not define. The genuine
  defect R1 found (the false three-way-split claim) is fixed the other way - by
  correcting the prose to the two-variant nested-error reality - as R2 argued.
- **R2 P2 #4, in part: move the migration-doc note to a separate step-6 pass.**
  Rejected (see the folded entry above): conflicts with the repo's
  bundle-markdown-with-code rule and the established Track B precedent. Only the
  self-hash and orchestrate.md sub-points of #4 were folded.
