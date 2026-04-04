use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, OptionalExtension, params};
use rtsk::db::DbState;

use super::pinned_searches::ensure_pinned_search_schema;

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

        log::info!("Opening database: {}", db_path.display());
        let conn = Connection::open(&db_path).map_err(|e| format!("open db: {e}"))?;

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
        let write_conn = Connection::open(&db_path).map_err(|e| format!("open write db: {e}"))?;
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
        rtsk::db::migrations::run_all(&write_conn).map_err(|e| format!("migrations: {e}"))?;
        ensure_pinned_search_schema(&write_conn)?;

        log::info!("Database opened, migrations applied");
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            write_conn: Arc::new(Mutex::new(write_conn)),
        })
    }

    pub fn read_db_state(&self) -> DbState {
        DbState::from_arc(Arc::clone(&self.conn))
    }

    pub fn write_db_state(&self) -> DbState {
        DbState::from_arc(Arc::clone(&self.write_conn))
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
        let conn = self
            .write_conn
            .lock()
            .map_err(|e| format!("db write lock: {e}"))?;
        f(&conn)
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

    pub fn get_calendar_default_view(&self) -> Result<Option<String>, String> {
        self.with_conn_sync(|conn| {
            conn.query_row(
                "SELECT value FROM settings WHERE key = 'calendar_default_view'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| e.to_string())
        })
    }

    pub fn mark_queued_drafts_failed(&self) -> Result<usize, String> {
        self.with_write_conn_sync(|conn| {
            conn.execute(
                "UPDATE local_drafts SET sync_status = 'failed' WHERE sync_status = 'queued'",
                [],
            )
            .map_err(|e| e.to_string())
        })
    }

    pub async fn save_local_draft(
        &self,
        id: String,
        account_id: String,
        to_addresses: Option<String>,
        cc_addresses: Option<String>,
        bcc_addresses: Option<String>,
        subject: Option<String>,
        body_html: Option<String>,
        reply_to_message_id: Option<String>,
        thread_id: Option<String>,
        from_email: Option<String>,
        signature_id: Option<String>,
        signature_separator_index: Option<i64>,
    ) -> Result<(), String> {
        self.with_write_conn(move |conn| save_local_draft_inner(
            conn,
            id,
            account_id,
            to_addresses,
            cc_addresses,
            bcc_addresses,
            subject,
            body_html,
            reply_to_message_id,
            thread_id,
            from_email,
            signature_id,
            signature_separator_index,
        ))
        .await
    }

    pub fn save_local_draft_sync(
        &self,
        id: String,
        account_id: String,
        to_addresses: Option<String>,
        cc_addresses: Option<String>,
        bcc_addresses: Option<String>,
        subject: Option<String>,
        body_html: Option<String>,
        reply_to_message_id: Option<String>,
        thread_id: Option<String>,
        from_email: Option<String>,
        signature_id: Option<String>,
        signature_separator_index: Option<i64>,
    ) -> Result<(), String> {
        self.with_write_conn_sync(move |conn| save_local_draft_inner(
            conn,
            id,
            account_id,
            to_addresses,
            cc_addresses,
            bcc_addresses,
            subject,
            body_html,
            reply_to_message_id,
            thread_id,
            from_email,
            signature_id,
            signature_separator_index,
        ))
    }
}

#[allow(clippy::too_many_arguments)]
fn save_local_draft_inner(
    conn: &Connection,
    id: String,
    account_id: String,
    to_addresses: Option<String>,
    cc_addresses: Option<String>,
    bcc_addresses: Option<String>,
    subject: Option<String>,
    body_html: Option<String>,
    reply_to_message_id: Option<String>,
    thread_id: Option<String>,
    from_email: Option<String>,
    signature_id: Option<String>,
    signature_separator_index: Option<i64>,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO local_drafts \
         (id, account_id, to_addresses, cc_addresses, bcc_addresses, \
          subject, body_html, reply_to_message_id, thread_id, \
          from_email, signature_id, signature_separator_index, \
          updated_at, sync_status) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, \
                 unixepoch(), 'pending') \
         ON CONFLICT(id) DO UPDATE SET \
           account_id = ?2, \
           to_addresses = ?3, cc_addresses = ?4, bcc_addresses = ?5, \
           subject = ?6, body_html = ?7, reply_to_message_id = ?8, \
           thread_id = ?9, from_email = ?10, signature_id = ?11, \
           signature_separator_index = ?12, \
           updated_at = unixepoch(), sync_status = 'pending'",
        params![
            id,
            account_id,
            to_addresses,
            cc_addresses,
            bcc_addresses,
            subject,
            body_html,
            reply_to_message_id,
            thread_id,
            from_email,
            signature_id,
            signature_separator_index,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
