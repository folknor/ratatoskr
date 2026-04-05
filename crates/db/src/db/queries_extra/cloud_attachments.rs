//! Cloud attachment upload queue persistence.

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

/// A row from the `cloud_attachments` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudAttachment {
    pub id: i64,
    pub message_id: Option<String>,
    pub account_id: String,
    pub direction: String,
    pub provider: String,
    pub cloud_url: Option<String>,
    pub file_name: Option<String>,
    pub file_size: Option<i64>,
    pub mime_type: Option<String>,
    pub drive_item_id: Option<String>,
    pub upload_session_url: Option<String>,
    pub upload_status: String,
    pub bytes_uploaded: i64,
    pub retry_count: i32,
    pub created_at: i64,
}

fn row_to_cloud_attachment(row: &rusqlite::Row<'_>) -> Result<CloudAttachment, rusqlite::Error> {
    Ok(CloudAttachment {
        id: row.get("id")?,
        message_id: row.get("message_id")?,
        account_id: row.get("account_id")?,
        direction: row.get("direction")?,
        provider: row.get("provider")?,
        cloud_url: row.get("cloud_url")?,
        file_name: row.get("file_name")?,
        file_size: row.get("file_size")?,
        mime_type: row.get("mime_type")?,
        drive_item_id: row.get("drive_item_id")?,
        upload_session_url: row.get("upload_session_url")?,
        upload_status: row.get("upload_status")?,
        bytes_uploaded: row.get("bytes_uploaded")?,
        retry_count: row.get("retry_count")?,
        created_at: row.get("created_at")?,
    })
}

/// Get all pending uploads for an account.
pub fn get_pending_uploads_sync(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<CloudAttachment>, String> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT * FROM cloud_attachments
             WHERE account_id = ?1 AND direction = 'outgoing' AND upload_status = 'pending'
             ORDER BY created_at ASC",
        )
        .map_err(|e| e.to_string())?;
    stmt.query_map(params![account_id], row_to_cloud_attachment)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Transition an upload to a new status, optionally updating bytes_uploaded.
pub fn update_upload_status_sync(
    conn: &Connection,
    id: i64,
    status: &str,
    bytes_uploaded: Option<i64>,
) -> Result<(), String> {
    if let Some(bytes) = bytes_uploaded {
        conn.execute(
            "UPDATE cloud_attachments SET upload_status = ?1, bytes_uploaded = ?2 WHERE id = ?3",
            params![status, bytes, id],
        )
        .map_err(|e| e.to_string())?;
    } else {
        conn.execute(
            "UPDATE cloud_attachments SET upload_status = ?1 WHERE id = ?2",
            params![status, id],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Mark an upload as failed, incrementing retry_count.
pub fn mark_upload_failed_sync(
    conn: &Connection,
    id: i64,
    new_status: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE cloud_attachments
         SET upload_status = ?1, retry_count = retry_count + 1
         WHERE id = ?2",
        params![new_status, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// On app restart: reset rows stuck in `uploading` back to `pending`.
pub fn reset_interrupted_uploads_sync(conn: &Connection) -> Result<usize, String> {
    conn.execute(
        "UPDATE cloud_attachments SET upload_status = 'pending' WHERE upload_status = 'uploading'",
        [],
    )
    .map_err(|e| e.to_string())
}

/// Create a new outgoing cloud attachment entry. Returns the row ID.
pub fn create_outgoing_upload_sync(
    conn: &Connection,
    account_id: &str,
    provider: &str,
    file_name: &str,
    file_size: i64,
    mime_type: &str,
) -> Result<i64, String> {
    conn.execute(
        "INSERT INTO cloud_attachments
            (account_id, direction, provider, file_name, file_size, mime_type, upload_status)
         VALUES (?1, 'outgoing', ?2, ?3, ?4, ?5, 'pending')",
        params![account_id, provider, file_name, file_size, mime_type],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

/// Get uploads that have permanently failed (retry_count >= max_retries).
pub fn get_permanently_failed_sync(
    conn: &Connection,
    max_retries: i32,
) -> Result<Vec<CloudAttachment>, String> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT * FROM cloud_attachments
             WHERE upload_status = 'failed' AND retry_count >= ?1
             ORDER BY created_at ASC",
        )
        .map_err(|e| e.to_string())?;
    stmt.query_map(params![max_retries], row_to_cloud_attachment)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Insert detected incoming cloud links.
pub fn insert_incoming_cloud_links_sync(
    conn: &Connection,
    message_id: &str,
    account_id: &str,
    links: &[(String, String)], // (provider_str, url)
) -> Result<usize, String> {
    if links.is_empty() {
        return Ok(0);
    }

    let mut stmt = conn
        .prepare_cached(
            "INSERT OR IGNORE INTO cloud_attachments
                (message_id, account_id, direction, provider, cloud_url, upload_status)
             VALUES (?1, ?2, 'incoming', ?3, ?4, 'complete')",
        )
        .map_err(|e| e.to_string())?;

    let mut count: usize = 0;
    for (provider, url) in links {
        count += stmt
            .execute(params![message_id, account_id, provider, url])
            .map_err(|e| e.to_string())?;
    }

    Ok(count)
}

/// Update metadata columns of a cloud_attachments row.
pub fn update_cloud_attachment_metadata_sync(
    conn: &Connection,
    id: i64,
    file_name: Option<&str>,
    file_size: Option<i64>,
    mime_type: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE cloud_attachments
         SET file_name = COALESCE(?1, file_name),
             file_size = COALESCE(?2, file_size),
             mime_type = COALESCE(?3, mime_type)
         WHERE id = ?4",
        params![file_name, file_size, mime_type, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
