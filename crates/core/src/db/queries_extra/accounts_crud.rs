use super::super::DbState;
use super::dynamic_update;
use rusqlite::{params, Connection};

/// Parameters for creating a new account.
#[derive(Debug, Clone)]
pub struct CreateAccountParams {
    pub email: String,
    pub provider: String,
    pub display_name: Option<String>,
    pub account_name: String,
    pub account_color: String,
    pub auth_method: String,
    // OAuth fields
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub token_expires_at: Option<i64>,
    pub oauth_provider: Option<String>,
    pub oauth_client_id: Option<String>,
    // IMAP fields
    pub imap_host: Option<String>,
    pub imap_port: Option<i64>,
    pub imap_security: Option<String>,
    pub imap_username: Option<String>,
    pub imap_password: Option<String>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<i64>,
    pub smtp_security: Option<String>,
    // SMTP credential fields
    pub smtp_username: Option<String>,
    pub smtp_password: Option<String>,
    // JMAP fields
    pub jmap_url: Option<String>,
    pub accept_invalid_certs: bool,
}

/// Parameters for updating an existing account's metadata.
#[derive(Debug, Clone)]
pub struct UpdateAccountParams {
    pub account_name: Option<String>,
    pub display_name: Option<String>,
    pub account_color: Option<String>,
    pub caldav_url: Option<String>,
    pub caldav_username: Option<String>,
    pub caldav_password: Option<String>,
}

/// Synchronous account creation — callable from any `&Connection`.
///
/// This is the single source of truth for the INSERT statement. Both
/// the async `db_create_account` (via `DbState`) and the app crate's
/// `Db::with_write_conn` use this function.
pub fn create_account_sync(
    conn: &Connection,
    params: CreateAccountParams,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    // Determine sort_order: one past the current maximum
    let max_sort: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(sort_order), -1) AS max_sort FROM accounts",
            [],
            |row| row.get("max_sort"),
        )
        .map_err(|e| e.to_string())?;
    let sort_order = max_sort + 1;

    conn.execute(
        "INSERT INTO accounts (
            id, email, provider, display_name, account_name, account_color,
            auth_method, access_token, refresh_token, token_expires_at,
            oauth_provider, oauth_client_id,
            imap_host, imap_port, imap_security, imap_username, imap_password,
            smtp_host, smtp_port, smtp_security,
            smtp_username, smtp_password,
            jmap_url, accept_invalid_certs, sort_order
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6,
            ?7, ?8, ?9, ?10,
            ?11, ?12,
            ?13, ?14, ?15, ?16, ?17,
            ?18, ?19, ?20,
            ?21, ?22,
            ?23, ?24, ?25
        )",
        params![
            id,
            params.email,
            params.provider,
            params.display_name,
            params.account_name,
            params.account_color,
            params.auth_method,
            params.access_token,
            params.refresh_token,
            params.token_expires_at,
            params.oauth_provider,
            params.oauth_client_id,
            params.imap_host,
            params.imap_port,
            params.imap_security,
            params.imap_username,
            params.imap_password,
            params.smtp_host,
            params.smtp_port,
            params.smtp_security,
            params.smtp_username,
            params.smtp_password,
            params.jmap_url,
            i64::from(params.accept_invalid_certs),
            sort_order,
        ],
    )
    .map_err(|e| e.to_string())?;

    Ok(id)
}

/// Synchronous duplicate-email check — callable from any `&Connection`.
pub fn account_exists_by_email_sync(
    conn: &Connection,
    email: &str,
) -> Result<bool, String> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) AS cnt FROM accounts WHERE email = ?1",
            params![email],
            |row| row.get("cnt"),
        )
        .map_err(|e| e.to_string())?;
    Ok(count > 0)
}

/// Create a new account and return its generated ID.
pub async fn db_create_account(
    db: &DbState,
    params: CreateAccountParams,
) -> Result<String, String> {
    db.with_conn(move |conn| create_account_sync(conn, params))
        .await
}

/// Update an account's editable metadata fields. Only fields that are `Some`
/// in `params` are changed.
pub async fn db_update_account(
    db: &DbState,
    id: String,
    params: UpdateAccountParams,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let mut sets: Vec<(&str, Box<dyn rusqlite::types::ToSql>)> = Vec::new();
        if let Some(v) = params.account_name {
            sets.push(("account_name", Box::new(v)));
        }
        if let Some(v) = params.display_name {
            sets.push(("display_name", Box::new(v)));
        }
        if let Some(v) = params.account_color {
            sets.push(("account_color", Box::new(v)));
        }
        if let Some(v) = params.caldav_url {
            sets.push(("caldav_url", Box::new(v)));
        }
        if let Some(v) = params.caldav_username {
            sets.push(("caldav_username", Box::new(v)));
        }
        if let Some(v) = params.caldav_password {
            sets.push(("caldav_password", Box::new(v)));
        }
        dynamic_update(conn, "accounts", "id", &id, sets)
    })
    .await
}

/// Update only the account color.
pub async fn db_update_account_color(
    db: &DbState,
    id: String,
    color: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE accounts SET account_color = ?1 WHERE id = ?2",
            params![color, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

/// Update only the account name.
pub async fn db_update_account_name(
    db: &DbState,
    id: String,
    name: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE accounts SET account_name = ?1 WHERE id = ?2",
            params![name, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

/// Batch-update sort order for multiple accounts.
pub async fn db_update_account_sort_order(
    db: &DbState,
    updates: Vec<(String, i64)>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        {
            let mut stmt = tx
                .prepare("UPDATE accounts SET sort_order = ?1 WHERE id = ?2")
                .map_err(|e| e.to_string())?;
            for (id, order) in &updates {
                stmt.execute(params![order, id]).map_err(|e| e.to_string())?;
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

/// Check whether an account with the given email already exists.
pub async fn db_account_exists_by_email(
    db: &DbState,
    email: String,
) -> Result<bool, String> {
    db.with_conn(move |conn| account_exists_by_email_sync(conn, &email))
        .await
}
