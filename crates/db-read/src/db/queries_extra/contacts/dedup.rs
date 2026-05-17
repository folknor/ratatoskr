use crate::db::ReadConn;

#[derive(Debug, Clone)]
pub struct DuplicatePairRow {
    pub contact_id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub source: String,
    pub seen_name: Option<String>,
    pub seen_account_id: String,
}

pub fn find_contact_duplicates_sync(
    conn: &ReadConn<'_>,
    limit: i64,
) -> Result<Vec<DuplicatePairRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT c.id, c.email, c.display_name, c.source,
                    s.display_name AS seen_name, s.account_id AS seen_account_id
             FROM contacts c
             INNER JOIN seen_addresses s ON LOWER(c.email) = LOWER(s.email)
             WHERE c.source != 'seen'
             LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_map(rusqlite::params![limit], |row| {
        Ok(DuplicatePairRow {
            contact_id: row.get("id")?,
            email: row.get("email")?,
            display_name: row.get("display_name")?,
            source: row.get("source")?,
            seen_name: row.get("seen_name")?,
            seen_account_id: row.get::<_, String>("seen_account_id").unwrap_or_default(),
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}
