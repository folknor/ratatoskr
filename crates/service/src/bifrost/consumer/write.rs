use std::collections::BTreeSet;

use db::db::queries_extra::{
    LabelWriteRow, compute_thread_aggregate, insert_messages, upsert_labels,
    upsert_thread_aggregate,
};
use provider_sync::consumer_support::{
    FolderWriteRow, KeywordProvider, index_search_documents, insert_folders_batch,
    recompute_thread_keyword_labels, replace_message_keywords,
    replace_message_membership_and_recompute, store_inline_images, store_message_bodies,
};

use super::BifrostConsumerStores;
use super::BifrostProviderKind;
use super::hydrate::ConsumerMessageRow;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PersistAffected {
    pub thread_ids: Vec<String>,
    pub message_ids: Vec<String>,
}

pub async fn persist(
    stores: &BifrostConsumerStores,
    provider: BifrostProviderKind,
    rows: &[ConsumerMessageRow],
) -> Result<PersistAffected, String> {
    if rows.is_empty() {
        return Ok(PersistAffected::default());
    }

    let rows = rows.to_vec();
    let affected = stores
        .db
        .with_write({
            let rows = rows.clone();
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
                insert_messages(&tx, &messages)?;

                let mut thread_ids = BTreeSet::new();
                let mut message_ids = Vec::new();
                for row in &rows {
                    thread_ids.insert(row.message.thread_id.clone());
                    message_ids.push(row.message.id.clone());
                    let label_rows = row
                        .labels
                        .iter()
                        .map(|label| {
                            let id = label.storage_id();
                            LabelWriteRow {
                                id,
                                account_id: row.message.account_id.clone(),
                                name: label.storage_id(),
                                visible: None,
                                sort_order: None,
                                server_color_bg: None,
                                server_color_fg: None,
                                user_color_bg: None,
                                user_color_fg: None,
                                is_undeletable: false,
                            }
                        })
                        .collect::<Vec<_>>();
                    upsert_labels(&tx, &label_rows)?;
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
                    insert_folders_batch(&tx, &folder_rows)?;
                    replace_message_membership_and_recompute(
                        &tx,
                        &row.message.account_id,
                        &row.message.thread_id,
                        &row.message.id,
                        &row.folders,
                        &row.labels,
                    )?;
                    replace_message_keywords(
                        &tx,
                        keyword_provider(provider),
                        &row.message.account_id,
                        &row.message.id,
                        &row.keywords,
                    )?;
                    recompute_thread_keyword_labels(
                        &tx,
                        keyword_provider(provider),
                        &row.message.account_id,
                        &row.message.thread_id,
                    )?;
                }

                for thread_id in &thread_ids {
                    let account_id = rows
                        .iter()
                        .find(|row| row.message.thread_id == *thread_id)
                        .map(|row| row.message.account_id.as_str())
                        .ok_or_else(|| format!("missing account for thread {thread_id}"))?;
                    let aggregate = compute_thread_aggregate(&tx, account_id, thread_id)?;
                    upsert_thread_aggregate(&tx, account_id, thread_id, &aggregate, None, None)?;
                }

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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::super::hydrate::hydrate_change_to_message_insert_row;
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
                match hydrate_change_to_message_insert_row("acc", BifrostProviderKind::Jmap, change)
                {
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
        let affected = persist(&stores, BifrostProviderKind::Jmap, &rows)
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
}
