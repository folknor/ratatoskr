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
pub use rusqlite::Connection;
pub use rusqlite::Error as SqlError;
pub use rusqlite::OptionalExtension;
pub use rusqlite::Row;
pub use rusqlite::params;

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

/// Apply the standard `PRAGMA` set the Service / UI use after opening a
/// connection. Extracted so the Service boot sequence and the existing
/// `ReadDbState::init` / `ReadWriteDb::init` use the same canonical pragmas.
pub fn apply_standard_pragmas(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA busy_timeout = 15000;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA temp_store = MEMORY;",
    )
    .map_err(|e| format!("pragmas: {e}"))
}

/// Shared database connection managed by Tauri state.
///
/// Uses `std::sync::Mutex` (not `tokio::sync::Mutex`) because rusqlite
/// operations are blocking I/O. All queries run via [`with_conn`] which
/// dispatches to `spawn_blocking` so the tokio async runtime is never blocked.
#[derive(Clone)]
pub struct ReadDbState {
    conn: Arc<Mutex<Connection>>,
}

impl ReadDbState {
    /// Access the underlying connection Arc for synchronous use.
    pub fn conn(&self) -> Arc<Mutex<Connection>> {
        Arc::clone(&self.conn)
    }

    /// Create a `ReadDbState` from an existing connection Arc.
    ///
    /// Useful for bridging between the app crate's `Db` connection and core
    /// CRUD functions that expect `&ReadDbState`.
    pub fn from_arc(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Open (or create) the SQLite database and apply all pending migrations.
    pub fn init(app_data_dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(app_data_dir).map_err(|e| format!("create app dir: {e}"))?;
        reconcile_velo_rename(app_data_dir)?;

        let db_path = app_data_dir.join("ratatoskr.db");
        let conn = Connection::open(&db_path)
            .map_err(|e| format!("open db {}: {e}", db_path.display()))?;
        apply_standard_pragmas(&conn)?;
        migrations::run_all(&conn)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
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
pub struct ReadWriteDb {
    read: ReadDbState,
    write: ReadDbState,
}

impl ReadWriteDb {
    /// Full init: reconcile the velo->ratatoskr rename, open both connections,
    /// run migrations on the writer.
    ///
    /// Phase 1.5 moved boot-side ownership of rename + migration to the
    /// Service. Production UI code MUST NOT call this method - call
    /// [`open_existing`] instead, which opens the connections without
    /// touching schema. `init` is retained for tests that exercise migrations
    /// directly and for any future single-process tooling that wants the full
    /// init.
    pub fn init(app_data_dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(app_data_dir).map_err(|e| format!("create app dir: {e}"))?;
        reconcile_velo_rename(app_data_dir)?;

        let (read, write) = open_read_write_conns(app_data_dir)?;
        migrations::run_all(&write)?;

        Ok(Self {
            read: ReadDbState::from_arc(Arc::new(Mutex::new(read))),
            write: ReadDbState::from_arc(Arc::new(Mutex::new(write))),
        })
    }

    /// Open both read and write connections against an already-migrated DB.
    /// The Service owns rename + migration as part of the boot handshake; by
    /// the time the UI calls this, the schema is current and no rename is
    /// pending. This method explicitly does NOT call `reconcile_velo_rename`
    /// or `migrations::run_all` - duplicating either Service-owned step from
    /// the UI side reintroduces the multiple-writer hazard the Phase 1.5
    /// boot ownership flip was meant to close.
    pub fn open_existing(app_data_dir: &Path) -> Result<Self, String> {
        let (read, write) = open_read_write_conns(app_data_dir)?;
        Ok(Self {
            read: ReadDbState::from_arc(Arc::new(Mutex::new(read))),
            write: ReadDbState::from_arc(Arc::new(Mutex::new(write))),
        })
    }

    pub fn read(&self) -> ReadDbState {
        self.read.clone()
    }

    pub fn write(&self) -> ReadDbState {
        self.write.clone()
    }
}

fn open_read_write_conns(app_data_dir: &Path) -> Result<(Connection, Connection), String> {
    let db_path = app_data_dir.join("ratatoskr.db");

    let read_conn = Connection::open(&db_path)
        .map_err(|e| format!("open db {}: {e}", db_path.display()))?;
    apply_standard_pragmas(&read_conn)?;
    read_conn
        .execute_batch("PRAGMA query_only = ON;")
        .map_err(|e| format!("query_only pragma: {e}"))?;

    let write_conn = Connection::open(&db_path)
        .map_err(|e| format!("open write db {}: {e}", db_path.display()))?;
    apply_standard_pragmas(&write_conn)?;

    Ok((read_conn, write_conn))
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
