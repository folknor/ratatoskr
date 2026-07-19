# B8 technical-implementation-spec: contacts on the bifrost contact surface

Replace ratatoskr's four hand-rolled provider contact stacks - Google
People (main + otherContacts), Microsoft Graph contacts (+ Exchange
groups), JMAP `ContactCard`, and the dormant hand-rolled `core/carddav`
- plus the Graph/Google GAL directory fetch, with the unified bifrost
contact surface: the `Account` contact primitives
(`address_books_list` / `contacts_list` / `contact_get` /
`contact_create` / `contact_update` / `contact_delete`) and
`directory_search`, driven off the resident engine account. This is the
contacts leg of the maximal-integration migration
(`docs/bifrost-migration.md` § 1, § 7 B8).

This spec is written against `reference/technical-implementation-spec.md`
(the contract it must satisfy - READ IT) and conforms to its ten
clauses. It is one item of `docs/bifrost-migration.md` (the governing
plan and TODO source - READ § 1, § 2, § 3, § 7 B8, § 9, § 10, § 11),
run through `reference/orchestrate.md`.

**Feature-preserving mandate (governing principle,
`docs/bifrost-migration.md` § 1).** B8 is a feature-preserving plumbing
replacement. Every contact capability ratatoskr ships today survives:
provider contact READ into the local `contacts` table (source-tagged,
prune-on-delete), single-contact WRITE-back (phone / company / notes; the
display name stays a local-only override), single-contact DELETE
(provider-first for synced contacts), and the GAL / organization
directory cache. Neither drop a live capability nor newly-wire one that
is not wired today, EXCEPT where the § 2 first principle (bifrost is
fixed first, then ratatoskr inherits it) legitimately lights up a
surface that ratatoskr currently stubs or leaves dormant - CardDAV
contacts, which today are a stubbed write-back
(`actions/contacts.rs:342`, `:408`) plus an unwired `core/carddav`
parser. Those two exceptions are called out explicitly in § 5.5 and are
the only capability the end state ADDS.

## Required reading (clause 10)

Every implementer and reviewer MUST read these before laying a brick.
Naming them is not enough; they are the ground this work is built on and
judged against.

- `reference/technical-implementation-spec.md` - the contract this spec
  is written against. The ten clauses below are its clauses.
- `reference/architecture.md` - ALWAYS required. The
  `MailActionIntent -> resolve_intent -> build_execution_plan ->
  batch_execute` action pipeline, the `ActionOutcome` / `OperationResult`
  taxonomy, the `core`/`app`/`service` crate boundary, and the Service
  IPC firewall (`AccountError` never crosses it) all bind the write-back
  leg (§ 5.4). Contacts are NOT a `MailActionIntent` today (they run
  through the dedicated `contacts.*` IPC handlers, not the 12-action
  pipeline); the cut keeps that separation.
- `docs/bifrost-migration.md` - the TODO source. § 1 (maximal
  integration - no parallel hand-rolled contact surface survives), § 2
  (the first principle and the side-quest protocol - B8 needs three
  bifrost side-quests, § 3 below), § 3 (target seam: bifrost is service-side, the
  app stays bifrost-free), § 7 B8 ("Needs B1; A7 for DAV"), § 9, § 10
  (behavioral gates are mandatory), § 11 (the frozen `../bifrost` commit
  discipline).
- `reference/glossary/harness.md` - the Service test harness,
  sync-harness scripts, `brokkr service-test` / `service-suite` /
  `sync-bench`, `saehrimnir` mock servers, and gate baselines. EVERY
  behavioral gate this spec pins (§ 7) is defined there. `saehrimnir`
  must grow contact / directory endpoints for the four providers if it
  does not already carry them (§ 7 names this as a gated instrument-build
  brick).
- `research/bifrost/reference/carddav.md` - the bifrost-carddav contact
  `Account` impl (A7, landed): `address_books_list`, `contacts_list`
  (offset-cursor paging + multiget hydration), `contact_get` /
  `contact_create` / `contact_update` / `contact_delete`, the ctag
  short-circuit, and the mass-delete-suppression hardening the pull's
  prune logic must respect (§ 5.3).
- `research/bifrost/reference/{google,graph,jmap}.md` - the native
  provider `Account` impls' contact primitives (People contacts, Graph
  contact folders, JMAP `ContactCard` / `AddressBook`) and, for Graph and
  Google, `directory_search` (`/users`, `listDirectoryPeople`).
- `research/bifrost/reference/sync.md` - the `SyncEngine` surface. The
  contact primitives are reached through the engine's `live_account`
  passthrough layer (mail mutations already do this,
  `engine.rs:1539-1748`; the folder list does it at `containers_list`,
  `engine.rs:1766`). B8 adds contact passthroughs alongside them (§ 3).

**Frozen bifrost commit.** `../bifrost` (the build dependency) and
`./research/bifrost` (the reading reference this spec cites by line
number) are the same tree. This spec was authored against
`./research/bifrost` at `a0a18c2`. Three bifrost side-quests land first
(§ 3 - B8-SQ0, B8-SQ1, B8-SQ3), advancing that HEAD; record the resulting
frozen `../bifrost` commit in the ground survey of the first ratatoskr
sub-spec landing and hold it for the item's full duration per
`docs/bifrost-migration.md` § 11. Do not let `../bifrost` mutate under an
in-flight ratatoskr step. NOTE on citations: the line numbers this spec
gives (bifrost and ratatoskr) are accurate to within a few lines at
authoring time but are NOT load-bearing to the digit - e.g.
`containers_list` is the fn at `engine.rs:1762` with the `live_account`
call body at `:1766`. Re-confirm the cited symbol, not the exact line, at
the frozen commit.

## 1. The goal (clause 7: the target as concrete artifacts)

Today ratatoskr owns four complete provider contact clients plus a GAL
fetch and a dormant CardDAV parser. All of it is protocol I/O that
bifrost already re-homes behind one `Account` surface. After B8, bifrost
owns every contact byte on the wire; ratatoskr owns only the local
`contacts` / `contact_groups` / `gal_cache` tables, their FTS, dedup,
import, and local search - and one provider-agnostic pull pass, one
provider-agnostic write path, and one provider-agnostic directory fetch
that sit on top of the bifrost surface.

The bifrost contact surface is already complete (authored against
`research/bifrost/crates/types/src/account.rs:591-650` and `:888`):

```
trait Account {
    fn address_books_list(&self) -> ... Result<Vec<AddressBook>, AccountError>;
    fn contacts_list(&self, address_book: Option<AddressBookId>, cursor: ...)
        -> ... Result<Page<ContactCard>, AccountError>;
    fn contact_get(&self, contact: ContactId) -> ... Result<ContactCard, AccountError>;
    fn contact_create(&self, contact: ContactCreate) -> ... Result<ContactId, AccountError>;
    fn contact_update(&self, contact: ContactId, patch: ContactPatch)
        -> ... Result<(), AccountError>;
    fn contact_delete(&self, contact: ContactId) -> ... Result<(), AccountError>;
    fn contact_search(&self, request: ContactSearchRequest)
        -> ... Result<Page<ContactCard>, AccountError>;
    fn directory_search(&self, query, cursor, limit)
        -> ... Result<Page<DirectoryCard>, AccountError>;   // GAL
}
```

`ContactCard` / `AddressBook` / `ContactCreate` / `ContactPatch` /
`ContactId` / `AddressBookId` live in
`research/bifrost/crates/types/src/contact.rs`; `DirectoryCard` in
`research/bifrost/crates/types/src/directory.rs`; the capability gates
(`address_books_list`, `contacts_list`, `contact_create/update/delete`,
`contact_search`, `directory_search`) in
`research/bifrost/crates/types/src/capabilities.rs:226-243`.

The target seam, pinned to concrete types:

```
ResidentEngine (service-owned, holds each account attached)
  -> NEW SyncEngine contact passthroughs (bifrost side-quest, § 3):
       address_books_list / contacts_list / contact_get /
       contact_create / contact_update / contact_delete / directory_search
       (each resolves via live_account, exactly like containers_list)

READ  (replaces the four per-provider aux contact syncs)
  resident_aux_loop -> run_aux_pass (crates/service/src/bifrost/resident.rs)
    -> run_contact_pull(engine, account_id, write_db, read_db)   [NEW, provider-agnostic]
         1. capability probe (Unsupported -> empty report, skip)
         2. enumerate account-wide: page contacts_list(None) -> Vec<ContactCard>
            + union of Page::failed_ids  (per-book only where required, e.g.
            CardDAV; account-wide avoids Google per-group amplification, § 5.3)
         3. map ContactCard -> Vec<ContactWriteRow> (§ 5.2), upsert into
            `contacts` (google_other corpus -> `seen_addresses`, B8-SQ1);
            record remote CLAIMS in `contact_claims` keyed
            (account_id, source, server_id, email) (§ 5.6)
         4. snapshot-reconcile: retire claims whose server_id is absent from
            the fetched set AND not in failed_ids; delete a `contacts` row
            only when no claim references its email (preserves cross-provider
            dedup; replaces graph prune_stale / jmap destroyed, uniformly)

WRITE (replaces service/actions/contacts.rs dispatch_write_back / dispatch_delete)
  contacts.contact_save_with_writeback / contacts.contact_delete IPC
    -> save_contact / delete_contact (service/actions/contacts.rs)
         -> engine.contact_update(account_id, ContactId(server_id), ContactPatch{ phone, company, notes })
         -> engine.contact_delete(account_id, ContactId(server_id))
       (ActionOutcome::Success / LocalOnly / Failed unchanged; wire
        WritebackOutcome unchanged)

GAL   (replaces core/contacts/gal.rs fetch_graph_gal / fetch_google_gal)
  gal.kick handler (service/src/handlers/gal.rs)
    -> engine.directory_search(account_id, "", limit, cursor) -> Page<DirectoryCard>
       (trait arg order is (query, limit, page_cursor), account.rs:645 - NOT
        (query, cursor, limit); the return is a Page, drained via next_cursor)
    -> gal_cache rows (unchanged table, unchanged cache-age gate)
       (Unsupported(DirectorySearch) on JMAP/IMAP -> Ok(None), matching today)
```

After B8 the following ratatoskr code is DELETED (§ 5.6): the four
per-provider contact READ clients (`crates/gmail/src/contacts/`,
`crates/graph/src/contact_sync.rs`, `crates/jmap/src/contacts_sync.rs`),
the hand-rolled `crates/core/src/carddav/` plus its `pub mod carddav`, the
provider-fetch halves of `crates/core/src/contacts/gal.rs`, the inline
write-back HTTP arms in `crates/service/src/actions/contacts.rs`, and the
provider contact sync-state helpers in `sync::state`. The provider
mapping / delta tables are NOT simply dropped: `graph_contact_delta_tokens`
retires, but the four `*_contact_map` tables are REPLACED by a unified
`contact_claims` remote-claim ledger (§ 5.6, R2-B1) - collapsing them onto
`contacts.server_id` would break cross-provider dedup. NOT deleted in B8:
`crates/graph/src/group_sync.rs` (Exchange groups, carved to the
`B8-groups` follow-up, § 3 B8-SQ2).

## 2. Survey of the ground (clause 8)

### 2.1 Contacts do NOT ride the change-stream - they are pull, at cadence

B3 inverted mail sync onto the engine broadcast, but contacts were never
on it. They run in the **resident auxiliary pass**
(`crates/service/src/bifrost/resident.rs`): `resident_aux_loop`
(line 957) wakes on `RESIDENT_AUX_CADENCE` (300 s, line 44) and calls
`run_aux_pass` (line 977), which rebuilds a per-provider client each tick
and calls `provider_sync::consumer_support::run_{jmap,graph,gmail}_auxiliary_sync`
(the IMAP aux does a PERMANENTFLAGS probe, no contacts). Those aux
runners invoke the per-provider contact sync inline:

- JMAP (`consumer_support.rs:73-107`): `jmap_contacts_delta_sync` when
  the account's initial sync completed, else `jmap_contacts_initial_sync`
  - every pass.
- Gmail (`gmail/aux_sync.rs:22-72`): a 20-cycle gate
  (`increment_gmail_sync_cycle`, `is_multiple_of(20)`) then
  `sync_google_contacts` + `sync_google_other_contacts`.
- Graph (`graph/aux_sync.rs`): `graph_contacts_initial_sync` /
  `graph_contacts_delta_sync` + `sync_exchange_groups`.

This is exactly the B7a calendar situation: contacts are a pull surface,
not a persistence-inverted change-stream. B8 mirrors B7a - one uniform
pull through the `Account` contact primitives, driven at the same
resident-aux cadence, treating any per-provider contact change-stream
(CardDAV `CursorScope::Type(Contact)`, JMAP `ContactCard/changes`, Graph
`contacts/delta`) as an OPTIONAL future optimization, never a
consumer-side special-case (`docs/bifrost-migration.md` § 2 first
principle). Uniform pull collapses the four per-provider initial-vs-delta
branches to one code path; the cadence gate (§ 5.3) preserves the "don't
refetch every 5 minutes" budget the Gmail 20-cycle gate expresses today.

### 2.2 The four provider READ impls to rip

- **JMAP** `crates/jmap/src/contacts_sync.rs` (550 LOC). `ContactCard/get`
  (all) on initial, `ContactCard/changes` on delta; JSContact field
  extraction (`extract_contact`, display name / emails / phone / org /
  notes); `persist_jmap_contact` upserts `ContactWriteRow` with
  `source = "jmap"`, `server_id`, local id `jmap-{account}-{email}`;
  `delete_contact_by_server_id_and_source_sync` on destroyed. State stored
  via `sync::state::save_jmap_sync_state(.., "ContactCard", ..)`. Also
  contains `jmap_contacts_push_update` (write-back, moves to § 2.3) and
  `get_jmap_contact_server_info` (write-back lookup).
- **Graph** `crates/graph/src/contact_sync.rs` (443 LOC). Enumerates
  `/me/contactFolders`, pages `/contacts`, `persist_synced_contacts`
  upserts `source = "graph"` AND writes `graph_contact_map`
  (account, graph_contact_id, email); delta via per-folder
  `contacts/delta` tokens in `graph_contact_delta_tokens`; `410 Gone` ->
  full re-sync; `prune_stale_contacts` removes rows whose
  graph_contact_id vanished. One Graph contact can fan out to multiple
  email rows (the map's purpose).
- **Graph groups** `crates/graph/src/group_sync.rs` (~610 LOC incl.
  tests). `sync_exchange_groups` enumerates `/groups`, classifies
  `Unified` (M365) vs distribution list (`ExchangeGroupType`), fetches
  transitive user members, and writes `contact_groups` (with
  `server_id`, `group_type`) + `contact_group_members`. THIS HAS NO
  BIFROST EQUIVALENT (bifrost's `address_books_list` models contact
  folders / address books, not directory distribution groups) - see § 3
  side-quest B8-SQ2.
- **Google** `crates/gmail/src/contacts/` (mod + `google_contacts.rs` +
  `other_contacts.rs`). `sync_google_contacts`: People
  `people/me/connections` with `nextSyncToken` delta, `google_contact_map`.
  `sync_google_other_contacts`: People `otherContacts` (auto-collected
  send/receive addresses) with its own sync token +
  `google_other_contact_map`, persisted via the `persist_google_other_contacts_write`
  closure the aux pass injects. otherContacts is a SECOND People corpus
  with no direct bifrost `contacts_list` equivalent - see § 3 side-quest
  B8-SQ1.

### 2.3 The WRITE-back path

`crates/service/src/actions/contacts.rs` (`save_contact` / `delete_contact`)
is reached from `crates/service/src/handlers/contacts.rs`
(`handle_contact_save_with_writeback`, `handle_contact_delete`) via the
`contacts.contact_save_with_writeback` / `contacts.contact_delete` IPC
methods (`crates/service-api/src/contacts.rs`). The action does the local
DB UPSERT/delete first, then `dispatch_write_back` / `dispatch_delete`
switch on `source`:

- `"jmap"` -> `jmap::contacts_sync::jmap_contacts_push_update` /
  `ContactCardSet` destroy (pushes phones / organizations / notes maps).
- `"google"` -> builds a People `updateContact` field-mask body / DELETE
  via `gmail::client` `patch_absolute` / `delete_absolute`.
- `"graph"` -> `PATCH` / `DELETE /me/contacts/{server_id}` via
  `graph::client`.
- `"carddav"` -> `ActionError::not_implemented` (STUB, both paths).
- `"user"` -> local only.

Only phone / company / notes are pushed; display name is a local-only
override (all three providers). Outcomes map: local fail -> `Failed`;
provider fail -> `LocalOnly { reason, retryable }` (save) or, for
JMAP/Google/Graph, provider-first `Failed` before the local delete
(delete). `ActionOutcome` -> wire `WritebackOutcome`
(`handlers/contacts.rs:159` `outcome_to_writeback`). The
account/server identity comes off `ContactSaveInput.{account_id,
server_id, source}` (save) or `get_contact_meta_by_id_sync` (delete).

### 2.4 GAL / organization directory

`crates/core/src/contacts/gal.rs`: `fetch_graph_gal` (`/users`
pagination, `GalEntry` extraction), `fetch_google_gal`
(`people:listDirectoryPeople`, 403 -> empty for personal accounts),
`gal_cache_age`. Orchestrated Service-side by
`crates/service/src/handlers/gal.rs` (`gal.kick` handler): per-account
60 s timeout, 24 h staleness gate, `cache_gal_entries_sync` +
`record_gal_refresh_sync` into `gal_cache`. JMAP / IMAP return `Ok(None)`
(no directory). `GalEntry` (email, display_name, phone, company, title,
department) maps field-for-field onto bifrost's `DirectoryCard`
(`research/bifrost/crates/types/src/directory.rs`) plus
`additional_emails` / `phones` vectors bifrost carries and ratatoskr
currently flattens to one.

### 2.5 The dormant CardDAV scaffolding

`crates/core/src/carddav/{client.rs,parse.rs}` (behind `pub mod carddav`,
`core/src/lib.rs:7`) plus `db` `persist_carddav_contacts_sync` and the
`carddav_contact_map` table exist but are NOT wired into any live pull
pass (the resident IMAP aux does PERMANENTFLAGS only; no caller invokes
`carddav::client`). CardDAV write-back is the § 2.3 stub. So CardDAV
contacts are effectively a no-op today. This is the one surface B8
lights up (§ 5.5) via bifrost-carddav (A7), because the maximal-
integration rule forbids a parallel hand-rolled `core/carddav` surviving
next to the bifrost equivalent.

### 2.6 What STAYS (the stopping line, cross-referenced by § 6)

Everything local. The `contacts` / `seen_addresses` /
`contact_groups` / `contact_group_members` / `contact_photo_cache` /
`gal_cache` tables and their FTS + triggers
(`crates/db/src/db/schema/03_contacts.sql`); the DB write helpers
(`upsert_contact_sync`, `save_group_sync`, `cache_gal_entries_sync`, the
`db_upsert_contact_full` action helper, etc.); local contact SEARCH and
autocomplete (`crates/core/src/contacts/search.rs` is a re-export shim
over `db::queries_extra::contact_search` - a LOCAL FTS query, the analog
of B10's "local tantivy stays app-level", so bifrost's provider-side
`contact_search` / `contact_autocomplete` are NOT consumed by B8);
contact import (`crates/import/`), dedup, and the group editor
(`contacts.group_save` / `group_delete` / local `contact_save` IPC
handlers - these are local-only DB writes and do not touch a provider).
`seen_addresses` (observed-from-mail addresses) is populated by the mail
sync `seen_ingest` pass (B3) and is out of scope AS A LOCAL TABLE - but
correct an earlier over-statement (R1-F1, R2-B2): contact sync DOES write
`seen_addresses` for one corpus, Google `otherContacts`
(`source='google_other'`, `crates/gmail/src/contacts/other_contacts.rs:20-23`).
B8 preserves that: the otherContacts leg of the pull writes
`seen_addresses` via `upsert_seen_address_google_other` (NOT `contacts`),
preserving `local_observed` rows (B8-SQ1, § 5.3). What stays out of scope
is the `seen_ingest` mail-sync writer and the table's schema/FTS, not the
otherContacts corpus, which B8 must keep flowing.

### 2.7 Where the account handle comes from

The resident engine already holds every account attached
(`ResidentEngine::attach_account`, `resident.rs:206`) and reaches
provider operations through the engine's `live_account` passthroughs
(private `engine.rs:1404`, exposed publicly as `containers_list`
`:1766`, `mark_read` etc. `:1539-1748`). The mail-action pipeline uses
`ResidentActionAccount` (`resident.rs:97`) carrying `Arc<SyncEngine>` to
reach those passthroughs. B8's read pull runs inside `run_aux_pass`
where the resident engine is in scope; B8's write-back runs where the
action pipeline already resolves `ResidentActionAccount`. Both reach the
NEW contact passthroughs (§ 3) on the resident `Arc<SyncEngine>` - no
fresh `AccountFactory::open` per pass, reusing the resident connection
(the amortization the resident lifecycle exists for).

## 3. Bifrost side-quests this item requires (clause 2 / first principle)

Per `docs/bifrost-migration.md` § 2, bifrost is fixed FIRST, in the
bifrost repo, before the corresponding ratatoskr work. B8 needs three
BIFROST changes - B8-SQ0 (engine passthroughs), B8-SQ1 (Google
otherContacts corpus), and B8-SQ3 (carddav `failed_ids`) - each landed via
the side-quest protocol (§ 2 of the migration doc: one Opus agent confined
to `./research/bifrost`, then orchestrator review / validate / commit /
`bash scripts/bifrost.sh`) BEFORE the ratatoskr bricks in § 5. B8-SQ2
(Exchange groups) is listed below too but is a DISPOSITION, not a bifrost
change: its resolution is to carve the capability out of B8 (no bifrost
work for B8), so it does not count toward the three.

- **B8-SQ0 - SyncEngine contact passthroughs.** The engine exposes NO
  contact methods today (grep of `engine.rs` for `contacts_list` /
  `contact_update` / `directory_search` is empty; only `live_account`
  private + mail/container passthroughs exist). Add passthroughs for
  `address_books_list`, `contacts_list`, `contact_get`,
  `contact_create`, `contact_update`, `contact_delete`, and
  `directory_search`, each resolving through `live_account` exactly like
  `containers_list` (`engine.rs:1762`, `live_account` call body `:1766`)
  and the mail mutations (`:1539-1748`). The passthroughs mirror the
  `Account` trait's ARGUMENT shapes (`account.rs:591-650`) but NOT its
  return type or receiver: like `containers_list` they take
  `account_id: &AccountId` (NOT `&str`) and return
  `Result<T, bifrost_sync::Error>` (NOT `Result<T, AccountError>`) -
  `live_account()` yields `Error::AccountNotAttached` and the trait's
  `AccountError` is folded in through `?`. Consumers therefore match
  `bifrost_sync::Error`, and the ratatoskr action boundary converts THAT
  to `ActionOutcome` via the existing bifrost error mapper (§ 5.4), not
  `AccountError` directly. Mind `directory_search`'s real arg order:
  `(query, limit, page_cursor)` (`account.rs:645`), not
  `(query, cursor, limit)`. This is pure delegation; the risk is low and
  the shape is already proven by the mail passthroughs. Without it the
  resident pull / write / GAL legs cannot reach the account.
- **B8-SQ1 - Google otherContacts corpus.** ratatoskr syncs Google
  `otherContacts` (auto-collected addresses) as a distinct People corpus
  (§ 2.2). CRITICAL corpus fact (was a contradiction in this spec; folded
  from R1-F1/F3, R2-B2): otherContacts do NOT land in the `contacts`
  table. They are upserted into `seen_addresses` with `source =
  'google_other'` as lower-priority autocomplete candidates
  (`crates/gmail/src/contacts/other_contacts.rs:20-23,:262,:283`,
  preserving locally observed rows). So the flat claim in § 2.6 / § 6 that
  contact sync "never touches `seen_addresses`" is FALSE for this one
  corpus, and `source` is NOT a pure function of provider kind (a single
  Google account emits both `"google"` and `"google_other"`). Two things
  this side-quest must land, not defer: (1) confirm/extend bifrost-google
  so `contacts_list` can surface otherContacts, ideally as a synthetic
  address book carrying a CORPUS DISCRIMINATOR on the `AddressBook` /
  `ContactCard` (main vs other) so the consumer can route without a Google
  `match`; (2) B8's pull (§ 5.3) must route the otherContacts corpus to
  the `seen_addresses` write path (`upsert_seen_address_google_other`,
  preserving `local_observed` rows) NOT `contacts` - this is a
  corpus-shaped write target, so § 5.1's "the only provider-shaped value is
  the source tag" is amended: the (provider, corpus) pair drives the write
  target. If the port evidence shows otherContacts was simply not carried
  over yet, this is a "carry it over" side-quest per § 2's origin note. The
  reviewer confirms against `research/bifrost/reference/google.md` and
  `crates/google/src/account/contacts.rs` before ratatoskr work starts.
- **B8-SQ2 - Exchange groups (distribution lists / M365 groups).**
  ratatoskr's `group_sync.rs` enumerates `/groups`, classifies
  Unified-vs-distribution, and fetches transitive members into
  `contact_groups` (§ 2.2). Bifrost's `address_books_list` models contact
  folders, not directory groups, so there is no equivalent today. R2-B5
  flags (correctly) that leaving this "decided during the survey"
  half-violates the spec contract's "resolve obstacles in the spec" rule
  AND contradicts § 5.6's grep audit (which lists `group_sync` as
  unconditionally absent post-B8g). Both are fixed by PINNING the lane
  now, not at survey time. Pinned disposition: **(b)** - Exchange group
  sync (`/groups` enumeration, Unified-vs-distribution classification,
  transitive member fetch into `contact_groups` / `contact_group_members`)
  is Graph-specific org-directory territory adjacent to GAL, has no
  cross-provider analog, and forcing it into `address_books_list` would
  contort the shared model. It is therefore hereby CARVED OUT of B8 into a
  named, separate governing-plan TODO item (add it to
  `docs/bifrost-migration.md` § 7 as "B8-groups: Exchange distribution /
  M365 group sync onto a bifrost groups surface") - a genuinely separate
  concern, NOT a B8 deferral hole. Consequence, reconciled into § 5.6 and
  the § 5.6 grep audit: `crates/graph/src/group_sync.rs` and
  `sync_exchange_groups` STAY in place through B8; B8g does NOT delete
  them and the grep audit does NOT list `group_sync`; that deletion is
  owed to the B8-groups item. Option (a) (a bifrost groups surface
  consumed uniformly) is explicitly deferred to that item, not attempted
  in B8.

- **B8-SQ3 - carddav `failed_ids` (snapshot-reconcile safety).** bifrost's
  `Page::failed_ids` (`research/bifrost/crates/types/src/page.rs:24-30`)
  exists precisely so a consumer can tell a transient per-resource
  hydration failure apart from a real remote deletion - the snapshot prune
  (§ 5.3) depends on it. But bifrost-carddav's `contacts_list` currently
  drops malformed hydrated vCards with `filter_map(..ok())` and returns an
  EMPTY `failed_ids` (`research/bifrost/crates/carddav/src/account.rs:203-224`).
  Under the snapshot prune those silently-dropped resources would look like
  deletions and get pruned from `contacts`. This side-quest makes
  bifrost-carddav record the native ids of vCards it fetched but could not
  parse into `Page::failed_ids` instead of dropping them silently. Without
  it, CardDAV cannot join the uniform snapshot-reconcile safely; it is a
  prerequisite for the CardDAV read cutover (B8b-d / § 5.5), not the
  write-back. (The JMAP/Graph/Google Account impls already surface their
  failed hydrations; confirm during the survey and extend any that do not.)

The frozen `../bifrost` commit for the ratatoskr bricks is the HEAD
after B8-SQ0, B8-SQ1, and B8-SQ3 land and `bifrost.sh` promotes it
(B8-SQ2 needs no bifrost work).

## 4. The split (clause 6: keep/revert, ordered so the tree stays green)

Each landing is one coherent, fully intrusive change kept or reverted on
its gates. Ordered so `brokkr check` is green at every boundary. A
provider read cannot be deleted until its uniform-pull replacement
persists identity-equivalent rows (identity columns match; enriched
fields additively populated, § 5.2), so the reads cut over per-provider
behind one new pull path, then the dead code is deleted. The three bifrost
side-quests (B8-SQ0, B8-SQ1, B8-SQ3) and the CardDAV factory composition
(§ 5.5) land BEFORE the ratatoskr bricks they gate.

- **B8a - the pull path + mapping (additive).** Land
  `run_contact_pull` + the `ContactCard -> Vec<ContactWriteRow>` mapping
  (§ 5.2) + the `contact_claims` ledger (§ 5.6) + the failed_ids-aware
  snapshot-reconcile (§ 5.3), wired into `run_aux_pass` for ONE provider
  first (JMAP - smallest, single address book, `server_id`-keyed), with the
  old JMAP aux contact sync REMOVED in the same landing (no dual-write).
  The other three providers still run their old aux sync. Green boundary:
  JMAP contacts pull through bifrost, the other three unchanged.
- **B8b / B8c / B8d - per-provider read cutovers.** Graph (incl. the
  fan-out projection + per-email claim rows, § 5.3), Google (main corpus +
  otherContacts-to-`seen_addresses` via B8-SQ1), CardDAV lit up (§ 5.5,
  requires the factory composition + B8-SQ3 landed first). Each removes
  that provider's old aux contact call and its sync-state/map usage in the
  same landing. Exchange groups are NOT touched (B8-SQ2 (b): `group_sync.rs`
  stays).
- **B8e - write-back cutover.** Re-point `dispatch_write_back` /
  `dispatch_delete` onto the engine `contact_update` / `contact_delete`
  passthroughs for all four sources at once (they share one `ContactPatch`
  mapping); CardDAV stops being a stub. Wire shape (`WritebackOutcome`)
  and `ActionOutcome` mapping unchanged.
- **B8f - GAL cutover.** Re-point `fetch_graph_gal` / `fetch_google_gal`
  callers onto `directory_search`; delete the two provider fetch fns; keep
  `gal_cache_age` and the whole `gal.kick` orchestration + `gal_cache`
  table.
- **B8g - deletion + collapse.** Delete the three provider READ contact
  modules, `core/carddav`, the provider sync-state helpers, and the
  delta-token table; migrate the four `*_contact_map` tables into
  `contact_claims` (v100 schema edit, § 5.6). `group_sync.rs` is NOT
  deleted here. The final green cut for contacts.

The exact intra-item ordering (which of B8b-d first) is pinned by the
implementer per the green-tree rule; JMAP-first for B8a is fixed because
it is the cleanest mapping (`server_id` == native id, single address
book).

## 5. The bricks

### 5.1 The contact passthroughs are reached, not rebuilt (clause 4: no shoehorning)

`run_contact_pull` and the write/GAL legs consume the B8-SQ0 engine
passthroughs directly. No ratatoskr-side provider dispatch, no `match
provider {}` over contact logic - the engine is provider-agnostic at this
seam (the `prepare_folder_map` / `containers_list` precedent,
`resident.rs:520`). The provider-shaped values that remain
ratatoskr-side are the `source` string tag written to `contacts.source`
(`"jmap"` / `"graph"` / `"google"` / `"carddav"`) AND, for Google, the
`"google_other"` corpus tag written to `seen_addresses` (B8-SQ1). Source
is therefore derived from the (provider kind, corpus) pair, NOT provider
kind alone (R1-F3): one Google account emits `"google"` (main ->
`contacts`) and `"google_other"` (otherContacts -> `seen_addresses`).
These tags matter because the local schema and the existing
`ContactWriteRow` upsert conflict-resolution (never overwrite
`source='user'`, respect `display_name_overridden`) key on them, and the
snapshot reconcile (§ 5.3) is source-scoped - it must never let the
`"google"` prune touch `"google_other"` rows or vice-versa.

### 5.2 `ContactCard -> ContactWriteRow` mapping (concrete artifact)

A single pure function (unit-testable, § 7). It returns a `Vec`, NOT an
`Option` (R1-F2, R2-B9): the Graph fan-out (§ 5.3, one row per email)
cannot be expressed by a single-row return, so the mapper emits zero, one,
or N rows and the fan-out-vs-`email2` choice lives inside it, keyed off a
`ContactProjection` value the caller passes (the (provider, corpus)-shaped
projection is the second ratatoskr-side provider-shaped value, § 5.1 - not
a `match provider {}` in the pull loop, but an explicit projection policy
threaded in):

```rust
// crates/service/src/bifrost/contacts/map.rs (NEW)
enum ContactProjection { PackSecondEmail, FanOutPerEmail }  // JMAP/Google/CardDAV vs Graph
fn contact_write_rows(card: &ContactCard, account_id: &str, source: &str,
    projection: ContactProjection) -> Vec<ContactWriteRow>
```

- `email`: each source email lowercased. Empty email set -> return `[]`
  (skip; matches every current provider's "no email -> no row" rule,
  `jmap` `extract_contact`, `graph` `extract_emails`, `google`
  `extract_primary_email`). Under `FanOutPerEmail` one row per email;
  under `PackSecondEmail` one row using the first email.
- `email2`: under `PackSecondEmail`, the second email if present (JMAP does
  this today). Under `FanOutPerEmail` (Graph) `email2` stays `None` and the
  extra emails become their own rows (§ 5.3).
- `display_name`: `card.display_name`, falling back to `email` (matches
  every provider's fallback).
- `phone`: first `card.phones.value`. `company`: first
  `card.organizations.name`. `notes`: `card.notes`.
- `avatar_url`: `card.photo_url` (JMAP/Graph leave `None` today; Google
  sets it - preserved).
- `source`: the provider tag. `account_id`: the account. `server_id`:
  `Some(card.native_id)` (== `card.provenance.native`).
- `id`: `format!("{source}-{account_id}-{email}")` - the EXACT existing
  local-id scheme (`jmap-{account}-{email}`, `graph-{account}-{email}`,
  `google-{account}-{email}`), so a re-pull upserts existing rows in place
  rather than orphaning them. This is load-bearing for the "no duplicate
  rows across the cut" gate (§ 7).

The richer bifrost fields (`ContactAddress`, typed email/phone kinds,
inline `ContactPhoto`, multiple orgs) are dropped at this boundary
because the local `contacts` table has no columns for them - matching
today's lossy extraction. A future schema enrichment is out of scope
(named, excluded).

**Field enrichment is real and APPROVED, not "byte-equivalent" (R2-B9).**
Today's Graph READ writes `phone` / `company` / `notes` / `email2` as
`None` unconditionally (`crates/graph/src/contact_sync.rs:309-322`), and
Google leaves `email2` / `notes` empty on READ. bifrost's `ContactCard`
carries those fields from the same provider fetch, so mapping them in
populates columns that were `NULL` before. This is strictly additive data
from the same bytes, not a new capability, so B8 APPROVES it rather than
suppressing it to match today. Consequence: the § 4 / § 8 "byte-equivalent
rows" language is downgraded to "identity-equivalent" - the cross-cut gate
(§ 7) asserts equivalence on the IDENTITY columns (row count, `email`,
`display_name`, `source`, `server_id`) and separately asserts the enriched
columns (`phone` / `company` / `notes` / `email2`) are populated where the
`ContactCard` carries them. A projection that instead reproduced today's
`None`s (to make the claim literally byte-equivalent) is the rejected
alternative, called out so the gate author does not "fix" the enrichment
as a regression.

### 5.3 The pull pass + snapshot reconcile (concrete artifact)

```rust
// crates/service/src/bifrost/contacts/pull.rs (NEW)
pub async fn run_contact_pull(
    engine: &SyncEngine,
    account_id: &str,
    source: &str,
    write_db: &WriteDbState,
) -> Result<ContactPullReport, String>
```

1. `address_books = engine.address_books_list(account_id).await`. An
   `Unsupported` error (capability flag `false`) -> return an empty report
   (the account has no contact backend; not an error). Any other
   `AccountError` -> `Err` (logged best-effort by the aux pass, same as
   today's `log::warn!` on contact sync failure - contacts never fail a
   sync kick).
2. Enumerate the account's contacts. **Prefer the account-wide route
   `engine.contacts_list(account_id, None, cursor)`** (address_book =
   `None`) paging `Page::next_cursor` to exhaustion, NOT a per-book loop.
   Rationale (R2-B7): bifrost-google's `address_books_list` returns the
   default book PLUS every contact group
   (`research/bifrost/crates/google/src/account/contacts.rs:22-58`), and a
   scoped `contacts_list(Some(book))` fetches ALL
   `/people/me/connections` and filters group membership locally (`:62-91`).
   So iterating books and paging each = roughly one full contact download
   PER GROUP - request amplification cadence cannot fix. Account-wide
   `contacts_list(None)` is one pass over the corpus. Per-book iteration is
   used ONLY where a provider requires it (bifrost-carddav's multiget is
   addressbook-scoped, `reference/carddav.md`; if `None` is unsupported
   there, fall back to `address_books_list` + per-book for CardDAV only).
   Collect `(Vec<ContactCard>, failed_ids: Vec<String>)` across the pages
   (union the per-page `Page::failed_ids`, § step 3).
3. In ONE `write_db.with_write` transaction per account: map each
   `ContactCard` to its `Vec<ContactWriteRow>` (§ 5.2) and
   `upsert_contact_sync` (routing the `google_other` corpus to
   `seen_addresses` instead, B8-SQ1); upsert the corresponding
   remote-claim rows (§ 5.6 claim table); collect the set of fetched
   `server_id`s; then **snapshot-reconcile** - retire every remote CLAIM
   with this `account_id` and `source` whose `server_id` is NOT in the
   fetched set, and delete the materialized `contacts` row only when NO
   claim (this or any other provider/account) still references its `email`
   (§ 5.6 - this preserves today's cross-provider dedup guard,
   `google_contacts.rs:304-326`). This one uniform prune replaces graph
   `prune_stale_contacts`, jmap `destroyed` handling, and google
   `prune_stale_*`. Two hardening rules on the "absent -> retire" step,
   both load-bearing (R2-B3):
   - **`failed_ids` are NOT deletions.** A `server_id` that the provider
     tried to hydrate but could not (parse failure in a multi-status
     response, `research/bifrost/crates/types/src/page.rs:24-30`) is
     transient, not a remote delete. EXCLUDE `failed_ids` from the "absent"
     set: a claim is retired only if its `server_id` is neither in the
     fetched set NOR in `failed_ids`. bifrost-carddav currently drops
     malformed vCards via `filter_map(..ok())` while returning an EMPTY
     `failed_ids` (`research/bifrost/crates/carddav/src/account.rs:203-224`),
     so those drops would masquerade as deletions - a bifrost side-quest
     (B8-SQ3, § 3) must make carddav populate `failed_ids` before its
     snapshot prune is safe.
   - **Transient-empty guard, bounded so it cannot block a real delete-all
     (R2-B3).** The blunt `if fetched.is_empty() && prior_count > 0 { skip
     }` guard permanently prevents legitimately deleting the last contact.
     Refine it: skip the prune only when the page enumeration did not
     complete cleanly (a non-`Ok` terminal page, or a non-empty
     `failed_ids` covering the whole prior snapshot) - i.e. an empty result
     is honored as a real delete-all when the enumeration terminated
     successfully with an empty `items` and empty `failed_ids`. This keeps
     the bifrost-carddav "no observation != destroy-all" protection
     (`reference/carddav.md`) without stranding rows forever.

The **Graph fan-out** (`ContactProjection::FanOutPerEmail`, § 5.2): one
Graph contact with N emails becomes N rows today. bifrost's `ContactCard`
carries all emails in `card.emails`, so the mapping can either (a) keep
fan-out by emitting one row per email (preserving today's row shape), or
(b) collapse to one row with `email` + `email2`. Decision pinned here:
**(a) for the read cut** to keep the cross-cut row count identical (the
gate in § 7 asserts identity-equivalence); a later simplification to (b)
is a named, excluded follow-up. Under (a) each fanned row gets its own
remote-claim row keyed `(account_id, source, server_id, email)`, and the
reconcile retires per-`(server_id, email)` claim - so removing one email
from a Graph contact retires exactly that claim, matching today's
per-email map row.

**Cadence (per-source, NOT a single uniform N - R2-B6).** The pull runs
on the resident-aux 300 s cadence. Today's cadences are NOT uniform:
JMAP pulls every pass (~5 min), Graph every ~5th cycle (~25 min), Gmail
every 20th cycle (~100 min). A single uniform `N = 20` would degrade JMAP
freshness ~20x and Graph ~4x - a feature regression the "feature-preserving
mandate" forbids. So the feature-preserving first cut PRESERVES each
provider's current effective cadence via a per-SOURCE cadence divisor
(reuse / generalize `increment_gmail_sync_cycle` into a provider-agnostic
`increment_contact_pull_cycle`, keyed by `(account_id, source)`): pull on
cycle 0 (initial) and every Nth pass thereafter where N is pinned per
source - `jmap: 1`, `graph: 5`, `google: 20`, `carddav: 5` (CardDAV is
newly lit, so a conservative 5 is chosen here, matching Graph's DAV-ish
budget; not a preservation constraint since it was dormant). A LATER move
to one uniform cadence is a named, excluded follow-up, not smuggled into
B8. The § 7 sync-bench request-budget gate pins these divisors against the
recorded per-provider baseline so a naive "every 300 s" regression is
caught.

### 5.4 Write-back cutover (concrete artifact)

`dispatch_write_back` and `dispatch_delete` in
`crates/service/src/actions/contacts.rs` lose all four provider arms and
become provider-agnostic:

```rust
// save: build a ContactPatch from the edit and call the passthrough
let patch = ContactPatch {
    phones: phone.map(|p| vec![ContactPhone { value: p.into(), kind: None, is_primary: true }]),
    organizations: company.map(|c| vec![ContactOrganization { name: c.into(), title: None }]),
    notes: notes.map(|n| Some(n.into())),  // Some(None) clears; None leaves untouched
    ..Default::default()          // display_name stays None -> not pushed (local-only override)
};
engine.contact_update(account_id, ContactId(server_id.into()), patch).await
// delete:
engine.contact_delete(account_id, ContactId(server_id.into())).await
```

Reached via `ResidentActionAccount` (the same handle the mail-action
pipeline resolves). Pin the plumbing (R2-B8): the contact IPC handlers
(`handle_contact_save_with_writeback` / `handle_contact_delete`,
`handlers/contacts.rs`) today construct only an `ActionContext`; B8
threads the resident `Arc<SyncEngine>` into `save_contact` /
`delete_contact` the SAME way the mail-action path resolves
`ResidentActionAccount` (`resident.rs:97`) - the handler resolves the
account's `ResidentActionAccount` from the resident engine and passes it
(or the engine handle plus `account_id`) into the action, rather than
opening a fresh account per call. The `ActionOutcome` mapping is
unchanged: a `bifrost_sync::Error` (wrapping `AccountError`) from the
passthrough -> `LocalOnly { reason, retryable }`
(save) or provider-first `Failed` before local delete (delete). The
`ActionError::remote` string is derived from `AccountError`'s
user-facing message (do NOT leak `AccountError` across the IPC wire -
`docs/bifrost-migration.md` § 3; convert at the action boundary as
`OperationResult`/`ActionOutcome` do). `WritebackOutcome` (`service-api`)
is byte-unchanged. CardDAV `source` now dispatches through the same
passthrough (bifrost-carddav implements `contact_update` /
`contact_delete`) instead of returning `not_implemented`.

Note the `ContactPatch` "leave untouched vs clear" semantics
(`contact.rs:142-153`: `None` untouched, `Some(None)` clears,
`Some(vec)` replaces). Today's field-mask build only pushes fields that
are `Some`, so map `Some(value) -> Some(replace)` and `None -> None`
(leave untouched), matching today. Do NOT emit `Some(None)` unless the
UI explicitly clears a field (out of scope today; the edit form always
carries the current value).

`build_execution_plan` / `resolve_intent` are untouched - contacts are
not a `MailActionIntent`. The `create` path (`contact_create`) is NOT
wired today (the import path is local-only, `service-api/contacts.rs`
"Out of scope: Bulk import"), so B8 does NOT newly-wire it; the
passthrough exists (B8-SQ0) for a future "sync uploaded contacts"
affordance but stays unused, matching today's feature set.

### 5.5 CardDAV lit up (the one added capability)

Delete `crates/core/src/carddav/` and `pub mod carddav`. CardDAV
accounts now flow through the SAME `run_contact_pull` (bifrost-carddav's
`address_books_list` / `contacts_list`, `reference/carddav.md`) and the
SAME write path (`contact_update` / `contact_delete` - NOT
`contact_create`, which stays unwired per § 5.4; the earlier draft listing
`contact_create` here was the R2-B10 contradiction, now removed). The
§ 2.3 write-back stub arms are gone (§ 5.4 handles CardDAV uniformly).
This is the § 1 exception: a dormant/stubbed surface becomes live because
A7 landed bifrost-carddav and § 1 forbids the parallel hand-rolled
`core/carddav` surviving. The `carddav_contact_map` +
`persist_carddav_contacts_sync` are reconciled in § 5.6.

**Composition seam - CardDAV must actually be attached (R2-B4, blocking).**
"CardDAV accounts flow through `run_contact_pull`" is only true if the
resident account HAS a CardDAV backend attached and advertises
`pim_methods` contact support. It does not today: bifrost-carddav composes
UNDER an IMAP-shaped account (`CalDAV composes into IMAP-shaped accounts`,
same for CardDAV), but ratatoskr's IMAP factory finishes at SMTP
submission and never calls `ImapAccountConfig::with_carddav`
(`crates/service/src/bifrost/factory.rs:335-339`). So a ratatoskr IMAP
account is contact-UNSUPPORTED, and deriving `source` from the account's
MAIL provider kind would yield `"imap"`, not `"carddav"`. B8 must pin,
before the CardDAV read/write cutover:
- **Factory composition.** Extend the IMAP factory to call
  `.with_carddav(..)` when the account carries CardDAV configuration
  (discovery URL / credentials), so the composed account advertises the
  contact `pim_methods`. An IMAP account WITHOUT CardDAV config stays
  contact-unsupported and `run_contact_pull` no-ops on it (the
  `Unsupported` -> empty-report path, § 5.3 step 1).
- **Credential / configuration rule.** Pin where the CardDAV endpoint +
  credentials come from (reuse the IMAP OAuth/password source, or a
  separate DAV discovery), and the account-settings shape that turns it on.
- **`source` from PROVENANCE, not mail kind.** The `source` tag for a
  CardDAV-composed account is `"carddav"`, derived from the CONTACT
  backend's provenance (`ContactCard.provenance.provider` /
  `AddressBook.provenance`), NOT the account's mail `ProtocolKind`. This
  is a concrete reason the (provider, corpus) derivation of § 5.1 reads
  provenance off the contact surface, not the account's mail kind.
- **Test endpoint.** `saehrimnir` grows a CardDAV contact endpoint (or the
  harness composes a DAV mock) so the § 7 `carddav_pull.lua` /
  `writeback_carddav.lua` gates can exercise a real attached backend, not
  a skipped `Unsupported`.

### 5.6 Deletion + collapse (clause 9 blast radius, executed in B8g)

Delete, after the replacements above are green:

- `crates/gmail/src/contacts/` (mod, google_contacts, other_contacts) -
  ~800 LOC; `crates/graph/src/contact_sync.rs` (443);
  `crates/jmap/src/contacts_sync.rs` (550, minus any write-back helper
  already re-homed - `jmap_contacts_push_update` is deleted with § 5.4);
  `crates/core/src/carddav/` + `core/src/lib.rs:7 pub mod carddav`.
- `crates/graph/src/group_sync.rs` - STAYS (B8-SQ2 pinned to disposition
  (b)). B8g does NOT delete it and it is NOT in the § 5.6 grep audit below;
  its deletion is owed to the named `B8-groups` follow-up item.
- The provider fetch fns in `crates/core/src/contacts/gal.rs`
  (`fetch_graph_gal`, `fetch_google_gal`); keep `gal_cache_age` and the
  `GalEntry` re-export.
- Inline write-back HTTP arms + the local `build_google_contact_update_body`
  / `build_graph_contact_update_body` helpers in
  `crates/service/src/actions/contacts.rs`.
- `sync::state` contact helpers: `save/load_jmap_sync_state` for
  `"ContactCard"`, `save/load_graph_contact_delta_token(s)`,
  `save/load/delete_google_other_contacts_sync_token`, and
  `increment_gmail_sync_cycle` (subsumed by the § 5.3 pull-cycle counter).
- Delta-token / cursor tables: `graph_contact_delta_tokens` (the cursor is
  now bifrost's `Page::next_cursor`, gone unconditionally). Same for the
  `sync::state` contact cursor helpers above.
- **The provider MAP tables are NOT "no map needed" - they are a
  remote-claim ledger and must be UNIFIED, not dropped (R2-B1, blocking).**
  `contacts.email` is globally `UNIQUE`
  (`crates/db/src/db/schema/03_contacts.sql:5`), so a single materialized
  `contacts` row can hold exactly ONE `server_id`. But the same
  deduplicated email can be claimed by MULTIPLE (account, provider, remote
  id) tuples - which is exactly why the current delete path checks
  `google_contact_map` AND `graph_contact_map` for remaining claims before
  deleting the materialized row (`google_contacts.rs:304-326`). Collapsing
  all claims onto `contacts.server_id` + `source` would lose that guard and
  let one provider's prune delete a contact another provider still claims.
  So B8 REPLACES `graph_contact_map` / `google_contact_map` /
  `google_other_contact_map` / `carddav_contact_map` with ONE unified
  remote-claim table, e.g. `contact_claims(account_id, source, server_id,
  email, address_book_id, corpus)` (corpus distinguishes `google` main
  from `google_other`), with `contacts` remaining the deduplicated
  materialization. The § 5.3 snapshot reconcile retires CLAIMS by
  `(account_id, source, server_id)`; a `contacts` row is deleted only when
  no claim references its `email`. This preserves today's cross-provider
  dedup exactly, uniformly, and is the piece that makes the pull correct
  rather than merely compiling. Because the claim ledger is populated from
  the fetched set on each pull (not read off `contacts.server_id`), the
  first post-cut pull does NOT depend on legacy `contacts` rows carrying a
  populated `server_id` (R1 flagged that Graph/Google delete helpers key on
  EMAIL today, not `server_id`): the pull upserts claims fresh, then
  reconciles. Dev DBs wipe-and-reseed (no in-the-wild rows to backfill), so
  no map-to-`contact_claims` data migration is owed; the v100 schema edit
  just swaps the table shapes.
- **Migration lands in v100, NOT v101 (R2-B10, R1).** The prior draft's
  "new v101 forward migration or dev-seed edit" hedge is wrong: the
  migration file's PRE-RELEASE POLICY is explicit - until a release ships,
  schema changes edit v100 in place (the `schema/*.sql` files) and MUST
  NOT add a v101 entry (`crates/db/src/db/migrations.rs:66-71`). Dev DBs
  are wiped and re-seeded on each launch, so there are no in-the-wild DBs
  to migrate. So: edit `schema/03_contacts.sql` in place - drop the four
  map tables + `graph_contact_delta_tokens`, add `contact_claims` - and
  update the schema-file comment in `migrations.rs:24-27`. No v101, no
  deferral (this closes the § 9 "schema migration" open item, which was a
  soft deferral dressed as resolved).
- The external `jmap-client` contact dependency stays until B15 per the
  migration doc; B8 removes ratatoskr's USE of the JMAP contact client
  but the `bifrost-jmap` contact_card types the write-back stub touched
  are on the engine side now.

B8 is the mechanical dependency-and-module audit's contacts slice
(`docs/bifrost-migration.md` § 7 B15): after B8g, `grep` for
`contacts_sync` / `contact_sync` / `core::carddav` / `fetch_graph_gal`
outside the deleted files must be empty. `group_sync` is deliberately
EXCLUDED from this audit - it stays through B8 (B8-SQ2 disposition (b))
and is audited by the `B8-groups` follow-up item.

## 6. Stopping rule (clause 9)

B8 stops at the provider boundary. In scope: provider contact READ,
WRITE, DELETE, the Google otherContacts corpus, and the org-directory
(GAL) fetch - everything that speaks a provider protocol. Out of scope and
explicitly UNTOUCHED: the local `contacts` / `contact_groups` /
`contact_photo_cache` / `gal_cache` / `seen_addresses` table SCHEMAS and
their FTS/triggers; local contact search + autocomplete
(`core/contacts/search.rs`, a local FTS query); contact import; dedup; the
group editor's local-only IPC (`contacts.group_save` / `group_delete` /
local `contact_save`); the `seen_addresses` MAIL `seen_ingest` writer
(owned by mail sync, B3). IN scope though (correcting an earlier
over-statement, R1-F1): the Google `otherContacts` WRITER into
`seen_addresses` (`source='google_other'`) is a provider-contact pull leg
and is re-homed onto the bifrost surface (B8-SQ1), NOT left untouched -
only the table schema and the mail-sourced ingest stay put. Also untouched:
the app UI (settings contacts/people/groups tabs - they call the unchanged
IPC surface). The `contacts.*` IPC wire contract
(`service-api`) is the firewall and does not change shape. Provider-side
contact SEARCH / autocomplete (`Account::contact_search` /
`contact_autocomplete`) are NOT consumed - ratatoskr searches its local
mirror, the B10 "local search stays app-level" precedent. The Exchange-
groups blast radius is bounded by B8-SQ2's disposition (§ 3).

## 7. Verification per brick (clause 5)

Contacts are a Service IO-boundary + provider-sync concern, so the norm
here is harness + sync-bench gates, not unit tests alone
(`reference/technical-implementation-spec.md` clause 5). Per gate the
EXACT command is pinned - no `...` placeholders (R2-B10): script paths are
under `crates/app/tests/sync-harness/contacts/`. Where no instrument
exists, building it is a gated brick laid first.

**Survey of the existing contact harness scripts first (R2-B10).** Before
laying new scripts, the harness author surveys the contact scripts that
already exist (e.g. `jmap-contacts-initial.lua`,
`graph-contacts-incremental.lua`, `people-contacts-writeback-delete.lua`
and siblings under the sync-harness tree) and assigns each to a brick:
either it is retargeted at the bifrost pull/write path and kept as the
pre/post golden source, or it is superseded by a new script and deleted in
the same landing. No existing contact script is silently orphaned.

- **Mapping (unit, B8a).** `crates/service/src/bifrost/contacts/map.rs`
  gets deterministic tests pinning `contact_write_rows`: no-email -> `[]`
  (empty vec, not `None`); email lowercasing; display-name fallback to
  email; first-phone / first-org / notes extraction; the exact local-id
  scheme `{source}-{account}-{email}`; `server_id == native_id`; and BOTH
  projections - `PackSecondEmail` (second email -> `email2`, one row) vs
  `FanOutPerEmail` (N emails -> N rows, `email2` empty). Command:
  `brokkr test -p service contact_write_rows`.
- **Reconcile guard (unit, B8a).** The `failed_ids`-aware retire (a
  `server_id` in `failed_ids` is NOT retired), the bounded transient-empty
  guard (clean empty enumeration DOES delete-all; a failed/partial page
  does not), the source-scoped retire, and the "delete `contacts` row only
  when no `contact_claims` row references its email" cross-provider dedup
  guard: `brokkr test -p service contact_pull_reconcile`.
- **Per-provider read behavioral (sync-harness, B8a-d).** The
  authoritative gate - a compile-only replacement MUST fail it. For each
  provider, a `saehrimnir` mock serving that provider's contact endpoints,
  a sync-harness script that pulls, asserts the `contacts` rows
  (count, email, display_name, source, server_id, AND the enriched
  phone/company/notes/email2 columns per § 5.2) match the pre-cut golden,
  then mutates the mock (add / update / delete a contact) and asserts the
  second pull upserts + retires correctly. If `saehrimnir` has no contact /
  directory endpoints, ADDING them is the first, gated brick
  (build-the-instrument, clause 5). Commands:
  `brokkr service-test crates/app/tests/sync-harness/contacts/jmap_pull.lua`,
  `brokkr service-test crates/app/tests/sync-harness/contacts/graph_pull.lua`,
  `brokkr service-test crates/app/tests/sync-harness/contacts/google_pull.lua`,
  `brokkr service-test crates/app/tests/sync-harness/contacts/carddav_pull.lua`.
  Suite: `brokkr service-suite --filter contacts`.
- **Delete-all vs transient-empty (sync-harness, B8a).** Distinct from the
  guard unit test: drive a mock that (1) legitimately empties an address
  book -> asserts the local rows for that source ARE pruned; (2) returns a
  transient failure / non-empty `failed_ids` over the whole prior snapshot
  -> asserts NO prune. Command:
  `brokkr service-test crates/app/tests/sync-harness/contacts/reconcile_deleteall.lua`.
- **otherContacts corpus (sync-harness, B8d).** Google otherContacts pull
  writes `seen_addresses` with `source='google_other'` (NOT `contacts`),
  and a locally-observed `seen_addresses` row survives the pull
  (`local_observed` preserved). Command:
  `brokkr service-test crates/app/tests/sync-harness/contacts/google_other_contacts.lua`.
- **Cross-provider dedup (sync-harness, B8b/d).** The same email claimed by
  a Google contact and a Graph contact yields ONE `contacts` row with two
  `contact_claims`; deleting the Google contact retires its claim but
  leaves the `contacts` row (Graph still claims it); deleting both removes
  the row. Command:
  `brokkr service-test crates/app/tests/sync-harness/contacts/dedup_claims.lua`.
- **Identity-equivalence across the cut (sync-harness, B8b Graph).** Assert
  the Graph fan-out row count and identity columns are identical pre/post
  cut (the § 5.3 (a) decision), with enriched columns asserted separately
  per § 5.2. Command:
  `brokkr service-test crates/app/tests/sync-harness/contacts/graph_contact_fanout.lua`.
- **Write-back (sync-harness, B8e).** For each of jmap/google/graph/carddav:
  drive `contacts.contact_save_with_writeback` and `contacts.contact_delete`
  against the mock, assert the mock received the expected update/delete AND
  the wire `WritebackOutcome` is `Success`; a provider 5xx yields
  `LocalOnly` (save) / `Failed`-before-local-delete (delete). CardDAV must
  now return `Success`, not the old `LocalOnly` stub (this requires the
  § 5.5 factory composition landed - a CardDAV account with no DAV config
  stays `Unsupported` and is a separate asserted case). Commands:
  `brokkr service-test crates/app/tests/sync-harness/contacts/writeback_jmap.lua`
  (and `_google` / `_graph` / `_carddav`).
- **GAL (sync-harness, B8f).** Graph `/users` + Google
  `listDirectoryPeople` mocks -> `directory_search` -> `gal_cache` rows
  match the pre-cut `GalEntry` golden; JMAP/IMAP return `Unsupported` /
  `Ok(None)` and write no rows; Google 403 -> empty (personal account).
  Command:
  `brokkr service-test crates/app/tests/sync-harness/contacts/gal_directory.lua`.
- **Request-budget / cadence (sync-bench, B8a-d).** A `sync-bench` gate on
  the resident aux pass asserting the contact-pull provider-request count
  per cadence window is within the recorded `brokkr.toml` baseline, PER
  SOURCE (jmap:1 / graph:5 / google:20 / carddav:5, § 5.3) - so both a
  naive "pull the whole book every 300 s" regression AND a "degrade JMAP to
  the Gmail cadence" regression are caught. Also asserts the account-wide
  `contacts_list(None)` route (no per-group Google amplification, R2-B7).
  Command:
  `brokkr sync-bench contacts_cadence --gate contact_pull_requests`.
- **Universal green-tree gate (every landing).** `brokkr check`.

The sync-harness + sync-bench gates are mandatory
(`docs/bifrost-migration.md` § 10): a green `brokkr check` proves the new
code compiles, not that real provider contact sync still behaves.

## 8. Stance (clause: structural over micro)

This is a full rewrite of ratatoskr's provider contact layer, not a
local tweak. ~2.2k LOC of hand-rolled provider contact clients plus the
GAL fetch and the dormant CardDAV parser are deleted and replaced by
thin consumers of the bifrost `Account` contact surface. The four
per-provider initial-vs-delta branches collapse to one uniform pull; the
four write-back arms collapse to one `ContactPatch` call; the two GAL
fetchers collapse to one `directory_search`. No env-var scaffolding, no
per-provider routing switch, no dual-write transition - each provider
cuts over behind one new path and its old code is deleted in the same
green landing (clause 6). Old abstractions earn no protection: the
delta-token tables retire, and the four `*_contact_map` tables are folded
into ONE `contact_claims` remote-claim ledger (§ 5.6) - the claim ledger
is kept (it carries the cross-provider dedup guard, not obsolete cursor
state), the per-provider maps are not.

## 9. Open items reconciled into the spec (no deferral holes)

- **Exchange groups (B8-SQ2).** PINNED to disposition (b) in § 3, not left
  to survey: Exchange group sync is carved out of B8 into a named separate
  `B8-groups` governing-plan item; `group_sync.rs` STAYS through B8 and is
  excluded from the § 5.6 grep audit.
- **Google otherContacts (B8-SQ1).** Resolved: otherContacts stay in
  `seen_addresses` (`source='google_other'`), re-homed onto the bifrost
  surface with a corpus discriminator; the pull routes that corpus to the
  `seen_addresses` writer, NOT `contacts` (§ 2.6, § 5.1, § 5.3 corrected).
- **`source` derivation.** Pinned: derived from the (provider kind, corpus)
  pair, NOT provider kind alone (google + google_other); for CardDAV,
  derived from the CONTACT backend provenance, not the account mail kind
  (§ 5.1, § 5.5).
- **Fan-out vs collapse.** Pinned to fan-out (a) via
  `ContactProjection::FanOutPerEmail`, with one `contact_claims` row per
  fanned email; collapse (b) is a named, excluded follow-up.
- **Remote-claim modeling.** Pinned: the four `*_contact_map` tables are
  replaced by ONE unified `contact_claims` ledger (`contacts` stays the
  deduplicated materialization); collapsing onto `contacts.server_id` was
  rejected because `contacts.email` is UNIQUE and cross-provider dedup
  needs multi-claim tracking (§ 5.6, R2-B1).
- **Snapshot reconcile safety.** Pinned: retire keyed on
  `contact_claims`, `failed_ids` excluded from the "absent -> retire" set,
  transient-empty guard bounded so a clean empty enumeration DOES delete-all
  (§ 5.3, R2-B3); carddav `failed_ids` fixed in B8-SQ3.
- **Cadence.** Pinned per-source (jmap:1 / graph:5 / google:20 /
  carddav:5), NOT a uniform N, to preserve each provider's current
  freshness (§ 5.3, R2-B6). Uniform cadence is a named, excluded follow-up.
- **CardDAV composition.** Pinned: the IMAP factory must call
  `.with_carddav` on DAV-configured accounts, else the account is
  contact-unsupported and the pull no-ops (§ 5.5, R2-B4).
- **Provider-side contact search.** Explicitly NOT consumed (§ 6); local
  FTS stays, matching B10's local-search precedent.
- **`contact_create` write path.** Passthrough exists (B8-SQ0) but stays
  unused - import is local-only today, and B8 is feature-preserving, so
  wiring a new "sync imported contacts to provider" affordance is out of
  scope (named, excluded). Removed from the § 5.5 CardDAV write-path list
  where an earlier draft contradictorily included it.
- **Schema migration for dropped/added tables.** RESOLVED, not hedged: the
  migration file's pre-release policy forbids a v101 entry - edit v100 in
  place (`schema/03_contacts.sql`), drop the maps + delta-token table, add
  `contact_claims` (§ 5.6, `crates/db/src/db/migrations.rs:66-71`). Dev DBs
  wipe-and-reseed, so no in-the-wild migration is owed.
