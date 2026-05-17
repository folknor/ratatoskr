use crate::db::types::DbContact;
use crate::db::{FromRow, ReadDbState};
use rusqlite::params;

#[derive(Debug, Clone)]
pub struct ExpandedGroupContact {
    pub email: String,
    pub display_name: Option<String>,
}

pub async fn db_get_all_contacts(
    db: &ReadDbState,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<DbContact>, String> {
    db.with_read(move |conn| {
        let lim = limit.unwrap_or(crate::db::DEFAULT_QUERY_LIMIT);
        let off = offset.unwrap_or(0);
        let mut stmt = conn
            .prepare(
                "SELECT * FROM contacts ORDER BY frequency DESC, display_name ASC LIMIT ?1 OFFSET ?2",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![lim, off], DbContact::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_find_contact_id_by_email(
    db: &ReadDbState,
    email: String,
) -> Result<Option<String>, String> {
    db.with_read(move |conn| {
        match conn.query_row(
            "SELECT id FROM contacts WHERE email = ?1 LIMIT 1",
            params![email],
            |row| row.get::<_, String>(0),
        ) {
            Ok(id) => Ok(Some(id)),
            Err(crate::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    })
    .await
}
