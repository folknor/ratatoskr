use rusqlite::{Connection, OptionalExtension};

use crate::provider::crypto::{decrypt_value, encrypt_value, is_encrypted};

/// Derive an account name from an email address.
/// "alice@corp.com" → "Corp", "bob@gmail.com" → "Gmail"
fn derive_account_name(email: &str) -> String {
    let domain = email.split('@').nth(1).unwrap_or("Account");
    let name = domain.split('.').next().unwrap_or(domain);
    let mut chars = name.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => "Account".to_string(),
    }
}

/// Pick the next unused account color from the label-colors palette.
/// Falls back to the first color if all are used.
fn next_account_color(conn: &Connection) -> String {
    let used: Vec<String> = conn
        .prepare("SELECT account_color FROM accounts WHERE account_color IS NOT NULL")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| row.get::<_, String>(0))
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    let presets = ratatoskr_label_colors::category_colors::all_presets();
    for (_, bg, _) in presets {
        if !used.iter().any(|u| u == *bg) {
            return (*bg).to_string();
        }
    }
    // All used — return the first preset
    presets.first().map_or("#3498db".to_string(), |(_, bg, _)| (*bg).to_string())
}

/// Check if a Gmail account with the given email already exists.
/// Returns `Some(id)` if a duplicate exists.
pub fn check_gmail_duplicate(
    conn: &Connection,
    email: &str,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT id FROM accounts WHERE email = ?1 AND provider = 'gmail_api' LIMIT 1",
        rusqlite::params![email],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(|e| format!("Duplicate Gmail check failed: {e}"))
}

/// Parameters for inserting a new Gmail account.
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
}

/// Insert a new Gmail account into the database.
pub fn insert_gmail_account(
    conn: &Connection,
    params: &InsertGmailAccountParams,
) -> Result<(), String> {
    let account_name = derive_account_name(&params.email);
    let account_color = next_account_color(conn);
    conn.execute(
        "INSERT INTO accounts (id, email, display_name, avatar_url, access_token, \
         refresh_token, token_expires_at, provider, auth_method, oauth_client_id, \
         oauth_client_secret, account_name, account_color) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'gmail_api', 'oauth2', ?8, ?9, ?10, ?11)",
        rusqlite::params![
            params.account_id,
            params.email,
            params.display_name,
            params.avatar_url,
            params.access_token,
            params.refresh_token,
            params.expires_at,
            params.encrypted_client_id,
            params.encrypted_client_secret,
            account_name,
            account_color,
        ],
    )
    .map_err(|e| format!("Failed to insert Gmail account: {e}"))?;
    Ok(())
}

/// Parameters for inserting a new IMAP OAuth account.
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
}

/// Insert a new IMAP OAuth account into the database.
#[allow(clippy::too_many_lines)]
pub fn insert_imap_oauth_account(
    conn: &Connection,
    params: &InsertImapOAuthAccountParams,
) -> Result<(), String> {
    let account_name = derive_account_name(&params.email);
    let account_color = next_account_color(conn);
    conn.execute(
        "INSERT INTO accounts (id, email, display_name, avatar_url, access_token, \
         refresh_token, token_expires_at, provider, auth_method, imap_host, imap_port, \
         imap_security, smtp_host, smtp_port, smtp_security, oauth_provider, \
         oauth_client_id, oauth_client_secret, oauth_token_url, imap_username, \
         accept_invalid_certs, account_name, account_color) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'imap', 'oauth2', ?8, ?9, ?10, ?11, ?12, \
         ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
        rusqlite::params![
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
            account_name,
            account_color,
        ],
    )
    .map_err(|e| format!("Failed to insert OAuth IMAP account: {e}"))?;
    Ok(())
}

/// Parameters for inserting a new Graph (Microsoft) account.
pub struct InsertGraphAccountParams {
    pub account_id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    pub encrypted_client_id: String,
}

/// Insert a new Graph (Microsoft) account into the database.
pub fn insert_graph_account(
    conn: &Connection,
    params: &InsertGraphAccountParams,
) -> Result<(), String> {
    let account_name = derive_account_name(&params.email);
    let account_color = next_account_color(conn);
    conn.execute(
        "INSERT INTO accounts (id, email, display_name, avatar_url, access_token, \
         refresh_token, token_expires_at, provider, auth_method, oauth_client_id, \
         account_name, account_color) \
         VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, 'graph', 'oauth2', ?7, ?8, ?9)",
        rusqlite::params![
            params.account_id,
            params.email,
            params.display_name,
            params.access_token,
            params.refresh_token,
            params.expires_at,
            params.encrypted_client_id,
            account_name,
            account_color,
        ],
    )
    .map_err(|e| format!("Failed to insert Graph account: {e}"))?;
    Ok(())
}

/// Finalize a Graph account profile after fetching from Microsoft Graph API.
pub fn finalize_graph_profile(
    conn: &Connection,
    account_id: &str,
    email: &str,
    display_name: &str,
) -> Result<(), String> {
    // Re-derive account_name from the finalized email, since the email
    // known at insert time may differ from the real one returned by Graph.
    let account_name = derive_account_name(email);
    conn.execute(
        "UPDATE accounts SET email = ?1, display_name = ?2, account_name = ?3, \
         updated_at = unixepoch() \
         WHERE id = ?4",
        rusqlite::params![email, display_name, account_name, account_id],
    )
    .map_err(|e| format!("Failed to finalize Graph account profile: {e}"))?;
    Ok(())
}

/// Update tokens and optionally credentials for a Gmail reauthorization.
pub fn update_gmail_reauth_tokens(
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
            rusqlite::params![
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
            rusqlite::params![access_token, refresh_token, expires_at, account_id],
        )
        .map_err(|e| format!("Failed to update Gmail account tokens: {e}"))?;
    }
    Ok(())
}

/// Update tokens and optionally client ID for a Graph reauthorization.
pub fn update_graph_reauth_tokens(
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
            rusqlite::params![access_token, refresh_token, expires_at, enc_cid, account_id],
        )
        .map_err(|e| format!("Failed to update Graph account tokens: {e}"))?;
    } else {
        conn.execute(
            "UPDATE accounts SET access_token = ?1, refresh_token = ?2, \
             token_expires_at = ?3, updated_at = unixepoch() WHERE id = ?4",
            rusqlite::params![access_token, refresh_token, expires_at, account_id],
        )
        .map_err(|e| format!("Failed to update Graph account tokens: {e}"))?;
    }
    Ok(())
}

/// Resolve OAuth credentials for a Gmail reauthorization. If `client_id` is
/// provided, use it directly. Otherwise, read and decrypt stored credentials.
pub fn resolve_gmail_reauth_credentials(
    conn: &Connection,
    account_id: &str,
    encryption_key: &[u8; 32],
) -> Result<(String, String), String> {
    conn.query_row(
        "SELECT oauth_client_id, oauth_client_secret FROM accounts WHERE id = ?1",
        rusqlite::params![account_id],
        |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
            ))
        },
    )
    .map_err(|e| format!("Failed to read account credentials: {e}"))
    .and_then(|(cid, cs)| {
        let cid = cid.filter(|s| !s.is_empty()).ok_or_else(|| {
            "Account has no stored OAuth credentials. Provide client_id to reauthorize."
                .to_string()
        })?;
        let cid = if is_encrypted(&cid) {
            decrypt_value(encryption_key, &cid).unwrap_or(cid)
        } else {
            cid
        };
        let cs = cs
            .filter(|s| !s.is_empty())
            .map(|s| {
                if is_encrypted(&s) {
                    decrypt_value(encryption_key, &s).unwrap_or(s)
                } else {
                    s
                }
            })
            .unwrap_or_default();
        Ok((cid, cs))
    })
}

/// Resolve OAuth client ID for a Graph reauthorization. If not provided,
/// read and decrypt the stored client ID, falling back to a default.
pub fn resolve_graph_reauth_client_id(
    conn: &Connection,
    account_id: &str,
    encryption_key: &[u8; 32],
    default_client_id: &str,
) -> Result<String, String> {
    conn.query_row(
        "SELECT oauth_client_id FROM accounts WHERE id = ?1",
        rusqlite::params![account_id],
        |row| row.get::<_, Option<String>>(0),
    )
    .map_err(|e| format!("Failed to read account credentials: {e}"))
    .map(|cid| match cid.filter(|s| !s.is_empty()) {
        Some(encrypted) => {
            if is_encrypted(&encrypted) {
                decrypt_value(encryption_key, &encrypted).unwrap_or(encrypted)
            } else {
                encrypted
            }
        }
        None => default_client_id.to_string(),
    })
}

/// Encrypt OAuth tokens for storage. Returns `(encrypted_access, encrypted_refresh, expires_at)`.
pub fn encrypt_oauth_tokens(
    encryption_key: &[u8; 32],
    access_token: &str,
    refresh_token: &str,
    expires_in: u64,
) -> Result<(String, String, i64), String> {
    let access = encrypt_value(encryption_key, access_token)?;
    let refresh = encrypt_value(encryption_key, refresh_token)?;
    let expires_at = chrono::Utc::now().timestamp()
        + i64::try_from(expires_in).map_err(|_| "Token expiry overflow".to_string())?;
    Ok((access, refresh, expires_at))
}
