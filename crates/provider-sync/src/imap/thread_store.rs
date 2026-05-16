use std::collections::{HashMap, HashSet};

use rusqlite::Connection;
use sync::threading::ThreadGroup;
use sync::types::MessageMeta;

const THREAD_BATCH_SIZE: usize = 100;

/// Store IMAP thread groups and update message thread IDs.
pub(super) fn store_threads(
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

            let mut all_folder_ids = HashSet::new();
            let mut all_label_ids = HashSet::new();
            for msg in &messages {
                partition_imap_memberships(&msg.label_ids, &mut all_folder_ids, &mut all_label_ids);
                if let Some(extra) = labels_by_rfc_id.get(&msg.rfc_message_id) {
                    partition_imap_memberships(extra, &mut all_folder_ids, &mut all_label_ids);
                }
            }

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
            replace_full_thread_folders(
                &tx,
                account_id,
                &group.thread_id,
                all_folder_ids.iter().map(String::as_str),
            )?;
            replace_full_thread_labels(
                &tx,
                account_id,
                &group.thread_id,
                all_label_ids.iter().map(String::as_str),
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

fn replace_full_thread_folders<'a>(
    tx: &rusqlite::Transaction,
    account_id: &str,
    thread_id: &str,
    folder_ids: impl IntoIterator<Item = &'a str>,
) -> Result<(), String> {
    let folder_ids = crate::thread_membership::filtered_membership_ids(folder_ids);
    db::db::queries_extra::delete_thread_folder_rows(tx, account_id, thread_id)?;
    db::db::queries_extra::insert_thread_folder_rows(tx, account_id, thread_id, folder_ids)
}

fn replace_full_thread_labels<'a>(
    tx: &rusqlite::Transaction,
    account_id: &str,
    thread_id: &str,
    label_ids: impl IntoIterator<Item = &'a str>,
) -> Result<(), String> {
    let label_ids = crate::thread_membership::filtered_membership_ids(label_ids);
    db::db::queries_extra::delete_thread_label_rows(tx, account_id, thread_id)?;
    db::db::queries_extra::insert_thread_label_rows(tx, account_id, thread_id, label_ids)?;
    db::db::queries_extra::finalize_provider_truth_label_membership(tx, account_id, thread_id)
}

fn partition_imap_memberships<'a>(
    labels: impl IntoIterator<Item = &'a String>,
    all_folder_ids: &mut HashSet<String>,
    all_label_ids: &mut HashSet<String>,
) {
    for label_id in labels {
        // IMAP-only pipeline: provider folder IDs are folder-shaped and
        // user-visible keywords are the only label-shaped memberships here.
        if label_id.starts_with("kw:") {
            all_label_ids.insert(label_id.clone());
        } else {
            all_folder_ids.insert(label_id.clone());
        }
    }
}
