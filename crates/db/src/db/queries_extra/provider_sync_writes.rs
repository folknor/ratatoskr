//! Per-message / per-thread / per-attachment / public-folder writes that
//! previously lived inline in provider sync paths. Agent-owned scaffold
//! for Phase 1.6 - functions get added here as call sites in
//! `crates/imap/src/sync_pipeline.rs`, `crates/imap/src/public_folders.rs`,
//! `crates/graph/src/public_folder_sync.rs`, and
//! `crates/stores/src/attachment_cache.rs` are routed through `db` APIs.
//!
//! Functions use typed DB capabilities; callers wrap them in the
//! appropriate state helper if they need async dispatch.

use rusqlite::params;

use crate::db::{ReadConn, ReadError, WriteTarget, WriteTxn};

// ---------------------------------------------------------------------------
// messages table
// ---------------------------------------------------------------------------

/// Update IMAP-backed message state on a single message matched by
/// `(account_id, imap_folder, imap_uid)`. Returns the number of rows updated.
#[allow(clippy::too_many_arguments)]
pub fn set_message_imap_flags(
    conn: &WriteTxn<'_>,
    account_id: &str,
    folder: &str,
    imap_uid: i64,
    is_read: bool,
    is_starred: bool,
    is_replied: bool,
    is_forwarded: bool,
) -> Result<usize, String> {
    conn.execute(
        "UPDATE messages SET is_read = ?1, is_starred = ?2, is_replied = ?3, is_forwarded = ?4 \
         WHERE account_id = ?5 AND imap_folder = ?6 AND imap_uid = ?7",
        params![
            is_read,
            is_starred,
            is_replied,
            is_forwarded,
            account_id,
            folder,
            imap_uid
        ],
    )
    .map_err(|e| format!("set_message_imap_flags: {e}"))
}

/// Return the `thread_id` for a message matched by
/// `(account_id, imap_folder, imap_uid)`. Returns `None` if not found.
pub fn get_thread_id_for_imap_uid(
    conn: &ReadConn<'_>,
    account_id: &str,
    folder: &str,
    imap_uid: i64,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT thread_id FROM messages \
         WHERE account_id = ?1 AND imap_folder = ?2 AND imap_uid = ?3",
        params![account_id, folder, imap_uid],
        |row| row.get::<_, String>("thread_id"),
    )
    .map(Some)
    .or_else(|e| match e {
        ReadError::Sql(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        other => Err(format!("get_thread_id_for_imap_uid: {other}")),
    })
}

// ---------------------------------------------------------------------------
// threads table
// ---------------------------------------------------------------------------

/// Recompute `is_read` / `is_starred` for a thread by aggregating non-reaction
/// messages.
///
/// `is_read` becomes the MIN of all constituent message flags (a thread is
/// read only when every message is read). `is_starred` becomes the MAX (starred
/// if any message is starred).
///
/// If a thread has no non-reaction messages, the fallback is "read, not
/// starred". Reaction-only threads should be transient cleanup cases rather
/// than user-visible unread/starred threads.
pub fn recompute_thread_read_starred(
    conn: &WriteTxn<'_>,
    account_id: &str,
    thread_id: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE threads SET \
           is_read    = COALESCE((SELECT MIN(is_read)    FROM messages WHERE account_id = ?1 AND thread_id = ?2 AND is_reaction = 0), 1), \
           is_starred = COALESCE((SELECT MAX(is_starred) FROM messages WHERE account_id = ?1 AND thread_id = ?2 AND is_reaction = 0), 0) \
         WHERE account_id = ?1 AND id = ?2",
        params![account_id, thread_id],
    )
    .map_err(|e| format!("recompute_thread_read_starred: {e}"))?;
    Ok(())
}

/// Remove legacy synthetic message-state rows from `thread_labels`.
///
/// No generation bump: `UNREAD` / `STARRED` are reserved message-state
/// label IDs that the user cannot apply or remove, so they cannot appear
/// as `pending_thread_label_intents` rows. Skipping the bump avoids
/// clearing unrelated overlay rows on threads where this cleanup fires.
pub fn sync_thread_read_starred_labels(
    conn: &WriteTxn<'_>,
    account_id: &str,
    thread_id: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM thread_labels \
         WHERE account_id = ?1 AND thread_id = ?2 AND label_id IN ('UNREAD', 'STARRED')",
        params![account_id, thread_id],
    )
    .map_err(|e| format!("delete legacy thread flag labels: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// attachments table
// ---------------------------------------------------------------------------

/// Cached-attachment lookup result for a single attachments row. Phase 7
/// added `text_indexed_at` (per-row pointer to the matching
/// `attachment_extracted_text.extracted_at`) and `extraction_status` (from
/// the joined `attachment_extracted_text` row, NULL if no row exists yet).
/// `attachment.fetch`'s cache-hit path consults `extraction_status` to
/// decide whether to enqueue extraction: NULL or retry-eligible -> enqueue;
/// permanent (`'indexed'` / `'skipped:<permanent>'`) -> skip.
pub struct AttachmentCacheInfo {
    pub id: String,
    pub remote_attachment_id: Option<String>,
    pub content_hash: Option<crate::blob_hash::BlobHash>,
    pub mime_type: Option<String>,
    pub is_inline: bool,
    pub text_indexed_at: Option<i64>,
    pub extraction_status: Option<String>,
    /// `attachment_blobs.tombstoned_at` for the joined `content_hash`.
    /// Distinguishes a logically-evicted blob (row exists, marker
    /// set) from a live one. `attachment.fetch`'s cache-hit branch
    /// treats `Some` as a miss and falls through to the provider
    /// re-fetch path, which revives the blob via `PackStore::put`.
    /// Without this signal, a fetch after retention eviction or
    /// clear-cache erred with "blob indexed in attachments but
    /// absent from pack store" - the prefetch sweep selects only
    /// `content_hash IS NULL` rows and would never refetch.
    pub blob_tombstoned_at: Option<i64>,
    /// `true` if the `attachment_blobs` row for the joined
    /// `content_hash` exists at all. `false` when the row was
    /// physically reclaimed by GC after a tombstone (post
    /// clear-cache + GC, post window-shrink + GC). Distinguishes
    /// "no row" from "row with tombstoned_at IS NULL", which the
    /// `blob_tombstoned_at` field alone collapses. The cache-hit
    /// branch treats either condition - tombstoned OR absent - as
    /// a miss and falls through to re-fetch.
    pub blob_present: bool,
}

/// Look up an attachment's cache info by message + attachment ID.
///
/// UI callers pass the local `attachments.id`; provider-specific callers can
/// still pass the remote attachment ID.
pub fn find_attachment_cache_info(
    conn: &ReadConn<'_>,
    account_id: &str,
    message_id: &str,
    remote_attachment_id: &str,
) -> Result<Option<AttachmentCacheInfo>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT a.id, a.remote_attachment_id, a.content_hash, \
                    a.mime_type, a.is_inline, a.text_indexed_at, \
                    t.status AS extraction_status, \
                    b.tombstoned_at AS blob_tombstoned_at, \
                    CASE WHEN b.content_hash IS NOT NULL THEN 1 ELSE 0 END AS blob_present \
             FROM attachments a \
             LEFT JOIN attachment_extracted_text t ON t.content_hash = a.content_hash \
             LEFT JOIN attachment_blobs b ON b.content_hash = a.content_hash \
             WHERE a.account_id = ?1 AND a.message_id = ?2 \
               AND (a.id = ?3 OR a.remote_attachment_id = ?3) \
             LIMIT 1",
        )
        .map_err(|e| format!("find_attachment_cache_info prepare: {e}"))?;

    let mut rows = stmt
        .query_map(
            params![account_id, message_id, remote_attachment_id],
            |row| {
                Ok(AttachmentCacheInfo {
                    id: row.get("id")?,
                    remote_attachment_id: row.get("remote_attachment_id")?,
                    content_hash: row.get("content_hash")?,
                    mime_type: row.get("mime_type")?,
                    is_inline: row.get::<_, i64>("is_inline")? != 0,
                    text_indexed_at: row.get("text_indexed_at")?,
                    extraction_status: row.get("extraction_status")?,
                    blob_tombstoned_at: row.get("blob_tombstoned_at")?,
                    blob_present: row.get::<_, i64>("blob_present")? != 0,
                })
            },
        )
        .map_err(|e| format!("find_attachment_cache_info query: {e}"))?;

    match rows.next() {
        Some(Ok(info)) => Ok(Some(info)),
        Some(Err(e)) => Err(format!("find_attachment_cache_info row: {e}")),
        None => Ok(None),
    }
}

/// Record the content hash of an attachment row after its bytes have
/// been persisted in PackStore. Only touches `content_hash`;
/// `attachments.size` is expected to be pre-filled by the sync path.
pub fn update_attachment_cache_fields(
    conn: &impl WriteTarget,
    attachment_id: &str,
    content_hash: &crate::blob_hash::BlobHash,
) -> Result<(), String> {
    conn.execute(
        "UPDATE attachments SET content_hash = ?1 WHERE id = ?2",
        params![content_hash, attachment_id],
    )
    .map_err(|e| format!("update_attachment_cache_fields: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// public_folders table
// ---------------------------------------------------------------------------

/// Parameters for a single `public_folders` upsert row.
pub struct PublicFolderRow {
    pub account_id: String,
    pub folder_id: String,
    pub parent_id: Option<String>,
    pub display_name: String,
    pub folder_class: String,
    pub unread_count: u32,
    pub total_count: u32,
    /// Assume readable until MYRIGHTS says otherwise.
    pub can_read: bool,
    pub can_create_items: bool,
    pub can_modify: bool,
    pub can_delete: bool,
}

/// Insert or update a batch of public folders in the `public_folders` table.
///
/// On conflict the discoverable metadata (display name, counts, can_read) is
/// updated; existing permission overrides survive unless the caller explicitly
/// passes the new values.
pub fn upsert_public_folders(
    conn: &impl WriteTarget,
    rows: &[PublicFolderRow],
) -> Result<(), String> {
    let mut stmt = conn
        .prepare(
            "INSERT INTO public_folders \
             (account_id, folder_id, parent_id, display_name, folder_class, \
              unread_count, total_count, can_create_items, can_modify, can_delete, can_read) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
             ON CONFLICT(account_id, folder_id) DO UPDATE SET \
               parent_id     = excluded.parent_id, \
               display_name  = excluded.display_name, \
               folder_class  = excluded.folder_class, \
               unread_count  = excluded.unread_count, \
               total_count   = excluded.total_count, \
               can_read      = excluded.can_read, \
               can_create_items = excluded.can_create_items, \
               can_modify    = excluded.can_modify, \
               can_delete    = excluded.can_delete",
        )
        .map_err(|e| format!("upsert_public_folders prepare: {e}"))?;

    for r in rows {
        stmt.execute(params![
            r.account_id,
            r.folder_id,
            r.parent_id,
            r.display_name,
            r.folder_class,
            r.unread_count,
            r.total_count,
            r.can_create_items as i32,
            r.can_modify as i32,
            r.can_delete as i32,
            r.can_read as i32,
        ])
        .map_err(|e| format!("upsert_public_folders row {}: {e}", r.folder_id))?;
    }

    Ok(())
}

/// Update the MYRIGHTS-derived permission columns for a single public folder.
pub fn update_public_folder_rights(
    conn: &impl WriteTarget,
    account_id: &str,
    folder_id: &str,
    can_read: bool,
    can_create_items: bool,
    can_modify: bool,
    can_delete: bool,
) -> Result<(), String> {
    conn.execute(
        "UPDATE public_folders \
         SET can_read = ?3, can_create_items = ?4, can_modify = ?5, can_delete = ?6 \
         WHERE account_id = ?1 AND folder_id = ?2",
        params![
            account_id,
            folder_id,
            can_read as i32,
            can_create_items as i32,
            can_modify as i32,
            can_delete as i32,
        ],
    )
    .map_err(|e| format!("update_public_folder_rights: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// public_folder_items table
// ---------------------------------------------------------------------------

/// Parameters for a single `public_folder_items` upsert row.
pub struct PublicFolderItemRow {
    pub account_id: String,
    pub folder_id: String,
    pub item_id: String,
    pub change_key: Option<String>,
    pub subject: Option<String>,
    pub sender_email: Option<String>,
    pub sender_name: Option<String>,
    pub received_at: Option<i64>,
    pub body_preview: Option<String>,
    pub is_read: bool,
    pub item_class: String,
}

/// Insert or update a batch of items in `public_folder_items`.
///
/// Returns `(new_count, updated_count)` where `updated_count` tracks rows
/// whose `change_key` differed from the stored value.
pub fn upsert_public_folder_items(
    conn: &impl WriteTarget,
    rows: &[PublicFolderItemRow],
) -> Result<(usize, usize), String> {
    let mut insert_stmt = conn
        .prepare(
            "INSERT INTO public_folder_items \
             (account_id, folder_id, item_id, change_key, subject, sender_email, sender_name, \
              received_at, body_preview, is_read, item_class) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
             ON CONFLICT(account_id, item_id) DO UPDATE SET \
               change_key   = excluded.change_key, \
               subject      = excluded.subject, \
               is_read      = excluded.is_read, \
               body_preview = excluded.body_preview",
        )
        .map_err(|e| format!("upsert_public_folder_items prepare: {e}"))?;

    let mut exists_stmt = conn
        .prepare(
            "SELECT change_key FROM public_folder_items \
             WHERE account_id = ?1 AND item_id = ?2",
        )
        .map_err(|e| format!("upsert_public_folder_items exists prepare: {e}"))?;

    let mut new_count = 0usize;
    let mut updated_count = 0usize;

    for row in rows {
        let existing_ck: Option<Option<String>> = exists_stmt
            .query_row(params![row.account_id, row.item_id], |r| {
                r.get::<_, Option<String>>("change_key")
            })
            .ok();

        let is_update = match &existing_ck {
            Some(ck) => ck.as_deref() != row.change_key.as_deref(),
            None => false,
        };
        let is_new = existing_ck.is_none();

        insert_stmt
            .execute(params![
                row.account_id,
                row.folder_id,
                row.item_id,
                row.change_key,
                row.subject,
                row.sender_email,
                row.sender_name,
                row.received_at,
                row.body_preview,
                row.is_read as i32,
                row.item_class,
            ])
            .map_err(|e| format!("upsert_public_folder_items row {}: {e}", row.item_id))?;

        if is_new {
            new_count += 1;
        } else if is_update {
            updated_count += 1;
        }
    }

    Ok((new_count, updated_count))
}

/// Delete all `public_folder_items` rows for a folder that are NOT in
/// `server_item_ids`. If `server_item_ids` is empty, deletes everything for
/// the folder. Returns the number of rows deleted.
pub fn delete_stale_public_folder_items(
    conn: &impl WriteTarget,
    account_id: &str,
    folder_id: &str,
    server_item_ids: &[String],
) -> Result<usize, String> {
    if server_item_ids.is_empty() {
        let deleted = conn
            .execute(
                "DELETE FROM public_folder_items WHERE account_id = ?1 AND folder_id = ?2",
                params![account_id, folder_id],
            )
            .map_err(|e| format!("delete_stale_public_folder_items (all): {e}"))?;
        return Ok(deleted);
    }

    let placeholders: Vec<String> = (0..server_item_ids.len())
        .map(|i| format!("?{}", i + 3))
        .collect();
    let sql = format!(
        "DELETE FROM public_folder_items \
         WHERE account_id = ?1 AND folder_id = ?2 AND item_id NOT IN ({})",
        placeholders.join(", ")
    );

    let mut param_vals: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    param_vals.push(Box::new(account_id.to_string()));
    param_vals.push(Box::new(folder_id.to_string()));
    for id in server_item_ids {
        param_vals.push(Box::new(id.clone()));
    }
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        param_vals.iter().map(AsRef::as_ref).collect();

    let deleted = conn
        .execute(&sql, param_refs.as_slice())
        .map_err(|e| format!("delete_stale_public_folder_items: {e}"))?;
    Ok(deleted)
}

/// Delete all `public_folder_items` rows for a folder (used during unpin).
pub fn delete_all_public_folder_items(
    conn: &impl WriteTarget,
    account_id: &str,
    folder_id: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM public_folder_items WHERE account_id = ?1 AND folder_id = ?2",
        params![account_id, folder_id],
    )
    .map_err(|e| format!("delete_all_public_folder_items: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// public_folder_pins table
// ---------------------------------------------------------------------------

/// Pin a public folder for offline sync. Upserts the pin row, setting
/// `sync_enabled = 1` and updating `sync_depth_days`.
pub fn pin_public_folder(
    conn: &impl WriteTarget,
    account_id: &str,
    folder_id: &str,
    sync_depth_days: i32,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO public_folder_pins (account_id, folder_id, sync_enabled, sync_depth_days) \
         VALUES (?1, ?2, 1, ?3) \
         ON CONFLICT(account_id, folder_id) DO UPDATE SET \
           sync_enabled     = 1, \
           sync_depth_days  = excluded.sync_depth_days",
        params![account_id, folder_id, sync_depth_days],
    )
    .map_err(|e| format!("pin_public_folder: {e}"))?;
    Ok(())
}

/// Delete the pin row for a public folder.
pub fn delete_public_folder_pin(
    conn: &impl WriteTarget,
    account_id: &str,
    folder_id: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM public_folder_pins WHERE account_id = ?1 AND folder_id = ?2",
        params![account_id, folder_id],
    )
    .map_err(|e| format!("delete_public_folder_pin: {e}"))?;
    Ok(())
}

/// Return the `sync_depth_days` for a pinned folder. Defaults to 30 if
/// no pin row exists.
pub fn get_public_folder_sync_depth(
    conn: &ReadConn<'_>,
    account_id: &str,
    folder_id: &str,
) -> Result<i32, String> {
    let depth = conn
        .query_row(
            "SELECT sync_depth_days FROM public_folder_pins \
             WHERE account_id = ?1 AND folder_id = ?2",
            params![account_id, folder_id],
            |row| row.get::<_, i32>("sync_depth_days"),
        )
        .unwrap_or(30);
    Ok(depth)
}

/// Return the IDs of all public folders that have `sync_enabled = 1` for an
/// account.
pub fn get_pinned_folder_ids(conn: &ReadConn<'_>, account_id: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT folder_id FROM public_folder_pins \
             WHERE account_id = ?1 AND sync_enabled = 1",
        )
        .map_err(|e| format!("get_pinned_folder_ids prepare: {e}"))?;

    let rows = stmt
        .query_map(params![account_id], |row| row.get::<_, String>("folder_id"))
        .map_err(|e| format!("get_pinned_folder_ids query: {e}"))?;

    let mut ids = Vec::new();
    for row in rows {
        ids.push(row.map_err(|e| format!("get_pinned_folder_ids row: {e}"))?);
    }
    Ok(ids)
}
