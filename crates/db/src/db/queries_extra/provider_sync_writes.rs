//! Per-message / per-thread / per-attachment / public-folder writes that
//! previously lived inline in provider sync paths. Agent-owned scaffold
//! for Phase 1.6 - functions get added here as call sites in
//! `crates/imap/src/sync_pipeline.rs`, `crates/imap/src/public_folders.rs`,
//! `crates/graph/src/public_folder_sync.rs`, and
//! `crates/stores/src/attachment_cache.rs` are routed through `db` APIs.
//!
//! Each function takes `&Connection` (sync); callers wrap in
//! `ReadDbState::with_conn(...)` if they need async dispatch.

use rusqlite::{Connection, params};

// ---------------------------------------------------------------------------
// messages table
// ---------------------------------------------------------------------------

/// Update the `is_read` and `is_starred` flags on a single message matched by
/// `(account_id, imap_folder, imap_uid)`. Returns the number of rows updated.
pub fn set_message_imap_flags(
    conn: &Connection,
    account_id: &str,
    folder: &str,
    imap_uid: i64,
    is_read: bool,
    is_starred: bool,
) -> Result<usize, String> {
    conn.execute(
        "UPDATE messages SET is_read = ?1, is_starred = ?2 \
         WHERE account_id = ?3 AND imap_folder = ?4 AND imap_uid = ?5",
        params![is_read, is_starred, account_id, folder, imap_uid],
    )
    .map_err(|e| format!("set_message_imap_flags: {e}"))
}

/// Return the `thread_id` for a message matched by
/// `(account_id, imap_folder, imap_uid)`. Returns `None` if not found.
pub fn get_thread_id_for_imap_uid(
    conn: &Connection,
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
    .or_else(|e| {
        if e == rusqlite::Error::QueryReturnedNoRows {
            Ok(None)
        } else {
            Err(format!("get_thread_id_for_imap_uid: {e}"))
        }
    })
}

// ---------------------------------------------------------------------------
// threads table
// ---------------------------------------------------------------------------

/// Recompute `is_read` / `is_starred` for a thread by aggregating its messages.
///
/// `is_read` becomes the MIN of all constituent message flags (a thread is
/// read only when every message is read). `is_starred` becomes the MAX (starred
/// if any message is starred).
pub fn recompute_thread_read_starred(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE threads SET \
           is_read    = (SELECT MIN(is_read)    FROM messages WHERE account_id = ?1 AND thread_id = ?2), \
           is_starred = (SELECT MAX(is_starred) FROM messages WHERE account_id = ?1 AND thread_id = ?2) \
         WHERE account_id = ?1 AND id = ?2",
        params![account_id, thread_id],
    )
    .map_err(|e| format!("recompute_thread_read_starred: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// attachments table
// ---------------------------------------------------------------------------

/// Cached-attachment lookup result for a single attachments row.
pub struct AttachmentCacheInfo {
    pub id: String,
    pub content_hash: Option<String>,
    pub mime_type: Option<String>,
}

/// Look up an attachment's cache info by message + provider-agnostic remote
/// attachment ID (checks both `gmail_attachment_id` and `imap_part_id`).
pub fn find_attachment_cache_info(
    conn: &Connection,
    account_id: &str,
    message_id: &str,
    remote_attachment_id: &str,
) -> Result<Option<AttachmentCacheInfo>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, content_hash, mime_type \
             FROM attachments \
             WHERE account_id = ?1 AND message_id = ?2 \
               AND (gmail_attachment_id = ?3 OR imap_part_id = ?3) \
             LIMIT 1",
        )
        .map_err(|e| format!("find_attachment_cache_info prepare: {e}"))?;

    let mut rows = stmt
        .query_map(
            params![account_id, message_id, remote_attachment_id],
            |row| {
                Ok(AttachmentCacheInfo {
                    id: row.get("id")?,
                    content_hash: row.get("content_hash")?,
                    mime_type: row.get("mime_type")?,
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

/// Update an attachment row's cache fields after the file has been written to
/// disk: sets `local_path`, `cached_at`, `cache_size`, and `content_hash`.
pub fn update_attachment_cache_fields(
    conn: &Connection,
    attachment_id: &str,
    local_path: &str,
    cache_size: i64,
    content_hash: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE attachments \
         SET local_path = ?1, cached_at = unixepoch(), cache_size = ?2, content_hash = ?3 \
         WHERE id = ?4",
        params![local_path, cache_size, content_hash, attachment_id],
    )
    .map_err(|e| format!("update_attachment_cache_fields: {e}"))?;
    Ok(())
}

/// Bump `cached_at` to the current epoch for a single attachment row that
/// already has a populated cache. Used by the cache-hit path of
/// `attachment.fetch` so an actively-opened attachment surfaces as recent
/// to the LRU eviction sweep instead of staying frozen at its first-fetch
/// timestamp.
pub fn bump_attachment_cached_at(
    conn: &Connection,
    attachment_id: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE attachments SET cached_at = unixepoch() \
         WHERE id = ?1 AND cached_at IS NOT NULL",
        params![attachment_id],
    )
    .map_err(|e| format!("bump_attachment_cached_at: {e}"))?;
    Ok(())
}

/// Clear the cache fields (`local_path`, `cached_at`, `cache_size`) for a
/// batch of attachment IDs in one statement (used during cache eviction).
pub fn clear_attachment_cache_fields_batch(
    conn: &Connection,
    attachment_ids: &[String],
) -> Result<(), String> {
    if attachment_ids.is_empty() {
        return Ok(());
    }
    let placeholders: String = attachment_ids
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "UPDATE attachments \
         SET local_path = NULL, cached_at = NULL, cache_size = NULL \
         WHERE id IN ({placeholders})"
    );
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        attachment_ids.iter().map(|s| s as &dyn rusqlite::types::ToSql).collect();
    conn.execute(&sql, param_refs.as_slice())
        .map_err(|e| format!("clear_attachment_cache_fields_batch: {e}"))?;
    Ok(())
}

/// Return how many attachment rows still reference a given content hash and
/// have a non-NULL `cached_at` (i.e., are still cached). Used to decide
/// whether to delete the backing file during eviction.
pub fn count_cached_attachment_refs(
    conn: &Connection,
    content_hash: &str,
) -> Result<i64, String> {
    conn.query_row(
        "SELECT COUNT(*) AS cnt FROM attachments \
         WHERE content_hash = ?1 AND cached_at IS NOT NULL",
        params![content_hash],
        |row| row.get("cnt"),
    )
    .map_err(|e| format!("count_cached_attachment_refs: {e}"))
}

/// Return the total size in bytes of all currently-cached attachments.
pub fn get_total_cached_attachment_size(conn: &Connection) -> Result<i64, String> {
    conn.query_row(
        "SELECT COALESCE(SUM(cache_size), 0) AS total \
         FROM attachments WHERE cached_at IS NOT NULL",
        [],
        |row| row.get("total"),
    )
    .map_err(|e| format!("get_total_cached_attachment_size: {e}"))
}

/// One row of `get_cached_attachments_oldest_first` output.
pub struct CachedAttachmentRow {
    pub attachment_id: String,
    pub local_path: String,
    pub content_hash: Option<String>,
    pub cache_size: i64,
}

/// Return all cached-attachment rows ordered oldest-first (for eviction).
pub fn get_cached_attachments_oldest_first(
    conn: &Connection,
) -> Result<Vec<CachedAttachmentRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, local_path, content_hash, cache_size \
             FROM attachments \
             WHERE cached_at IS NOT NULL \
             ORDER BY cached_at ASC",
        )
        .map_err(|e| format!("get_cached_attachments_oldest_first prepare: {e}"))?;

    let rows = stmt
        .query_map([], |row| {
            Ok(CachedAttachmentRow {
                attachment_id: row.get("id")?,
                local_path: row.get("local_path")?,
                content_hash: row.get("content_hash")?,
                cache_size: row.get("cache_size")?,
            })
        })
        .map_err(|e| format!("get_cached_attachments_oldest_first query: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("get_cached_attachments_oldest_first row: {e}"))?;

    Ok(rows)
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
    conn: &Connection,
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
    conn: &Connection,
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
    conn: &Connection,
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
    conn: &Connection,
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
    conn: &Connection,
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
    conn: &Connection,
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
    conn: &Connection,
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
    conn: &Connection,
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
pub fn get_pinned_folder_ids(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<String>, String> {
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
