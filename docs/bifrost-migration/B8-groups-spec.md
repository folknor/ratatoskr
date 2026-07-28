# B8-groups technical-implementation-spec: Exchange distribution / M365 group sync onto the bifrost directory-groups surface

Written against the contract in `reference/technical-implementation-spec.md`.
Spawned from the `B8-groups` item under `docs/bifrost-migration.md` section 7
(the B8 carve-out). Tracked for the item's implementation window; removed at
landing, per the B12 precedent.

Frozen dependency references (clause 8; see `docs/bifrost-migration.md`
section 11 for the freeze discipline):

- `../bifrost` (and its in-tree twin `research/bifrost`) frozen at
  `59b9e2d` - "directory groups: mail-enabled org-group listing and
  transitive expansion", the B8-groups-SQ landing. This spec is authored and
  must be gated against exactly that commit.
- `research/saehrimnir` surveyed at `0cf44e4`. This item carries its own
  saehrimnir side-quest (Brick 1); the freeze reference for the mock advances
  when that side-quest is promoted through `scripts/saehrimnir.sh`
  (orchestrator-only; it round-trips through GitHub and reinstalls the
  binary brokkr spawns).

The item is a B15 PREREQUISITE: B15 deletes the `graph` crate, and
`crates/graph/src/group_sync.rs` must not still be a live consumer at that
point. This spec deletes it.

## Required reading (clause 10)

Reviewers and implementers must READ these, not merely acknowledge them:

- `reference/technical-implementation-spec.md` - the contract this document
  is written against.
- `reference/architecture.md` - crate boundaries, the action pipeline, scope
  wiring. Groups do not touch the action pipeline, but the service/core/db
  boundary rules bind every brick here.
- `docs/bifrost-migration.md` - section 7 (the B8 landing accounting and the
  B8-groups carve-out this spec implements), section 11 (freeze discipline,
  side-quest promotion, the B8-groups-SQ accounting this spec consumes).
- `reference/glossary/harness.md` - sync-harness lifecycle, saehrimnir
  orchestration, request-log contract, sync-bench gates. Brick 3 is built on
  it.
- Bifrost side (read in `research/bifrost`, frozen at `59b9e2d`):
  `crates/types/src/directory.rs` (the `DirectoryGroup*` types),
  `crates/types/src/account.rs` lines 654-682 (`directory_groups_list` /
  `directory_group_expand` trait contract),
  `crates/graph/src/account/groups.rs` (the Graph implementation whose
  behavior this consumer inherits), `reference/graph.md` for the error-model
  context of `NoPermission` vs `Unsupported`.
- Saehrimnir side (read in `research/saehrimnir`): `CLAUDE.md` "Groups"
  status section, `src/graph/group_sync.rs`, `src/fixture.rs` (the `Group`
  type, the `ChangeOp` vocabulary, and `record_transition`'s empty-diff
  early return, which O2 turns on), `src/test_admin.rs` (`StepTouches`
  and the atomic-apply / rewind path a new op category must join),
  `src/lua.rs::builder_change`, `notes/fixture-format.md`.
- Ratatoskr side, beyond the files the bricks name:
  `crates/service/src/bifrost/contacts/pull.rs` (the B8 pull this module
  mirrors, including how it opens and commits its transaction),
  `crates/db/src/db/mod.rs` `WriteTarget` / `WriteTxn` / `WriteConn` (O9),
  `crates/core/src/oauth.rs` and `crates/core/src/discovery/registry.rs`
  scope lists (O10), and `crates/app/tests/sync-harness/graph-initial.lua`
  (the gate this item re-baselines, O11).

Not required: `reference/glossary/folders-labels.md` (directory groups live
in `contact_groups`, not the `labels` table) and `UI.md` (no UI work; see
the stopping rule).

## 1. The goal (clause 7: the target as concrete artifacts)

Replace the Graph-specific `/groups` enumeration
(`crates/graph/src/group_sync.rs::sync_exchange_groups`, driven from
`provider-sync/src/graph/aux_sync.rs`) with ONE provider-agnostic pull over
the uniform bifrost surface, mirroring the B8 contact-pull decomposition:

- New module `crates/service/src/bifrost/contacts/groups.rs` (sibling of
  `pull.rs` / `map.rs`), owning:

  ```rust
  /// Provider rows keep the pre-cut source label so existing rows,
  /// the (account_id, server_id) unique index, and every generic
  /// contact_groups consumer (compose expansion, settings UI) are
  /// untouched.
  pub const DIRECTORY_GROUP_SOURCE: &str = "exchange";
  /// Settings-table cycle key, distinct from the contact pull's
  /// `contact_pull_cycle:{account}:{provider}` keys.
  pub const DIRECTORY_GROUP_CYCLE_SOURCE: &str = "directory_groups";
  pub const DIRECTORY_GROUP_CYCLE_DIVISOR: u32 = 20;

  #[must_use]
  pub fn should_pull_groups_on_cycle(cycle: u32) -> bool;
  // cycle.is_multiple_of(DIRECTORY_GROUP_CYCLE_DIVISOR); cycle 0 pulls.

  /// One enumerated group, post-mapping, pre-persist.
  pub(crate) struct PulledGroup {
      pub server_id: String,
      /// display_name, with the legacy "Unnamed Group" fallback applied
      /// when the provider row carried an empty name (obstacle O6).
      pub name: String,
      pub email: Option<String>,
      /// "m365" | "distribution_list" | "mail_security" (obstacle O5).
      pub group_type: &'static str,
      /// Lowercased member emails from the transitive expansion.
      /// `None` = the expansion for THIS group failed transiently;
      /// persist must keep the group row and leave its existing
      /// member rows untouched (legacy semantics).
      pub members: Option<Vec<String>>,
  }

  /// Outcome of one pull, so callers (and the harness ack) can tell a
  /// clean delete-all apart from a capability no-op - both would
  /// otherwise report zero groups with opposite DB effects.
  pub struct GroupPullOutcome {
      /// False when the protocol reported `Unsupported` on the first
      /// call: NOTHING was written, existing rows are intact.
      pub supported: bool,
      /// Groups in the completed snapshot (0 when `supported` is false).
      pub groups: usize,
  }

  /// Enumerate + expand through the engine, then persist the snapshot.
  pub async fn run_group_pull(
      engine: &bifrost_sync::SyncEngine,
      account_id: &str,
      write_db: &service_state::WriteDbState,
  ) -> Result<GroupPullOutcome, String>;

  /// Pure persistence half, unit-testable without an engine: one
  /// transaction doing upserts, member replaces, and the stale prune.
  /// Takes the TRANSACTION handle, not a `WriteConn` - see obstacle O9
  /// for the `db`-side signature work that makes that compile.
  pub(crate) fn persist_group_snapshot(
      tx: &db::db::WriteTxn<'_>,
      account_id: &str,
      pulled: &[PulledGroup],
  ) -> Result<usize, String>;
  ```

- `run_group_pull` data flow, all through the resident engine (no
  provider-crate types anywhere in the new module):
  1. Page `engine.directory_groups_list(&AccountId, cursor)` to
     exhaustion via `Page::next_cursor` (the cursor is opaque bytes; the
     Graph impl stores the verbatim `@odata.nextLink`).
  2. An `Unsupported(_)` recovery class on the FIRST call returns
     `Ok(GroupPullOutcome { supported: false, groups: 0 })` without
     touching the DB (same `is_unsupported` shape as `pull.rs`; JMAP /
     Gmail / IMAP / CardDAV / CalDAV all stub this). Any other error -
     including Graph `NoPermission` on an unconsented tenant - returns
     `Err` and writes NOTHING (obstacle O4).
     ALSO load-bearing, and the hazard O4 exists to prevent: an error on
     ANY page of the enumeration, including page 2 after page 1 already
     succeeded, aborts the whole pull with `Err` and zero writes. A
     partial enumeration must NEVER be treated as a completed snapshot -
     doing so would prune every group that happened to live on the
     unreached pages. The `Unsupported` no-op is recognized on the first
     call only; an `Unsupported` arriving mid-enumeration is a protocol
     contradiction and takes the `Err` path like any other error. Pinned
     by a unit test (section 6).
  3. Per group: map `DirectoryGroupKind` -> `group_type` string, apply
     the empty-name fallback, then page
     `engine.directory_group_expand(&AccountId, group.id, cursor)` to
     exhaustion. An expansion error for one group logs a warning and
     records `members: None` - it never fails the whole pull and never
     wipes that group's existing members (legacy behavior, preserved).
  4. One `write_db.with_write` transaction (`conn.transaction()`, then
     everything against the resulting `WriteTxn`, then `commit` - the
     shape `run_contact_pull` already uses):
     `persist_group_snapshot` upserts each group as
     `ContactGroupRow { id: format!("exchange-{account_id}-{server_id}"),
     source: "exchange", .. }` via the existing `upsert_contact_group`,
     replaces members (`delete_contact_group_members` +
     `insert_contact_group_member_email`) only when `members` is `Some`,
     prunes rows whose `server_id` is absent from the completed
     enumeration (via `list_contact_groups_for_account_by_source` +
     `delete_contact_group_by_id`, FK-cascading members), and treats a
     clean empty enumeration as a real delete-all
     (`delete_contact_groups_for_account_by_source`). The prune's read
     (`list_contact_groups_for_account_by_source`) takes a
     `&ReadConn<'_>`, so it is called as `tx.as_read()` - legacy does
     exactly this. This is the collect-then-single-transaction shape
     `run_contact_pull` already uses; the legacy per-write callback
     plumbing (`ExchangeGroupWrite` / `persist_exchange_group_write`) is
     not carried over. Three of the five named `db` helpers already take
     `&WriteTxn`; two do not, and fixing that is declared work - O9.

     Memory profile of collecting before persisting: every group's
     expanded member list stays resident until the transaction, where
     legacy held one group's expansion at a time. The retained payload is
     lowercased member email strings, so a large tenant (order 100 groups
     x order 1000 members) is single-digit MB - acceptable, and the price
     of the atomicity legacy did not have (legacy's per-write callbacks
     left partial state behind on a mid-run failure). This is a bounded
     accepted cost, not an open question: no disk-backed staging, no
     streaming persist. If a real tenant ever makes it visible, the fix
     is a per-group transaction boundary, which trades the atomicity
     back; do not pre-build it.

- Wiring: `crates/service/src/bifrost/resident.rs::run_aux_pass` gains,
  directly after the existing contact-pull tail, a DB-backed group cycle
  (`next_contact_pull_cycle_sync(conn, account_id, "directory_groups")` -
  the existing settings-table counter, reused with the new source key) and
  calls `run_group_pull` when `should_pull_groups_on_cycle(cycle)`. It runs
  for EVERY provider kind, with no consumer-side provider special-case, per
  the migration doc's section 2 first principle.

  The free no-op for non-directory providers comes from the PROTOCOL
  layer, not the engine: `SyncEngine::directory_groups_list`
  (`crates/sync/src/engine.rs:2115`) is a bare 1:1 forwarder through
  `live_account`, and it is each protocol crate's `Unsupported` stub -
  with `pim_methods.directory_groups_list` left `false`, e.g.
  `crates/jmap/src/sync/capabilities.rs:167` - that returns the no-op.
  The distinction is load-bearing, not pedantic: `is_unsupported` only
  matches `Error::Account(..)` with an `Unsupported` recovery class, so
  an engine-level `live_account` failure (a different `Error` variant) is
  correctly NOT a no-op and falls to the `Err` path with zero writes.

  The pull is NOT gated on `initial_sync_completed`, matching the
  contact-pull tail it sits behind. Note that this is a real behavior
  change, not merely a cadence change: legacy
  `run_graph_auxiliary_sync` returns EARLY (after master-category label
  sync) when `initial_sync_completed_before_run` is false, so a freshly
  added account never synced groups on its first aux pass at all. Under
  this spec it does. That is the intended behavior - a new account should
  get its groups promptly - but it means the first aux pass of a fresh
  Graph account gains provider requests, which is what forces the
  `graph_containers_attach` re-baseline in section 6.

- Deletions (the rip): `crates/graph/src/group_sync.rs` (whole file,
  including its unit tests - the classification logic now lives, tested, in
  bifrost's `crates/graph/src/account/groups.rs`), the `pub mod group_sync;`
  line in `crates/graph/src/lib.rs`, the `group_sync` re-export in
  `crates/provider-sync/src/graph/mod.rs`, and the entire Exchange-groups
  block in `crates/provider-sync/src/graph/aux_sync.rs` (the
  `sync_exchange_groups` call inside the `cycle.is_multiple_of(20)` arm).
  `graph_label_sync` and the reaction refresh in that file stay. Two
  comment corrections while touching the file, not one: the block at
  lines 40-48 still narrates a "contacts delta" that B8 already moved off
  this path, AND line 61's "Master categories + Exchange groups: every
  20th cycle" must lose the groups half. A third stale reference lives
  outside the file: the module doc of
  `crates/db/src/db/queries_extra/calendar_contacts_writes.rs:5` names
  `crates/graph/src/group_sync.rs` as a routed call site - update it in
  the same landing, since the file will not exist.

- `db` signature work (obstacle O9): `upsert_contact_group` and
  `delete_contact_groups_for_account_by_source` become
  `conn: &impl WriteTarget` instead of `conn: &WriteConn<'_>`, matching
  `upsert_contact_sync` / `delete_account_row_sync` and every other
  transaction-capable helper in that crate. Both call sites in the
  deleted `group_sync.rs` go away with it; no other caller changes,
  because `WriteConn` implements `WriteTarget`.

- Harness surface, three pieces:
  1. A new `TestGroupPull` request (service-api `Request` variant +
     `TestGroupPullParams { account_id }` /
     `TestGroupPullAck { groups: u64, supported: bool }`, handler in
     `crates/service/src/handlers/test_helpers.rs` mirroring
     `contact_pull_handle` - attach resident, resolve the action account,
     call `run_group_pull` - and the passthrough in
     `crates/app/src/harness/mod.rs`). `supported` is not decoration: a
     capability no-op and a clean delete-all both report `groups == 0`
     with opposite DB effects, so a script asserting only the count
     proves nothing.
  2. `TestDbContactGroupRow` gains `#[serde(default)] pub member_emails:
     Vec<String>` populated by `read_harness_contact_groups` so gates can
     assert membership without a second request family. Today that
     function reads seven columns in ONE statement; members live in
     `contact_group_members (group_id, member_type, member_value)`, so
     this is a deliberate per-group second query filtered to
     `member_type = 'email'`. The N+1 is accepted: the row limit is
     capped at 200 and this is test-only read-back, not a production
     path.
  3. `contacts.group_save` is exposed in the app harness Lua request
     registry (`crates/app/src/harness/mod.rs:2557`, beside
     `contacts.contact_save` / `contacts.contact_delete`). The
     `ContactsGroupSave` request and `ContactGroupSaveParams` already
     exist in service-api (`crates/service-api/src/request.rs:1183`);
     only the Lua name mapping is missing. Brick 3's reconcile script
     cannot seed its local `source='user'` group without it.

DB schema: ONE change, and it is not this item's invention.
`contact_groups` / `contact_group_members` (v100,
`crates/db/src/db/schema/03_contacts.sql`) already carry everything the
pull needs - `source`, `account_id`, `server_id`, `email`, `group_type`,
the `(account_id, server_id)` unique index, and the group -> members FK
cascade. But `contact_groups.account_id` is plain `TEXT` with NO
foreign key to `accounts(id)`, while `contact_photo_cache.account_id` in
the same file has one. Account deletion
(`delete_account_orchestrate_sync` ->  `delete_account_row_sync`,
`crates/db/src/db/queries_extra/account_delete.rs:173`) leans on
cascades, so every imported Exchange group and its member rows SURVIVE
deleting the account that produced them, then sit orphaned and
unreachable forever (the `(account_id, server_id)` unique index means a
re-added account with the same id even collides with them).

This is a pre-existing v100 defect, not a regression this cut
introduces - legacy `group_sync.rs` wrote the same orphan-prone rows.
It is repaired here anyway because this spec is the item that makes
`contact_groups` a real provider-owned table, v100 is a single
pre-release migration that is still editable, and "every account-scoped
table cascades on account delete" is an architecture invariant. The
change: `account_id TEXT REFERENCES accounts(id) ON DELETE CASCADE`
(nullable, so user-authored `source='user'` groups with a NULL
`account_id` are unaffected), plus a `db` test that seeds an account
with an exchange group + members, deletes the account, and asserts both
tables are empty while a `source='user'` group survives.

## 2. Survey of the ground (clause 8)

### 2.1 The legacy consumer being ripped

`crates/graph/src/group_sync.rs` (614 lines), consumed ONLY by
`crates/provider-sync/src/graph/aux_sync.rs:74` inside
`run_graph_auxiliary_sync`, which the resident aux loop
(`crates/service/src/bifrost/resident.rs::run_aux_pass`, Graph arm) invokes
every `RESIDENT_AUX_CADENCE` (5 min; first pass 5s after attach). The group
block fires when `increment_graph_sync_cycle` (a `sync::state` counter,
which stays - reactions and label sync still use it) hits a multiple of 20.

Behavioral catalog the cut must preserve (each mapped to a gate in
section 6):

1. Enumeration: `GET {prefix}/memberOf/microsoft.graph.group` filtered to
   `mailEnabled eq true`, paged via `@odata.nextLink`.
2. Classification: `Unified` in `groupTypes` -> `m365`; else
   `securityEnabled` -> `mail_security`; else `distribution_list`.
   Mail-disabled groups dropped.
3. Expansion: `GET /groups/{id}/transitiveMembers/microsoft.graph.user`
   (server-side flattening + cycle detection), paged; member email prefers
   `mail` over `userPrincipalName`, skips empty, lowercases; unresolvable
   members dropped.
4. Persistence: local id `exchange-{account_id}-{server_id}`, source
   `exchange`, member replace inside a transaction, stale-group prune by
   seen `server_id` set, `PruneAll` on a clean empty enumeration.
5. Failure isolation: a failed expansion logs and keeps the group's
   existing member rows; it does not fail the run.
6. Legacy display-name fallback: `"Unnamed Group"` for a missing name.

Items 1-3 and the mail-disabled drop, the email preference/lowercasing, and
unresolvable-member drop are now implemented INSIDE bifrost
(`research/bifrost/crates/graph/src/account/groups.rs`, unit-tested there);
the consumer inherits them and must not re-implement them. Items 4-6 are
consumer-side and move to `groups.rs`.

Downstream consumers of the persisted rows - compose-time group expansion
(`db_expand_contact_group*`, `expand_group_with_names_sync`), the settings
UI (`load_groups_for_settings_sync`), `db_find_group_matching_emails` - are
all generic over `contact_groups` and never branch on `source='exchange'`
beyond the sync-side helpers. Preserving the id/source/group_type strings
means ZERO change lands outside the sync path. The user-authored group CRUD
surface (`save_group_sync` / `delete_group_sync`, `source='user'`) is
untouched.

### 2.2 The bifrost surface at the freeze (`59b9e2d`)

- Types (`crates/types/src/directory.rs`): `DirectoryGroupId(pub String)`,
  `DirectoryGroup { id, display_name: String, email: Option<String>, kind,
  provider }`, `#[non_exhaustive] DirectoryGroupKind { Unified,
  DistributionList, MailEnabledSecurity }`,
  `DirectoryGroupMember { email: String /* lowercased, always present */,
  display_name: Option<String> }`.
- Trait (`crates/types/src/account.rs`):
  `directory_groups_list(page_cursor) -> Page<DirectoryGroup>` and
  `directory_group_expand(group, page_cursor) -> Page<DirectoryGroupMember>`,
  paged exactly like `directory_search`. Capability flags
  `pim_methods.directory_groups_list` / `.directory_group_expand`; a
  supporting protocol on an unconsented tenant fails with `NoPermission`
  AT CALL TIME (support is protocol-level, consent is per-tenant runtime
  state - the `/groups` read needs a `GroupMember.Read.All`-class grant,
  harder than `directory_search`'s `User.ReadBasic.All`). The exact
  permission name matters and this spec previously had it wrong: the
  frozen bifrost implementation documents `GroupMember.Read.All`
  (`research/bifrost/crates/graph/src/account/groups.rs:9`), while
  `docs/bifrost-migration.md:1950` says `Group.Read.All`. Ratatoskr
  requests NEITHER; see obstacle O10.
- Engine (`crates/sync/src/engine.rs` ~2110-2140): 1:1 forwarders
  `SyncEngine::directory_groups_list(account_id, cursor)` /
  `directory_group_expand(account_id, group, cursor)` in the contact
  passthrough cluster. No caching, no reconciliation - consumer-side work.
- Graph impl details the consumer leans on: `display_name` defaults to
  `""` (NOT `"Unnamed Group"` - the fallback stays consumer-side, O6),
  `email` is `None` when empty, `failed_ids` is always empty on both pages
  (no per-item hydration on this surface - a page either arrives or the
  call errors), `estimated_total` is `None`.
- Non-Graph protocol crates return `Unsupported` and leave both capability
  flags false.

Ratatoskr already compiles green against this freeze with no consumer-side
change (nothing here implements `Account`; no exhaustive `PimMethodSupport`
literal exists on this side).

### 2.3 The saehrimnir ground

`docs/bifrost-migration.md` section 11 (line 1966) already says the
`/me/memberOf` / `/transitiveMembers` mock routes "are already built",
and already names the same narrower gap this section names. An earlier
draft of this spec claimed the roadmap said the routes were "NOT yet
built" and framed 2.3 as a correction; that sentence exists nowhere in
`docs/` and the framing was a phantom. There is no roadmap sentence to
fix here, and the corresponding stopping-rule doc edit is dropped
(section 5). The SUBSTANCE below - what the mock genuinely lacks - was
verified against the fixture and `serialize_group` and stands.

`research/saehrimnir/src/graph/group_sync.rs` (at `0cf44e4`) serves
`/v1.0/groups`, `/v1.0/groups/{id}`, `/v1.0/groups/{id}/members`,
`/v1.0/groups/{id}/transitiveMembers[/microsoft.graph.user]`,
`/v1.0/me/memberOf[/microsoft.graph.group]`, and
`/v1.0/users/{id}/memberOf[/microsoft.graph.group]`, with Lua override tags
`list_groups` / `get_group` / `list_group_members` / `list_member_of`, and
the fixture carries a cross-account `[[group]]` table (Lua builder
`group({...})`). v0 has no nested groups, so `transitiveMembers` equals the
direct member set and the OData type-casts are all-pass - which matches the
consumer's needs exactly (bifrost relies on the server for flattening).

What is genuinely MISSING for this item's gates (the real side-quest
content, obstacles O1/O2):

- The fixture `Group` has NO `group_types` field and `serialize_group`
  never emits `groupTypes`, so a `Unified` (M365) group cannot be staged:
  bifrost's classifier can only ever see `DistributionList` /
  `MailEnabledSecurity` against today's mock.
- There are no group change-script ops, so a remote group deletion or
  membership change - the prune-stale and member-replace gates - cannot
  be staged mid-run. `ChangeOp`'s complete vocabulary is email / mailbox
  / event / contact / contact-folder / ACL; there is no category op
  either (an earlier draft of this spec listed one - `MutationDiff` has a
  `category_created` field, but no `ChangeOp` produces it, and the
  `StepTouches` match in `src/test_admin.rs:1020` is exhaustive without
  it). That matters because O2 previously cited "the master-categories
  mock" as precedent for a mutating op that skips the change log; that
  precedent does not exist and O2 no longer leans on it.
- Query params on the group routes are parsed-and-ignored (`$filter`,
  `$select`, `$top`) and listing is single-page. Both are acceptable:
  bifrost drops mail-disabled rows client-side, and an absent
  `@odata.nextLink` is a legal one-page enumeration. Not side-quest work.
- Member projection serves `mail` = `userPrincipalName` = `account.name`.
  Acceptable: the preference logic is already pinned by bifrost unit
  tests; the harness asserts lowercasing and presence, not the preference
  branch.

### 2.4 Harness and service scaffolding already in place

- `TestContactPull` (`test_helpers.rs::contact_pull_handle`) is the
  established on-demand trigger pattern; `contacts/graph_pull.lua` shows the
  full script shape including `test/fixture/step` mid-run mutation.
- `TestQueryDbState` already returns `contact_groups`
  (`TestDbContactGroupRow { id, name, source, account_id, server_id, email,
  group_type }`) and a `contact_group_count`; it does NOT expose member
  rows (extended in section 1).
- The DB-backed cadence counter
  (`next_contact_pull_cycle_sync(conn, account_id, source)`, settings key
  `contact_pull_cycle:{account}:{source}`) is generic over its source
  string; the group cycle reuses it under `"directory_groups"` with no new
  DB helper. Restart survival and the "cycle 0 always pulls" property come
  for free, both already unit-pinned for contacts.
- Gate mechanics: `[ratatoskr.gate.contacts_cadence]` in `brokkr.toml` is
  the precedent for a contacts-family sync-bench gate;
  `graph_steady_state_delta`'s baseline label documents that steady-state
  scripts wait out and clear the resident aux pass before the measured
  window - relevant because the group pull now ALSO fires on the first aux
  pass (cycle 0).

## 3. Obstacles resolved inline (clause 2)

- **O1 - Unified groups unstageable in the mock.** Resolved by Brick 1:
  fixture `Group` gains `group_types: Vec<String>` (TOML key
  `group_types = ["Unified"]`, Lua builder key `group_types`, default
  empty), and `serialize_group` always emits `groupTypes` (empty array for
  classic DLs, matching real Graph). Without this, the `m365` arm of the
  classification mapping would be gate-unprovable - a hole, not a note.
- **O2 - remote group deletion / membership change unstageable.** Resolved
  by Brick 1: new change-script ops `group_create` (full `[[group]]`
  shape), `group_update` (sparse: `display_name?`, `mail?`,
  `group_types?`, `members?` = full replace, validated against declared
  accounts), `group_destroy { id }`. Group ops mutate `fixture.groups`
  under the normal step write guard, record NO per-account change-log
  transitions, and touch no push fan-out: no protocol serves a groups
  delta or a groups push, and `accounts_touched` must stay empty so an
  unrelated group edit never advances a mail delta. Observability is via
  re-read of the group routes, not via the change log.

  An earlier draft ALSO required the ops to "bump the primary state
  token". That is self-contradictory and is withdrawn:
  `Fixture::record_transition` (`src/fixture.rs:1118`) returns early on
  an empty `MutationDiff` and explicitly does NOT bump `state`, and the
  primary state IS an account change-log state - so "bump the token but
  record no transition and touch no account" cannot both hold without
  inventing a second, fixture-wide revision counter that nothing reads.
  Group edits leave account state UNCHANGED, full stop; the step
  response's `primary_state` is identical before and after a group step,
  and Brick 1's delta test asserts exactly that.

  Two files the earlier draft omitted from Brick 1 and that the ops
  cannot work without: `src/test_admin.rs`, where `StepTouches`
  (line 1020) classifies which fixture sections a step touches so the
  atomic-apply path can snapshot and rewind them - it needs a `groups`
  category and the three new match arms, or a failed group op will not
  roll back; and `src/lua.rs::builder_change` (line 1392), which is
  where the Lua `group_create` / `group_update` / `group_destroy` step
  keys are actually read.
- **O3 - the 20th-cycle cadence is unreachable inside a 120s harness
  ceiling.** Resolved twice over: gates drive the pull deterministically
  through the new `TestGroupPull` request (the `TestContactPull` pattern),
  and the production cadence itself becomes "cycle 0 pulls" (first aux
  pass, ~5s after attach) so a fresh account gets its groups promptly -
  the same deliberate choice B8 made for contacts. The steady-state
  request-budget gates are unaffected because they already wait out and
  clear the aux window (section 2.4); re-running them is still mandatory
  (section 6).
- **O4 - tenant-consent failure must not destroy local state.** Graph
  distinguishes `Unsupported` (protocol has no surface; permanent) from a
  call-time `NoPermission` (consent missing; potentially transient, admin
  can grant it tomorrow). The consumer maps a FIRST-CALL `Unsupported`
  -> clean no-op `supported: false, groups: 0`, and EVERY other error,
  on ANY page -> `Err` with zero writes. Deleting all groups on
  `NoPermission` would turn a revoked consent into silent local data
  loss; leaving them is the legacy-equivalent behavior (legacy only
  pruned after a SUCCESSFUL enumeration). The mid-enumeration variant of
  the same hazard - page 2 failing after page 1 succeeded, and the
  partial set being mistaken for a completed snapshot - is covered by
  the same rule (section 1, data-flow step 2).

  This guard was previously asserted but never GATED. It now carries
  both a unit test (partial-page abort) and a harness assertion:
  Brick 3's reconcile script drives a baseline Graph pull, arms the
  `list_member_of` override to return a Graph permission-error envelope,
  pulls again, and asserts the request errored AND that every group row
  and member row is byte-identical to the baseline snapshot. Without
  that, an implementation that pruned on `NoPermission` would pass every
  other gate in this spec.
- **O5 - `DirectoryGroupKind` is `#[non_exhaustive]`.** The mapping match
  needs a wildcard arm. Decision: an unknown future kind maps to
  `"distribution_list"` - the neutral "plain mail-enabled group" bucket -
  rather than being dropped (dropping would silently prune the group on
  the next snapshot reconcile, destroying members for a kind bifrost
  deliberately added). Pinned by unit test.
- **O6 - display-name and email divergence.** Bifrost emits
  `display_name: String` defaulting to `""`; legacy wrote
  `"Unnamed Group"`. The consumer applies the fallback at map time
  (`PulledGroup::name`). Pinned by unit test.

  The earlier claim that this keeps the DB shape "identical across the
  cut" was too strong and is corrected: two rows change shape, both
  deliberately.
  1. Legacy `classify_group` applied `"Unnamed Group"` only when
     `displayName` was ABSENT or null; a present-but-empty `displayName`
     was stored verbatim as `""`. Bifrost collapses absent and empty to
     the same `""`, so the consumer's fallback now also rewrites the
     present-but-empty case to `"Unnamed Group"`.
  2. Same class on email: bifrost does `mail.filter(|m| !m.is_empty())`,
     so an empty `mail` arrives as `None` and persists as SQL NULL,
     where legacy stored `Some("")`.
  Both are improvements (an empty-string group name renders as a blank
  row in the settings UI; an empty email is not an email), the affected
  rows are pathological, and no downstream consumer branches on either
  value. Accepted, and named here so a future reader does not read the
  divergence as a bug. Not gate-worthy on its own.
- **O7 - cadence counter isolation.** The group cycle MUST NOT share the
  contact pull's counter rows: sharing a key would double-increment per aux
  pass and silently halve both cadences. Distinct source key
  (`"directory_groups"`), one counter per account (not per provider - a
  single account has exactly one provider), DB-backed for restart survival.
  A counter read/write failure returns cycle 0 and forces a pull,
  conservatively, exactly like contacts.
- **O8 - the side-quest promotion loop.** Brick 1 is authored in
  `research/saehrimnir`, gated by that repo's own `brokkr check`, and is
  DONE only when this repo's Brick 3 harness gates pass against the
  reinstalled binary (the B12 rule, restated in section 11 of the
  migration doc). The promotion (`scripts/saehrimnir.sh`) is
  orchestrator-only and happens between Brick 1 and Brick 3. Brick 2 has no
  mock dependency and can land while the promotion is pending, but Brick 3
  cannot start before it.
- **O9 - the single transaction cannot be built from the helpers as they
  are signed today.** `persist_group_snapshot` needs one `WriteTxn`, but
  the five `db` helpers it calls are split across two handle types:
  `delete_contact_group_members` (`calendar_contacts_writes.rs:979`),
  `insert_contact_group_member_email` (`:989`) and
  `delete_contact_group_by_id` (`:1004`) take `&WriteTxn<'_>`, while
  `upsert_contact_group` (`:954`) and
  `delete_contact_groups_for_account_by_source` (`:1014`) take
  `&WriteConn<'_>`, and a `WriteTxn` cannot be passed to the latter two.
  This is precisely why legacy `group_sync.rs` ran three separate
  transactions instead of one - the atomicity was blocked by a signature,
  not chosen. Resolution: widen those two to `&impl WriteTarget`
  (`crates/db/src/db/mod.rs:197`), the shape `upsert_contact_sync` and
  `delete_account_row_sync` already use; `WriteConn` implements
  `WriteTarget`, so no existing caller changes. The prune's read helper
  `list_contact_groups_for_account_by_source` takes a `&ReadConn<'_>` and
  is reached inside the transaction as `tx.as_read()`.

  Where the code lives: the orchestration (open transaction, loop,
  commit) stays in `service`, matching `run_contact_pull`, which does
  exactly this today. The architecture rule that shared-table SQL and
  transaction-scoped persistence belong to `db` is satisfied by the
  helpers themselves living in `db` - which is why `persist_group_snapshot`
  must call them rather than hand-rolling SQL. It writes NO raw SQL of
  its own.
- **O10 - production OAuth never requests a directory-group scope.**
  `MICROSOFT_GRAPH_SCOPES` (`crates/core/src/oauth.rs:42`) and the
  Outlook discovery registry entry
  (`crates/core/src/discovery/registry.rs:145`) request only Mail /
  MailboxSettings / User.Read-class scopes. Neither
  `GroupMember.Read.All` nor `Group.Read.All` is requested, so a real
  Graph token gets a call-time `NoPermission` on `directory_groups_list`
  and no groups ever import.

  This is PRE-EXISTING, not a regression this cut introduces: legacy
  `sync_exchange_groups` walked the same `/memberOf` route with the same
  token and had the same problem. It is nonetheless in scope, because
  this spec is the item that claims the surface works and gates it.

  Resolution, in Brick 2: add `GroupMember.Read.All` (the name the
  frozen bifrost implementation documents, and the narrower of the two -
  it grants group membership reads without the full directory-object
  read `Group.Read.All` implies) to BOTH scope lists, and correct
  `docs/bifrost-migration.md:1950` to the same name so the roadmap and
  the code agree. Pinned by a scope-list unit test asserting the scope is
  present in both places and that the two lists have not silently
  diverged on it.

  Consequence to state plainly rather than discover in the field:
  existing Microsoft accounts hold tokens issued WITHOUT this scope.
  Adding it does not retroactively grant it - those accounts keep
  failing until the user reauthorizes, and on a managed tenant an admin
  consent may be required. O4 is what makes that acceptable: the failure
  is a logged `Err` with zero writes, repeated every 20th aux cycle,
  never data loss. No forced-reauth prompt is added; the scope simply
  takes effect on the next reauthorization. If the orchestrator prefers
  to defer the scope entirely, the deferral must be written down here -
  the one outcome this spec forbids is landing the surface while
  silently believing production consent exists.
- **O11 - the cycle-0 pull changes an existing gate's measured window.**
  `graph-initial.lua` deliberately WAITS for the resident aux pass
  (it polls until `cat:Work` appears) and then counts EVERY mock request,
  so the aux pass is inside the `graph_containers_attach` window, which
  pins `meta.provider_requests` at `max_delta = 0`. The cycle-0 group
  pull adds at least the `memberOf` enumeration to that window, so the
  gate WILL fail and must be re-baselined with a baseline label naming
  the group pull. This is distinct from O3: the steady-state gates wait
  out and CLEAR the aux window, so they must hold without re-baselining,
  whereas this one deliberately includes it and must be re-recorded.

## 4. The bricks (clause 1), ordered (clause 6)

Three landings; `brokkr check` green at every boundary. Each is one
coherent keep/revert unit.

### Brick 1 - saehrimnir: stageable Unified groups + group mutation ops

Repo: `research/saehrimnir` (edited ONLY there; promoted by the
orchestrator per O8).

1. `src/fixture.rs`: `Group.group_types: Vec<String>` + `RawGroup` /
   TOML loader key + `normalize` passthrough (no validation beyond string
   list - real Graph treats it as an open set).
2. `src/lua.rs::builder_group`: accept optional `group_types` string array.
3. `src/graph/group_sync.rs::serialize_group`: emit `groupTypes` always.
4. `src/fixture.rs`: `ChangeOp::GroupCreate(Box<Group>)`,
   `ChangeOp::GroupUpdate { id, display_name: Option<String>,
   mail: Option<Option<String>>, group_types: Option<Vec<String>>,
   members: Option<Vec<String>> }`, `ChangeOp::GroupDestroy { id }`;
   step keys `group_create` / `group_update` / `group_destroy`; member
   lists validated against declared accounts at step-apply time; no
   change-log transitions, no `accounts_touched`, no state-token bump
   (O2).
5. `src/lua.rs::builder_change` (line 1392): read the three new step
   keys, alongside its existing `contact_update` / `acl_grant` readers,
   with the sparse `group_update` reader distinguishing "absent" from
   "empty list" the way `read_contact_email_array_opt_present` already
   does for contact emails (a `members = {}` must mean "replace with the
   empty set", not "leave members alone").
6. `src/test_admin.rs`: `StepTouches` (line 1020) gains a `groups` field
   and three match arms, and the atomic-apply path snapshots
   `fix.groups` when touched so a failing group op rewinds like every
   other category. Omitting this silently breaks the rollback contract
   for group steps only.
7. Tests (`tests/graph.rs` + the step suite): `groupTypes` projection
   round-trip (TOML and Lua paths byte-equivalent), `memberOf` reflects a
   `group_destroy` and a `group_update` member replace on re-read, ops on
   an unknown group id fail the step AND leave `fix.groups` unchanged
   (the rewind), a `group_update` with `members = {}` empties the group,
   and a group step does NOT advance any account's delta (a follow-up
   `messages/delta` with a pre-step token returns empty, and the step
   response's `primary_state` is unchanged).
8. Docs tag-along: `notes/ratatoskr-graph-surface.md` + the `CLAUDE.md`
   Groups status paragraph + `notes/fixture-format.md` for the new
   `group_types` key and the three step keys.

Gate: `brokkr check` in `research/saehrimnir` (its own suite; that repo
allows mock-server integration tests - the bifrost testing prohibition does
not apply to saehrimnir). Then orchestrator promotion.

### Brick 2 - ratatoskr: the consumer cut

Everything in section 1, in one landing:

1. `crates/db`: widen `upsert_contact_group` and
   `delete_contact_groups_for_account_by_source` to `&impl WriteTarget`
   (O9), add the `account_id` FK on `contact_groups` in the v100 schema
   plus the account-deletion cascade test, and correct the
   `calendar_contacts_writes.rs` module doc that names the file this
   brick deletes. This step comes FIRST - the rest does not compile
   without it.
2. Add `crates/service/src/bifrost/contacts/groups.rs` with the artifacts,
   unit tests included (section 6 names them).
3. Wire `run_aux_pass` (group cycle + `run_group_pull` tail) and the
   `next_group_pull_cycle` sibling helper beside `next_contact_pull_cycle`
   in `resident.rs`.
4. Add `TestGroupPull` end to end (service-api request + params/ack
   carrying `groups` and `supported`, handler dispatch in
   `handlers/mod.rs`, `group_pull_handle` in `test_helpers.rs`, app-side
   harness passthrough), the `member_emails` field on
   `TestDbContactGroupRow` + `read_harness_contact_groups`, and the
   `contacts.group_save` name mapping in the app harness Lua registry.
5. Add `GroupMember.Read.All` to `MICROSOFT_GRAPH_SCOPES` and the Outlook
   discovery registry entry, with the scope-list unit test (O10).
6. Delete `crates/graph/src/group_sync.rs`, its `lib.rs` module line, the
   `provider-sync` re-export, and the aux_sync group block; fix BOTH
   stale comments in `aux_sync.rs` while there (the lines 40-48
   contacts-delta block and the line 61 "Master categories + Exchange
   groups" cadence comment).

This brick is self-contained against the frozen bifrost: unit tests do not
need saehrimnir, and the tree is green even if Brick 1's promotion is still
pending. Revert = revert the one commit.

### Brick 3 - ratatoskr: harness gates + baselines

Requires Brick 1 promoted and Brick 2 landed.

1. Fixture `crates/app/tests/sync-fixtures/graph-groups.lua`: two accounts;
   four groups - `grp-unified` (`group_types = ["Unified"]`,
   mail-enabled), `grp-dl` (plain DL, cross-account membership: both
   accounts), `grp-sec` (mail-enabled + security-enabled), `grp-hidden`
   (mail_enabled = false; must never import); a change script with step 1 =
   `group_destroy grp-sec` + `group_update grp-dl` (drop the second
   member); a `list_group_members` Lua override that fails expansion
   with a Graph error envelope for a deterministic `call_index` window;
   and a `list_member_of` override that fails the ENUMERATION with a
   Graph permission-error envelope for a second window (the O4 gate).

   Call-index arithmetic, corrected - an earlier draft said "pull 3's
   three calls" and that is wrong. Pull 1 expands the three imported
   groups (3 calls). Step 1 destroys `grp-sec`, leaving two imported
   groups, so pull 2 and pull 3 make TWO expansion calls each. The
   transient-failure window is therefore pull 3's two calls: indices
   5 and 6 zero-based (3 + 2 preceding calls), 6 and 7 one-based. State
   the indexing base explicitly in the fixture rather than leaving the
   reader to infer it.

   Second arithmetic trap the fixture must document: saehrimnir aliases
   `/groups/{id}/members`, `/groups/{id}/transitiveMembers`, and the
   `/microsoft.graph.user` cast ALL to the single `list_group_members`
   override tag (`src/graph/group_sync.rs:48-58`), so the call counter is
   shared across every one of those routes, not per-route. The
   enumeration walks a separate tag (`list_member_of`, line 145), so the
   two override windows do not interfere.
2. Script `crates/app/tests/sync-harness/contacts/graph_groups_pull.lua`:
   seed Graph account, `TestGroupPull`, `TestQueryDbState`; assert 3
   imported groups (never `grp-hidden`), `group_type` mapping (`m365` /
   `distribution_list` / `mail_security`), local id shape
   `exchange-{account}-{server_id}`, `source == "exchange"`, group email,
   lowercased `member_emails` with the cross-account member present; assert
   the request log carries the `memberOf` enumeration and one
   `transitiveMembers` request per imported group; summary meta:
   `correct`, `group_count`, `provider_requests`.

   Request-window isolation, mandatory because this script backs a
   `--bench` gate: `TestGroupPull` attaches the resident account, so mail
   startup and the 5s aux task overlap the counted requests unless the
   script fences them out. Follow the pattern
   `graph-steady-state-delta.lua:67` establishes - attach, sleep past
   `RESIDENT_AUX_INITIAL_DELAY`, poll the mock log until it quiesces,
   THEN `harness.clear_mock_requests`, then issue the explicit
   `TestGroupPull`. `meta.provider_requests` must count the explicit pull
   and nothing else, or the baseline measures aux timing jitter and the
   `max_delta = 0` gate flakes.
3. Script
   `crates/app/tests/sync-harness/contacts/graph_groups_reconcile.lua`:
   pull 1 (baseline), apply step 1, pull 2 - assert `grp-sec` pruned with
   its member rows cascaded and `grp-dl`'s member list shrunk; pull 3
   (expansion override window) - assert every surviving group still
   present with pull-2 members INTACT (transient expansion failure keeps
   members); then pull 4 with the `list_member_of` permission-error
   override armed - assert the request FAILS and that the full group +
   member snapshot is IDENTICAL to the pull-3 snapshot, which is the
   only gate on O4's no-data-loss guarantee; finally seed a JMAP account
   plus a local `source='user'` group (via the newly exposed
   `contacts.group_save`) and `TestGroupPull` it - assert ack
   `supported == false` AND `groups == 0` AND the user group untouched.
   The `supported` assertion is the one that carries the signal: a
   delete-all bug also reports `groups == 0`.
4. Script
   `crates/app/tests/sync-harness/contacts/graph_groups_cadence.lua` -
   the only gate on the PRODUCTION wiring. Every other script here calls
   `TestGroupPull` directly, so if the `run_aux_pass` tail were deleted
   or wired behind the wrong cycle predicate, all of them would still
   pass. This one seeds a Graph account, attaches normally, never issues
   `TestGroupPull`, and polls (bounded well inside the 120s ceiling) for
   the cycle-0 group rows to appear from the ~5s aux pass. Assert the
   three groups landed and that the `directory_groups` cadence counter
   advanced independently of the contact-pull counters (O7).
5. `brokkr.toml`: `[ratatoskr.gate.graph_groups_pull]` over script 2 with
   `success == 1`, `exit_code == 0`, `meta.correct == 1`,
   `meta.group_count` equal_to_baseline, `meta.provider_requests`
   `max_delta = 0` (exact-match by design: a new group-surface request
   must be reviewed or re-baselined), plus `elapsed_ms`
   (`max_relative = 1.25` with an absolute `max` ceiling) and
   `sidecar.rss_peak_kb` (`max_relative = 1.20`). The time and memory
   metrics are required by `reference/technical-implementation-spec.md`
   for any `--bench` gate and every steady-state gate in `brokkr.toml`
   carries them; `contacts_cadence` omits them, but that gate measures a
   pure divisor property with no IO in its window, which is not this
   script. The RSS budget also stands in for the collect-then-persist
   memory note in section 1. Record the baseline, then hold the gate.
6. Re-run the aux-adjacent request-budget gates. The list is four, not
   two: `graph_steady_state_delta`, `contacts_cadence`,
   `graph_shared_mailbox_steady_state`, and
   `graph_public_folder_steady_state`. The last two are Graph accounts
   with `meta.provider_requests` at `max_delta = 0` whose cycle-0 aux
   pass now also runs the group pull; they DO wait out and clear the aux
   window (`harness.sleep(6000)` plus a quiesce loop), so the
   expectation is that they hold unchanged - but "expected to hold" is
   what a gate run is for, not a reason to skip it. All four must pass
   WITHOUT re-baselining (O3, O7).
7. RE-BASELINE `graph_containers_attach` (`graph-initial.lua`). Unlike
   the four above, that script deliberately waits FOR the aux pass and
   counts it, so the cycle-0 group pull lands inside its measured window
   and its `max_delta = 0` provider-request budget will fail by design
   (O11). Record a new baseline with a label naming the group pull as the
   cause. This is the one expected baseline movement in the item; any
   OTHER gate needing a re-baseline is a finding, not a formality.

## 5. Stopping rule (clause 9)

- READ-ONLY surface. No group write-back of any kind exists (bifrost has no
  directory-group mutation primitives; real Graph group membership is an
  admin operation). Out of scope permanently, not deferred.
- No UI work. The settings UI and compose expansion consume `contact_groups`
  generically and are untouched.
- Schema scope is exactly ONE line: the missing
  `contact_groups.account_id` FK with `ON DELETE CASCADE` (section 1).
  No migration - v100 is a single pre-release migration edited in place.
  No new columns, no new tables, no index changes.
- No production UI or OAuth flow change beyond adding one scope string to
  the two Microsoft scope lists (O10). No forced re-consent prompt, no
  re-auth nag, no scope-diff detection UI - if a user's token predates
  the scope, groups simply do not import until they reauthorize.
- Google People contact-group labels are NOT this surface (bifrost flattens
  them into `AddressBook`s; B8 handles them); nothing here touches the
  Google pull.
- GAL (`directory_search`) is untouched.
- The `graph` crate's OTHER remaining modules (`client`, `label_sync`,
  reaction refresh, `parse`, `types`) are B15's business; this item removes
  exactly one of B15's prerequisites and stops there.
- Saehrimnir scope stops at O1/O2: no `$filter`/`$select` parsing, no
  multi-page group listing, no nested-group modeling. Bifrost's classifier
  and pager are unit-tested upstream; adding unconsumed mock fidelity is
  shoehorning in reverse.
- `docs/bifrost-migration.md` updates (close the B8-groups TODO entry,
  correct the `Group.Read.All` permission name at line 1950 to
  `GroupMember.Read.All` per O10, record the new freeze references) land
  bundled with Brick 3 per the markdown commit rules, alongside the
  `reference/glossary/harness.md` contacts-section sentence naming the new
  scripts. NOT included, because an earlier draft asked for it and the
  target does not exist: there is no "mock routes are NOT yet built"
  sentence to correct - section 11 line 1966 already says "are already
  built" (section 2.3).

## 6. Verification per brick (clause 5)

Universal gate for every landing: `brokkr check`.

### Brick 1 (run in `research/saehrimnir`)

- `brokkr check` - covers the new fixture field, ops, and the graph/step
  integration tests listed in the brick. The specific new cases:
  `tests/graph.rs` `group_types_projected_on_memberof_and_groups`,
  `group_destroy_disappears_from_memberof`,
  `group_update_replaces_members`,
  `group_update_empty_member_list_clears_members`,
  `group_step_does_not_advance_mail_delta` (asserts both an empty
  `messages/delta` and an unchanged `primary_state`),
  `group_step_unknown_id_fails_and_rewinds` (the `StepTouches` rollback);
  `tests/lua_fixture.rs` equivalence extended to a `group_types`-carrying
  fixture pair.

### Brick 2 (ratatoskr)

Unit tests in `crates/service/src/bifrost/contacts/groups.rs`, each named
and runnable exactly as:

- `brokkr test -p service group_type_str_maps_kinds` - the three known
  kinds plus the wildcard arm (O5) and the empty-name fallback (O6).
- `brokkr test -p service persist_group_snapshot_upserts_and_replaces_members`
  - id shape, source, group_type, member replace semantics.
- `brokkr test -p service persist_group_snapshot_prunes_stale` - a seeded
  group absent from the snapshot is deleted and its members cascade.
- `brokkr test -p service persist_group_snapshot_clean_empty_deletes_all` -
  empty snapshot retires every `source='exchange'` row for the account,
  and ONLY for that account and source (a seeded `source='user'` group and
  a second account's exchange group survive).
- `brokkr test -p service persist_group_snapshot_expansion_failure_keeps_members`
  - `members: None` keeps existing member rows while still upserting the
  group row and protecting it from the prune.
- `brokkr test -p service group_pull_cadence_divisor` - cycle 0 pulls,
  19 does not, 20 does; counter key isolation from the contact pull keys
  (seed both, assert independent progression).
- `brokkr test -p service group_pull_partial_enumeration_writes_nothing` -
  a paged enumeration whose SECOND page errors returns `Err` and leaves
  the DB untouched: seed two groups, serve page 1 carrying only the
  first, fail page 2, assert BOTH seeded groups and all their members
  survive. The single most valuable test in this list; without it, the
  prune-on-partial-snapshot bug ships silently.
- `brokkr test -p service group_pull_unsupported_first_call_is_clean_noop`
  - `supported: false`, `groups: 0`, zero writes, seeded rows intact -
  and its inverse, that an `Unsupported` arriving mid-enumeration takes
  the `Err` path rather than the no-op path.

Elsewhere in the workspace:

- `brokkr test -p db contact_groups_cascade_on_account_delete` - the new
  FK: an account with an exchange group + member rows leaves both tables
  empty after `delete_account_orchestrate_sync`, while a `source='user'`
  group with a NULL `account_id` survives.
- `brokkr test -p rtsk microsoft_scopes_include_group_member_read` - the
  scope string is present in BOTH `MICROSOFT_GRAPH_SCOPES` and the
  Outlook discovery registry entry (O10). Two lists, one assertion each,
  because they have already drifted from each other elsewhere
  (`User.Read` vs `email`).

Compile-boundary proof of the rip: `brokkr check` fails if any
`group_sync` reference survives; no bespoke instrument needed.

### Brick 3 (ratatoskr, after promotion)

These are SYNC-harness scripts (`sync_script_dir =
"crates/app/tests/sync-harness"`), so they run under `brokkr sync`, not
`brokkr service-test` - `service-test` runs the Service-harness scripts
under `crates/app/tests/service-harness/` and takes a directory argument;
`brokkr sync` takes a single script or `--all --filter`. An earlier draft
of this section named `service-test` and a directory cohort, which would
not have run anything.

- `brokkr sync crates/app/tests/sync-harness/contacts/graph_groups_pull.lua`
- `brokkr sync crates/app/tests/sync-harness/contacts/graph_groups_reconcile.lua`
- `brokkr sync crates/app/tests/sync-harness/contacts/graph_groups_cadence.lua`
- `brokkr sync --all --filter contacts` - the whole contacts cohort,
  proving the new scripts coexist with the B8 suite.
- `brokkr sync crates/app/tests/sync-harness/contacts/graph_groups_pull.lua --gate graph_groups_pull --bench --as-baseline`
  once, then
  `brokkr sync crates/app/tests/sync-harness/contacts/graph_groups_pull.lua --gate graph_groups_pull --bench`
  as the held gate (exact `meta.provider_requests`, `meta.group_count`
  equal to baseline, plus `elapsed_ms` and `sidecar.rss_peak_kb`).
- Four gates that must hold WITHOUT re-baselining - the group pull must
  not leak into any measured window (O3) nor disturb the contact cadence
  counters (O7):
  `brokkr sync crates/app/tests/sync-harness/graph-steady-state-delta.lua --gate graph_steady_state_delta --bench`,
  `brokkr sync crates/app/tests/sync-harness/contacts_cadence.lua --gate contacts_cadence --bench`,
  `brokkr sync crates/app/tests/sync-harness/graph-shared-mailbox-steady-state.lua --gate graph_shared_mailbox_steady_state --bench`,
  `brokkr sync crates/app/tests/sync-harness/graph-public-folder-steady-state.lua --gate graph_public_folder_steady_state --bench`.
- ONE gate that must be re-baselined, deliberately (O11):
  `brokkr sync crates/app/tests/sync-harness/graph-initial.lua --gate graph_containers_attach --bench --as-baseline`,
  then held. Its script waits for and counts the aux pass, so the
  cycle-0 group pull is inside its budget by design.

## 7. Stance

Structural over micro: the item deletes the last Graph-specific aux-sync
enumeration and moves the fourth and final contacts-family surface (contact
folders, contacts, GAL - all B8 - and now groups) onto the one engine
passthrough cluster, which is precisely the shape B15 needs to delete the
provider crate. No compatibility shims: the legacy write-callback plumbing
dies with the file, the provider-request shape is re-pinned by a fresh
baseline rather than mimicked request-for-request, and the only
deliberately preserved legacy artifacts are the DB strings
(the `exchange-` id prefix, `source='exchange'`, the three `group_type`
values)
because live rows and generic consumers key on them - preserving those is
data compatibility, not abstraction protection.

## 8. Review disposition

Two independent reviews (R1, Opus; R2, codex gpt-5.6-sol xhigh) were run
against the draft above. Every finding was re-verified against the tree
before folding. This section records what was NOT taken as stated, so a
later reader does not re-litigate it.

Rejected outright:

- **"The whole snapshot transaction should move into a new `db` API,
  with the DTO and tests in `db`" (R2-2, second half).** The signature
  problem is real and is O9. The relocation is not taken: `run_contact_pull`
  - the direct B8 sibling this module mirrors, landed under the same
  architecture rules - opens its transaction, loops, and commits in
  `service`, calling `db` helpers throughout. Moving groups alone to a
  different shape would make the two halves of one subsystem diverge for
  no behavioral gain. The boundary rule is satisfied by every statement
  living in a `db` helper: `persist_group_snapshot` writes no SQL of its
  own. If the orchestration shape is genuinely wrong, that is a finding
  against `pull.rs` and belongs in its own item, applied to both.
- **"Bounded disk-backed staging for the collected memberships" (R2-9).**
  The retained payload is lowercased email strings for the groups one
  mailbox belongs to; a large tenant is single-digit MB. Disk-backed
  staging for that is machinery with no load on it. The scale concern is
  acknowledged in section 1 as an accepted bound plus an RSS budget on
  the gate, which is the proportionate response.

Taken, but re-framed rather than accepted at the stated severity:

- **OAuth scope missing (R2-1, "Critical").** The gap is real and is
  O10, but it is PRE-EXISTING: legacy `sync_exchange_groups` walked the
  same route with the same token and the same missing scope. This item
  does not introduce the break; it inherits it and fixes it. The
  reviewer's `Group.Read.All` / `GroupMember.Read.All` discrepancy is
  real and resolved in favor of the narrower `GroupMember.Read.All`.
- **`contact_groups.account_id` orphaning (R2-6, "High").** Real, verified,
  and fixed here - but also pre-existing v100, not caused by the cut, and
  it is a one-line schema change rather than the migration the phrasing
  implies.
- **Gate metric completeness (R2-8, first half).** `elapsed_ms` and
  `sidecar.rss_peak_kb` are added, but the claim that the contract
  requires them of every gate is too strong: `contacts_cadence` holds
  today with neither. They are added because this gate measures real IO,
  not because a blanket rule forces them.

Found during validation, in neither review:

- **`graph_containers_attach` will fail and needs re-baselining (O11).**
  R1 flagged the two namespace steady-state gates as missing from the
  re-run list; both reviews missed that `graph-initial.lua` deliberately
  waits for and COUNTS the aux pass, so its `max_delta = 0`
  provider-request budget breaks the moment a cycle-0 group pull exists.
  That gate is the one expected baseline movement in the item.
- **The cycle-0 pull is a behavior change, not just a cadence change.**
  Legacy `run_graph_auxiliary_sync` returns early when
  `initial_sync_completed_before_run` is false, so a fresh account never
  synced groups on its first aux pass at all. The draft described the
  cycle-0 choice as purely a cadence decision; it is also a new-account
  behavior change, and it is what makes O11 bite.
- **`ChangeOp` has no category vocabulary.** The draft listed one and
  cited the master-categories mock as precedent for a change-log-free
  mutating op. `MutationDiff` carries a `category_created` field, but no
  `ChangeOp` produces it and `StepTouches`'s match is exhaustive without
  it. O2 no longer leans on that precedent.
