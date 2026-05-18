use std::collections::HashMap;

use rusqlite::OptionalExtension;

use db::db::{ReadDbState, WriteConn};
use db::db::queries_extra::{
    AttachmentInsertRow, FolderWriteRow, MessageInsertRow, insert_attachments,
    insert_folders_batch, insert_messages, recompute_thread_read_starred, set_message_imap_flags,
    sync_thread_read_starred_labels,
};
use search::SearchDocument;
use service_state::{BodyStoreWriteState, InlineImageStoreWriteState, SearchWriteHandle, WriteDbState};
use seen::MessageAddresses;
use store::inline_image_store::InlineImage;
use crate::keyword_membership::{
    KeywordProvider, recompute_thread_keyword_labels, replace_message_keywords,
};
use crate::persistence;

use super::convert::ConvertedMessage;
use super::folder_mapper::map_folder_to_folder;
use super::types::{FlagChange, ImapFolder};

// ---------------------------------------------------------------------------
// Constants (matching TS imapSyncFetch.ts)
// ---------------------------------------------------------------------------

/// Messages to fetch per IMAP FETCH command.
pub(crate) const CHUNK_SIZE: usize = 200;

// ---------------------------------------------------------------------------
// DB insert helper (moveable across threads for spawn_blocking)
// ---------------------------------------------------------------------------

/// Data needed to insert a message into the DB, extractable from ConvertedMessage.
pub(crate) struct DbInsertData {
    id: String,
    account_id: String,
    thread_id: String,
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
    is_replied: bool,
    is_forwarded: bool,
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
    mdn_requested: bool,
    keyword_categories: Vec<String>,
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
    content_hash: Option<db::blob_hash::BlobHash>,
}

impl DbInsertData {
    pub(crate) fn from_converted(c: &ConvertedMessage, account_id: &str) -> Self {
        let imap = &c.imap_msg;
        Self {
            id: c.id.clone(),
            account_id: account_id.to_string(),
            thread_id: c.id.clone(),
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
            is_replied: imap.is_replied,
            is_forwarded: imap.is_forwarded,
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
            mdn_requested: imap.mdn_requested,
            keyword_categories: imap.keyword_categories.clone(),
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
                    content_hash: att.content_hash,
                })
                .collect(),
        }
    }

    fn adopt_existing_identity(&mut self, id: &str, thread_id: String) {
        self.id = id.to_string();
        self.thread_id = thread_id;
        for att in &mut self.attachments {
            att.message_id = id.to_string();
            att.att_id = format!("{}_{}", id, att.part_id);
        }
    }

    pub(crate) fn insert(&self, tx: &db::db::WriteTxn<'_>) -> Result<(), String> {
        if self.thread_id == self.id {
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
        }

        let invite_idx = self.attachments.iter().position(|att| {
            common::email_parsing::is_calendar_content_type(&att.mime_type)
        });
        let invite_method = invite_idx.and_then(|i| {
            common::email_parsing::extract_imip_method(&self.attachments[i].mime_type)
        });

        let message = MessageInsertRow {
            id: self.id.clone(),
            account_id: self.account_id.clone(),
            thread_id: self.thread_id.clone(),
            from_address: self.from_address.clone(),
            from_name: self.from_name.clone(),
            to_addresses: self.to_addresses.clone(),
            cc_addresses: self.cc_addresses.clone(),
            bcc_addresses: self.bcc_addresses.clone(),
            reply_to: self.reply_to.clone(),
            subject: self.subject.clone(),
            snippet: self.snippet.clone(),
            date: self.date,
            is_read: self.is_read,
            is_starred: self.is_starred,
            is_replied: self.is_replied,
            is_forwarded: self.is_forwarded,
            raw_size: Some(i64::from(self.raw_size)),
            internal_date: Some(self.date),
            list_unsubscribe: self.list_unsubscribe.clone(),
            list_unsubscribe_post: self.list_unsubscribe_post.clone(),
            auth_results: self.auth_results.clone(),
            message_id_header: self.message_id_header.clone(),
            references_header: self.references_header.clone(),
            in_reply_to_header: self.in_reply_to_header.clone(),
            body_cached: self.has_body,
            mdn_requested: self.mdn_requested,
            is_reaction: false,
            imap_uid: Some(i64::from(self.imap_uid)),
            imap_folder: Some(self.imap_folder.clone()),
            has_meeting_invite: invite_idx.is_some(),
            meeting_invite_method: invite_method,
            // The iCalendar UID lives inside the attachment payload; populating
            // it requires fetching + parsing the bytes. Deferred until the
            // body/attachment cache is available at message-insert time.
            meeting_invite_uid: None,
        };
        insert_messages(tx, &[message])?;

        let attachments: Vec<AttachmentInsertRow> = self
            .attachments
            .iter()
            .map(|att| AttachmentInsertRow {
                id: att.att_id.clone(),
                message_id: att.message_id.clone(),
                account_id: att.account_id.clone(),
                filename: Some(att.filename.clone()),
                mime_type: Some(att.mime_type.clone()),
                size: Some(i64::from(att.size)),
                remote_attachment_id: Some(att.part_id.clone()),
                content_hash: att.content_hash,
                content_id: att.content_id.clone(),
                is_inline: att.is_inline,
            })
            .collect();
        insert_attachments(tx, &attachments)?;

        Ok(())
    }
}

fn existing_imap_message_identity(
    conn: &db::db::WriteTxn<'_>,
    account_id: &str,
    folder: &str,
    uid: u32,
) -> Result<Option<(String, String)>, String> {
    conn.query_row(
        "SELECT id, thread_id FROM messages \
         WHERE account_id = ?1 AND imap_folder = ?2 AND imap_uid = ?3 \
         LIMIT 1",
        rusqlite::params![account_id, folder, i64::from(uid)],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(|e| format!("lookup existing IMAP message: {e}"))
}

// ---------------------------------------------------------------------------
// Shared helper: store a chunk of converted messages to DB + body store + search
// ---------------------------------------------------------------------------

/// Store a chunk of converted messages to all four subsystems.
pub(crate) async fn store_chunk(
    db: &WriteDbState,
    read_db: &ReadDbState,
    body_store: &BodyStoreWriteState,
    inline_images: &InlineImageStoreWriteState,
    search: &SearchWriteHandle,
    chunk: &[ConvertedMessage],
    account_id: &str,
) -> Result<(), String> {
    // 1. Store to DB (blocking)
    let mut db_data: Vec<DbInsertData> = chunk
        .iter()
        .map(|c| DbInsertData::from_converted(c, account_id))
        .collect();
    let resolved_ids = db.with_write(move |conn| {
        let tx = conn
                .transaction()
            .map_err(|e| format!("begin tx: {e}"))?;
        let mut resolved_ids = Vec::with_capacity(db_data.len());
        for d in &mut db_data {
            let original_id = d.id.clone();
            if let Some((existing_id, existing_thread_id)) =
                existing_imap_message_identity(&tx, &d.account_id, &d.imap_folder, d.imap_uid)?
            {
                d.adopt_existing_identity(&existing_id, existing_thread_id);
            }
            resolved_ids.push((original_id, d.id.clone()));
            d.insert(&tx)?;
            replace_message_keywords(
                &tx,
                KeywordProvider::Imap,
                &d.account_id,
                &d.id,
                &d.keyword_categories,
            )?;
        }
        tx.commit().map_err(|e| format!("commit: {e}"))?;
        Ok(resolved_ids)
    })
    .await?;
    let resolved_ids: HashMap<String, String> = resolved_ids.into_iter().collect();

    // 2-5. Fire-and-forget post-DB writes - all independent, run concurrently.
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
        store_bodies(body_store, chunk, &resolved_ids),
        store_inline_images(inline_images, chunk),
        index_messages(search, chunk, account_id, &resolved_ids),
        seen::ingest_from_messages(read_db, account_id, &addr_data),
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

fn resolved_message_id(resolved_ids: &HashMap<String, String>, original_id: &str) -> String {
    resolved_ids.get(original_id).cloned().unwrap_or_else(|| {
        // store_chunk populates every entry. The fallback keeps these helpers
        // tolerant of focused tests that call them directly with partial maps.
        original_id.to_string()
    })
}

/// Store bodies in the body store (compressed, separate DB).
/// Fire-and-forget pattern - errors are logged but don't fail the sync.
pub async fn store_bodies(
    body_store: &BodyStoreWriteState,
    messages: &[ConvertedMessage],
    resolved_ids: &HashMap<String, String>,
) {
    let bodies: Vec<store::body_store::MessageBody> = messages
        .iter()
        .filter(|m| m.imap_msg.body_html.is_some() || m.imap_msg.body_text.is_some())
        .map(|m| store::body_store::MessageBody {
            message_id: resolved_message_id(resolved_ids, &m.id),
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
    inline_images: &InlineImageStoreWriteState,
    messages: &[ConvertedMessage],
) {
    let images: Vec<InlineImage> = messages
        .iter()
        .flat_map(|m| &m.imap_msg.attachments)
        .filter_map(|att| {
            let data = att.inline_data.as_ref()?;
            let hash = att.content_hash.as_ref()?;
            Some(InlineImage {
                content_hash: hash.to_hex(),
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
pub async fn index_messages(
    search: &SearchWriteHandle,
    messages: &[ConvertedMessage],
    account_id: &str,
    resolved_ids: &HashMap<String, String>,
) {
    let docs: Vec<SearchDocument> = messages
        .iter()
        .map(|m| {
            let message_id = resolved_message_id(resolved_ids, &m.id);
            SearchDocument {
                message_id: message_id.clone(),
                account_id: account_id.to_string(),
                thread_id: message_id, // placeholder, updated after threading
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
                // Phase 7: provider crates emit thin docs with no
                // attachment fragments; the writer task's apply-time
                // enrichment populates `attachments` from
                // attachment_extracted_text in 7-3c.
                attachments: Vec::new(),
            }
        })
        .collect();

    if let Err(e) = search.index_messages_batch(docs).await {
        log::warn!("Failed to index messages in tantivy: {e}");
    }
}

// ---------------------------------------------------------------------------
// IMAP-specific sync DB operations
// ---------------------------------------------------------------------------

/// Sync IMAP folders to the labels table.
pub fn sync_folders_to_folders(
    conn: &WriteConn<'_>,
    account_id: &str,
    folders: &[&ImapFolder],
) -> Result<(), String> {
    let tx = conn
                .transaction()
        .map_err(|e| format!("begin label tx: {e}"))?;

    // Build path → folder_id map for parent resolution
    let path_to_folder_id: std::collections::HashMap<&str, String> = folders
        .iter()
        .map(|f| {
            let mapping = map_folder_to_folder(f)?;
            Ok::<_, String>((f.path.as_str(), mapping.folder_id))
        })
        .collect::<Result<_, _>>()?;

    let rows: Vec<FolderWriteRow> = folders
        .iter()
        .map(|folder| {
            let mapping = map_folder_to_folder(folder)?;
            let parent_id =
                derive_imap_parent_folder_id(&folder.path, &folder.delimiter, &path_to_folder_id);

            Ok::<FolderWriteRow, String>(FolderWriteRow {
                id: mapping.folder_id,
                account_id: account_id.to_string(),
                name: mapping.folder_name,
                visible: None,
                sort_order: None,
                imap_folder_path: Some(folder.raw_path.clone()),
                imap_special_use: folder.special_use.clone(),
                namespace_type: None,
                parent_id,
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
                // Only block deletion when the server advertised SPECIAL-USE
                // for this mailbox. A user-created folder that happens to be
                // named "Drafts" on a server without SPECIAL-USE was treated
                // as a system folder by the previous name-fallback rule,
                // blocking the user from deleting their own folder. The
                // mapper's name-fallback still routes such folders to
                // canonical IDs at the storage layer; we only refuse to
                // mark them as system here. `is_undeletable` should reflect
                // a provider's system classification, not Ratatoskr's
                // role-name inference.
                is_undeletable: folder.special_use.is_some(),
            })
        })
        .collect::<Result<_, _>>()?;

    insert_folders_batch(&tx, &rows)?;
    tx.commit().map_err(|e| format!("commit folders: {e}"))?;
    Ok(())
}

/// Derive a parent folder ID for an IMAP folder by splitting its path on the
/// hierarchy delimiter and looking up the parent path in the path-to-folder map.
///
/// For example, with delimiter `/` and path `Work/Projects/Active`, the parent
/// path is `Work/Projects`. If that path exists in the map, its folder ID is
/// returned.
fn derive_imap_parent_folder_id(
    path: &str,
    delimiter: &str,
    path_to_folder_id: &std::collections::HashMap<&str, String>,
) -> Option<String> {
    if delimiter.is_empty() {
        return None;
    }
    let last_delim = path.rfind(delimiter)?;
    if last_delim == 0 {
        return None;
    }
    let parent_path = &path[..last_delim];
    path_to_folder_id.get(parent_path).cloned()
}

/// Update folder sync state in DB.
pub fn upsert_folder_sync_state(
    conn: &WriteConn<'_>,
    account_id: &str,
    folder_path: &str,
    uidvalidity: u32,
    last_uid: u32,
    last_sync_at: i64,
    modseq: Option<u64>,
) -> Result<(), String> {
    #[allow(clippy::cast_possible_wrap)]
    let modseq_i64 = modseq.map(|v| v as i64);
    conn.execute(
        "INSERT OR REPLACE INTO folder_sync_state \
         (account_id, folder_path, uidvalidity, last_uid, modseq, last_sync_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            account_id,
            folder_path,
            uidvalidity,
            last_uid,
            modseq_i64,
            last_sync_at
        ],
    )
    .map_err(|e| format!("upsert folder sync state: {e}"))?;
    Ok(())
}

/// Get all folder sync states for an account.
pub fn get_all_folder_sync_states(
    conn: &WriteConn<'_>,
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
            #[allow(clippy::cast_sign_loss)]
            Ok(FolderSyncState {
                folder_path: row.get("folder_path")?,
                uidvalidity: row.get("uidvalidity")?,
                last_uid: row.get("last_uid")?,
                modseq: row.get::<_, Option<i64>>("modseq")?.map(|v| v as u64),
                _last_sync_at: row.get("last_sync_at")?,
            })
        })
        .map_err(|e| format!("query folder sync states: {e}"))?;

    let mut states = Vec::new();
    for row in rows {
        states.push(row.map_err(|e| format!("read row: {e}"))?);
    }
    Ok(states)
}

pub fn get_local_message_counts_by_folder(
    conn: &WriteConn<'_>,
    account_id: &str,
) -> Result<HashMap<String, u32>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT imap_folder, COUNT(*) FROM messages \
             WHERE account_id = ?1 AND imap_folder IS NOT NULL \
             GROUP BY imap_folder",
        )
        .map_err(|e| format!("prepare local folder message counts: {e}"))?;

    let rows = stmt
        .query_map(rusqlite::params![account_id], |row| {
            let folder: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            Ok((folder, u32::try_from(count).unwrap_or(u32::MAX)))
        })
        .map_err(|e| format!("query local folder message counts: {e}"))?;

    let mut counts = HashMap::new();
    for row in rows {
        let (folder, count) = row.map_err(|e| format!("read folder message count row: {e}"))?;
        counts.insert(folder, count);
    }
    Ok(counts)
}

pub struct FolderSyncState {
    pub folder_path: String,
    pub uidvalidity: Option<u32>,
    pub last_uid: u32,
    pub modseq: Option<u64>,
    pub _last_sync_at: Option<i64>,
}

/// Batch-update message flags from CONDSTORE CHANGEDSINCE or full FLAGS results.
///
/// Matches messages by `(account_id, imap_folder, imap_uid)` - the indexed
/// columns - and updates message-state booleans plus per-message keyword
/// membership. Thread-level `kw:` labels are then recomputed from that
/// per-message table so removed keywords do not stick forever.
pub fn apply_flag_changes(
    conn: &WriteConn<'_>,
    account_id: &str,
    folder: &str,
    changes: &[FlagChange],
) -> Result<u64, String> {
    if changes.is_empty() {
        return Ok(0);
    }

    let tx = conn
                .transaction()
        .map_err(|e| format!("flag change tx: {e}"))?;

    let mut updated = 0u64;
    let mut affected_threads: std::collections::HashSet<String> = std::collections::HashSet::new();

    for change in changes {
        // Update message flags via shared db helper
        let count =
            set_message_imap_flags(
                &tx,
                account_id,
                folder,
                i64::from(change.uid),
                change.is_read,
                change.is_starred,
                change.is_replied,
                change.is_forwarded,
            )?;
        updated += count as u64;

        if let Some((message_id, tid)) =
            existing_imap_message_identity(&tx, account_id, folder, change.uid)?
        {
            replace_message_keywords(
                &tx,
                KeywordProvider::Imap,
                account_id,
                &message_id,
                &change.keywords,
            )?;
            affected_threads.insert(tid);
        }
    }

    // Reaggregate thread-level state from constituent messages.
    for tid in &affected_threads {
        recompute_thread_read_starred(&tx, account_id, tid)?;
        sync_thread_read_starred_labels(&tx, account_id, tid)?;
        recompute_thread_keyword_labels(&tx, KeywordProvider::Imap, account_id, tid)?;
    }

    tx.commit()
        .map_err(|e| format!("flag change commit: {e}"))?;
    Ok(updated)
}

/// Get the timestamp of the last deletion detection check for a folder.
pub fn get_last_deletion_check_at(
    conn: &WriteConn<'_>,
    account_id: &str,
    folder_path: &str,
) -> Result<Option<i64>, String> {
    conn.query_row(
        "SELECT last_deletion_check_at FROM folder_sync_state \
         WHERE account_id = ?1 AND folder_path = ?2",
        rusqlite::params![account_id, folder_path],
        |row| row.get("last_deletion_check_at"),
    )
    .map_err(|e| format!("get last deletion check: {e}"))
}

/// Update the last deletion detection check timestamp for a folder.
pub fn set_last_deletion_check_at(
    conn: &WriteConn<'_>,
    account_id: &str,
    folder_path: &str,
    timestamp: i64,
) -> Result<(), String> {
    conn.execute(
        "UPDATE folder_sync_state SET last_deletion_check_at = ?1 \
         WHERE account_id = ?2 AND folder_path = ?3",
        rusqlite::params![timestamp, account_id, folder_path],
    )
    .map_err(|e| format!("set last deletion check: {e}"))?;
    Ok(())
}

/// Get all locally-cached IMAP UIDs for a given folder+account.
///
/// Returns `(message_id, imap_uid)` pairs so callers can identify which
/// messages to delete by their local ID.
pub fn get_local_uids_for_folder(
    conn: &WriteConn<'_>,
    account_id: &str,
    folder_path: &str,
) -> Result<Vec<(String, u32)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, imap_uid FROM messages \
             WHERE account_id = ?1 AND imap_folder = ?2 AND imap_uid IS NOT NULL",
        )
        .map_err(|e| format!("prepare get_local_uids: {e}"))?;

    let rows = stmt
        .query_map(rusqlite::params![account_id, folder_path], |row| {
            Ok((
                row.get::<_, String>("id")?,
                u32::try_from(row.get::<_, i64>("imap_uid")?).unwrap_or(0),
            ))
        })
        .map_err(|e| format!("query local uids: {e}"))?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|e| format!("read uid row: {e}"))?);
    }
    Ok(result)
}

#[derive(Debug, Clone)]
pub struct LocalImapFlags {
    pub uid: u32,
    pub is_read: bool,
    pub is_starred: bool,
    pub is_replied: bool,
    pub is_forwarded: bool,
    pub keywords: Vec<String>,
}

/// Get locally cached flags for all messages in a folder.
pub fn get_local_flags_for_folder(
    conn: &WriteConn<'_>,
    account_id: &str,
    folder_path: &str,
) -> Result<Vec<LocalImapFlags>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, imap_uid, is_read, is_starred, is_replied, is_forwarded FROM messages \
             WHERE account_id = ?1 AND imap_folder = ?2 AND imap_uid IS NOT NULL",
        )
        .map_err(|e| format!("prepare get_local_flags: {e}"))?;

    let rows = stmt
        .query_map(rusqlite::params![account_id, folder_path], |row| {
            Ok((
                row.get::<_, String>("id")?,
                u32::try_from(row.get::<_, i64>("imap_uid")?).unwrap_or(0),
                row.get::<_, bool>("is_read")?,
                row.get::<_, bool>("is_starred")?,
                row.get::<_, bool>("is_replied")?,
                row.get::<_, bool>("is_forwarded")?,
            ))
        })
        .map_err(|e| format!("query local flags: {e}"))?;

    let mut keywords_by_message: HashMap<String, Vec<String>> = HashMap::new();
    let mut kw_stmt = conn
        .prepare(
            "SELECT m.id, mk.keyword \
             FROM messages m \
             JOIN message_keywords mk ON mk.account_id = m.account_id AND mk.message_id = m.id \
             WHERE m.account_id = ?1 AND m.imap_folder = ?2 AND m.imap_uid IS NOT NULL",
        )
        .map_err(|e| format!("prepare local keyword flags: {e}"))?;
    let kw_rows = kw_stmt
        .query_map(rusqlite::params![account_id, folder_path], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("query local keyword flags: {e}"))?;
    for row in kw_rows {
        let (message_id, keyword) = row.map_err(|e| format!("read keyword flag row: {e}"))?;
        keywords_by_message
            .entry(message_id)
            .or_default()
            .push(keyword);
    }

    let mut result = Vec::new();
    for row in rows {
        let (message_id, uid, is_read, is_starred, is_replied, is_forwarded) =
            row.map_err(|e| format!("read flag row: {e}"))?;
        let mut keywords = keywords_by_message.remove(&message_id).unwrap_or_default();
        keywords.sort();
        result.push(LocalImapFlags {
            uid,
            is_read,
            is_starred,
            is_replied,
            is_forwarded,
            keywords,
        });
    }
    Ok(result)
}

/// Remove messages that were deleted on the server and update/remove their
/// parent threads accordingly.
///
/// Returns the list of affected thread IDs (for UI refresh).
pub fn remove_deleted_messages(
    conn: &WriteConn<'_>,
    account_id: &str,
    deleted_message_ids: &[String],
) -> Result<Vec<String>, String> {
    if deleted_message_ids.is_empty() {
        return Ok(vec![]);
    }

    let tx = conn
                .transaction()
        .map_err(|e| format!("deletion tx: {e}"))?;

    let affected_threads =
        persistence::delete_messages_and_cleanup_threads(&tx, account_id, deleted_message_ids)?;

    tx.commit().map_err(|e| format!("deletion commit: {e}"))?;

    log::info!(
        "[sync] Removed {} deleted messages, {} threads affected",
        deleted_message_ids.len(),
        affected_threads.len()
    );

    Ok(affected_threads)
}
