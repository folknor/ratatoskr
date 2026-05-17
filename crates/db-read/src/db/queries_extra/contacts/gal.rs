use crate::db::ReadConn;

#[derive(Debug, Clone)]
pub struct GalEntry {
    pub email: String,
    pub display_name: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub title: Option<String>,
    pub department: Option<String>,
}

pub fn gal_cache_age_sync(
    conn: &ReadConn<'_>,
    account_id: &str,
) -> Result<Option<i64>, String> {
    let key = format!("gal_refresh_{account_id}");
    match conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        rusqlite::params![key],
        |row| row.get::<_, String>(0),
    ) {
        Ok(value) => value
            .parse::<i64>()
            .map(Some)
            .map_err(|e| format!("parse gal timestamp: {e}")),
        Err(crate::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
