use crate::db::Connection;
use crate::db::queries_extra::{
    InsertGmailAccountParams as DbInsertGmailAccountParams,
    InsertGraphAccountParams as DbInsertGraphAccountParams,
    InsertImapOAuthAccountParams as DbInsertImapOAuthAccountParams, check_gmail_duplicate_sync,
    finalize_graph_profile_sync, get_stored_graph_client_id_sync,
    get_stored_oauth_credentials_sync, get_used_account_colors_sync, insert_gmail_account_sync,
    insert_graph_account_sync, insert_imap_oauth_account_sync, update_gmail_reauth_tokens_sync,
    update_graph_reauth_tokens_sync,
};
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
    let used = get_used_account_colors_sync(conn).unwrap_or_default();

    let presets = label_colors::preset_colors::all_presets();
    for (_, bg, _) in presets {
        if !used.iter().any(|u| u == *bg) {
            return (*bg).to_string();
        }
    }
    // All used - return the first preset
    presets
        .first()
        .map_or("#3498db".to_string(), |(_, bg, _)| (*bg).to_string())
}

/// Check if a Gmail account with the given email already exists.
/// Returns `Some(id)` if a duplicate exists.
pub fn check_gmail_duplicate(conn: &Connection, email: &str) -> Result<Option<String>, String> {
    check_gmail_duplicate_sync(conn, email)
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
    pub account_name: String,
    pub account_color: String,
}

/// Insert a new Gmail account into the database.
pub fn insert_gmail_account(
    conn: &Connection,
    params: &InsertGmailAccountParams,
) -> Result<(), String> {
    insert_gmail_account_sync(
        conn,
        &DbInsertGmailAccountParams {
            account_id: params.account_id.clone(),
            email: params.email.clone(),
            display_name: params.display_name.clone(),
            avatar_url: params.avatar_url.clone(),
            access_token: params.access_token.clone(),
            refresh_token: params.refresh_token.clone(),
            expires_at: params.expires_at,
            encrypted_client_id: params.encrypted_client_id.clone(),
            encrypted_client_secret: params.encrypted_client_secret.clone(),
            account_name: params.account_name.clone(),
            account_color: params.account_color.clone(),
        },
    )
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
    pub account_name: String,
    pub account_color: String,
}

/// Insert a new IMAP OAuth account into the database.
#[allow(clippy::too_many_lines)]
pub fn insert_imap_oauth_account(
    conn: &Connection,
    params: &InsertImapOAuthAccountParams,
) -> Result<(), String> {
    insert_imap_oauth_account_sync(
        conn,
        &DbInsertImapOAuthAccountParams {
            account_id: params.account_id.clone(),
            email: params.email.clone(),
            display_name: params.display_name.clone(),
            avatar_url: params.avatar_url.clone(),
            access_token: params.access_token.clone(),
            refresh_token: params.refresh_token.clone(),
            expires_at: params.expires_at,
            imap_host: params.imap_host.clone(),
            imap_port: params.imap_port,
            imap_security: params.imap_security.clone(),
            smtp_host: params.smtp_host.clone(),
            smtp_port: params.smtp_port,
            smtp_security: params.smtp_security.clone(),
            oauth_provider: params.oauth_provider.clone(),
            oauth_client_id: params.oauth_client_id.clone(),
            oauth_client_secret: params.oauth_client_secret.clone(),
            oauth_token_url: params.oauth_token_url.clone(),
            imap_username: params.imap_username.clone(),
            accept_invalid_certs: params.accept_invalid_certs,
            account_name: params.account_name.clone(),
            account_color: params.account_color.clone(),
        },
    )
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
    pub account_name: String,
    pub account_color: String,
}

/// Insert a new Graph (Microsoft) account into the database.
pub fn insert_graph_account(
    conn: &Connection,
    params: &InsertGraphAccountParams,
) -> Result<(), String> {
    insert_graph_account_sync(
        conn,
        &DbInsertGraphAccountParams {
            account_id: params.account_id.clone(),
            email: params.email.clone(),
            display_name: params.display_name.clone(),
            access_token: params.access_token.clone(),
            refresh_token: params.refresh_token.clone(),
            expires_at: params.expires_at,
            encrypted_client_id: params.encrypted_client_id.clone(),
            account_name: params.account_name.clone(),
            account_color: params.account_color.clone(),
        },
    )
}

/// Finalize a Graph account profile after fetching from Microsoft Graph API.
pub fn finalize_graph_profile(
    conn: &Connection,
    account_id: &str,
    email: &str,
    display_name: &str,
) -> Result<(), String> {
    let account_name = derive_account_name(email);
    finalize_graph_profile_sync(conn, account_id, email, display_name, &account_name)
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
    update_gmail_reauth_tokens_sync(
        conn,
        account_id,
        access_token,
        refresh_token,
        expires_at,
        new_encrypted_cid,
        new_encrypted_cs,
    )
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
    update_graph_reauth_tokens_sync(
        conn,
        account_id,
        access_token,
        refresh_token,
        expires_at,
        new_encrypted_cid,
    )
}

/// Resolve OAuth credentials for a Gmail reauthorization. If `client_id` is
/// provided, use it directly. Otherwise, read and decrypt stored credentials.
pub fn resolve_gmail_reauth_credentials(
    conn: &Connection,
    account_id: &str,
    encryption_key: &[u8; 32],
) -> Result<(String, String), String> {
    get_stored_oauth_credentials_sync(conn, account_id).and_then(|creds| {
        let cid = creds.oauth_client_id.filter(|s| !s.is_empty()).ok_or_else(|| {
            "Account has no stored OAuth credentials. Provide client_id to reauthorize.".to_string()
        })?;
        let cid = if is_encrypted(&cid) {
            decrypt_value(encryption_key, &cid).unwrap_or(cid)
        } else {
            cid
        };
        let cs = creds
            .oauth_client_secret
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
    get_stored_graph_client_id_sync(conn, account_id).map(|cid| match cid.filter(|s| !s.is_empty()) {
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
