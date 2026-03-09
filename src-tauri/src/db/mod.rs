pub mod migrations;
pub mod queries;
pub mod types;

use std::path::PathBuf;
use std::sync::Arc;

use rusqlite::Connection;
use tokio::sync::Mutex;

/// Shared database connection managed by Tauri state.
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

    /// Acquire the connection lock.
    pub async fn conn(&self) -> tokio::sync::MutexGuard<'_, Connection> {
        self.conn.lock().await
    }
}
