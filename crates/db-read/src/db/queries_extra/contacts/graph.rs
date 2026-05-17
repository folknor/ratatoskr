use crate::db::ReadConn;

#[derive(Debug, Clone)]
pub struct GraphServerInfo {
    pub graph_contact_id: String,
    pub account_id: String,
}

pub fn get_graph_contact_server_info_sync(
    conn: &ReadConn<'_>,
    email: &str,
) -> Result<Option<GraphServerInfo>, String> {
    let normalized = email.to_lowercase();
    match conn.query_row(
        "SELECT m.graph_contact_id, m.account_id
         FROM graph_contact_map m
         WHERE m.email = ?1
         LIMIT 1",
        rusqlite::params![normalized],
        |row| {
            Ok(GraphServerInfo {
                graph_contact_id: row.get("graph_contact_id")?,
                account_id: row.get("account_id")?,
            })
        },
    ) {
        Ok(info) => Ok(Some(info)),
        Err(crate::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
