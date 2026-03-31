use rusqlite::Connection;

use super::types::{AccountDeletionData, AccountDeletionPlan};

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
            Ok((
                row.get::<_, String>("local_path")?,
                row.get::<_, String>("content_hash")?,
            ))
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
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(std::convert::AsRef::as_ref).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                row.get::<_, String>("content_hash")
            })
            .map_err(|e| format!("query batch ref count: {e}"))?;
        for row in rows {
            let hash = row.map_err(|e| format!("read batch ref count row: {e}"))?;
            referenced.insert(hash);
        }
    }
    Ok(referenced)
}

/// Batch-check which inline image hashes are still referenced by other
/// accounts. Returns the set of hashes that have at least one inline
/// reference from a different account (regardless of cache state).
///
/// Must be called **before** `delete_account_row` so the target account's
/// attachment rows still exist for the exclusion filter.
pub fn inline_hashes_referenced_by_other_accounts(
    conn: &Connection,
    inline_hashes: &[String],
    account_id: &str,
) -> Result<std::collections::HashSet<String>, String> {
    use std::collections::HashSet;

    if inline_hashes.is_empty() {
        return Ok(HashSet::new());
    }

    let mut referenced = HashSet::new();
    for chunk in inline_hashes.chunks(500) {
        let placeholders: Vec<String> = (0..chunk.len()).map(|i| format!("?{}", i + 2)).collect();
        let sql = format!(
            "SELECT content_hash FROM attachments \
             WHERE content_hash IN ({}) AND account_id != ?1 AND is_inline = 1 \
             GROUP BY content_hash",
            placeholders.join(", ")
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| format!("prepare inline ref check: {e}"))?;
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
            vec![Box::new(account_id.to_string())];
        for hash in chunk {
            params.push(Box::new(hash.clone()));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(std::convert::AsRef::as_ref).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                row.get::<_, String>("content_hash")
            })
            .map_err(|e| format!("query inline ref check: {e}"))?;
        for row in rows {
            let hash = row.map_err(|e| format!("read inline ref check row: {e}"))?;
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

/// Synchronous phase of full account deletion: gather cleanup data,
/// determine shared references, then delete the account row (which
/// CASCADE-deletes messages, attachments, etc. from the main DB).
///
/// Returns an [`AccountDeletionPlan`] containing everything needed for
/// the subsequent async cleanup of external stores.
///
/// Must run inside a single connection call so the gather queries see
/// the attachment rows before CASCADE removes them.
pub fn delete_account_orchestrate(
    conn: &Connection,
    account_id: &str,
) -> Result<AccountDeletionPlan, String> {
    let data = gather_deletion_data(conn, account_id)?;
    let shared_cache_hashes =
        referenced_hashes_excluding_account(conn, &data.cached_files, account_id)?;
    let shared_inline_hashes =
        inline_hashes_referenced_by_other_accounts(conn, &data.inline_hashes, account_id)?;
    delete_account_row(conn, account_id)?;
    Ok(AccountDeletionPlan {
        data,
        shared_cache_hashes,
        shared_inline_hashes,
    })
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use rusqlite::{Connection, params};

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("enable FK");
        db::db::migrations::run_all(&conn).expect("migrations");
        conn
    }

    fn insert_account(conn: &Connection, id: &str) {
        conn.execute(
            "INSERT INTO accounts (id, email, provider, is_active) \
             VALUES (?1, ?2, 'gmail_api', 1)",
            params![id, format!("{id}@test.com")],
        )
        .expect("insert account");
    }

    fn insert_thread(conn: &Connection, account_id: &str, thread_id: &str) {
        conn.execute(
            "INSERT INTO threads (id, account_id, subject, last_message_at, message_count) \
             VALUES (?1, ?2, 'test', 1000, 1)",
            params![thread_id, account_id],
        )
        .expect("insert thread");
    }

    fn insert_message(conn: &Connection, account_id: &str, thread_id: &str, msg_id: &str) {
        conn.execute(
            "INSERT INTO messages (id, account_id, thread_id, date) \
             VALUES (?1, ?2, ?3, 1000)",
            params![msg_id, account_id, thread_id],
        )
        .expect("insert message");
    }

    fn insert_attachment(
        conn: &Connection,
        id: &str,
        account_id: &str,
        message_id: &str,
        content_hash: Option<&str>,
        is_inline: bool,
        cached_path: Option<&str>,
    ) {
        conn.execute(
            "INSERT INTO attachments \
             (id, message_id, account_id, content_hash, is_inline, local_path, cached_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, \
                     CASE WHEN ?6 IS NOT NULL THEN 1000 ELSE NULL END)",
            params![
                id,
                message_id,
                account_id,
                content_hash,
                is_inline as i32,
                cached_path
            ],
        )
        .expect("insert attachment");
    }

    #[test]
    fn orchestrate_gathers_before_cascade() {
        let conn = test_db();
        insert_account(&conn, "acct-a");
        insert_thread(&conn, "acct-a", "t1");
        insert_message(&conn, "acct-a", "t1", "m1");
        insert_message(&conn, "acct-a", "t1", "m2");
        insert_attachment(&conn, "att1", "acct-a", "m1", Some("hash1"), true, None);
        insert_attachment(
            &conn,
            "att2",
            "acct-a",
            "m2",
            Some("hash2"),
            false,
            Some("attachment_cache/hash2"),
        );

        let plan = super::delete_account_orchestrate(&conn, "acct-a").expect("orchestrate");

        // Data was gathered before cascade
        assert_eq!(plan.data.message_ids.len(), 2);
        assert!(plan.data.message_ids.contains(&"m1".to_string()));
        assert!(plan.data.message_ids.contains(&"m2".to_string()));
        assert_eq!(plan.data.inline_hashes, vec!["hash1".to_string()]);
        assert_eq!(plan.data.cached_files.len(), 1);
        assert_eq!(plan.data.cached_files[0].1, "hash2");

        // Account row is gone after orchestrate
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM accounts WHERE id = 'acct-a'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);

        // CASCADE cleaned messages + attachments
        let msg_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE account_id = 'acct-a'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(msg_count, 0);
    }

    #[test]
    fn shared_inline_hashes_preserved() {
        let conn = test_db();
        insert_account(&conn, "acct-a");
        insert_account(&conn, "acct-b");
        insert_thread(&conn, "acct-a", "ta1");
        insert_thread(&conn, "acct-b", "tb1");
        insert_message(&conn, "acct-a", "ta1", "ma1");
        insert_message(&conn, "acct-b", "tb1", "mb1");

        // Shared inline hash
        insert_attachment(&conn, "a1", "acct-a", "ma1", Some("shared"), true, None);
        insert_attachment(&conn, "b1", "acct-b", "mb1", Some("shared"), true, None);
        // Account-A-only inline hash
        insert_attachment(&conn, "a2", "acct-a", "ma1", Some("only-a"), true, None);

        let plan = super::delete_account_orchestrate(&conn, "acct-a").expect("orchestrate");

        assert!(plan.shared_inline_hashes.contains("shared"));
        assert!(!plan.shared_inline_hashes.contains("only-a"));
        assert!(plan.data.inline_hashes.contains(&"shared".to_string()));
        assert!(plan.data.inline_hashes.contains(&"only-a".to_string()));
    }

    #[test]
    fn shared_cached_files_preserved() {
        let conn = test_db();
        insert_account(&conn, "acct-a");
        insert_account(&conn, "acct-b");
        insert_thread(&conn, "acct-a", "ta1");
        insert_thread(&conn, "acct-b", "tb1");
        insert_message(&conn, "acct-a", "ta1", "ma1");
        insert_message(&conn, "acct-b", "tb1", "mb1");

        let cache = "attachment_cache/shared-cache";
        insert_attachment(
            &conn,
            "a1",
            "acct-a",
            "ma1",
            Some("shared-cache"),
            false,
            Some(cache),
        );
        insert_attachment(
            &conn,
            "b1",
            "acct-b",
            "mb1",
            Some("shared-cache"),
            false,
            Some(cache),
        );
        insert_attachment(
            &conn,
            "a2",
            "acct-a",
            "ma1",
            Some("only-a"),
            false,
            Some("attachment_cache/only-a"),
        );

        let plan = super::delete_account_orchestrate(&conn, "acct-a").expect("orchestrate");

        assert!(plan.shared_cache_hashes.contains("shared-cache"));
        assert!(!plan.shared_cache_hashes.contains("only-a"));
    }

    #[test]
    fn inline_and_cache_refs_independent() {
        let conn = test_db();
        insert_account(&conn, "acct-a");
        insert_account(&conn, "acct-b");
        insert_thread(&conn, "acct-a", "ta1");
        insert_thread(&conn, "acct-b", "tb1");
        insert_message(&conn, "acct-a", "ta1", "ma1");
        insert_message(&conn, "acct-b", "tb1", "mb1");

        // Account A: inline image with hash "cross"
        insert_attachment(&conn, "a1", "acct-a", "ma1", Some("cross"), true, None);
        // Account B: cached (non-inline) file with same hash
        insert_attachment(
            &conn,
            "b1",
            "acct-b",
            "mb1",
            Some("cross"),
            false,
            Some("attachment_cache/cross"),
        );

        let plan = super::delete_account_orchestrate(&conn, "acct-a").expect("orchestrate");

        // Non-inline ref from another account should NOT protect the inline blob
        assert!(!plan.shared_inline_hashes.contains("cross"));
    }

    #[test]
    fn empty_account_deletes_cleanly() {
        let conn = test_db();
        insert_account(&conn, "acct-empty");

        let plan = super::delete_account_orchestrate(&conn, "acct-empty").expect("orchestrate");

        assert!(plan.data.message_ids.is_empty());
        assert!(plan.data.cached_files.is_empty());
        assert!(plan.data.inline_hashes.is_empty());
        assert!(plan.shared_cache_hashes.is_empty());
        assert!(plan.shared_inline_hashes.is_empty());

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM accounts WHERE id = 'acct-empty'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn pending_ops_cascade_deleted() {
        let conn = test_db();
        insert_account(&conn, "acct-a");

        conn.execute(
            "INSERT INTO pending_operations \
             (id, account_id, operation_type, resource_id, params, status) \
             VALUES ('op1', 'acct-a', 'archive', 'thread1', '{}', 'pending')",
            [],
        )
        .expect("insert pending op");

        let _plan = super::delete_account_orchestrate(&conn, "acct-a").expect("orchestrate");

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pending_operations WHERE account_id = 'acct-a'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "pending ops should be CASCADE-deleted");
    }

    #[test]
    fn null_content_hash_excluded_from_cleanup_lists() {
        let conn = test_db();
        insert_account(&conn, "acct-a");
        insert_thread(&conn, "acct-a", "t1");
        insert_message(&conn, "acct-a", "t1", "m1");

        // Attachment with content_hash = NULL (inline)
        insert_attachment(&conn, "att1", "acct-a", "m1", None, true, None);
        // Attachment with content_hash = NULL (cached path but no hash)
        conn.execute(
            "INSERT INTO attachments \
             (id, message_id, account_id, content_hash, is_inline, local_path, cached_at) \
             VALUES ('att2', 'm1', 'acct-a', NULL, 0, 'attachment_cache/orphan', 1000)",
            [],
        )
        .expect("insert null-hash cached attachment");
        // One with a real hash for contrast
        insert_attachment(&conn, "att3", "acct-a", "m1", Some("real"), true, None);

        let plan = super::delete_account_orchestrate(&conn, "acct-a").expect("orchestrate");

        // Only the real hash should appear
        assert_eq!(plan.data.inline_hashes, vec!["real".to_string()]);
        assert!(
            plan.data.cached_files.is_empty(),
            "null-hash cached file should be excluded"
        );
    }
}
