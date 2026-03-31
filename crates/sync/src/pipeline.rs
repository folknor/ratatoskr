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

        let user_emails = super::persistence::query_user_emails(&tx)?;

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

            // Collect old thread IDs for messages being reassigned so we can
            // clean up orphaned thread_participants after the UPDATE.
            let message_ids: Vec<&str> = messages.iter().map(|m| m.id.as_str()).collect();
            let mut old_thread_ids: HashSet<String> = HashSet::new();
            for chunk in message_ids.chunks(100) {
                let placeholders: String = chunk
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format!("?{}", i + 2))
                    .collect::<Vec<_>>()
                    .join(", ");

                let sql = format!(
                    "SELECT DISTINCT thread_id FROM messages \
                     WHERE account_id = ?1 AND id IN ({placeholders})"
                );

                let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
                params.push(Box::new(account_id.to_string()));
                for id in chunk {
                    params.push(Box::new(id.to_string()));
                }
                let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                    params.iter().map(AsRef::as_ref).collect();

                let mut stmt = tx
                    .prepare(&sql)
                    .map_err(|e| format!("prepare old thread query: {e}"))?;
                let rows = stmt
                    .query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))
                    .map_err(|e| format!("query old thread ids: {e}"))?;
                for tid in rows.flatten() {
                    if tid != group.thread_id {
                        old_thread_ids.insert(tid);
                    }
                }
            }

            // Batch-update message thread IDs
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

            // Clean up thread_participants for old threads that lost messages
            // to re-threading. Threads with 0 remaining messages get their
            // participants (and thread/label rows) deleted outright; threads
            // that still have messages get participants recomputed.
            for old_tid in &old_thread_ids {
                let remaining: i64 = tx
                    .query_row(
                        "SELECT COUNT(*) FROM messages \
                         WHERE thread_id = ?1 AND account_id = ?2",
                        rusqlite::params![old_tid, account_id],
                        |row| row.get(0),
                    )
                    .map_err(|e| format!("count remaining in old thread: {e}"))?;

                if remaining == 0 {
                    tx.execute(
                        "DELETE FROM thread_participants \
                         WHERE thread_id = ?1 AND account_id = ?2",
                        rusqlite::params![old_tid, account_id],
                    )
                    .map_err(|e| format!("delete orphan thread participants: {e}"))?;
                    tx.execute(
                        "DELETE FROM thread_labels \
                         WHERE thread_id = ?1 AND account_id = ?2",
                        rusqlite::params![old_tid, account_id],
                    )
                    .map_err(|e| format!("delete orphan thread labels: {e}"))?;
                    tx.execute(
                        "DELETE FROM threads WHERE id = ?1 AND account_id = ?2",
                        rusqlite::params![old_tid, account_id],
                    )
                    .map_err(|e| format!("delete orphan thread: {e}"))?;
                } else {
                    // Recompute participants from remaining messages
                    tx.execute(
                        "DELETE FROM thread_participants \
                         WHERE account_id = ?1 AND thread_id = ?2",
                        rusqlite::params![account_id, old_tid],
                    )
                    .map_err(|e| format!("clear old thread participants: {e}"))?;

                    let mut addr_stmt = tx
                        .prepare(
                            "SELECT from_address, to_addresses, cc_addresses, bcc_addresses \
                             FROM messages WHERE account_id = ?1 AND thread_id = ?2",
                        )
                        .map_err(|e| format!("prepare old thread addr query: {e}"))?;
                    let rows: Vec<(
                        Option<String>,
                        Option<String>,
                        Option<String>,
                        Option<String>,
                    )> = addr_stmt
                        .query_map(rusqlite::params![account_id, old_tid], |row| {
                            Ok((
                                row.get::<_, Option<String>>(0)?,
                                row.get::<_, Option<String>>(1)?,
                                row.get::<_, Option<String>>(2)?,
                                row.get::<_, Option<String>>(3)?,
                            ))
                        })
                        .map_err(|e| format!("query old thread addr: {e}"))?
                        .filter_map(Result::ok)
                        .collect();
                    for (from, to, cc, bcc) in &rows {
                        super::persistence::upsert_thread_participants(
                            &tx,
                            account_id,
                            old_tid,
                            from.as_deref(),
                            to.as_deref(),
                            cc.as_deref(),
                            bcc.as_deref(),
                        )?;
                    }
                }
            }

            // Populate thread_participants from the messages' address fields.
            // IMAP messages were inserted earlier with placeholder thread IDs;
            // now that JWZ assigned final IDs, we can read the address fields
            // from the DB and populate participants for the real thread ID.
            {
                let mut addr_stmt = tx
                    .prepare(
                        "SELECT from_address, to_addresses, cc_addresses, bcc_addresses \
                         FROM messages WHERE account_id = ?1 AND thread_id = ?2",
                    )
                    .map_err(|e| format!("prepare addr query: {e}"))?;
                let rows: Vec<(
                    Option<String>,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                )> = addr_stmt
                    .query_map(rusqlite::params![account_id, group.thread_id], |row| {
                        Ok((
                            row.get::<_, Option<String>>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, Option<String>>(3)?,
                        ))
                    })
                    .map_err(|e| format!("query addr: {e}"))?
                    .filter_map(Result::ok)
                    .collect();
                for (from, to, cc, bcc) in &rows {
                    super::persistence::upsert_thread_participants(
                        &tx,
                        account_id,
                        &group.thread_id,
                        from.as_deref(),
                        to.as_deref(),
                        cc.as_deref(),
                        bcc.as_deref(),
                    )?;
                }
            }
            super::persistence::maybe_update_chat_state(
                &tx,
                account_id,
                &group.thread_id,
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
