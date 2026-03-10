use rusqlite::Connection;

use crate::imap::types::ImapConfig;

/// Account record read from the DB (minimal fields needed for sync).
pub struct SyncAccount {
    pub id: String,
    pub email: String,
    pub provider: String,
    #[allow(dead_code)]
    pub history_id: Option<String>,
    pub imap_host: Option<String>,
    pub imap_port: Option<i64>,
    pub imap_security: Option<String>,
    pub imap_username: Option<String>,
    pub imap_password: Option<String>,
    pub auth_method: Option<String>,
    pub accept_invalid_certs: bool,
}

/// Read an account from the DB.
pub fn get_account(conn: &Connection, account_id: &str) -> Result<SyncAccount, String> {
    conn.query_row(
        "SELECT id, email, provider, history_id, imap_host, imap_port, \
         imap_security, imap_username, imap_password, auth_method, \
         accept_invalid_certs \
         FROM accounts WHERE id = ?1",
        rusqlite::params![account_id],
        |row| {
            let accept: Option<i64> = row.get(10)?;
            Ok(SyncAccount {
                id: row.get(0)?,
                email: row.get(1)?,
                provider: row.get(2)?,
                history_id: row.get(3)?,
                imap_host: row.get(4)?,
                imap_port: row.get(5)?,
                imap_security: row.get(6)?,
                imap_username: row.get(7)?,
                imap_password: row.get(8)?,
                auth_method: row.get(9)?,
                accept_invalid_certs: accept.unwrap_or(0) != 0,
            })
        },
    )
    .map_err(|e| format!("get account {account_id}: {e}"))
}

/// Map DB security value to config type ('ssl' → 'tls').
fn map_security(security: Option<&str>) -> String {
    match security.map(str::to_lowercase).as_deref() {
        Some("ssl") | Some("tls") | None => "tls".to_string(),
        Some("starttls") => "starttls".to_string(),
        Some("none") => "none".to_string(),
        Some(other) => {
            log::warn!("Unknown security mode '{other}', defaulting to tls");
            "tls".to_string()
        }
    }
}

/// Build an ImapConfig from a SyncAccount.
pub fn build_imap_config(account: &SyncAccount) -> Result<ImapConfig, String> {
    let host = account
        .imap_host
        .as_deref()
        .ok_or_else(|| format!("Account {} has no IMAP host configured", account.id))?;

    let auth_method = match account.auth_method.as_deref() {
        Some("oauth2") => "oauth2".to_string(),
        _ => "password".to_string(),
    };

    let username = account
        .imap_username
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(&account.email)
        .to_string();

    Ok(ImapConfig {
        host: host.to_string(),
        port: account.imap_port.unwrap_or(993) as u16,
        security: map_security(account.imap_security.as_deref()),
        username,
        password: account.imap_password.clone().unwrap_or_default(),
        auth_method,
        accept_invalid_certs: account.accept_invalid_certs,
    })
}

/// Read the `sync_period_days` setting from DB, defaulting to 365.
pub fn get_sync_period_days(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT value FROM settings WHERE key = 'sync_period_days'",
        [],
        |row| {
            let val: String = row.get(0)?;
            Ok(val.parse::<i64>().unwrap_or(365))
        },
    )
    .unwrap_or(365)
}
