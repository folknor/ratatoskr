use rusqlite::{Connection, params};

use super::from_row::FromRow;
use super::sql_fragments::LATEST_MESSAGE_SUBQUERY;
use super::types::ThreadInfoRow;

/// Read a single value from the `settings` table, returning `Ok(None)` when
/// the key does not exist.
pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    let result = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>("value"),
        )
        .ok();
    Ok(result)
}

/// Persist a refreshed access token to the `accounts` table.
///
/// The caller is responsible for encrypting the token before calling this.
pub fn persist_refreshed_token(
    conn: &Connection,
    account_id: &str,
    encrypted_access_token: &str,
    expires_at: i64,
) -> Result<(), String> {
    conn.execute(
        "UPDATE accounts SET access_token = ?1, token_expires_at = ?2, \
         updated_at = unixepoch() WHERE id = ?3",
        params![encrypted_access_token, expires_at, account_id],
    )
    .map_err(|e| format!("Failed to persist refreshed token: {e}"))?;
    Ok(())
}

pub fn load_recent_rule_bundled_threads(
    conn: &Connection,
    account_id: &str,
    limit: i64,
) -> Result<Vec<ThreadInfoRow>, String> {
    let sql = format!(
        "SELECT t.id, t.subject, t.snippet, m.from_address
         FROM threads t
         INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
         INNER JOIN thread_bundles tc ON tc.account_id = t.account_id AND tc.thread_id = t.id
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
