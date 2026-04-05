use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use crate::threading::ThreadGroup;

use crate::types::MessageMeta;
use db::db::queries_extra::{
    ThreadAggregate, query_user_emails, reassign_messages_and_repair_threads,
    replace_thread_labels, upsert_thread_aggregate,
};

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
    log::debug!(
        "Storing {} thread groups for account {}",
        thread_groups.len(),
        account_id
    );
    let mut affected_thread_ids = Vec::new();

    for batch in thread_groups.chunks(THREAD_BATCH_SIZE) {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("begin thread tx: {e}"))?;

        let user_emails = query_user_emails(&tx)?;

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

            let message_ids: Vec<&str> = messages.iter().map(|m| m.id.as_str()).collect();
            let aggregate = ThreadAggregate {
                subject: first.subject.clone(),
                snippet: last.snippet.clone(),
                last_date: last.date,
                message_count: i64::try_from(messages.len()).unwrap_or(i64::MAX),
                is_read,
                is_starred,
                has_attachments,
            };
            upsert_thread_aggregate(&tx, account_id, &group.thread_id, &aggregate, Some(false), None)?;
            replace_thread_labels(&tx, account_id, &group.thread_id, all_label_ids.iter().map(String::as_str))?;
            reassign_messages_and_repair_threads(
                &tx,
                account_id,
                &group.thread_id,
                &message_ids,
                &user_emails,
            )?;

            affected_thread_ids.push(group.thread_id.clone());
        }

        tx.commit().map_err(|e| format!("commit threads: {e}"))?;
    }

    log::debug!(
        "Stored threads for account {}: {} affected thread IDs",
        account_id,
        affected_thread_ids.len()
    );
    Ok(affected_thread_ids)
}

/// Delete orphaned placeholder threads that are no longer referenced by any final thread group.
pub fn cleanup_orphan_threads(
    conn: &Connection,
    account_id: &str,
    all_message_ids: &HashSet<String>,
    final_thread_ids: &HashSet<String>,
) -> Result<u64, String> {
    log::debug!(
        "Cleaning up orphan threads for account {}: checking {} message IDs against {} final threads",
        account_id,
        all_message_ids.len(),
        final_thread_ids.len()
    );
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
    if count > 0 {
        log::info!("Cleaned up {count} orphan threads for account {account_id}");
    }
    Ok(count)
}

/// Mark initial sync as completed for providers whose delta state is stored elsewhere.
pub fn mark_initial_sync_completed(conn: &Connection, account_id: &str) -> Result<(), String> {
    log::info!("Marking initial sync completed for account {account_id}");
    conn.execute(
        "UPDATE accounts SET initial_sync_completed = 1, updated_at = unixepoch() WHERE id = ?1",
        rusqlite::params![account_id],
    )
    .map_err(|e| format!("mark initial sync completed: {e}"))?;
    Ok(())
}

/// Clear account history_id (forces next sync to be initial).
pub fn clear_account_history_id(conn: &Connection, account_id: &str) -> Result<(), String> {
    log::info!("Clearing history_id for account {account_id} (forcing initial sync)");
    conn.execute(
        "UPDATE accounts SET history_id = NULL, initial_sync_completed = 0, updated_at = unixepoch() WHERE id = ?1",
        rusqlite::params![account_id],
    )
    .map_err(|e| format!("clear account history_id: {e}"))?;
    Ok(())
}

/// Clear all folder sync states for an account (forces full folder resync).
pub fn clear_all_folder_sync_states(conn: &Connection, account_id: &str) -> Result<(), String> {
    log::info!("Clearing all folder sync states for account {account_id}");
    conn.execute(
        "DELETE FROM folder_sync_state WHERE account_id = ?1",
        rusqlite::params![account_id],
    )
    .map_err(|e| format!("clear folder sync states: {e}"))?;
    Ok(())
}
