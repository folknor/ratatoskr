//! Auto-response (vacation/out-of-office) cache storage.

use rusqlite::{Connection, params};

/// Raw auto-response row from the database.
///
/// The `external_audience` field is stored as a string and parsed into
/// the domain-level `ExternalAudience` enum by core.
#[derive(Debug, Clone)]
pub struct AutoResponseRow {
    pub enabled: bool,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub internal_message_html: Option<String>,
    pub external_message_html: Option<String>,
    /// Raw string: "all", "contacts", or "none".
    pub external_audience: String,
}

/// Get the cached auto-response for an account.
pub fn get_auto_response_sync(
    conn: &Connection,
    account_id: &str,
) -> Result<Option<AutoResponseRow>, String> {
    conn.query_row(
        "SELECT enabled, start_date, end_date, internal_message_html, \
         external_message_html, external_audience \
         FROM auto_responses WHERE account_id = ?1",
        params![account_id],
        |row| {
            Ok(AutoResponseRow {
                enabled: row.get::<_, i32>(0)? != 0,
                start_date: row.get(1)?,
                end_date: row.get(2)?,
                internal_message_html: row.get(3)?,
                external_message_html: row.get(4)?,
                external_audience: row.get::<_, String>(5).unwrap_or_default(),
            })
        },
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        _ => Err(format!("get auto_response: {e}")),
    })
}

/// Upsert an auto-response cache entry.
pub fn upsert_auto_response_sync(
    conn: &Connection,
    account_id: &str,
    enabled: bool,
    start_date: Option<&str>,
    end_date: Option<&str>,
    internal_message_html: Option<&str>,
    external_message_html: Option<&str>,
    external_audience: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO auto_responses \
         (account_id, enabled, start_date, end_date, \
          internal_message_html, external_message_html, \
          external_audience, last_synced_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, unixepoch()) \
         ON CONFLICT(account_id) DO UPDATE SET \
           enabled = ?2, start_date = ?3, end_date = ?4, \
           internal_message_html = ?5, external_message_html = ?6, \
           external_audience = ?7, last_synced_at = unixepoch()",
        params![
            account_id,
            enabled as i32,
            start_date,
            end_date,
            internal_message_html,
            external_message_html,
            external_audience,
        ],
    )
    .map_err(|e| format!("upsert auto_response: {e}"))?;
    Ok(())
}

/// Check whether any account has an active auto-response.
pub fn any_auto_response_active_sync(conn: &Connection) -> Result<bool, String> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM auto_responses WHERE enabled = 1)",
        [],
        |row| row.get::<_, bool>(0),
    )
    .map_err(|e| format!("any_auto_response_active: {e}"))
}
