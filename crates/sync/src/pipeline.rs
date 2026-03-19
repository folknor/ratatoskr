use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use crate::threading::ThreadGroup;

use crate::types::MessageMeta;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Number of thread groups to process per transaction.
const THREAD_BATCH_SIZE: usize = 100;

// ---------------------------------------------------------------------------
// Store threads after JWZ threading pass
// ---------------------------------------------------------------------------

/// Store thread groups and update message thread IDs.
///
/// This is the equivalent of the TS `storeThreadsAndMessages` for initial sync,
/// and the Phase 4 thread storage loop.
pub fn store_threads(
    conn: &Connection,
    account_id: &str,
    thread_groups: &[ThreadGroup],
    all_meta: &HashMap<String, MessageMeta>,
    labels_by_rfc_id: &HashMap<String, HashSet<String>>,
    skipped_thread_ids: &HashSet<String>,
) -> Result<Vec<String>, String> {
    let mut affected_thread_ids = Vec::new();

    for batch in thread_groups.chunks(THREAD_BATCH_SIZE) {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("begin thread tx: {e}"))?;

        for group in batch {
            if skipped_thread_ids.contains(&group.thread_id) {
                continue;
            }

            let mut messages: Vec<&MessageMeta> = group
                .message_ids
                .iter()
                .filter_map(|id| all_meta.get(id))
                .collect();

            if messages.is_empty() {
                continue;
            }

            // Sort by date ascending
            messages.sort_by_key(|m| m.date);

            let first = messages[0];
            let last = messages[messages.len() - 1];

            // Collect all label IDs including cross-folder copies
            let mut all_label_ids = HashSet::new();
            for msg in &messages {
                for lid in &msg.label_ids {
                    all_label_ids.insert(lid.clone());
                }
                if let Some(extra) = labels_by_rfc_id.get(&msg.rfc_message_id) {
                    for lid in extra {
                        all_label_ids.insert(lid.clone());
                    }
                }
            }

            let is_read = messages.iter().all(|m| m.is_read);
            let is_starred = messages.iter().any(|m| m.is_starred);
            let has_attachments = messages.iter().any(|m| m.has_attachments);

            // Upsert the real thread
            tx.execute(
                "INSERT OR REPLACE INTO threads \
                 (id, account_id, subject, snippet, last_message_at, message_count, \
                  is_read, is_starred, is_important, has_attachments) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9)",
                rusqlite::params![
                    group.thread_id,
                    account_id,
                    first.subject,
                    last.snippet,
                    last.date,
                    i64::try_from(messages.len()).unwrap_or(i64::MAX),
                    is_read,
                    is_starred,
                    has_attachments,
                ],
            )
            .map_err(|e| format!("upsert thread: {e}"))?;

            // Set thread labels (delete old, insert new)
            tx.execute(
                "DELETE FROM thread_labels WHERE account_id = ?1 AND thread_id = ?2",
                rusqlite::params![account_id, group.thread_id],
            )
            .map_err(|e| format!("delete thread labels: {e}"))?;

            for label_id in &all_label_ids {
                tx.execute(
                    "INSERT OR IGNORE INTO thread_labels (account_id, thread_id, label_id) \
                     VALUES (?1, ?2, ?3)",
                    rusqlite::params![account_id, group.thread_id, label_id],
                )
                .map_err(|e| format!("insert thread label: {e}"))?;
            }

            // Batch-update message thread IDs
            let message_ids: Vec<&str> = messages.iter().map(|m| m.id.as_str()).collect();
            for chunk in message_ids.chunks(100) {
                let placeholders: String = chunk
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format!("?{}", i + 3))
                    .collect::<Vec<_>>()
                    .join(", ");

                let sql = format!(
                    "UPDATE messages SET thread_id = ?1 \
                     WHERE account_id = ?2 AND id IN ({placeholders})"
                );

                let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
                params.push(Box::new(group.thread_id.clone()));
                params.push(Box::new(account_id.to_string()));
                for id in chunk {
                    params.push(Box::new(id.to_string()));
                }
                let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                    params.iter().map(AsRef::as_ref).collect();

                tx.execute(&sql, param_refs.as_slice())
                    .map_err(|e| format!("update message thread_ids: {e}"))?;
            }

            affected_thread_ids.push(group.thread_id.clone());
        }

        tx.commit().map_err(|e| format!("commit threads: {e}"))?;
    }

    Ok(affected_thread_ids)
}

/// Delete orphaned placeholder threads that are no longer referenced by any final thread group.
pub fn cleanup_orphan_threads(
    conn: &Connection,
    account_id: &str,
    all_message_ids: &HashSet<String>,
    final_thread_ids: &HashSet<String>,
) -> Result<u64, String> {
    let mut count: u64 = 0;
    for msg_id in all_message_ids {
        if !final_thread_ids.contains(msg_id) {
            let deleted = conn
                .execute(
                    "DELETE FROM threads WHERE id = ?1 AND account_id = ?2",
                    rusqlite::params![msg_id, account_id],
                )
                .map_err(|e| format!("delete orphan thread: {e}"))?;
            count += deleted as u64;
        }
    }
    Ok(count)
}

/// Check which thread IDs have pending local operations (should be skipped during sync).
pub fn get_skipped_thread_ids(
    conn: &Connection,
    account_id: &str,
    thread_ids: &[String],
) -> Result<HashSet<String>, String> {
    let mut skipped = HashSet::new();
    for tid in thread_ids {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) AS cnt FROM pending_operations \
                 WHERE account_id = ?1 AND resource_id = ?2 AND status != 'failed'",
                rusqlite::params![account_id, tid],
                |row| row.get("cnt"),
            )
            .map_err(|e| format!("check pending ops: {e}"))?;
        if count > 0 {
            log::info!("Skipping thread {tid}: has {count} pending local ops");
            skipped.insert(tid.clone());
        }
    }
    Ok(skipped)
}

/// Update account sync state (history_id column).
pub fn update_account_sync_state(
    conn: &Connection,
    account_id: &str,
    history_id: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE accounts SET history_id = ?1, initial_sync_completed = 1 WHERE id = ?2",
        rusqlite::params![history_id, account_id],
    )
    .map_err(|e| format!("update account sync state: {e}"))?;
    Ok(())
}

/// Mark initial sync as completed for providers whose delta state is stored elsewhere.
pub fn mark_initial_sync_completed(conn: &Connection, account_id: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE accounts SET initial_sync_completed = 1, updated_at = unixepoch() WHERE id = ?1",
        rusqlite::params![account_id],
    )
    .map_err(|e| format!("mark initial sync completed: {e}"))?;
    Ok(())
}

/// Get thread count for an account (used for recovery detection).
pub fn get_thread_count(conn: &Connection, account_id: &str) -> Result<i64, String> {
    conn.query_row(
        "SELECT COUNT(*) AS cnt FROM threads WHERE account_id = ?1",
        rusqlite::params![account_id],
        |row| row.get("cnt"),
    )
    .map_err(|e| format!("get thread count: {e}"))
}

/// Clear account history_id (forces next sync to be initial).
pub fn clear_account_history_id(conn: &Connection, account_id: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE accounts SET history_id = NULL, initial_sync_completed = 0, updated_at = unixepoch() WHERE id = ?1",
        rusqlite::params![account_id],
    )
    .map_err(|e| format!("clear account history_id: {e}"))?;
    Ok(())
}

/// Clear all folder sync states for an account (forces full folder resync).
pub fn clear_all_folder_sync_states(conn: &Connection, account_id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM folder_sync_state WHERE account_id = ?1",
        rusqlite::params![account_id],
    )
    .map_err(|e| format!("clear folder sync states: {e}"))?;
    Ok(())
}
