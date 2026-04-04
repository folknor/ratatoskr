# Contract #12: SQLite Boundaries

## Problem

`rusqlite` currently leaks across much more of the workspace than the architecture wants to allow. If `app` is presentation-only, `rtsk` is the business-logic facade, and storage crates own persistence details, then low-level SQLite coupling should be concentrated near storage boundaries rather than spread across feature and domain code. Broad `rusqlite` exposure makes it too easy for crates to bypass intended abstractions and encode persistence behavior directly where higher-level logic should live.

The problem is not simply dependency hygiene for its own sake. Every additional crate that depends directly on `rusqlite` weakens the architectural boundary between storage mechanics and domain behavior. It becomes harder to see which crate owns query shape, transaction scope, row mapping, migration assumptions, and database invariants. That in turn makes refactors riskier and encourages feature work to grow around existing SQL call sites instead of around stable contracts.

This contract defines which crates are allowed to depend on `rusqlite` directly, which crates must instead depend on higher-level storage APIs, and what migration path will push existing direct usage downward.

## Current Leakage

As of this contract draft, direct `rusqlite` dependencies exist in at least these crates:

- `app`
- `ai`
- `calendar`
- `common`
- `core`
- `db`
- `dev-seed`
- `gmail`
- `graph`
- `imap`
- `jmap`
- `seen`
- `smart-folder`
- `stores`
- `sync`

This is too broad. In practice, those usages fall into a few distinct buckets:

- true storage ownership
  - `db`
  - `stores`
  - `dev-seed`
- query/helper crates that currently embed SQL directly
  - `core`
  - `smart-folder`
  - `seen`
  - `calendar`
  - `ai`
- provider and sync crates that currently write rows directly
  - `gmail`
  - `graph`
  - `imap`
  - `jmap`
  - `sync`
- presentation-layer leakage
  - `app`
- shared-type / error leakage
  - `common`

The architectural problem is not that all of these crates are equally wrong. It is that the boundary is currently undefined, so there is no structural distinction between legitimate storage ownership and convenience-driven leakage.

## Contract

### 1. Direct SQLite ownership is narrow

Only these crates should own direct `rusqlite` access in the long-term architecture:

- `crates/db`
- `crates/stores`
- `crates/dev-seed`

Their roles are:

- `db`
  - main application database
  - migrations
  - canonical query/update entry points
  - transaction helpers
  - row mapping primitives
- `stores`
  - bodies / inline images / attachment cache / similar storage subsystems
  - storage-specific schemas and read/write helpers
- `dev-seed`
  - development-only database population
  - explicit exception because it exists to create SQLite state directly

No other crate should be allowed to grow new direct `rusqlite` usage.

### 2. `app` must not depend on `rusqlite`

`app` is presentation-only. It may own view models, UI-local state, and dispatch logic, but it must not own:

- raw SQLite connections
- SQL text
- `rusqlite::Connection` / `Transaction`
- row-mapping functions tied to `rusqlite::Row`

When `app` needs data or persistence, it must go through:

- `db` APIs directly, if the operation is UI-local persistence and already intentionally app-owned
- or `rtsk` / `core` services, when the operation is domain behavior rather than local UI state

The current `app::db` layer is transitional. It may remain as a facade module for now, but the implementation underneath should stop requiring direct `rusqlite` ownership in `app`.

### 3. `core` / `rtsk` must not be a second SQLite host

`core` is allowed to depend on `db` and to define business operations over stored data, but it should not itself become another general-purpose SQL/query host. In the target architecture:

- `core` defines domain operations, contracts, and orchestration
- `db` owns SQL shape, row mapping, and transaction mechanics
- `core` may request database work through stable APIs, but should not scatter `rusqlite` throughout feature modules

Narrow exceptions may exist temporarily during migration, but the direction is explicit: SQLite mechanics move downward.

### 4. Provider and sync crates must not write arbitrary application rows directly

Provider crates and `sync` should operate on provider protocols, sync normalization, and persistence contracts. They should not independently own broad slices of the application SQLite schema.

In the target model:

- provider crates translate provider payloads into typed sync/update inputs
- `sync` orchestrates sync flows and batching
- `db` owns the concrete SQL writes and transaction boundaries for the main database

Provider-local cache stores are a separate question. If a crate truly owns a provider-specific local cache with its own schema, that can be treated as storage ownership, but that should be explicit and narrow rather than accidental reuse of the main application DB boundary.

### 5. Shared crates must not leak `rusqlite` types in public contracts

Crates like `common` and `types` must not expose `rusqlite` in their public surface area. In particular:

- no public traits requiring `rusqlite::Row`
- no public error types that force downstream crates to depend on `rusqlite`
- no shared contract types parameterized over `Connection` / `Transaction`

If SQLite-specific conversion exists, it belongs at the storage layer.

## Allowed Temporary Exceptions

This contract does not require one-step purity. During migration, the following temporary states are acceptable:

- `core` still contains some SQL-heavy modules, provided the direction is to move that logic into `db`
- provider crates still call SQLite-backed helpers while storage APIs are being extracted
- `app` still carries transitional DB facade code while its direct `rusqlite` dependency is being removed

What is not acceptable is adding new broad `rusqlite` usage outside the approved owner crates while these migrations are still open.

## Migration Shape

### Phase A: Freeze the boundary

- Treat `db`, `stores`, and `dev-seed` as the only approved direct SQLite owners for new code.
- Do not add new `rusqlite` dependencies elsewhere.
- If a crate outside that set needs DB work, add or extend a lower-level API instead.

### Phase B: Remove presentation-layer leakage

- Remove direct `rusqlite` ownership from `app`.
- Move `app::db` connection/row-mapping mechanics downward into `db` or `stores`.
- Leave `app` with async facade calls and view-model conversion only.

### Phase C: Collapse domain SQL into `db`

- Audit `core` modules that still take `&Connection` / `&Transaction`.
- Extract SQL text, row mapping, and transaction helpers into `db`.
- Leave `core` with typed domain operations and orchestration.

### Phase D: Collapse provider/sync writes into storage APIs

- Audit `sync` and provider crates for direct main-DB writes.
- Replace direct SQL with typed persistence APIs in `db`.
- Keep provider crates focused on fetch/translate/normalize behavior.

### Phase E: Remove shared-surface leakage

- Remove `rusqlite` from public/shared contract surfaces in crates like `common`.
- Ensure shared crates expose storage-agnostic errors and types.

## Enforcement

The steady-state rule should be easy to verify:

- only approved storage-owner crates list `rusqlite` in `Cargo.toml`
- no public APIs outside storage crates expose `rusqlite` types
- no new feature work adds raw SQL outside storage-owner crates

This should eventually be enforced mechanically, ideally by:

- a workspace dependency check in CI
- grep-based or lint-based checks for public `rusqlite` exposure outside approved crates

## What This Eliminates

- `app` quietly becoming a database crate
- `core` and `db` competing to own SQL/query shape
- provider crates writing application rows directly as a convenience shortcut
- shared contract types leaking SQLite implementation details across the workspace
- refactors that must touch many crates because SQLite mechanics are spread everywhere

## Open Questions

1. Should `smart-folder`, `seen`, `calendar`, and `ai` eventually move their SQL entirely into `db`, or do any of them deserve to become explicit storage-owner crates with their own narrow schemas?
2. Should provider-specific caches, if they exist, live under `stores`, under provider crates, or in separate dedicated storage crates?
3. Does `app::db` disappear entirely, or remain as a thin async facade that delegates to `db` without owning SQLite types?
4. Should `common` convert SQLite-specific errors at the storage boundary, or should those be normalized one layer higher in `db`?
5. Is the long-term allowed-owner set exactly `{db, stores, dev-seed}`, or should one additional crate be sanctioned for provider-local cache storage?
