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
  serde-only deps), not from `common` — `common` directly depends on
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
- `rtsk`, for **DB access**, depends on `db-read` only — no `db`,
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

**Single reader vs. pool — explicit deferral.** The proposed
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

## Migration plan

Land this in **three PRs**, in order. Doing them at once means every
call-site change conflicts with every crate-move change in review.

PR 0 (Prerequisite, in two parts) removes the load-bearing
`app -> service` dependency: 0a relocates action DTOs and typed IDs;
0b extracts the `runner` binary crate. PR 1 splits the type surface
inside `db`. PR 2 extracts `db-read` and locks the rule in. PR 1 alone is **not** a full
enforcement of the read-crate rule: while it lands, `rtsk` still
depends on `db` and therefore can still name `db::WriteConn`. The
honest framing of PR 1 is "narrow the type surface and remove the
obvious leaks." Full enforcement requires PR 2. Keep the gap short
(same week if possible) and, if PR 2 is delayed, add a temporary
`brokkr.toml` rule forbidding `WriteConn` mentions in `rtsk` source
via a simple grep-style check.

### PR 0: relocate action DTOs and extract `runner` crate

Goal: remove `app -> service` so the app process does not have a
writer-side crate in its direct Cargo graph at all. Two distinct
deliverables; commit them in this order.

#### PR 0a: DTOs to `service-api`, typed IDs to `types`

Today `crates/app/src/` has both direct `use service::actions::*`
imports and fully-qualified `service::actions::*` paths sprinkled
throughout. The complete set the implementer must rewrite (audit
with `grep -rn 'service::' crates/app/src/` before starting):

- `crates/app/src/action_wire.rs:19` — `use service::actions::{ActionError, ActionOutcome, MailOperation, RemoteFailureKind}`.
- `crates/app/src/action_resolve.rs:11` — `use service::actions::{ActionOutcome, FolderId, LabelGroupId, LabelId, MailOperation}`.
- `crates/app/src/handlers/commands.rs:8` — `use service::actions::{ActionOutcome, FolderId, LabelGroupId}`.
- `crates/app/src/handlers/pop_out/compose_send.rs:10` — `use service::actions::{SendAttachment, SendIntent}`.
- `crates/app/src/message.rs:226`, `crates/app/src/update.rs:862`,
  `crates/app/src/app.rs:251`, plus fully-qualified paths at
  `crates/app/src/handlers/pop_out/compose_send.rs:83` and
  `crates/app/src/handlers/commands.rs:330` — these reference
  `service::actions::*` types without `use` statements. All must
  flip to `service_api::*`.

Run the grep before the migration and again after; the post-grep
should return zero hits for `service::` in `crates/app/src/`.

Steps:

1. Move the type definitions from `crates/service/src/actions/`
   (`ActionError`, `ActionOutcome`, `MailOperation`,
   `RemoteFailureKind`, `SendAttachment`, `SendIntent`, `SendRequest`)
   into `crates/service-api/`. Restructure into
   `service-api::actions` for DTOs and `service-api::wire` for the
   existing IPC envelopes. `SendRequest` was missing from earlier
   drafts of this list; it is used by `compose_send` and must move.
2. **Verify** the typed-ID home, do not move. The definitions are
   already in `crates/types/src/typed_ids.rs:11` (`FolderId`,
   `LabelId`, `LabelGroupId`); `crates/common/src/typed_ids.rs:1`
   is already a re-export shim from `types`. Add
   `service-api`'s `Cargo.toml` dep on `types` (if not already
   present) and re-export the typed IDs from `service-api`:
   ```rust
   // service-api
   pub use types::{FolderId, LabelId, LabelGroupId};
   ```
   (`crates/types/src/lib.rs:5` keeps the `typed_ids` module
   private and re-exports the IDs at the crate root at line 14, so
   `types::FolderId` is the public path, not `types::typed_ids::FolderId`.)
   **Do not** have `service-api` re-export from `common` —
   `common` directly depends on `rusqlite` and `db`, which would
   put writer-side crates back into `service-api`'s graph and into
   `app`'s via `app -> service-api -> common -> db`. The
   `service-api-is-pure-leaf` brokkr rule forbids the dep.
3. Have `service::actions::*` re-export from `service_api::actions`
   so existing service-side import paths keep working. The existing
   writer-side `action-types` crate similarly re-exports from
   `service-api` (`action-types` keeps the writer-side
   `ActionContext`/`CalendarActionContext`/`MutationLog` types and
   layers them on top of the wire DTOs from `service-api`).
4. Rewrite every `service::actions::*` reference in `crates/app/src/`
   to `service_api::actions::*` (or the appropriate sub-path). Use
   the grep above as the worklist; verify zero hits afterwards.
5. Remove `service = { path = "../service" }` from
   `crates/app/Cargo.toml`. Verify `cargo tree -p app --depth 1`
   does not list `service`.
6. Add the brokkr rules (see Lockdown below) that forbid
   `app -> service`, `service-api -> common`, etc. Without these,
   the deps can drift back in.

`service-api` after PR 0a holds both IPC envelopes and action DTOs
but remains a pure leaf: no `db`, `db-read`, `rusqlite`,
`service-state`, `store`, `search`, `common`, `service`, or
`action-types` deps. If a type pulled in from `service::actions`
transitively requires one of those, it belongs in `action-types`
(writer-side), not `service-api`, and the app does not need it.

#### PR 0b: extract the `runner` binary crate

`crates/app/src/main.rs:5` currently calls
`service::run_service_blocking()` when launched with `--service`,
and `crates/app/src/service_client.rs:2927` spawns the current
executable with `--service` to start the service process. The
single binary serves two roles. Removing `app -> service` requires
moving the dispatch out of `app`.

Steps:

1. Create `crates/runner/`. `Cargo.toml`:
   ```toml
   [package]
   name = "runner"
   ...
   [[bin]]
   name = "ratatoskr"   # whatever the current binary name is
   path = "src/main.rs"

   [dependencies]
   # Disable app's default features. app currently has
   # default = ["dev-seed"]; runner must opt in explicitly so the
   # production binary does not pull dev-seed by default.
   app = { path = "../app", default-features = false }
   service = { path = "../service", default-features = false }

   [features]
   default = []
   # Development build: enables dev-seed wiping/reseeding on launch.
   dev-seed = ["app/dev-seed"]
   # If hotpath is wanted in the runner, forward it the same way:
   # hotpath = ["app/hotpath", "service/hotpath"]
   ```
   Production builds use `cargo build -p runner` (no features).
   Development builds use `cargo build -p runner --features dev-seed`.
   Update any local launch scripts or aliases that previously ran
   `cargo run -p app` (per `AGENTS.md`) to `cargo run -p runner --features dev-seed`.
2. `crates/runner/src/main.rs` is the new entry point:
   ```rust
   fn main() {
       if std::env::args().any(|a| a == "--service") {
           service::run_service_blocking();
       } else {
           app::run_app_blocking();
       }
   }
   ```
3. Convert `crates/app/` to a library crate: remove the product
   `[[bin]]` target (or `src/main.rs`), expose `app::run_app_blocking()`
   (or whatever name fits) from `lib.rs`. The function body is
   whatever `app/src/main.rs` previously did in the non-`--service`
   branch.

   **Handle `crates/app/src/bin/parent_death_helper.rs` explicitly.**
   Cargo auto-discovers files under `src/bin/` as binary targets,
   and the harness expects this sibling binary
   (`crates/app/src/harness/mod.rs:472`). Pick one:
   - Move `parent_death_helper.rs` into `crates/runner/src/bin/`
     so it lives next to the product binary. Update the harness
     reference. Cleanest.
   - Or keep it in `crates/app/src/bin/` and add
     `autobins = false` + an explicit `[[bin]]` entry to
     `crates/app/Cargo.toml` that names only
     `parent_death_helper`. Documents the intent that this is a
     test/harness helper, not a product binary.
   Pick the first unless there is a reason to keep
   `parent_death_helper` co-located with app source.
4. `crates/app/src/service_client.rs` continues to spawn
   `current_exe()` with `--service`. The spawned process is now
   the `runner` binary, and its `main` dispatches to
   `service::run_service_blocking`. No change to the spawn
   mechanism itself; just the dispatch target.
5. Add `runner` to the workspace `members` in `Cargo.toml`.
6. Update `brokkr.toml`. The current `[ratatoskr.harness]` section
   at `brokkr.toml:87` sets `package = "app"`; change it to
   `package = "runner"` (and adjust the spawned `binary` setting if
   it names `app` explicitly). Without this, `brokkr service-test`
   and `brokkr service-suite` will build and spawn the wrong
   target and the boot harness will silently bypass the runner
   dispatch.
7. Confirm `cargo build -p runner` produces a single executable
   that behaves identically to today's `app` binary in both modes.
   Run the existing harness suite (`brokkr service-suite`) to
   verify nothing in the boot path regressed.

### PR 1: type split inside the existing `db` crate

Goal: every `&Connection` parameter in `rtsk` and `app` becomes
`&ReadConn`. Every `&Connection` parameter in `db` and `service`
becomes `&WriteConn` (for writes) or `&ReadConn` (for read-only
helpers). The app holds no writable connection.

1. Add `ReadConn<'a>`, `ReadStatement<'a, 'b>`, `WriteConn<'a>`,
   `WriteTxn<'t>` to `crates/db/src/db/`. Initially they wrap
   `rusqlite::Connection` and forward to it. Include the
   `Statement::readonly()` validation in `ReadConn::prepare`/
   `prepare_cached`.
2. Add `db::db::WriteConn::from_raw` as `pub(crate)` in `db`
   (callable only from `db::WriterPool::with_write` and `WriteTxn`
   internals). Add `db::db::ReadConn::from_raw` as `pub` +
   `#[doc(hidden)]` for symmetry with the eventual cross-crate
   `db -> db-read` boundary. In PR 1, `ReadConn` still lives in
   `db`, so `WriteConn::as_read()` returns `db::ReadConn`; PR 2
   moves `ReadConn` to `db-read` and the return type becomes
   `db_read::ReadConn` without changing the call shape. Both
   constructors rely on the brokkr `*-no-rusqlite` rules for
   protection from `rtsk` and `app`.
3. Retype `ReadDbState::with_conn*` to `ReadDbState::with_read*`,
   handing out `&ReadConn`. Retype
   `service_state::WriteDbState` to hold a `db::WriterPool` (not an
   `Arc<Mutex<Connection>>`) and delegate
   `WriteDbState::with_write(|&db::WriteConn| ...)` to
   `WriterPool::with_write`. Keep the old `with_conn*` methods alive
   but `#[deprecated]` for the transition; remove at the end.

   Delete the existing escape hatches on `WriteDbState`:
   - Delete `WriteDbState::conn() -> Arc<Mutex<Connection>>`
     (`crates/service-state/src/lib.rs:80`-ish). Callers that drive
     their own `spawn_blocking + lock` shape must route through
     `with_write` instead. The calendar action helpers that lean on
     this (per the docstring at `crates/service-state/src/lib.rs:71-79`)
     are the main consumers and need to be migrated as part of PR 1.
   - Delete `WriteDbState::to_read_state()` outright. Today it hands
     out a `ReadDbState` backed by the **writer** connection
     (`crates/service-state/src/lib.rs:66-68`), which silently
     bypasses the OS-level `SQLITE_OPEN_READ_ONLY` flag the reader
     pool will have. Service code that needs both reads and writes
     holds both `ReadDbState` (over the reader pool) and
     `WriteDbState` (over the writer pool) directly. They are
     separate handles backed by separate connections, as
     `db::open_reader_pool` and `db::open_writer_pool` produce.
4. Split `apply_standard_pragmas` into a writer path and a reader
   path, and place each next to the code that calls it.
   `crates/db/src/db/mod.rs:103` currently sets
   `PRAGMA journal_mode = WAL` for both connections; that is
   writer-side setup and will fail on a connection opened with
   `SQLITE_OPEN_READ_ONLY`. Refactor:
   - `apply_writer_pragmas(&Connection)` in `db`: `journal_mode = WAL`,
     `foreign_keys = ON`, `synchronous = NORMAL`, etc. Called by
     `db::open_writer_pool`. Service boot calls this on the writer
     connection **before** the app opens its read handle. The
     existing `boot.ready` signal enforces this ordering.
   - `apply_reader_pragmas(&Connection)` lives **alongside
     `ReadDbState::open_existing`** — in `db` in PR 1, then moves
     to `db-read` in PR 2 along with `ReadDbState`. It is never a
     public function called across the crate boundary; it is an
     implementation detail of `ReadDbState::open_existing`. This
     keeps `db-read` from needing to depend on `db` after PR 2
     (which would reverse the topology). Reader-safe pragmas only:
     `busy_timeout` (the reader needs to wait on writer-held
     transactions under WAL; without this the reader errors out
     instead of blocking briefly), `query_only = ON` (a
     belt-and-braces SQL-level read gate that survives even if the
     OS-level `SQLITE_OPEN_READ_ONLY` flag is somehow lost),
     `foreign_keys = ON`, and `temp_store`. Does not touch
     `journal_mode` (a database-wide setting persisted by the
     writer).
   `ReadDbState::open_existing` opens the connection with
   `Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)`
   and applies `apply_reader_pragmas` internally. `db::open_reader_pool`
   is a thin wrapper that just calls `ReadDbState::open_existing`.
   The writer side continues to open with full read-write flags and
   `apply_writer_pragmas`. The doc's claim that the app opens
   read-only must match runtime behavior (today it does not; see
   `crates/db/src/db/mod.rs:239,246`).
5. Remove `ReadWriteDb` entirely. Returning a
   `service_state::WriteDbState` from `db::ReadWriteDb::write()`
   would create a `db -> service-state -> db` cycle. Replace
   `ReadWriteDb` with two free constructors in `db`, each returning
   an **opaque** pool type (no raw `Arc<Mutex<Connection>>`
   escaping):
   - `db::open_writer_pool(app_data_dir) -> Result<db::WriterPool, String>`
     opens the writer connection, runs rename reconciliation,
     migrations, applies `apply_writer_pragmas`. The returned
     `db::WriterPool` exposes only `with_write(|&WriteConn| ...)`;
     its inner `Arc<Mutex<Connection>>` is `pub(crate)` and never
     escapes. Service calls this once during boot.
   - `db::open_reader_pool(app_data_dir) -> Result<db::ReadDbState, String>`
     opens a fresh connection with `SQLITE_OPEN_READ_ONLY` and
     applies `apply_reader_pragmas`. `db::ReadDbState` is opaque:
     only `with_read(|&ReadConn| ...)` exposes the wrapped
     connection. Both Service and app call this after writer setup
     completes. (PR 2 moves `ReadDbState` to `db-read` and the
     return type becomes `db_read::ReadDbState`; callers are
     unchanged because the read surface is identical.)

   Service composes a `db::ReadDbState` and a
   `service_state::WriteDbState::from_pool(writer_pool)` where
   `from_pool` takes the opaque `db::WriterPool` (not a raw
   `Arc<Mutex<Connection>>`). App does not depend on `db` directly
   and therefore calls `rtsk::open_reader_pool(app_data_dir)`.

   In PR 1 (`rtsk` still depends on `db`), this is a thin re-export
   of `db::open_reader_pool`.

   In PR 2 (`rtsk` depends on `db-read`, not `db`), it becomes an
   explicit wrapper:
   ```rust
   // rtsk, after PR 2
   pub fn open_reader_pool(app_data_dir: &Path) -> Result<db_read::ReadDbState, String> {
       db_read::ReadDbState::open_existing(&app_data_dir.join("ratatoskr.db"))
   }
   ```
   The app-facing call site stays
   `rtsk::open_reader_pool(app_data_dir)` across both PRs;
   the body changes type and source.

   App never calls `open_writer_pool`. The writable connection is
   never wrapped in a `ReadDbState` and never enters the app
   process. (A future "I need migrations to have run" shortcut
   from app code must instead wait on the Service's existing
   `boot.ready` signal, as it does today.)
6. Walk `rtsk` and `app`: convert every `&Connection` parameter and
   every `with_conn(|c| ...)` closure to take `&ReadConn`. The compile
   errors land as a worklist.

   Watch out for `query_row_and_then` call sites. The closure returns
   `Result<T, E>` where `E` is often `String` or a domain-specific
   error type. `ReadConn::query_row_and_then` routes through
   `prepare`, which means the closure's `E` must accept the new
   `NotReadOnly` failure variant. In PR 1 this variant lives on
   `db::Error` (or wherever `db` already exposes its error type);
   in PR 2 it moves to `db_read::Error`. Three migration shapes:
   - If `E = db::Error` (PR 1) / `db_read::Error` (PR 2), no work
     needed.
   - If `E: From<db::Error>` (PR 1) / `From<db_read::Error>` (PR 2),
     no work needed at the call site, but the domain error needs
     the `From` impl.
   - If `E = String` (common today), the wrapper either calls
     `err.to_string()` internally or exposes a sibling
     `query_row_and_then_string` for ergonomics. Pick one shape and
     document it before mass conversion; flipping mid-migration will
     churn every call site twice.
7. Update the in-flight write helper at
   `crates/db/src/db/queries_extra/label_groups.rs` to take
   `&WriteConn`. Update the Service handler call site at
   `crates/service/src/handlers/label.rs:17`.
8. Convert remaining write helpers in `db` to `&WriteConn`. Convert
   read helpers in `db` (if any are called from `core`/`app`) to
   `&ReadConn`.
9. Delete `pub use db::db::Connection` from
   `crates/core/src/db/mod.rs:4`. Delete `pub use db::db::ReadWriteDb`
   from `crates/core/src/db/mod.rs:3` (`rtsk` should not surface the
   read/write opener at all). Also delete the
   `pub use rusqlite::Connection;` inside the `db` crate at
   `crates/db/src/db/mod.rs:15`: leaving it alive invites a future
   "I'll just re-export it for convenience" PR that silently
   reintroduces the symbol to anyone depending on `db`. Writer-side
   crates that need `Connection` should `use rusqlite::Connection;`
   directly.

   Add `rtsk` re-exports so `app` can construct its read handle
   without depending on `db` directly. In PR 1 these are direct
   re-exports of `db`'s symbols (`pub use db::open_reader_pool;` and
   `pub use db::ReadDbState;` in `crates/core/src/db/mod.rs`); PR 2
   replaces them with the explicit wrapper described in PR 1 step 5
   and a re-export of `db_read::ReadDbState`. `app::Db::open` calls
   `rtsk::open_reader_pool(app_data_dir)` across both PRs.
10. Delete `pub use db::db::queries_extra::*` from
    `crates/core/src/db/queries_extra.rs:2`. Replace with explicit
    re-exports of the read modules `rtsk` actually surfaces.
11. Change `crates/app/src/db/connection.rs`: hold `ReadDbState`,
    not `ReadWriteDb`. Drop `with_conn`/`with_conn_sync` returning
    `&Connection`. Add `with_read`/`with_read_sync` returning
    `&ReadConn`. The app process must not hold a writable connection,
    even one it never uses.
12. Run `brokkr check`. Resolve any remaining call sites.

At the end of PR 1: type discipline holds for reads inside `rtsk` and
`app`. The app holds no writable connection. The doc and runtime
behavior agree about the read-only flag. Writer surface is still
reachable from `rtsk` through its transitive `db` dependency; that is
PR 2's job.

### PR 2: crate extraction (`db-read`) and dependency lockdown

Goal: make the type discipline impossible to subvert by re-export.

1. **Break the `navigation.rs` cycle first.** This is the only
   non-mechanical work in PR 2 and should be the first commit, not
   buried inside the crate move.
   `crates/core/src/db/queries_extra/navigation.rs:247` calls
   `smart_folder::count_smart_folder_unread(conn, ...)`, and
   `smart-folder` depends on `db`. After PR 2's move, `rtsk` would
   need `db-read`, which would need `smart-folder`, which depends on
   `db` — a cycle through the wrong layers. Resolve by **moving**
   `count_smart_folder_unread` (and its supporting query glue) into
   `db-read` rather than carving a duplicate helper. If `smart-folder`
   still needs to expose the function for other callers, it
   re-exports from `db-read`. The invariant after this commit:
   `count_smart_folder_unread` lives in `db-read`, takes a
   `&ReadConn`, and is reachable from both `navigation.rs` and
   `smart-folder` without either pulling in `db`. Land as its own
   commit so the rest of PR 2 stays mechanical.
2. Create `crates/db-read/` with its own `Cargo.toml`. Add it to the
   workspace members list in `Cargo.toml`. Depends on `rusqlite`.
3. Move `ReadConn`, `ReadStatement`, `ReadCachedStatement`,
   `ReadDbState` (with `open_existing` and `with_read*` only), and
   read queries from `db` (and any read queries still in `rtsk`)
   into `db-read`. **Do not** move directory creation, rename
   reconciliation, or the migration runner: those stay in `db` as
   part of `open_writer_pool`. `db-read::ReadDbState` is opaque: its
   only public constructor is `open_existing(&Path)`; there is no
   `from_arc`, no `conn()`, no way to extract or re-wrap the inner
   `Arc<Mutex<Connection>>`.
4. `db` adds `db-read` as a dependency. `WriteConn::as_read` returns
   `db_read::ReadConn`. `db::open_reader_pool`'s return type changes
   from `db::ReadDbState` (PR 1) to `db_read::ReadDbState` (PR 2);
   callers are unchanged because both expose the same `with_read*`
   surface.
5. Remove `db = { path = "../db" }` from `crates/core/Cargo.toml`.
   Add `db-read = { path = "../db-read" }`. Confirm `rtsk` no longer
   needs anything from `db`.
6. `rtsk` re-exports the read surface explicitly. No globs.
   `open_reader_pool` becomes a function in `rtsk` (not a re-export
   of `db::open_reader_pool`, since `rtsk` no longer depends on
   `db`):
   ```rust
   pub use db_read::{ReadConn, ReadDbState};
   pub mod queries_extra {
       pub use db_read::queries_extra::navigation;
       pub use db_read::queries_extra::thread_detail;
       pub use db_read::queries_extra::scoped_queries;
       // ... one line per read module, no globs.
   }

   pub fn open_reader_pool(app_data_dir: &Path) -> Result<db_read::ReadDbState, String> {
       db_read::ReadDbState::open_existing(&app_data_dir.join("ratatoskr.db"))
   }
   ```
   The app-facing call site stays `rtsk::open_reader_pool(app_data_dir)`.
7. `app`'s direct deps for this discipline are `rtsk` (for DB read
   access) and `service-api` (for IPC envelopes and action DTOs).
   It never names `db-read` or `db` directly. Confirm by inspecting
   `crates/app/Cargo.toml`: no `db`, no `db-read`, no `rusqlite`, no
   `service-state`, no `service`, no `action-types`, no `common`.
   `rtsk` and `service-api` (and the other allowed deps unrelated
   to this discipline, e.g. `iced`, `types`, `store`) remain.
8. Add `brokkr.toml` `dependency_rule` entries (see Lockdown below).
9. Add the trybuild lockdown crate (see Lockdown below).
10. Delete the deprecated `with_conn`/`with_conn_sync` bridges from
    PR 1.

## Lockdown tests

The defenses stack. Each layer catches a different mistake.

### Cargo manifests

The first line of defense costs nothing: `rtsk/Cargo.toml` has no
`db = ...` dependency, and `app/Cargo.toml` has no `db`, `db-read`,
or `rusqlite` dependency. Adding `use db::WriteConn;` in `rtsk`
source fails to resolve at `cargo check` time.

This catches direct-dependency violations only. Transitive paths
(e.g. `rtsk -> db-read -> rusqlite`) are expected and not blocked
by either Cargo or by brokkr — the brokkr `forbid` rules below
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
Otherwise the trybuild assertions are advisory — they catch
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
directly and bypass the read discipline — the type system alone
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

The change is done when all of the following hold:

1. `crates/core/src/db/mod.rs` does not contain `pub use ... Connection`
   or any glob re-export of writer modules.
2. `crates/core/src/db/queries_extra.rs` does not contain
   `pub use db::db::queries_extra::*`.
3. `crates/app/src/db/connection.rs` does not name `Connection`,
   `ReadWriteDb`, or `WriteConn`. Its `Db` holds `ReadDbState`. Its
   closures take `&ReadConn<'_>`.
4. `cargo tree -p rtsk --depth 1` does not list `db`, `rusqlite`,
   or `service-state`. (Transitive `rtsk -> db-read -> rusqlite` is
   expected and allowed.)
5. `cargo tree -p app --depth 1` does not list `db`, `db-read`,
   `rusqlite`, `service-state`, `service`, `action-types`, or
   `common`.
6. `brokkr check` passes with the new `core-no-writer-db` and
   `app-no-db-internals` `dependency_rule` entries in place. The
   existing `core-no-rusqlite` and `app-no-rusqlite` rules remain
   in `brokkr.toml`.
7. The trybuild lockdown crate exists at `crates/db-read-lockdown/`
   and all its compile-fail cases compile-fail; all its positive
   cases compile.
8. `crates/db-read/tests/` includes the stepping-bypass regression
   tests for `UPDATE ... RETURNING` and `INSERT ... RETURNING`, and
   they pass.
9. The Service still services `label_group.reorder` (and every other
   write) end to end. `brokkr service-suite` is green.
10. The app's read connection is opened with
    `SQLITE_OPEN_READ_ONLY` (not just `PRAGMA query_only = ON`).
    `crates/db/src/db/mod.rs` no longer uses bare `Connection::open`
    for the read handle.
11. For the main `ratatoskr.db` connection only: the app process
    holds no writable `rusqlite::Connection` and no `WriteDbState`.
    `ReadWriteDb` is deleted. The writer-side pool is constructed
    via `db::open_writer_pool` (Service only). The reader-side pool
    is constructed by Service via `db::open_reader_pool` and by app
    via `rtsk::open_reader_pool` (a thin re-export of the same
    function; app does not depend on `db` directly).
    `apply_writer_pragmas` is called only on the writer connection;
    `apply_reader_pragmas` is called only on reader connections.
    (Body store / inline image store / attachment cache connections
    in `crates/stores/` are out of scope for this PR pair; their own
    read/write split is the follow-up.)
12. `service_state::WriteDbState::with_*` methods hand out
    `&db::WriteConn<'_>` in their closures, not
    `&rusqlite::Connection`. `WriteDbState` holds a `db::WriterPool`
    (not `Arc<Mutex<Connection>>`). `WriteDbState::conn()` is
    deleted. `WriteDbState::to_read_state()` is deleted. Service
    code that needs both reads and writes holds both a
    `db_read::ReadDbState` (over the reader pool) and a
    `WriteDbState` (over the writer pool) as separate handles.
    `db::WriterPool` exposes only `with_write(|&WriteConn| ...)`;
    its inner `Arc<Mutex<Connection>>` is `pub(crate)` and never
    escapes. `db_read::ReadDbState` exposes only
    `open_existing(&Path)` and `with_read*(|&ReadConn| ...)`; no
    `from_arc`, no `conn()`.
13. No `pub use rusqlite::Connection;` (or analogous re-exports of
    mutating rusqlite types) anywhere in `db-read`'s public surface.
    The lockdown grep check passes.
14. `crates/db/src/db/mod.rs` no longer contains
    `pub use rusqlite::Connection;`. Writer-side crates name the
    type directly via `use rusqlite::Connection;`.
15. `store`'s app-facing API surface does not hand out
    `&rusqlite::Connection` or `Arc<Mutex<Connection>>` for the main
    DB. (Audit only in PR 1; deeper restructuring of `store`'s own
    SQLite databases is the follow-up.)
16. `crates/app/Cargo.toml` does not list `service` as a direct
    dependency. `cargo tree -p app --depth 1` does not include
    `service` or `action-types`. `grep -rn 'service::' crates/app/src/`
    returns zero hits. All previously-`service::actions::*` imports
    in app now resolve through `service_api::*`.
17. `service-api` lists no writer-side or heavy-dep crates as deps.
    Specifically its `Cargo.toml` does not include `db`, `db-read`,
    `rusqlite`, `service-state`, `store`, `search`, `service`,
    `action-types`, or `common`. Typed IDs (`FolderId`, `LabelId`,
    `LabelGroupId`) come from the `types` crate, which `service-api`
    depends on.
18. `crates/runner/` exists, owns the Ratatoskr app/service product
    binary (other workspace bins in `squeeze` and `coverage` are
    unrelated tools and unaffected), depends on `app` and `service`
    as libraries, and contains the `--service`-vs-app dispatch in
    `main`. `crates/app/` is a library crate (no `[[bin]]`, exposes
    a `run_app_blocking` or equivalent entry function).
    `brokkr.toml`'s `[ratatoskr.harness]` section names `runner` as
    the package/binary, not `app`. The harness suite
    (`brokkr service-suite`) passes against the new `runner`
    binary.
19. `db-read`'s public surface does not name `rusqlite::Connection`,
    `rusqlite::Transaction`, or `rusqlite::CachedStatement` outside
    its single private `raw` module. The grep-based internal
    quarantine check (see Lockdown) passes: no file under
    `crates/db-read/src/` other than `raw.rs` matches
    `rusqlite::Connection|rusqlite::Transaction|rusqlite::CachedStatement|\.execute\(|\.execute_batch\(|unchecked_transaction|\.transaction\(|pragma_update`.
20. `dev-seed` is exempt from these criteria by design (development
    feature only). The runner gates it behind a `dev-seed` feature;
    production builds use `cargo build -p runner` (no features),
    development builds use `cargo build -p runner --features dev-seed`.
    A CI step verifies that the production runner dep graph excludes
    `dev-seed`: `cargo tree -p runner` (no `--features`) does not
    list `dev-seed`. The exemption is recorded in `brokkr.toml`.
21. The lockdown CI gate (`brokkr test -p db-read-lockdown` or
    equivalent named command) runs the trybuild compile-fail crate,
    the `db-read` stepping regression tests, the `db-read` internal
    raw-rusqlite quarantine grep, and the updated
    `service-state/tests/lockdown.rs` transitive check. CI fails on
    any of them. Running it is not optional.

After landing, the failure modes split cleanly:

- **Method-level writes from the read crate or app** (calling
  `.execute()`, `.transaction()`, `.unchecked_transaction()`, etc.)
  fail to compile. Three layers stop them: Cargo direct-dep
  resolution (no path to `db`/`rusqlite`/`service`/`service-state`
  symbols), type checking (no write methods on `ReadConn`), and the
  trybuild lockdown crate.
- **Mutating SQL strings through read methods** (e.g.
  `query_row("UPDATE ... RETURNING ...", ...)`) **do compile** —
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
