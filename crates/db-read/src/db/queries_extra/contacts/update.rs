use crate::db::ReadConn;

#[derive(Debug, Clone)]
pub struct ContactUpdate {
    pub email: String,
    pub display_name: Option<String>,
    pub email2: Option<Option<String>>,
    pub phone: Option<Option<String>>,
    pub company: Option<Option<String>>,
    pub notes: Option<Option<String>>,
}

pub fn get_contact_source_sync(conn: &ReadConn<'_>, email: &str) -> Result<Option<String>, String> {
    match conn.query_row(
        "SELECT source FROM contacts WHERE email = ?1",
        rusqlite::params![email],
        |row| row.get("source"),
    ) {
        Ok(source) => Ok(Some(source)),
        Err(crate::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
