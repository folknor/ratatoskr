# Fix Now: DB Writer/Reader Boundary Lockdown

## Why this doc exists

Ratatoskr's architecture has one load-bearing invariant: **the Service
process is the only writer to SQLite**. The app/UI process is read-only.
Every mutation from the UI must route through Service IPC. This rule
prevents UI crashes from corrupting the database and gives the system
exactly one serialization point for writes.

The invariant is currently enforced by:

1. A `brokkr.toml` `app-no-db` rule that forbids `crates/app/` from
   depending on `crates/db/`.
2. Two separate `rusqlite::Connection` objects opened against the same
   DB file at `crates/db/src/db/mod.rs:236-251`. One is the "read"
   connection with `PRAGMA query_only = ON` applied at line 243; the
   other is fully writable, with no `query_only` gate.
3. Comments in `crates/app/src/db/connection.rs` and project docs.

A write helper was recently added to `crates/core/` (the read crate)
instead of `crates/db/`. The function compiled, clippy was clean, tests
passed. It was caught only at runtime when SQLite refused the write
against the `query_only` connection. None of the three defenses above
could catch it at build time:

- The Cargo rule fires on `app -> db`, not on `core -> db` or on what
  `core` does internally.
- The runtime `query_only` check fires after the build is shipped.
- The comments are invisible to the compiler.

Worse: the app process today holds **both** connections via
`ReadWriteDb` (see `crates/app/src/db/connection.rs:7,24` and
`crates/db/src/db/mod.rs:218-234`). The writable one is reachable
through the `inner` field even though no public method on `app::Db`
hands it out. `Connection::open` is used without
`SQLITE_OPEN_READ_ONLY`, so the OS file handle is not read-only either.
The "runtime defense" is one missing `pub` keyword away from being no
defense at all.

This doc specifies the structural fix.

## Status

As of 2026-05-17, PR 0a (action DTOs to `service-api`, typed IDs via
`types`), PR 0b (`runner` binary, `app` library), PR 1 (type split:
`WriteConn`/`WriteTxn`/`WriteStatement`/`ReadConn`/`ReadStatement`/
`ReadCachedStatement`/`WriterPool`/`ReadDbState`, `ReadWriteDb`
deletion, `SQLITE_OPEN_READ_ONLY` + `apply_reader_pragmas`), and the
mechanical parts of PR 2 (the `db-read` crate exists, the
`db-read-lockdown` trybuild crate exists, the brokkr
`core-no-writer-db` / `app-no-db-internals` /
`service-api-is-pure-leaf` rules are in place) have landed.

What remains is the strict end-state shape this doc actually
specifies. Three structural debts:

1. **`db-read` is a facade.** `ReadConn` / `ReadStatement` /
   `ReadCachedStatement` / `ReadDbState` source lives in
   `writer_db::db` and `db-read/src/lib.rs` re-exports them. The
   plan's topology has `db -> db-read`; today it is `db-read -> db`.
   The original `raw.rs` was deleted during the type-consolidation
   pass that fixed an earlier two-struct mismatch; the strict design
   wants the consolidation but with the source parked in `db-read`,
   not `db`.
2. **`from_arc` escape hatches on all three state types.**
   `ReadDbState::from_arc`, `WriterPool::from_arc`, and
   `WriteDbState::from_arc` all take `Arc<Mutex<Connection>>` and
   bypass the opaque-pool design. Two are `#[doc(hidden)]` but all
   are `pub`. Plan calls for `open_existing` / `open_writer_pool` as
   the only constructors with the inner Arc `pub(crate)`.
3. **Untyped `with_conn{,_mapped,_sync}` on `WriterPool` and
   `WriteDbState`.** The migrated action fanout uses these instead
   of `with_write`, so the closures take raw `&Connection` rather
   than `&WriteConn`. The boundary is enforced by call-site
   convention, not by the type system.

The Remaining migration section below covers them as PR 3, PR 4,
and PR 5 (the deferred peripheral crates from the Scope section).
The PR 0 / PR 1 / PR 2 sections have been removed from this doc;
their content is in the git history (start at `729eabe4 db boundary:
complete the slice + fix review blockers` on `main`).

## Root cause

Three leaks compound:

1. **Type collapse.** `rusqlite::Connection` carries `.execute()` and
   `.query_row()` on the same type. As long as that type leaks out of
   `db`, any function reachable from any allowed crate can write. The
   compiler cannot distinguish a read call from a write call.

2. **State-type collapse.** `ReadWriteDb::write()` returns a
   `ReadDbState`, not a separate writer type. So even the existing
   "read state" vs "write state" naming does not enforce anything: the
   writable connection is wrapped in the same type as the read-only
   one, and `service-state::WriteDbState` is a thin alias that does
   not constrain what callers can do once they have it.

3. **Facade too wide.** `crates/core/src/db/mod.rs:4` re-exports the
   raw `Connection`. `crates/core/src/db/queries_extra.rs:2` does
   `pub use db::db::queries_extra::*;` (a glob over the writer
   crate's modules). Once the writer crate's internals are re-exported
   through `rtsk`, the `app-no-db` Cargo rule cannot help: the app
   reaches writer surface through the allowed `rtsk` dependency.

The current concrete evidence:

- `crates/core/src/db/mod.rs:2-4` re-exports `ReadDbState`,
  `ReadWriteDb`, and `Connection`.
- `crates/core/src/db/queries_extra.rs:2` is the glob re-export.
- `crates/app/src/db/connection.rs:7` holds `inner: ReadWriteDb`.
- `crates/app/src/db/connection.rs:32-46` exposes
  `with_conn(&Connection)` and `with_conn_sync(&Connection)`, handing
  any caller a raw connection.
- `crates/db/src/db/mod.rs:222-223` wraps both connections as
  `ReadDbState::from_arc(...)`, including the writable one.
- `crates/db/src/db/mod.rs:239,246` use plain `Connection::open`, not
  `open_with_flags(..., SQLITE_OPEN_READ_ONLY)`.
- The in-flight bad helper is at
  `crates/db/src/db/queries_extra/label_groups.rs` (moved from
  `core`); the Service handler call site is at
  `crates/service/src/handlers/label.rs:17`.

## The fix in one paragraph

Stop treating the database boundary as a Cargo-only or comment-only
invariant. Enforce it in three layers that catch different failure
modes:

- **Cargo topology** that prevents the read crate from naming the
  writer crate at all.
- **Borrowed capability types** that prevent the read crate from
  naming write operations even if it had access.
- **Runtime guards inside the read wrappers** that prevent SQL
  stepping (e.g. `UPDATE ... RETURNING` through `prepare` +
  `query`) from sneaking past the type discipline.

The result: writing a write helper inside `rtsk` fails because the
symbols are not in scope (Cargo + `brokkr check` at workspace level),
writing one inside `db-read` fails because the read-side connection
wrapper has no write methods (type checking), and constructing a
read-only statement from mutating SQL fails at prepare time (runtime
check returning `Err`, plus a regression test).

## Target topology

```
                +-----------+
                |  rusqlite |
                +-----+-----+
                      |
                      v
              +---------------+
              |    db-read    |  ReadConn, ReadStatement, ReadDbState
              |               |  read queries
              +-------+-------+
                      |
        +-------------+-----------------+
        |                               |
        v                               v
  +-----------+                +-----------------+
  |   rtsk    |                |       db        |  WriteConn, WriteTxn,
  |           |                |                 |  write helpers,
  +-----+-----+                +--+-----------+--+  migrations, schema
        |                         |           |
        |   +-------------+       v           v
        |   | service-api |  +-----------------+
        |   | DTOs + IPC  |  |  service-state  |
        |   +------+------+  |  WriteDbState   |
        |          |         +--------+--------+
        |          |                  |
        v          v                  v
  +-----------+    +-----------------------------+
  |    app    |--->|          service            |
  +-----------+    +-----------------------------+
```

- `db-read` defines `ReadConn`, `ReadStatement`, `ReadDbState`, and
  every read query function. Depends on `rusqlite`.
- `db` defines `WriteConn`, `WriteTxn`, write helpers, migrations,
  schema. Depends on `db-read` (so `WriteConn::as_read()` can return
  `db_read::ReadConn`) and on `rusqlite`.
- `service-state` keeps its existing role as the writer-side
  collection of state types (`WriteDbState`, `BodyStoreWriteState`,
  `InlineImageStoreWriteState`, `SearchWriteHandle`). Its
  `WriteDbState` is retyped so its closures hand out `&db::WriteConn`
  instead of `&rusqlite::Connection`. Depends on `db` and `db-read`.
- `service-api` expands beyond its existing IPC-wire role to also
  hold the action DTOs the app currently imports from
  `service::actions::*`: `ActionError`, `ActionOutcome`,
  `MailOperation`, `RemoteFailureKind`, `SendAttachment`,
  `SendIntent`, `SendRequest`. Typed IDs (`FolderId`, `LabelId`,
  `LabelGroupId`) come from the existing `types` crate (lightweight,
  serde-only deps), not from `common` - `common` directly depends on
  `rusqlite` and `db`, so re-exporting from it would transitively
  pull writer-side deps into `service-api`. The typed IDs already
  live in `crates/types/src/typed_ids.rs:11` (PR 0 verifies this
  and that `crates/common/src/typed_ids.rs:1` is already a re-export
  shim); `service-api` re-exports from `types`. `service-api` itself
  has no
  `db`/`db-read`/`rusqlite`/`service-state`/`store`/`search`/`common`
  deps. Pure leaf crate.
- `action-types` (the existing writer-side crate that pairs the wire
  DTOs with `ActionContext`/`CalendarActionContext`/`MutationLog`)
  re-exports from `service-api` so service-side code keeps its
  current import paths. Depends on `service-api`, `db`,
  `service-state`, `store`, `search`.
- `rtsk`, for **DB access**, depends on `db-read` only - no `db`,
  no `rusqlite`, no `service-state`. Re-exports the read surface
  explicitly. No glob re-exports. (`rtsk` retains its many other
  direct deps unrelated to this discipline: `store`, `common`,
  `sync`, `search`, etc. Those are addressed in the follow-up; see
  Scope below.)
- `app` becomes a library crate. Depends on `rtsk` and `service-api`.
  Names `ReadConn` and read queries through `rtsk`'s explicit
  re-exports; names action DTOs through `service-api`. Does not
  depend on `service`, `db`, `db-read`, `rusqlite`, `service-state`,
  or `action-types`. The current same-binary spawn path (where
  `app::service_client` does `current_exe()` + `--service` to start
  the service process) keeps working because the dispatch happens in
  the new `runner` binary, not in `app`.
- `service` depends on `db`, `db-read` (directly, so it can name
  `db_read::ReadConn` / `ReadDbState`), `service-state`,
  `service-api`, and `action-types`.
- `runner` is a new top-level binary crate (`crates/runner/`). It
  owns the Ratatoskr app/service product binary, depends on `app`
  and `service` as libraries, and dispatches in `main` based on
  whether `--service` is in `argv`. This is the only place
  `service` and `app` appear together. Keeps the single-executable
  deployment model intact while removing `app -> service`. (Other
  workspace binaries exist in `squeeze` and `coverage` for
  unrelated tooling; they are not affected.)

`WriteDbState` stays in `service-state` rather than moving to `db`
because (a) `app` already does not depend on `service-state`, so the
crate-level enforcement is already in place; (b) `service-state` is
the natural home for the broader family of writer-side state types,
not just the DB one. The change is **typing**, not relocation: its
`with_*` methods stop handing out raw `&Connection` and start handing
out `&db::WriteConn`.

## Type design

### Borrowed wrappers, not owned

```rust
// db-read
pub struct ReadConn<'a> {
    raw: &'a rusqlite::Connection,
}

pub struct ReadStatement<'a, 'b> {
    raw: rusqlite::Statement<'b>,
    _marker: std::marker::PhantomData<&'a ReadConn<'a>>,
}
```

`ReadConn` is a lifetime-branded view, not an owner. This composes
cleanly with the existing `Arc<Mutex<rusqlite::Connection>>` pattern:
lock the mutex, take `&*guard`, hand it to `ReadConn::from_raw`, pass
into the closure. No re-wrapping at state-construction time, no
duplicated ownership.

### `ReadConn` exposes the read subset only, and validates every SQL string

The following are present:

- `query_row(sql, params, mapper)`
- `query_row_and_then(sql, params, mapper)`
- `prepare(sql) -> Result<ReadStatement, Error>`
- `prepare_cached(sql) -> Result<ReadCachedStatement, Error>`

The following are **deliberately absent**:

- `execute`
- `execute_batch`
- `transaction`
- `unchecked_transaction`
- `pragma_query`, `pragma_query_value`, `pragma_update`. Callers who
  need a PRAGMA go through `prepare("PRAGMA ...")`. The
  `readonly()` validation accepts read-only pragmas (e.g. reading
  `query_only`) and rejects mutating ones (e.g.
  `journal_mode = WAL`). Removing the convenience wrappers also
  removes a class of bypass: rusqlite's `pragma_*` methods build
  statements through `Connection::prepare` directly, not through
  `ReadConn::prepare`.
- Any method or `Deref`/`AsRef` impl that hands out
  `&rusqlite::Connection`.

**Every SQL-taking method routes through `prepare`.** Removing
`execute` is not enough. SQLite stepped statements can mutate even
through `query()`: e.g. `UPDATE foo SET bar = 1 RETURNING id` returns
rows and would be reachable from any `query_row` call that forwards
directly to `rusqlite::Connection::query_row`. So `ReadConn::query_row`
is **not** a direct forwarder; it routes through the validated
`prepare`:

```rust
impl<'a> ReadConn<'a> {
    pub fn prepare(&self, sql: &str) -> Result<ReadStatement<'_, '_>, Error> {
        let stmt = self.raw.prepare(sql)?;
        if !stmt.readonly() {
            return Err(Error::NotReadOnly(sql.to_string()));
        }
        Ok(ReadStatement::wrap(stmt))
    }

    pub fn prepare_cached(&self, sql: &str) -> Result<ReadCachedStatement<'_>, Error> {
        let stmt = self.raw.prepare_cached(sql)?;
        if !stmt.readonly() {
            return Err(Error::NotReadOnly(sql.to_string()));
        }
        Ok(ReadCachedStatement::wrap(stmt))
    }

    pub fn query_row<T, P, F>(&self, sql: &str, params: P, f: F) -> Result<T, Error>
    where
        P: rusqlite::Params,
        F: FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    {
        let mut stmt = self.prepare(sql)?;
        stmt.query_row(params, f).map_err(Into::into)
    }

    pub fn query_row_and_then<T, E, P, F>(&self, sql: &str, params: P, f: F) -> Result<T, E>
    where
        // routes through prepare similarly; E covers both rusqlite::Error
        // and Error::NotReadOnly via From impls.
    { ... }
}
```

`prepare_cached` returns `rusqlite::CachedStatement<'_>`, a distinct
type from `Statement<'_>`. Wrap it in its own `ReadCachedStatement<'a>`
rather than trying to make `ReadStatement` cover both. The lifetimes
and internal storage differ. Both wrappers expose the same read subset
(`query`, `query_map`, `query_row`); their internals differ. A small
`ReadStatementLike` trait on top is optional; the migration does not
need it.

`rusqlite::Statement::readonly()` calls `sqlite3_stmt_readonly()`,
SQLite's own classification of whether a prepared statement can mutate
the database. The check fires on every `prepare`/`prepare_cached` and
therefore on every `query_row`/`query_row_and_then` call as well,
which means every read SQL string in the codebase passes through it
exactly once (or once per cache lookup). It does not give compile-time
read-only guarantees: it is a runtime gate that closes the
stepping-bypass class of mistakes. Pair it with regression tests (see
Lockdown below).

`ReadStatement` and `ReadCachedStatement` expose `query`, `query_map`,
`query_row`. No `execute`. Because every wrapped statement came
through a `readonly()`-validated constructor, stepping it cannot
mutate.

### `ReadDbState` is an opaque reader pool

```rust
// db-read
pub struct ReadDbState {
    conn: Arc<Mutex<rusqlite::Connection>>,
}

impl ReadDbState {
    /// The ONLY constructor. Opens the DB read-only and applies
    /// reader pragmas. Both Service and app call this after the
    /// Service has signalled `boot.ready`.
    pub fn open_existing(path: &Path) -> Result<Self, String> { ... }

    pub async fn with_read<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&ReadConn<'_>) -> Result<T, String> + Send + 'static,
        T: Send + 'static,
    { ... }

    pub fn with_read_sync<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&ReadConn<'_>) -> Result<T, String>,
    { ... }
}
```

`ReadDbState` is opaque: no `pub fn conn(&self) -> Arc<Mutex<...>>`,
no `pub fn from_arc(...)`. The owned `rusqlite::Connection` never
escapes. Closures receive `&ReadConn<'_>`. The mutex lock happens
inside `with_read*` and lasts the closure body.

**`db-read::ReadDbState` is open-existing-only.** Today
`crates/db/src/db/mod.rs:138` and the surrounding init flow create
directories, reconcile the velo->ratatoskr rename, and run migrations.
Those are writer-side operations and **stay in `db`**, not move to
`db-read`. After PR 2:

- `db::open_writer_pool(app_data_dir)` (see PR 1 step 5) handles
  directory creation, rename reconciliation, migrations, and
  `apply_writer_pragmas`. Service calls this once at boot.
- `db_read::ReadDbState::open_existing(path)` opens a database file
  the writer has already prepared, applies `apply_reader_pragmas`,
  and runs nothing else. Both Service and app construct their read
  handles this way **after** the Service has signalled `boot.ready`.

**Single reader vs. pool - explicit deferral.** The proposed
`ReadDbState` holds `Arc<Mutex<rusqlite::Connection>>` (singular),
which is what the code does today. With WAL + `SQLITE_OPEN_READ_ONLY`,
SQLite supports multiple concurrent readers, so an `r2d2`-style pool
of read connections behind `ReadDbState` would let the app run
parallel queries (e.g. sidebar nav + thread list + count badges
simultaneously) without contending on a single mutex. This fix does
not change the existing single-reader shape, by design: it is
orthogonal to the writer/reader type discipline and would expand the
PR scope. If multi-reader scale becomes a measured bottleneck, swap
the inner `Arc<Mutex<Connection>>` for a pool inside `ReadDbState`
without changing `ReadConn`'s API. Flagging here so the choice is a
deliberate "not yet," not an oversight.

### `WriteConn` in `db`

```rust
// db
pub struct WriteConn<'a> {
    raw: &'a rusqlite::Connection,
}

impl<'a> WriteConn<'a> {
    pub fn execute<P: rusqlite::Params>(&self, sql: &str, params: P) -> Result<usize> {
        self.raw.execute(sql, params)
    }

    pub fn transaction<'t>(&'t self) -> Result<WriteTxn<'t>> {
        // The Mutex inside WriteDbState is the real cross-call
        // serialization boundary; using unchecked_transaction here
        // avoids the &mut dance that the Arc<Mutex<Connection>>
        // pattern would otherwise force.
        Ok(WriteTxn { raw: self.raw.unchecked_transaction()? })
    }

    pub fn as_read(&self) -> db_read::ReadConn<'_> {
        db_read::ReadConn::from_raw(self.raw)
    }
}

pub struct WriteTxn<'t> {
    raw: rusqlite::Transaction<'t>,
}

impl<'t> WriteTxn<'t> {
    pub fn execute<P: rusqlite::Params>(...) -> Result<usize> { ... }
    pub fn prepare(...) -> Result<WriteStatement<'_>> { ... }
    pub fn commit(self) -> Result<()> { self.raw.commit() }
    pub fn rollback(self) -> Result<()> { self.raw.rollback() }
    pub fn as_read(&self) -> db_read::ReadConn<'_> { ... }
}
```

**Non-negotiable rules on `WriteConn`'s API:**

- No method may return `&rusqlite::Connection`, `*const`/`*mut`, or
  any owning handle to the inner connection.
- No `Deref`/`AsRef<rusqlite::Connection>`/`Borrow<rusqlite::Connection>`
  impl.
- `as_read` returns a `db_read::ReadConn<'_>`, which is itself
  immutable-reference-only and cannot leak the raw type back.

If a future PR violates these, the `from_raw` argument below breaks
silently. They belong in a comment on `WriteConn` and in code review.

**Nested transactions.** `unchecked_transaction` skips rusqlite's
`&mut self` discipline, which catches accidental nested transactions
at compile time. The mutex serializes across calls, but intra-call
nesting (a write helper calling another that opens its own
transaction) will fail at runtime with SQLite's "cannot start a
transaction within a transaction."

A debug-assertion nesting guard is worth adding, but it **must** live
in the synchronous critical section, not around an `.await`. A
`thread_local!` counter incremented before `.await` and decremented
after can run on different executor threads under tokio's
work-stealing, producing either false positives or silent misses.
Two correct placements:

- Inside `WriteDbState::with_write`'s `spawn_blocking` closure (where
  the mutex is held and execution is single-threaded), bracketing
  the call to `f`.
- On `WriteConn::transaction` itself: bump a counter on entry,
  decrement on `WriteTxn::drop`. This catches both nested
  `with_write` calls **and** nested transactions inside a single
  closure (a write helper calling another helper that opens its own
  transaction), which the `with_write`-only placement misses.

Prefer the latter. Costs nothing in release. Catches the mistake on
the first test run.

### `WriterPool` (in `db`) is the opaque writer pool

```rust
// db
pub struct WriterPool {
    conn: Arc<Mutex<rusqlite::Connection>>,
}

impl WriterPool {
    pub async fn with_write<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&WriteConn<'_>) -> Result<T, String> + Send + 'static,
        T: Send + 'static,
    {
        // Lock the mutex, construct &WriteConn from the guard inside
        // the spawn_blocking body, and hand it to the closure. The
        // raw Arc never escapes.
        ...
    }
}

pub fn open_writer_pool(path: &Path) -> Result<WriterPool, String> { ... }
```

`WriterPool` is the only thing `db::open_writer_pool` returns. Its
inner `Arc<Mutex<Connection>>` is `pub(crate)` and never escapes.
`WriteConn::from_raw` is `pub(crate)` in `db` and called only from
`WriterPool::with_write` (and from `WriteTxn` internals).

### `WriteDbState` stays in `service-state`, wraps `WriterPool`

```rust
// service-state
pub struct WriteDbState {
    pool: db::WriterPool,
}

impl WriteDbState {
    pub fn from_pool(pool: db::WriterPool) -> Self {
        Self { pool }
    }

    pub async fn with_write<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&db::WriteConn<'_>) -> Result<T, String> + Send + 'static,
        T: Send + 'static,
    {
        self.pool.with_write(f).await
    }
}
```

The crate-level enforcement is already correct: `app` does not depend
on `service-state`, so `WriteDbState` is unreachable from UI code.
Wrapping `db::WriterPool` keeps `WriteDbState` co-located with the
rest of the service-side writer state (`BodyStoreWriteState`,
`InlineImageStoreWriteState`, `SearchWriteHandle`) without forcing
`db -> service-state` for the return type of `open_writer_pool`
(which would be a cycle). `WriteDbState` no longer holds an
`Arc<Mutex<Connection>>` directly; it holds a `WriterPool`, which
holds the Arc privately. `WriteDbState::conn()` is gone;
`WriteDbState::to_read_state()` is gone (Service code holds a
separate `ReadDbState` over the reader pool, not a downgraded view
of the writer pool).

### Internal quarantine: `db-read` confines raw `rusqlite` to one module

The type discipline protects the boundary out of `db-read`, but
inside `db-read` any helper that takes `&rusqlite::Connection` and
calls `.execute()` bypasses it. `db-read` depends on `rusqlite` by
necessity, so Cargo cannot stop this.

Confine all raw-rusqlite access to a single private module:

```
crates/db-read/src/
  lib.rs           // pub re-exports of ReadConn, ReadStatement, ReadDbState
  raw.rs           // PRIVATE. The only file that may name
                   // rusqlite::Connection, rusqlite::Transaction,
                   // rusqlite::CachedStatement directly. Owns
                   // ReadConn::from_raw, ReadStatement::wrap,
                   // ReadCachedStatement::wrap, and the
                   // ReadDbState::with_read* lock-and-wrap glue.
  queries/         // All read queries. Take &ReadConn. May not
                   // name rusqlite::Connection or Transaction.
```

Anything that needs to step a statement, lock the mutex, or convert
between raw rusqlite types and the wrapper types lives in `raw.rs`.
Every other file in `db-read` operates on `&ReadConn` and
`ReadStatement` / `ReadCachedStatement` only.

Pin this with a grep-based lockdown check that scans every file
under `crates/db-read/src/` **except** `raw.rs` and fails the build
if it sees `rusqlite::Connection`, `rusqlite::Transaction`,
`rusqlite::CachedStatement`, `.execute(`, `.execute_batch(`,
`unchecked_transaction`, `.transaction(`, or `pragma_update`. The
check lives next to the trybuild lockdown crate.

### `from_raw` constructors

Two constructors convert a raw `&rusqlite::Connection` into a wrapper.
They have different visibilities for structural reasons:

- `db::WriteConn::from_raw(raw: &rusqlite::Connection) -> WriteConn<'_>`
  is `pub(crate)` in `db`. Called only from `db::WriterPool::with_write`
  and `db::WriteTxn` internals. Service-side code never names it,
  because `service_state::WriteDbState::with_write` delegates to
  `WriterPool::with_write` rather than constructing a `WriteConn`
  itself.
- `db_read::ReadConn::from_raw(raw: &rusqlite::Connection) -> ReadConn<'_>`
  is `pub` + `#[doc(hidden)]`. It needs cross-crate visibility because
  `db::WriteConn::as_read()` calls it, and `db_read` cannot be a
  dependency-direction-reversal of `db`. The same protection still
  applies via the brokkr rules below.

The protection argument: `from_raw` is callable only from code that
can name `&rusqlite::Connection`. That requires `rusqlite` as a
**direct** dependency. Cargo enforces this part for free: if
`rtsk/Cargo.toml` does not list `rusqlite`, then `use rusqlite::...`
in `rtsk` source fails to resolve at `cargo check` time, regardless
of what `rtsk`'s transitive dep graph contains. (Transitive presence
of a crate in the dep tree does not put its symbols in scope.) The
brokkr `core-no-rusqlite`/`app-no-rusqlite` rules pin this as a
tested invariant per `reference/architecture.md:61` ("rules are
direct-edge only"); they cover the case where someone adds the dep
"just for one thing."

## Scope: what this fix covers and what it does not

**Covered by PR 1 and PR 2 below:**

- `rtsk` (read-only consumer of `db-read`).
- `app` (read-only consumer of `rtsk`'s explicit re-exports).
- `db` (writer crate; gains `WriteConn`/`WriteTxn` types).
- `db-read` (new crate; owns `ReadConn`/`ReadStatement`/`ReadDbState`
  and read queries).
- `service-state` (`WriteDbState` retyped to hand out `&WriteConn`).
- `service` (write call sites converted to `&WriteConn`).

**Not covered, deferred to a follow-up:**

The workspace has several other crates that currently depend on
`db` and/or `rusqlite` and therefore could still host an
inappropriately-placed write helper or hold a raw `Connection`:
`sync`, `provider-sync`, `gmail`, `jmap`, `graph`, `imap`,
`common`, `stores`, `smart-folder`, `seen`, `calendar`,
`search`, `ai`, `import`, `dev-seed`.

**`dev-seed` is explicitly exempt.** `crates/app/Cargo.toml:51,68`
makes `dev-seed` a default app feature for development convenience.
`dev-seed` depends on `db` and `rusqlite` and writes to the database
on every app launch (it wipes and re-seeds the dev data directory
per `AGENTS.md`). This is by design: development builds always run
with `--features dev-seed`, production builds never do. The doc's
discipline applies to production builds. Add a CI check that
production-target builds reject the `dev-seed` feature (e.g.
`cargo check -p app --no-default-features --features <prod-set>`
fails if `dev-seed` is in the dep graph), and document the
exemption in `brokkr.toml` so it does not get accidentally tightened.

**Scope of "no writable `rusqlite::Connection` in app" claim.** The
acceptance criterion applies to the **main `ratatoskr.db`
connection** only. The `store` crate exposes body store, inline
image store, and attachment file cache, each of which has its own
SQLite database (`bodies.db` and others, per `AGENTS.md`). PR 1
audits `store`'s app-facing API to verify it does not hand out
`&rusqlite::Connection` or `Arc<Mutex<Connection>>` for the main DB
specifically. Splitting `store`'s own writer/reader connections
along the same lines is a separate axis of work and lives in the
follow-up alongside the other writer-side crates.

Several of these crates (notably `store`, `common`, `sync`, `search`)
are reached by `app` via `rtsk`'s direct dependencies and re-exports
(`crates/core/Cargo.toml:24`, `crates/core/src/lib.rs:5`). They are
therefore linked into the app process today, not Service-only. The
PR pair below does not change that. What it does change: even though
these crates remain in app's transitive graph, the discipline
forbids them from handing `app` (or `rtsk`) a writer-capable handle
to the **main `ratatoskr.db`**. Writes against the main DB must go
through a `&db::WriteConn`, which only `service-state::WriteDbState`
can construct, and `service-state` is not on app's path.

The follow-up classifies these crates by whether they actually
mutate:

- **Writer-side (mutate the main DB via `WriteConn`):** `sync`,
  `provider-sync`, `import`, `dev-seed`. Convert their
  `&Connection` parameters to `&WriteConn`.
- **Reader-side (migrate to `db-read`):** likely `smart-folder`
  (after `count_smart_folder_unread` moves into `db-read` in PR 2),
  `seen`, parts of `search`. Convert their `&Connection` parameters
  to `&ReadConn`.
- **Own separate SQLite databases (out of scope for this PR pair):**
  `store` manages `bodies.db`, the inline image store, and the
  attachment cache; the same read/write discipline can be applied
  to those connections later but is independent of the main-DB
  work.

The follow-up PR adds `dependency_rule` entries that forbid reader
crates from depending on `db` and that force writer crates to take
`&WriteConn` instead of `&Connection`. That work is out of scope
here; this doc fixes the load-bearing `rtsk`/`app` boundary first.

## Remaining migration

Three PRs in order. PR 3 is the structural flip and the load-bearing
one; PR 4 ratchets the type discipline closed; PR 5 carries it into
the deferred peripheral crates. Land them sequentially to keep each
review tractable.

The original PR 0 / PR 1 / PR 2 step lists used to live here. They
have been removed now that they are landed; their content is in the
git history under the commits that closed them (`729eabe4 db
boundary: complete the slice + fix review blockers` and `db boundary:
route calendar sync + pending retry through writer pool`).

### PR 3: db-read owns the read types

Goal: the strict end-state topology where `db` depends on `db-read`.
Eliminates `db-read`'s facade-over-`writer_db` shape and gives
`db-read` the single private `raw.rs` module the plan originally
specified.

1. Create `crates/db-read/src/raw.rs` as a `pub(crate)` module
   containing the canonical definitions of `ReadConn`,
   `ReadStatement`, `ReadCachedStatement`, `ReadError`, `ReadDbState`,
   `open_reader_pool`, `apply_reader_pragmas`. Move the source from
   `crates/db/src/db/mod.rs` (and any helper files) verbatim; do
   not change semantics in this PR.
2. `crates/db-read/src/lib.rs` re-exports the new types from `raw`.
   No `writer_db::*` re-exports. Audit each entry in the current
   `db-read/src/lib.rs` `pub mod queries` / `pub mod queries_extra`
   blocks:
   - Items that take `&ReadConn` move into `db-read` proper, retyped
     where needed.
   - Items that still take `&Connection` get retyped to `&ReadConn`
     as part of the move.
   - Items that turn out to be reachable only from writer-side code
     get deleted from `db-read`'s surface; their callers can name
     them through `db` directly post-flip.
3. Flip the Cargo direction. `crates/db-read/Cargo.toml` removes its
   `writer_db = ...` (aka `db = ...`) dep. `crates/db/Cargo.toml`
   adds `db-read = { path = "../db-read" }`. `db::WriteConn::as_read()`
   returns `db_read::ReadConn` (not `db::ReadConn`).
4. Delete the orphaned read-side types from `crates/db/src/db/`:
   `ReadConn`, `ReadStatement`, `ReadCachedStatement`, `ReadError`,
   `ReadDbState`, `apply_reader_pragmas`, `open_reader_pool`. Their
   single home is `db-read`.
5. Make the `db-read` internal-quarantine grep real (the
   `db_read_raw_rusqlite_access_is_quarantined` lockdown test
   already exempts `raw.rs`; today the exemption is dead because the
   file does not exist). Land the grep that scans
   `crates/db-read/src/` excluding `raw.rs` for `rusqlite::Connection`,
   `rusqlite::Transaction`, `rusqlite::CachedStatement`, `.execute(`,
   `.execute_batch(`, `unchecked_transaction`, `.transaction(`,
   `pragma_update`.
6. Walk every `rtsk` and `app` consumer that named a type through
   the old facade path. The compile errors are the worklist.

Acceptance: `cargo tree -p db-read` lists `rusqlite` but not `db`.
`cargo tree -p db` lists `db-read` as a direct dep. The
single-source-of-truth for `ReadConn` is `db-read::raw`.

### PR 4: delete the escape hatches

Goal: make the type discipline impossible to subvert by construction.
Removes the `from_arc` constructors, makes the writer-pool Arc
non-escaping, and retires the untyped `with_conn*` shims.

1. Delete `ReadDbState::from_arc(Arc<Mutex<Connection>>)`. The only
   constructor is `open_existing(&Path)`. Test fixtures that built
   a `ReadDbState` from a shared writer conn (boot.rs tests at
   `crates/service/src/boot.rs:1817,1851` are the known ones) open
   a separate read-only handle via `open_existing` instead.
2. Delete `WriterPool::from_arc(Arc<Mutex<Connection>>)`.
   `WriterPool`'s only constructor is `open_writer_pool(&Path)`.
   Mark the inner `Arc<Mutex<Connection>>` `pub(crate)`.
3. Delete `WriteDbState::from_arc(Arc<Mutex<Connection>>)` from
   `service-state`. The only constructor is
   `WriteDbState::from_pool(WriterPool)`. Boot constructs the pool
   via `db::open_writer_pool` and hands it to
   `WriteDbState::from_pool`.
4. Delete `WriterPool::with_conn{,_mapped,_sync}` and
   `WriteDbState::with_conn{,_mapped,_sync}`. The remaining writer
   surface is `with_write{,_mapped,_sync}(|&WriteConn| ...)`. For
   reads from the writer side (rare; typically a read inside the
   same scope as a planned write) use the existing
   `with_read{,_sync}(|&ReadConn| ...)`.
5. The compile errors land as the call-site worklist. Retype each
   closure to `&WriteConn` and use the typed `WriteConn::execute` /
   `WriteConn::prepare` / `WriteConn::transaction` methods.
   - The action fanout (`crates/service/src/actions/*`) is the bulk
     of the work. Every `db.with_conn(move |conn| ...)` becomes
     `db.with_write(move |conn| ...)`.
   - The pending-ops `_sync` helpers
     (`crates/db/src/db/pending_ops.rs`) have their `&Connection`
     signatures retyped to `&WriteConn`.
   - The `db::queries::set_thread_messages_starred` and similar
     helpers that still take `&Connection` get retyped to
     `&WriteConn`.
   - `WriteTxn::as_raw_tx` (the transitional bridge added in the
     gap-closing slice for `provider-sync/src/graph/sync/persistence.rs`
     and the message_reactions helpers in `calendar_contacts_writes.rs`)
     gets removed once the surrounding `&rusqlite::Transaction`-typed
     helpers in `crates/db/src/db/queries_extra/{message,label}_persistence.rs`
     are retyped to `&WriteTxn`.
6. Tighten the transitive
   `crates/service-state/tests/lockdown.rs` check to assert that
   `app` cannot reach `service-state` through any path at all (the
   PR 0 work removed the `except through service` carve-out's
   justification).

Acceptance: a grep for `from_arc` across `crates/db/`,
`crates/db-read/`, and `crates/service-state/` returns zero hits.
A grep for `Arc<Mutex<Connection>>` in those crates returns hits
only inside the opaque pool's private storage. No untyped
`with_conn*` exists on `WriterPool` or `WriteDbState`. Every closure
that writes the main DB takes `&WriteConn`, not `&Connection`.

### PR 5: migrate the deferred peripheral crates

Goal: extend the boundary into the long-tail crates the original
Scope section deferred. Classify each crate by mutation behavior,
then retype its function signatures.

**Writer-side (convert `&Connection` to `&WriteConn`):**
- `crates/sync/`
- `crates/provider-sync/`
- `crates/import/` (if present at migration time)
- `crates/dev-seed/` stays writer-side; the production-build
  exemption documented in Scope is what gates it, not the type
  discipline.

**Reader-side (convert `&Connection` to `&ReadConn`):**
- `crates/smart-folder/` (after `count_smart_folder_unread` and
  similar helpers move into `db-read`; this was originally PR 2
  step 1 and is still pending)
- `crates/seen/`
- The read paths inside `crates/search/`
- The read paths inside `crates/calendar/` (the write paths went
  through `WriteDbState` in the most recent slice)

**Own separate SQLite databases (same discipline, separate work
item):**
- `crates/stores/` owns `bodies.db`, the inline image store, and
  the attachment file cache. Each gets its own `WriteConn` /
  `ReadConn` pair against its own connection.

**Audit-only:**
- `crates/common/` keeps `rusqlite` for helpers that take typed
  connections passed in by callers. Whether to fold its DB helpers
  into `db` or `db-read` is a separate call; until it loses the
  direct `rusqlite` / `db` deps, the brokkr
  `service-api-is-pure-leaf` rule continues to forbid
  `service-api -> common`, and typed IDs stay in the `types` crate.

For each migrated crate, add a matching `dependency_rule` to
`brokkr.toml`: writer-side crates may depend on `db`, reader-side
crates may depend on `db-read` only.

Acceptance: `cargo tree -p <crate> --depth 1` for each reader-side
crate does not list `db`. The brokkr rules for each crate are
listed in `brokkr.toml` and the gate passes.

## Lockdown tests

The defenses stack. Each layer catches a different mistake.

### Cargo manifests

The first line of defense costs nothing: `rtsk/Cargo.toml` has no
`db = ...` dependency, and `app/Cargo.toml` has no `db`, `db-read`,
or `rusqlite` dependency. Adding `use db::WriteConn;` in `rtsk`
source fails to resolve at `cargo check` time.

This catches direct-dependency violations only. Transitive paths
(e.g. `rtsk -> db-read -> rusqlite`) are expected and not blocked
by either Cargo or by brokkr - the brokkr `forbid` rules below
also check direct edges only. The protection still holds: a
transitive path does not put symbols in scope, so `use rusqlite::...`
in `rtsk` fails to resolve regardless of the dep graph below.
Transitive crate visibility (e.g. "can app reach `service-state`
through any path?") is asserted separately by the integration
tests in `crates/service-state/tests/lockdown.rs`.

### `brokkr.toml` dependency rules

These pin direct-dependency invariants as tested invariants. Catches
the day someone adds `db = { path = "../db" }` to `rtsk/Cargo.toml`
"for one thing":

```toml
[[dependency_rule]]
name = "core-no-writer-db"
from = "rtsk"
forbid = ["db", "rusqlite", "service-state", "service", "action-types"]

[[dependency_rule]]
name = "app-no-db-internals"
from = "app"
forbid = ["db", "db-read", "rusqlite", "service-state", "service", "action-types", "common"]

[[dependency_rule]]
name = "service-api-is-pure-leaf"
from = "service-api"
# service-api is the IPC + action-DTO leaf. Its only allowed
# workspace dep is `types` (typed IDs, serde-only). Forbid every
# other workspace crate so the leaf stays a leaf. External crates
# (serde, serde_json, thiserror, etc.) are fine.
forbid = [
    "db", "db-read", "rusqlite",
    "rtsk", "service", "service-state", "action-types",
    "store", "search", "common", "cal", "calendar",
    "sync", "provider-sync",
    "gmail", "jmap", "graph", "imap", "smtp",
    "smart-folder", "seen", "ai", "import", "dev-seed",
    "app", "runner",
]
```

`common` is in the forbid list because `crates/common/Cargo.toml`
directly depends on `rusqlite` and `db`; re-exporting from `common`
would transitively pull writer-side crates into `service-api`'s
graph and into `app`'s via `app -> service-api -> common -> db`.
Typed IDs go to `service-api` via the `types` crate (serde-only)
instead.

The rule lists most workspace crates explicitly rather than using a
whitelist because `dependency_rule` syntax is `forbid`-only. If
brokkr gains a `allow_only`/whitelist mode in the future, swap this
to `allow = ["types"]` (plus external-crate flexibility).

All brokkr `dependency_rule` entries check direct edges only (per
`reference/architecture.md:61`: "rules are direct-edge only").
Direct rules are sufficient because Cargo only puts a crate's
symbols in scope when it is a **direct** dependency. Transitive
presence of `rusqlite` in `rtsk`'s dep tree (via `db-read`) does not
put `rusqlite`'s symbols in scope from `rtsk` source. The
direct-edge check is therefore the right unit of enforcement; the
brokkr rules do not, and do not need to, handle transitive paths.

Three forbid lists, one purpose each:

- `core-no-writer-db` keeps writer crates out of `rtsk`'s direct
  graph so `rtsk` cannot host write helpers.
- `app-no-db-internals` keeps writer crates and the raw DB out of
  `app`'s direct graph so the app process cannot construct or name
  writer-side handles. Newly includes `service` (closed by PR 0) and
  `action-types` (the writer-side action context crate).
- `service-api-is-pure-leaf` pins `service-api`'s new dual role
  (IPC + action DTOs) as a leaf with no writer-side deps. Without
  this, a future PR could "just add a small `db` import" to
  `service-api` and silently put writer-side symbols back into
  `app`'s graph through the allowed `app -> service-api` edge.

The existing `core-no-rusqlite` and `app-no-rusqlite` rules forbid
the direct `rusqlite` dep specifically and remain in place. The
existing `app-no-db` rule is subsumed by `app-no-db-internals` and
can be removed.

The transitive lockdown already in
`crates/service-state/tests/lockdown.rs` (asserting that `app`
cannot reach `cal` or `service-state` through any dep-path chain
except through `service`) needs updating once `app -> service` is
removed in PR 0: the carve-out "except through `service`" is no
longer needed and the test should assert app reaches
`service-state` / `cal` through **no** path. Tighten it as part of
PR 0.

### `trybuild` tests

These pin the type API. Catches the day someone adds
`pub fn execute(...)` to `ReadConn` "for convenience".

Place the trybuild harness in a dedicated crate
(`crates/db-read-lockdown/`, `publish = false`) so the failure cases
do not pollute the production build and so the crate can also hold
positive cases that prove the read API still works. Add the crate to
the workspace members list but keep it out of `brokkr check`'s
default scope so CI does not pay for it on every diagnostic pass.

**CI gate (must be named explicitly).** Run the lockdown crate via
`brokkr test -p db-read-lockdown` (or, if `brokkr` doesn't already
have a `lockdown` subcommand, add one that runs the trybuild crate,
the `db-read::tests` stepping regressions, and the
`service-state/tests/lockdown.rs` transitive check together). The
existing CI must invoke this gate, not just `brokkr check`.
Otherwise the trybuild assertions are advisory - they catch
regressions only when someone remembers to run them. The acceptance
criteria below name the gate explicitly.

Compile-fail cases:

- `read_conn_no_execute.rs`:
  ```rust
  fn _proves(c: &db_read::ReadConn<'_>) { c.execute("UPDATE x SET y = 1", []); }
  ```
- `read_conn_no_unchecked_transaction.rs`:
  ```rust
  fn _proves(c: &db_read::ReadConn<'_>) { let _ = c.unchecked_transaction(); }
  ```
- `read_conn_no_transaction.rs`:
  ```rust
  fn _proves(c: &db_read::ReadConn<'_>) { let _ = c.transaction(); }
  ```
- `read_statement_no_execute.rs`: same shape for `ReadStatement`.

Positive cases (must compile):

- `read_conn_query_row.rs`: a representative `query_row` call works.
- `read_conn_prepare_select.rs`: `prepare("SELECT ...")` returns a
  `ReadStatement`.

**Re-export discipline.** `db-read` MUST NOT
`pub use rusqlite::Connection;` (or any other `pub use rusqlite::*`
that exposes mutating types like `Statement`, `Transaction`,
`CachedStatement`). The `from_raw` argument depends on `rtsk` and
`app` being unable to name `&rusqlite::Connection`; a re-export in
`db-read` would silently undo that, since `rtsk` depends directly on
`db-read`. Pin this with a lockdown check that greps `db-read`'s
public surface for `pub use rusqlite` and fails the build if any
such re-export appears.

**Internal raw-rusqlite quarantine.** Inside `db-read`, raw
`rusqlite::Connection` / `Transaction` / `CachedStatement` access
lives in exactly one private module (`crates/db-read/src/raw.rs`).
Everything else in `db-read` operates on `&ReadConn`,
`ReadStatement`, and `ReadCachedStatement`. A second grep-based
check scans every file under `crates/db-read/src/` other than
`raw.rs` and fails the build on any of:
`rusqlite::Connection`, `rusqlite::Transaction`,
`rusqlite::CachedStatement`, `.execute(`, `.execute_batch(`,
`unchecked_transaction`, `.transaction(`, `pragma_update`. Without
this, a `db-read`-internal helper could take `&rusqlite::Connection`
directly and bypass the read discipline - the type system alone
cannot stop it, because `db-read` necessarily depends on `rusqlite`.

Both grep checks live in (or are invoked by) the lockdown crate so
they run under the same CI gate as the trybuild and stepping
regression tests.

### Runtime regression test for SQL stepping

The `Statement::readonly()` validation in `ReadConn::prepare` is the
defense against stepping-bypass writes. Pin it with a runtime test
in `crates/db-read/tests/`:

```rust
#[test]
fn prepare_rejects_update_returning() {
    let conn = open_test_db();
    let read = db_read::ReadConn::from_raw(&conn);
    let err = read.prepare("UPDATE messages SET seen = 1 RETURNING id")
        .expect_err("UPDATE ... RETURNING should not be prepareable on ReadConn");
    assert!(matches!(err, db_read::Error::NotReadOnly(_)));
}

#[test]
fn prepare_rejects_insert_returning() {
    let conn = open_test_db();
    let read = db_read::ReadConn::from_raw(&conn);
    assert!(read.prepare("INSERT INTO labels (name) VALUES ('x') RETURNING id").is_err());
}

#[test]
fn prepare_accepts_plain_select() {
    let conn = open_test_db();
    let read = db_read::ReadConn::from_raw(&conn);
    assert!(read.prepare("SELECT id FROM messages WHERE label = ?1").is_ok());
}

#[test]
fn query_row_rejects_update_returning() {
    // ReadConn::query_row routes through validated prepare, so
    // mutating SQL via query_row fails the same way as via prepare.
    let conn = open_test_db();
    let read = db_read::ReadConn::from_raw(&conn);
    let err = read.query_row(
        "UPDATE messages SET seen = 1 RETURNING id",
        [],
        |row| row.get::<_, i64>(0),
    ).expect_err("query_row with mutating SQL should fail readonly check");
    assert!(matches!(err, db_read::Error::NotReadOnly(_)));
}
```

### Runtime defenses stay (and get stronger)

Today the app's read connection has `PRAGMA query_only = ON`. The
writable connection has no such gate. After PR 1 step 4, the app's
read connection is opened with `SQLITE_OPEN_READ_ONLY` at the OS file
flag level, on top of `query_only`. The structural fix makes these
the last line of defense rather than the only line; both layers are
cheap and still useful against bugs we have not imagined yet.

## Out of scope

- **Routing reads through Service IPC.** Cleanest in theory: the app
  has no SQLite access at all. Rejected because
  `rtsk::db::queries_extra::*` is called heavily on the read path
  and moving every read through IPC is a perf and churn hit out of
  proportion to the bug. Direct read-only SQLite access from `app`
  stays.
- **Zero-sized `WriteCap` capability token.** A `WriteCap(())` with
  a `pub(crate)` constructor, required as a parameter on every write
  helper, defends against a future refactor accidentally widening
  `WriteConn` construction. Skipped: with crate split, type wrappers,
  brokkr rules, trybuild tests, the `Statement::readonly()` runtime
  gate, the `SQLITE_OPEN_READ_ONLY` flag, and `PRAGMA query_only`,
  this is suspenders on a belted-braced outfit.
- **Splitting `db-read` further.** No reason yet. Keep it flat until
  a second axis (read-side caches, projection layers) actually
  appears.
- **Migrating `sync`/`provider-sync`/`stores`/etc. off raw
  `Connection`.** Real and worth doing; see Scope above. Out of
  scope for this PR pair to keep the change reviewable.

## Acceptance criteria

Status markers: **LANDED** for criteria already met as of the most
recent commit; **OPEN** for criteria still outstanding. PR
cross-references point to the Remaining migration section above.

1. **LANDED.** `crates/core/src/db/mod.rs` does not contain
   `pub use ... Connection` or any glob re-export of writer modules.
2. **LANDED.** `crates/core/src/db/queries_extra.rs` does not
   contain `pub use db::db::queries_extra::*`.
3. **LANDED.** `crates/app/src/db/connection.rs` does not name
   `Connection`, `ReadWriteDb`, or `WriteConn`. Its `Db` holds
   `ReadDbState`. Its closures take `&ReadConn<'_>`.
4. **OPEN (PR 3).** `cargo tree -p rtsk --depth 1` does not list
   `db`, `rusqlite`, or `service-state`. Today `rtsk` depends on
   `db-read`, which depends on `db`. PR 3 flips this so
   `db -> db-read`.
5. **LANDED.** `cargo tree -p app --depth 1` does not list `db`,
   `db-read`, `rusqlite`, `service-state`, `service`,
   `action-types`, or `common`.
6. **LANDED.** `brokkr check` passes with `core-no-writer-db`,
   `app-no-db-internals`, and `service-api-is-pure-leaf` in place.
7. **LANDED.** The `db-read-lockdown` crate exists with passing
   compile-fail and positive cases.
8. **LANDED.** The stepping-bypass regression tests in
   `crates/db-read/tests/` pass.
9. **LANDED.** `brokkr service-suite` is green for the migrated
   write surface.
10. **LANDED.** The app's read connection is opened with
    `SQLITE_OPEN_READ_ONLY` and `apply_reader_pragmas`.
11. **OPEN (PR 4).** The app process holds no writable
    `rusqlite::Connection` and no `WriteDbState`. Holds for app
    today, but `WriteDbState::from_arc` and `WriterPool::from_arc`
    are publicly constructible from any crate that can name
    `Arc<Mutex<Connection>>`. PR 4 deletes them.
12. **OPEN (PR 4).** `WriteDbState::with_*` methods hand out
    `&db::WriteConn<'_>`. Today both typed (`with_write*`) and
    untyped (`with_conn*`) variants are exposed; the untyped ones
    pass raw `&Connection`. PR 4 deletes the untyped surface.
13. **LANDED.** `db-read` does not `pub use rusqlite::Connection`
    on its public surface.
14. **LANDED.** `crates/db/src/db/mod.rs` no longer
    `pub use rusqlite::Connection;`.
15. **OPEN (audit, PR 5).** `store`'s app-facing API does not hand
    out `&rusqlite::Connection` for the main DB.
16. **LANDED.** `crates/app/Cargo.toml` has no `service` dep;
    `grep -rn 'service::' crates/app/src/` returns zero hits.
17. **LANDED.** `service-api` has no writer-side or heavy-dep
    crates as deps.
18. **LANDED.** `crates/runner/` exists, harness suite passes
    against it.
19. **OPEN (PR 3).** `db-read`'s internal raw-rusqlite quarantine:
    `raw.rs` exists and the grep check scans every other file.
    Today neither holds: no `raw.rs`, dead exemption in the
    lockdown test.
20. **LANDED.** `dev-seed` exemption documented in `brokkr.toml`;
    production runner build excludes it.
21. **PARTIAL.** Lockdown CI gate runs the trybuild crate and the
    stepping regressions today. PR 3 adds the internal quarantine
    grep; PR 4 tightens the transitive
    `service-state/tests/lockdown.rs` check (the `except through
    service` carve-out is no longer needed post-PR-0).
22. **OPEN (PR 3).** `ReadConn`/`ReadStatement`/
    `ReadCachedStatement`/`ReadDbState` source lives in
    `crates/db-read/src/raw.rs`, not in `writer_db::db`.
    `cargo tree -p db-read` does not list `db`; `cargo tree -p db`
    lists `db-read`.
23. **OPEN (PR 4).** No `from_arc` constructor exists on
    `ReadDbState`, `WriterPool`, or `WriteDbState`. No untyped
    `with_conn*` exists on `WriterPool` or `WriteDbState`. Every
    closure that writes the main DB takes `&WriteConn`, not
    `&Connection`.
24. **OPEN (PR 5).** The peripheral crates from the Scope section
    (`sync`, `provider-sync`, `import`, `smart-folder`, `seen`,
    parts of `search` and `calendar`) take `&WriteConn` or
    `&ReadConn`, not `&Connection`. Matching `dependency_rule`
    entries are in `brokkr.toml`.

After all three PRs land, the failure modes split cleanly:

- **Method-level writes from the read crate or app** (calling
  `.execute()`, `.transaction()`, `.unchecked_transaction()`, etc.)
  fail to compile. Three layers stop them: Cargo direct-dep
  resolution (no path to `db`/`rusqlite`/`service`/`service-state`
  symbols), type checking (no write methods on `ReadConn`), and the
  trybuild lockdown crate.
- **Mutating SQL strings through read methods** (e.g.
  `query_row("UPDATE ... RETURNING ...", ...)`) **do compile** -
  this is a runtime check, not a compile-time one. They fail at
  prepare time via `Statement::readonly()`, surfaced as
  `Error::NotReadOnly`, and are pinned by regression tests in
  `db-read/tests/`. This is a deliberate trade: closing it at
  compile time would require removing `prepare` from the read API
  entirely, which is too costly.
- **Adding `app -> service` back, or adding `service-api -> db`**
  fails the brokkr CI gate via the new `app-no-db-internals` and
  `service-api-is-pure-leaf` rules.

The runtime SQLite check and the OS-level `SQLITE_OPEN_READ_ONLY`
flag stop being the only things standing between a typo and
database corruption. The runtime check on `ReadConn::prepare` covers
the one class of mistake that cannot be moved to compile time
without an unacceptable ergonomics cost.
