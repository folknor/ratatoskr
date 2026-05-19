use super::super::{ReadConn, ReadDbState};
use super::super::types::DbAccount;
use rusqlite::{Row, params};

fn row_to_account(row: &Row<'_>) -> rusqlite::Result<DbAccount> {
    Ok(DbAccount {
        id: row.get("id")?,
        email: row.get("email")?,
        display_name: row.get("display_name")?,
        avatar_url: row.get("avatar_url")?,
        access_token: row.get("access_token")?,
        refresh_token: row.get("refresh_token")?,
        token_expires_at: row.get("token_expires_at")?,
        history_id: row.get("history_id")?,
        initial_sync_completed: row.get("initial_sync_completed")?,
        last_sync_at: row.get("last_sync_at")?,
        is_active: row.get("is_active")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        provider: row.get("provider")?,
        imap_host: row.get("imap_host")?,
        imap_port: row.get("imap_port")?,
        imap_security: row.get("imap_security")?,
        smtp_host: row.get("smtp_host")?,
        smtp_port: row.get("smtp_port")?,
        smtp_security: row.get("smtp_security")?,
        auth_method: row.get("auth_method")?,
        imap_password: row.get("imap_password")?,
        oauth_provider: row.get("oauth_provider")?,
        oauth_client_id: row.get("oauth_client_id")?,
        oauth_client_secret: row.get("oauth_client_secret")?,
        oauth_extra_scopes: row.get("oauth_extra_scopes")?,
        imap_username: row.get("imap_username")?,
        smtp_username: row.get("smtp_username")?,
        smtp_password: row.get("smtp_password")?,
        caldav_url: row.get("caldav_url")?,
        caldav_username: row.get("caldav_username")?,
        caldav_password: row.get("caldav_password")?,
        caldav_principal_url: row.get("caldav_principal_url")?,
        caldav_home_url: row.get("caldav_home_url")?,
        calendar_provider: row.get("calendar_provider")?,
        accept_invalid_certs: row.get("accept_invalid_certs")?,
        jmap_url: row.get("jmap_url")?,
        account_color: row.get("account_color")?,
        account_name: row.get("account_name")?,
        sort_order: row.get("sort_order")?,
        is_deleting: row.get("is_deleting")?,
    })
}

pub async fn db_get_all_accounts(db: &ReadDbState) -> Result<Vec<DbAccount>, String> {
    db.with_read(move |conn| {
        get_all_accounts_sync(conn)
    })
    .await
}

pub async fn db_get_account(db: &ReadDbState, id: String) -> Result<Option<DbAccount>, String> {
    db.with_read(move |conn| {
        get_account_sync(conn, &id)
    })
    .await
}

pub async fn db_get_account_by_email(
    db: &ReadDbState,
    email: String,
) -> Result<Option<DbAccount>, String> {
    db.with_read(move |conn| {
        Ok(conn
            .query_row(
                "SELECT * FROM accounts WHERE email = ?1",
                params![email],
                row_to_account,
            )
            .ok())
    })
    .await
}

pub fn get_account_sync(conn: &ReadConn<'_>, id: &str) -> Result<Option<DbAccount>, String> {
    Ok(conn
        .query_row(
            "SELECT * FROM accounts WHERE id = ?1",
            params![id],
            row_to_account,
        )
        .ok())
}

pub fn get_all_accounts_sync(conn: &ReadConn<'_>) -> Result<Vec<DbAccount>, String> {
    let mut stmt = conn
        .prepare("SELECT * FROM accounts ORDER BY sort_order ASC, created_at ASC")
        .map_err(|e| e.to_string())?;
    stmt.query_map([], row_to_account)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

pub fn get_active_account_ids_sync(conn: &ReadConn<'_>) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT id FROM accounts WHERE is_active = 1 ORDER BY email ASC")
        .map_err(|e| e.to_string())?;
    stmt.query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Return every account id ordered by `sort_order`. Used by the GAL
/// kick handler, which iterates all accounts and lets
/// `fetch_gal_entries_if_stale` self-gates unsupported providers via
/// `Ok(0)`.
pub fn list_all_account_ids_sync(conn: &ReadConn<'_>) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT id FROM accounts ORDER BY sort_order")
        .map_err(|e| e.to_string())?;
    stmt.query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Return ids of accounts whose providers can run a calendar sync.
/// Filters at enumeration time so the calendar kick handler does not
/// repeatedly start runners for IMAP/JMAP-only accounts (which would
/// fail through to `"No calendar provider configured for account ..."`
/// every hour and stamp `last_completed`, producing nuisance log noise).
///
/// Mirrors the provider routing in `cal::sync::calendar_sync_account_impl`:
/// google_api/gmail_api -> Google calendar, graph -> Microsoft, caldav
/// (any of `calendar_provider`, `provider == 'caldav'` with a configured
/// url, or an explicit `calendar_provider = 'caldav'`), jmap -> JMAP.
pub fn list_calendar_capable_account_ids_sync(
    conn: &ReadConn<'_>,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id FROM accounts \
             WHERE calendar_provider IN ('google_api', 'graph', 'caldav', 'jmap') \
                OR provider IN ('gmail_api', 'graph', 'jmap') \
                OR (provider = 'caldav' AND caldav_url IS NOT NULL AND caldav_url != '') \
             ORDER BY sort_order",
        )
        .map_err(|e| e.to_string())?;
    stmt.query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}
