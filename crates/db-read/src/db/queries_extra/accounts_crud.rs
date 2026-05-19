use super::super::{ReadConn, ReadDbState};
use rusqlite::params;
use types::MailProviderKind;

/// Parameters for updating account metadata through the Service API.
#[derive(Debug, Clone)]
pub struct UpdateAccountParams {
    pub account_name: Option<String>,
    pub display_name: Option<String>,
    pub account_color: Option<String>,
    pub caldav_url: Option<String>,
    pub caldav_username: Option<String>,
    pub caldav_password: Option<String>,
    /// Per-account offline attachment-cache switch.
    pub cache_attachments_enabled: Option<bool>,
}

/// Lightweight auth info for re-authentication. Contains just enough
/// to determine which auth flow to run and pre-populate server fields.
#[derive(Debug, Clone)]
pub struct AccountAuthInfo {
    pub provider: String,
    pub auth_method: String,
    pub oauth_provider: Option<String>,
    pub oauth_client_id: Option<String>,
    /// Space-separated extra OAuth scopes carried into the auth-code
    /// request on re-auth, so the renewed token covers the same scope
    /// set the original grant did.
    pub oauth_extra_scopes: Option<String>,
    pub imap_host: Option<String>,
    pub imap_port: Option<i64>,
    pub imap_security: Option<String>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<i64>,
    pub smtp_security: Option<String>,
    pub imap_username: Option<String>,
}

pub fn account_exists_by_email_sync(conn: &ReadConn<'_>, email: &str) -> Result<bool, String> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) AS cnt FROM accounts WHERE email = ?1",
            params![email],
            |row| row.get("cnt"),
        )
        .map_err(|e| e.to_string())?;
    Ok(count > 0)
}

pub fn check_gmail_duplicate_sync(
    conn: &ReadConn<'_>,
    email: &str,
) -> Result<Option<String>, String> {
    match conn.query_row(
        "SELECT id FROM accounts WHERE email = ?1 AND provider = 'gmail_api' LIMIT 1",
        params![email],
        |row| row.get::<_, String>(0),
    ) {
        Ok(id) => Ok(Some(id)),
        Err(crate::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
        Err(e) => Err(format!("Duplicate Gmail check failed: {e}")),
    }
}

pub fn get_used_account_colors_sync(conn: &ReadConn<'_>) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT account_color FROM accounts WHERE account_color IS NOT NULL")
        .map_err(|e| format!("prepare used account colors: {e}"))?;
    stmt.query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| format!("query used account colors: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect used account colors: {e}"))
}

pub fn get_account_auth_info_sync(
    conn: &ReadConn<'_>,
    account_id: &str,
) -> Result<AccountAuthInfo, String> {
    conn.query_row(
        "SELECT provider, auth_method, oauth_provider, oauth_client_id,
                oauth_extra_scopes,
                imap_host, imap_port, imap_security,
                smtp_host, smtp_port, smtp_security, imap_username
         FROM accounts WHERE id = ?1",
        params![account_id],
        |row| {
            Ok(AccountAuthInfo {
                provider: row.get("provider")?,
                auth_method: row.get("auth_method")?,
                oauth_provider: row.get("oauth_provider")?,
                oauth_client_id: row.get("oauth_client_id")?,
                oauth_extra_scopes: row.get("oauth_extra_scopes")?,
                imap_host: row.get("imap_host")?,
                imap_port: row.get("imap_port")?,
                imap_security: row.get("imap_security")?,
                smtp_host: row.get("smtp_host")?,
                smtp_port: row.get("smtp_port")?,
                smtp_security: row.get("smtp_security")?,
                imap_username: row.get("imap_username")?,
            })
        },
    )
    .map_err(|e| format!("Account not found: {e}"))
}

pub fn get_account_provider_sync(
    conn: &ReadConn<'_>,
    account_id: &str,
) -> Result<MailProviderKind, String> {
    let raw: String = conn
        .query_row(
            "SELECT provider FROM accounts WHERE id = ?1",
            params![account_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("lookup provider: {e}"))?;
    MailProviderKind::parse(&raw)
}

pub async fn db_account_exists_by_email(db: &ReadDbState, email: String) -> Result<bool, String> {
    db.with_read(move |conn| account_exists_by_email_sync(conn, &email))
        .await
}
