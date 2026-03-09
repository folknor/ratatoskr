pub mod migrations;
pub mod queries;
pub mod queries_extra;
pub mod types;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

/// Shared database connection managed by Tauri state.
///
/// Uses `std::sync::Mutex` (not `tokio::sync::Mutex`) because rusqlite
/// operations are blocking I/O. All queries run via [`with_conn`] which
/// dispatches to `spawn_blocking` so the tokio async runtime is never blocked.
pub struct DbState {
    conn: Arc<Mutex<Connection>>,
}

impl DbState {
    /// Open (or create) the SQLite database and apply all pending migrations.
    pub fn init(app_data_dir: &PathBuf) -> Result<Self, String> {
        std::fs::create_dir_all(app_data_dir).map_err(|e| format!("create app dir: {e}"))?;

        let db_path = app_data_dir.join("velo.db");
        let conn = Connection::open(&db_path)
            .map_err(|e| format!("open db {}: {e}", db_path.display()))?;

        // Performance pragmas — match the TS side
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
}
