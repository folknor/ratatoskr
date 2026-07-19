# B13 technical-implementation-spec: identities, signatures, vacation, quota

Closes the "identities, signatures, vacation, quota" seam listed in
`docs/bifrost-migration.md` § 5 and enumerated as § 7 item B13 ("Identities,
signatures, vacation, quota. Rewire onto the bifrost settings surface. Needs
B1."). Unlike B11 (a pure dead-code close) and unlike B9's cloud-hosting pin,
this seam is MIXED: it has ONE genuinely live provider caller (the Gmail
`sendAs` bidirectional signature reconciliation, run every auxiliary kick under
the resident engine), plus several hand-rolled provider duplicates that are
DEAD (zero callers), plus one bifrost surface (`quota_get`) that ratatoskr has
no equivalent for at all. The correct disposition therefore combines all three
of the precedents Track B has already established:

- a REAL caller rewire (B4/B5/B6/B8 shape) for the live Gmail signature sync -
  move it off the hand-rolled `gmail::api::{list_send_as, update_send_as_signature}`
  onto bifrost's `identities_list` / `identity_update`;
- a maximal-integration DELETE (B11 shape) for the dead hand-rolled duplicates -
  `crates/jmap/src/signatures.rs` (a JMAP `Identity/get` + `Identity/set` CRUD
  module with no caller) and the dead per-provider vacation surface in
  `crates/core/src/auto_responses.rs`;
- a capability-gated PIN (B9/B11 shape) for the surfaces with no live caller -
  `vacation_get` / `vacation_set` / `quota_get` behind a `#[allow(dead_code)]`
  ratatoskr forwarder, so a future account-settings product item consumes
  bifrost's surface rather than reviving a hand-rolled one.

The unifying vehicle is a single new `crates/service/src/bifrost/settings.rs`
module holding an `AccountSettingsSurface` (structural sibling of B9's
`AttachmentByteSource` and B11's `ServerFilterSurface`) that forwards all five
settings primitives through the resident `SyncEngine`. Two of its methods
(`identities_list`, `identity_update`) gain a live caller in this item (the
rewired Gmail signature sync); the other three (`vacation_get`, `vacation_set`,
`quota_get`) are pinned without a caller.

This spec is written against `reference/technical-implementation-spec.md` (the
contract it must satisfy - READ IT) and conforms to its ten clauses. It is one
item of `docs/bifrost-migration.md` (the governing plan and TODO source - READ
§ 1, § 3, § 5, § 7 B9/B11/B13, § 8, § 11), run through the `orchestrate`
procedure (the standing spec-loop workflow per the project CLAUDE.md).

## Required reading (clause 10)

Every implementer and reviewer MUST read these before laying a brick. They are
the ground this work is built on and judged against; naming them is not enough.

- `reference/technical-implementation-spec.md` - the contract this spec is
  written against. Clause 3 (no deferral; separate-TODO work is named and
  excluded), clause 4 (no shoehorning), clause 8 (survey the ground; reconcile
  against sibling surveys), and clause 9 (a bounded stopping rule) are the
  load-bearing clauses for a mixed rewire-delete-pin item.
- `reference/architecture.md` - ALWAYS required. The `core`/`app` firewall (the
  app depends on `rtsk` + `service-api` wire types only, never bifrost), the
  crate map, and the `MailActionIntent -> resolve_intent ->
  build_execution_plan -> batch_execute` action pipeline (which account settings
  DO NOT flow through - identities/signatures/vacation/quota are an
  account-settings surface, not a per-message action) all bind where the rewired
  and pinned surfaces may live.
- `docs/bifrost-migration.md` - the TODO source. § 1 (feature-preserving AND
  maximal-integration - both bind here, and they pull in opposite directions the
  spec must reconcile: preserve the ONE live Gmail signature feature, delete the
  dead duplicates, add NO absent settings UI), § 3 (target architecture / the
  seam), § 5 ("Rewired: ... identities/settings"), § 7 B9 (the pinned-surface
  precedent, lines 1473-1487), B11 (the delete-duplicate-plus-pin precedent,
  line 1501-1507), and B13 (line 1510-1511), § 8 (sequencing), § 11 (the bifrost
  freeze; B13 advances it - see below).
- `research/bifrost/crates/types/src/settings.rs` (frozen tree) - the bifrost
  settings model B13 consumes: `Identity` (id / name / address /
  `signature_text` / `signature_html` / `reply_to` / `is_default`),
  `IdentityPatch` (double-`Option` partial patch), `VacationConfig`, `QuotaInfo`.
- `research/bifrost/crates/types/src/account.rs` (frozen tree) - the five
  `Account` methods B13 pins/rewires onto: `identities_list` (`:538`),
  `identity_update` (`:542`), `vacation_get` (`:549`), `vacation_set` (`:552`),
  `quota_get` (`:555`), plus the `IdentityId` compose type used by
  `identity_update`.
- `research/bifrost/crates/types/src/capabilities.rs` (frozen tree) - the
  `PimMethodSupport` gates (`:204-208`: `identities_list`, `identity_update`,
  `vacation_get`, `vacation_set`, `quota_get`) that each Account advertises;
  the pinned/rewired forwarders are capability-dispatched exactly as B8's
  `directory_search` and B11's `filter_*` are.
- `research/bifrost/crates/sync/src/engine.rs` (frozen tree) - the `SyncEngine`
  passthrough cluster the B13-SQ extends. `directory_search` (`:2049-2060`) is
  the exact shape the new settings passthroughs mirror: `pub async fn`,
  `Ok(self.live_account(account_id)?.<method>(args).await?)`, FLATTENING the
  trait's `AccountError` into `bifrost_sync::Error::Account` via `?` (NOT
  `host_attachment`'s nested `pub fn` shape). The five settings methods follow
  `directory_search`, not `host_attachment` (§ 4.1).
- `research/bifrost/reference/sync.md` (frozen tree) - REQUIRED per
  `research/bifrost/AGENTS.md`. Its direct-passthrough cluster section documents
  the flattened engine-`Error` contract the new settings cluster joins ("returns
  the engine `Error`; the trait's `AccountError` folds in through `?`"). The
  B13-SQ extends this sheet's cluster enumeration to name the settings cluster.
- `crates/service/src/bifrost/attachment.rs` and
  `crates/service/src/bifrost/server_filters.rs` (created by B11, if landed; else
  its spec) - the two ratatoskr-side pinned-surface precedents. B13's
  `AccountSettingsSurface` is a direct structural copy of `ServerFilterSurface`'s
  two-variant-error, `action_account.engine.*`-forwarding shape.
- `crates/service/src/bifrost/resident.rs` - `ResidentEngine::action_account`
  (`:355`) returns `ResidentActionAccount { engine: Arc<SyncEngine>, .. }`, and
  `run_aux_pass` (`:977`) is the live auxiliary loop that today dispatches
  `provider_sync::consumer_support::run_gmail_auxiliary_sync` (`:1044`) - the
  entry point the rewired Gmail signature sync is reached through. READ this: the
  rewire's blast radius is exactly this dispatch plus the Gmail aux arm.
- `crates/provider-sync/src/gmail/aux_sync.rs` - the LIVE Gmail signature
  reconciliation (`sync_gmail_signatures`), its conflict-resolution state machine
  (`determine_sync_action`), and its two hand-rolled Gmail-API calls
  (`list_send_as`, `update_send_as_signature`) that this item rewires onto
  bifrost.
- `reference/glossary/harness.md` - REQUIRED. The sync-harness Lua model,
  `saehrimnir` mock-provider server, `brokkr service-suite`, and gate baselines;
  the Gmail signature round-trip gate (§ 4.2, § 6) and the green-tree backstop
  are defined against this doc. The rewrite of
  `gmail-send-as-multi-account-import.lua` (§ 4.2) is judged against it.
- `reference/glossary/folders-labels.md` - REQUIRED only insofar as a future
  identity/reply-to mapping touches system-folder routing; B13 adds no such
  mapping (it rewires signatures and pins the raw settings surface), but a
  reviewer judging the `Identity`/`send_identities` relationship needs the model.
- `docs/roadmap/signatures.md` - REQUIRED (the feature-area design doc for the
  signature seam B13 rewires, per contract clause 10). NOTE: this doc is STALE
  against the tree and B13 reconciles it (§ 4.7). It currently claims (`:4`, `:34`,
  `:35`) that BOTH Gmail bidirectional sync AND "JMAP Identity signature sync" are
  "live" end-to-end. The Gmail path IS live (§ 2.1); the JMAP path
  (`sync_jmap_identity_signatures` in `crates/jmap/src/signatures.rs`) is DEAD -
  zero callers (§ 2.2), so the doc's "[x] JMAP Identity signature sync ... live"
  claim is already inaccurate today, before B13 deletes it. B13 corrects this doc
  as part of the landing; a reviewer must not read its "JMAP sync is live" line as
  a feature-preservation obligation that blocks the § 4.4 deletion (it is stale
  documentation of dead code, not a shipped feature - confirmed by § 2.2's
  zero-caller sweep).

The `../bifrost` dependency checkout is frozen for the full duration of this
item per `docs/bifrost-migration.md` § 11. The current frozen reference is
`bc97132` (§ 11, the B11-SQ commit, the sixteenth advance). B13 consumes a
bifrost surface AND adds a bifrost side-quest (the five `SyncEngine` settings
passthroughs, § 4.1), so the freeze ADVANCES here for the seventeenth time;
record the exact new frozen commit in the ground survey of the landing (§ 3) and
in the § 11 ledger, as every prior Track B item did.

## 1. The goal (clause 7: the target as concrete artifacts)

Today the "identities, signatures, vacation, quota" work of
`docs/bifrost-migration.md` § 5 is distributed across the tree as follows:

- LIVE GMAIL SIGNATURE RECONCILIATION (the ONE live caller). The resident aux
  loop (`resident.rs:977 run_aux_pass` -> `:1044
  run_gmail_auxiliary_sync`) calls
  `provider_sync::gmail::aux_sync::sync_gmail_signatures` every kick. That
  function reconciles the local `signatures` table against the account's Gmail
  `sendAs` aliases with a hash-based conflict state machine (`determine_sync_action`:
  NoOp / PullFromServer / PushToServer / ConflictServerWins), reading via
  `gmail::api::list_send_as` (`GET /settings/sendAs`) and writing via
  `gmail::api::update_send_as_signature` (`PUT /settings/sendAs/{email}`). Both
  Gmail-API methods, and the `GmailSendAs` / `ListSendAsResponse` types
  (`gmail/src/types.rs:199-213`), are hand-rolled duplicates of bifrost's
  Google `identities_list` / `identity_update` (whose Account impl maps exactly
  `users.settings.sendAs.list`, per `settings.rs:4`). This is a § 1
  maximal-integration target AND a live feature to preserve.
- DEAD HAND-ROLLED JMAP IDENTITY/SIGNATURE CRUD (no caller). `crates/jmap/src/signatures.rs`
  (`sync_jmap_identity_signatures`, `push_signature_to_jmap`) drives
  `bifrost_jmap::identity::{IdentityGet, IdentitySet}` directly and upserts into
  the `signatures` table. It is declared `pub mod signatures;` at
  `crates/jmap/src/lib.rs:14` and is called by NOTHING (verified § 2.2). This is
  the JMAP twin of the Gmail path, but dead - the § 11 B11-mirror deletion.
- DEAD PER-PROVIDER VACATION SURFACE (no caller). `crates/core/src/auto_responses.rs`
  holds `fetch_{graph,gmail,jmap}_auto_response` / `push_{graph,gmail,jmap}_auto_response`
  over a unified `AutoResponseConfig`, driving Graph `mailboxSettings`, Gmail
  `settings/vacation`, and JMAP `VacationResponse/set` directly. NONE of the six
  provider functions has a caller (verified § 2.3). They are hand-rolled
  duplicates of bifrost's `vacation_get` / `vacation_set`. The module's ONLY
  live export is `any_auto_response_active` (`:45`), which reads the LOCAL
  `auto_responses` table for a status-bar indicator; that table has no live
  writer (`upsert_auto_response_sync` is dead too, § 2.3), so the indicator is
  effectively always-empty app-level scaffolding.
- IDENTITIES / SEND-AS TABLE, UNWIRED IMPORT. The `send_identities` table
  (`schema/04_compose.sql:36`) is READ by compose (from/reply-to detection,
  `get_send_identities` / `get_send_identities_read`) but has NO production
  writer (only `handlers/test_helpers.rs:1736` inserts; verified § 2.4). Identity
  import is not a live feature. `service/src/send.rs:71` states explicitly that
  `send_as`/`identity` "stay default until B12/B13", confirming this seam is
  where identity wiring was deferred to.
- NO QUOTA SURFACE. `quota` has zero references in ratatoskr's crate tree
  (verified § 2.5). Bifrost's `quota_get` has no ratatoskr equivalent, live or
  dead.

After B13, the state is:

- A bifrost `SyncEngine::{identities_list, identity_update, vacation_get,
  vacation_set, quota_get}` passthrough cluster exists (B13-SQ, § 4.1),
  mirroring `directory_search`.
- A ratatoskr `crates/service/src/bifrost/settings.rs` module (§ 4.3) holds an
  `AccountSettingsSurface` whose five methods forward through
  `action_account.engine.*`, capability-dispatched. `identities` /
  `identity_update` have a live caller; `vacation_get` / `vacation_set` /
  `quota_get` are `#[allow(dead_code)]` pins with NO caller.
- The live Gmail signature reconciliation is REWIRED (§ 4.2) to source server
  identities via `AccountSettingsSurface::identities` (mapping
  `Identity.signature_html` / `signature_text` / `address` / `is_default`) and to
  push via `AccountSettingsSurface::identity_update` (setting
  `IdentityPatch.signature_html`). The reconciliation state machine and the local
  `signatures` table are UNCHANGED in behavior. The hand-rolled
  `gmail::api::{list_send_as, update_send_as_signature}` and the `GmailSendAs` /
  `ListSendAsResponse` types are DELETED.
- The dead `crates/jmap/src/signatures.rs` and its `pub mod signatures;` line are
  DELETED (§ 4.4).
- The dead per-provider vacation surface in `crates/core/src/auto_responses.rs`
  (`fetch_*` / `push_*`) is DELETED (§ 4.5). The local `auto_responses` table,
  `any_auto_response_active`, and the dead-but-local `upsert_auto_response_sync`
  are LEFT as app-level scaffolding (mirrors B11 keeping the demo Filters tab and
  local `sync/filters.rs`).
- The § 4.6 invariant gate locks the deletions and routes all five engine
  settings methods through `settings.rs` only.

The concrete artifacts B13 produces:

1. (bifrost, B13-SQ) five `SyncEngine` methods, edited into
   `research/bifrost/crates/sync/src/engine.rs` (the SQ staging copy; promoted to
   `../bifrost` by `scripts/bifrost.sh`), mirroring `directory_search` (async,
   awaited, flattened error):

```rust
pub async fn identities_list(&self, account_id: &AccountId)
    -> Result<Vec<bifrost_types::Identity>, Error>;
pub async fn identity_update(&self, account_id: &AccountId, identity: bifrost_types::IdentityId, patch: bifrost_types::IdentityPatch)
    -> Result<(), Error>;
pub async fn vacation_get(&self, account_id: &AccountId)
    -> Result<Option<bifrost_types::VacationConfig>, Error>;
pub async fn vacation_set(&self, account_id: &AccountId, config: bifrost_types::VacationConfig)
    -> Result<(), Error>;
pub async fn quota_get(&self, account_id: &AccountId)
    -> Result<Option<bifrost_types::QuotaInfo>, Error>;
```

   Each body is `Ok(self.live_account(account_id)?.<method>(args).await?)`,
   identical in shape to `directory_search` (`:2049-2060`): the synchronous
   `live_account` surfaces `AccountNotAttached` up front, the awaited
   `AccountFuture` surfaces the trait's `AccountError` (including `Unsupported`
   for accounts whose `PimMethodSupport` flag for the method is false) folded
   into `bifrost_sync::Error::Account` via `?`.

2. (ratatoskr) `crates/service/src/bifrost/settings.rs`:

```rust
pub(crate) struct AccountSettingsSurface {
    resident: ResidentEngine,
}

impl AccountSettingsSurface {
    pub(crate) fn new(resident: ResidentEngine) -> Self { Self { resident } }

    // Live callers (the rewired Gmail signature sync, § 4.2):
    pub(crate) async fn identities(&self, account_id: &str)
        -> Result<Vec<Identity>, AccountSettingsError> { /* forward */ }
    pub(crate) async fn identity_update(&self, account_id: &str, identity: IdentityId, patch: IdentityPatch)
        -> Result<(), AccountSettingsError> { /* forward */ }

    // Pinned, no caller (future account-settings item):
    #[allow(dead_code)] // pinned vacation surface; B13 adds no UI caller
    pub(crate) async fn vacation_get(&self, account_id: &str)
        -> Result<Option<VacationConfig>, AccountSettingsError> { /* forward */ }
    #[allow(dead_code)] // pinned vacation surface; B13 adds no UI caller
    pub(crate) async fn vacation_set(&self, account_id: &str, config: VacationConfig)
        -> Result<(), AccountSettingsError> { /* forward */ }
    #[allow(dead_code)] // pinned quota surface; B13 adds no UI caller
    pub(crate) async fn quota_get(&self, account_id: &str)
        -> Result<Option<QuotaInfo>, AccountSettingsError> { /* forward */ }
}
```

   Each method resolves `let action_account =
   self.resident.action_account(account_id).await.map_err(AccountSettingsError::Attach)?;`
   builds `let account = AccountId(account_id.to_string());`, and forwards
   `action_account.engine.<method>(&account, <args>).await.map_err(AccountSettingsError::Engine)`.
   `AccountSettingsError` is a TWO-variant enum (`Attach(String)`,
   `Engine(bifrost_sync::Error)`), matching B11's `ServerFilterError` exactly -
   the flattened `directory_search`-shaped engine methods fold `AccountError`
   (with its `RecoveryClass`) NESTED into `bifrost_sync::Error::Account`, so
   `Unsupported` and every account error survive inside the `Engine` variant, NOT
   as a distinct top-level `Account` arm.

3. (ratatoskr) rewired `sync_gmail_signatures` (§ 4.2) sourcing/sinking through
   `AccountSettingsSurface`, plus deletion of `gmail::api::{list_send_as,
   update_send_as_signature}` and `GmailSendAs` / `ListSendAsResponse`.

4. Deletion of `crates/jmap/src/signatures.rs` + its `pub mod signatures;` line
   (§ 4.4), and of the dead `fetch_*` / `push_*` provider functions in
   `crates/core/src/auto_responses.rs` (§ 4.5).

5. The § 4.6 invariant gate, and the rewritten Gmail signature harness gate
   (§ 4.2).

There is NO new wire message, DB table, cursor, schema, action type, or
`app`/`service-api` change. `Identity` / `VacationConfig` / `QuotaInfo` do NOT
cross the `service-api` wire in B13 - the local `signatures` table remains the
only thing the app reads, and the vacation/quota surfaces have no caller - so no
wire type is minted. That is a future account-settings item's job (§ 5).

## 2. Survey of the ground (clause 8)

The survey must be falsifiable: it must show which occupants of the seam are live
(exactly one - the Gmail signature sync) and which are dead (all the rest), so
the rewire/delete/pin split is correct and drops no load-bearing work.

### 2.1 The Gmail signature reconciliation is the one live provider caller

`resident.rs:977 run_aux_pass` dispatches per provider; the Gmail arm (`:1044`)
calls `provider_sync::consumer_support::run_gmail_auxiliary_sync`, which
(`consumer_support.rs:102`) calls `gmail::aux_sync::run_gmail_auxiliary_sync`,
whose FIRST action (`aux_sync.rs:18`) is `sync_gmail_signatures`, unconditionally
every kick (non-fatal on error). That function (`aux_sync.rs:27-111`):

- lists server aliases via `client.list_send_as(read_db)` (`gmail/src/api.rs:288`,
  `GET /settings/sendAs`);
- reads local `signatures` rows with a non-null `server_id`
  (`read_local_signatures`);
- for each alias, runs `determine_sync_action` (hash of server HTML vs stored
  `server_html_hash`) yielding NoOp / PullFromServer / PushToServer /
  ConflictServerWins;
- upserts server-wins rows into `signatures` (`upsert_signature_from_server`);
- pushes local-wins HTML via `client.update_send_as_signature`
  (`gmail/src/api.rs:297`, `PUT /settings/sendAs/{email}`).

This is a genuine live feature (the `gmail-send-as-multi-account-import.lua`
sync-harness gate exercises it end-to-end today). It must be PRESERVED (§ 1
feature-preserving), and its two hand-rolled Gmail-API calls are exactly the § 1
maximal-integration duplicates of bifrost `identities_list` /
`identity_update` - which is why B13 rewires rather than pins here. The other
three providers' aux passes do NOT touch signatures: JMAP aux runs
shared-account discovery / identity resolution / share-notification polling only
(`consumer_support.rs:69-73`); Graph and IMAP aux run folder-map / PERMANENTFLAGS
probes. Gmail is the sole live signature/identity provider caller.

### 2.2 `crates/jmap/src/signatures.rs` has zero callers (dead duplicate)

`sync_jmap_identity_signatures` and `push_signature_to_jmap` are declared `pub`
in `crates/jmap/src/signatures.rs` (`pub mod signatures;` at
`crates/jmap/src/lib.rs:14`) and are invoked by NOTHING in `core`, `service`,
`provider-sync`, `app`, or `sync` (workspace sweep: the only hits for both symbols
are their own definitions). The module drives `bifrost_jmap::identity::{IdentityGet,
IdentitySet}` directly - the JMAP twin of the Gmail signature path, but dead. It
is the B11-mirror maximal-integration deletion (delete the dead duplicate now;
the whole `jmap` crate retires at B15). Deleting it removes no reachable
behavior. (Mechanized as the § 4.6 gate so this stays true.)

CAVEAT - a SECOND `bifrost_jmap::identity` occupant survives and must NOT be swept
up. `crates/jmap/src/helpers.rs:1` imports `bifrost_jmap::identity::IdentityGet`
and `get_first_identity_id` (`helpers.rs:58`) executes an `Identity/get` to resolve
the identity id for EMAIL SUBMISSION (reached from `JmapOps::send_email`,
`crates/jmap/src/ops.rs:370`). This is NOT the signature/settings CRUD B13 deletes
- it is send-path identity RESOLUTION (pick an identity to send AS), a different
concern B13 does not touch (send-identity population is § 5 / B12 territory,
§ 2.4). B13 deletes only `signatures.rs` (the `Identity/set` signature writer + its
`Identity/get` signature reader), leaving `helpers.rs` intact. The § 4.6 gate MUST
allow-list `crates/jmap/src/helpers.rs`'s `IdentityGet` use, or it false-positives
on a legitimate surviving submission-path call (§ 4.6 item 2 is corrected
accordingly). Whether `get_first_identity_id` has a live runtime caller today is
immaterial to B13 - it is out of the signature/settings seam either way; its
disposition belongs to the JMAP send-path / B15 crate retirement, not here.

### 2.3 The per-provider vacation surface is dead; only the local read is live

`crates/core/src/auto_responses.rs` exposes six provider functions
(`fetch_{graph,gmail,jmap}_auto_response`, `push_{graph,gmail,jmap}_auto_response`).
A workspace sweep finds NO caller for any of the six (the only hits are their own
definitions). They drive Graph `PATCH /me/mailboxSettings`, Gmail `PUT
/settings/vacation`, and JMAP `VacationResponse/set` directly - hand-rolled
duplicates of bifrost `vacation_get` / `vacation_set`. The module's only LIVE
export is `any_auto_response_active` (`:45`), reached from
`app/src/db/accounts.rs:165` and `app/src/handlers/core.rs:566` (a status-bar
indicator). It reads the local `auto_responses` table via
`db_..::auto_responses::any_auto_response_active_sync`. That table's only writer,
`upsert_auto_response_sync` (`db/src/db/queries_extra/auto_responses.rs:63`), has
NO caller either - so the indicator is always-empty scaffolding today. B13 DELETES
the six dead provider functions (the bifrost duplicates) and LEAVES the local
table + read + dead upsert as app-level scaffolding (the § 2 analogue of B11
keeping the demo Filters tab and dormant `sync/filters.rs`): removing the local
surface would be scope creep (a UI/status-bar behavior change), and wiring it to
bifrost `vacation_get` would ADD the absent settings feature (forbidden by § 1).

### 2.4 The `send_identities` table has no production writer (unwired import)

`send_identities` (`schema/04_compose.sql:36`) is READ by compose participant /
from-address detection (`get_send_identities`, `get_send_identities_read`,
`thread_detail.rs:449`) but has NO production INSERT: the only writer in the tree
is `handlers/test_helpers.rs:1736`. Identity import from the provider is NOT a
live feature (confirmed by `send.rs:71`, which pins `send_as`/`identity` to
protocol defaults "until B12/B13"). B13 therefore does NOT wire a
`send_identities` populate path - that would ADD an absent feature (§ 1
feature-preserving). The identity LISTING surface is pinned onto bifrost
`identities_list` via `AccountSettingsSurface::identities` (which the Gmail
signature rewire already exercises); populating `send_identities` from it is a
future account-settings/compose item (§ 5).

### 2.5 Quota has no ratatoskr surface at all (pure pin)

`quota` has zero references anywhere under `crates/*/src` (the only tree hit is
`process-lifetime/src/windows.rs`, an OS process-quota concept, unrelated).
Bifrost's `quota_get` is a new capability with no ratatoskr equivalent. B13 pins
it behind `AccountSettingsSurface::quota_get` (`#[allow(dead_code)]`, no caller) -
the pure-pin disposition of B9's `host_large_attachment`.

### 2.6 The `SyncEngine` has no settings passthrough yet (the B13-SQ gap)

`research/bifrost/crates/sync/src/engine.rs` exposes `host_attachment` (`:1515`),
`directory_search` (`:2049`), and the B11 `filter_*` cluster (`:2066-2118`) but
NO `identities_list` / `identity_update` / `vacation_get` / `vacation_set` /
`quota_get`. The five `Account` trait methods exist and every provider Account
impl implements them (capability-gated via `PimMethodSupport`, `capabilities.rs:204-208`),
but the engine passthrough is missing. The pinned/rewired ratatoskr forwarder
resolves the account through `ResidentEngine::action_account` (whose `engine`
field is `Arc<SyncEngine>`), so it calls `engine.<method>`, which must exist on
`SyncEngine`. Hence the B13-SQ (§ 4.1) is a prerequisite of the ratatoskr
landing, exactly as B11-SQ's `SyncEngine::filter_*` was. Going through the
`SyncEngine` passthrough (not reaching for `live_account` from ratatoskr) matches
every prior Track B item and keeps ratatoskr off bifrost's internal surface.

### 2.7 The seam does not flow through the action pipeline

Account settings (list/update identities, get/set vacation, get quota) are an
account-SETTINGS surface, not a per-message `MailActionIntent`. They do not enter
`resolve_intent -> build_execution_plan -> batch_execute`
(`reference/architecture.md`), carry no `CompletionBehavior`, and need no undo /
toast / auto-advance wiring. The rewired Gmail signature sync runs in the resident
aux loop (a background reconciliation), not the action pipeline. B13 adds nothing
to the action pipeline.

### 2.8 Table / cursor / schema disposition

B13 touches no table, cursor, or schema. The local `signatures`,
`send_identities`, and `auto_responses` tables all exist and are unchanged: the
rewire only changes WHERE `sync_gmail_signatures` sources/sinks its server data
(bifrost engine vs hand-rolled Gmail API), not the local `signatures` schema or
the reconciliation semantics. No persisted settings state is added.

### 2.9 The Gmail writeback (`identity_update` sink) is UNVERIFIED in production - OAuth scope gap

Ratatoskr's Google OAuth scope set (`crates/core/src/oauth.rs:16-28`,
`GOOGLE_SCOPES`) is `gmail.readonly`, `gmail.modify`, `gmail.send`,
`gmail.labels`, `userinfo.email`, `userinfo.profile`, `calendar.readonly`,
`calendar.events`, `drive.file`, `contacts.readonly`, `contacts.other.readonly`.
It does NOT include `gmail.settings.basic` (or `gmail.settings.sharing`). Reading
`sendAs` aliases (`GET /settings/sendAs`, the source side / `identities_list`)
works under `gmail.modify` or `gmail.readonly`; but WRITING a sendAs signature
(`PUT/PATCH /settings/sendAs/{email}`, the sink side / `identity_update`) requires
`gmail.settings.basic` per Google's SendAs API. `docs/roadmap/signatures.md:139`
itself records this ("`gmail.settings.basic` - covers read/write access to
`sendAs` settings"), yet the scope is absent from `oauth.rs`.

Consequence: the sink half of the "bidirectional" signature sync
(`SigSyncAction::PushToServer`, `update_send_as_signature` today /
`identity_update` after B13) CANNOT succeed against a real Google account with the
tokens ratatoskr currently mints - the PATCH returns 403 insufficient scope. This
is a PRE-EXISTING condition, not one B13 introduces: today's hand-rolled
`update_send_as_signature` has the same scope requirement and the same latent
failure. The sync-harness gate passes only because `saehrimnir` issues
unrestricted mock tokens that never enforce scope (which is exactly why the
harness has never caught it), and `TODO.md` tracks SendAs writeback as untested.

This forces an explicit disposition in B13 rather than silence (clause 2: an
obstacle on the road is solved in the document). B13 does NOT silently claim to
ship a working writeback it cannot. Two admissible dispositions, and B13 picks the
second unless the user directs otherwise:

- (A) Make writeback a genuine live feature: add `gmail.settings.basic` to
  `GOOGLE_SCOPES` AND specify the existing-account re-consent path (a scope
  addition does not retroactively upgrade already-issued refresh tokens; every
  connected Google account must re-authorize before its PATCH works). That
  re-consent flow is a product/onboarding change with its own UX and migration
  surface - out of proportion to a maximal-integration rewire.
- (B) FEATURE-PRESERVING (chosen): B13 rewires the sink onto `identity_update`
  exactly as it rewires the source onto `identities_list`, preserving today's
  behavior bit-for-bit - including the fact that the PATCH is a no-op-or-403
  against production Google until the scope lands. B13 does NOT regress writeback
  (it was already non-functional in production) and does NOT pretend to fix it.
  The § 4.2 gate therefore proves the writeback path THROUGH the engine against
  the mock (Identity.id mapping + IdentityPatch forwarding + the PATCH body), which
  is the reconciliation-correctness proof; the production scope gap is recorded
  here and carried into the future account-settings item (§ 5) that owes the scope
  addition + re-consent when it makes signature writeback a real, user-visible
  feature.

Either way, the § 4.7 doc reconciliation must stop `docs/roadmap/signatures.md`
from asserting a fully-working "bidirectional sync" the scopes do not support -
the "[x] Gmail bidirectional sync" line (`:34`) is downgraded to note the scope
prerequisite.

## 3. The split (clause 6: keep/revert, ordered so the tree stays green)

Two landings, in order. Each is coherent and fully intrusive; `brokkr check` is
green at the boundary before and after each.

Record the new frozen `../bifrost` commit (the B13-SQ) in this ratatoskr
landing's commit message and ground-survey note AND in the § 11 migration ledger,
per § 11 (as every prior Track B item did). Do NOT ask the B13-SQ commit message
to carry its own final hash - a commit cannot contain the hash it has not yet
been assigned; the promoted-commit hash is recorded downstream (the ratatoskr
landing and the ledger), never inside the bifrost commit itself.

### B13-SQ (bifrost repo, lands first) - expose the settings passthrough on `SyncEngine`

Add the five additive `SyncEngine` settings forwarders (§ 4.1) - AND the matching
`research/bifrost/reference/sync.md` cluster-inventory update (§ 4.1) - by editing
`research/bifrost/crates/sync/src/engine.rs`, the in-tree staging copy where all
side-quest edits to bifrost are made; the orchestrator commits it there and `bash
scripts/bifrost.sh` promotes the committed SQ to the `../bifrost` dependency path.
Do NOT edit `../bifrost` directly - that is the frozen dependency the ratatoskr
gates build against, and editing it in place bypasses the staging-and-promotion
protocol and invalidates the freeze. Mirror `directory_search`. Pure additive; no
existing bifrost behavior changes. Gate: bifrost `brokkr check` green PLUS the
named compile-and-dispatch unit test (§ 4.1). This advances the § 11 freeze a
seventeenth time from `bc97132`; the promoted commit is the frozen reference the
ratatoskr landing builds against.

### B13 (ratatoskr repo) - rewire the live caller + delete the dead duplicates + pin the rest + gate

Ordered so the tree stays green:

1. Add `crates/service/src/bifrost/settings.rs` (§ 4.3) and `pub mod settings;`
   in `crates/service/src/bifrost/mod.rs`. This compiles against the new frozen
   bifrost `SyncEngine` settings methods. `identities` / `identity_update` are
   about to gain a caller; the other three are `#[allow(dead_code)]` pins. Adding
   the module changes no runtime behavior yet.
2. RELOCATE `sync_gmail_signatures` and its private helpers from
   `crates/provider-sync/src/gmail/aux_sync.rs` into a `service`-crate module and
   rewire it (§ 4.2) to source via `AccountSettingsSurface::identities` and push
   via `identity_update`, then DELETE `gmail::api::{list_send_as,
   update_send_as_signature}` and the `GmailSendAs` / `ListSendAsResponse` types.
   The relocation is FORCED: `AccountSettingsSurface` is a `pub(crate)` `service`
   type and `service` depends on `provider-sync`, so the reconciliation cannot stay
   in `provider-sync` and name it (crate cycle - see § 4.2 for the exact plumbing
   and the owned-`ResidentEngine` threading). Because § 2.1 pins the only live
   caller, this is the single behavior-changing step; its gate is the rewritten
   `gmail-send-as-multi-account-import.lua` (source AND writeback legs).
3. Delete `crates/jmap/src/signatures.rs` + `pub mod signatures;` (§ 4.4). Zero
   callers (§ 2.2), so drops no reachable code.
4. Delete the six dead `fetch_*` / `push_*` provider functions in
   `crates/core/src/auto_responses.rs` (§ 4.5), leaving the local read surface.
5. Add the § 4.6 invariant gate.
6. Fold the § 4.7 migration-doc reconciliation into this same commit.

There is one ordering hazard: step 2's rewire depends on step 1's module. Steps
3-6 depend only on the B13-SQ freeze and step 1. The whole ratatoskr landing is
ONE commit (rewire + deletes + pins + gate + doc note are one coherent keep/revert
unit); it is internally ordered so an intermediate compile is never required, but
the keep/revert boundary is the commit, not the step.

## 4. The bricks

### 4.1 B13-SQ: the `SyncEngine` settings passthrough (bifrost)

Edit `research/bifrost/crates/sync/src/engine.rs` (the SQ staging copy, promoted
to `../bifrost` by `scripts/bifrost.sh` - never edit `../bifrost` in place, § 3),
in the PIM passthrough cluster alongside `directory_search` and the `filter_*`
methods, adding the five methods in § 1 artifact 1. Each is `pub async fn`,
resolves `self.live_account(account_id)?` synchronously (surfacing
`AccountNotAttached`), then `.<Account method>(args).await?` (folding
`AccountError` into `bifrost_sync::Error::Account` via `?`, including `Unsupported`
when the account's `PimMethodSupport` flag denies the method - the flattened
direct-passthrough contract, NOT `host_attachment`'s nested shape). Doc-comment
each with the `Account::<method>` forward and the `PimMethodSupport` gate,
mirroring `directory_search`'s doc block. Import `bifrost_types::{Identity,
IdentityPatch, VacationConfig, QuotaInfo, IdentityId}` (or reference
fully-qualified, as `host_attachment` does for `CloudUploadMeta`).

Extend `research/bifrost/reference/sync.md`'s direct-passthrough cluster
enumeration to name the new settings cluster and its flattened engine-`Error`
contract - otherwise the reference goes stale the moment the forwarders land. This
doc edit rides in the B13-SQ commit.

Verification (bifrost repo): `brokkr check` green, PLUS a REQUIRED
compile-and-dispatch unit test (clause 5 mandates the smallest deterministic test
where a behavior can be pinned; the forwarders ARE deterministically testable
without any product UI). Name it (e.g. `settings_passthrough_forwards_and_flattens`)
and cover: (a) each of the five delegations dispatches to the matching
`Account::<method>` with arguments forwarded unchanged; (b) an unattached engine
returns `Err(AccountNotAttached)` up front (the synchronous `live_account` bail);
and (c) a trait method returning `Err(AccountError)` surfaces as `Error::Account`
with the account error carried intact (the flattened-fold contract § 1 artifact 2
relies on). No mock round-trip gate is added, matching the B9-SQ / B11-SQ
precedent - the unit test pins dispatch and error-fold, which is the whole of the
added behavior. The exact command is `brokkr test -p bifrost-sync
settings_passthrough_forwards_and_flattens` - the bifrost sync crate's package
name is `bifrost-sync` (`research/bifrost/crates/sync/Cargo.toml:2`), NOT `sync`;
do not copy the `-p sync` form (there is no `sync` package in the bifrost
workspace and the ratatoskr `sync` crate is a different tree).

### 4.2 Rewire the live Gmail signature reconciliation (ratatoskr)

Keep `sync_gmail_signatures`'s conflict-resolution state machine
(`determine_sync_action`, the hash comparison, NoOp/Pull/Push/Conflict arms, and
the local `signatures` upsert/update SQL) UNCHANGED. Replace only its two
data-boundary calls:

- Source: replace `client.list_send_as(read_db)` (returning `Vec<GmailSendAs>`)
  with `settings.identities(account_id)` (returning `Vec<bifrost_types::Identity>`).
  Map each `Identity` onto the reconciliation's expected shape: server key =
  `Identity.address` (today's `alias.send_as_email`); server HTML =
  `Identity.signature_html.unwrap_or_default()` (today's `alias.signature`);
  display name = `Identity.name`; `is_default` = `Identity.is_default`. This is a
  1:1 field map (§ survey; `settings.rs:15-31`); the reconciliation keys local
  rows by `server_id` = address, unchanged. Three field-type simplifications the
  map makes are all safe and must be preserved: `alias.is_default: Option<bool>` ->
  `Identity.is_default: bool` (drops the `unwrap_or(false)`); `alias.signature:
  Option<String>` -> `Identity.signature_html: Option<String>` (same
  `unwrap_or_default`); and `build_sig_name` (see below) must be re-signatured off
  `GmailSendAs`.
  - `build_sig_name` REWRITE (do not miss this): `build_sig_name(alias:
    &GmailSendAs, server_id: &str)` (`aux_sync.rs:231`) takes the soon-deleted
    `GmailSendAs` and reads `alias.display_name: Option<String>`, filtered on
    non-empty. `Identity.name` is a NON-optional `String` that can still be EMPTY
    (`settings.rs:18-20` documents "often empty for protocols that store only an
    address"). Re-signature it to take the name string and KEEP the empty-string
    fallback (`.filter(|n| !n.is_empty())` semantics), so an empty
    `Identity.name` still falls back to the address exactly as an empty/absent
    `display_name` does today. Dropping the empty check would silently change the
    imported signature `name` for identities with no display name.
- Sink: replace `client.update_send_as_signature(send_as_email, html, read_db)`
  with `settings.identity_update(account_id, identity_id, patch)`. `IdentityPatch`
  is `#[non_exhaustive]` in `bifrost_types` (`settings.rs:36-44`), so a
  struct-literal build with `..Default::default()` from ratatoskr (a DIFFERENT
  crate) does NOT compile - Rust forbids literal construction of a
  `#[non_exhaustive]` struct outside its defining crate, functional-update spread
  included. Build the patch by mutating a default instead:

```rust
let mut patch = IdentityPatch::default();
patch.signature_html = Some(Some(html));
// all other fields stay `None` = "do not change"
```

  The `identity_id` is `Identity.id` (the bifrost `IdentityId`) captured from the
  matching listed identity during the source pass - the push queue carries
  `(IdentityId, html)` instead of `(String email, html)`.

Plumbing (CORRECTED - the naive "pass the surface into the aux runner" does not
compile). `sync_gmail_signatures` and `run_gmail_auxiliary_sync` live in the
`provider-sync` crate (`crates/provider-sync/src/gmail/aux_sync.rs`).
`AccountSettingsSurface` is a `pub(crate)` type of the `service` crate, and
`service` already depends on `provider-sync` (`crates/service/Cargo.toml:42`).
Passing an `AccountSettingsSurface` (or any `service`/`ResidentEngine` handle) INTO
`provider_sync::gmail::run_gmail_auxiliary_sync` is therefore a DOUBLE violation:
(a) a `pub(crate)` service type is not nameable from `provider-sync`, and (b)
making `provider-sync` name a `service` type inverts the dependency edge -
`provider-sync -> service -> provider-sync` is a crate cycle cargo rejects. The
spec's earlier "pass it into the aux runner" instruction is void; use one of these
two boundaries instead (B13 picks the first):

- MOVE the reconciliation into `service` (chosen). Relocate `sync_gmail_signatures`
  (and its private helpers `read_local_signatures`, `determine_sync_action`,
  `upsert_signature_from_server`, `build_sig_name`, `html_hash`, the
  `LocalSignature` / `SigSyncAction` types) from
  `crates/provider-sync/src/gmail/aux_sync.rs` into a new `service`-crate module
  (e.g. `crates/service/src/bifrost/gmail_signatures.rs`), where
  `AccountSettingsSurface`, `ResidentEngine`, and the DB state handles are all in
  scope. `run_aux_pass`'s Gmail arm calls the relocated function directly instead
  of `provider_sync::consumer_support::run_gmail_auxiliary_sync`. This keeps the
  rewire GMAIL-ONLY and matches B11's "reconciliation logic that needs a bifrost
  surface lives in `service`" placement. It also empties `provider-sync`'s Gmail
  aux path of its only remaining pass - confirm whether `run_gmail_auxiliary_sync`
  / the `consumer_support` Gmail wrapper become dead after the move and delete them
  if so (the § 4.6 gate does not require them, and the `initial_sync_completed`
  flag they thread is already unused, `aux_sync.rs:22-24`).
- (Alternative, NOT chosen) Define a narrow data-only trait/callback OWNED by
  `provider-sync` (or a lower crate) that `service` implements and passes down, so
  the settings surface is reached through an interface `provider-sync` already
  names. Rejected as more machinery than the move, for a pass that has no reason to
  stay in `provider-sync` once its Gmail-client dependency is cut.

Owned-engine note (independent of the move): `AccountSettingsSurface::new` needs an
OWNED `ResidentEngine` (`ResidentEngine { inner: Arc<ResidentEngineInner> }`), but
`run_aux_pass(inner: &ResidentEngineInner, ...)` (`resident.rs:977`) holds only a
BORROW of the inner - there is no `ResidentEngine` value to hand off at that frame.
The owning `Arc<ResidentEngineInner>` exists one frame up
(`resident_aux_loop(inner: Arc<ResidentEngineInner>, ...)`, `resident.rs:957`,
calling `run_aux_pass(&inner, ...)` at `:973`). So thread an owned
`ResidentEngine` down: either change `run_aux_pass`'s signature to take
`Arc<ResidentEngineInner>` (reconstructing `ResidentEngine { inner }`) or
reconstruct the `ResidentEngine` at the `:973` call site and pass it in. `inner` is
private but reachable - all these frames are in the same `resident.rs` module.
(Design-consistency aside: routing through `action_account` re-locks the `slots`
mutex per call whereas the aux arm already has the `Arc<SyncEngine>` at hand; this
is harmless here - the aux pass holds no `slots` lock - and matching B11's
`action_account.engine.*` shape is worth the redundant lock. Keep the precedent.)

Since bifrost's Google Account now serves the `sendAs` data, the `GmailClient` is
no longer needed by `sync_gmail_signatures` at all - remove it from that function's
signature; if no other aux pass needs the Gmail client after this cut, drop the
client construction from the Gmail arm too (confirm against the relocated function
and any other Gmail aux passes during implementation). This keeps the rewire
GMAIL-ONLY: the reconciliation still runs only in the Gmail aux arm, so no other
provider gains signature sync (§ 1 feature-preserving).

DELETE `gmail::api::list_send_as` (`gmail/src/api.rs:288`),
`gmail::api::update_send_as_signature` (`:297`), and the `GmailSendAs` /
`ListSendAsResponse` types (`gmail/src/types.rs:199-213`) - the hand-rolled
duplicates of bifrost's identity surface. Confirm no other caller (the § 4.6 gate
mechanizes this).

Gate: REWRITE `crates/app/tests/sync-harness/gmail-send-as-multi-account-import.lua`.
Its behavioral assertions on the local `signatures` table (per-account import
count, `body_html`, `is_default`, `source = gmail_sync`, `server_html_hash`
populated, and the cross-account no-leakage checks) MUST all still hold - that is
the feature-preservation proof. Its transport-level assertion currently counts
raw `GET /gmail/v1/users/me/settings/sendAs` requests (`:192-201`); after the
rewire the request is issued by bifrost's Google Account through the engine, so
update this assertion to the request path bifrost actually issues for
`identities_list` against `saehrimnir` (confirm the endpoint the mock serves;
`saehrimnir`'s Gmail layer already answers `settings/sendAs`, so the path likely
holds, but the per-account attribution and count assertion must be re-verified
against the engine-issued request, not the deleted `gmail::api` call). Exact gate
command (clause 5):

```
brokkr service-test crates/app/tests/sync-harness/gmail-send-as-multi-account-import.lua
```

If the transport-level count cannot be re-pinned deterministically through the
engine (e.g. bifrost batches the identity fetch differently), keep the
behavioral local-table assertions as the load-bearing gate and relax the raw
request-count assertion to a presence check, saying so in the script's comment -
the feature-preservation contract is the `signatures`-table outcome, not the
wire-level request shape.

REQUIRED: the gate MUST also cover the SINK (`identity_update` / writeback) half,
not only the source import. The existing script asserts only import (the
`PullFromServer` arm) plus a `GET` request count, so as written it exercises
`identities_list` alone and leaves `identity_update` - the `Identity.id` capture,
the `IdentityPatch { signature_html }` forwarding, and the `PushToServer` arm -
entirely UNTESTED (the repo already tracks SendAs writeback as untested). Since the
rewire moves the sink onto a NEW type mapping (`IdentityId` instead of the email
string), an untested sink is exactly where a silent regression hides. Extend the
Lua script with a writeback leg: (1) seed / import a signature so a local row
exists with a `server_html_hash`; (2) mutate the LOCAL `signatures.body_html` so
`determine_sync_action` yields `PushToServer` (server unchanged, local changed);
(3) drive another auxiliary pass deterministically; (4) assert the mock received
the sendAs write for the right identity with the expected body (the PATCH/PUT body
carrying the new HTML), keyed to the correct `Identity.address`; and (5) re-read
the mock's stored identity to confirm the pushed HTML round-tripped. This pins
`Identity.id` mapping and `IdentityPatch` forwarding end-to-end against
`saehrimnir` (whose unrestricted mock tokens accept the write - see § 2.9; the
production `gmail.settings.basic` scope gap is orthogonal to this mock gate). If
`saehrimnir`'s Gmail layer does not yet answer the sendAs WRITE endpoint, adding
that mock handler is itself a brick of this gate (clause 5: build the instrument
before the brick it gates), specified to this document's standard.

This makes the § 6 claim that the harness proves BOTH `identities_list` and
`identity_update` end-to-end actually true; without the writeback leg that claim is
unsupported.

### 4.3 The `AccountSettingsSurface` (ratatoskr)

Add `crates/service/src/bifrost/settings.rs` with the `AccountSettingsSurface`
struct (§ 1 artifact 2), holding a `ResidentEngine` and constructed via
`new(resident)` - a direct structural copy of B11's `ServerFilterSurface`. Its
five methods each resolve the account through `self.resident.action_account(...)`,
build `AccountId(account_id.to_string())`, and forward
`action_account.engine.<method>(&account, <args>).await`, re-exporting bifrost's
`Identity` / `IdentityPatch` / `VacationConfig` / `QuotaInfo` / `IdentityId` for
the signatures. Define the TWO-variant `AccountSettingsError` (`Attach(String)`,
`Engine(bifrost_sync::Error)`) - do NOT collapse to `ServiceError`, and do NOT
add a third top-level `Account` arm (the flattened `filter_*`/`directory_search`
cluster preserves `AccountError` NESTED inside `Engine`, § 1 artifact 2). Mark the
three no-caller methods (`vacation_get`, `vacation_set`, `quota_get`)
`#[allow(dead_code)]` with the comment `// pinned <vacation|quota> surface; B13
adds no UI caller`. `identities` and `identity_update` need NO `allow` - the § 4.2
rewire is their caller. Wire `pub mod settings;` into
`crates/service/src/bifrost/mod.rs`.

### 4.4 Delete the dead hand-rolled JMAP identity/signature duplicate (ratatoskr)

Delete `crates/jmap/src/signatures.rs` in full and remove `pub mod signatures;`
at `crates/jmap/src/lib.rs:14`. Confirm no other `jmap` module references
`signatures::` (§ 2.2 shows none). The `jmap` crate still compiles - the module
was a leaf. This is the § 1 maximal-integration deletion for the JMAP
identity/signature slice, discharged early (the whole `jmap` crate retires at
B15); B13's deletion is INPUT to B15's workspace audit, not a waiver of it.

### 4.5 Delete the dead per-provider vacation duplicate (ratatoskr)

Delete the six dead provider functions in `crates/core/src/auto_responses.rs`
(`fetch_{graph,gmail,jmap}_auto_response`, `push_{graph,gmail,jmap}_auto_response`)
and any now-unused helpers they alone used (`normalize_dotnet_datetime`, the
`ExternalAudience` enum if it has no other consumer - confirm during
implementation; `AutoResponseConfig` too if nothing else references it). LEAVE
`any_auto_response_active` (`:45`) and its local DB read path, the
`auto_responses` table, and the dead-but-local `upsert_auto_response_sync` intact
- they are app-level scaffolding (§ 2.3), not bifrost duplicates. If the whole
`auto_responses.rs` module collapses to only `any_auto_response_active` after the
deletion, that is fine; keep the module. This is the vacation slice of § 1
maximal-integration.

### 4.6 The invariant gate (mechanize the § 2.2-2.5 audit)

The threat this gate guards: a future change reintroducing a hand-rolled
identity/signature/vacation provider CRUD surface alongside bifrost's settings
methods (violating § 1 maximal-integration), or the pinned/rewired surface being
bypassed by a direct provider call. Following B11's `server_filters_route_through_bifrost`
precedent (a source lockdown in the surviving `service` crate whose SCAN is
workspace-wide):

**Gate - `account_settings_route_through_bifrost` (test lives in the `service`
crate; SCAN is workspace-wide).** A lockdown test that walks EVERY ratatoskr Rust
source under `crates/*/src` (source only, EXCLUDING `#[cfg(test)]` / test-module
bodies and fixture strings) and asserts:

1. The deleted duplicates stay deleted: `crates/jmap/src/signatures.rs` does not
   exist AND no `pub mod signatures;` line appears in `crates/jmap/src/lib.rs`;
   `gmail::api::list_send_as` / `update_send_as_signature` and the `GmailSendAs`
   / `ListSendAsResponse` types are gone; the six `fetch_*` / `push_*` vacation
   provider functions are gone from `crates/core/src/auto_responses.rs`.
2. No source drives the hand-rolled provider SIGNATURE/VACATION settings surfaces -
   `bifrost_jmap::identity::IdentitySet` (the signature WRITER),
   `bifrost_jmap::vacation_response`, a raw `/settings/sendAs` /
   `settings/vacation` / `mailboxSettings`/`automaticRepliesSetting` provider
   call - anywhere in the workspace (the surviving app-level `auto_responses`
   local read touches none of these tokens, so it does not false-positive).
   EXPLICIT ALLOW-LIST: `bifrost_jmap::identity::IdentityGet` in
   `crates/jmap/src/helpers.rs` is EXEMPT - it is send-path identity RESOLUTION
   for `JmapOps::send_email` (§ 2.2 caveat), not the deleted signature CRUD. The
   gate must scope its `IdentityGet` ban to the DELETED `signatures.rs` (which no
   longer exists) and NOT flag `helpers.rs`; the cleanest expression is to ban
   `IdentitySet` outright (no legitimate survivor) and to assert `signatures.rs`'s
   non-existence (item 1) rather than banning `IdentityGet` workspace-wide, since a
   legitimate `IdentityGet` survivor exists. If the gate does scan for `IdentityGet`
   at all, `crates/jmap/src/helpers.rs` is on its allow-list.
3. Server-side settings access appears ONLY as `engine.{identities_list,
   identity_update, vacation_get, vacation_set, quota_get}` inside the single
   allow-listed call site `crates/service/src/bifrost/settings.rs`
   (`AccountSettingsSurface`). A direct `engine.<settings method>` call from any
   other file FAILS the gate.

The `service`-crate placement of the TEST is load-bearing: `service` survives the
migration (unlike `jmap`/`gmail`, which retire at B15), so the gate outlives the
provider crates - but the SCAN is the whole workspace, reaching the crates that
held the duplicates. Exact gate command (clause 5, package is `service`):

```
brokkr test -p service account_settings_route_through_bifrost
```

Implement under this exact name so the § 6 gate command resolves without
substitution.

### 4.7 Migration-doc reconciliation (rides with the ratatoskr code landing)

Update `docs/bifrost-migration.md`: rewrite the § 7 B13 line (`:1510-1511`) from a
TODO into a done-note recording that the seam is CLOSED - the live Gmail
signature reconciliation rewired onto bifrost `identities_list` /
`identity_update` (hand-rolled `gmail::api` sendAs calls deleted); the dead
`crates/jmap/src/signatures.rs` and the dead per-provider vacation surface in
`core/auto_responses.rs` deleted (maximal-integration); the local
`signatures`/`send_identities`/`auto_responses` tables + `any_auto_response_active`
left as app-level surfaces (§ 5); the bifrost `vacation_get` / `vacation_set` /
`quota_get` pinned behind `AccountSettingsSurface` (`#[allow(dead_code)]`, no UI
caller, mirroring B9/B11); and the invariant gated (§ 4.6). Record the B13-SQ
freeze advance (seventeenth, from `bc97132`) in § 11 with the new commit hash and
a one-line description of the additive `SyncEngine` settings passthrough,
mirroring the B11-SQ § 11 entry (`:1785-1794`). Bundle this markdown with the
§ 4.2-4.6 code in the same commit (never a standalone markdown commit). The full
crate-map / `reference/architecture.md` reconciliation remains B16's job.

ALSO reconcile `docs/roadmap/signatures.md` (the feature-area doc, required
reading, § clause 10) in the SAME commit - it is stale against the tree in two
ways B13 must not leave standing:
- The "[x] JMAP Identity signature sync ... live" claim (`:4`, `:34`-`:35`)
  documents `crates/jmap/src/signatures.rs` as a shipped live feature, but it has
  zero callers (§ 2.2) and B13 DELETES it. Rewrite these lines to record that JMAP
  identity signature sync was never wired (dead code) and is removed at B13; the
  local signature store + Gmail sync remain.
- The "[x] Gmail bidirectional sync" claim (`:34`) overstates production
  writeback: the sink requires `gmail.settings.basic`, absent from `oauth.rs`
  (§ 2.9). Downgrade it to note the reconciliation is rewired onto bifrost
  `identity_update` and that production writeback is gated on a future scope
  addition + re-consent (the harness proves it against the mock).
This keeps the required-reading doc from asserting features the tree does not have
(a reviewer reading it must not treat the stale JMAP/writeback claims as
preservation obligations - see the required-reading note on this doc).

## 5. Stopping rule (clause 9)

- IN: the B13-SQ `SyncEngine` settings passthrough (§ 4.1); the
  `AccountSettingsSurface` (§ 4.3); the Gmail signature reconciliation rewire +
  `gmail::api` sendAs deletion + harness-gate rewrite (§ 4.2); deletion of the
  dead `crates/jmap/src/signatures.rs` (§ 4.4) and the dead vacation provider
  surface (§ 4.5); the invariant gate (§ 4.6); the migration-doc closure note +
  § 11 freeze advance (§ 4.7).
- OUT, named not deferred (clause 3):
  - A working account-SETTINGS FEATURE (a settings tab that lists/edits identities,
    reads/writes vacation, or renders quota): a NEW product item, forbidden here
    by § 1 feature-preserving (absent today). The pinned `vacation_get/set` /
    `quota_get` and the `identities` surface are its future seam, not B13.
  - A `send_identities` POPULATE path from bifrost `identities_list` (§ 2.4):
    identity import is unwired today; wiring it is a compose/settings product
    item, not B13. The `send_identities` table, its reads, and the compose
    from/reply-to detection are untouched.
  - The `gmail.settings.basic` OAuth SCOPE addition + existing-account re-consent
    flow (§ 2.9): without it, sendAs signature WRITEBACK cannot succeed against a
    real Google account. This is a PRE-EXISTING production gap (today's hand-rolled
    writeback has the same requirement), not a B13 regression. B13 preserves the
    existing behavior bit-for-bit and does NOT add the scope or the re-consent UX -
    that is the future account-settings item's job (a product/onboarding change).
    B13 records the gap and reconciles `docs/roadmap/signatures.md`'s overstated
    "bidirectional sync" claim (§ 4.7); it does not fix it.
  - The `crates/jmap/src/helpers.rs` `IdentityGet` submission-path call (§ 2.2
    caveat): send-identity RESOLUTION, not the deleted signature CRUD. Untouched
    here; its disposition is the JMAP send-path / B15 crate retirement. B13 only
    allow-lists it in the § 4.6 gate.
  - The local `auto_responses` table, `any_auto_response_active`, and the dead
    `upsert_auto_response_sync` (§ 2.3): app-level status-bar scaffolding, left
    exactly as-is (the B11 demo-tab analogue). Neither deleted (scope creep) nor
    wired (would add the absent feature).
  - The local `signatures` table and its CRUD handlers (`signature.create/update/
    delete/reorder`): app-level local editing store, unchanged. The rewire only
    changes how the Gmail reconciliation sources/sinks SERVER data.
  - JMAP shared-account identity resolution (`resolve_shared_account_identities`,
    live in the JMAP aux pass): a SHARED-MAILBOX concern, B12's territory, not the
    per-account sending-identity surface B13 covers. Untouched.
  - `service-api` wire types for `Identity` / `VacationConfig` / `QuotaInfo`:
    minted by the future settings item when a caller crosses the wire, not by B13
    (no wire caller = no wire type).
  - The whole `jmap`/`gmail`-crate deletion and the workspace-wide
    maximal-integration audit: B15. B13 discharges only the identity/signature/
    vacation slices early and records them as input to B15.
- Blast radius: one new ratatoskr module (`settings.rs`); one rewired function
  (`sync_gmail_signatures`) RELOCATED from `provider-sync` into `service` with its
  private helpers, plus the resident-aux plumbing change (owned `ResidentEngine`
  threaded through `run_aux_pass`) and any now-dead `provider-sync` Gmail aux
  wrapper removed; deletions in `gmail/src/api.rs` + `gmail/src/types.rs`,
  `crates/jmap/src/signatures.rs` (+ its `mod` line), and `core/src/auto_responses.rs`
  (six functions); one new invariant test; one rewritten harness gate (source +
  writeback); one bifrost additive passthrough cluster; and doc notes
  (`docs/bifrost-migration.md` + `docs/roadmap/signatures.md`). No `app` /
  `service-api` / wire change; no schema / cursor / table change; no action-pipeline
  change; no change to the local `signatures`/`send_identities`/`auto_responses`
  read surfaces or the status-bar indicator. NOTE: production Gmail signature
  WRITEBACK remains gated on the absent `gmail.settings.basic` OAuth scope (§ 2.9);
  B13 preserves today's behavior and does not add the scope (out of scope, § 5
  future settings item).

## 6. Verification per brick (clause 5)

- The bifrost B13-SQ (§ 4.1), in the bifrost repo - green tree AND the required
  compile-and-dispatch unit test:

```
brokkr check
brokkr test -p bifrost-sync settings_passthrough_forwards_and_flattens
```

  (Pins the five delegations, argument forwarding, the `AccountNotAttached`
  up-front bail, and the `AccountError -> Error::Account` fold. No round-trip mock
  gate, matching B9-SQ / B11-SQ. The bifrost sync crate's package name is
  `bifrost-sync`, `research/bifrost/crates/sync/Cargo.toml:2` - NOT `sync`.)

- The ratatoskr landing's universal green-tree gate (proves the `jmap` crate
  compiles after the `signatures.rs` deletion, the `gmail` crate compiles after
  the `api`/`types` deletions, `core` compiles after the vacation deletion, and
  `service` compiles the new `AccountSettingsSurface` against the new bifrost
  surface):

```
brokkr check
```

- The live-feature preservation gate (the ONE behavior B13 changes - the Gmail
  signature reconciliation now routed through bifrost):

```
brokkr service-test crates/app/tests/sync-harness/gmail-send-as-multi-account-import.lua
```

- The invariant gate (§ 4.6), the durable maximal-integration lock:

```
brokkr test -p service account_settings_route_through_bifrost
```

- The Service-boundary backstop - a GENERAL green-tree gate that the Service boot
  + handler surface still assembles after the module add / deletes (NOT proof of
  vacation/quota behavior, since those pins wire no caller):

```
brokkr service-suite
```

- Performance / provider-request-count treatment (contract lines 32, 84: a spec
  touching a provider-sync path owes a `brokkr sync-bench ... --gate <name>` run
  against its `brokkr.toml` baseline, OR an EXPLICIT statement of why none
  applies). B13's disposition, stated explicitly rather than by silence: the
  rewired path is the Gmail signature reconciliation in the resident AUX loop
  (aux cadence, not the steady-state sync hot path), and the rewire does not change
  its request shape at the provider - it still issues one sendAs LIST per pass and
  one sendAs write per changed local signature, now sourced through bifrost's
  Google Account instead of the hand-rolled `gmail::api` call. No new per-message,
  per-thread, or backfill request is added; no hot-path allocation or RSS budget is
  touched. Therefore B13 asserts NO steady-state sync-bench regression is possible
  from this change and adds no new sync-bench baseline. HOWEVER, the one request
  characteristic the rewire CAN shift is the sendAs LIST request COUNT/shape if
  bifrost's Google Account batches or caches the identity fetch differently from
  the hand-rolled `GET /settings/sendAs`. That is pinned by the harness gate's
  transport-level assertion (§ 4.2), which re-verifies the per-account
  request attribution against the engine-issued request - the correct instrument
  for a request-count delta on an aux path, since `sync-bench` measures
  steady-state mail sync throughput, not aux-loop settings traffic. If, during
  implementation, the engine's identity fetch is found to enter a metered sync
  path with a recorded baseline, add the named `brokkr sync-bench` gate for it then
  (specified to this document's standard); absent that, the harness request-count
  assertion is the request-shape gate and this explicit no-regression statement
  stands in for the sync-bench baseline (clause 5's provision for a path no
  existing bench instruments).

Coverage splits by layer. The bifrost `SyncEngine` settings forwarders ARE
deterministically testable and carry the required unit test (clause 5). The Gmail
signature rewire is the live behavior change and is pinned by the rewritten
sync-harness gate end-to-end (source via `identities_list`, reconcile, sink via
`identity_update`, no cross-account leakage). The pinned `vacation_get` /
`vacation_set` / `quota_get` methods, like B9's `host_large_attachment` and B11's
`filter_*`, have no live caller, so their coverage is compile-and-capability-
dispatch plus the maximal-integration invariant gate - the spec says so
explicitly, per clause 5's provision for a behavior no round-trip instrument can
yet pin (there is no product caller to drive a vacation/quota round trip). When a
future settings item adds that caller, it owes the round-trip harness gate; B13
does not, because driving a real vacation/quota round trip with no product caller
would be building throwaway scaffolding, not pinning shipped behavior.

## 7. The falsifiability challenge (why the rewrite/delete/pin split is a finding, not a hand-wave)

B13's disposition is correct only if exactly one occupant of the seam is live
(the Gmail signature sync) and the rest are dead or absent. The finding is REFUTED
if any of these is true at implementation time:

1. Any crate references `jmap::signatures` (`sync_jmap_identity_signatures` /
   `push_signature_to_jmap`). (§ 2.2. As surveyed: false. If a caller exists,
   `signatures.rs` cannot be deleted and B13 becomes a second real rewire, moving
   that caller onto `AccountSettingsSurface::{identities, identity_update}` before
   landing.)
2. Any crate calls a `fetch_*` / `push_*` provider function in
   `core/auto_responses.rs`. (§ 2.3. As surveyed: false - all six are dead. If a
   caller exists, that leg rewires onto `vacation_get` / `vacation_set` and the
   pin becomes a rewire.)
3. The `send_identities` table has a live production writer (a settings/import
   path that populates it). (§ 2.4. As surveyed: false - only test_helpers writes.
   If true, B13 must rewire that writer onto `identities_list` rather than pin the
   surface, and preserve the import feature.)
4. Bifrost lacks a working `Account` settings method for a provider ratatoskr
   ships. (§ 2.6. As surveyed: false - all provider Account impls implement the
   five methods, capability-gated. A provider returning `Unsupported` still
   compiles and dispatches correctly through the capability-driven surface.)
5. `docs/bifrost-migration.md` § 1 is reinterpreted to REQUIRE wiring the
   settings feature (identity import, vacation UI, quota display) now. (That makes
   B13 a feature-add, contradicting feature-preserving - a separate product item,
   not a B13 reopening.)

If (1), (2), or (3) is non-empty at land, that leg stops being delete/pin and
becomes a real caller migration onto `AccountSettingsSurface`, specified to this
document's standard, before landing. Absent that, the finding stands: the seam
has one live caller (Gmail signatures, rewired), dead hand-rolled duplicates
(deleted), and no-caller bifrost surfaces (pinned) - the mixed close this spec
specifies.
