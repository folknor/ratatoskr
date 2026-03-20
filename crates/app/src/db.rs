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
    pub account_name: Option<String>,
    pub account_color: Option<String>,
    pub last_sync_at: Option<i64>,
    pub sort_order: i64,
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

// ── Pinned search type ───────────────────────────────────────

/// A pinned search with its stored thread snapshot.
#[derive(Debug, Clone)]
pub struct PinnedSearch {
    pub id: i64,
    pub query: String,
    pub created_at: i64,
    pub updated_at: i64,
}

// ── DB connection ───────────────────────────────────────────

pub struct Db {
    /// Read-only connection for sync data queries.
    conn: Arc<Mutex<Connection>>,
    /// Writable connection for local state (account creation, pinned
    /// searches, session restore, keybinding overrides, etc.).
    /// This is the cross-cutting writable connection pattern from the
    /// implementation plan — multiple features need local-state writes.
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
        // SQLite has no ADD COLUMN IF NOT EXISTS, so ignore errors.
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

    /// Execute a closure on the writable connection. Use this for
    /// account creation, local state writes, and any operation that
    /// modifies the database.
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

    pub async fn get_accounts(&self) -> Result<Vec<Account>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, email, display_name, provider,
                            account_name, account_color, last_sync_at,
                            COALESCE(sort_order, 0) AS sort_order
                     FROM accounts WHERE is_active = 1
                     ORDER BY sort_order ASC, created_at ASC",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map([], |row| {
                Ok(Account {
                    id: row.get("id")?,
                    email: row.get("email")?,
                    display_name: row.get("display_name")?,
                    provider: row.get("provider")?,
                    account_name: row.get("account_name")?,
                    account_color: row.get("account_color")?,
                    last_sync_at: row.get("last_sync_at")?,
                    sort_order: row.get("sort_order")?,
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
                     LIMIT ?3",
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
                     LIMIT ?2",
                    false,
                )
            };

            let mut stmt =
                conn.prepare(sql).map_err(|e| e.to_string())?;

            let rows = if do_label {
                stmt.query_map(
                    params![account_id, label_id.unwrap_or_default(), limit],
                    row_to_thread,
                )
            } else {
                stmt.query_map(
                    params![account_id, limit],
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

// ── Message view data loading ────────────────────────────────

/// Attachment data for a single message in a pop-out view.
#[derive(Debug, Clone)]
pub struct MessageViewAttachment {
    pub id: String,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub size: Option<i64>,
}

impl Db {
    /// Load the body (text + HTML) for a single message.
    ///
    /// In the full app, this would query the body store (bodies.db).
    /// For the prototype, it uses the snippet as a fallback.
    pub async fn load_message_body(
        &self,
        account_id: String,
        message_id: String,
    ) -> Result<(Option<String>, Option<String>), String> {
        self.with_conn(move |conn| {
            let snippet: Option<String> = conn
                .query_row(
                    "SELECT snippet FROM messages
                     WHERE account_id = ?1 AND id = ?2",
                    params![account_id, message_id],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;

            // Return snippet as body_text; body_html is None for now.
            Ok((snippet, None))
        })
        .await
    }

    /// Load attachments for a single message.
    pub async fn load_message_attachments(
        &self,
        account_id: String,
        message_id: String,
    ) -> Result<Vec<MessageViewAttachment>, String> {
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, filename, mime_type, size
                     FROM attachments
                     WHERE account_id = ?1 AND message_id = ?2
                       AND is_inline = 0
                       AND filename IS NOT NULL AND filename != ''
                     ORDER BY filename ASC",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map(params![account_id, message_id], |row| {
                Ok(MessageViewAttachment {
                    id: row.get("id")?,
                    filename: row.get("filename")?,
                    mime_type: row.get("mime_type")?,
                    size: row.get("size")?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
    }
}

// ── Pinned search CRUD ───────────────────────────────────────

impl Db {
    /// Creates a pinned search, or updates the existing one if
    /// `query` already exists. Returns the pinned search ID.
    pub async fn create_or_update_pinned_search(
        &self,
        query: String,
        thread_ids: Vec<(String, String)>,
    ) -> Result<i64, String> {
        self.with_write_conn(move |conn| {
            let now = chrono::Utc::now().timestamp();

            let existing_id: Option<i64> = conn
                .query_row(
                    "SELECT id FROM pinned_searches WHERE query = ?1",
                    params![query],
                    |row| row.get(0),
                )
                .ok();

            let pinned_id = if let Some(id) = existing_id {
                conn.execute(
                    "UPDATE pinned_searches SET updated_at = ?1 WHERE id = ?2",
                    params![now, id],
                )
                .map_err(|e| e.to_string())?;
                id
            } else {
                conn.execute(
                    "INSERT INTO pinned_searches (query, created_at, updated_at)
                     VALUES (?1, ?2, ?2)",
                    params![query, now],
                )
                .map_err(|e| e.to_string())?;
                conn.last_insert_rowid()
            };

            // Replace thread snapshot
            conn.execute(
                "DELETE FROM pinned_search_threads WHERE pinned_search_id = ?1",
                params![pinned_id],
            )
            .map_err(|e| e.to_string())?;

            let mut stmt = conn
                .prepare(
                    "INSERT INTO pinned_search_threads
                        (pinned_search_id, thread_id, account_id)
                     VALUES (?1, ?2, ?3)",
                )
                .map_err(|e| e.to_string())?;

            for (thread_id, account_id) in &thread_ids {
                stmt.execute(params![pinned_id, thread_id, account_id])
                    .map_err(|e| e.to_string())?;
            }

            Ok(pinned_id)
        })
        .await
    }

    /// Updates a pinned search's query string and thread snapshot.
    /// If the new query conflicts with another pinned search, the
    /// conflicting row is deleted (merge behavior).
    pub async fn update_pinned_search(
        &self,
        id: i64,
        query: String,
        thread_ids: Vec<(String, String)>,
    ) -> Result<(), String> {
        self.with_write_conn(move |conn| {
            let now = chrono::Utc::now().timestamp();

            // Check for a different pinned search with this query
            let conflict_id: Option<i64> = conn
                .query_row(
                    "SELECT id FROM pinned_searches WHERE query = ?1 AND id != ?2",
                    params![query, id],
                    |row| row.get(0),
                )
                .ok();
            if let Some(cid) = conflict_id {
                conn.execute(
                    "DELETE FROM pinned_searches WHERE id = ?1",
                    params![cid],
                )
                .map_err(|e| e.to_string())?;
            }

            conn.execute(
                "UPDATE pinned_searches
                 SET query = ?1, updated_at = ?2
                 WHERE id = ?3",
                params![query, now, id],
            )
            .map_err(|e| e.to_string())?;

            conn.execute(
                "DELETE FROM pinned_search_threads WHERE pinned_search_id = ?1",
                params![id],
            )
            .map_err(|e| e.to_string())?;

            let mut stmt = conn
                .prepare(
                    "INSERT INTO pinned_search_threads
                        (pinned_search_id, thread_id, account_id)
                     VALUES (?1, ?2, ?3)",
                )
                .map_err(|e| e.to_string())?;

            for (thread_id, account_id) in &thread_ids {
                stmt.execute(params![id, thread_id, account_id])
                    .map_err(|e| e.to_string())?;
            }

            Ok(())
        })
        .await
    }

    /// Deletes a pinned search. CASCADE handles thread cleanup.
    pub async fn delete_pinned_search(&self, id: i64) -> Result<(), String> {
        self.with_write_conn(move |conn| {
            conn.execute(
                "DELETE FROM pinned_searches WHERE id = ?1",
                params![id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
    }

    /// Returns all pinned searches ordered by most recently updated.
    pub async fn list_pinned_searches(&self) -> Result<Vec<PinnedSearch>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, query, created_at, updated_at
                     FROM pinned_searches
                     ORDER BY updated_at DESC",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map([], |row| {
                Ok(PinnedSearch {
                    id: row.get("id")?,
                    query: row.get("query")?,
                    created_at: row.get("created_at")?,
                    updated_at: row.get("updated_at")?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
    }

    /// Loads the thread ID snapshot for a specific pinned search.
    pub async fn get_pinned_search_thread_ids(
        &self,
        pinned_search_id: i64,
    ) -> Result<Vec<(String, String)>, String> {
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT thread_id, account_id
                     FROM pinned_search_threads
                     WHERE pinned_search_id = ?1",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map(params![pinned_search_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
    }

    /// Fetches threads by (thread_id, account_id) pairs with current
    /// metadata. Threads that no longer exist are silently omitted.
    pub async fn get_threads_by_ids(
        &self,
        ids: Vec<(String, String)>,
    ) -> Result<Vec<Thread>, String> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        self.with_conn(move |conn| {
            let chunk_size = 400; // 2 params per ID, stay under 999
            let mut results = Vec::with_capacity(ids.len());

            for chunk in ids.chunks(chunk_size) {
                let placeholders: Vec<String> = chunk
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        let p1 = i * 2 + 1;
                        let p2 = i * 2 + 2;
                        format!("(?{p1}, ?{p2})")
                    })
                    .collect();
                let values_clause = placeholders.join(", ");

                let sql = format!(
                    "WITH target_ids(tid, aid) AS (VALUES {values_clause})
                     SELECT t.*, m.from_name, m.from_address
                     FROM target_ids ti
                     JOIN threads t ON t.id = ti.tid
                         AND t.account_id = ti.aid
                     LEFT JOIN messages m
                         ON m.account_id = t.account_id
                         AND m.thread_id = t.id
                         AND m.date = (
                             SELECT MAX(m2.date) FROM messages m2
                             WHERE m2.account_id = t.account_id
                               AND m2.thread_id = t.id
                         )
                     GROUP BY t.account_id, t.id
                     ORDER BY t.last_message_at DESC"
                );

                let mut stmt =
                    conn.prepare(&sql).map_err(|e| e.to_string())?;

                let param_values: Vec<Box<dyn rusqlite::types::ToSql>> = chunk
                    .iter()
                    .flat_map(|(tid, aid)| {
                        vec![
                            Box::new(tid.clone()) as Box<dyn rusqlite::types::ToSql>,
                            Box::new(aid.clone()) as Box<dyn rusqlite::types::ToSql>,
                        ]
                    })
                    .collect();

                let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                    param_values.iter().map(|p| p.as_ref()).collect();

                let rows = stmt
                    .query_map(param_refs.as_slice(), row_to_thread)
                    .map_err(|e| e.to_string())?;

                for row in rows {
                    results.push(row.map_err(|e| e.to_string())?);
                }
            }

            Ok(results)
        })
        .await
    }

    /// Removes pinned searches older than `max_age_secs` that haven't
    /// been accessed (updated_at == created_at).
    pub async fn expire_stale_pinned_searches(
        &self,
        max_age_secs: i64,
    ) -> Result<u64, String> {
        self.with_write_conn(move |conn| {
            let cutoff = chrono::Utc::now().timestamp() - max_age_secs;
            let deleted = conn
                .execute(
                    "DELETE FROM pinned_searches
                     WHERE updated_at < ?1
                       AND updated_at = created_at",
                    params![cutoff],
                )
                .map_err(|e| e.to_string())?;
            #[allow(clippy::cast_sign_loss)]
            Ok(deleted as u64)
        })
        .await
    }
}

// ── Palette query methods ────────────────────────────────────

impl Db {
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

    /// User-visible folders/labels for an account, excluding system labels.
    ///
    /// For Gmail, splits `/`-delimited labels into path segments.
    /// Returns `OptionItem`s for the palette's ListPicker stage 2.
    pub fn get_user_folders_for_palette(
        &self,
        account_id: &str,
    ) -> Result<Vec<ratatoskr_command_palette::OptionItem>, String> {
        self.with_conn_sync(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, name FROM labels
                     WHERE account_id = ?1 AND type != 'system' AND visible = 1
                     ORDER BY sort_order ASC, name ASC",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map(params![account_id], |row| {
                let id: String = row.get("id")?;
                let name: String = row.get("name")?;
                Ok((id, name))
            })
            .map_err(|e| e.to_string())?
            .map(|r| {
                let (id, name) = r.map_err(|e| e.to_string())?;
                Ok(label_name_to_option_item(id, &name))
            })
            .collect::<Result<Vec<_>, String>>()
        })
    }

    /// All user labels for an account (same as folders for now).
    pub fn get_user_labels_for_palette(
        &self,
        account_id: &str,
    ) -> Result<Vec<ratatoskr_command_palette::OptionItem>, String> {
        self.get_user_folders_for_palette(account_id)
    }

    /// Labels currently applied to a specific thread.
    pub fn get_thread_labels_for_palette(
        &self,
        account_id: &str,
        thread_id: &str,
    ) -> Result<Vec<ratatoskr_command_palette::OptionItem>, String> {
        self.with_conn_sync(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT l.id, l.name FROM labels l
                     INNER JOIN thread_labels tl
                       ON tl.account_id = l.account_id AND tl.label_id = l.id
                     WHERE tl.account_id = ?1 AND tl.thread_id = ?2
                       AND l.type != 'system' AND l.visible = 1
                     ORDER BY l.sort_order ASC, l.name ASC",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map(params![account_id, thread_id], |row| {
                let id: String = row.get("id")?;
                let name: String = row.get("name")?;
                Ok((id, name))
            })
            .map_err(|e| e.to_string())?
            .map(|r| {
                let (id, name) = r.map_err(|e| e.to_string())?;
                Ok(label_name_to_option_item(id, &name))
            })
            .collect::<Result<Vec<_>, String>>()
        })
    }

    /// All user labels across all accounts, with account name in path.
    ///
    /// Each `OptionItem.id` is encoded as `"account_id:label_id"` so
    /// the palette can split them when building `CommandArgs`.
    pub fn get_all_labels_cross_account(
        &self,
    ) -> Result<Vec<ratatoskr_command_palette::OptionItem>, String> {
        self.with_conn_sync(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT a.id AS account_id,
                            COALESCE(a.display_name, a.email) AS account_name,
                            l.id AS label_id,
                            l.name AS label_name
                     FROM labels l
                     INNER JOIN accounts a ON a.id = l.account_id
                     WHERE l.type != 'system' AND l.visible = 1 AND a.is_active = 1
                     ORDER BY a.email ASC, l.sort_order ASC, l.name ASC",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map([], |row| {
                let account_id: String = row.get("account_id")?;
                let account_name: String = row.get("account_name")?;
                let label_id: String = row.get("label_id")?;
                let label_name: String = row.get("label_name")?;
                Ok((account_id, account_name, label_id, label_name))
            })
            .map_err(|e| e.to_string())?
            .map(|r| {
                let (account_id, account_name, label_id, label_name) =
                    r.map_err(|e| e.to_string())?;
                let mut item = label_name_to_option_item(label_id.clone(), &label_name);
                // Prefix path with account name
                let mut new_path = vec![account_name];
                if let Some(existing) = item.path.take() {
                    new_path.extend(existing);
                }
                item.path = Some(new_path);
                // Encode account_id into item.id for disambiguation
                item.id = format!("{account_id}:{label_id}");
                Ok(item)
            })
            .collect::<Result<Vec<_>, String>>()
        })
    }

    /// Check whether an account uses folder-based semantics (Exchange/IMAP/JMAP)
    /// as opposed to tag-based (Gmail). Folder-based providers don't support
    /// Add Label / Remove Label — only Move to Folder.
    pub fn is_folder_based_provider(
        &self,
        account_id: &str,
    ) -> Result<bool, String> {
        self.with_conn_sync(|conn| {
            let provider: String = conn
                .query_row(
                    "SELECT provider FROM accounts WHERE id = ?1",
                    params![account_id],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            Ok(provider != "gmail_api")
        })
    }
}

/// Convert a label name to an `OptionItem`, splitting `/`-delimited names
/// into path segments (Gmail convention).
fn label_name_to_option_item(
    id: String,
    name: &str,
) -> ratatoskr_command_palette::OptionItem {
    let segments: Vec<&str> = name.split('/').collect();
    let (label, path) = if segments.len() > 1 {
        let label = segments.last().unwrap_or(&name).to_string();
        let path: Vec<String> = segments[..segments.len() - 1]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        (label, Some(path))
    } else {
        (name.to_string(), None)
    };

    ratatoskr_command_palette::OptionItem {
        id,
        label,
        path,
        keywords: None,
        disabled: false,
    }
}

// ── Contact search types ─────────────────────────────────────

/// A contact result from the autocomplete search.
#[derive(Debug, Clone)]
pub struct ContactMatch {
    pub email: String,
    pub display_name: Option<String>,
}

/// Search contacts and seen addresses for autocomplete.
///
/// Searches the `contacts` table and `seen_addresses` table using LIKE
/// matching. Deduplicates by email (contacts take priority over seen
/// addresses). Returns up to `limit` results ordered by relevance.
pub fn search_contacts_for_autocomplete(
    conn: &Connection,
    query: &str,
    limit: i64,
) -> Result<Vec<ContactMatch>, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let pattern = format!("%{trimmed}%");
    let mut results = Vec::new();
    let mut seen_emails: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Search contacts table first (higher priority).
    // Order by frequency DESC so frequently-contacted people rank higher,
    // matching the product spec's "recency dominates" ranking model.
    let contacts_sql = "SELECT email, display_name FROM contacts
                        WHERE email LIKE ?1 OR display_name LIKE ?1
                        ORDER BY frequency DESC, display_name ASC
                        LIMIT ?2";
    let mut stmt = conn.prepare(contacts_sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![&pattern, limit], |row| {
            Ok(ContactMatch {
                email: row.get("email")?,
                display_name: row.get("display_name")?,
            })
        })
        .map_err(|e| e.to_string())?;
    for row in rows {
        let contact = row.map_err(|e| e.to_string())?;
        let key = contact.email.to_lowercase();
        if seen_emails.insert(key) {
            results.push(contact);
        }
    }

    // Search seen_addresses table (lower priority, fills remaining slots).
    // Order by last_seen_at DESC for recency.
    let remaining = limit - results.len() as i64;
    if remaining > 0 {
        let seen_sql = "SELECT email, display_name FROM seen_addresses
                        WHERE email LIKE ?1 OR display_name LIKE ?1
                        ORDER BY last_seen_at DESC
                        LIMIT ?2";
        let mut seen_stmt = conn.prepare(seen_sql).map_err(|e| e.to_string())?;
        let seen_rows = seen_stmt
            .query_map(params![&pattern, remaining], |row| {
                Ok(ContactMatch {
                    email: row.get("email")?,
                    display_name: row.get("display_name")?,
                })
            })
            .map_err(|e| e.to_string())?;
        for row in seen_rows {
            let contact = row.map_err(|e| e.to_string())?;
            let key = contact.email.to_lowercase();
            if seen_emails.insert(key) {
                results.push(contact);
            }
        }
    }

    Ok(results)
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

// ── Contact management types ─────────────────────────────────

/// A contact entry for the settings management UI.
#[derive(Debug, Clone)]
pub struct ContactEntry {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub email2: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub notes: Option<String>,
    pub account_id: Option<String>,
    pub account_color: Option<String>,
    pub groups: Vec<String>,
}

/// A contact group entry for the settings management UI.
#[derive(Debug, Clone)]
pub struct GroupEntry {
    pub id: String,
    pub name: String,
    pub member_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

// ── Contact management CRUD ──────────────────────────────────

impl Db {
    /// Load contacts for the settings management list, optionally filtered.
    pub async fn get_contacts_for_settings(
        &self,
        filter: String,
    ) -> Result<Vec<ContactEntry>, String> {
        self.with_conn(move |conn| {
            load_contacts_filtered(conn, &filter)
        })
        .await
    }

    /// Load contact groups for the settings management list.
    pub async fn get_groups_for_settings(
        &self,
        filter: String,
    ) -> Result<Vec<GroupEntry>, String> {
        self.with_conn(move |conn| {
            load_groups_filtered(conn, &filter)
        })
        .await
    }

    /// Get member emails for a group.
    pub async fn get_group_member_emails(
        &self,
        group_id: String,
    ) -> Result<Vec<String>, String> {
        self.with_conn(move |conn| {
            load_group_member_emails(conn, &group_id)
        })
        .await
    }

    /// Insert or update a contact.
    pub async fn save_contact(
        &self,
        entry: ContactEntry,
    ) -> Result<(), String> {
        self.with_write_conn(move |conn| {
            save_contact_inner(conn, &entry)
        })
        .await
    }

    /// Delete a contact by ID.
    pub async fn delete_contact(
        &self,
        contact_id: String,
    ) -> Result<(), String> {
        self.with_write_conn(move |conn| {
            conn.execute(
                "DELETE FROM contacts WHERE id = ?1",
                params![contact_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
    }

    /// Insert or update a contact group.
    pub async fn save_group(
        &self,
        group: GroupEntry,
        member_emails: Vec<String>,
    ) -> Result<(), String> {
        self.with_write_conn(move |conn| {
            save_group_inner(conn, &group, &member_emails)
        })
        .await
    }

    /// Delete a contact group by ID.
    pub async fn delete_group(
        &self,
        group_id: String,
    ) -> Result<(), String> {
        self.with_write_conn(move |conn| {
            conn.execute(
                "DELETE FROM contact_groups WHERE id = ?1",
                params![group_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
    }
}

fn load_contacts_filtered(
    conn: &Connection,
    filter: &str,
) -> Result<Vec<ContactEntry>, String> {
    let trimmed = filter.trim();
    let pattern = format!("%{trimmed}%");

    // Always pass the pattern param; when no filter is active the WHERE clause
    // is trivially true (empty pattern = '%') so the param is harmless.
    let sql = if trimmed.is_empty() {
        "SELECT c.id, c.email, c.display_name, c.email2, c.phone,
                c.company, c.notes, c.account_id,
                a.account_color
         FROM contacts c
         LEFT JOIN accounts a ON a.id = c.account_id
         WHERE c.source != 'seen'
         ORDER BY c.frequency DESC, c.display_name ASC
         LIMIT 200"
    } else {
        "SELECT c.id, c.email, c.display_name, c.email2, c.phone,
                c.company, c.notes, c.account_id,
                a.account_color
         FROM contacts c
         LEFT JOIN accounts a ON a.id = c.account_id
         WHERE c.source != 'seen'
           AND (c.email LIKE ?1
                OR c.display_name LIKE ?1
                OR c.company LIKE ?1)
         ORDER BY c.frequency DESC, c.display_name ASC
         LIMIT 200"
    };

    let params: &[&dyn rusqlite::types::ToSql] = if trimmed.is_empty() {
        &[]
    } else {
        &[&pattern]
    };

    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params, |row| {
            Ok(ContactEntry {
                id: row.get("id")?,
                email: row.get("email")?,
                display_name: row.get("display_name")?,
                email2: row.get("email2")?,
                phone: row.get("phone")?,
                company: row.get("company")?,
                notes: row.get("notes")?,
                account_id: row.get("account_id")?,
                account_color: row.get("account_color")?,
                groups: Vec::new(),
            })
        })
        .map_err(|e| e.to_string())?;

    let mut contacts: Vec<ContactEntry> = Vec::new();
    for row in rows {
        contacts.push(row.map_err(|e| e.to_string())?);
    }

    // Load group memberships for each contact.
    for contact in &mut contacts {
        contact.groups = load_contact_groups(conn, &contact.email)?;
    }
    Ok(contacts)
}

fn load_contact_groups(
    conn: &Connection,
    email: &str,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT g.name FROM contact_groups g
             INNER JOIN contact_group_members m
               ON m.group_id = g.id
             WHERE m.member_type = 'email' AND m.member_value = ?1
             ORDER BY g.name ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![email], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn load_groups_filtered(
    conn: &Connection,
    filter: &str,
) -> Result<Vec<GroupEntry>, String> {
    let trimmed = filter.trim();
    let pattern = format!("%{trimmed}%");

    let sql = if trimmed.is_empty() {
        "SELECT g.id, g.name, g.created_at, g.updated_at,
                (SELECT COUNT(*) FROM contact_group_members m
                 WHERE m.group_id = g.id) AS member_count
         FROM contact_groups g
         ORDER BY g.updated_at DESC
         LIMIT 100"
    } else {
        "SELECT g.id, g.name, g.created_at, g.updated_at,
                (SELECT COUNT(*) FROM contact_group_members m
                 WHERE m.group_id = g.id) AS member_count
         FROM contact_groups g
         WHERE g.name LIKE ?1
         ORDER BY g.updated_at DESC
         LIMIT 100"
    };

    let params: &[&dyn rusqlite::types::ToSql] = if trimmed.is_empty() {
        &[]
    } else {
        &[&pattern]
    };

    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params, |row| {
            Ok(GroupEntry {
                id: row.get("id")?,
                name: row.get("name")?,
                member_count: row.get("member_count")?,
                created_at: row.get("created_at")?,
                updated_at: row.get("updated_at")?,
            })
        })
        .map_err(|e| e.to_string())?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn load_group_member_emails(
    conn: &Connection,
    group_id: &str,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT member_value FROM contact_group_members
             WHERE group_id = ?1 AND member_type = 'email'
             ORDER BY member_value ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![group_id], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn save_contact_inner(
    conn: &Connection,
    entry: &ContactEntry,
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO contacts (id, email, display_name, email2, phone,
                               company, notes, account_id, source,
                               created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'user', ?9, ?9)
         ON CONFLICT(id) DO UPDATE SET
             email = excluded.email,
             display_name = excluded.display_name,
             email2 = excluded.email2,
             phone = excluded.phone,
             company = excluded.company,
             notes = excluded.notes,
             account_id = excluded.account_id,
             updated_at = excluded.updated_at",
        params![
            entry.id,
            entry.email,
            entry.display_name,
            entry.email2,
            entry.phone,
            entry.company,
            entry.notes,
            entry.account_id,
            now,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn save_group_inner(
    conn: &Connection,
    group: &GroupEntry,
    member_emails: &[String],
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO contact_groups (id, name, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?3)
         ON CONFLICT(id) DO UPDATE SET
             name = excluded.name,
             updated_at = excluded.updated_at",
        params![group.id, group.name, now],
    )
    .map_err(|e| e.to_string())?;

    // Replace all members
    conn.execute(
        "DELETE FROM contact_group_members WHERE group_id = ?1",
        params![group.id],
    )
    .map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare(
            "INSERT INTO contact_group_members (group_id, member_type, member_value)
             VALUES (?1, 'email', ?2)",
        )
        .map_err(|e| e.to_string())?;

    for email in member_emails {
        stmt.execute(params![group.id, email])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
