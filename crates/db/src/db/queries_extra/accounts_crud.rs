use super::super::{ReadConn, ReadDbState};
use super::dynamic_update;
use rusqlite::{Connection, OptionalExtension, params};
use types::MailProviderKind;

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
    pub oauth_token_url: Option<String>,
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
    /// Attachments roadmap Phase 6: per-account offline-cache master
    /// switch. `Some(true)` -> `cache_attachments_enabled = 1`,
    /// `Some(false)` -> `= 0`. None leaves the column untouched.
    pub cache_attachments_enabled: Option<bool>,
}

/// Synchronous account creation - callable from any `&Connection`.
///
/// This is the single source of truth for the INSERT statement. Both
/// the async `db_create_account` (via `ReadDbState`) and the app crate's
/// `Db::with_write_conn` use this function.
pub fn create_account_sync(
    conn: &Connection,
    params: &CreateAccountParams,
) -> Result<String, String> {
    log::info!(
        "Creating account: email={}, provider={}",
        params.email,
        params.provider
    );
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
            oauth_provider, oauth_client_id, oauth_token_url,
            imap_host, imap_port, imap_security, imap_username, imap_password,
            smtp_host, smtp_port, smtp_security,
            smtp_username, smtp_password,
            jmap_url, accept_invalid_certs, sort_order
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6,
            ?7, ?8, ?9, ?10,
            ?11, ?12, ?13,
            ?14, ?15, ?16, ?17, ?18,
            ?19, ?20, ?21,
            ?22, ?23,
            ?24, ?25, ?26
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
            params.oauth_token_url,
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
    .map_err(|e| {
        log::error!("Failed to create account {}: {e}", params.email);
        e.to_string()
    })?;

    log::info!("Account created: id={id}, email={}", params.email);
    Ok(id)
}

/// Synchronous duplicate-email check.
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

/// Create a new account and return its generated ID.
pub async fn db_create_account(
    db: &ReadDbState,
    params: CreateAccountParams,
) -> Result<String, String> {
    db.with_conn(move |conn| create_account_sync(conn, &params))
        .await
}

/// Synchronous account update - callable from any `&Connection`.
///
/// This is the single source of truth for the update logic. Both the async
/// `db_update_account` (via `ReadDbState`) and the app crate's
/// `Db::with_write_conn` use this function.
pub fn update_account_sync(
    conn: &Connection,
    id: &str,
    params: UpdateAccountParams,
) -> Result<(), String> {
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
    if let Some(v) = params.cache_attachments_enabled {
        sets.push(("cache_attachments_enabled", Box::new(if v { 1i64 } else { 0i64 })));
    }
    dynamic_update(conn, "accounts", "id", id, sets)
}

// db_update_account async wrapper removed in Phase 6a: the only
// caller (`handle_save_account_changes`) is now an IPC dispatch via
// `account.update`, which calls `update_account_sync` directly inside
// `WriteDbState::with_conn`.

/// Update only the account color.
pub async fn db_update_account_color(
    db: &ReadDbState,
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
pub async fn db_update_account_name(db: &ReadDbState, id: String, name: String) -> Result<(), String> {
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

/// Batch-update sort order for multiple accounts inside one
/// transaction. Account ids absent from `updates` keep their existing
/// `sort_order`; this matches the signature reorder shape and lets
/// callers reorder a sidebar subset without enumerating every row.
///
/// Phase 6a: paired sync helper for the `account.reorder` IPC handler.
pub fn update_account_sort_order_sync(
    conn: &Connection,
    updates: &[(String, i64)],
) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("account.reorder begin tx: {e}"))?;
    {
        let mut stmt = tx
            .prepare("UPDATE accounts SET sort_order = ?1 WHERE id = ?2")
            .map_err(|e| e.to_string())?;
        for (id, order) in updates {
            stmt.execute(params![order, id])
                .map_err(|e| e.to_string())?;
        }
    }
    tx.commit()
        .map_err(|e| format!("account.reorder commit: {e}"))?;
    Ok(())
}

/// Parameters for re-authentication - updating an existing account's
/// credentials without changing its identity or provider.
#[derive(Debug, Clone)]
pub struct ReauthAccountParams {
    /// OAuth fields (set when re-auth was OAuth-based)
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub token_expires_at: Option<i64>,
    /// IMAP/SMTP password fields (set when re-auth was password-based)
    pub imap_password: Option<String>,
    pub smtp_password: Option<String>,
}

#[derive(Debug, Clone)]
pub struct InsertGmailAccountParams {
    pub account_id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    pub encrypted_client_id: String,
    pub encrypted_client_secret: Option<String>,
    pub account_name: String,
    pub account_color: String,
}

#[derive(Debug, Clone)]
pub struct InsertImapOAuthAccountParams {
    pub account_id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    pub imap_host: String,
    pub imap_port: i64,
    pub imap_security: String,
    pub smtp_host: String,
    pub smtp_port: i64,
    pub smtp_security: String,
    pub oauth_provider: String,
    pub oauth_client_id: String,
    pub oauth_client_secret: Option<String>,
    pub oauth_token_url: Option<String>,
    pub imap_username: Option<String>,
    pub accept_invalid_certs: bool,
    pub account_name: String,
    pub account_color: String,
}

#[derive(Debug, Clone)]
pub struct InsertGraphAccountParams {
    pub account_id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    pub encrypted_client_id: String,
    pub account_name: String,
    pub account_color: String,
}

#[derive(Debug, Clone)]
pub struct StoredOAuthCredentials {
    pub oauth_client_id: Option<String>,
    pub oauth_client_secret: Option<String>,
}

pub fn check_gmail_duplicate_sync(conn: &Connection, email: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT id FROM accounts WHERE email = ?1 AND provider = 'gmail_api' LIMIT 1",
        params![email],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(|e| format!("Duplicate Gmail check failed: {e}"))
}

pub fn get_used_account_colors_sync(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT account_color FROM accounts WHERE account_color IS NOT NULL")
        .map_err(|e| format!("prepare used account colors: {e}"))?;
    stmt.query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| format!("query used account colors: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect used account colors: {e}"))
}

pub fn insert_gmail_account_sync(
    conn: &Connection,
    params: &InsertGmailAccountParams,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO accounts (id, email, display_name, avatar_url, access_token, \
         refresh_token, token_expires_at, provider, auth_method, oauth_client_id, \
         oauth_client_secret, account_name, account_color) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'gmail_api', 'oauth2', ?8, ?9, ?10, ?11)",
        params![
            params.account_id,
            params.email,
            params.display_name,
            params.avatar_url,
            params.access_token,
            params.refresh_token,
            params.expires_at,
            params.encrypted_client_id,
            params.encrypted_client_secret,
            params.account_name,
            params.account_color,
        ],
    )
    .map_err(|e| format!("Failed to insert Gmail account: {e}"))?;
    Ok(())
}

#[allow(clippy::too_many_lines)]
pub fn insert_imap_oauth_account_sync(
    conn: &Connection,
    params: &InsertImapOAuthAccountParams,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO accounts (id, email, display_name, avatar_url, access_token, \
         refresh_token, token_expires_at, provider, auth_method, imap_host, imap_port, \
         imap_security, smtp_host, smtp_port, smtp_security, oauth_provider, \
         oauth_client_id, oauth_client_secret, oauth_token_url, imap_username, \
         accept_invalid_certs, account_name, account_color) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'imap', 'oauth2', ?8, ?9, ?10, ?11, ?12, \
         ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
        params![
            params.account_id,
            params.email,
            params.display_name,
            params.avatar_url,
            params.access_token,
            params.refresh_token,
            params.expires_at,
            params.imap_host,
            params.imap_port,
            params.imap_security,
            params.smtp_host,
            params.smtp_port,
            params.smtp_security,
            params.oauth_provider,
            params.oauth_client_id,
            params.oauth_client_secret,
            params.oauth_token_url,
            params.imap_username,
            if params.accept_invalid_certs { 1 } else { 0 },
            params.account_name,
            params.account_color,
        ],
    )
    .map_err(|e| format!("Failed to insert OAuth IMAP account: {e}"))?;
    Ok(())
}

pub fn insert_graph_account_sync(
    conn: &Connection,
    params: &InsertGraphAccountParams,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO accounts (id, email, display_name, avatar_url, access_token, \
         refresh_token, token_expires_at, provider, auth_method, oauth_client_id, \
         account_name, account_color) \
         VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, 'graph', 'oauth2', ?7, ?8, ?9)",
        params![
            params.account_id,
            params.email,
            params.display_name,
            params.access_token,
            params.refresh_token,
            params.expires_at,
            params.encrypted_client_id,
            params.account_name,
            params.account_color,
        ],
    )
    .map_err(|e| format!("Failed to insert Graph account: {e}"))?;
    Ok(())
}

pub fn finalize_graph_profile_sync(
    conn: &Connection,
    account_id: &str,
    email: &str,
    display_name: &str,
    account_name: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE accounts SET email = ?1, display_name = ?2, account_name = ?3, \
         updated_at = unixepoch() \
         WHERE id = ?4",
        params![email, display_name, account_name, account_id],
    )
    .map_err(|e| format!("Failed to finalize Graph account profile: {e}"))?;
    Ok(())
}

pub fn update_gmail_reauth_tokens_sync(
    conn: &Connection,
    account_id: &str,
    access_token: &str,
    refresh_token: &str,
    expires_at: i64,
    new_encrypted_cid: Option<&str>,
    new_encrypted_cs: Option<&str>,
) -> Result<(), String> {
    if let Some(enc_cid) = new_encrypted_cid {
        conn.execute(
            "UPDATE accounts SET access_token = ?1, refresh_token = ?2, \
             token_expires_at = ?3, oauth_client_id = ?4, oauth_client_secret = ?5, \
             updated_at = unixepoch() WHERE id = ?6",
            params![
                access_token,
                refresh_token,
                expires_at,
                enc_cid,
                new_encrypted_cs,
                account_id,
            ],
        )
        .map_err(|e| format!("Failed to update Gmail account tokens: {e}"))?;
    } else {
        conn.execute(
            "UPDATE accounts SET access_token = ?1, refresh_token = ?2, \
             token_expires_at = ?3, updated_at = unixepoch() WHERE id = ?4",
            params![access_token, refresh_token, expires_at, account_id],
        )
        .map_err(|e| format!("Failed to update Gmail account tokens: {e}"))?;
    }
    Ok(())
}

pub fn update_graph_reauth_tokens_sync(
    conn: &Connection,
    account_id: &str,
    access_token: &str,
    refresh_token: &str,
    expires_at: i64,
    new_encrypted_cid: Option<&str>,
) -> Result<(), String> {
    if let Some(enc_cid) = new_encrypted_cid {
        conn.execute(
            "UPDATE accounts SET access_token = ?1, refresh_token = ?2, \
             token_expires_at = ?3, oauth_client_id = ?4, \
             updated_at = unixepoch() WHERE id = ?5",
            params![access_token, refresh_token, expires_at, enc_cid, account_id],
        )
        .map_err(|e| format!("Failed to update Graph account tokens: {e}"))?;
    } else {
        conn.execute(
            "UPDATE accounts SET access_token = ?1, refresh_token = ?2, \
             token_expires_at = ?3, updated_at = unixepoch() WHERE id = ?4",
            params![access_token, refresh_token, expires_at, account_id],
        )
        .map_err(|e| format!("Failed to update Graph account tokens: {e}"))?;
    }
    Ok(())
}

pub fn get_stored_oauth_credentials_sync(
    conn: &Connection,
    account_id: &str,
) -> Result<StoredOAuthCredentials, String> {
    conn.query_row(
        "SELECT oauth_client_id, oauth_client_secret FROM accounts WHERE id = ?1",
        params![account_id],
        |row| {
            Ok(StoredOAuthCredentials {
                oauth_client_id: row.get::<_, Option<String>>(0)?,
                oauth_client_secret: row.get::<_, Option<String>>(1)?,
            })
        },
    )
    .map_err(|e| format!("Failed to read account credentials: {e}"))
}

pub fn get_stored_graph_client_id_sync(
    conn: &Connection,
    account_id: &str,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT oauth_client_id FROM accounts WHERE id = ?1",
        params![account_id],
        |row| row.get::<_, Option<String>>(0),
    )
    .map_err(|e| format!("Failed to read account credentials: {e}"))
}

/// Synchronous token/credential update for re-authentication.
/// Updates only the credential columns for an existing account.
pub fn update_account_tokens_sync(
    conn: &Connection,
    account_id: &str,
    params: ReauthAccountParams,
) -> Result<(), String> {
    let mut sets: Vec<(&str, Box<dyn rusqlite::types::ToSql>)> = Vec::new();
    if let Some(v) = params.access_token {
        sets.push(("access_token", Box::new(v)));
    }
    if let Some(v) = params.refresh_token {
        sets.push(("refresh_token", Box::new(v)));
    }
    if let Some(v) = params.token_expires_at {
        sets.push(("token_expires_at", Box::new(v)));
    }
    if let Some(v) = params.imap_password {
        sets.push(("imap_password", Box::new(v)));
    }
    if let Some(v) = params.smtp_password {
        sets.push(("smtp_password", Box::new(v)));
    }
    super::dynamic_update(conn, "accounts", "id", account_id, sets)
}

/// Lightweight auth info for re-authentication. Contains just enough
/// to determine which auth flow to run and pre-populate server fields.
#[derive(Debug, Clone)]
pub struct AccountAuthInfo {
    pub provider: String,
    pub auth_method: String,
    pub oauth_provider: Option<String>,
    pub oauth_client_id: Option<String>,
    // Server fields for password re-auth
    pub imap_host: Option<String>,
    pub imap_port: Option<i64>,
    pub imap_security: Option<String>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<i64>,
    pub smtp_security: Option<String>,
    pub imap_username: Option<String>,
}

/// Fetch the auth info for a single account (synchronous).
pub fn get_account_auth_info_sync(
    conn: &ReadConn<'_>,
    account_id: &str,
) -> Result<AccountAuthInfo, String> {
    conn.query_row(
        "SELECT provider, auth_method, oauth_provider, oauth_client_id,
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

/// Look up the typed mail provider for an account.
pub fn get_account_provider_sync(
    conn: &Connection,
    account_id: &str,
) -> Result<MailProviderKind, String> {
    let raw = get_account_provider_raw_sync(conn, account_id)?;
    MailProviderKind::parse(&raw)
}

/// Look up the raw provider string for an account.
///
/// This is for explicit boundary code such as harness-provider handling.
/// Normal mail-provider callers should use `get_account_provider_sync`.
pub fn get_account_provider_raw_sync(conn: &Connection, account_id: &str) -> Result<String, String> {
    conn.query_row(
        "SELECT provider FROM accounts WHERE id = ?1",
        params![account_id],
        |row| row.get(0),
    )
    .map_err(|e| format!("lookup provider: {e}"))
}

/// Check whether an account with the given email already exists.
pub async fn db_account_exists_by_email(db: &ReadDbState, email: String) -> Result<bool, String> {
    db.with_read(move |conn| account_exists_by_email_sync(conn, &email))
        .await
}

#[cfg(test)]
mod sync_account_tests {
    use super::*;
    use crate::db::migrations;
    use rusqlite::Connection;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("pragmas");
        migrations::run_all(&conn).expect("migrations");
        // Three accounts so reorder has something to permute.
        for (id, sort_order) in [("acc-a", 0), ("acc-b", 1), ("acc-c", 2)] {
            conn.execute(
                "INSERT INTO accounts (id, email, sort_order) \
                 VALUES (?1, ?2, ?3)",
                params![id, format!("{id}@example.com"), sort_order],
            )
            .expect("seed account");
        }
        conn
    }

    fn get_sort_order(conn: &Connection, id: &str) -> i64 {
        conn.query_row(
            "SELECT sort_order FROM accounts WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .expect("sort_order")
    }

    fn get_text_col(conn: &Connection, id: &str, col: &str) -> Option<String> {
        let sql = format!("SELECT {col} FROM accounts WHERE id = ?1");
        conn.query_row(&sql, params![id], |row| row.get(0))
            .expect("text col")
    }

    #[test]
    fn update_account_sync_partial_only_changes_named_fields() {
        let conn = setup_db();
        // Seed initial values.
        conn.execute(
            "UPDATE accounts SET account_name = 'Old', display_name = 'Old D' \
             WHERE id = 'acc-a'",
            [],
        )
        .expect("seed");

        update_account_sync(
            &conn,
            "acc-a",
            UpdateAccountParams {
                account_name: Some("New".into()),
                display_name: None,
                account_color: None,
                caldav_url: None,
                caldav_username: None,
                caldav_password: None,
                cache_attachments_enabled: None,
            },
        )
        .expect("update");

        assert_eq!(get_text_col(&conn, "acc-a", "account_name"), Some("New".into()));
        assert_eq!(
            get_text_col(&conn, "acc-a", "display_name"),
            Some("Old D".into()),
            "display_name must survive a single-field update"
        );
    }

    #[test]
    fn reorder_assigns_indices_in_order() {
        let conn = setup_db();
        // Reorder to c, a, b.
        update_account_sort_order_sync(
            &conn,
            &[
                ("acc-c".into(), 0),
                ("acc-a".into(), 1),
                ("acc-b".into(), 2),
            ],
        )
        .expect("reorder");
        assert_eq!(get_sort_order(&conn, "acc-c"), 0);
        assert_eq!(get_sort_order(&conn, "acc-a"), 1);
        assert_eq!(get_sort_order(&conn, "acc-b"), 2);
    }

    #[test]
    fn reorder_leaves_absent_ids_untouched() {
        let conn = setup_db();
        // Set acc-c to a sentinel value; reorder only acc-a and
        // acc-b; assert acc-c is unchanged.
        conn.execute(
            "UPDATE accounts SET sort_order = 99 WHERE id = 'acc-c'",
            [],
        )
        .expect("seed sort_order");

        update_account_sort_order_sync(
            &conn,
            &[("acc-b".into(), 0), ("acc-a".into(), 1)],
        )
        .expect("reorder");

        assert_eq!(get_sort_order(&conn, "acc-b"), 0);
        assert_eq!(get_sort_order(&conn, "acc-a"), 1);
        assert_eq!(
            get_sort_order(&conn, "acc-c"),
            99,
            "absent ids keep their prior sort_order"
        );
    }

    #[test]
    fn reorder_empty_list_is_noop() {
        let conn = setup_db();
        update_account_sort_order_sync(&conn, &[]).expect("noop");
        assert_eq!(get_sort_order(&conn, "acc-a"), 0);
        assert_eq!(get_sort_order(&conn, "acc-b"), 1);
        assert_eq!(get_sort_order(&conn, "acc-c"), 2);
    }
}
