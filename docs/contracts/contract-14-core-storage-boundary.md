# Contract #14: Core Storage Boundary

## Problem

`core` (the `rtsk` crate) is supposed to express domain rules and business workflows. Instead, it is the largest SQLite host in the workspace. 47 files import `rusqlite`. The `db/queries_extra/` directory alone is 8,200 lines of raw SQL across 19 modules. Another 25 feature modules outside `db/` — account management, contacts, calendars, email actions, search, MDN, BIMI, auto-responses — contain direct SQL with `&Connection` parameters, row mapping, transaction orchestration, and dynamic parameter building.

The result is that `db` is not the true storage owner. `core` knows table layout, SQL syntax, and SQLite-specific mechanics. Business logic and storage mechanics are interleaved in the same functions. Adding a column, changing an index, or altering conflict resolution requires touching code across dozens of feature modules that should only be expressing domain decisions.

Contract #12 established that `app` should not depend on `rusqlite`. This contract extends the same principle to `core`: domain logic should compose storage APIs, not write SQL. This contract is about SQLite ownership in `core` specifically — it does not attempt to resolve the provider/sync boundary (Contract #13) or absorb domain logic into `db`. The goal is separation of storage mechanics from business rules, not consolidation of all persistence-related concepts into one crate.

## Current State

### Scale

- **47 files** in `crates/core/src/` import `rusqlite`
- **~350 SQL statements** across all files
- **~400 public functions** with database access
- **~10,000 lines** of database code (queries_extra: 8,200 + queries.rs: 893 + pending_ops.rs: 527 + feature modules)

### Where the SQL lives

#### 1. `db/` directory (legitimate storage code in the wrong crate)

These modules are storage modules that happen to live in `core` instead of `db`:

| Module | Lines | SQL statements | Domain |
|---|---|---|---|
| `queries.rs` | 893 | 33 | Thread CRUD, label ops, settings, unread counts |
| `pending_ops.rs` | 527 | 23 | Pending operation queue (enqueue, status, retry, compact) |
| `queries_extra/calendars.rs` | 965 | 46 | Calendar/event/attendee/reminder CRUD |
| `queries_extra/compose.rs` | 923 | 43 | Draft/signature/template/scheduled-send CRUD |
| `queries_extra/scoped_queries.rs` | 720 | 24 | Multi-account thread/folder scope queries |
| `queries_extra/navigation.rs` | 719 | ~16 | Sidebar state, shared mailbox, pinned folders, typeahead |
| `queries_extra/thread_detail.rs` | 763 | 9 | Thread detail view (messages, labels, attachments) |
| `queries_extra/contact_groups.rs` | 490 | 30 | Contact group CRUD and expansion |
| `queries_extra/misc.rs` | 466 | 20 | Heterogeneous: notifications, keybindings, snooze, settings |
| `queries_extra/contacts.rs` | 448 | 20 | Contact settings, save, upsert, stats |
| `queries_extra/bundles.rs` | 417 | 21 | Thread bundle/category assignment |
| `queries_extra/accounts_crud.rs` | 340 | 8 | Account CRUD, color, auth info |
| `queries_extra/tasks.rs` | 340 | 15 | Task CRUD and label assignment |
| `queries_extra/filters_smart.rs` | 303 | 12 | Smart filter rules and actions |
| `queries_extra/ai_state.rs` | 291 | 19 | AI settings and rules |
| `queries_extra/follow_up_quick.rs` | 280 | 17 | Follow-up reminders |
| `queries_extra/allowlists.rs` | 270 | 14 | Allowlists |
| `queries_extra/labels_attachments.rs` | 176 | 8 | Attachment queries, cloud attachments |
| `queries_extra/accounts_messages.rs` | 108 | 5 | Account-scoped message queries |
| `queries_extra/message_queries.rs` | 96 | 3 | Message lookups by ID |
| `queries_extra/thread_ui_state.rs` | 78 | 4 | Thread collapse/expand state |

These are all query-shaped code. They belong in `db`.

#### 2. Feature modules (business logic with embedded SQL)

These are domain/feature modules that should express rules, not write SQL:

**Account management:**
- `account/delete.rs` — multi-table cascade deletion with 500-row batch chunking
- `account/info.rs` — account reads (email, CalDAV credentials, OAuth)
- `account/provider_init.rs` — account creation (21-parameter INSERT), color picking, duplicate detection, reauth token updates

**Contacts:**
- `contacts/search.rs` — unified FTS5+LIKE autocomplete across 4 sources
- `contacts/dedup.rs` — duplicate detection and merge with transactions
- `contacts/gal.rs` — GAL cache refresh (DELETE + bulk INSERT)
- `contacts/save.rs` — conditional field-level contact updates
- `contacts/seen_addresses.rs` — seen-to-contact promotion, stats aggregation
- `contacts/sync_google.rs` — phone/company enrichment via UPDATE+COALESCE
- `contacts/sync_graph.rs` — server_id enrichment via subquery
- `contacts/sync_carddav.rs` — CTag/ETag sync state persistence

**Email operations:**
- `email_actions/mod.rs` — thread label add/remove (INSERT OR IGNORE, DELETE)
- `mdn.rs` — hierarchical read-receipt policy lookup (sender → domain → account → global)
- `send_identity.rs` — send identity selection with priority matching
- `send.rs` — draft status updates
- `scheduled_send.rs` — scheduled email types

**Other domains:**
- `auto_responses.rs` — auto-response upsert
- `bimi.rs` — BIMI cache with domain warming queries against `messages`
- `caldav/sync.rs` — CalDAV sync state
- `cloud_attachments.rs` — cloud attachment tracking
- `command_palette_queries.rs` — label/folder search for command palette
- `contact_photos.rs` — avatar cache
- `search_pipeline.rs` — SQL fallback search with dynamic LIKE patterns

### Pattern analysis

Feature modules use two patterns:

**Pattern A — `&Connection` directly (15 modules):** Functions take `&Connection` as a parameter, write inline SQL, and return domain types. The caller is responsible for obtaining the connection. This is the most tightly coupled pattern.

**Pattern B — `&DbState` wrapped (9 modules):** Functions take `&DbState`, call `db.with_conn(|conn| { ... }).await`, and write SQL inside the closure. Structurally async, but the SQL is still inline in the feature module.

Both patterns mean the feature module owns SQL shape and row mapping.

## Contract

### 1. Core must not depend on `rusqlite` directly

In the target architecture, `core`'s `Cargo.toml` does not list `rusqlite`. Core depends on `db` for all storage access. Core functions take `&DbState` or domain-specific storage handles, never `&Connection` or `&Transaction`.

### 2. `core/db/` modules move into the `db` crate

The entire `queries_extra/` directory, `queries.rs`, `pending_ops.rs`, and storage-specific types (row types, query parameter types, storage state types) are storage-shaped code. They belong in `crates/db`, not in `crates/core`. Core re-exports what it needs from `db` for its public API, but the SQL lives in `db`.

Not every type that currently lives next to SQL must move. The distinction:

- **Storage types** (row structs mapped from `rusqlite::Row`, query parameter structs, storage state types) move to `db`.
- **Domain result types** (types that express business concepts and happen to be returned by queries) may stay in `core` if they are used by domain logic beyond the storage boundary. `db` returns storage rows; `core` may map them to domain types.

### 3. Feature modules express domain logic through `db` APIs

Feature modules in core (`account/`, `contacts/`, `email_actions/`, etc.) must not contain SQL. They call `db` functions that return typed results. The feature module makes domain decisions (which contacts to merge, which policy applies, what priority order to use); `db` handles the query mechanics.

Where a feature module currently interleaves domain logic with SQL (e.g., `contacts/dedup.rs` finds duplicates via GROUP BY and then decides merge strategy), the SQL portion moves to `db` as a query function, and the domain decision stays in core.

### 4. Transaction boundaries are negotiated, not assumed

Many feature modules currently open transactions directly (`conn.unchecked_transaction()`). In the target state, `db` owns transaction scope. If a feature module needs transactional atomicity across multiple storage operations, `db` exposes a transactional API (either a closure-based transaction wrapper, or a multi-step operation that internally manages its own transaction).

Core should never hold a raw `Transaction`.

Disentangling domain orchestration from transaction scope is the main complexity driver for Phases B–D. The hardest modules are those where domain decisions happen mid-transaction — `account/delete.rs` (cascade logic interleaved with batch queries), `contacts/dedup.rs` (merge strategy decided between duplicate-finding and merge-executing queries). These cannot be moved mechanically; they require designing `db` APIs that give core enough control over multi-step operations without exposing transaction handles.

## Migration Shape

### Phase A: Move `queries_extra/` into `db`

This is the largest single move (~8,200 lines). The modules are storage-shaped code, but the move is not purely mechanical:

- Import paths will shift widely across the workspace. Many callers import via `rtsk::db::queries_extra::*`.
- Some modules depend on core-local types and helpers that may need to move with them or be re-exported from `db`.
- Transaction helpers and `DbState`-based wrappers may need reshaping if they reference core-internal state.
- The module organization in `db` may not match the current `queries_extra/` layout.

The migration rule: **move files first, preserve APIs.** Do not redesign APIs during the move. Re-export aggressively from `core` so that existing callers do not break. Prune re-exports in a separate pass once the move is stable.

1. Move all 19 `queries_extra/` modules into `crates/db`
2. Move `queries.rs` and `pending_ops.rs` into `crates/db`
3. Move storage row types and query parameter types. Leave domain result types in `core` if they are used beyond storage.
4. Re-export from `core` via `pub use db::...` to preserve all existing import paths
5. Incrementally remove re-exports in later passes as callers are updated to import from `db` directly

### Phase B: Extract SQL from account management

This phase is the first non-mechanical storage extraction after Phase A. It is intentionally scoped to one coherent domain:

- `account/delete.rs`
- `account/info.rs`
- `account/provider_init.rs`

The goal is not to redesign account management. The goal is to remove SQL from these modules while preserving their current public behavior and keeping domain decisions in `core`.

#### Why accounts first

This slice is a good Phase B target because it contains all three patterns that later phases will need to handle:

- pure reads with post-query interpretation (`account/info.rs`)
- persistence writes with pre-query domain preparation (`account/provider_init.rs`)
- multi-step mutation orchestration where query order matters (`account/delete.rs`)

If this slice is clean, it establishes the pattern for the later contacts and email-operation phases.

#### Module-by-module shape

##### `account/info.rs`

`account/info.rs` should end Phase B as a domain adapter over `db` reads.

One function is already mostly boundary-clean:

- `get_calendar_provider_info()` already delegates account lookup through existing config/account access and only does provider interpretation locally.

Move to `db`:
- account row reads for:
  - basic account info
  - CalDAV settings
  - OAuth credential fields

Keep in `core`:
- decryption (`decrypt_value`, `is_encrypted`)
- interpretation of empty-string / missing-field cases
- provider-specific shaping such as:
  - "CalDAV credentials not configured"
  - Graph/Gmail-only OAuth interpretation
  - username fallback from CalDAV username to account email

The migration rule for `info.rs` is:
- `db` returns typed storage rows containing the raw persisted fields
- `core` converts those rows into `AccountBasicInfo`, `AccountCaldavSettingsInfo`, `AccountOAuthCredentials`, and `CaldavConnectionInfo`

This keeps storage mechanics in `db` while leaving secret handling and domain interpretation in `core`.

##### `account/provider_init.rs`

`account/provider_init.rs` currently mixes three concerns:

- local domain derivation:
  - `derive_account_name`
  - token encryption / decryption helpers
- local account-init policy:
  - duplicate checking
  - deciding which credential fields to persist
- raw SQL writes:
  - Gmail/IMAP/Graph INSERTs
  - finalize-profile UPDATE
  - reauth-token UPDATEs
  - stored-credential lookups for reauth

Phase B splits these concerns as follows.

Move to `db`:
- duplicate/account-exists lookups
- next-account-color query input (`SELECT account_color ...`)
- account insert/update persistence primitives:
  - insert Gmail account row
  - insert IMAP OAuth account row
  - insert Graph account row
  - finalize Graph profile row update
  - Gmail/Graph reauth token updates
  - stored OAuth credential reads used by reauth flows

Keep in `core`:
- `derive_account_name`
- `encrypt_oauth_tokens`
- decrypting stored credentials for reauth
- choosing which persisted write to call for a given provider/auth flow

Important boundary rule:
- color selection policy does **not** move into `db`
- the query that reads used colors moves into `db`
- the palette decision remains in `core`

That means the insert/update persistence functions in `db` no longer derive `account_name` or choose `account_color` internally. The insert parameter structs grow the persisted `account_name` and `account_color` fields, and `core` computes both before calling `db`.

That means `db` exposes something like "list used account colors" or "read account colors in use", while `core` continues to call `label_colors::preset_colors::all_presets()` and pick the next available one.

This avoids pushing label-color policy down into `db`.

##### `account/delete.rs`

`account/delete.rs` is the hardest account module because it is not just a set of queries. It gathers cleanup data, checks for cross-account shared references in batches, deletes the account row, and returns a plan for later async cleanup.

Move to `db`:
- gather message IDs for an account
- gather cached attachment `(local_path, content_hash)` rows
- gather inline-image content hashes
- batch-check cached attachment hashes referenced by other accounts
- batch-check inline hashes referenced by other accounts
- delete the account row

Keep in `core`:
- deciding the overall deletion flow
- constructing `AccountDeletionPlan`
- subsequent non-SQL cleanup of body store, inline-image cache, and search cleanup

Transaction rule for `delete.rs`:
- `core` must not hold a raw `Transaction`
- `db` must expose one operation-specific function that performs the synchronous DB phase atomically:
  - gather deletion data
  - compute shared-reference sets
  - delete the account row
  - return the storage results needed to build `AccountDeletionPlan`

In other words, the current `delete_account_orchestrate(conn, account_id)` remains a valid shape, but its SQL body moves to `db` and `core` calls it through a `DbState`/`db` API rather than a raw `&Connection`.

This is the Phase B precedent for later transactional feature work: operation-specific transactional APIs, not generic transaction handles leaked into `core`.

The existing storage-oriented tests for `account/delete.rs` move with that storage function. They verify storage behavior, not domain orchestration.

#### Storage/API shape

Phase B should prefer new account-focused `db` APIs over reusing unrelated modules opportunistically.

Likely homes:
- account read/write CRUD helpers extend existing account-related modules in `db`
- account deletion storage helpers may live in a dedicated `db` account-deletion module rather than being bolted into a generic query file

The important rule is API ownership, not exact file names:
- `db` owns account-table reads/writes and account-deletion storage operations
- `core` owns account workflow decisions, secret handling, and external cleanup

#### Type ownership

The account types in `account/types.rs` do not all move together.

Stay in `core` as domain result types:
- `AccountBasicInfo`
- `AccountCaldavSettingsInfo`
- `AccountOAuthCredentials`
- `CaldavConnectionInfo`
- `CalendarProviderInfo`
- `AccountDeletionPlan`

Likely move with storage if it simplifies the deletion API:
- `AccountDeletionData`

The rule is the contract-wide one: storage-shaped intermediate data may move down, but domain-facing result types stay in `core`.

#### Migration steps

1. `account/info.rs`
- add typed `db` row reads for the persisted fields it needs
- rewrite `info.rs` to consume those rows and keep decryption/interpretation locally
- remove `rusqlite` from `info.rs`

2. `account/provider_init.rs`
- add `db` functions for duplicate lookup, used-color reads, inserts, finalize-profile updates, reauth-token updates, and stored-credential reads
- rewrite `provider_init.rs` to keep derivation/encryption/decryption logic only
- remove `rusqlite` from `provider_init.rs`

3. `account/delete.rs`
- add one operation-specific `db` orchestration API for the synchronous deletion phase
- rewrite `delete.rs` to call that API and build `AccountDeletionPlan`
- remove `rusqlite` from `delete.rs`

4. verification
- `cargo check -p db`
- `cargo check -p rtsk`
- account creation/reauth flows still compile without API churn
- account deletion tests still pass

#### What Phase B does not do

- it does not remove `rusqlite` from all of `core`
- it does not solve the cycle-blocked Phase A exception modules
- it does not redesign account-management APIs beyond what is needed to remove SQL
- it does not move label-color policy or crypto policy into `db`

### Phase C: Extract SQL from contacts

This phase removes the remaining direct SQL from all 8 modules under `contacts/`. The contacts domain has no core-internal dependency blockers (no crypto, no body_store, no label_colors), so every module can be fully extracted. The amount of work varies: sync_google.rs and sync_graph.rs are already mostly core wrappers with a couple of `DbState` SQL closures, while dedup.rs and sync_carddav.rs are full SQL hosts with transactional complexity.

The contacts modules divide into three difficulty tiers:

- **pure storage** (SQL moves directly to db, no design questions): search.rs, save.rs, seen_addresses.rs, sync_google.rs, sync_graph.rs
- **bulk storage with trivial domain wrapper** (cache management): gal.rs
- **transactional domain+storage mix** (domain decisions inside transactions): dedup.rs, sync_carddav.rs

#### Why contacts second

Contacts shares the same three patterns as accounts (reads, writes, transactional orchestration) but adds two new challenges that Phase B did not face:

- **Business policy expressed at the storage boundary.** sync_carddav.rs encodes source-priority rules as multi-way CASE statements inside INSERT ON CONFLICT. This is business policy that must live in SQL because atomic upsert semantics require it — reading the row first, deciding in Rust, then writing would lose atomicity and add a round-trip. This is the first place where the contract intentionally allows policy-shaped logic to remain in `db`.
- **Cross-provider orphan detection.** sync_carddav.rs checks three provider mapping tables (carddav, google, graph) before deleting a contact. This cascading check couples the write path to knowledge of all provider sync strategies.

If Phase C handles these cleanly, the remaining phases (D and E) will be straightforward — email_actions, mdn, bimi, etc. are all simpler than this.

#### Module-by-module shape

##### `contacts/search.rs`

Pure storage. No domain logic, no transactions, no core-internal deps.

Move to `db`:
- `search_contacts_unified` and all 5 private helpers (FTS5+LIKE waterfall across contacts, gal_cache, seen_addresses, contact_groups)

Keep in `core`:
- `ContactSearchResult` and `ContactSearchKind` are defined in `db::queries_extra::contact_search` (they have no core-internal deps). Core's `contacts/search.rs` re-exports them, preserving the `rtsk::contacts::search::*` import path as the stable API surface.
- Callers should not be updated to import from `db` directly during Phase C — that is downstream import churn for a separate cleanup pass.

This is the simplest module in Phase C. The implementation and private helpers move to db; the public types and module path are preserved through re-exports.

##### `contacts/save.rs`

Pure storage with dual-save semantics. No transactions.

Move to `db`:
- `get_contact_source` (single-row SELECT)
- Two explicit db operations:
  - `save_local_contact_sync` — applies all field updates without setting `display_name_overridden`
  - `save_synced_contact_sync` — applies field updates and sets `display_name_overridden = 1` for display name changes
- The `ContactSource` and `ContactUpdate` types if they are storage-shaped

Keep in `core`:
- `save_local_contact` and `save_synced_contact` as thin wrappers that delegate to the corresponding explicit db operation
- Re-export shim preserving `rtsk::contacts::save::*`

The db layer exposes two explicit operations rather than one function with an `is_local` boolean flag. The product semantics (local contacts save immediately, synced contacts mark display name as user-overridden) are different enough that hiding them behind a flag obscures the policy.

The `display_name_overridden` flag logic stays in the SQL — it is a storage-level invariant ("this column tracks whether the value was user-set"), not a business workflow decision.

##### `contacts/seen_addresses.rs`

Mostly re-exports from the `seen` crate plus two local functions. No transactions.

Move to `db`:
- `promote_seen_to_contact` (check-exists → fetch display_name → INSERT)
- `get_seen_address_stats` (aggregate query)

Keep in `core`:
- The re-exports from `seen` crate (`ingest_from_messages`, `backfill_seen_addresses`, etc.) — these stay because `seen` is a separate crate and core is the facade

The promotion logic (normalize email, check if already a contact, insert with source='user') is a domain decision about when to promote, but the actual SQL is pure storage. Core callers decide *when* to call promote; the db function performs it.

##### `contacts/sync_google.rs`

Thin storage enrichment wrapper. No transactions. Already mostly a core wrapper with a couple of `DbState` SQL closures.

Move to `db`:
- `enrich_google_contacts` (UPDATE with COALESCE across phone/company/account_id/server_id)
- `get_google_contact_server_info` (SELECT from google_contact_map)

Keep in `core`:
- `extract_google_contact_fields` (extracts fields from Google API response — no SQL)
- `build_google_contact_update_body` (builds JSON for PATCH request — no SQL)

The COALESCE strategy ("only enrich if not already set") is a storage-level merge rule, acceptable to keep in the SQL.

##### `contacts/sync_graph.rs`

Thin storage enrichment wrapper. No transactions. Same shape as sync_google.rs.

Move to `db`:
- `enrich_graph_contacts` (UPDATE with subquery correlation against graph_contact_map)
- `get_graph_contact_server_info` (SELECT from graph_contact_map)

Keep in `core`:
- `build_graph_contact_update_body` (builds JSON for PATCH request — no SQL)

##### `contacts/gal.rs`

Bulk cache management with a transactional clear-and-refill, plus cache-age checking and provider-specific HTTP fetch.

Move to `db`:
- `cache_gal_entries` (DELETE + INSERT OR REPLACE in a transaction)
- GAL cache-age read (SELECT from settings — current function name in code is `gal_cache_age`)
- GAL refresh timestamp write (INSERT OR REPLACE into settings — if a new db API name is needed, say so explicitly during implementation)
- Provider-type lookup (SELECT provider FROM accounts) — this read is part of the `refresh_gal_for_account` orchestration entry point. It must become a db helper that the core orchestration function consumes, since core should not do the SELECT itself.

Keep in `core`:
- `refresh_gal_for_account` as domain orchestration: call db for provider lookup → check cache age via db → dispatch to provider-specific HTTP fetch → call db cache function
- `fetch_graph_gal` and `fetch_google_gal` (HTTP client calls, no SQL)
- The 24-hour staleness threshold decision

The transaction in `cache_gal_entries` (DELETE all → INSERT loop) is self-contained storage — no domain decisions happen mid-transaction. It moves to db as-is.

##### `contacts/dedup.rs`

Transactional domain+storage mix. This is one of the two hard modules.

Move to `db`:
- The duplicate-finding query (JOIN contacts with seen_addresses, with limit)
- A per-pair merge operation that performs one merge atomically: read merge contact fields → COALESCE update into keep contact → migrate group memberships → delete merge contact
- The manual `merge_contacts` operation (same SQL as pair merge but for explicit user-initiated merge)

Keep in `core`:
- `find_duplicates` as a domain function that calls the db query and returns `DuplicatePair` results
- `auto_merge_duplicates` as domain orchestration: calls db to find duplicates, then loops and calls db merge per pair, accumulating error/skip counts
- Source-priority decisions (which contact to keep, which to merge) — computed in core before calling db

Transaction rule for dedup:
- `db` exposes a per-pair merge function that performs one pair merge atomically within its own transaction (read fields → update → migrate groups → delete → commit)
- `core` drives the loop: find duplicates, then for each pair, call db's per-pair merge function, accumulating success/skip/error counts
- The current behavior wraps all pairs in one `unchecked_transaction()` and continues on per-pair errors. Phase C replaces this with per-pair transactional db operations while preserving the user-visible partial-success counting behavior. This is the cleaner boundary and matches the current "best effort" spirit — the one-big-transaction wrapper added no real atomicity guarantee since per-pair errors were already tolerated.

##### `contacts/sync_carddav.rs`

The hardest contacts module. Transactional sync with business policy encoded at the storage boundary.

Move to `db`:
- `persist_carddav_contact_full` (INSERT ON CONFLICT with source-priority CASE statements)
- `delete_carddav_contact` (cascading orphan check across 3 provider mapping tables → conditional DELETE)
- `load_ctag` / `save_ctag` (settings table reads/writes)
- `load_stored_etags` (SELECT from carddav_contact_map)

Keep in `core`:
- `sync_carddav_contacts_full` as domain orchestration: CTag staleness check → HTTP fetch → ETag comparison → vCard parsing → call db persist/delete functions
- The change-detection logic (which URIs are new, changed, or deleted) is purely computational and stays in core

Transaction rule for sync_carddav:
- `db` exposes an operation-specific sync-persist function that takes parsed contact data and deleted URIs, and performs the full persist+delete pass atomically within a single transaction
- The current approach (core opens transaction, loops, commits) must not survive — core should not hold a raw Transaction

The source-priority CASE logic in the ON CONFLICT clause:
- This stays in the SQL. It is business policy expressed at the storage boundary because atomic upsert semantics require it. Moving it out of SQL would mean reading the existing row first, applying the rule in Rust, then writing — which loses atomicity and adds a round-trip. This is intentional: db is allowed to encode policy-shaped logic when the alternative would break storage invariants.

The cascading delete check (3-way provider map lookup):
- This also stays in db. It is storage-level referential integrity ("only delete a contact if no provider still claims it"). The check queries `carddav_contact_map`, `google_contact_map`, and `graph_contact_map` — all storage tables.

#### Migration steps

1. `contacts/search.rs`
   - Move `search_contacts_unified` and helpers to db (likely a new `db::queries_extra::contact_search` module, or extend existing `contacts.rs`)
   - `ContactSearchResult` and `ContactSearchKind` live in db; core re-exports via shim
   - Keep `contacts/search.rs` as a re-export shim preserving `rtsk::contacts::search::*`

2. `contacts/save.rs`, `contacts/seen_addresses.rs`
   - Add two explicit db save operations (local and synced) plus contact source lookup, promotion, and stats queries
   - Rewrite core modules to delegate to db
   - Remove rusqlite from both

3. `contacts/sync_google.rs`, `contacts/sync_graph.rs`
   - Move enrichment queries and server-info lookups to db
   - Keep HTTP/JSON helpers in core
   - Remove rusqlite from both

4. `contacts/gal.rs`
   - Move cache CRUD, settings queries, and provider-type lookup to db
   - Keep HTTP fetch and cache-age orchestration in core
   - Remove rusqlite from gal.rs

5. `contacts/dedup.rs`
   - Add db per-pair merge operation (atomically transactional per pair)
   - Add db duplicate-finding query
   - Rewrite core to orchestrate find → merge loop with per-pair db calls, preserving partial-success counting
   - Remove rusqlite from dedup.rs

6. `contacts/sync_carddav.rs`
   - Add db sync-persist function (batch persist+delete in one transaction, with source-priority and cascading-delete logic in SQL)
   - Add db ctag/etag helpers
   - Rewrite core to orchestrate fetch → parse → call db persist/delete
   - Remove rusqlite from sync_carddav.rs

7. Verification
   - `cargo check -p db`
   - `cargo check -p rtsk`
   - `cargo check --workspace` (required final gate)
   - Contact autocomplete still resolves across all 4 sources
   - Contact save (local and synced) still works
   - GAL cache refresh compiles
   - CardDAV sync compiles (cascading delete logic preserved)

#### What Phase C does not do

- It does not redesign the contact sync pipeline or change sync behavior
- It does not unify the 3 provider mapping table schemas (google_contact_map, graph_contact_map, carddav_contact_map) — that is a Contract #13 concern
- It does not change the source-priority rules, only moves them to their correct crate
- It does not address the `seen` crate's own SQL (that crate has its own `rusqlite` dependency, which is a separate Contract #12 question)
- It does not update downstream callers to import from `db` directly — re-export shims preserve existing import paths during Phase C

### Phase D: Extract SQL from email operations and other domains

This phase covers the remaining feature modules in core that contain direct SQL. None have core-internal dependency blockers. The modules split into two groups by signal value.

#### Group 1: Email operation modules (pattern-setting)

##### `email_actions/mod.rs`

Trivial. Three functions (`remove_label`, `insert_label`, `remove_inbox_label`) that do DELETE/INSERT OR IGNORE on `thread_labels`. Pure storage, no domain logic, no transactions. All take `&Connection`.

Move to `db`: all three functions.
Keep in `core`: nothing — module becomes a re-export shim or is emptied.

##### `mdn.rs`

Mixes SQL (policy lookup, MDN-sent flag read/write on `messages`, `read_receipt_policy`) with non-SQL (MDN MIME message building, IMAP/JMAP flag-setting protocol calls). All SQL takes `&Connection`.

Move to `db`:
- `resolve_read_receipt_policy` — hierarchical lookup: sender → domain → account → global default
- `query_policy` — single-row SELECT from `read_receipt_policy`
- `is_mdn_already_sent` — SELECT `mdn_sent` from `messages`
- `mark_mdn_sent_local` — UPDATE `mdn_sent = 1` on `messages`
- `is_mdn_requested_graph` — SELECT `mdn_requested` from `messages`

Keep in `core`:
- `build_mdn_message` — MIME construction, no SQL
- `mark_mdn_sent_jmap` — JMAP protocol call
- `mark_mdn_sent_imap` / `is_mdn_sent_imap` — IMAP protocol calls

The fallback-chain logic in `resolve_read_receipt_policy` (sender → domain → account → global) is query-shaped, not domain-shaped. Each level is a separate query with the same structure. It stays in db as a single function.

##### `send_identity.rs`

Pure storage reads plus Rust-side selection logic. All take `&Connection`.

Move to `db`:
- `get_send_identities` — SELECT from `send_identities` ordered by `is_primary`
- `get_all_send_identity_emails` — SELECT DISTINCT email

Keep in `core`:
- `select_from_address` — priority-based selection (mailbox match → reply_to → primary). This reads identities via the db function, then applies domain logic to choose the best match. The selection algorithm is domain behavior; the query is storage.
- `SendIdentity` struct and `FromSelectionContext` — these are domain types used by the selection logic. They stay in core if they have no storage coupling, or move to db if they are pure data structs returned by the query. Since `SendIdentity` maps 1:1 to the `send_identities` row, it can live in db and be re-exported.

#### Group 2: Standalone query hosts

##### `auto_responses.rs`

Mixes SQL (SELECT/INSERT ON CONFLICT on `auto_responses`) with provider-specific HTTP fetch/push (Graph, Gmail, JMAP).

Move to `db`:
- `db_get_auto_response` — SELECT from `auto_responses`
- `db_upsert_auto_response` — INSERT ON CONFLICT
- `any_auto_response_active` — SELECT EXISTS

Keep in `core`:
- `fetch_graph_auto_response` / `push_graph_auto_response` — Graph HTTP
- `fetch_gmail_auto_response` / `push_gmail_auto_response` — Gmail HTTP
- `fetch_jmap_auto_response` / `push_jmap_auto_response` — JMAP protocol
- `normalize_dotnet_datetime` — date parsing utility
- `AutoResponseConfig` — domain type (may move to db if it's a pure row struct)

##### `bimi.rs`

Mixes SQL (cache read/write on `bimi_cache`, domain-warming query on `messages`) with DNS lookup, SVG fetch/validation, and rasterization.

Move to `db`:
- `get_bimi_cache` — SELECT from `bimi_cache` with expiry check
- `upsert_bimi_cache` — INSERT ON CONFLICT
- `domains_to_warm` — SELECT DISTINCT domains from `messages` (complex SUBSTR/INSTR query)

Keep in `core`:
- `lookup_bimi` — orchestration (check cache → DNS → fetch SVG → validate → rasterize → cache)
- `cache_negative` — calls upsert_bimi_cache with `has_bimi = false`
- `warm_bimi_cache` — concurrent warming loop
- All DNS, HTTP, SVG validation, rasterization functions
- `BimiCacheEntry`, `BimiRecord` — domain types

##### `command_palette_queries.rs`

Pure storage. All reads, no domain logic beyond Rust-side formatting. All take `&Connection`.

Move to `db`: all 5 query functions:
- `get_user_folders_for_palette`
- `get_user_labels_for_palette`
- `get_thread_labels_for_palette`
- `get_all_labels_cross_account`
- `is_folder_based_provider`

Keep in `core`: nothing — module becomes a re-export shim.

Note: these functions return `cmdk::OptionItem`. The `cmdk` crate must be accessible from `db` for this to work. If `cmdk` depends on `db` (creating a cycle), the functions stay in core or the `OptionItem` construction moves to a mapping layer. Check the dependency graph before moving.

##### `search_pipeline.rs`

The hardest module in group 2. One direct SQL query (`search_sql_fallback`) mixed with multi-backend routing (SQL, Tantivy, combined).

Move to `db`:
- `search_sql_fallback` — SELECT threads with LIKE pattern, scope clause
- `search_sql_only` — routes to `smart_folder::query_threads` which already lives in a separate crate

Keep in `core`:
- `search` — top-level routing (parse query → choose backend → combine results)
- `search_tantivy_only` — Tantivy-only path
- `search_combined` — intersection + enrichment
- All result mapping functions (`db_thread_to_unified`, `enrich_from_sql`, etc.)

The `search_sql_fallback` function dynamically builds a scope clause (`AND t.account_id = ?` or `AND t.account_id IN (?...)`) and uses `params_from_iter`. This dynamic SQL is storage-shaped. It moves to db and takes `AccountScope` as a parameter.

#### Migration steps

1. `email_actions/mod.rs` — move 3 label-mutation functions to db
2. `send_identity.rs` — move identity queries to db, keep selection logic in core
3. `mdn.rs` — move policy lookup and MDN state queries to db, keep MIME/protocol in core
4. `auto_responses.rs` — move get/upsert/exists queries to db, keep HTTP in core
5. `bimi.rs` — move cache CRUD and domain-warming query to db, keep DNS/SVG/raster in core
6. `command_palette_queries.rs` — move all queries to db (check cmdk dep first)
7. `search_pipeline.rs` — move SQL fallback to db, keep routing in core

Verification:
- `cargo check -p db`, `cargo check -p rtsk`, `cargo check --workspace` after each group
- Confirm rusqlite removed from each touched module

### Phase E: Remove `rusqlite` from core's `Cargo.toml`

Once all SQL has moved to `db`, remove `rusqlite` from core's dependencies. This is the enforcement gate: if it compiles without `rusqlite`, the boundary holds.

## What This Eliminates

- Business logic modules that must change when table schemas change
- SQL shape owned by two crates (`core` and `db`) for the same tables
- Feature modules that cannot be tested without a SQLite connection
- `&Connection` appearing in function signatures that should be domain-typed
- The current state where "moving a query to `db`" requires understanding which of 47 files currently owns it

## Relationship to Other Contracts

- **Contract #12** (SQLite Boundaries): Established the principle; removed `rusqlite` from `app`. This contract applies the same principle to `core`.
- **Contract #13** (Provider DAL): Defines the provider write path through `db`. Contract #13 depends on this contract: providers need `db` to own a complete storage API surface before they can route writes through it.
- This contract is one part of the broader convergence toward `db`, `stores`, and `dev-seed` as the only `rusqlite` owners. It does not resolve the provider/sync boundary — that work remains in Contract #13 and will build on the `db` APIs that this contract creates.

## Migration Rules

These rules apply during the transition period to prevent regression while the boundary is being established:

1. **Phase A is move-first, preserve-APIs.** Do not redesign storage APIs while relocating them. Re-export aggressively from `core` to avoid import churn during the move. Prune re-exports in a separate later pass.
2. **No new `&Connection` / `&Transaction` parameters in core feature modules.** New storage needs must go through `db` APIs from the start.
3. **Temporary `&DbState`-wrapped SQL in core feature modules is acceptable during transition.** The target is `db` APIs, but existing `DbState`-closure patterns need not be rewritten before the `db` API they would call exists.
4. **`core` should continue re-exporting `DbState` from `db`.** `DbState` already lives in `db`. The question is not where it lives but whether `core` keeps re-exporting it. During migration: yes. Long-term: callers should import from `db` directly.

## Open Questions

1. Should `queries_extra/` modules keep their current file organization when moved to `db`, or should they be reorganized by domain (e.g., `db::contacts`, `db::calendars`, `db::compose`)?
2. How should `search_pipeline.rs` be handled? Its SQL is interleaved with Tantivy integration — the Tantivy routing logic is domain behavior, but the SQL fallback path is storage.
3. For transactional feature operations (account deletion, contact merge), should `db` expose operation-specific transaction functions, or a generic transaction wrapper?
