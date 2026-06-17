use crate::db::ReadConn;

#[derive(Debug, Clone)]
pub struct AutoResponseRow {
    pub id: i64,
    pub account_id: String,
    pub folder_id: Option<String>,
    pub subject_contains: Option<String>,
    pub body_html: String,
    pub is_active: bool,
}

pub fn get_auto_response_sync(
    conn: &ReadConn<'_>,
    account_id: &str,
    folder_id: Option<&str>,
) -> Result<Option<AutoResponseRow>, String> {
    let sql = if folder_id.is_some() {
        "SELECT id, account_id, folder_id, subject_contains, body_html, is_active
         FROM auto_responses
         WHERE account_id = ?1 AND folder_id = ?2 AND is_active = 1
         LIMIT 1"
    } else {
        "SELECT id, account_id, folder_id, subject_contains, body_html, is_active
         FROM auto_responses
         WHERE account_id = ?1 AND folder_id IS NULL AND is_active = 1
         LIMIT 1"
    };
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let row = if let Some(folder_id) = folder_id {
        stmt.query_row(
            rusqlite::params![account_id, folder_id],
            auto_response_from_row,
        )
    } else {
        stmt.query_row(rusqlite::params![account_id], auto_response_from_row)
    };
    match row {
        Ok(value) => Ok(Some(value)),
        Err(crate::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn any_auto_response_active_sync(conn: &ReadConn<'_>) -> Result<bool, String> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM auto_responses WHERE is_active = 1)",
        [],
        |row| Ok(row.get::<_, i64>(0)? != 0),
    )
    .map_err(|e| e.to_string())
}

fn auto_response_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AutoResponseRow> {
    Ok(AutoResponseRow {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        folder_id: row.get("folder_id")?,
        subject_contains: row.get("subject_contains")?,
        body_html: row.get("body_html")?,
        is_active: row.get::<_, i64>("is_active")? != 0,
    })
}
