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

    // Search contacts table first (higher priority)
    let contacts_sql = "SELECT email, display_name FROM contacts
                        WHERE email LIKE ?1 OR display_name LIKE ?1
                        ORDER BY display_name ASC
                        LIMIT ?2";
    if let Ok(mut stmt) = conn.prepare(contacts_sql) {
        if let Ok(rows) = stmt.query_map(params![&pattern, limit], |row| {
            Ok(ContactMatch {
                email: row.get("email")?,
                display_name: row.get("display_name")?,
            })
        }) {
            for row in rows.flatten() {
                let key = row.email.to_lowercase();
                if seen_emails.insert(key) {
                    results.push(row);
                }
            }
        }
    }

    // Search seen_addresses table (lower priority, fills remaining slots)
    let remaining = limit - results.len() as i64;
    if remaining > 0 {
        let seen_sql = "SELECT email, display_name FROM seen_addresses
                        WHERE email LIKE ?1 OR display_name LIKE ?1
                        ORDER BY last_seen_at DESC
                        LIMIT ?2";
        if let Ok(mut stmt) = conn.prepare(seen_sql) {
            if let Ok(rows) = stmt.query_map(params![&pattern, remaining], |row| {
                Ok(ContactMatch {
                    email: row.get("email")?,
                    display_name: row.get("display_name")?,
                })
            }) {
                for row in rows.flatten() {
                    let key = row.email.to_lowercase();
                    if seen_emails.insert(key) {
                        results.push(row);
                    }
                }
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
