pub mod action_journal;
pub mod folder_roles;
pub mod from_row;
mod from_row_impls;
pub mod lookups;
pub mod migrations;
pub mod pending_ops;
pub mod pinned_searches;
pub mod queries;
pub mod queries_extra;
pub mod sql_fragments;
pub mod time;
pub mod types;
pub use from_row::{FromRow, query_as, query_one};
pub use rusqlite::Error as SqlError;
use rusqlite::Connection;
use rusqlite::OpenFlags;
pub use rusqlite::OptionalExtension;
pub use rusqlite::Row;
pub use rusqlite::params;
pub use rusqlite::types::ToSql;

/// Default row limit for queries (contact lists, search results, thread
/// batches) when the caller doesn't specify an explicit limit.
pub const DEFAULT_QUERY_LIMIT: i64 = 500;

use std::path::Path;
use std::sync::{Arc, Mutex};

/// Reconcile the `velo.db` -> `ratatoskr.db` rename, including the partial-
/// rename case (`.db` renamed but `.db-wal` / `.db-shm` not yet).
///
/// The original rename was three independent `std::fs::rename` calls (`.db`,
/// `.db-wal`, `.db-shm`). A crash between the first and second call leaves
/// `ratatoskr.db` alongside `velo.db-wal` / `velo.db-shm`. SQLite's WAL
/// recovery on open relies on the `.db-wal` file being present alongside
/// `.db`; opening `ratatoskr.db` without the matching `ratatoskr.db-wal`
/// would silently lose any WAL-only transactions.
///
/// Recovery rules:
/// - Full pre-rename state (only `velo.db` / `velo.db-wal` / `velo.db-shm`):
///   rename all three.
/// - Already migrated (`ratatoskr.db` exists, no `velo.*` left): no-op.
/// - Partial-rename state (`ratatoskr.db` exists AND `velo.db-wal` or
///   `velo.db-shm` still exist): complete the WAL/SHM rename, but only if
///   the corresponding `ratatoskr.db-wal` / `ratatoskr.db-shm` is absent
///   (otherwise we'd clobber a fresh WAL written by a post-rename open).
///
/// Failure semantics: WAL/SHM rename failures are FATAL (return Err). The
/// caller maps the error to `BootExitCode::MigrationFailure`. Continuing
/// past a failed WAL rename would let the next DB open silently drop WAL-
/// only transactions - the very data-loss mode this function exists to
/// prevent. Orphan-removal failures (when both old and new sidecars exist)
/// are non-fatal because the new sidecar is authoritative; the orphan only
/// wastes disk.
pub fn reconcile_velo_rename(app_data_dir: &Path) -> Result<(), String> {
    let new_db = app_data_dir.join("ratatoskr.db");
    let new_wal = app_data_dir.join("ratatoskr.db-wal");
    let new_shm = app_data_dir.join("ratatoskr.db-shm");
    let old_db = app_data_dir.join("velo.db");
    let old_wal = app_data_dir.join("velo.db-wal");
    let old_shm = app_data_dir.join("velo.db-shm");

    if !new_db.exists() && old_db.exists() {
        log::info!("Migrating database: velo.db -> ratatoskr.db");
        std::fs::rename(&old_db, &new_db)
            .map_err(|e| format!("rename velo.db -> ratatoskr.db: {e}"))?;
    }

    if old_wal.exists() && !new_wal.exists() {
        std::fs::rename(&old_wal, &new_wal)
            .map_err(|e| format!("rename velo.db-wal -> ratatoskr.db-wal: {e}"))?;
        log::info!("Migrated WAL: velo.db-wal -> ratatoskr.db-wal");
    } else if old_wal.exists() && new_wal.exists() {
        log::warn!(
            "both velo.db-wal and ratatoskr.db-wal exist; \
             leaving the new WAL in place and removing the orphan"
        );
        if let Err(error) = std::fs::remove_file(&old_wal) {
            log::warn!("failed to remove orphan velo.db-wal: {error}");
        }
    }

    if old_shm.exists() && !new_shm.exists() {
        std::fs::rename(&old_shm, &new_shm)
            .map_err(|e| format!("rename velo.db-shm -> ratatoskr.db-shm: {e}"))?;
        log::info!("Migrated SHM: velo.db-shm -> ratatoskr.db-shm");
    } else if old_shm.exists() && new_shm.exists() {
        log::warn!(
            "both velo.db-shm and ratatoskr.db-shm exist; \
             leaving the new SHM in place and removing the orphan"
        );
        if let Err(error) = std::fs::remove_file(&old_shm) {
            log::warn!("failed to remove orphan velo.db-shm: {error}");
        }
    }

    Ok(())
}

pub fn apply_writer_pragmas(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA busy_timeout = 15000;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA temp_store = MEMORY;",
    )
    .map_err(|e| format!("pragmas: {e}"))
}

pub fn apply_reader_pragmas(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "PRAGMA busy_timeout = 15000;
         PRAGMA query_only = ON;
         PRAGMA foreign_keys = ON;
         PRAGMA temp_store = MEMORY;",
    )
    .map_err(|e| format!("reader pragmas: {e}"))
}

pub fn apply_standard_pragmas(conn: &Connection) -> Result<(), String> {
    apply_writer_pragmas(conn)
}

/// Error returned by `ReadConn` SQL methods.
///
/// `NotReadOnly` is the typed signal that `prepare`/`prepare_cached`
/// rejected a statement whose `Statement::readonly()` came back false
/// (every mutating SQL string, including `UPDATE ... RETURNING` and
/// `INSERT ... RETURNING` which step through `query`). Previously the
/// bridge type abused `rusqlite::Error::ExecuteReturnedResults` for
/// this case - semantically the opposite condition ("execute() got
/// rows"), and any caller pattern-matching the error would draw the
/// wrong conclusion.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("{0}")]
    Sql(#[from] rusqlite::Error),
    #[error("SQL is not read-only: {0}")]
    NotReadOnly(String),
}

pub struct ReadConn<'a> {
    raw: &'a Connection,
}

impl<'a> ReadConn<'a> {
    #[doc(hidden)]
    pub fn from_raw(raw: &'a Connection) -> Self {
        Self { raw }
    }

    pub fn prepare<'b>(&'b self, sql: &str) -> Result<ReadStatement<'b>, ReadError> {
        let stmt = self.raw.prepare(sql)?;
        if !stmt.readonly() {
            return Err(ReadError::NotReadOnly(sql.to_string()));
        }
        Ok(ReadStatement { raw: stmt })
    }

    pub fn prepare_cached<'b>(
        &'b self,
        sql: &str,
    ) -> Result<ReadCachedStatement<'b>, ReadError> {
        let stmt = self.raw.prepare_cached(sql)?;
        if !stmt.readonly() {
            return Err(ReadError::NotReadOnly(sql.to_string()));
        }
        Ok(ReadCachedStatement { raw: stmt })
    }

    /// Route through validated `prepare` so a mutating SQL string supplied to
    /// `query_row` (e.g. `UPDATE ... RETURNING id`) cannot bypass the
    /// readonly check.
    pub fn query_row<T, P, F>(&self, sql: &str, params: P, f: F) -> Result<T, ReadError>
    where
        P: rusqlite::Params,
        F: FnOnce(&Row<'_>) -> rusqlite::Result<T>,
    {
        let mut stmt = self.prepare(sql)?;
        stmt.query_row(params, f)
    }
}

pub struct ReadStatement<'a> {
    raw: rusqlite::Statement<'a>,
}

impl<'a> ReadStatement<'a> {
    pub fn query<P>(&mut self, params: P) -> Result<rusqlite::Rows<'_>, ReadError>
    where
        P: rusqlite::Params,
    {
        Ok(self.raw.query(params)?)
    }

    pub fn query_map<T, P, F>(
        &mut self,
        params: P,
        f: F,
    ) -> Result<rusqlite::MappedRows<'_, F>, ReadError>
    where
        P: rusqlite::Params,
        F: FnMut(&Row<'_>) -> rusqlite::Result<T>,
    {
        Ok(self.raw.query_map(params, f)?)
    }

    pub fn query_row<T, P, F>(&mut self, params: P, f: F) -> Result<T, ReadError>
    where
        P: rusqlite::Params,
        F: FnOnce(&Row<'_>) -> rusqlite::Result<T>,
    {
        Ok(self.raw.query_row(params, f)?)
    }
}

pub struct ReadCachedStatement<'a> {
    raw: rusqlite::CachedStatement<'a>,
}

impl<'a> ReadCachedStatement<'a> {
    pub fn query<P>(&mut self, params: P) -> Result<rusqlite::Rows<'_>, ReadError>
    where
        P: rusqlite::Params,
    {
        Ok(self.raw.query(params)?)
    }

    pub fn query_map<T, P, F>(
        &mut self,
        params: P,
        f: F,
    ) -> Result<rusqlite::MappedRows<'_, F>, ReadError>
    where
        P: rusqlite::Params,
        F: FnMut(&Row<'_>) -> rusqlite::Result<T>,
    {
        Ok(self.raw.query_map(params, f)?)
    }

    pub fn query_row<T, P, F>(&mut self, params: P, f: F) -> Result<T, ReadError>
    where
        P: rusqlite::Params,
        F: FnOnce(&Row<'_>) -> rusqlite::Result<T>,
    {
        Ok(self.raw.query_row(params, f)?)
    }
}

#[derive(Clone)]
pub struct ReadDbState {
    conn: Arc<Mutex<Connection>>,
}

impl ReadDbState {
    /// Create a `ReadDbState` from an existing connection Arc.
    ///
    /// Useful for bridging between the app crate's `Db` connection and core
    /// CRUD functions that expect `&ReadDbState`.
    pub fn from_arc(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub fn open_existing(app_data_dir: &Path) -> Result<Self, String> {
        let db_path = app_data_dir.join("ratatoskr.db");
        let conn = Connection::open_with_flags(
            &db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| format!("open read db {}: {e}", db_path.display()))?;
        apply_reader_pragmas(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub async fn with_read<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&ReadConn<'_>) -> Result<T, String> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| format!("db lock poisoned: {e}"))?;
            let read = ReadConn::from_raw(&conn);
            f(&read)
        })
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?
    }

    pub async fn with_read_mapped<F, T, E, M>(&self, f: F, map_error: M) -> Result<T, E>
    where
        F: FnOnce(&ReadConn<'_>) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: Send + 'static,
        M: Fn(String) -> E + Copy + Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| map_error(format!("db lock poisoned: {e}")))?;
            let read = ReadConn::from_raw(&conn);
            f(&read)
        })
        .await
        .map_err(|e| map_error(format!("spawn_blocking: {e}")))?
    }

    pub fn with_read_sync<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&ReadConn<'_>) -> Result<T, String>,
    {
        let conn = self
            .conn
            .lock()
            .map_err(|e| format!("db lock poisoned: {e}"))?;
        let read = ReadConn::from_raw(&conn);
        f(&read)
    }

    /// Run a closure with the database connection on the blocking thread pool.
    ///
    /// This ensures rusqlite's synchronous I/O never blocks tokio worker threads.
    pub async fn with_conn<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&Connection) -> Result<T, String> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| format!("db lock poisoned: {e}"))?;
            f(&conn)
        })
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?
    }

    pub fn with_conn_sync<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&Connection) -> Result<T, String>,
    {
        let conn = self
            .conn
            .lock()
            .map_err(|e| format!("db lock poisoned: {e}"))?;
        f(&conn)
    }
}

#[derive(Clone)]
pub struct WriterPool {
    conn: Arc<Mutex<Connection>>,
}

impl WriterPool {
    #[doc(hidden)]
    pub fn from_arc(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub async fn with_conn<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&Connection) -> Result<T, String> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| format!("db lock poisoned: {e}"))?;
            f(&conn)
        })
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?
    }

    pub async fn with_conn_mapped<F, T, E, M>(&self, f: F, map_error: M) -> Result<T, E>
    where
        F: FnOnce(&Connection) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: Send + 'static,
        M: Fn(String) -> E + Copy + Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| map_error(format!("db lock poisoned: {e}")))?;
            f(&conn)
        })
        .await
        .map_err(|e| map_error(format!("spawn_blocking: {e}")))?
    }

    pub fn with_conn_sync<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&Connection) -> Result<T, String>,
    {
        let conn = self
            .conn
            .lock()
            .map_err(|e| format!("db lock poisoned: {e}"))?;
        f(&conn)
    }

    pub async fn with_write<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&WriteConn<'_>) -> Result<T, String> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| format!("db lock poisoned: {e}"))?;
            let write = WriteConn::from_raw(&conn);
            f(&write)
        })
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?
    }

    pub async fn with_write_mapped<F, T, E, M>(&self, f: F, map_error: M) -> Result<T, E>
    where
        F: FnOnce(&WriteConn<'_>) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: Send + 'static,
        M: Fn(String) -> E + Copy + Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| map_error(format!("db lock poisoned: {e}")))?;
            let write = WriteConn::from_raw(&conn);
            f(&write)
        })
        .await
        .map_err(|e| map_error(format!("spawn_blocking: {e}")))?
    }

    pub fn with_write_sync<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&WriteConn<'_>) -> Result<T, String>,
    {
        let conn = self
            .conn
            .lock()
            .map_err(|e| format!("db lock poisoned: {e}"))?;
        let write = WriteConn::from_raw(&conn);
        f(&write)
    }

    pub async fn with_read<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&ReadConn<'_>) -> Result<T, String> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| format!("db lock poisoned: {e}"))?;
            let read = ReadConn::from_raw(&conn);
            f(&read)
        })
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?
    }

    pub fn with_read_sync<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&ReadConn<'_>) -> Result<T, String>,
    {
        let conn = self
            .conn
            .lock()
            .map_err(|e| format!("db lock poisoned: {e}"))?;
        let read = ReadConn::from_raw(&conn);
        f(&read)
    }
}

pub struct WriteConn<'a> {
    raw: &'a Connection,
}

impl<'a> WriteConn<'a> {
    #[doc(hidden)]
    pub fn from_raw(raw: &'a Connection) -> Self {
        Self { raw }
    }

    pub fn execute<P: rusqlite::Params>(&self, sql: &str, params: P) -> rusqlite::Result<usize> {
        self.raw.execute(sql, params)
    }

    pub fn prepare<'b>(&'b self, sql: &str) -> rusqlite::Result<WriteStatement<'b>> {
        Ok(WriteStatement {
            raw: self.raw.prepare(sql)?,
        })
    }

    pub fn query_row<T, P, F>(&self, sql: &str, params: P, f: F) -> rusqlite::Result<T>
    where
        P: rusqlite::Params,
        F: FnOnce(&Row<'_>) -> rusqlite::Result<T>,
    {
        self.raw.query_row(sql, params, f)
    }

    pub fn unchecked_transaction<'b>(&'b self) -> rusqlite::Result<WriteTxn<'b>> {
        Ok(WriteTxn {
            raw: self.raw.unchecked_transaction()?,
        })
    }

    pub fn as_read(&self) -> ReadConn<'_> {
        ReadConn::from_raw(self.raw)
    }
}

pub struct WriteTxn<'a> {
    raw: rusqlite::Transaction<'a>,
}

impl<'a> WriteTxn<'a> {
    pub fn execute<P: rusqlite::Params>(&self, sql: &str, params: P) -> rusqlite::Result<usize> {
        self.raw.execute(sql, params)
    }

    pub fn prepare<'b>(&'b self, sql: &str) -> rusqlite::Result<WriteStatement<'b>> {
        Ok(WriteStatement {
            raw: self.raw.prepare(sql)?,
        })
    }

    pub fn query_row<T, P, F>(&self, sql: &str, params: P, f: F) -> rusqlite::Result<T>
    where
        P: rusqlite::Params,
        F: FnOnce(&Row<'_>) -> rusqlite::Result<T>,
    {
        self.raw.query_row(sql, params, f)
    }

    pub fn commit(self) -> rusqlite::Result<()> {
        self.raw.commit()
    }

    pub fn rollback(self) -> rusqlite::Result<()> {
        self.raw.rollback()
    }

    pub fn as_read(&self) -> ReadConn<'_> {
        ReadConn::from_raw(&self.raw)
    }

    /// Transitional bridge for writer-side crates whose helpers still
    /// take `&rusqlite::Transaction`. Lets a `WriteTxn`-typed caller
    /// hand the underlying transaction to those helpers without
    /// duplicating their SQL. Goes away once those helpers are
    /// retyped to `&WriteTxn` (plan PR 4).
    #[doc(hidden)]
    pub fn as_raw_tx(&self) -> &rusqlite::Transaction<'_> {
        &self.raw
    }
}

pub struct WriteStatement<'a> {
    raw: rusqlite::Statement<'a>,
}

impl<'a> WriteStatement<'a> {
    pub fn execute<P: rusqlite::Params>(&mut self, params: P) -> rusqlite::Result<usize> {
        self.raw.execute(params)
    }

    pub fn query<P>(&mut self, params: P) -> rusqlite::Result<rusqlite::Rows<'_>>
    where
        P: rusqlite::Params,
    {
        self.raw.query(params)
    }

    pub fn query_map<T, P, F>(
        &mut self,
        params: P,
        f: F,
    ) -> rusqlite::Result<rusqlite::MappedRows<'_, F>>
    where
        P: rusqlite::Params,
        F: FnMut(&Row<'_>) -> rusqlite::Result<T>,
    {
        self.raw.query_map(params, f)
    }

    pub fn query_row<T, P, F>(&mut self, params: P, f: F) -> rusqlite::Result<T>
    where
        P: rusqlite::Params,
        F: FnOnce(&Row<'_>) -> rusqlite::Result<T>,
    {
        self.raw.query_row(params, f)
    }
}

pub fn open_reader_pool(app_data_dir: &Path) -> Result<ReadDbState, String> {
    ReadDbState::open_existing(app_data_dir)
}

pub fn open_writer_pool(app_data_dir: &Path) -> Result<WriterPool, String> {
    std::fs::create_dir_all(app_data_dir).map_err(|e| format!("create app dir: {e}"))?;
    reconcile_velo_rename(app_data_dir)?;
    let db_path = app_data_dir.join("ratatoskr.db");
    let conn = Connection::open_with_flags(
        &db_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
        .map_err(|e| format!("open write db {}: {e}", db_path.display()))?;
    apply_writer_pragmas(&conn)?;
    migrations::run_all(&conn)?;
    Ok(WriterPool {
        conn: Arc::new(Mutex::new(conn)),
    })
}

#[cfg(test)]
mod reconcile_velo_rename_tests {
    use super::reconcile_velo_rename;
    use std::fs;
    use tempfile::TempDir;

    fn touch(dir: &std::path::Path, name: &str, contents: &[u8]) {
        fs::write(dir.join(name), contents).expect("write fixture file");
    }

    fn assert_absent(dir: &std::path::Path, name: &str) {
        assert!(
            !dir.join(name).exists(),
            "{name} should be absent in {}",
            dir.display(),
        );
    }

    fn assert_present(dir: &std::path::Path, name: &str) {
        assert!(
            dir.join(name).exists(),
            "{name} should be present in {}",
            dir.display(),
        );
    }

    /// Empty data dir. Nothing to reconcile; no-op success.
    #[test]
    fn empty_dir_is_noop() {
        let tmp = TempDir::new().expect("temp dir");
        reconcile_velo_rename(tmp.path()).expect("empty dir should reconcile cleanly");
        assert_absent(tmp.path(), "ratatoskr.db");
        assert_absent(tmp.path(), "velo.db");
    }

    /// Already-migrated state (only ratatoskr.* exists). No-op success.
    #[test]
    fn already_migrated_is_noop() {
        let tmp = TempDir::new().expect("temp dir");
        touch(tmp.path(), "ratatoskr.db", b"db");
        touch(tmp.path(), "ratatoskr.db-wal", b"wal");
        touch(tmp.path(), "ratatoskr.db-shm", b"shm");
        reconcile_velo_rename(tmp.path()).expect("already-migrated should reconcile cleanly");
        assert_present(tmp.path(), "ratatoskr.db");
        assert_present(tmp.path(), "ratatoskr.db-wal");
        assert_present(tmp.path(), "ratatoskr.db-shm");
    }

    /// Full pre-rename state (only velo.* exists). All three rename to
    /// ratatoskr.*; the velo.* files are gone.
    #[test]
    fn full_pre_rename_renames_all_three() {
        let tmp = TempDir::new().expect("temp dir");
        touch(tmp.path(), "velo.db", b"db");
        touch(tmp.path(), "velo.db-wal", b"wal");
        touch(tmp.path(), "velo.db-shm", b"shm");
        reconcile_velo_rename(tmp.path()).expect("full pre-rename should succeed");
        assert_present(tmp.path(), "ratatoskr.db");
        assert_present(tmp.path(), "ratatoskr.db-wal");
        assert_present(tmp.path(), "ratatoskr.db-shm");
        assert_absent(tmp.path(), "velo.db");
        assert_absent(tmp.path(), "velo.db-wal");
        assert_absent(tmp.path(), "velo.db-shm");
    }

    /// Partial-rename state (.db renamed but .db-wal / .db-shm not yet).
    /// Reconcile completes the WAL + SHM rename. This is the critical case
    /// the partial-rename comment in `reconcile_velo_rename` documents - a
    /// regression that opens the DB without the WAL would silently lose
    /// uncheckpointed transactions.
    #[test]
    fn partial_rename_completes_wal_and_shm() {
        let tmp = TempDir::new().expect("temp dir");
        touch(tmp.path(), "ratatoskr.db", b"db-renamed");
        touch(tmp.path(), "velo.db-wal", b"wal-from-prior-run");
        touch(tmp.path(), "velo.db-shm", b"shm-from-prior-run");
        reconcile_velo_rename(tmp.path()).expect("partial rename should complete");
        assert_present(tmp.path(), "ratatoskr.db");
        assert_present(tmp.path(), "ratatoskr.db-wal");
        assert_present(tmp.path(), "ratatoskr.db-shm");
        assert_absent(tmp.path(), "velo.db-wal");
        assert_absent(tmp.path(), "velo.db-shm");
        assert_eq!(
            fs::read(tmp.path().join("ratatoskr.db-wal")).expect("read wal"),
            b"wal-from-prior-run",
            "the renamed WAL must carry the original bytes",
        );
    }

    /// Partial-rename with only WAL still in velo namespace (SHM already
    /// renamed). Completes the WAL rename only.
    #[test]
    fn partial_rename_wal_only_completes_wal() {
        let tmp = TempDir::new().expect("temp dir");
        touch(tmp.path(), "ratatoskr.db", b"db");
        touch(tmp.path(), "ratatoskr.db-shm", b"shm-already-migrated");
        touch(tmp.path(), "velo.db-wal", b"wal-from-prior-run");
        reconcile_velo_rename(tmp.path()).expect("partial-rename WAL-only should complete");
        assert_present(tmp.path(), "ratatoskr.db-wal");
        assert_absent(tmp.path(), "velo.db-wal");
    }

    /// Both old and new WAL exist. Per the function's documented contract
    /// the new WAL is authoritative; the orphan velo.db-wal is removed and
    /// the new one is left untouched.
    #[test]
    fn dual_existence_preserves_new_wal_and_removes_orphan() {
        let tmp = TempDir::new().expect("temp dir");
        touch(tmp.path(), "ratatoskr.db", b"db");
        touch(tmp.path(), "ratatoskr.db-wal", b"new-wal-keep");
        touch(tmp.path(), "velo.db-wal", b"orphan-wal-discard");
        reconcile_velo_rename(tmp.path()).expect("dual-existence should reconcile cleanly");
        assert_present(tmp.path(), "ratatoskr.db-wal");
        assert_absent(tmp.path(), "velo.db-wal");
        assert_eq!(
            fs::read(tmp.path().join("ratatoskr.db-wal")).expect("read wal"),
            b"new-wal-keep",
            "the new WAL must be untouched",
        );
    }

    /// Same dual-existence guarantee for SHM.
    #[test]
    fn dual_existence_preserves_new_shm_and_removes_orphan() {
        let tmp = TempDir::new().expect("temp dir");
        touch(tmp.path(), "ratatoskr.db", b"db");
        touch(tmp.path(), "ratatoskr.db-shm", b"new-shm-keep");
        touch(tmp.path(), "velo.db-shm", b"orphan-shm-discard");
        reconcile_velo_rename(tmp.path()).expect("dual-existence should reconcile cleanly");
        assert_present(tmp.path(), "ratatoskr.db-shm");
        assert_absent(tmp.path(), "velo.db-shm");
        assert_eq!(
            fs::read(tmp.path().join("ratatoskr.db-shm")).expect("read shm"),
            b"new-shm-keep",
            "the new SHM must be untouched",
        );
    }
}
