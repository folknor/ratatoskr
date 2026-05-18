use std::collections::HashSet;

use crate::blob_hash::BlobHash;
use crate::db::WriteTarget;

pub struct DbAccountDeletionData {
    pub message_ids: Vec<String>,
    /// Content hashes of cached attachments for this account. The
    /// flat-cache `local_path` retired in attachments roadmap Phase 3;
    /// the consumer tombstones each hash in `PackStore` instead of
    /// unlinking files.
    pub cached_hashes: Vec<BlobHash>,
    pub inline_hashes: Vec<BlobHash>,
}

pub struct DbAccountDeletionPlan {
    pub data: DbAccountDeletionData,
    pub shared_cache_hashes: HashSet<BlobHash>,
    pub shared_inline_hashes: HashSet<BlobHash>,
}

/// Gather all data needed for account deletion cleanup (message IDs, cached
/// file paths, inline image hashes) in a single DB pass.
pub fn gather_account_deletion_data_sync(
    conn: &impl WriteTarget,
    account_id: &str,
) -> Result<DbAccountDeletionData, String> {
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

    let cached_hashes = {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT content_hash
                 FROM attachments
                 WHERE account_id = ?1
                   AND content_hash IS NOT NULL",
            )
            .map_err(|e| format!("prepare account cached attachment query: {e}"))?;
        stmt.query_map(rusqlite::params![account_id], |row| {
            row.get::<_, BlobHash>("content_hash")
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
            row.get::<_, BlobHash>("content_hash")
        })
        .map_err(|e| format!("query account inline hashes: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect account inline hashes: {e}"))?
    };

    Ok(DbAccountDeletionData {
        message_ids,
        cached_hashes,
        inline_hashes,
    })
}

/// Batch-count remaining cached attachment references for multiple content
/// hashes, excluding a specific account. Returns the set of hashes that
/// still have at least one reference from another account.
pub fn referenced_hashes_excluding_account_sync(
    conn: &impl WriteTarget,
    content_hashes: &[BlobHash],
    account_id: &str,
) -> Result<HashSet<BlobHash>, String> {
    if content_hashes.is_empty() {
        return Ok(HashSet::new());
    }

    let unique: HashSet<BlobHash> = content_hashes.iter().copied().collect();
    let hashes: Vec<BlobHash> = unique.into_iter().collect();

    let mut referenced = HashSet::new();
    for chunk in hashes.chunks(500) {
        let placeholders: Vec<String> = (0..chunk.len()).map(|i| format!("?{}", i + 2)).collect();
        let sql = format!(
            "SELECT content_hash FROM attachments \
             WHERE content_hash IN ({}) AND account_id != ?1 \
             GROUP BY content_hash",
            placeholders.join(", ")
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| format!("prepare batch ref count: {e}"))?;
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
            vec![Box::new(account_id.to_string())];
        for hash in chunk {
            params.push(Box::new(*hash));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(std::convert::AsRef::as_ref).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                row.get::<_, BlobHash>("content_hash")
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
pub fn inline_hashes_referenced_by_other_accounts_sync(
    conn: &impl WriteTarget,
    inline_hashes: &[BlobHash],
    account_id: &str,
) -> Result<HashSet<BlobHash>, String> {
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
            params.push(Box::new(*hash));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(std::convert::AsRef::as_ref).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                row.get::<_, BlobHash>("content_hash")
            })
            .map_err(|e| format!("query inline ref check: {e}"))?;
        for row in rows {
            let hash = row.map_err(|e| format!("read inline ref check row: {e}"))?;
            referenced.insert(hash);
        }
    }
    Ok(referenced)
}

pub fn delete_account_row_sync(conn: &impl WriteTarget, account_id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM accounts WHERE id = ?1",
        rusqlite::params![account_id],
    )
    .map_err(|e| format!("delete account: {e}"))?;
    Ok(())
}

pub fn delete_account_orchestrate_sync(
    conn: &impl WriteTarget,
    account_id: &str,
) -> Result<DbAccountDeletionPlan, String> {
    let data = gather_account_deletion_data_sync(conn, account_id)?;
    let shared_cache_hashes =
        referenced_hashes_excluding_account_sync(conn, &data.cached_hashes, account_id)?;
    let shared_inline_hashes =
        inline_hashes_referenced_by_other_accounts_sync(conn, &data.inline_hashes, account_id)?;
    delete_account_row_sync(conn, account_id)?;
    Ok(DbAccountDeletionPlan {
        data,
        shared_cache_hashes,
        shared_inline_hashes,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rusqlite::{Connection, params};

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("enable FK");
        crate::db::migrations::run_all(&conn).expect("migrations");
        conn
    }

    fn write(conn: &Connection) -> crate::db::WriteConn<'_> {
        crate::db::WriteConn::from_raw(conn)
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
        content_hash: Option<&BlobHash>,
        is_inline: bool,
    ) {
        conn.execute(
            "INSERT INTO attachments \
             (id, message_id, account_id, content_hash, is_inline) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, message_id, account_id, content_hash, is_inline as i32],
        )
        .expect("insert attachment");
    }

    fn h(label: &[u8]) -> BlobHash {
        BlobHash::hash(label)
    }

    #[test]
    fn orchestrate_gathers_before_cascade() {
        let conn = test_db();
        insert_account(&conn, "acct-a");
        insert_thread(&conn, "acct-a", "t1");
        insert_message(&conn, "acct-a", "t1", "m1");
        insert_message(&conn, "acct-a", "t1", "m2");
        let hash1 = h(b"hash1");
        let hash2 = h(b"hash2");
        insert_attachment(&conn, "att1", "acct-a", "m1", Some(&hash1), true);
        insert_attachment(&conn, "att2", "acct-a", "m2", Some(&hash2), false);

        let plan = delete_account_orchestrate_sync(&write(&conn), "acct-a").expect("orchestrate");

        assert_eq!(plan.data.message_ids.len(), 2);
        assert!(plan.data.message_ids.contains(&"m1".to_string()));
        assert!(plan.data.message_ids.contains(&"m2".to_string()));
        // Both hashes flow through `inline_hashes` and `cached_hashes`
        // because both columns are populated; the inline filter is
        // `is_inline = 1`, the cache filter is `content_hash IS NOT NULL`.
        assert!(plan.data.inline_hashes.contains(&hash1));
        assert!(plan.data.cached_hashes.contains(&hash2));

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM accounts WHERE id = 'acct-a'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);

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

        let shared = h(b"shared");
        let only_a = h(b"only-a");
        insert_attachment(&conn, "a1", "acct-a", "ma1", Some(&shared), true);
        insert_attachment(&conn, "b1", "acct-b", "mb1", Some(&shared), true);
        insert_attachment(&conn, "a2", "acct-a", "ma1", Some(&only_a), true);

        let plan = delete_account_orchestrate_sync(&write(&conn), "acct-a").expect("orchestrate");

        assert!(plan.shared_inline_hashes.contains(&shared));
        assert!(!plan.shared_inline_hashes.contains(&only_a));
        assert!(plan.data.inline_hashes.contains(&shared));
        assert!(plan.data.inline_hashes.contains(&only_a));
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

        let shared_cache = h(b"shared-cache");
        let only_a = h(b"only-a");
        insert_attachment(&conn, "a1", "acct-a", "ma1", Some(&shared_cache), false);
        insert_attachment(&conn, "b1", "acct-b", "mb1", Some(&shared_cache), false);
        insert_attachment(&conn, "a2", "acct-a", "ma1", Some(&only_a), false);

        let plan = delete_account_orchestrate_sync(&write(&conn), "acct-a").expect("orchestrate");

        assert!(plan.shared_cache_hashes.contains(&shared_cache));
        assert!(!plan.shared_cache_hashes.contains(&only_a));
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

        let cross = h(b"cross");
        insert_attachment(&conn, "a1", "acct-a", "ma1", Some(&cross), true);
        insert_attachment(&conn, "b1", "acct-b", "mb1", Some(&cross), false);

        let plan = delete_account_orchestrate_sync(&write(&conn), "acct-a").expect("orchestrate");

        assert!(!plan.shared_inline_hashes.contains(&cross));
    }

    #[test]
    fn empty_account_deletes_cleanly() {
        let conn = test_db();
        insert_account(&conn, "acct-empty");

        let plan = delete_account_orchestrate_sync(&write(&conn), "acct-empty").expect("orchestrate");

        assert!(plan.data.message_ids.is_empty());
        assert!(plan.data.cached_hashes.is_empty());
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

        let _plan = delete_account_orchestrate_sync(&write(&conn), "acct-a").expect("orchestrate");

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

        insert_attachment(&conn, "att1", "acct-a", "m1", None, true);
        let real = h(b"real");
        insert_attachment(&conn, "att2", "acct-a", "m1", Some(&real), true);

        let plan = delete_account_orchestrate_sync(&write(&conn), "acct-a").expect("orchestrate");

        assert_eq!(plan.data.inline_hashes, vec![real]);
        // Null-hash attachments do not contribute to the cleanup list -
        // the `cached_hashes` query filters on `content_hash IS NOT NULL`.
        assert_eq!(plan.data.cached_hashes, vec![real]);
    }
}
