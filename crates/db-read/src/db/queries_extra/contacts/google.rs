use crate::db::ReadConn;

#[derive(Debug, Clone)]
pub struct GoogleContactFields {
    pub email: String,
    pub resource_name: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GoogleServerInfo {
    pub resource_name: String,
    pub account_id: String,
}

pub fn get_google_contact_server_info_sync(
    conn: &ReadConn<'_>,
    email: &str,
) -> Result<Option<GoogleServerInfo>, String> {
    let normalized = email.to_lowercase();
    match conn.query_row(
        "SELECT m.resource_name, m.account_id
         FROM google_contact_map m
         WHERE m.contact_email = ?1
         LIMIT 1",
        rusqlite::params![normalized],
        |row| {
            Ok(GoogleServerInfo {
                resource_name: row.get("resource_name")?,
                account_id: row.get("account_id")?,
            })
        },
    ) {
        Ok(info) => Ok(Some(info)),
        Err(crate::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
