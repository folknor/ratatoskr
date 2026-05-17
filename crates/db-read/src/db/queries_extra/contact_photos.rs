use crate::db::ReadConn;
use rusqlite::params;

pub fn get_cached_photo_path_sync(
    conn: &ReadConn<'_>,
    email: &str,
    account_id: &str,
) -> Result<Option<String>, String> {
    match conn.query_row(
        "SELECT file_path FROM contact_photo_cache
         WHERE email = ?1 AND account_id = ?2",
        params![email, account_id],
        |row| row.get("file_path"),
    ) {
        Ok(path) => Ok(Some(path)),
        Err(crate::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
        Err(e) => Err(format!("query contact photo cache: {e}")),
    }
}

pub fn get_cache_total_size_sync(conn: &ReadConn<'_>) -> Result<i64, String> {
    conn.query_row(
        "SELECT COALESCE(SUM(size_bytes), 0) AS total FROM contact_photo_cache",
        [],
        |row| row.get("total"),
    )
    .map_err(|e| format!("query contact photo cache size: {e}"))
}

pub fn get_oldest_cache_entry_sync(
    conn: &ReadConn<'_>,
) -> Result<Option<(String, String, String)>, String> {
    match conn.query_row(
        "SELECT email, account_id, file_path
         FROM contact_photo_cache
         ORDER BY last_accessed_at ASC
         LIMIT 1",
        [],
        |row| {
            Ok((
                row.get("email")?,
                row.get("account_id")?,
                row.get("file_path")?,
            ))
        },
    ) {
        Ok(entry) => Ok(Some(entry)),
        Err(crate::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
        Err(e) => Err(format!("query oldest contact photo: {e}")),
    }
}

pub fn get_uncached_graph_contacts_sync(
    conn: &ReadConn<'_>,
    account_id: &str,
) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT gcm.email, gcm.graph_contact_id
             FROM graph_contact_map gcm
             WHERE gcm.account_id = ?1
               AND NOT EXISTS (
                 SELECT 1 FROM contact_photo_cache cpc
                 WHERE cpc.email = gcm.email AND cpc.account_id = ?1
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

pub fn get_uncached_google_contacts_sync(
    conn: &ReadConn<'_>,
    account_id: &str,
) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT c.email, c.avatar_url
             FROM contacts c
             INNER JOIN google_contact_map gcm ON gcm.contact_email = c.email
               AND gcm.account_id = ?1
             WHERE c.avatar_url IS NOT NULL
               AND c.avatar_url LIKE 'http%'
               AND NOT EXISTS (
                 SELECT 1 FROM contact_photo_cache cpc
                 WHERE cpc.email = c.email AND cpc.account_id = ?1
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
