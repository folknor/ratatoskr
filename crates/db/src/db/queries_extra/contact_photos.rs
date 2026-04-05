//! Contact photo cache persistence.

use rusqlite::{Connection, OptionalExtension, params};

/// Upsert a contact photo cache entry and update the contact's avatar_url.
pub fn upsert_photo_cache_sync(
    conn: &Connection,
    email: &str,
    account_id: &str,
    content_hash: &str,
    file_path: &str,
    size_bytes: i64,
    etag: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO contact_photo_cache \
         (email, account_id, content_hash, file_path, size_bytes, etag, \
          fetched_at, last_accessed_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, unixepoch(), unixepoch()) \
         ON CONFLICT(email, account_id) DO UPDATE SET \
           content_hash = excluded.content_hash, \
           file_path = excluded.file_path, \
           size_bytes = excluded.size_bytes, \
           etag = excluded.etag, \
           fetched_at = excluded.fetched_at, \
           last_accessed_at = excluded.last_accessed_at",
        params![email, account_id, content_hash, file_path, size_bytes, etag],
    )
    .map_err(|e| format!("upsert contact photo cache: {e}"))?;

    conn.execute(
        "UPDATE contacts SET avatar_url = ?1, updated_at = unixepoch() WHERE email = ?2",
        params![file_path, email],
    )
    .map_err(|e| format!("update contact avatar_url: {e}"))?;

    Ok(())
}

/// Get the cached file path for a contact photo, updating last_accessed_at.
pub fn get_cached_photo_path_sync(
    conn: &Connection,
    email: &str,
    account_id: &str,
) -> Result<Option<String>, String> {
    let path: Option<String> = conn
        .query_row(
            "SELECT file_path FROM contact_photo_cache \
             WHERE email = ?1 AND account_id = ?2",
            params![email, account_id],
            |row| row.get("file_path"),
        )
        .optional()
        .map_err(|e| format!("query contact photo cache: {e}"))?;

    if path.is_some() {
        conn.execute(
            "UPDATE contact_photo_cache SET last_accessed_at = unixepoch() \
             WHERE email = ?1 AND account_id = ?2",
            params![email, account_id],
        )
        .map_err(|e| format!("update contact photo last_accessed_at: {e}"))?;
    }

    Ok(path)
}

/// Get total cache size in bytes.
pub fn get_cache_total_size_sync(conn: &Connection) -> Result<i64, String> {
    conn.query_row(
        "SELECT COALESCE(SUM(size_bytes), 0) AS total FROM contact_photo_cache",
        [],
        |row| row.get("total"),
    )
    .map_err(|e| format!("query contact photo cache size: {e}"))
}

/// Get the oldest cache entry (by last_accessed_at).
pub fn get_oldest_cache_entry_sync(
    conn: &Connection,
) -> Result<Option<(String, String, String)>, String> {
    conn.query_row(
        "SELECT email, account_id, file_path \
         FROM contact_photo_cache \
         ORDER BY last_accessed_at ASC \
         LIMIT 1",
        [],
        |row| {
            Ok((
                row.get("email")?,
                row.get("account_id")?,
                row.get("file_path")?,
            ))
        },
    )
    .optional()
    .map_err(|e| format!("query oldest contact photo: {e}"))
}

/// Delete a cache entry.
pub fn delete_cache_entry_sync(
    conn: &Connection,
    email: &str,
    account_id: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM contact_photo_cache WHERE email = ?1 AND account_id = ?2",
        params![email, account_id],
    )
    .map_err(|e| format!("delete evicted contact photo: {e}"))?;
    Ok(())
}

/// Get Graph contacts that need photo fetching (no cache entry yet).
pub fn get_uncached_graph_contacts_sync(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT gcm.email, gcm.graph_contact_id \
             FROM graph_contact_map gcm \
             WHERE gcm.account_id = ?1 \
               AND NOT EXISTS ( \
                 SELECT 1 FROM contact_photo_cache cpc \
                 WHERE cpc.email = gcm.email AND cpc.account_id = ?1 \
               )",
        )
        .map_err(|e| format!("prepare graph photo query: {e}"))?;

    stmt.query_map(params![account_id], |row| {
        Ok((
            row.get::<_, String>("email")?,
            row.get::<_, String>("graph_contact_id")?,
        ))
    })
    .map_err(|e| format!("query graph contacts for photos: {e}"))?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| format!("collect graph contacts for photos: {e}"))
}

/// Get Google contacts that need photo fetching (have remote avatar_url, no cache entry yet).
pub fn get_uncached_google_contacts_sync(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT c.email, c.avatar_url \
             FROM contacts c \
             INNER JOIN google_contact_map gcm ON gcm.contact_email = c.email \
               AND gcm.account_id = ?1 \
             WHERE c.avatar_url IS NOT NULL \
               AND c.avatar_url LIKE 'http%' \
               AND NOT EXISTS ( \
                 SELECT 1 FROM contact_photo_cache cpc \
                 WHERE cpc.email = c.email AND cpc.account_id = ?1 \
               )",
        )
        .map_err(|e| format!("prepare google photo query: {e}"))?;

    stmt.query_map(params![account_id], |row| {
        Ok((
            row.get::<_, String>("email")?,
            row.get::<_, String>("avatar_url")?,
        ))
    })
    .map_err(|e| format!("query google contacts for photos: {e}"))?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| format!("collect google contacts for photos: {e}"))
}
