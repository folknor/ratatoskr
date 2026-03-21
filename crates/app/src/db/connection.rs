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
<<<<<<< HEAD
=======

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
                        to_addresses, cc_addresses, date, subject, snippet,
                        is_read, is_starred
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
                    cc_addresses: row.get("cc_addresses")?,
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

    /// Load raw email source for a message (headers + body).
    /// Synthesizes from available fields as a prototype — full implementation
    /// would query the raw message from the body store or provider cache.
    pub async fn load_raw_source(
        &self,
        account_id: String,
        message_id: String,
    ) -> Result<String, String> {
        self.with_conn(move |conn| {
            let result = conn.query_row(
                "SELECT from_address, to_addresses, cc_addresses,
                        subject, date, snippet
                 FROM messages
                 WHERE account_id = ?1 AND id = ?2",
                params![account_id, message_id],
                |row| {
                    let from: Option<String> = row.get("from_address")?;
                    let to: Option<String> = row.get("to_addresses")?;
                    let cc: Option<String> = row.get("cc_addresses")?;
                    let subject: Option<String> = row.get("subject")?;
                    let date: Option<i64> = row.get("date")?;
                    let snippet: Option<String> = row.get("snippet")?;
                    Ok((from, to, cc, subject, date, snippet))
                },
            );
            match result {
                Ok((from, to, cc, subject, date, snippet)) => {
                    let mut source = String::new();
                    if let Some(f) = from {
                        source.push_str(&format!("From: {f}\r\n"));
                    }
                    if let Some(t) = to {
                        source.push_str(&format!("To: {t}\r\n"));
                    }
                    if let Some(c) = cc {
                        source.push_str(&format!("Cc: {c}\r\n"));
                    }
                    if let Some(s) = subject {
                        source.push_str(&format!("Subject: {s}\r\n"));
                    }
                    if let Some(d) = date {
                        source.push_str(&format!("Date: {d}\r\n"));
                    }
                    source.push_str("\r\n");
                    if let Some(body) = snippet {
                        source.push_str(&body);
                    }
                    Ok(source)
                }
                Err(e) => Err(e.to_string()),
            }
        })
        .await
    }

    // ── Calendar event CRUD ────────────────────────────────

    /// Load a single calendar event by its DB id.
    pub async fn get_calendar_event(
        &self,
        event_id: String,
    ) -> Result<Option<CalendarEvent>, String> {
        self.with_conn(move |conn| {
            let result = conn.query_row(
                "SELECT id, summary, description, location,
                        start_time, end_time, is_all_day, calendar_id
                 FROM calendar_events WHERE id = ?1",
                params![event_id],
                |row| {
                    Ok(CalendarEvent {
                        id: row.get("id")?,
                        summary: row.get("summary")?,
                        description: row.get("description")?,
                        location: row.get("location")?,
                        start_time: row.get("start_time")?,
                        end_time: row.get("end_time")?,
                        is_all_day: row.get::<_, i64>("is_all_day")? != 0,
                        calendar_id: row.get("calendar_id")?,
                    })
                },
            );
            match result {
                Ok(event) => Ok(Some(event)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e.to_string()),
            }
        })
        .await
    }

    /// Create a new calendar event. Returns the new event's id.
    /// `account_id` must be a real account ID (not empty).
    #[allow(clippy::too_many_arguments)]
    pub async fn create_calendar_event(
        &self,
        account_id: String,
        title: String,
        description: String,
        location: String,
        start_time: i64,
        end_time: i64,
        is_all_day: bool,
        calendar_id: Option<String>,
    ) -> Result<String, String> {
        self.with_write_conn(move |conn| {
            let id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO calendar_events
                    (id, account_id, google_event_id, summary, description,
                     location, start_time, end_time, is_all_day, status,
                     calendar_id)
                 VALUES (?1, ?2, ?1, ?3, ?4, ?5, ?6, ?7, ?8, 'confirmed', ?9)",
                params![
                    id,
                    account_id,
                    title,
                    description,
                    location,
                    start_time,
                    end_time,
                    is_all_day as i64,
                    calendar_id,
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(id)
        })
        .await
    }

    /// Update an existing calendar event.
    #[allow(clippy::too_many_arguments)]
    pub async fn update_calendar_event(
        &self,
        event_id: String,
        title: String,
        description: String,
        location: String,
        start_time: i64,
        end_time: i64,
        is_all_day: bool,
        calendar_id: Option<String>,
    ) -> Result<(), String> {
        self.with_write_conn(move |conn| {
            conn.execute(
                "UPDATE calendar_events SET
                    summary = ?2, description = ?3, location = ?4,
                    start_time = ?5, end_time = ?6, is_all_day = ?7,
                    calendar_id = ?8, updated_at = unixepoch()
                 WHERE id = ?1",
                params![
                    event_id,
                    title,
                    description,
                    location,
                    start_time,
                    end_time,
                    is_all_day as i64,
                    calendar_id,
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
    }

    /// Load all calendar events as TimeGridEvent for view rendering.
    pub async fn load_calendar_events_for_view(
        &self,
    ) -> Result<Vec<crate::ui::calendar_time_grid::TimeGridEvent>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT e.id, e.summary, e.start_time, e.end_time,
                            e.is_all_day, COALESCE(c.color, '#3498db') AS color,
                            c.display_name AS calendar_name
                     FROM calendar_events e
                     LEFT JOIN calendars c
                       ON c.account_id = e.account_id AND c.id = e.calendar_id
                     ORDER BY e.start_time ASC",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(crate::ui::calendar_time_grid::TimeGridEvent {
                        id: row.get::<_, String>("id")?,
                        title: row.get::<_, Option<String>>("summary")?
                            .unwrap_or_default(),
                        start_time: row.get("start_time")?,
                        end_time: row.get("end_time")?,
                        all_day: row.get::<_, i64>("is_all_day")? != 0,
                        color: row.get::<_, Option<String>>("color")?
                            .unwrap_or_else(|| "#3498db".to_string()),
                        calendar_name: row.get("calendar_name")?,
                    })
                })
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .await
    }

    /// Delete a calendar event by id.
    pub async fn delete_calendar_event(
        &self,
        event_id: String,
    ) -> Result<(), String> {
        self.with_write_conn(move |conn| {
            conn.execute(
                "DELETE FROM calendar_events WHERE id = ?1",
                params![event_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
    }
>>>>>>> worktree-agent-a7996d65
}
