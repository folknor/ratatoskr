use std::collections::BTreeSet;

use common::types::LabelKind;
use db::db::queries_extra::{
    LabelWriteRow, compute_thread_aggregate, delete_messages_and_cleanup_threads,
    insert_attachments, insert_messages, maybe_update_chat_state, query_user_emails, upsert_labels,
    upsert_thread_aggregate, upsert_thread_participants,
};
use provider_sync::consumer_support::{
    FolderWriteRow, KeywordProvider, index_search_documents, insert_folders_batch,
    recompute_thread_keyword_labels, replace_message_folders_and_recompute,
    replace_message_keywords, replace_message_membership_and_recompute, store_inline_images,
    store_message_bodies,
};

use super::BifrostConsumerStores;
use super::BifrostProviderKind;
use super::hydrate::ConsumerMessageRow;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PersistAffected {
    pub thread_ids: Vec<String>,
    pub message_ids: Vec<String>,
}

// Finding G (pending-ops guard): the legacy Graph delta path ran
// `filter_pending_ops` to skip persisting server state for threads that still
// carried un-acked optimistic local mutations, so an in-flight user action was
// not clobbered by a stale server snapshot. This consumer write path has NO
// such guard, DELIBERATELY, matching the LANDED JMAP/B-series precedent: under
// the bifrost migration the action pipeline (B4) owns optimistic local state
// and its server reconciliation, not the read-side sync consumer. Re-homing a
// pending-ops filter here would duplicate (and risk diverging from) that B4
// ownership before B4 exists. Until B4 lands, the exposure is the same one the
// JMAP cut already accepted and is bounded by the action pipeline's own
// re-issue/ack semantics; it is recorded here rather than left silent (spec
// section 9 finding 6).
pub async fn persist(
    stores: &BifrostConsumerStores,
    account_id: &str,
    provider: BifrostProviderKind,
    rows: &[ConsumerMessageRow],
    deleted_ids: &[String],
) -> Result<PersistAffected, String> {
    if rows.is_empty() && deleted_ids.is_empty() {
        return Ok(PersistAffected::default());
    }

    let rows = rows.to_vec();
    let deleted_ids = deleted_ids.to_vec();
    let account_id = account_id.to_string();
    let affected = stores
        .db
        .with_write({
            let rows = rows.clone();
            let deleted_ids = deleted_ids.clone();
            let account_id = account_id.clone();
            move |conn| {
                let tx = conn.transaction().map_err(|error| error.to_string())?;
                for row in &rows {
                    tx.execute(
                "INSERT OR IGNORE INTO threads (account_id, id, message_count) VALUES (?1, ?2, 0)",
                rusqlite::params![row.message.account_id, row.message.thread_id],
            )
            .map_err(|error| format!("insert thread placeholder: {error}"))?;
                }
                let messages: Vec<_> = rows.iter().map(|row| row.message.clone()).collect();
                insert_messages(&tx, &messages)
                    .map_err(|error| format!("insert messages: {error}"))?;

                let mut thread_ids = BTreeSet::new();
                let mut message_ids = Vec::new();
                let user_emails = if matches!(
                    provider,
                    BifrostProviderKind::Jmap
                        | BifrostProviderKind::Graph
                        | BifrostProviderKind::Gmail
                ) {
                    Some(query_user_emails(&tx)?)
                } else {
                    None
                };
                for row in &rows {
                    thread_ids.insert(row.message.thread_id.clone());
                    message_ids.push(row.message.id.clone());
                    let label_rows = row
                        .labels
                        .iter()
                        .map(|label| label_write_row(label, &row.message.account_id))
                        .collect::<Vec<_>>();
                    upsert_labels(&tx, &label_rows)
                        .map_err(|error| format!("upsert labels: {error}"))?;
                    // The message_folders FK targets folders(account_id, id), so the
                    // baseline membership write must ensure each folder row exists
                    // before it writes membership (spec 1 / 4.1.4 "folder-row
                    // creation"). Production folder sync seeds these; the consumer's
                    // provider-agnostic path mints a minimal row from the FolderKind.
                    let folder_rows = row
                        .folders
                        .iter()
                        .map(|folder| {
                            let id = folder.storage_id();
                            FolderWriteRow {
                                name: id.clone(),
                                id,
                                account_id: row.message.account_id.clone(),
                                visible: None,
                                sort_order: None,
                                imap_folder_path: None,
                                imap_special_use: None,
                                namespace_type: None,
                                parent_id: None,
                                right_read: None,
                                right_add: None,
                                right_remove: None,
                                right_set_seen: None,
                                right_set_keywords: None,
                                right_create_child: None,
                                right_rename: None,
                                right_delete: None,
                                right_submit: None,
                                is_subscribed: None,
                                is_undeletable: false,
                            }
                        })
                        .collect::<Vec<_>>();
                    insert_folders_batch(&tx, &folder_rows)
                        .map_err(|error| format!("insert folders: {error}"))?;
                    match provider {
                        BifrostProviderKind::Jmap => {
                            replace_message_folders_and_recompute(
                                &tx,
                                &row.message.account_id,
                                &row.message.thread_id,
                                &row.message.id,
                                &row.folders,
                            )
                            .map_err(|error| {
                                format!(
                                    "replace JMAP message folders for {}: {error}",
                                    row.message.id
                                )
                            })?;
                            insert_attachments(&tx, &row.attachments)
                                .map_err(|error| format!("insert JMAP attachments: {error}"))?;
                            upsert_thread_participants(
                                &tx,
                                &row.message.account_id,
                                &row.message.thread_id,
                                row.message.from_address.as_deref(),
                                row.message.to_addresses.as_deref(),
                                row.message.cc_addresses.as_deref(),
                                row.message.bcc_addresses.as_deref(),
                            )
                            .map_err(|error| {
                                format!(
                                    "upsert JMAP thread participants for {}: {error}",
                                    row.message.id
                                )
                            })?;
                        }
                        BifrostProviderKind::Graph | BifrostProviderKind::Gmail => {
                            replace_message_membership_and_recompute(
                                &tx,
                                &row.message.account_id,
                                &row.message.thread_id,
                                &row.message.id,
                                &row.folders,
                                &row.labels,
                            )
                            .map_err(|error| {
                                format!(
                                    "replace {} message membership for {}: {error}",
                                    provider.as_str(),
                                    row.message.id
                                )
                            })?;
                            insert_attachments(&tx, &row.attachments).map_err(|error| {
                                format!("insert {} attachments: {error}", provider.as_str())
                            })?;
                            if provider == BifrostProviderKind::Gmail {
                                insert_gmail_reaction(&tx, &account_id, row)?;
                            }
                            upsert_thread_participants(
                                &tx,
                                &row.message.account_id,
                                &row.message.thread_id,
                                row.message.from_address.as_deref(),
                                row.message.to_addresses.as_deref(),
                                row.message.cc_addresses.as_deref(),
                                row.message.bcc_addresses.as_deref(),
                            )
                            .map_err(|error| {
                                format!(
                                    "upsert {} thread participants for {}: {error}",
                                    provider.as_str(),
                                    row.message.id
                                )
                            })?;
                        }
                        _ => {
                            replace_message_membership_and_recompute(
                                &tx,
                                &row.message.account_id,
                                &row.message.thread_id,
                                &row.message.id,
                                &row.folders,
                                &row.labels,
                            )
                            .map_err(|error| {
                                format!(
                                    "replace baseline message membership for {}: {error}",
                                    row.message.id
                                )
                            })?;
                        }
                    }
                    replace_message_keywords(
                        &tx,
                        keyword_provider(provider),
                        &row.message.account_id,
                        &row.message.id,
                        &row.keywords,
                    )
                    .map_err(|error| {
                        format!("replace message keywords for {}: {error}", row.message.id)
                    })?;
                    recompute_thread_keyword_labels(
                        &tx,
                        keyword_provider(provider),
                        &row.message.account_id,
                        &row.message.thread_id,
                    )
                    .map_err(|error| {
                        format!(
                            "recompute thread keyword labels for {}: {error}",
                            row.message.thread_id
                        )
                    })?;
                }

                for thread_id in &thread_ids {
                    let thread_rows = rows
                        .iter()
                        .filter(|row| row.message.thread_id == *thread_id)
                        .collect::<Vec<_>>();
                    let account_id = thread_rows
                        .first()
                        .map(|row| row.message.account_id.as_str())
                        .ok_or_else(|| format!("missing account for thread {thread_id}"))?;
                    let aggregate =
                        compute_thread_aggregate(&tx, account_id, thread_id).map_err(|error| {
                            format!("compute thread aggregate {thread_id}: {error}")
                        })?;
                    let is_important = if matches!(
                        provider,
                        BifrostProviderKind::Jmap
                            | BifrostProviderKind::Graph
                            | BifrostProviderKind::Gmail
                    ) {
                        // Importance is carried on the hydrated row (from
                        // `Message::importance` for real JMAP, from the
                        // `$important` keyword for synthetic fixtures). It is
                        // NOT read from `row.keywords`: `$important` is
                        // `$`-prefixed and stripped by `is_user_visible_keyword`
                        // during hydration, so a keyword-string probe would
                        // always be false on the real path.
                        Some(thread_rows.iter().any(|row| row.is_important))
                    } else {
                        None
                    };
                    upsert_thread_aggregate(
                        &tx,
                        account_id,
                        thread_id,
                        &aggregate,
                        is_important,
                        None,
                    )
                    .map_err(|error| format!("upsert thread aggregate {thread_id}: {error}"))?;
                    if matches!(
                        provider,
                        BifrostProviderKind::Jmap
                            | BifrostProviderKind::Graph
                            | BifrostProviderKind::Gmail
                    ) && let Some(user_emails) = &user_emails
                    {
                        maybe_update_chat_state(&tx, account_id, thread_id, user_emails)
                            .map_err(|error| format!("update chat state {thread_id}: {error}"))?;
                    }
                }

                if matches!(
                    provider,
                    BifrostProviderKind::Jmap
                        | BifrostProviderKind::Graph
                        | BifrostProviderKind::Gmail
                ) && !deleted_ids.is_empty()
                {
                    delete_messages_and_cleanup_threads(&tx, &account_id, &deleted_ids)
                        .map_err(|error| format!("delete destroyed messages: {error}"))?;
                }

                assert_no_foreign_key_violations(&tx)?;
                tx.commit().map_err(|error| error.to_string())?;
                Ok(PersistAffected {
                    thread_ids: thread_ids.into_iter().collect(),
                    message_ids,
                })
            }
        })
        .await?;

    store_message_bodies(
        &stores.body_store,
        &rows,
        provider.as_str(),
        |row| row.message.id.as_str(),
        |row| row.body_html.as_ref(),
        |row| row.body_text.as_ref(),
    )
    .await;
    let inline_images = rows
        .iter()
        .flat_map(|row| row.inline_images.clone())
        .collect::<Vec<_>>();
    store_inline_images(&stores.inline_images, inline_images, provider.as_str()).await;
    let search_docs = rows
        .iter()
        .map(|row| row.search_document.clone())
        .collect::<Vec<_>>();
    index_search_documents(&stores.search, search_docs, provider.as_str()).await;
    if !deleted_ids.is_empty() {
        if let Err(error) = stores.body_store.delete(deleted_ids.clone()).await {
            log::warn!("Failed to delete bifrost bodies: {error}");
        }
        if let Err(error) = stores.search.delete_messages_batch(deleted_ids).await {
            log::warn!("Failed to delete bifrost search documents: {error}");
        }
    }

    Ok(affected)
}

fn keyword_provider(provider: BifrostProviderKind) -> KeywordProvider {
    match provider {
        BifrostProviderKind::Imap => KeywordProvider::Imap,
        BifrostProviderKind::Gmail | BifrostProviderKind::Graph | BifrostProviderKind::Jmap => {
            KeywordProvider::Jmap
        }
    }
}

fn label_write_row(label: &LabelKind, account_id: &str) -> LabelWriteRow {
    let id = label.storage_id();
    let (name, sort_order, is_undeletable) = match label {
        LabelKind::GraphCategory(_) => (
            id.strip_prefix("cat:").unwrap_or(id.as_str()).to_string(),
            None,
            false,
        ),
        LabelKind::GraphImportance(level) => (
            level.display_name().to_string(),
            Some(level.sort_order()),
            true,
        ),
        _ => (id.clone(), None, false),
    };
    LabelWriteRow {
        id,
        account_id: account_id.to_string(),
        name,
        visible: None,
        sort_order,
        server_color_bg: None,
        server_color_fg: None,
        user_color_bg: None,
        user_color_fg: None,
        is_undeletable,
    }
}

fn insert_gmail_reaction(
    tx: &db::db::WriteTxn<'_>,
    account_id: &str,
    row: &ConsumerMessageRow,
) -> Result<(), String> {
    let Some(emoji) = row.reaction_emoji.as_deref() else {
        return Ok(());
    };
    let Some(in_reply_to) = row.message.in_reply_to_header.as_deref() else {
        log::warn!(
            "Gmail reaction message {} has no In-Reply-To header, skipping",
            row.message.id
        );
        return Ok(());
    };
    let Some(reactor_email) = row.message.from_address.as_deref() else {
        return Ok(());
    };

    let target_message_id: Option<String> = tx
        .query_row(
            "SELECT id FROM messages WHERE message_id_header = ?1 AND account_id = ?2 LIMIT 1",
            rusqlite::params![in_reply_to, account_id],
            |lookup| lookup.get("id"),
        )
        .ok();
    let target_id = target_message_id.as_deref().unwrap_or(in_reply_to);

    tx.execute(
        "INSERT INTO message_reactions \
         (message_id, account_id, reactor_email, reactor_name, reaction_type, reacted_at, source) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'gmail_mime') \
         ON CONFLICT(message_id, account_id, reactor_email, reaction_type) DO UPDATE SET \
           reactor_name = ?4, reacted_at = ?6",
        rusqlite::params![
            target_id,
            account_id,
            reactor_email,
            row.message.from_name,
            emoji,
            row.message.date,
        ],
    )
    .map_err(|error| format!("insert Gmail reaction: {error}"))?;
    Ok(())
}

fn assert_no_foreign_key_violations(tx: &db::db::WriteTxn<'_>) -> Result<(), String> {
    let mut stmt = tx
        .prepare("PRAGMA foreign_key_check")
        .map_err(|error| format!("prepare foreign_key_check: {error}"))?;
    let mut rows = stmt
        .query([])
        .map_err(|error| format!("run foreign_key_check: {error}"))?;
    if let Some(row) = rows
        .next()
        .map_err(|error| format!("read foreign_key_check: {error}"))?
    {
        let table: String = row.get(0).unwrap_or_else(|_| "<unknown>".to_string());
        let rowid: i64 = row.get(1).unwrap_or(-1);
        let parent: String = row.get(2).unwrap_or_else(|_| "<unknown>".to_string());
        let fkid: i64 = row.get(3).unwrap_or(-1);
        return Err(format!(
            "foreign key violation table={table} rowid={rowid} parent={parent} fkid={fkid}"
        ));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::super::hydrate::hydrate_change_to_message_insert_row_offline;
    use super::super::{BifrostConsumerStores, BifrostProviderKind};
    use super::persist;
    use bifrost_types::{Change, ObjectChange, ObjectChangeKind, ObjectId};
    use service_state::{
        BodyStoreWriteState, InlineImageStoreWriteState, SearchWriteHandle, WriteDbState,
    };
    use tokio::sync::mpsc;

    fn state() -> (WriteDbState, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let pool = db::db::open_writer_pool(tmp.path()).unwrap();
        pool.with_write_sync(|conn| {
            conn.execute(
                "INSERT INTO accounts (id, email, provider) VALUES ('acc', 'a@example.com', 'jmap')",
                [],
            )
            .unwrap();
            Ok(())
        })
        .unwrap();
        (WriteDbState::from_pool(pool), tmp)
    }

    fn stores(state: WriteDbState, tmp: &tempfile::TempDir) -> BifrostConsumerStores {
        let (search_tx, _search_rx) = mpsc::channel(8);
        BifrostConsumerStores {
            db: state,
            body_store: BodyStoreWriteState::init(tmp.path()).unwrap(),
            inline_images: InlineImageStoreWriteState::init(tmp.path()).unwrap(),
            search: SearchWriteHandle::from_sender(search_tx),
        }
    }

    fn synthetic_change(
        id: &str,
        thread_id: &str,
        folders: &[&str],
        labels: &[&str],
        keywords: &[&str],
    ) -> Change {
        let synthetic = super::super::hydrate::SyntheticMessage {
            id: id.to_string(),
            thread_id: Some(thread_id.to_string()),
            subject: format!("subject {id}"),
            from_addr: "peer@example.com".to_string(),
            to_addrs: vec!["me@example.com".to_string()],
            folder_ids: folders.iter().map(|f| (*f).to_string()).collect(),
            label_ids: labels.iter().map(|l| (*l).to_string()).collect(),
            keywords: keywords.iter().map(|k| (*k).to_string()).collect(),
            raw_body: b"hello world".to_vec(),
            degraded_body: false,
            forced_outcome: None,
            reaction_emoji: None,
        };
        let encoded = super::super::hydrate::encode_synthetic_message(&synthetic).unwrap();
        Change::ObjectChange(ObjectChange {
            id: ObjectId(encoded),
            kind: ObjectChangeKind::Created,
        })
    }

    async fn count(state: &WriteDbState, sql: &'static str, id: &'static str) -> i64 {
        let id = id.to_string();
        state
            .with_read(move |conn| {
                conn.query_row(sql, rusqlite::params![id], |row| row.get::<_, i64>(0))
                    .map_err(|e| e.to_string())
            })
            .await
            .unwrap()
    }

    /// Spec 6.1 `consumer_membership_baseline`: a multi-message thread with
    /// folders / labels / keywords must produce REAL, consistent rows in
    /// every membership table plus the recomputed thread rollup - not merely
    /// land the message rows.
    #[tokio::test(flavor = "multi_thread")]
    async fn consumer_membership_baseline() {
        let (state, _tmp) = state();
        let changes = [
            synthetic_change("m1", "t1", &["INBOX"], &["kw:project"], &["project"]),
            synthetic_change("m2", "t1", &["INBOX"], &["kw:project"], &["project"]),
        ];
        let rows: Vec<_> = changes
            .iter()
            .map(|change| {
                match hydrate_change_to_message_insert_row_offline(
                    "acc",
                    BifrostProviderKind::Jmap,
                    change,
                ) {
                    super::super::hydrate::HydratedChange::Message(row, _) => *row,
                    other => panic!("unexpected result {other:?}"),
                }
            })
            .collect();
        // The hydrated rows must actually carry the membership content; a
        // baseline that silently dropped folders/labels/keywords would still
        // land message rows, so assert the content survived hydration.
        assert!(
            rows.iter().all(|row| !row.folders.is_empty()),
            "folders hydrated"
        );

        let stores = stores(state.clone(), &_tmp);
        let affected = persist(&stores, "acc", BifrostProviderKind::Jmap, &rows, &[])
            .await
            .unwrap();
        assert_eq!(affected.message_ids, vec!["m1", "m2"]);
        assert_eq!(affected.thread_ids, vec!["t1"]);

        // Every message row landed.
        assert_eq!(
            count(
                &state,
                "SELECT COUNT(*) FROM messages WHERE thread_id = ?1",
                "t1"
            )
            .await,
            2,
            "both message rows landed"
        );
        // Each message produced a message_folders row from its synthetic folder.
        assert_eq!(
            count(
                &state,
                "SELECT COUNT(*) FROM message_folders mf JOIN messages m \
                 ON m.id = mf.message_id WHERE m.thread_id = ?1",
                "t1"
            )
            .await,
            2,
            "each message produced a message_folders row"
        );
        // Each message produced a message_keywords row.
        assert_eq!(
            count(
                &state,
                "SELECT COUNT(*) FROM message_keywords mk JOIN messages m \
                 ON m.id = mk.message_id WHERE m.thread_id = ?1",
                "t1"
            )
            .await,
            2,
            "each message produced a message_keywords row"
        );
        // The thread folder rollup recomputed from the message rows.
        assert!(
            count(
                &state,
                "SELECT COUNT(*) FROM thread_folders WHERE thread_id = ?1",
                "t1"
            )
            .await
                >= 1,
            "thread_folders rollup recomputed"
        );
        // The thread label rollup (keyword-derived) recomputed.
        assert!(
            count(
                &state,
                "SELECT COUNT(*) FROM thread_labels WHERE thread_id = ?1",
                "t1"
            )
            .await
                >= 1,
            "thread_labels rollup recomputed"
        );
    }

    // The JMAP byte-identical membership-AND-threading equality gate
    // (`jmap_consumer_membership_equals_legacy`) lives in `golden_test.rs`:
    // it drives the full hydrate -> persist -> post_persist path against a
    // frozen golden captured from the legacy JMAP persist semantics, rather
    // than the consumer-only row counts this module used to assert
    // (B3a-cut-jmap 4.0 / 6.1).
}
