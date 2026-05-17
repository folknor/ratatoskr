use crate::db::ReadConn;
use rusqlite::params;

pub fn get_attachments_collapsed(
    conn: &ReadConn<'_>,
    account_id: &str,
    thread_id: &str,
) -> Result<bool, String> {
    let result: Option<bool> = match conn.query_row(
        "SELECT attachments_collapsed FROM thread_ui_state
         WHERE account_id = ?1 AND thread_id = ?2",
        params![account_id, thread_id],
        |row| row.get(0),
    ) {
        Ok(value) => Some(value),
        Err(crate::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => None,
        Err(e) => return Err(e.to_string()),
    };
    Ok(result.unwrap_or(false))
}
