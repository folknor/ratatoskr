use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use rusqlite::OptionalExtension;

use ratatoskr_db::db::DbState;
use ratatoskr_provider_utils::crypto::{decrypt_value, encrypt_value, is_encrypted};
use ratatoskr_provider_utils::token::refresh_oauth_token;
use ratatoskr_smtp::types::SmtpConfig;

use super::types::ImapConfig;

static IMAP_REFRESH_LOCKS: OnceLock<StdMutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    OnceLock::new();

fn shared_http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

fn get_refresh_lock(account_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    let map = IMAP_REFRESH_LOCKS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut guard = map.lock().expect("IMAP refresh lock map poisoned");
    guard
        .entry(account_id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

const MICROSOFT_TOKEN_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";
const YAHOO_TOKEN_URL: &str = "https://api.login.yahoo.com/oauth2/get_token";

struct AccountConfigRecord {
    email: String,
    imap_host: Option<String>,
    imap_port: Option<i64>,
    imap_security: Option<String>,
    smtp_host: Option<String>,
    smtp_port: Option<i64>,
    smtp_security: Option<String>,
    imap_username: Option<String>,
    imap_password: Option<String>,
    auth_method: String,
    accept_invalid_certs: bool,
    access_token: Option<String>,
    token_expires_at: Option<i64>,
    oauth_provider: Option<String>,
    oauth_client_id: Option<String>,
    oauth_client_secret: Option<String>,
    oauth_token_url: Option<String>,
}

pub struct ImapAndSmtpConfig {
    pub imap: ImapConfig,
    pub smtp: SmtpConfig,
}

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

fn decrypt_if_needed(
    encryption_key: &[u8; 32],
    value: Option<String>,
) -> Result<Option<String>, String> {
    value
        .map(|raw| {
            if is_encrypted(&raw) {
                decrypt_value(encryption_key, &raw)
                    .map_err(|e| format!("decrypt stored account credential: {e}"))
            } else {
                Ok(raw)
            }
        })
        .transpose()
}

fn oauth_token_endpoint<'a>(
    provider_id: &str,
    stored_url: Option<&'a str>,
) -> Result<std::borrow::Cow<'a, str>, String> {
    if let Some(url) = stored_url.filter(|u| !u.is_empty()) {
        return Ok(std::borrow::Cow::Borrowed(url));
    }
    match provider_id {
        "microsoft" | "microsoft_graph" => Ok(std::borrow::Cow::Borrowed(MICROSOFT_TOKEN_URL)),
        "yahoo" => Ok(std::borrow::Cow::Borrowed(YAHOO_TOKEN_URL)),
        other => Err(format!(
            "Unsupported OAuth provider for IMAP account: {other}. Set oauth_token_url in the account record."
        )),
    }
}

async fn load_account_record(
    db: &DbState,
    account_id: &str,
) -> Result<AccountConfigRecord, String> {
    let aid = account_id.to_string();
    let account_id_for_error = account_id.to_string();
    db.with_conn(move |conn| {
        conn.query_row(
            "SELECT email, imap_host, imap_port, imap_security, smtp_host, smtp_port, \
             smtp_security, imap_username, imap_password, auth_method, accept_invalid_certs, \
             access_token, token_expires_at, oauth_provider, oauth_client_id, \
             oauth_client_secret, oauth_token_url FROM accounts WHERE id = ?1",
            rusqlite::params![aid],
            |row| {
                let accept_invalid_certs: Option<i64> = row.get("accept_invalid_certs")?;
                Ok(AccountConfigRecord {
                    email: row.get("email")?,
                    imap_host: row.get("imap_host")?,
                    imap_port: row.get("imap_port")?,
                    imap_security: row.get("imap_security")?,
                    smtp_host: row.get("smtp_host")?,
                    smtp_port: row.get("smtp_port")?,
                    smtp_security: row.get("smtp_security")?,
                    imap_username: row.get("imap_username")?,
                    imap_password: row.get("imap_password")?,
                    auth_method: row
                        .get::<_, Option<String>>("auth_method")?
                        .unwrap_or_else(|| "password".to_string()),
                    accept_invalid_certs: accept_invalid_certs.unwrap_or(0) != 0,
                    access_token: row.get("access_token")?,
                    token_expires_at: row.get("token_expires_at")?,
                    oauth_provider: row.get("oauth_provider")?,
                    oauth_client_id: row.get("oauth_client_id")?,
                    oauth_client_secret: row.get("oauth_client_secret")?,
                    oauth_token_url: row.get("oauth_token_url")?,
                })
            },
        )
        .optional()
        .map_err(|e| format!("Failed to read IMAP account config for {account_id_for_error}: {e}"))?
        .ok_or_else(|| format!("Account {account_id_for_error} not found"))
    })
    .await
}

async fn ensure_oauth_access_token(
    db: &DbState,
    account_id: &str,
    encryption_key: &[u8; 32],
    record: &AccountConfigRecord,
) -> Result<String, String> {
    let oauth_provider = record
        .oauth_provider
        .as_deref()
        .ok_or_else(|| format!("OAuth IMAP account {account_id} has no provider configured"))?;
    let client_id = record
        .oauth_client_id
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("OAuth IMAP account {account_id} has no client ID"))?;
    let access_token = decrypt_if_needed(encryption_key, record.access_token.clone())?
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("OAuth IMAP account {account_id} has no access token"))?;
    let expires_at = record.token_expires_at.unwrap_or_default();

    if expires_at - chrono::Utc::now().timestamp() >= 300 {
        return Ok(access_token);
    }

    // Acquire per-account lock to prevent concurrent refreshes
    let lock = get_refresh_lock(account_id);
    let _guard = lock.lock().await;

    // Double-check after acquiring lock — another task may have already refreshed
    let aid = account_id.to_string();
    let (fresh_access, fresh_expires, fresh_refresh) = db
        .with_conn(move |conn| {
            conn.query_row(
                "SELECT access_token, token_expires_at, refresh_token FROM accounts WHERE id = ?1",
                rusqlite::params![aid],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>("access_token")?,
                        row.get::<_, Option<i64>>("token_expires_at")?,
                        row.get::<_, Option<String>>("refresh_token")?,
                    ))
                },
            )
            .map_err(|e| format!("Re-check token query failed: {e}"))
        })
        .await?;

    if fresh_expires.unwrap_or_default() - chrono::Utc::now().timestamp() >= 300 {
        return decrypt_if_needed(encryption_key, fresh_access)?
            .filter(|v| !v.is_empty())
            .ok_or_else(|| format!("IMAP token re-check: missing access token for {account_id}"));
    }

    let refresh_token = decrypt_if_needed(encryption_key, fresh_refresh)?
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("OAuth IMAP account {account_id} has no refresh token"))?;
    let client_secret = decrypt_if_needed(encryption_key, record.oauth_client_secret.clone())?;
    let token_url = oauth_token_endpoint(oauth_provider, record.oauth_token_url.as_deref())?;
    let refreshed = refresh_oauth_token(
        shared_http_client(),
        &token_url,
        &refresh_token,
        client_id,
        client_secret.as_deref(),
    )
    .await?;
    let encrypted_access_token = encrypt_value(encryption_key, &refreshed.access_token)?;
    let aid = account_id.to_string();
    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE accounts SET access_token = ?1, token_expires_at = ?2, \
             updated_at = unixepoch() WHERE id = ?3",
            rusqlite::params![encrypted_access_token, refreshed.expires_at, aid],
        )
        .map_err(|e| format!("Failed to persist refreshed IMAP OAuth token: {e}"))?;
        Ok(())
    })
    .await?;

    Ok(refreshed.access_token)
}

async fn resolve_account_password(
    db: &DbState,
    account_id: &str,
    encryption_key: &[u8; 32],
    record: &AccountConfigRecord,
) -> Result<String, String> {
    if record.auth_method == "oauth2" {
        ensure_oauth_access_token(db, account_id, encryption_key, record).await
    } else {
        Ok(decrypt_if_needed(encryption_key, record.imap_password.clone())?.unwrap_or_default())
    }
}

fn username_for_record(record: &AccountConfigRecord) -> String {
    record
        .imap_username
        .clone()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| record.email.clone())
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn imap_config_from_record(
    account_id: &str,
    record: &AccountConfigRecord,
    username: String,
    password: String,
) -> Result<ImapConfig, String> {
    let host = record
        .imap_host
        .clone()
        .ok_or_else(|| format!("Account {account_id} has no IMAP host configured"))?;

    Ok(ImapConfig {
        host,
        port: record.imap_port.unwrap_or(993) as u16,
        security: map_security(record.imap_security.as_deref()),
        username,
        password,
        auth_method: record.auth_method.clone(),
        accept_invalid_certs: record.accept_invalid_certs,
    })
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn smtp_config_from_record(
    account_id: &str,
    record: &AccountConfigRecord,
    username: String,
    password: String,
) -> Result<SmtpConfig, String> {
    // The schema stores a single password/token for both IMAP and SMTP auth.
    // Until separate SMTP credentials exist, SMTP deliberately reuses `imap_password`.
    let host = record
        .smtp_host
        .clone()
        .ok_or_else(|| format!("Account {account_id} has no SMTP host configured"))?;

    Ok(SmtpConfig {
        host,
        port: record.smtp_port.unwrap_or(587) as u16,
        security: map_security(record.smtp_security.as_deref()),
        username,
        password,
        auth_method: record.auth_method.clone(),
        accept_invalid_certs: record.accept_invalid_certs,
    })
}

pub async fn load_imap_config(
    db: &DbState,
    account_id: &str,
    encryption_key: &[u8; 32],
) -> Result<ImapConfig, String> {
    let record = load_account_record(db, account_id).await?;
    let username = username_for_record(&record);
    let password = resolve_account_password(db, account_id, encryption_key, &record).await?;
    imap_config_from_record(account_id, &record, username, password)
}

pub async fn load_smtp_config(
    db: &DbState,
    account_id: &str,
    encryption_key: &[u8; 32],
) -> Result<SmtpConfig, String> {
    let record = load_account_record(db, account_id).await?;
    let username = username_for_record(&record);
    let password = resolve_account_password(db, account_id, encryption_key, &record).await?;
    smtp_config_from_record(account_id, &record, username, password)
}

pub async fn load_both_configs(
    db: &DbState,
    account_id: &str,
    encryption_key: &[u8; 32],
) -> Result<ImapAndSmtpConfig, String> {
    let record = load_account_record(db, account_id).await?;
    let username = username_for_record(&record);
    let password = resolve_account_password(db, account_id, encryption_key, &record).await?;
    let imap = imap_config_from_record(account_id, &record, username.clone(), password.clone())?;
    let smtp = smtp_config_from_record(account_id, &record, username, password)?;
    Ok(ImapAndSmtpConfig { imap, smtp })
}

#[cfg(test)]
mod tests {
    use super::decrypt_if_needed;

    #[test]
    fn decrypt_failure_returns_err() {
        let key = [7_u8; 32];
        let encrypted_like = Some("AAAAAAAAAAAAAAAA:AAAA".to_string());
        let err = decrypt_if_needed(&key, encrypted_like).expect_err("expected decrypt failure");
        assert!(err.contains("decrypt stored account credential"));
    }
}
