use std::collections::{HashMap, HashSet};

use rusqlite::Connection;
use sync::threading::ThreadGroup;
use sync::types::MessageMeta;

const THREAD_BATCH_SIZE: usize = 100;

/// Store IMAP thread groups and update message thread IDs.
///
/// IMAP thread membership is fully derived from per-message ground truth:
/// `messages.imap_folder` for folders (each IMAP message lives in exactly
/// one folder) and `message_keywords` for keyword labels. The
/// `reassign_messages_and_repair_threads` call below owns the recompute
/// of `thread_folders` and `thread_labels` from those sources, so no
/// separate thread-scope replace is performed here.
pub(super) fn store_threads(
    conn: &Connection,
    account_id: &str,
    thread_groups: &[ThreadGroup],
    all_meta: &HashMap<String, MessageMeta>,
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

        let user_emails = db::db::queries_extra::query_user_emails(&tx)?;

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

            messages.sort_by_key(|m| m.date);

            let message_ids: Vec<&str> = messages.iter().map(|m| m.id.as_str()).collect();
            let aggregate_messages: Vec<db::db::queries_extra::NonReactionMessage> = messages
                .iter()
                .map(|m| {
                    db::db::queries_extra::NonReactionMessage::new(
                        m.subject.clone(),
                        m.snippet.clone(),
                        m.date,
                        m.is_read,
                        m.is_starred,
                        m.has_attachments,
                    )
                })
                .collect();
            let (first_aggregate_message, rest_aggregate_messages) = aggregate_messages
                .split_first()
                .ok_or_else(|| "thread group missing aggregate messages".to_string())?;
            let aggregate = db::db::queries_extra::ThreadAggregate::compute_from_messages(
                first_aggregate_message,
                rest_aggregate_messages,
            );
            db::db::queries_extra::upsert_thread_aggregate(
                &tx,
                account_id,
                &group.thread_id,
                &aggregate,
                Some(false),
                None,
            )?;
            db::db::queries_extra::reassign_messages_and_repair_threads(
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
