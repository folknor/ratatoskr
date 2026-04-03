use rusqlite::{Connection, params};

use crate::accounts::Account;

struct SeededPinnedSearch {
    query: String,
    updated_at: i64,
    scope_account_id: Option<String>,
    thread_ids: Vec<(String, String)>,
}

pub fn seed_pinned_searches(conn: &Connection, accounts: &[Account]) -> Result<(), String> {
    ensure_pinned_search_tables(conn)?;
    let seeded_now = chrono::Utc::now().timestamp();

    let mut searches = Vec::new();

    let unread = load_unread_snapshot(conn, 12)?;
    if !unread.is_empty() {
        searches.push(SeededPinnedSearch {
            query: "is:unread".to_string(),
            updated_at: seeded_now - 2 * 3600,
            scope_account_id: None,
            thread_ids: unread,
        });
    }

    let attachments = load_attachment_snapshot(conn, 10)?;
    if !attachments.is_empty() {
        searches.push(SeededPinnedSearch {
            query: "has:attachment".to_string(),
            updated_at: seeded_now - 26 * 3600,
            scope_account_id: None,
            thread_ids: attachments,
        });
    }

    if let Some(account) = accounts.get(1) {
        let inbox = load_account_inbox_snapshot(conn, &account.id, 10)?;
        if !inbox.is_empty() {
            searches.push(SeededPinnedSearch {
                query: format!("account:\"{}\" in:inbox", account.email),
                updated_at: seeded_now - 5 * 24 * 3600,
                scope_account_id: Some(account.id.clone()),
                thread_ids: inbox,
            });
        }
    }

    for search in searches {
        insert_pinned_search(conn, &search)?;
    }

    Ok(())
}

fn ensure_pinned_search_tables(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS pinned_searches (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             query TEXT NOT NULL,
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             scope_account_id TEXT
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
    .map_err(|e| format!("create pinned search tables: {e}"))
}

fn load_unread_snapshot(conn: &Connection, limit: i64) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT t.id, t.account_id
             FROM threads t
             WHERE t.is_read = 0
               AND t.shared_mailbox_id IS NULL
               AND t.is_chat_thread = 0
             ORDER BY t.last_message_at DESC
             LIMIT ?1",
        )
        .map_err(|e| format!("prepare unread snapshot query: {e}"))?;

    let rows = stmt
        .query_map(params![limit], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("query unread snapshot: {e}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect unread snapshot: {e}"))
}

fn load_attachment_snapshot(
    conn: &Connection,
    limit: i64,
) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT t.id, t.account_id
             FROM threads t
             WHERE t.has_attachments = 1
               AND t.shared_mailbox_id IS NULL
               AND t.is_chat_thread = 0
             ORDER BY t.last_message_at DESC
             LIMIT ?1",
        )
        .map_err(|e| format!("prepare attachment snapshot query: {e}"))?;

    let rows = stmt
        .query_map(params![limit], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("query attachment snapshot: {e}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect attachment snapshot: {e}"))
}

fn load_account_inbox_snapshot(
    conn: &Connection,
    account_id: &str,
    limit: i64,
) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT t.id, t.account_id
             FROM threads t
             INNER JOIN thread_labels tl
               ON tl.account_id = t.account_id AND tl.thread_id = t.id
             WHERE t.account_id = ?1
               AND tl.label_id = 'INBOX'
               AND t.shared_mailbox_id IS NULL
               AND t.is_chat_thread = 0
             ORDER BY t.last_message_at DESC
             LIMIT ?2",
        )
        .map_err(|e| format!("prepare account inbox snapshot query: {e}"))?;

    let rows = stmt
        .query_map(params![account_id, limit], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("query account inbox snapshot: {e}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect account inbox snapshot: {e}"))
}

fn insert_pinned_search(conn: &Connection, search: &SeededPinnedSearch) -> Result<(), String> {
    conn.execute(
        "INSERT INTO pinned_searches (query, created_at, updated_at, scope_account_id)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            search.query,
            search.updated_at,
            search.updated_at,
            search.scope_account_id
        ],
    )
    .map_err(|e| format!("insert pinned search: {e}"))?;

    let pinned_id: i64 = conn
        .query_row(
            "SELECT id FROM pinned_searches WHERE query = ?1",
            params![search.query],
            |row| row.get(0),
        )
        .map_err(|e| format!("fetch pinned search id: {e}"))?;

    conn.execute(
        "DELETE FROM pinned_search_threads WHERE pinned_search_id = ?1",
        params![pinned_id],
    )
    .map_err(|e| format!("clear pinned search snapshot: {e}"))?;

    let mut stmt = conn
        .prepare(
            "INSERT INTO pinned_search_threads (pinned_search_id, thread_id, account_id)
             VALUES (?1, ?2, ?3)",
        )
        .map_err(|e| format!("prepare pinned search snapshot insert: {e}"))?;

    for (thread_id, account_id) in &search.thread_ids {
        stmt.execute(params![pinned_id, thread_id, account_id])
            .map_err(|e| format!("insert pinned search snapshot: {e}"))?;
    }

    Ok(())
}
