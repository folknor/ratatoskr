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
        if let Err(error) = std::fs::rename(&old_wal, &new_wal) {
            log::warn!(
                "failed to migrate WAL file (continuing without it): {error}"
            );
        } else {
            log::info!("Migrated WAL: velo.db-wal -> ratatoskr.db-wal");
        }
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
        if let Err(error) = std::fs::rename(&old_shm, &new_shm) {
            log::warn!(
                "failed to migrate WAL shm file (continuing without it): {error}"
            );
        } else {
            log::info!("Migrated SHM: velo.db-shm -> ratatoskr.db-shm");
        }
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
/// `DbState::init` / `ReadWriteDb::init` use the same canonical pragmas.
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
pub struct DbState {
    conn: Arc<Mutex<Connection>>,
}

impl DbState {
    /// Access the underlying connection Arc for synchronous use.
    pub fn conn(&self) -> Arc<Mutex<Connection>> {
        Arc::clone(&self.conn)
    }

    /// Create a `DbState` from an existing connection Arc.
    ///
    /// Useful for bridging between the app crate's `Db` connection and core
    /// CRUD functions that expect `&DbState`.
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
    read: DbState,
    write: DbState,
}

impl ReadWriteDb {
    pub fn init(app_data_dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(app_data_dir).map_err(|e| format!("create app dir: {e}"))?;
        reconcile_velo_rename(app_data_dir)?;

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

        migrations::run_all(&write_conn)?;

        Ok(Self {
            read: DbState::from_arc(Arc::new(Mutex::new(read_conn))),
            write: DbState::from_arc(Arc::new(Mutex::new(write_conn))),
        })
    }

    pub fn read(&self) -> DbState {
        self.read.clone()
    }

    pub fn write(&self) -> DbState {
        self.write.clone()
    }
}
