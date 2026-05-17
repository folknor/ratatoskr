use crate::db::from_row::FromRow;
use crate::db::types::DbScheduledEmail;
use crate::db::ReadConn;
use rusqlite::params;

pub fn get_remote_draft_id_sync(
    conn: &ReadConn<'_>,
    draft_id: &str,
) -> Result<Option<String>, String> {
    match conn.query_row(
        "SELECT remote_draft_id FROM local_drafts WHERE id = ?1",
        params![draft_id],
        |row| row.get(0),
    ) {
        Ok(id) => Ok(Some(id)),
        Err(crate::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
        Err(e) => Err(format!("draft lookup: {e}")),
    }
}

pub fn get_overdue_local_scheduled_sync(
    conn: &ReadConn<'_>,
    now_unix: i64,
) -> Result<Vec<DbScheduledEmail>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT * FROM scheduled_emails
             WHERE status = 'pending' AND delegation = 'local' AND scheduled_at <= ?1
             ORDER BY scheduled_at ASC",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_map(params![now_unix], DbScheduledEmail::from_row)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}
