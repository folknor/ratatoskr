use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, Row, params};

// ── Date display mode ───────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateDisplay {
    /// Absolute date + relative offset from first message ("+14d")
    RelativeOffset,
    /// "Mar 12, 2026 at 2:34 PM"
    Absolute,
}

// ── Types (subset of src-tauri/src/db/types.rs) ─────────────

#[derive(Debug, Clone)]
pub struct Account {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub provider: String,
}

#[derive(Debug, Clone)]
pub struct Thread {
    pub id: String,
    pub account_id: String,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub last_message_at: Option<i64>,
    pub message_count: i64,
    pub is_read: bool,
    pub is_starred: bool,
    pub has_attachments: bool,
    pub from_name: Option<String>,
    pub from_address: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Label {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct ThreadMessage {
    pub id: String,
    pub thread_id: String,
    pub account_id: String,
    pub from_name: Option<String>,
    pub from_address: Option<String>,
    pub to_addresses: Option<String>,
    pub date: Option<i64>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub is_read: bool,
    pub is_starred: bool,
}

#[derive(Debug, Clone)]
pub struct ThreadAttachment {
    pub id: String,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub size: Option<i64>,
    pub from_name: Option<String>,
    pub date: Option<i64>,
}

// ── DB connection ───────────────────────────────────────────

pub struct Db {
    conn: Arc<Mutex<Connection>>,
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

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
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

    pub async fn get_accounts(&self) -> Result<Vec<Account>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, email, display_name, provider
                     FROM accounts WHERE is_active = 1
                     ORDER BY created_at ASC",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map([], |row| {
                Ok(Account {
                    id: row.get("id")?,
                    email: row.get("email")?,
                    display_name: row.get("display_name")?,
                    provider: row.get("provider")?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
    }

    pub async fn get_labels(
        &self,
        account_id: String,
    ) -> Result<Vec<Label>, String> {
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, name FROM labels
                     WHERE account_id = ?1 AND visible = 1
                     ORDER BY sort_order ASC, name ASC",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map(params![account_id], |row| {
                Ok(Label {
                    id: row.get("id")?,
                    name: row.get("name")?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
    }

    pub async fn get_threads(
        &self,
        account_id: String,
        label_id: Option<String>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Thread>, String> {
        self.with_conn(move |conn| {
            let (sql, do_label) = if label_id.is_some() {
                (
                    "SELECT t.*, m.from_name, m.from_address FROM threads t
                     INNER JOIN thread_labels tl
                       ON tl.account_id = t.account_id AND tl.thread_id = t.id
                     LEFT JOIN messages m
                       ON m.account_id = t.account_id AND m.thread_id = t.id
                       AND m.date = (SELECT MAX(m2.date) FROM messages m2
                                     WHERE m2.account_id = t.account_id
                                       AND m2.thread_id = t.id)
                     WHERE t.account_id = ?1 AND tl.label_id = ?2
                     GROUP BY t.account_id, t.id
                     ORDER BY t.is_pinned DESC, t.last_message_at DESC
                     LIMIT ?3 OFFSET ?4",
                    true,
                )
            } else {
                (
                    "SELECT t.*, m.from_name, m.from_address FROM threads t
                     LEFT JOIN messages m
                       ON m.account_id = t.account_id AND m.thread_id = t.id
                       AND m.date = (SELECT MAX(m2.date) FROM messages m2
                                     WHERE m2.account_id = t.account_id
                                       AND m2.thread_id = t.id)
                     WHERE t.account_id = ?1
                     ORDER BY t.is_pinned DESC, t.last_message_at DESC
                     LIMIT ?2 OFFSET ?3",
                    false,
                )
            };

            let mut stmt =
                conn.prepare(sql).map_err(|e| e.to_string())?;

            let rows = if do_label {
                stmt.query_map(
                    params![account_id, label_id.unwrap_or_default(), limit, offset],
                    row_to_thread,
                )
            } else {
                stmt.query_map(
                    params![account_id, limit, offset],
                    row_to_thread,
                )
            };

            rows.map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
    }
    pub async fn get_thread_messages(
        &self,
        account_id: String,
        thread_id: String,
    ) -> Result<Vec<ThreadMessage>, String> {
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, thread_id, account_id, from_name, from_address,
                        to_addresses, date, subject, snippet, is_read, is_starred
                 FROM messages
                 WHERE account_id = ?1 AND thread_id = ?2
                 ORDER BY date DESC"
            ).map_err(|e| e.to_string())?;

            stmt.query_map(params![account_id, thread_id], |row| {
                Ok(ThreadMessage {
                    id: row.get("id")?,
                    thread_id: row.get("thread_id")?,
                    account_id: row.get("account_id")?,
                    from_name: row.get("from_name")?,
                    from_address: row.get("from_address")?,
                    to_addresses: row.get("to_addresses")?,
                    date: row.get("date")?,
                    subject: row.get("subject")?,
                    snippet: row.get("snippet")?,
                    is_read: row.get::<_, i64>("is_read")? != 0,
                    is_starred: row.get::<_, i64>("is_starred")? != 0,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
    }

    pub async fn get_thread_attachments(
        &self,
        account_id: String,
        thread_id: String,
    ) -> Result<Vec<ThreadAttachment>, String> {
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT a.id, a.filename, a.mime_type, a.size,
                        m.from_name, m.date
                 FROM attachments a
                 JOIN messages m ON a.message_id = m.id AND a.account_id = m.account_id
                 WHERE a.account_id = ?1 AND m.thread_id = ?2
                   AND a.is_inline = 0
                   AND a.filename IS NOT NULL AND a.filename != ''
                 ORDER BY m.date DESC"
            ).map_err(|e| e.to_string())?;

            stmt.query_map(params![account_id, thread_id], |row| {
                Ok(ThreadAttachment {
                    id: row.get("id")?,
                    filename: row.get("filename")?,
                    mime_type: row.get("mime_type")?,
                    size: row.get("size")?,
                    from_name: row.get("from_name")?,
                    date: row.get("date")?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
    }
}

fn row_to_thread(row: &Row<'_>) -> rusqlite::Result<Thread> {
    Ok(Thread {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        subject: row.get("subject")?,
        snippet: row.get("snippet")?,
        last_message_at: row.get("last_message_at")?,
        message_count: row.get("message_count")?,
        is_read: row.get::<_, i64>("is_read")? != 0,
        is_starred: row.get::<_, i64>("is_starred")? != 0,
        has_attachments: row.get::<_, i64>("has_attachments")? != 0,
        from_name: row.get("from_name")?,
        from_address: row.get("from_address")?,
    })
}
