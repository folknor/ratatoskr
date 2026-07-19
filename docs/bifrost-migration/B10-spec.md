# B10 technical-implementation-spec: server-side search

Closes the "server-side search" seam listed in `docs/bifrost-migration.md`
§ 5 ("Rewired"). The pivotal survey finding (§ 2, established by the
workspace-wide caller audit of § 4.1 - every crate, not only `core` /
`service`; see § 4.1 for the enumerated scope and the classification of the
query-shaped provider primitives in `jmap` / `gmail` / `imap`) is that this
seam has exactly ONE live user-search
instance anywhere in ratatoskr - the contacts / GAL directory lookup
`directory_search` - and that instance was ALREADY rewired onto
`engine.directory_search` by B8 (`crates/service/src/handlers/gal.rs:167`,
migration done-note at `docs/bifrost-migration.md:1420-1422`). All MAIL
search in ratatoskr is the LOCAL tantivy + smart-folder-SQL pipeline, which
`docs/bifrost-migration.md` § 3 explicitly keeps app-level and which never
touched a provider. There is therefore NO provider-side mail-search surface
to drive onto bifrost, and none may be built (adding one would violate the
§ 1 feature-preserving mandate and `docs/search/problem-statement.md`
Open Question 4).

B10 consequently lays NO mail-search rewire brick. Its deliverable is the
enforcing close-out: (a) the § 1 maximal-integration audit, scoped to
search, that proves no hand-rolled provider mail-search surface with a live
caller survives; and (b) a permanent invariant gate that pins "mail search
stays local / provider-free" so a future change cannot silently reintroduce
a hand-rolled provider search alongside the local pipeline. This is the
honest disposition of a seam whose only real occupant already migrated: per
`reference/technical-implementation-spec.md` clause 8, "a sibling's survey
may already state the fact that refutes this spec's premise" - B8's survey
did exactly that for `directory_search`, and this spec records the
reconciliation rather than re-migrating settled ground (clause 3: work
belonging to a genuinely separate TODO is named and excluded, which is not
deferral).

This spec is written against `reference/technical-implementation-spec.md`
(the contract it must satisfy - READ IT) and conforms to its ten clauses.
It is one item of `docs/bifrost-migration.md` (the governing plan and TODO
source - READ § 1, § 3, § 5, § 7 B10, § 8), run through
`reference/orchestrate.md`.

## Required reading (clause 10)

Every implementer and reviewer MUST read these before laying a brick. They
are the ground this work is built on and judged against; naming them is not
enough. Because B10's whole thesis is a negative claim ("no provider-side
mail search exists to rewire"), the required reading is heavier on the
survey sources that could FALSIFY that claim than on any bifrost surface to
consume.

- `reference/technical-implementation-spec.md` - the contract this spec is
  written against. Clause 3 (no deferral; separate-TODO work is named and
  excluded) and clause 8 (survey the ground; reconcile against sibling
  surveys - a sibling may already refute the premise) are the load-bearing
  clauses for a confirm-and-close item.
- `reference/architecture.md` - ALWAYS required. The `core`/`app` firewall
  (the app depends on `rtsk` + `service-api` wire types only, never
  bifrost), the crate map, and the multi-store durability contract (main DB
  / body store / inline-image store / attachment file cache / SEARCH are
  separate durable stores, all app-level) are what pin "search stays local."
- `docs/bifrost-migration.md` - the TODO source. § 1 (feature-preserving:
  no capability newly-wired that is absent today, AND maximal-integration:
  no hand-rolled duplicate of a bifrost surface survives), § 3 (target
  architecture - "ratatoskr keeps ... tantivy local search (storage and
  local search are app-level)", lines 179-180), § 5 ("Rewired: ...
  server-side search" - the seam this item closes, and "Survives untouched:
  ... tantivy search", lines 228-236), § 7 B10 / B8 done-note (line 1420),
  § 8 (sequencing).
- `docs/search/problem-statement.md` - the mail-search design contract. §
  "All search is local" (line 7), Open Question 4 ("Provider-side search:
  Not needed ... There is no 'not yet synced' gap to fill with provider API
  search", line 456), and Open Question 5 (body text always local). This
  doc is the authority that B10 must NOT build provider search.
- `docs/search/implementation-spec.md` - the local pipeline's build spec
  (the tantivy + SQL slices), confirming the search entry point
  (`core::search_pipeline`) routes to local engines only, never a provider.
- `reference/glossary/folders-labels.md` - REQUIRED because the smart-folder
  SQL search path filters on `labels` / `thread_labels` / `label_kind` /
  system-folder IDs; a reviewer confirming the SQL search path is
  provider-free needs the labels model.
- `reference/glossary/harness.md` - the Service test harness, `brokkr
  service-test` / `service-suite`, and gate baselines. The green-tree
  backstop gate (§ 6) is defined here.
- `research/bifrost/reference/sync.md` + `crates/sync/src/engine.rs` (the
  frozen tree) - to CONFIRM the negative claim from the bifrost side: the
  `SyncEngine` exposes exactly one search-shaped method, `directory_search`
  (`engine.rs:2049`), and NO mail-message search/query method exists. The
  JMAP `Email/query`, Graph `$search`, and Gmail `q=` primitives that the
  per-provider reference sheets (`research/bifrost/reference/{jmap,graph,
  google}.md`) describe are SYNC enumeration primitives internal to bifrost,
  not a user-search surface bifrost re-exports for ratatoskr to call.

The `../bifrost` dependency checkout is frozen for the full duration of
this item per `docs/bifrost-migration.md` § 11; record the exact frozen
commit in the ground survey of the landing (§ 3). B10 consumes NO bifrost
surface and adds NO bifrost side-quest, so the freeze does not advance here
- but the survey must still read the frozen `engine.rs` to confirm the
negative claim against the same tree the build resolves.

## 1. The goal (clause 7: the target as concrete artifacts)

Today the "server-side search" work of `docs/bifrost-migration.md` § 5 is
distributed as follows across the tree:

- CONTACTS / GAL directory search (already migrated, B8). The one live
  provider-side "search" caller is the Global Address List lookup:
  `crates/service/src/handlers/gal.rs` calls
  `action_account.engine.directory_search(&account, String::new(),
  Some(1000), cursor)` (`:167`) - note the query argument is an EMPTY string,
  not a user query: the call pages the ENTIRE provider directory into the
  24-hour `gal_cache` (`fetch_gal_entries_if_stale`, `:143`). It is
  search-shaped provider ENUMERATION, not a user mail/contact query - a
  distinction that is central to why this is the seam's sole occupant and why
  no user-search verb exists. B8 replaced the retired hand-rolled `fetch_graph_gal` /
  `fetch_google_gal` with this single `engine.directory_search` call
  (`docs/bifrost-migration.md:1420-1422`). This is DONE; B10 does not touch
  it. `rtsk::contacts::search` and `rtsk::contacts::gal` (the caching /
  merge layer around it) are contacts-domain code, not a second search
  seam.
- MAIL search (local, app-level, provider-free BY DESIGN). The user-facing
  mail search is the unified local pipeline: `core::search_pipeline`
  (`crates/core/src/search_pipeline.rs`, 807 LOC) parses the query and
  routes to the tantivy full-text index (`crates/search/`, the `search`
  crate) and the smart-folder SQL engine (`core::db::queries` +
  `core::smart_folder`). `docs/search/problem-statement.md` (line 7, Open
  Q4) makes provider delegation an explicit non-goal: ratatoskr syncs the
  full mailbox locally, so tantivy searches complete data and there is no
  "not yet synced" gap for a provider API to fill. This path never calls a
  provider and must never start.
- IMAP protocol SEARCH (sync enumeration, not user search - B3/B15). The
  IMAP provider crate has `client::sync::search_folder` (`:166`) and
  `client::commands::search_all_uids` (`:38`); both issue an IMAP `SEARCH`
  / `UID SEARCH` to ENUMERATE the UID set of a folder during SYNC, not to
  serve a user query. They are part of the retiring IMAP provider-sync
  surface (`docs/bifrost-migration.md` § 5 sync inversion) and retire with
  the provider crates at B15, on the B3 sync-cutover disposition - NOT a
  B10 target.

After B10, the state is unchanged in behavior but CLOSED and GATED:

- The § 5 "server-side search" seam is recorded as fully satisfied - its
  sole live instance (`directory_search`) migrated in B8, with no
  hand-rolled provider mail-search surface surviving anywhere (proven by the
  § 4.1 audit, not asserted).
- Two complementary invariant gates (§ 4.2) pin "mail search is local and
  provider-free": Gate A asserts the leaf `search` crate carries no provider
  / bifrost / `common` dependency, and Gate B is a source lockdown over the
  `rtsk` / `service` search-path source asserting no mail-search path calls a
  provider or engine search method beyond the allow-listed B8 GAL call. A
  future change that tried to bolt a hand-rolled provider search onto the
  local pipeline (violating § 1 maximal-integration) trips Gate B - the one
  that lives in the crate where the threat can actually land, since `rtsk`
  already links every provider and a manifest guard there cannot catch it.
- The migration-doc reconciliation (§ 4.3) rides with B10's OWN code landing
  (not B11): because § 4.2 lands real test bricks, B10 is a code commit and
  can carry its own doc note, so the never-a-standalone-markdown-commit rule
  is satisfied without deferring the note to B11 (§ 3, § 4.3).

There is no new type, signature, module, table, cursor, wire message, or
bifrost passthrough. The concrete artifacts B10 produces are: the audit
finding (§ 4.1, recorded in the landing commit message and this spec's
survey) and the two invariant tests (§ 4.2, Gate A + Gate B).

## 2. Survey of the ground (clause 8)

The survey is the substance of this item. Because the goal is a negative
claim, the survey must be exhaustive enough to be falsifiable: it enumerates
every place a provider-side mail search COULD live and shows each is either
absent, already migrated, or out of scope by a named sibling item.

### 2.1 The `ProviderOps` trait has no search method (the decisive fact)

`crates/common/src/ops.rs` is the single provider surface every mail
operation flows through. Its complete method set is: `archive`, `trash`,
`permanent_delete`, `mark_read`, `star`, `spam`, `move_to_folder`,
`add_label`, `remove_label`, `mark_mdn_sent`, `send_email`,
`mark_send_intent`, `create_draft`, `update_draft`, `delete_draft`,
`fetch_attachment`, `fetch_message`, `fetch_raw_message`, `test_connection`,
`get_profile`. There is NO `search`, `query`, `find_messages`, or any
search-shaped method. A provider-side mail search cannot be invoked through
the mail action / fetch surface because the surface has no such verb. This
is the load-bearing fact: mail search was never a provider seam in
ratatoskr.

### 2.2 Mail search is local and provider-free, structurally (clause 8: the load-bearing work the seam-close must not disturb)

- The `search` crate (`crates/search/Cargo.toml`) depends ONLY on `serde`,
  `types`, `tokio`, `tantivy`, `log`, and (optional) `hotpath`. It has NO
  dependency on `common`, any provider crate (`gmail` / `graph` / `jmap` /
  `imap`), `provider-sync`, or bifrost. It cannot call a provider.
- `core::search_pipeline` (`crates/core/src/search_pipeline.rs`) imports no
  provider / bifrost / `ops` type; it routes a parsed query to the tantivy
  index and the SQL engine only. A grep for `provider` / `bifrost` / `ops::`
  in that file returns nothing.
- `docs/bifrost-migration.md` § 3 (lines 179-180) and § 5 ("Survives
  untouched", line 230) both name tantivy local search as app-level and
  untouched by the migration. The DESIGN intent (`problem-statement.md` Open
  Q4) is that provider search is never wired.

This is the load-bearing structure the seam-close must NOT change: B10 keeps
the local pipeline exactly as-is. Its only addition is a test that PINS this
already-true property.

### 2.3 The one live provider-side "search" is `directory_search`, and B8 owns it

`crates/service/src/handlers/gal.rs:167` is the only live caller of any
provider-side search in the tree. It calls `engine.directory_search`, the
bifrost `SyncEngine`'s GAL/people-directory method (`research/bifrost/
crates/sync/src/engine.rs:2049`), through the B4a resident-action-account
handle, handling the `RecoveryClass::Unsupported` capability-absent case
declaratively. This IS the "server-side search" rewire - and it landed in
B8 (`docs/bifrost-migration.md:1420-1422`: the B8 commit "dropped
`fetch_graph_gal` / `fetch_google_gal` for one `engine.directory_search`
call, keeping `gal_cache_age` and the `gal_cache` table unchanged"). B10
neither touches nor re-migrates it. Naming it here is the clause-8
reconciliation: the sibling B8 survey already stated the fact that empties
B10's mail-search premise.

### 2.4 Bifrost exposes no mail-search surface to rewire ONTO

Confirming the negative from the bifrost side (frozen tree): the entire
`SyncEngine` public surface (`research/bifrost/crates/sync/src/engine.rs`)
has exactly one search-shaped method - `directory_search` (`:2049`). There
is no `search`, `email_query`, `message_search`, or `query` method a mail
search could be driven onto. Bifrost DOES use per-provider server query
primitives (JMAP `Email/query`, Graph `$search`/`$filter`, Gmail `q=`)
INTERNALLY to drive sync enumeration, but it does not surface them as a user
mail-search API for ratatoskr to consume, and B10 must not invent one:
adding a provider-search feature absent today violates § 1's
feature-preserving mandate. Even if a future product wanted server-side mail
search (it does not - `problem-statement.md` Open Q4), that would be a NEW
feature item building a NEW bifrost surface, categorically out of the
bifrost-migration's feature-preserving scope.

### 2.5 IMAP `search_folder` / `search_all_uids` are sync enumeration (B3/B15, named-not-deferred)

`crates/imap/src/client/sync.rs:166` (`search_folder`) and
`crates/imap/src/client/commands.rs:38` (`search_all_uids`) issue IMAP
`SEARCH` to enumerate a folder's UID set during SYNC (initial/backfill
enumeration), not to serve a user query. They are part of the retiring
per-provider sync surface. Their disposition is the B3 sync-cutover (the
consumer no longer drives per-provider enumeration) and B15 (provider-crate
deletion), NOT B10. This is a clause-3 named exclusion, not a deferral: the
work belongs to a genuinely separate, already-scoped TODO item.

### 2.6 Adjacent search-shaped surfaces that are NOT this seam (disambiguation)

To keep the audit falsifiable, the following search-named code is explicitly
identified as OUT of the "server-side search" seam:

- `crates/service/src/handlers/pinned_search.rs` - CRUD for PINNED SEARCHES
  (saved local queries persisted in the DB); it stores/updates/deletes query
  strings and kicks a local re-run. No provider call.
- `crates/service/src/search_writer.rs` and
  `crates/service-state/src/search_write.rs` - the writers that index synced
  message text INTO the local tantivy index. App-level indexing, no provider
  read.
- `crates/service/src/extract.rs` / `text_extract` - attachment text
  extraction feeding the local index. No provider search.
- `rtsk::contacts::search` (`crates/core/src/contacts/search.rs`) - the
  local contacts FTS (`contacts_fts`) lookup used by `from:` / `to:`
  typeahead. Local; the provider directory side of contacts is
  `directory_search` (§ 2.3, B8).

None of these is a provider mail-search seam; all stay exactly as they are.

### 2.7 Table / cursor disposition

B10 touches no table, no cursor, no schema. The `gal_cache` table stays as
B8 left it. The tantivy index and its writers are unchanged. There is
nothing to migrate or drop.

## 3. The split (clause 6: keep/revert, ordered so the tree stays green)

One landing. It is coherent and fully intrusive in the only sense available
to a confirm-and-close item: it lands the audit finding plus the invariant
gate, and is kept or reverted on that gate. `brokkr check` is green at the
boundary before and after.

Record the frozen `../bifrost` commit in the landing commit message per §
11, even though B10 consumes no bifrost surface - the § 2.4 negative claim
is pinned against that tree.

### B10 - close the server-side-search seam + pin the local-search invariant

- Run the § 4.1 mechanical maximal-integration audit (scoped to search) and
  record its finding in the commit message: no hand-rolled provider
  mail-search surface with a live caller survives; the sole seam instance
  (`directory_search`) is B8-owned; IMAP sync-search is B3/B15-owned.
- Add the § 4.2 invariant tests (Gate A in `search`, Gate B in `rtsk`).
- Fold the § 4.3 migration-doc reconciliation into this same commit. Because
  § 4.2 adds real test bricks, B10 IS a code commit and carries its own doc
  note, satisfying the never-a-standalone-markdown-commit rule without
  deferring to B11. This is the pinned disposition (not conditional).

There is no ordering hazard: B10 depends on B8 (already landed) and B1
(already landed, resident engine + GAL path); it blocks nothing except the
final B15/B16 reference-doc reconciliation, which folds B10's finding into
the crate map. B10 may land at any point after B8.

## 4. The bricks

### 4.1 The maximal-integration search audit (mechanical, § 1)

`docs/bifrost-migration.md` § 1's maximal-integration rule (no parallel
hand-rolled or duplicated dependency surviving alongside a bifrost
equivalent) is stronger than the § 5 enumeration and is the standing B15
obligation. B10 discharges the SEARCH slice of it early and records the
result as INPUT to B15 - it does NOT exempt search from B15's final sweep.
`docs/bifrost-migration.md:1499-1504` makes B15's whole-workspace
dependency-and-module audit a hard, non-negotiable floor over every crate;
because B10 may land long before B15 and its § 4.2 gate is a narrow
leaf-crate + lockdown guard (not a blanket provider-search interdict), B15
MUST still re-audit the search slice at final landing. B10's recorded
finding is a starting classification for that sweep, not a waiver of it.

The audit is a bounded, repeatable sweep (not a subjective judgement), and
it is WORKSPACE-WIDE - every crate under `crates/`, not only `core` /
`service`. The intro's "whole workspace" claim is only honest if the sweep
actually covers the provider crates, `app`, `service-state`, and every other
member:

1. Confirm `ProviderOps` (`crates/common/src/ops.rs`) has no search/query
   method (§ 2.1). This is SUPPORTING evidence (mail actions cannot invoke
   provider search through the action surface), not decisive proof on its
   own - a provider crate can expose a query primitive OUTSIDE `ProviderOps`,
   so steps 2-4 do the load-bearing work.
2. Confirm no live caller in ANY crate drives a provider or
   `engine`/`SyncEngine` search method as a USER mail search, other than the
   B8 `directory_search` at `gal.rs:167`. The sweep runs over all of
   `crates/*/src` for `directory_search`, `email_query`, `Email/query`,
   `_search`, `uid_search`, `.query(`, `find_message`, `list_threads`;
   classify every hit as (a) the B8 GAL call, (b) local pipeline / typeahead
   / indexing (§ 2.2, § 2.6), (c) URL/SQL `.query()` false positive, (d)
   provider-internal SYNC enumeration or ACTION expansion (§ 2.4, § 2.5 -
   e.g. JMAP `Email/query` in `crates/jmap/src/helpers.rs:6` scoped to a
   single thread, Gmail's query-bearing `list_threads` in
   `crates/gmail/src/api.rs:76`, the IMAP `uid_search` calls in
   `crates/imap/src/client/{commands,sync}.rs`), or (e) a NEW provider
   USER-mail-search surface (which would falsify the finding and become a
   real rewire brick). Every (d) hit is enumerated and shown to be a bifrost-
   internal SYNC/ACTION primitive that retires with the provider crates at
   B15, NOT a user-search surface ratatoskr re-exports. As of this spec's
   survey, category (e) is EMPTY.
3. Confirm the `search` crate's `Cargo.toml` dependency set is
   provider-free (§ 2.2).
4. Confirm the IMAP `search_folder` / `search_all_uids` (and the underlying
   `uid_search` calls) are sync enumeration and are on the B3/B15
   disposition (§ 2.5), i.e. not a live user-search caller.

The audit's OUTPUT is a recorded finding (commit message + this survey). If
step 2 category (e) is ever non-empty at implementation time (a provider
USER-mail-search caller was added after this spec was written), THAT caller
becomes a genuine rewire brick and this spec must be re-opened to specify
its migration onto a bifrost surface (or, if bifrost has none, onto the
local pipeline) before landing - the spec must not silently ship an
incomplete audit.

### 4.2 The invariant gate (the code artifacts, and what each one actually gates)

The threat this item guards is precise (§ 7 item 2): a future change bolting
a hand-rolled provider USER-mail-search onto the local pipeline. The
non-obvious fact that shapes the gate is WHERE that threat can land. The
user-facing router is `rtsk::search_pipeline::search` (package name `rtsk`,
`crates/core/`), and `rtsk` ALREADY depends on `common`, `gmail`, `jmap`,
`graph`, `imap`, and `bifrost-jmap` (`crates/core/Cargo.toml:31-53`). So a
provider search added in `rtsk` (or `service`, which depends on the same)
would change NO manifest edge and trip NO dependency guard. A manifest guard
therefore cannot be the gate against the named threat; it can only protect
the one crate that has no provider deps to begin with. The spec states this
honestly rather than dressing a leaf-crate purity check up as the enforcing
invariant. B10 lays TWO gates, each scoped to what it can actually prove:

**Gate A - `search` leaf-crate isolation guard (dependency-level).** A test
in the `search` crate asserting that its own direct dependency set (read
from `env!("CARGO_MANIFEST_DIR")/Cargo.toml` via a `cargo_toml` parse -
precedent: `crates/db-read-lockdown/tests/lockdown.rs`, which does exactly
this style of manifest lockdown) contains none of `common`, `gmail`,
`graph`, `jmap`, `imap`, `provider-sync`, `bifrost-sync`, `bifrost-types`,
or any `bifrost-*`. This is a REAL but NARROW property: it pins that the
leaf tantivy crate stays provider-free. It is NOT the gate against the named
threat, because nobody would add provider delegation in the leaf crate; it
is named here as what it is, a purity guard on the crate that is already
clean.

**Gate B - `rtsk`/`service` search-path source lockdown (the load-bearing
gate).** Because the threat lives in a crate that already links every
provider, a manifest guard cannot catch it and a "does search succeed with
no provider handle in scope" behavioral test proves nothing (the public
`rtsk::search_pipeline::search` fn - `crates/core/src/search_pipeline.rs:65`
- takes no provider/engine handle in its signature, so "it runs without one"
is structurally trivial and does not prove no provider call happened
elsewhere on some future path). The gate that DOES bite is a mechanized
version of the § 4.1 audit step 2, turning the one-time manual sweep into a
permanent test: a lockdown test (same walk-the-rust-files shape as
`db-read-lockdown/tests/lockdown.rs`) over the `rtsk` and `service`
search-path source that asserts no call to an `engine`/`SyncEngine` search
method (`directory_search`, `email_query`, `..._search`, `.query(` against a
provider handle) appears in a mail-search code path other than the single
allow-listed B8 GAL call at `crates/service/src/handlers/gal.rs:167`. A new
provider-search call added to the router trips this scan; that is the
keep/revert gate B10 actually earns. (This replaces the earlier draft's
"manifest guard on the `core` search module" fallback, which was impossible
- Cargo dependencies are crate-wide, and `rtsk` is one crate depending on
all four providers, so a module-scoped manifest guard does not exist.)

The two gates are complementary: Gate A pins the clean leaf, Gate B pins the
dirty-by-necessity router. Neither the vacuous "runs without a handle"
behavioral test nor the impossible module-manifest fallback is carried
forward.

Exact gate commands (clause 5, copy-pasteable; package is `rtsk`, NOT
`core`):

```
brokkr test -p search local_search_links_no_provider_surface
brokkr test -p rtsk search_pipeline_routes_local_only
```

The first runs Gate A (in the `search` crate); the second runs Gate B (the
`rtsk`/`service` search-path source lockdown, placed in the `rtsk` crate).
These names and placements are pinned intent - implement under these exact
names so the § 6 gate commands resolve without substitution.

### 4.3 Migration-doc reconciliation (rides with the code landing)

Update `docs/bifrost-migration.md`: annotate the § 5 "Rewired: ...
server-side search" entry (or add a B10 done-note in § 7) recording that the
server-side-search seam is CLOSED - its sole live instance
(`directory_search`) migrated in B8, mail search is local by § 3 design, and
the invariant is now gated (§ 4.2). This markdown change is bundled with the
§ 4.2 code brick in the same commit (never a standalone markdown commit).
The full crate-map / `reference/architecture.md` reconciliation remains
B16's job; B10 only records its own closure in the migration doc.

## 5. Stopping rule (clause 9)

- IN: the search-scoped maximal-integration audit (§ 4.1), the local-search
  invariant gate (§ 4.2), and the migration-doc closure note (§ 4.3).
- OUT, named not deferred (clause 3):
  - Contacts / GAL `directory_search`: already migrated (B8). B10 does not
    touch it.
  - Local mail search (tantivy + smart-folder SQL): app-level BY DESIGN
    (`docs/bifrost-migration.md` § 3; `problem-statement.md` Open Q4). B10
    keeps it exactly as-is and must NOT wire any provider delegation - doing
    so would violate § 1's feature-preserving mandate.
  - IMAP `search_folder` / `search_all_uids` (sync-time UID enumeration):
    B3 sync cutover + B15 provider-crate deletion.
  - Server-side FILTERS / Sieve: B11 (`filter_*`), a separate item. B10 is
    search, not filters, despite § 5 pairing the phrase "search/filters".
  - Provider-crate deletion and the whole-workspace maximal-integration
    audit: B15. B10 discharges only the SEARCH slice early and records it.
  - Any future server-side mail-search PRODUCT feature: out of the
    bifrost-migration entirely (a new feature, not a feature-preserving
    plumbing swap).
- Blast radius: two new tests (Gate A in `search`, Gate B in `rtsk`) plus a
  migration-doc note. No production code path changes. No `app` /
  `service-api` / wire change; no schema / cursor / table change; no bifrost
  change; no action-pipeline change.

## 6. Verification per brick (clause 5)

B10 is the rare item whose behavioral surface genuinely cannot regress
(it changes no production code path), so the gate that stands in is
`brokkr check` green plus the new invariant test - and the spec says so
explicitly, per clause 5's provision for a behavior no instrument can
otherwise pin.

- The two invariant gates (§ 4.2), the behavior B10 adds (package is `rtsk`,
  NOT `core`):

```
brokkr test -p search local_search_links_no_provider_surface
brokkr test -p rtsk search_pipeline_routes_local_only
```

- The universal green-tree gate:

```
brokkr check
```

- The Service-boundary backstop - a GENERAL green-tree gate that the Service
  boot + handler surface still assembles, NOT proof of the specific
  local-search or GAL behaviors:

```
brokkr service-suite
```

  Precisely: the suite's search endpoint calls
  `search::SearchReadState::search_with_filters` directly
  (`crates/service/src/handlers/test_helpers.rs:1502`), which BYPASSES
  `rtsk::search_pipeline::search` - so it does not exercise the router B10's
  Gate B locks down. GAL coverage lives in the contacts sync harness (below),
  which B10 does not rerun. `service-suite` is therefore a backstop that the
  audit did not break the boot state, not coverage of either behavior; the
  real behavioral gates are Gate B (§ 4.2) for the router and B8's contacts
  harness for GAL.

- The GAL `directory_search` path is NOT re-gated here: its behavioral gate
  is B8's contacts sync-harness suite
  (`crates/app/tests/sync-harness/contacts/`, `docs/bifrost-migration.md:1428-1436`),
  which B10 does not modify and which already proves the one live
  provider-side search call. Re-running it is unnecessary because B10 does
  not touch that code; it is named here so a reviewer knows the seam's sole
  instance is gated elsewhere, not ungated.

## 7. The falsifiability challenge (why "empty" is a finding, not a hand-wave)

A confirm-and-close spec earns its keep only if it states what would refute
it and shows the check was run. The B10 finding is refuted if ANY of these
is true at implementation time:

1. `ProviderOps` (or any surviving provider surface) grows a search/query
   method with a live `core`/`service` caller. (§ 2.1 audit step 1-2. As
   surveyed: false.)
2. Any crate's mail path calls `engine`/`SyncEngine` search as a USER mail
   search, other than the B8 GAL `directory_search`. (§ 4.1 audit step 2
   category (e). As surveyed: empty. § 4.2 Gate B keeps it empty by locking
   down the `rtsk`/`service` search-path source.)
3. The `search` crate acquires a provider/bifrost dependency edge. (§ 4.2
   Gate A. As surveyed: false; the gate makes it stay false.)
4. `problem-statement.md` Open Q4 is reversed to REQUIRE provider-side mail
   search. (That would be a new product feature outside the
   feature-preserving migration - a separate item, not a B10 reopening.)

If (1) or (2) is non-empty at land, B10 stops being confirm-and-close and
becomes a real rewire: the found caller is migrated onto a bifrost surface
(or, if bifrost exposes none, kept on the local pipeline with the § 1
maximal-integration duplicate deleted), specified to this document's
standard before landing. Absent that, the finding stands and B10 closes the
seam by audit + gate, which is the correct and complete disposition of a
seam whose only occupant already migrated.

## 8. Review reconciliation (R1 + R2 folded)

This spec was revised against two independent reviews (Opus R1, Codex R2).
Every finding was validated against the frozen tree; all were accurate and
have been folded. None were rejected.

- **Gate does not gate the named threat** (R1 §1, R2 §1). The router is
  `rtsk::search_pipeline::search` and `rtsk` already links every provider
  (`crates/core/Cargo.toml:31-53`), so the search-crate manifest guard is
  orthogonal to the threat. FOLDED: § 4.2 now splits into Gate A (honest
  leaf-crate purity guard) and Gate B (the load-bearing `rtsk`/`service`
  source lockdown that mechanizes § 4.1). § 1 summary and § 7 updated.
- **Impossible module-manifest fallback** (R1 §2, R2 §1). Cargo deps are
  crate-wide; there is no per-module manifest guard on `rtsk`. FOLDED: the
  fallback is removed and explicitly retired in § 4.2.
- **Vacuous behavioral assertion** (R1 §3). `search()` takes no provider
  handle (`search_pipeline.rs:65`), so "runs without a handle" proves
  nothing. FOLDED: replaced by Gate B's source lockdown; § 4.2 states why the
  behavioral test is not carried forward.
- **Invalid `-p core` command** (R2 §2). Package is `rtsk`
  (`crates/core/Cargo.toml:2`). FOLDED: § 4.2 and § 6 commands corrected to
  `-p rtsk`; the "substitute at land" latitude removed - names are pinned.
- **Audit not whole-workspace** (R2 §3). Intro claimed a whole-workspace
  audit but the sweep covered only `crates/core/src` + `crates/service/src`,
  missing JMAP `Email/query` (`jmap/src/helpers.rs:6`), Gmail `list_threads`
  (`gmail/src/api.rs:76`), and the IMAP `uid_search` calls. FOLDED: § 4.1
  step 2 now sweeps all `crates/*/src`, enumerates and classifies those
  query-shaped primitives as sync/action (category (d)), and the intro's
  claim is scoped to match.
- **`service-suite` overclaim** (R2 §4). The suite's search endpoint calls
  `search_with_filters` directly (`test_helpers.rs:1502`), bypassing the
  router; GAL rides the contacts harness. FOLDED: § 6 reframes `service-suite`
  as a general green-tree backstop, not behavioral coverage of either path.
- **B15 exemption** (R2 §5). `docs/bifrost-migration.md:1499-1504` mandates a
  whole-workspace audit at B15. FOLDED: § 4.1 now states B10 records INPUT to
  B15, not a waiver of the search slice.
- **Doc-landing contradiction** (R2 §6). Old § 1 said the note rides with
  B11; § 3/§ 4.3 said B10's own commit. FOLDED: resolved to B10's own commit
  (it lands real test bricks) in § 1, § 3.
- **GAL invocation inaccuracy** (R1 §4, R2 §7). The call passes
  `String::new()`, not a user query, paging the whole directory into a 24h
  cache. FOLDED: § 1 corrected and the enumeration-vs-user-query distinction
  made explicit.
