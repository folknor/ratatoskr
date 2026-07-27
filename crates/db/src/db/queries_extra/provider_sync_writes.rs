//! Per-message / per-thread / per-attachment writes that
//! previously lived inline in provider sync paths. Agent-owned scaffold
//! for Phase 1.6 - functions get added here as call sites in
//! `crates/imap/src/sync_pipeline.rs` and
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
    pub blob_id: Option<String>,
    pub content_hash: Option<crate::blob_hash::BlobHash>,
    pub mime_type: Option<String>,
    pub size: Option<i64>,
    pub imap_folder: Option<String>,
    pub imap_uid: Option<i64>,
    pub imap_uidvalidity: Option<i64>,
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
            "SELECT a.id, a.remote_attachment_id, a.blob_id, a.content_hash, \
                    a.mime_type, a.size, a.is_inline, a.text_indexed_at, \
                    m.imap_folder, m.imap_uid, m.imap_uidvalidity, \
                    t.status AS extraction_status, \
                    b.tombstoned_at AS blob_tombstoned_at, \
                    CASE WHEN b.content_hash IS NOT NULL THEN 1 ELSE 0 END AS blob_present \
             FROM attachments a \
             LEFT JOIN messages m ON m.account_id = a.account_id AND m.id = a.message_id \
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
                    blob_id: row.get("blob_id")?,
                    content_hash: row.get("content_hash")?,
                    mime_type: row.get("mime_type")?,
                    size: row.get("size")?,
                    imap_folder: row.get("imap_folder")?,
                    imap_uid: row.get("imap_uid")?,
                    imap_uidvalidity: row.get("imap_uidvalidity")?,
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
