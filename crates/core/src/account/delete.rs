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
            row.get::<_, String>("id")
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
            Ok((row.get::<_, String>("local_path")?, row.get::<_, String>("content_hash")?))
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
            row.get::<_, String>("content_hash")
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
        "SELECT COUNT(*) AS cnt FROM attachments
         WHERE content_hash = ?1 AND cached_at IS NOT NULL",
        rusqlite::params![content_hash],
        |row| row.get("cnt"),
    )
    .map_err(|e| format!("count remaining cached attachment refs: {e}"))
}

/// Batch-count remaining cached attachment references for multiple content
/// hashes, excluding a specific account. Returns the set of hashes that
/// still have at least one reference from another account.
pub fn referenced_hashes_excluding_account(
    conn: &Connection,
    content_hashes: &[(String, String)],
    account_id: &str,
) -> Result<std::collections::HashSet<String>, String> {
    use std::collections::HashSet;

    if content_hashes.is_empty() {
        return Ok(HashSet::new());
    }

    // Collect unique hashes
    let unique: HashSet<&str> = content_hashes.iter().map(|(_, h)| h.as_str()).collect();
    let hashes: Vec<&str> = unique.into_iter().collect();

    // Process in batches to stay within SQLite variable limits
    let mut referenced = HashSet::new();
    for chunk in hashes.chunks(500) {
        let placeholders: Vec<String> = (0..chunk.len()).map(|i| format!("?{}", i + 2)).collect();
        let sql = format!(
            "SELECT content_hash FROM attachments \
             WHERE content_hash IN ({}) AND account_id != ?1 AND cached_at IS NOT NULL \
             GROUP BY content_hash",
            placeholders.join(", ")
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| format!("prepare batch ref count: {e}"))?;
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
            vec![Box::new(account_id.to_string())];
        for hash in chunk {
            params.push(Box::new((*hash).to_string()));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(std::convert::AsRef::as_ref).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| row.get::<_, String>("content_hash"))
            .map_err(|e| format!("query batch ref count: {e}"))?;
        for row in rows {
            let hash = row.map_err(|e| format!("read batch ref count row: {e}"))?;
            referenced.insert(hash);
        }
    }
    Ok(referenced)
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
