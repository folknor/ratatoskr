use std::collections::HashSet;

use ratatoskr_db::db::DbState;
use rusqlite::Connection;

/// Synchronous version: check which thread IDs have pending local operations.
pub fn get_blocked_thread_ids(
    conn: &Connection,
    account_id: &str,
    thread_ids: &[String],
) -> Result<HashSet<String>, String> {
    let mut blocked = HashSet::new();
    for tid in thread_ids {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) AS cnt FROM pending_operations \
                 WHERE account_id = ?1 AND resource_id = ?2 \
                 AND status != 'failed'",
                rusqlite::params![account_id, tid],
                |row| row.get("cnt"),
            )
            .unwrap_or(0);
        if count > 0 {
            blocked.insert(tid.clone());
        }
    }
    Ok(blocked)
}

/// Async version: check which thread IDs have pending local operations.
pub async fn blocked_thread_ids(
    db: &DbState,
    account_id: &str,
    thread_ids: Vec<String>,
) -> Result<HashSet<String>, String> {
    if thread_ids.is_empty() {
        return Ok(HashSet::new());
    }

    let aid = account_id.to_string();
    db.with_conn(move |conn| get_blocked_thread_ids(conn, &aid, &thread_ids))
        .await
}

pub fn filter_by_blocked_threads<T, F>(
    items: Vec<T>,
    blocked_threads: &HashSet<String>,
    thread_id_of: F,
) -> Vec<T>
where
    F: Fn(&T) -> &str,
{
    items
        .into_iter()
        .filter(|item| !blocked_threads.contains(thread_id_of(item)))
        .collect()
}
