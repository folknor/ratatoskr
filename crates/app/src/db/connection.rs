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

        // Create pinned searches tables (local app state).
        write_conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS pinned_searches (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    query TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                );
                CREATE UNIQUE INDEX IF NOT EXISTS idx_pinned_searches_query
                    ON pinned_searches(query);
                CREATE TABLE IF NOT EXISTS pinned_search_threads (
                    pinned_search_id INTEGER NOT NULL
                        REFERENCES pinned_searches(id) ON DELETE CASCADE,
                    thread_id TEXT NOT NULL,
                    account_id TEXT NOT NULL,
                    PRIMARY KEY (pinned_search_id, thread_id, account_id)
                );",
            )
            .map_err(|e| format!("create pinned_searches tables: {e}"))?;

        // Ensure contact management columns exist (idempotent).
        for alter in &[
            "ALTER TABLE contacts ADD COLUMN phone TEXT",
            "ALTER TABLE contacts ADD COLUMN company TEXT",
            "ALTER TABLE contacts ADD COLUMN email2 TEXT",
            "ALTER TABLE contacts ADD COLUMN account_id TEXT",
        ] {
            let _ = write_conn.execute_batch(alter);
        }

        // Ensure contact_groups tables exist.
        write_conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS contact_groups (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
                );
                CREATE TABLE IF NOT EXISTS contact_group_members (
                    group_id TEXT NOT NULL
                        REFERENCES contact_groups(id) ON DELETE CASCADE,
                    member_type TEXT NOT NULL CHECK (member_type IN ('email', 'group')),
                    member_value TEXT NOT NULL,
                    PRIMARY KEY (group_id, member_type, member_value)
                );",
            )
            .map_err(|e| format!("create contact_groups tables: {e}"))?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            write_conn: Arc::new(Mutex::new(write_conn)),
        })
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
