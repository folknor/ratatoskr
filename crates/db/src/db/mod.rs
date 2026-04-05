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
pub mod types;
pub use from_row::{FromRow, query_as, query_one};
pub use rusqlite::Connection;

/// Default row limit for queries (contact lists, search results, thread
/// batches) when the caller doesn't specify an explicit limit.
pub const DEFAULT_QUERY_LIMIT: i64 = 500;

use std::path::Path;
use std::sync::{Arc, Mutex};

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

        // Migrate from old database name (velo.db -> ratatoskr.db)
        let db_path = app_data_dir.join("ratatoskr.db");
        let old_db_path = app_data_dir.join("velo.db");
        if !db_path.exists() && old_db_path.exists() {
            log::info!("Migrating database: velo.db -> ratatoskr.db");
            std::fs::rename(&old_db_path, &db_path)
                .map_err(|e| format!("rename velo.db -> ratatoskr.db: {e}"))?;
            // Also migrate WAL files if they exist
            let old_shm = app_data_dir.join("velo.db-shm");
            let old_wal = app_data_dir.join("velo.db-wal");
            if old_shm.exists()
                && let Err(err) = std::fs::rename(&old_shm, app_data_dir.join("ratatoskr.db-shm"))
            {
                log::warn!("Failed to migrate WAL shm file: {err}");
            }
            if old_wal.exists()
                && let Err(err) = std::fs::rename(&old_wal, app_data_dir.join("ratatoskr.db-wal"))
            {
                log::warn!("Failed to migrate WAL file: {err}");
            }
        }
        let conn = Connection::open(&db_path)
            .map_err(|e| format!("open db {}: {e}", db_path.display()))?;

        // Performance pragmas -- match the TS side
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 15000;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA temp_store = MEMORY;",
        )
        .map_err(|e| format!("pragmas: {e}"))?;

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

        let db_path = app_data_dir.join("ratatoskr.db");
        let old_db_path = app_data_dir.join("velo.db");
        if !db_path.exists() && old_db_path.exists() {
            log::info!("Migrating database: velo.db -> ratatoskr.db");
            std::fs::rename(&old_db_path, &db_path)
                .map_err(|e| format!("rename velo.db -> ratatoskr.db: {e}"))?;
            let old_shm = app_data_dir.join("velo.db-shm");
            let old_wal = app_data_dir.join("velo.db-wal");
            if old_shm.exists()
                && let Err(err) = std::fs::rename(&old_shm, app_data_dir.join("ratatoskr.db-shm"))
            {
                log::warn!("Failed to migrate WAL shm file: {err}");
            }
            if old_wal.exists()
                && let Err(err) = std::fs::rename(&old_wal, app_data_dir.join("ratatoskr.db-wal"))
            {
                log::warn!("Failed to migrate WAL file: {err}");
            }
        }

        let read_conn = Connection::open(&db_path)
            .map_err(|e| format!("open db {}: {e}", db_path.display()))?;
        read_conn
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA busy_timeout = 15000;
                 PRAGMA synchronous = NORMAL;
                 PRAGMA foreign_keys = ON;
                 PRAGMA temp_store = MEMORY;
                 PRAGMA query_only = ON;",
            )
            .map_err(|e| format!("pragmas: {e}"))?;

        let write_conn = Connection::open(&db_path)
            .map_err(|e| format!("open write db {}: {e}", db_path.display()))?;
        write_conn
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA busy_timeout = 15000;
                 PRAGMA synchronous = NORMAL;
                 PRAGMA foreign_keys = ON;
                 PRAGMA temp_store = MEMORY;",
            )
            .map_err(|e| format!("write pragmas: {e}"))?;

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
