use rusqlite::{Connection, params};

use super::from_row::FromRow;
use super::sql_fragments::LATEST_MESSAGE_SUBQUERY;
use super::types::ThreadInfoRow;

pub fn load_recent_rule_categorized_threads(
    conn: &Connection,
    account_id: &str,
    limit: i64,
) -> Result<Vec<ThreadInfoRow>, String> {
    let sql = format!(
        "SELECT t.id, t.subject, t.snippet, m.from_address
         FROM threads t
         INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
         INNER JOIN thread_categories tc ON tc.account_id = t.account_id AND tc.thread_id = t.id
         LEFT JOIN ({LATEST_MESSAGE_SUBQUERY}
         ) m ON m.account_id = t.account_id AND m.thread_id = t.id
         WHERE t.account_id = ?1 AND tl.label_id = 'INBOX' AND tc.is_manual = 0
         ORDER BY t.last_message_at DESC
         LIMIT ?2"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    stmt.query_map(params![account_id, limit], ThreadInfoRow::from_row)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}
