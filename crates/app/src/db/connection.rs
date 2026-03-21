use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

// ── DB connection ───────────────────────────────────────

pub struct Db {
    /// Read-only connection for sync data queries.
    conn: Arc<Mutex<Connection>>,
    /// Writable connection for local state (account creation, pinned
    /// searches, session restore, keybinding overrides, etc.).
    write_conn: Arc<Mutex<Connection>>,
}

impl Db {
    pub fn open(app_data_dir: &Path) -> Result<Self, String> {
        let db_path = app_data_dir.join("ratatoskr.db");
        if !db_path.exists() {
            return Err(format!("database not found: {}", db_path.display()));
        }

        let conn =
            Connection::open(&db_path).map_err(|e| format!("open db: {e}"))?;

        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 15000;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA temp_store = MEMORY;
             PRAGMA query_only = ON;",
        )
        .map_err(|e| format!("pragmas: {e}"))?;

        // Writable connection — same DB, no query_only restriction.
        let write_conn =
            Connection::open(&db_path).map_err(|e| format!("open write db: {e}"))?;
        write_conn
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA busy_timeout = 15000;
                 PRAGMA synchronous = NORMAL;
                 PRAGMA foreign_keys = ON;
                 PRAGMA temp_store = MEMORY;",
            )
            .map_err(|e| format!("write pragmas: {e}"))?;

        // Run pending migrations on the writable connection.
        ratatoskr_core::db::migrations::run_all(&write_conn)
            .map_err(|e| format!("migrations: {e}"))?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            write_conn: Arc::new(Mutex::new(write_conn)),
        })
    }

    /// Access the underlying read-only connection Arc for synchronous use
    /// across thread boundaries (e.g., passing to core functions).
    pub fn conn_arc(&self) -> Arc<Mutex<Connection>> {
        Arc::clone(&self.conn)
    }

    /// Access the underlying writable connection Arc for synchronous use
    /// across thread boundaries.
    pub fn write_conn_arc(&self) -> Arc<Mutex<Connection>> {
        Arc::clone(&self.write_conn)
    }

    /// Execute a closure on the writable connection.
    pub async fn with_write_conn<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&Connection) -> Result<T, String> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.write_conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| format!("db write lock: {e}"))?;
            f(&conn)
        })
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?
    }

    /// Synchronous access to the writable connection.
    pub fn with_write_conn_sync<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&Connection) -> Result<T, String>,
    {
        let conn = self.write_conn.lock().map_err(|e| format!("db write lock: {e}"))?;
        f(&conn)
    }

    pub async fn with_conn<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&Connection) -> Result<T, String> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn =
                conn.lock().map_err(|e| format!("db lock poisoned: {e}"))?;
            f(&conn)
        })
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?
    }

    /// Synchronous DB access for use inside `spawn_blocking`.
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
