use std::collections::HashMap;

use crate::db::ReadConn;
use rusqlite::params;

#[derive(Debug, Clone)]
pub struct CarddavContactUpsert {
    pub uri: String,
    pub etag: String,
    pub email: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
}

pub fn load_carddav_ctag_sync(
    conn: &ReadConn<'_>,
    account_id: &str,
) -> Result<Option<String>, String> {
    let key = format!("carddav_ctag:{account_id}");
    crate::db::queries::get_setting(conn, &key)
}

pub fn load_carddav_etags_sync(
    conn: &ReadConn<'_>,
    account_id: &str,
) -> Result<HashMap<String, String>, String> {
    let mut stmt = conn
        .prepare("SELECT uri, etag FROM carddav_contact_map WHERE account_id = ?1")
        .map_err(|e| format!("prepare etag query: {e}"))?;

    let rows = stmt
        .query_map(params![account_id], |row| {
            Ok((
                row.get::<_, String>("uri")?,
                row.get::<_, Option<String>>("etag")?,
            ))
        })
        .map_err(|e| format!("query etags: {e}"))?;

    let mut map = HashMap::new();
    for row in rows {
        let (uri, etag) = row.map_err(|e| format!("read etag row: {e}"))?;
        if let Some(etag) = etag {
            map.insert(uri, etag);
        }
    }

    Ok(map)
}
