use crate::db::ReadConn;

pub fn get_calendar_default_view_sync(conn: &ReadConn<'_>) -> Result<Option<String>, String> {
    match conn.query_row(
        "SELECT value FROM settings WHERE key = 'calendar_default_view' LIMIT 1",
        [],
        |row| row.get::<_, String>(0),
    ) {
        Ok(value) => Ok(Some(value)),
        Err(crate::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
