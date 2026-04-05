//! Send identity queries.

use rusqlite::{Connection, params};

/// A row from the `send_identities` table.
#[derive(Debug, Clone)]
pub struct SendIdentity {
    pub id: i64,
    pub account_id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub mailbox_id: Option<String>,
    pub send_mode: String,
    pub is_primary: bool,
}

/// Return all send identities for the given account, ordered so that the
/// primary identity comes first.
pub fn get_send_identities(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<SendIdentity>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, account_id, email, display_name, mailbox_id, send_mode, is_primary
             FROM send_identities
             WHERE account_id = ?1
             ORDER BY is_primary DESC, id ASC",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_map(params![account_id], |row| {
        Ok(SendIdentity {
            id: row.get("id")?,
            account_id: row.get("account_id")?,
            email: row.get("email")?,
            display_name: row.get("display_name")?,
            mailbox_id: row.get("mailbox_id")?,
            send_mode: row.get("send_mode")?,
            is_primary: row.get::<_, i64>("is_primary")? != 0,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

/// Return all distinct send-identity email addresses across accounts.
pub fn get_all_send_identity_emails(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT DISTINCT email FROM send_identities")
        .map_err(|e| e.to_string())?;
    stmt.query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}
