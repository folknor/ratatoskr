use rusqlite::Connection;

use super::types::AccountDeletionData;

/// Gather all data needed for account deletion cleanup (message IDs, cached
/// file paths, inline image hashes) in a single DB pass.
pub fn gather_deletion_data(
    conn: &Connection,
    account_id: &str,
) -> Result<AccountDeletionData, String> {
    let message_ids = {
        let mut stmt = conn
            .prepare("SELECT id FROM messages WHERE account_id = ?1")
            .map_err(|e| format!("prepare account message query: {e}"))?;
        stmt.query_map(rusqlite::params![account_id], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|e| format!("query account message ids: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect account message ids: {e}"))?
    };

    let cached_files = {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT local_path, content_hash
                 FROM attachments
                 WHERE account_id = ?1
                   AND cached_at IS NOT NULL
                   AND local_path IS NOT NULL
                   AND content_hash IS NOT NULL",
            )
            .map_err(|e| format!("prepare account cached attachment query: {e}"))?;
        stmt.query_map(rusqlite::params![account_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("query account cached attachments: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect account cached attachments: {e}"))?
    };

    let inline_hashes = {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT content_hash
                 FROM attachments
                 WHERE account_id = ?1
                   AND is_inline = 1
                   AND content_hash IS NOT NULL",
            )
            .map_err(|e| format!("prepare account inline hash query: {e}"))?;
        stmt.query_map(rusqlite::params![account_id], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|e| format!("query account inline hashes: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect account inline hashes: {e}"))?
    };

    Ok(AccountDeletionData {
        message_ids,
        cached_files,
        inline_hashes,
    })
}

/// Count remaining cached attachment references for a given content hash.
pub fn count_cached_refs(conn: &Connection, content_hash: &str) -> Result<i64, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM attachments
         WHERE content_hash = ?1 AND cached_at IS NOT NULL",
        rusqlite::params![content_hash],
        |row| row.get(0),
    )
    .map_err(|e| format!("count remaining cached attachment refs: {e}"))
}

/// Delete the account row from the database.
pub fn delete_account_row(conn: &Connection, account_id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM accounts WHERE id = ?1",
        rusqlite::params![account_id],
    )
    .map_err(|e| format!("delete account: {e}"))?;
    Ok(())
}
