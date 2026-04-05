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

- `account/delete.rs`: Move cascade-deletion queries to `db`. Core keeps the orchestration (deciding what to delete and in what order).
- `account/info.rs`: Move account reads to `db`. Core keeps any domain interpretation of the results.
- `account/provider_init.rs`: Move INSERT/UPDATE queries to `db`. Core keeps account creation workflow (color selection, duplicate checking, token management).

### Phase C: Extract SQL from contacts

- `contacts/search.rs`: Already moved to core (from app). Next move is into `db` — the FTS5+LIKE waterfall is pure query mechanics.
- `contacts/dedup.rs`: Move duplicate-finding query and merge upsert to `db`. Core keeps merge-strategy decisions.
- `contacts/gal.rs`: Move cache refresh to `db`.
- `contacts/save.rs`: Move conditional update to `db`.
- `contacts/seen_addresses.rs`: Move promotion and stats queries to `db`.
- `contacts/sync_*.rs`: Move enrichment queries to `db`.

### Phase D: Extract SQL from email operations and other domains

- `email_actions/mod.rs`: Move label add/remove to `db`.
- `mdn.rs`: Move hierarchical policy lookup to `db`. Core keeps the fallback-chain logic.
- `send_identity.rs`: Move identity queries to `db`. Core keeps priority selection logic.
- `auto_responses.rs`, `bimi.rs`, `command_palette_queries.rs`, `search_pipeline.rs`: Move queries to `db`. Core keeps domain interpretation.

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
