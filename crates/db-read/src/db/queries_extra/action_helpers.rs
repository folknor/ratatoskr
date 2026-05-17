use crate::db::ReadConn;
use rusqlite::params;

pub fn thread_exists_sync(
    conn: &ReadConn<'_>,
    account_id: &str,
    thread_id: &str,
) -> Result<bool, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM threads WHERE account_id = ?1 AND id = ?2",
        params![account_id, thread_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .map_err(|e| e.to_string())
}

pub fn label_exists_sync(
    conn: &ReadConn<'_>,
    label_id: &str,
    account_id: &str,
) -> Result<bool, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM labels WHERE id = ?1 AND account_id = ?2",
        params![label_id, account_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .map_err(|e| e.to_string())
}

pub struct ContactMeta {
    pub source: Option<String>,
    pub server_id: Option<String>,
    pub account_id: Option<String>,
}

pub fn get_contact_meta_by_id_sync(
    conn: &ReadConn<'_>,
    contact_id: &str,
) -> Result<Option<ContactMeta>, String> {
    match conn.query_row(
        "SELECT source, server_id, account_id FROM contacts WHERE id = ?1",
        params![contact_id],
        |row| {
            Ok(ContactMeta {
                source: row.get(0)?,
                server_id: row.get(1)?,
                account_id: row.get(2)?,
            })
        },
    ) {
        Ok(meta) => Ok(Some(meta)),
        Err(crate::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn get_message_ids_for_account_sync(
    conn: &ReadConn<'_>,
    account_id: &str,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT id FROM messages WHERE account_id = ?1")
        .map_err(|e| format!("prepare resync message query: {e}"))?;
    stmt.query_map(params![account_id], |row| row.get::<_, String>(0))
        .map_err(|e| format!("query resync message ids: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect resync message ids: {e}"))
}
