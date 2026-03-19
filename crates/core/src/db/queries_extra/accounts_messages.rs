use super::super::DbState;
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
        imap_username: row.get("imap_username")?,
        caldav_url: row.get("caldav_url")?,
        caldav_username: row.get("caldav_username")?,
        caldav_password: row.get("caldav_password")?,
        caldav_principal_url: row.get("caldav_principal_url")?,
        caldav_home_url: row.get("caldav_home_url")?,
        calendar_provider: row.get("calendar_provider")?,
        accept_invalid_certs: row.get("accept_invalid_certs")?,
        jmap_url: row.get("jmap_url")?,
    })
}

pub async fn db_get_all_accounts(db: &DbState) -> Result<Vec<DbAccount>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare("SELECT * FROM accounts ORDER BY created_at ASC")
            .map_err(|e| e.to_string())?;
        stmt.query_map([], row_to_account)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_get_account(db: &DbState, id: String) -> Result<Option<DbAccount>, String> {
    db.with_conn(move |conn| {
        Ok(conn
            .query_row(
                "SELECT * FROM accounts WHERE id = ?1",
                params![id],
                row_to_account,
            )
            .ok())
    })
    .await
}

pub async fn db_get_account_by_email(
    db: &DbState,
    email: String,
) -> Result<Option<DbAccount>, String> {
    db.with_conn(move |conn| {
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

pub async fn db_delete_account(db: &DbState, id: String) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute("DELETE FROM accounts WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

