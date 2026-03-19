use std::sync::{Arc, RwLock as StdRwLock};

use jmap_client::client::{Client, Credentials};
use tokio::sync::RwLock;

use ratatoskr_db::db::DbState;
use ratatoskr_provider_utils::crypto::{decrypt_if_needed, encrypt_value};
use ratatoskr_provider_utils::http::shared_http_client;
use ratatoskr_provider_utils::token::{get_refresh_lock, oauth_token_endpoint, refresh_oauth_token};

/// Cached mailbox list entry: (mailbox_id, role, name).
pub type MailboxListEntry = (String, Option<String>, String);

/// Per-account JMAP client with support for both Basic and Bearer (OAuth2)
/// authentication.
///
/// For Basic auth the wrapped client is immutable after construction.
/// For OAuth/Bearer, the client is rebuilt when the access token is refreshed.
#[derive(Clone)]
pub struct JmapClient {
    /// The underlying `jmap_client::Client`, swapped atomically on token refresh.
    inner: Arc<StdRwLock<Arc<Client>>>,

    /// Cached mailbox list with timestamp for TTL-based invalidation.
    mailbox_cache: Arc<RwLock<MailboxCache>>,

    // ── OAuth infrastructure (only used when auth_method == "oauth2") ────
    db: Option<DbState>,
    account_id: String,
    encryption_key: Option<[u8; 32]>,
    auth_method: String,
    jmap_url: String,
}

/// Cached mailbox list with fetch timestamp for TTL-based invalidation.
type MailboxCache = Option<(Vec<MailboxListEntry>, std::time::Instant)>;

/// Mailbox cache TTL — 60 seconds matches Graph's folder_map_age threshold.
const MAILBOX_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(60);

impl JmapClient {
    /// Get an `Arc<Client>` for making API calls.
    ///
    /// For OAuth accounts, callers should call [`ensure_valid_token`] first
    /// to trigger a refresh if the token is about to expire.
    ///
    /// Returns a cloned `Arc` so the caller can hold it across `.await`
    /// points without blocking token refresh.
    pub fn inner(&self) -> Arc<Client> {
        self.inner
            .read()
            .expect("JMAP client lock poisoned")
            .clone()
    }

    /// Ensure the access token is valid, refreshing if needed.
    ///
    /// For Basic auth accounts this is a no-op. For OAuth accounts it
    /// checks the token expiry in the DB and refreshes if <5 min remain,
    /// following the same double-check-under-lock pattern as IMAP OAuth.
    pub async fn ensure_valid_token(&self) -> Result<(), String> {
        if self.auth_method != "oauth2" {
            return Ok(());
        }

        let db = self
            .db
            .as_ref()
            .ok_or("JMAP OAuth client missing DB reference")?;
        let key = self
            .encryption_key
            .ok_or("JMAP OAuth client missing encryption key")?;

        // Quick check: is the token still valid?
        let aid = self.account_id.clone();
        let (expires_at,) = db
            .with_conn(move |conn| {
                conn.query_row(
                    "SELECT token_expires_at FROM accounts WHERE id = ?1",
                    rusqlite::params![aid],
                    |row| Ok((row.get::<_, Option<i64>>(0)?,)),
                )
                .map_err(|e| format!("JMAP token expiry check: {e}"))
            })
            .await?;

        let expires_at = expires_at.unwrap_or_default();
        if expires_at - chrono::Utc::now().timestamp() >= 300 {
            return Ok(());
        }

        // Token is expiring — acquire per-account lock
        let lock = get_refresh_lock(&self.account_id);
        let _guard = lock.lock().await;

        // Double-check after acquiring lock — another task may have refreshed
        let aid = self.account_id.clone();
        let (fresh_access, fresh_expires, fresh_refresh, oauth_provider, oauth_client_id, oauth_client_secret, oauth_token_url) = db
            .with_conn(move |conn| {
                conn.query_row(
                    "SELECT access_token, token_expires_at, refresh_token, \
                     oauth_provider, oauth_client_id, oauth_client_secret, oauth_token_url \
                     FROM accounts WHERE id = ?1",
                    rusqlite::params![aid],
                    |row| {
                        Ok((
                            row.get::<_, Option<String>>(0)?,
                            row.get::<_, Option<i64>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, Option<String>>(4)?,
                            row.get::<_, Option<String>>(5)?,
                            row.get::<_, Option<String>>(6)?,
                        ))
                    },
                )
                .map_err(|e| format!("JMAP token re-check: {e}"))
            })
            .await?;

        if fresh_expires.unwrap_or_default() - chrono::Utc::now().timestamp() >= 300 {
            // Another task refreshed — rebuild client with the fresh token
            let access_token = decrypt_if_needed(&key, fresh_access)?
                .filter(|v| !v.is_empty())
                .ok_or_else(|| {
                    format!(
                        "JMAP token re-check: missing access token for {}",
                        self.account_id
                    )
                })?;
            self.rebuild_client_with_token(&access_token).await?;
            return Ok(());
        }

        // Need to actually refresh
        let refresh_token = decrypt_if_needed(&key, fresh_refresh)?
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                format!(
                    "JMAP OAuth account {} has no refresh token",
                    self.account_id
                )
            })?;
        let client_id = oauth_client_id
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                format!(
                    "JMAP OAuth account {} has no client ID",
                    self.account_id
                )
            })?;
        let client_secret = decrypt_if_needed(&key, oauth_client_secret)?;
        let provider = oauth_provider.unwrap_or_default();
        let token_url = oauth_token_endpoint(&provider, oauth_token_url.as_deref())?;

        let refreshed = refresh_oauth_token(
            shared_http_client(),
            &token_url,
            &refresh_token,
            &client_id,
            client_secret.as_deref(),
        )
        .await?;

        // Persist new token
        let encrypted_access = encrypt_value(&key, &refreshed.access_token)?;
        let aid = self.account_id.clone();
        let new_expires = refreshed.expires_at;
        db.with_conn(move |conn| {
            ratatoskr_db::db::queries::persist_refreshed_token(
                conn,
                &aid,
                &encrypted_access,
                new_expires,
            )
        })
        .await?;

        log::info!(
            "JMAP OAuth token refreshed for account {}",
            self.account_id
        );

        // Rebuild the inner client with the new token
        self.rebuild_client_with_token(&refreshed.access_token)
            .await?;

        Ok(())
    }

    /// Rebuild the inner `jmap_client::Client` with a new Bearer token.
    async fn rebuild_client_with_token(&self, access_token: &str) -> Result<(), String> {
        let client = Client::new()
            .credentials(Credentials::bearer(access_token))
            .connect(&self.jmap_url)
            .await
            .map_err(|e| format!("JMAP reconnect with new token failed: {e}"))?;

        *self
            .inner
            .write()
            .expect("JMAP client lock poisoned") = Arc::new(client);

        // Invalidate mailbox cache — session may have changed
        *self.mailbox_cache.write().await = None;

        Ok(())
    }

    /// Get the cached mailbox list, or fetch and cache it if stale/missing.
    pub async fn mailbox_list(&self) -> Result<Vec<MailboxListEntry>, String> {
        // Check cache
        {
            let cache = self.mailbox_cache.read().await;
            if let Some((ref list, fetched_at)) = *cache
                && fetched_at.elapsed() < MAILBOX_CACHE_TTL
            {
                return Ok(list.clone());
            }
        }

        // Cache miss or stale — fetch from server
        let list = super::helpers::fetch_mailbox_list_from_server(self).await?;
        *self.mailbox_cache.write().await = Some((list.clone(), std::time::Instant::now()));
        Ok(list)
    }

    /// Invalidate the mailbox cache (e.g. after creating/deleting a mailbox).
    pub async fn invalidate_mailbox_cache(&self) {
        *self.mailbox_cache.write().await = None;
    }

    /// Create a JMAP client from a DB account record.
    ///
    /// Reads credentials from the database and connects using either Basic
    /// or Bearer auth depending on the account's `auth_method`.
    pub async fn from_account(
        db: &DbState,
        account_id: &str,
        encryption_key: &[u8; 32],
    ) -> Result<Self, String> {
        let aid = account_id.to_string();
        let key = *encryption_key;

        let creds = db
            .with_conn(move |conn| read_jmap_credentials(conn, &aid, &key))
            .await?;

        let jmap_credentials = match creds.auth_method.as_str() {
            "oauth2" | "bearer" => {
                let access_token = creds
                    .access_token
                    .ok_or("JMAP OAuth account has no access token")?;
                Credentials::bearer(access_token)
            }
            _ => {
                // Basic auth (default)
                let password = creds.password.ok_or("No password for JMAP account")?;
                Credentials::basic(&creds.login, &password)
            }
        };

        let client = Client::new()
            .credentials(jmap_credentials)
            .connect(&creds.jmap_url)
            .await
            .map_err(|e| format!("JMAP connect failed: {e}"))?;

        let is_oauth = matches!(creds.auth_method.as_str(), "oauth2" | "bearer");

        Ok(Self {
            inner: Arc::new(StdRwLock::new(Arc::new(client))),
            mailbox_cache: Arc::new(RwLock::new(None)),
            db: if is_oauth { Some(db.clone()) } else { None },
            account_id: account_id.to_string(),
            encryption_key: if is_oauth { Some(key) } else { None },
            auth_method: creds.auth_method,
            jmap_url: creds.jmap_url,
        })
    }
}

// ---------------------------------------------------------------------------
// Credential reading
// ---------------------------------------------------------------------------

/// Parsed JMAP credentials from the database.
struct JmapCredentials {
    jmap_url: String,
    login: String,
    password: Option<String>,
    auth_method: String,
    access_token: Option<String>,
}

/// Read JMAP credentials from the database.
///
/// Returns all fields needed to connect with either Basic or Bearer auth.
fn read_jmap_credentials(
    conn: &rusqlite::Connection,
    account_id: &str,
    key: &[u8; 32],
) -> Result<JmapCredentials, String> {
    let mut stmt = conn
        .prepare(
            "SELECT jmap_url, email, imap_password, imap_username, \
             auth_method, access_token \
             FROM accounts WHERE id = ?1",
        )
        .map_err(|e| format!("prepare: {e}"))?;

    let row = stmt
        .query_row([account_id], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .map_err(|e| format!("JMAP account {account_id} not found: {e}"))?;

    let jmap_url = row.0.ok_or("No jmap_url configured for account")?;
    let email = row.1;
    let enc_password = row.2;
    let imap_username = row.3;
    let auth_method = row.4.unwrap_or_else(|| "password".to_string());
    let enc_access_token = row.5;

    // Use imap_username as login if set, otherwise use email
    let login = imap_username
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| email.clone());

    let password = decrypt_if_needed(key, enc_password)?;
    let access_token = decrypt_if_needed(key, enc_access_token)?;

    Ok(JmapCredentials {
        jmap_url,
        login,
        password,
        auth_method,
        access_token,
    })
}

// ---------------------------------------------------------------------------
// JmapState — global JMAP client registry
// ---------------------------------------------------------------------------

/// State holding all JMAP clients and the encryption key.
pub type JmapState = ratatoskr_provider_utils::state::ProviderState<JmapClient>;

/// Create a new `JmapState` with the given encryption key.
pub fn new_jmap_state(encryption_key: [u8; 32]) -> JmapState {
    JmapState::new(encryption_key, "JMAP")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decrypt_if_needed_passes_through_plaintext() {
        let key = [0u8; 32];
        let result = decrypt_if_needed(&key, Some("plain-value".to_string()));
        assert_eq!(result.unwrap(), Some("plain-value".to_string()));
    }

    #[test]
    fn decrypt_if_needed_returns_none_for_none() {
        let key = [0u8; 32];
        let result = decrypt_if_needed(&key, None);
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn decrypt_if_needed_returns_err_for_bad_encrypted() {
        let key = [7u8; 32];
        let encrypted_like = Some("AAAAAAAAAAAAAAAA:AAAA".to_string());
        let err = decrypt_if_needed(&key, encrypted_like).expect_err("expected decrypt failure");
        assert!(err.contains("decrypt credential"));
    }

    #[test]
    fn oauth_token_endpoint_uses_stored_url() {
        let url = oauth_token_endpoint("unknown", Some("https://example.com/token"));
        assert_eq!(url.unwrap(), "https://example.com/token");
    }

    #[test]
    fn oauth_token_endpoint_resolves_microsoft() {
        let url = oauth_token_endpoint("microsoft", None);
        assert_eq!(
            url.unwrap(),
            "https://login.microsoftonline.com/common/oauth2/v2.0/token"
        );
    }

    #[test]
    fn oauth_token_endpoint_resolves_fastmail() {
        let url = oauth_token_endpoint("fastmail", None);
        assert_eq!(url.unwrap(), "https://api.fastmail.com/oauth/token");
    }

    #[test]
    fn oauth_token_endpoint_errors_on_unknown() {
        let err = oauth_token_endpoint("unknown-provider", None).expect_err("expected error");
        assert!(err.contains("Unsupported OAuth provider"));
    }
}
