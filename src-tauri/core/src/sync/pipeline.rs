use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use crate::body_store::BodyStoreState;
use crate::db::DbState;
use crate::inline_image_store::{InlineImage, InlineImageStoreState};
use crate::search::{SearchDocument, SearchState};
use crate::seen_addresses::MessageAddresses;
use crate::threading::ThreadGroup;

use super::convert::ConvertedMessage;
use super::types::MessageMeta;

// ---------------------------------------------------------------------------
// Constants (matching TS imapSyncFetch.ts)
// ---------------------------------------------------------------------------

/// Messages to fetch per IMAP FETCH command.
pub(crate) const CHUNK_SIZE: usize = 200;

/// Number of thread groups to process per transaction.
pub const THREAD_BATCH_SIZE: usize = 100;

// ---------------------------------------------------------------------------
// DB insert helper (moveable across threads for spawn_blocking)
// ---------------------------------------------------------------------------

/// Data needed to insert a message into the DB, extractable from ConvertedMessage.
pub(crate) struct DbInsertData {
    id: String,
    account_id: String,
    from_address: Option<String>,
    from_name: Option<String>,
    to_addresses: Option<String>,
    cc_addresses: Option<String>,
    bcc_addresses: Option<String>,
    reply_to: Option<String>,
    subject: Option<String>,
    snippet: String,
    date: i64,
    is_read: bool,
    is_starred: bool,
    has_attachments: bool,
    raw_size: u32,
    list_unsubscribe: Option<String>,
    list_unsubscribe_post: Option<String>,
    auth_results: Option<String>,
    message_id_header: Option<String>,
    references_header: Option<String>,
    in_reply_to_header: Option<String>,
    imap_uid: u32,
    imap_folder: String,
    has_body: bool,
    attachments: Vec<DbAttachment>,
}

struct DbAttachment {
    att_id: String,
    message_id: String,
    account_id: String,
    filename: String,
    mime_type: String,
    size: u32,
    part_id: String,
    content_id: Option<String>,
    is_inline: bool,
    content_hash: Option<String>,
}

impl DbInsertData {
    pub(crate) fn from_converted(c: &ConvertedMessage, account_id: &str) -> Self {
        let imap = &c.imap_msg;
        Self {
            id: c.id.clone(),
            account_id: account_id.to_string(),
            from_address: imap.from_address.clone(),
            from_name: imap.from_name.clone(),
            to_addresses: imap.to_addresses.clone(),
            cc_addresses: imap.cc_addresses.clone(),
            bcc_addresses: imap.bcc_addresses.clone(),
            reply_to: imap.reply_to.clone(),
            subject: imap.subject.clone(),
            snippet: c.meta.snippet.clone(),
            date: c.meta.date,
            is_read: imap.is_read,
            is_starred: imap.is_starred,
            has_attachments: c.meta.has_attachments,
            raw_size: imap.raw_size,
            list_unsubscribe: imap.list_unsubscribe.clone(),
            list_unsubscribe_post: imap.list_unsubscribe_post.clone(),
            auth_results: imap.auth_results.clone(),
            message_id_header: imap.message_id.clone(),
            references_header: imap.references.clone(),
            in_reply_to_header: imap.in_reply_to.clone(),
            imap_uid: imap.uid,
            imap_folder: imap.folder.clone(),
            has_body: imap.body_html.is_some() || imap.body_text.is_some(),
            attachments: imap
                .attachments
                .iter()
                .map(|att| DbAttachment {
                    att_id: format!("{}_{}", c.id, att.part_id),
                    message_id: c.id.clone(),
                    account_id: account_id.to_string(),
                    filename: att.filename.clone(),
                    mime_type: att.mime_type.clone(),
                    size: att.size,
                    part_id: att.part_id.clone(),
                    content_id: att.content_id.clone(),
                    is_inline: att.is_inline,
                    content_hash: att.content_hash.clone(),
                })
                .collect(),
        }
    }

    pub(crate) fn insert(&self, tx: &rusqlite::Transaction) -> Result<(), String> {
        // Placeholder thread
        tx.execute(
            "INSERT OR REPLACE INTO threads \
             (id, account_id, subject, snippet, last_message_at, message_count, \
              is_read, is_starred, is_important, has_attachments) \
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7, 0, ?8)",
            rusqlite::params![
                self.id,
                self.account_id,
                self.subject,
                self.snippet,
                self.date,
                self.is_read,
                self.is_starred,
                self.has_attachments,
            ],
        )
        .map_err(|e| format!("upsert placeholder thread: {e}"))?;

        // Message (bodies in body store)
        tx.execute(
            "INSERT OR REPLACE INTO messages \
             (id, account_id, thread_id, from_address, from_name, to_addresses, \
              cc_addresses, bcc_addresses, reply_to, subject, snippet, date, \
              is_read, is_starred, raw_size, internal_date, \
              list_unsubscribe, list_unsubscribe_post, auth_results, \
              message_id_header, references_header, in_reply_to_header, \
              imap_uid, imap_folder, body_cached) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, \
                     ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)",
            rusqlite::params![
                self.id,
                self.account_id,
                self.id, // placeholder thread_id = message_id
                self.from_address,
                self.from_name,
                self.to_addresses,
                self.cc_addresses,
                self.bcc_addresses,
                self.reply_to,
                self.subject,
                self.snippet,
                self.date,
                self.is_read,
                self.is_starred,
                self.raw_size,
                self.date, // internal_date
                self.list_unsubscribe,
                self.list_unsubscribe_post,
                self.auth_results,
                self.message_id_header,
                self.references_header,
                self.in_reply_to_header,
                self.imap_uid as i64,
                self.imap_folder,
                if self.has_body { 1i64 } else { 0i64 },
            ],
        )
        .map_err(|e| format!("upsert message: {e}"))?;

        // Attachments
        for att in &self.attachments {
            tx.execute(
                "INSERT INTO attachments \
                 (id, message_id, account_id, filename, mime_type, size, \
                  gmail_attachment_id, content_id, is_inline, content_hash) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
                 ON CONFLICT(id) DO UPDATE SET \
                   filename = ?4, mime_type = ?5, size = ?6, \
                   gmail_attachment_id = ?7, content_id = ?8, is_inline = ?9, \
                   content_hash = COALESCE(?10, attachments.content_hash)",
                rusqlite::params![
                    att.att_id,
                    att.message_id,
                    att.account_id,
                    att.filename,
                    att.mime_type,
                    att.size,
                    att.part_id,
                    att.content_id,
                    att.is_inline,
                    att.content_hash,
                ],
            )
            .map_err(|e| format!("upsert attachment: {e}"))?;
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Shared helper: store a chunk of converted messages to DB + body store + search
// ---------------------------------------------------------------------------

/// Store a chunk of converted messages to all four subsystems.
pub(crate) async fn store_chunk(
    db: &DbState,
    body_store: &BodyStoreState,
    inline_images: &InlineImageStoreState,
    search: &SearchState,
    chunk: &[ConvertedMessage],
    account_id: &str,
) -> Result<(), String> {
    // 1. Store to DB (blocking)
    let db_data: Vec<DbInsertData> = chunk
        .iter()
        .map(|c| DbInsertData::from_converted(c, account_id))
        .collect();
    db.with_conn(move |conn| {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("begin tx: {e}"))?;
        for d in &db_data {
            d.insert(&tx)?;
        }
        tx.commit().map_err(|e| format!("commit: {e}"))?;
        Ok(())
    })
    .await?;

    // 2-5. Fire-and-forget post-DB writes — all independent, run concurrently.
    let addr_data: Vec<ImapAddressData> = chunk
        .iter()
        .map(|c| ImapAddressData {
            from_address: c.imap_msg.from_address.clone(),
            from_name: c.imap_msg.from_name.clone(),
            to_addresses: c.imap_msg.to_addresses.clone(),
            cc_addresses: c.imap_msg.cc_addresses.clone(),
            bcc_addresses: c.imap_msg.bcc_addresses.clone(),
            date: c.meta.date,
        })
        .collect();

    tokio::join!(
        store_bodies(body_store, chunk),
        store_inline_images(inline_images, chunk),
        index_messages(search, chunk, account_id),
        crate::seen_addresses::ingest_from_messages(db, account_id, &addr_data),
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Address data for seen_addresses ingestion (IMAP path)
// ---------------------------------------------------------------------------

struct ImapAddressData {
    from_address: Option<String>,
    from_name: Option<String>,
    to_addresses: Option<String>,
    cc_addresses: Option<String>,
    bcc_addresses: Option<String>,
    date: i64,
}

impl MessageAddresses for ImapAddressData {
    fn sender_address(&self) -> Option<&str> {
        self.from_address.as_deref()
    }
    fn sender_name(&self) -> Option<&str> {
        self.from_name.as_deref()
    }
    fn to_addresses(&self) -> Option<&str> {
        self.to_addresses.as_deref()
    }
    fn cc_addresses(&self) -> Option<&str> {
        self.cc_addresses.as_deref()
    }
    fn bcc_addresses(&self) -> Option<&str> {
        self.bcc_addresses.as_deref()
    }
    fn msg_date_ms(&self) -> i64 {
        self.date
    }
}

// ---------------------------------------------------------------------------
// Body store + search index helpers
// ---------------------------------------------------------------------------

/// Store bodies in the body store (compressed, separate DB).
/// Fire-and-forget pattern — errors are logged but don't fail the sync.
pub async fn store_bodies(body_store: &BodyStoreState, messages: &[ConvertedMessage]) {
    let bodies: Vec<crate::body_store::MessageBody> = messages
        .iter()
        .filter(|m| m.imap_msg.body_html.is_some() || m.imap_msg.body_text.is_some())
        .map(|m| crate::body_store::MessageBody {
            message_id: m.id.clone(),
            body_html: m.imap_msg.body_html.clone(),
            body_text: m.imap_msg.body_text.clone(),
        })
        .collect();

    if bodies.is_empty() {
        return;
    }

    if let Err(e) = body_store.put_batch(bodies).await {
        log::warn!("Failed to store bodies in body store: {e}");
    }
}

/// Store small inline images in the content-addressed blob store. Fire-and-forget.
pub async fn store_inline_images(
    inline_images: &InlineImageStoreState,
    messages: &[ConvertedMessage],
) {
    let images: Vec<InlineImage> = messages
        .iter()
        .flat_map(|m| &m.imap_msg.attachments)
        .filter_map(|att| {
            let data = att.inline_data.as_ref()?;
            let hash = att.content_hash.as_ref()?;
            Some(InlineImage {
                content_hash: hash.clone(),
                data: data.clone(),
                mime_type: att.mime_type.clone(),
            })
        })
        .collect();

    if images.is_empty() {
        return;
    }

    log::debug!("Storing {} inline images in blob store", images.len());
    if let Err(e) = inline_images.put_batch(images).await {
        log::warn!("Failed to store inline images: {e}");
    }
}

/// Index messages in tantivy search. Fire-and-forget.
pub async fn index_messages(search: &SearchState, messages: &[ConvertedMessage], account_id: &str) {
    let docs: Vec<SearchDocument> = messages
        .iter()
        .map(|m| SearchDocument {
            message_id: m.id.clone(),
            account_id: account_id.to_string(),
            thread_id: m.id.clone(), // placeholder, updated after threading
            subject: m.imap_msg.subject.clone(),
            from_name: m.imap_msg.from_name.clone(),
            from_address: m.imap_msg.from_address.clone(),
            to_addresses: m.imap_msg.to_addresses.clone(),
            body_text: m.imap_msg.body_text.clone(),
            snippet: Some(m.meta.snippet.clone()),
            date: m.meta.date / 1000, // tantivy expects seconds
            is_read: m.meta.is_read,
            is_starred: m.meta.is_starred,
            has_attachment: m.meta.has_attachments,
        })
        .collect();

    if let Err(e) = search.index_messages_batch(&docs).await {
        log::warn!("Failed to index messages in tantivy: {e}");
    }
}

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
                "SELECT COUNT(*) FROM pending_operations \
                 WHERE account_id = ?1 AND resource_id = ?2 AND status != 'failed'",
                rusqlite::params![account_id, tid],
                |row| row.get(0),
            )
            .map_err(|e| format!("check pending ops: {e}"))?;
        if count > 0 {
            log::info!("Skipping thread {tid}: has {count} pending local ops");
            skipped.insert(tid.clone());
        }
    }
    Ok(skipped)
}

/// Update folder sync state in DB.
pub fn upsert_folder_sync_state(
    conn: &Connection,
    account_id: &str,
    folder_path: &str,
    uidvalidity: u32,
    last_uid: u32,
    last_sync_at: i64,
) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO folder_sync_state \
         (account_id, folder_path, uidvalidity, last_uid, modseq, last_sync_at) \
         VALUES (?1, ?2, ?3, ?4, NULL, ?5)",
        rusqlite::params![account_id, folder_path, uidvalidity, last_uid, last_sync_at],
    )
    .map_err(|e| format!("upsert folder sync state: {e}"))?;
    Ok(())
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

/// Sync IMAP folders to the labels table.
pub fn sync_folders_to_labels(
    conn: &Connection,
    account_id: &str,
    folders: &[&crate::imap::types::ImapFolder],
) -> Result<(), String> {
    for folder in folders {
        let mapping = super::folder_mapper::map_folder_to_label(folder);
        conn.execute(
            "INSERT OR REPLACE INTO labels \
             (id, account_id, name, type, imap_folder_path, imap_special_use) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                mapping.label_id,
                account_id,
                mapping.label_name,
                mapping.label_type,
                folder.raw_path,
                folder.special_use,
            ],
        )
        .map_err(|e| format!("upsert label: {e}"))?;
    }

    // Ensure UNREAD pseudo-label exists
    conn.execute(
        "INSERT OR IGNORE INTO labels (id, account_id, name, type) VALUES (?1, ?2, 'Unread', 'system')",
        rusqlite::params!["UNREAD", account_id],
    )
    .map_err(|e| format!("upsert UNREAD label: {e}"))?;

    Ok(())
}

/// Get all folder sync states for an account.
pub fn get_all_folder_sync_states(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<FolderSyncState>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT folder_path, uidvalidity, last_uid, modseq, last_sync_at \
             FROM folder_sync_state WHERE account_id = ?1",
        )
        .map_err(|e| format!("prepare folder sync states: {e}"))?;

    let rows = stmt
        .query_map(rusqlite::params![account_id], |row| {
            Ok(FolderSyncState {
                folder_path: row.get(0)?,
                uidvalidity: row.get(1)?,
                last_uid: row.get(2)?,
                _modseq: row.get(3)?,
                _last_sync_at: row.get(4)?,
            })
        })
        .map_err(|e| format!("query folder sync states: {e}"))?;

    let mut states = Vec::new();
    for row in rows {
        states.push(row.map_err(|e| format!("read row: {e}"))?);
    }
    Ok(states)
}

pub struct FolderSyncState {
    pub folder_path: String,
    pub uidvalidity: Option<u32>,
    pub last_uid: u32,
    pub _modseq: Option<i64>,
    pub _last_sync_at: Option<i64>,
}

/// Get thread count for an account (used for recovery detection).
pub fn get_thread_count(conn: &Connection, account_id: &str) -> Result<i64, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM threads WHERE account_id = ?1",
        rusqlite::params![account_id],
        |row| row.get(0),
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
