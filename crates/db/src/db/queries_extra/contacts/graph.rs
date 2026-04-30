/// Server-side info for a Graph contact.
#[derive(Debug, Clone)]
pub struct GraphServerInfo {
    pub graph_contact_id: String,
    pub account_id: String,
}

/// Enrich contacts with Graph account_id and server_id via graph_contact_map.
pub fn enrich_graph_contacts_sync(
    conn: &rusqlite::Connection,
    account_id: &str,
) -> Result<usize, String> {
    conn.execute(
        "UPDATE contacts SET \
           account_id = ?1, \
           server_id = (
             SELECT m.graph_contact_id FROM graph_contact_map m
             WHERE m.email = contacts.email AND m.account_id = ?1
             LIMIT 1
           ) \
         WHERE source = 'graph' \
           AND email IN (
             SELECT m2.email FROM graph_contact_map m2 WHERE m2.account_id = ?1
           ) \
           AND (account_id IS NULL OR account_id = ?1)",
        rusqlite::params![account_id],
    )
    .map_err(|e| format!("enrich graph contacts: {e}"))
}

/// Look up the Graph contact ID and account for a contact email.
pub fn get_graph_contact_server_info_sync(
    conn: &rusqlite::Connection,
    email: &str,
) -> Result<Option<GraphServerInfo>, String> {
    let normalized = email.to_lowercase();
    conn.query_row(
        "SELECT m.graph_contact_id, m.account_id \
         FROM graph_contact_map m \
         WHERE m.email = ?1 \
         LIMIT 1",
        rusqlite::params![normalized],
        |row| {
            Ok(GraphServerInfo {
                graph_contact_id: row.get("graph_contact_id")?,
                account_id: row.get("account_id")?,
            })
        },
    )
    .map_err(|e| e.to_string())
    .map(Some)
    .or(Ok(None))
}
