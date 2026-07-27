use std::collections::{HashMap, HashSet};

use db::db::WriteConn;
use sync::threading::{ThreadGroup, ThreadableMessage};
use sync::types::MessageMeta;
use types::NamespaceAttribution;

use super::hydrate::ConsumerMessageRow;

const THREAD_BATCH_SIZE: usize = 100;

#[derive(Default)]
pub(super) struct ImapThreadAccumulator {
    partitions: HashMap<NamespaceAttribution, ImapThreadPartition>,
}

impl ImapThreadAccumulator {
    pub(super) fn clear(&mut self) {
        self.partitions.clear();
    }
}

#[derive(Default)]
struct ImapThreadPartition {
    threadable: Vec<ThreadableMessage>,
    meta: HashMap<String, MessageMeta>,
    folder_ids: HashMap<String, Vec<String>>,
}

impl ImapThreadAccumulator {
    /// Accumulate hydrated rows keyed by the id each row was actually
    /// PERSISTED under (`PersistAffected::message_ids`, post-adoption), in 1:1
    /// positional correspondence with `rows`. The stored id - not the
    /// provisional hydrate-time id - is load-bearing: the IMAP write arm
    /// adopts the existing local id of any `(account_id, imap_folder,
    /// imap_uid)` row before insert, and the drive-end threading pass
    /// reassigns by message id, so keying on the provisional id would make
    /// `reassign_messages_and_repair_threads` target a non-existent id and
    /// leave the message stranded on its legacy thread.
    pub(super) fn push_rows_with_ids(
        &mut self,
        rows: &[ConsumerMessageRow],
        stored_ids: &[String],
        namespace: &NamespaceAttribution,
    ) {
        let partition = self.partitions.entry(namespace.clone()).or_default();
        for (row, stored_id) in rows.iter().zip(stored_ids) {
            partition.push_row(row, stored_id);
        }
    }
}

impl ImapThreadPartition {
    fn push_row(&mut self, row: &ConsumerMessageRow, stored_id: &str) {
        let message_id = imap_thread_message_id(row, stored_id);
        self.threadable.push(ThreadableMessage {
            id: stored_id.to_string(),
            message_id: message_id.clone(),
            in_reply_to: row.message.in_reply_to_header.clone(),
            references: row.message.references_header.clone(),
            subject: row.message.subject.clone(),
            date: row.message.date,
        });
        self.meta.insert(
            stored_id.to_string(),
            MessageMeta {
                id: stored_id.to_string(),
                rfc_message_id: message_id,
                label_ids: row
                    .folders
                    .iter()
                    .map(common::types::FolderKind::storage_id)
                    .chain(row.labels.iter().map(common::types::LabelKind::storage_id))
                    .collect(),
                is_read: row.message.is_read,
                is_starred: row.message.is_starred,
                has_attachments: !row.attachments.is_empty(),
                subject: row.message.subject.clone(),
                snippet: row.message.snippet.clone(),
                date: row.message.date,
            },
        );
        self.folder_ids.insert(
            stored_id.to_string(),
            row.folders
                .iter()
                .map(common::types::FolderKind::storage_id)
                .collect(),
        );
    }
}

pub(super) async fn run_drive_end_threading(
    stores: &super::BifrostConsumerStores,
    account_id: &str,
    accumulator: &ImapThreadAccumulator,
) -> Result<Vec<String>, String> {
    if accumulator.partitions.is_empty() {
        return Ok(Vec::new());
    }
    let mut groups = Vec::new();
    let mut meta = HashMap::new();
    let mut folder_ids = HashMap::new();
    // JWZ runs once per namespace partition, so a shared folder's messages can
    // never merge into a personal thread (obstacle J.1), and the resulting
    // thread key is namespace-qualified so the two partitions cannot collide
    // on `threads`' `(account_id, id)` primary key (obstacle P).
    let mut thread_namespaces: HashMap<String, &NamespaceAttribution> = HashMap::new();
    for (namespace, partition) in &accumulator.partitions {
        let mut partition_groups = sync::threading::build_threads(&partition.threadable);
        for group in &mut partition_groups {
            group.thread_id = namespace.qualified_id(&group.thread_id);
            thread_namespaces.insert(group.thread_id.clone(), namespace);
        }
        groups.extend(partition_groups);
        meta.extend(partition.meta.clone());
        folder_ids.extend(partition.folder_ids.clone());
    }
    // Drive-end threading REASSIGNS messages onto JWZ-computed thread ids that
    // the per-batch persist never saw, so it is the writer that creates those
    // thread rows. It must therefore carry the attribution itself - inheriting
    // it via the aggregate's COALESCE is not available for a fresh row.
    let namespace_columns: HashMap<String, (Option<String>, Option<String>)> = thread_namespaces
        .into_iter()
        .map(|(thread_id, namespace)| {
            let kind = match namespace {
                NamespaceAttribution::Personal => None,
                NamespaceAttribution::Shared { .. } => Some("shared".to_string()),
                NamespaceAttribution::Public { .. } => Some("public".to_string()),
            };
            (
                thread_id,
                (kind, namespace.storage_key().map(str::to_string)),
            )
        })
        .collect();
    let account_id = account_id.to_string();
    stores
        .db
        .with_write(move |conn| {
            let thread_ids = groups
                .iter()
                .map(|group| group.thread_id.clone())
                .collect::<Vec<_>>();
            let skipped =
                sync::pending::get_blocked_thread_ids(&conn.as_read(), &account_id, &thread_ids)
                    .map_err(|error| format!("get blocked IMAP thread ids: {error}"))?;
            store_threads(
                conn,
                &account_id,
                &groups,
                &meta,
                &folder_ids,
                &skipped,
                &namespace_columns,
            )
        })
        .await
}

fn imap_thread_message_id(row: &ConsumerMessageRow, stored_id: &str) -> String {
    row.message
        .message_id_header
        .clone()
        .unwrap_or_else(|| format!("synthetic-{stored_id}@ratatoskr.local"))
}

fn store_threads(
    conn: &WriteConn<'_>,
    account_id: &str,
    thread_groups: &[ThreadGroup],
    all_meta: &HashMap<String, MessageMeta>,
    all_folder_ids: &HashMap<String, Vec<String>>,
    skipped_thread_ids: &HashSet<String>,
    namespace_columns: &HashMap<String, (Option<String>, Option<String>)>,
) -> Result<Vec<String>, String> {
    let mut affected_thread_ids = Vec::new();
    let all_message_ids = all_meta.keys().cloned().collect::<HashSet<_>>();
    let final_thread_ids = thread_groups
        .iter()
        .map(|group| group.thread_id.clone())
        .collect::<HashSet<_>>();

    for batch in thread_groups.chunks(THREAD_BATCH_SIZE) {
        let tx = conn
            .transaction()
            .map_err(|error| format!("begin IMAP thread tx: {error}"))?;
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

            messages.sort_by_key(|message| message.date);
            let message_ids: Vec<&str> =
                messages.iter().map(|message| message.id.as_str()).collect();
            let aggregate_messages: Vec<db::db::queries_extra::NonReactionMessage> = messages
                .iter()
                .map(|message| {
                    db::db::queries_extra::NonReactionMessage::new(
                        message.subject.clone(),
                        message.snippet.clone(),
                        message.date,
                        message.is_read,
                        message.is_starred,
                        message.has_attachments,
                    )
                })
                .collect();
            let (first, rest) = aggregate_messages
                .split_first()
                .ok_or_else(|| "IMAP thread group missing aggregate messages".to_string())?;
            let aggregate =
                db::db::queries_extra::ThreadAggregate::compute_from_messages(first, rest);
            let (namespace_kind, namespace_id) = namespace_columns
                .get(&group.thread_id)
                .map_or((None, None), |(kind, id)| (kind.as_deref(), id.as_deref()));
            db::db::queries_extra::upsert_thread_aggregate(
                &tx,
                account_id,
                &group.thread_id,
                &aggregate,
                Some(false),
                namespace_kind,
                namespace_id,
            )?;
            db::db::queries_extra::reassign_messages_and_repair_threads(
                &tx,
                account_id,
                &group.thread_id,
                &message_ids,
                &user_emails,
            )?;
            let folder_ids = message_ids
                .iter()
                .filter_map(|message_id| all_folder_ids.get(*message_id))
                .flatten()
                .map(String::as_str)
                .collect::<HashSet<_>>();
            db::db::queries_extra::delete_thread_folder_rows(&tx, account_id, &group.thread_id)?;
            db::db::queries_extra::insert_thread_folder_rows(
                &tx,
                account_id,
                &group.thread_id,
                folder_ids,
            )?;
            affected_thread_ids.push(group.thread_id.clone());
        }

        tx.commit()
            .map_err(|error| format!("commit IMAP thread tx: {error}"))?;
    }

    sync::pipeline::cleanup_orphan_threads(conn, account_id, &all_message_ids, &final_thread_ids)?;
    Ok(affected_thread_ids)
}
