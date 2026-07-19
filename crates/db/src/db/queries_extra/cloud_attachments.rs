//! Cloud attachment upload queue persistence.

use crate::db::WriteTarget;
use rusqlite::params;

/// Insert detected incoming cloud links.
pub fn insert_incoming_cloud_links_sync(
    conn: &impl WriteTarget,
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
    conn: &impl WriteTarget,
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
