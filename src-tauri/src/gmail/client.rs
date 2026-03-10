use std::collections::HashMap;
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::sync::{Mutex, RwLock};

use crate::db::DbState;
use crate::provider::crypto;
use crate::provider::http::{self, RetryConfig};
use crate::provider::token::{self, TokenState};

const GMAIL_API_BASE: &str = "https://www.googleapis.com/gmail/v1/users/me";
const RETRY_CONFIG: RetryConfig = RetryConfig {
    max_attempts: 3,
    initial_backoff_ms: 1000,
};

/// Per-account Gmail API client.
///
/// Internally reference-counted — cloning is cheap (Arc increment).
/// All API methods take `&self`, supporting concurrent use.
#[derive(Clone)]
pub struct GmailClient {
    inner: Arc<ClientInner>,
}

struct ClientInner {
    http: reqwest::Client,
    account_id: String,
    token: RwLock<TokenState>,
    refresh_lock: Mutex<()>,
    client_id: String,
    client_secret: Option<String>,
    encryption_key: [u8; 32],
}

/// Tauri-managed state holding all Gmail clients and the encryption key.
pub struct GmailState {
    clients: RwLock<HashMap<String, GmailClient>>,
    encryption_key: [u8; 32],
}

impl GmailState {
    pub fn new(encryption_key: [u8; 32]) -> Self {
        Self {
            clients: RwLock::new(HashMap::new()),
            encryption_key,
        }
    }

    /// Get a client for the given account, or return an error if not initialized.
    pub async fn get(&self, account_id: &str) -> Result<GmailClient, String> {
        self.clients
            .read()
            .await
            .get(account_id)
            .cloned()
            .ok_or_else(|| format!("Gmail client not initialized for account {account_id}"))
    }

    /// Insert (or replace) a client for the given account.
    pub async fn insert(&self, account_id: String, client: GmailClient) {
        self.clients.write().await.insert(account_id, client);
    }

    /// Remove the client for the given account.
    pub async fn remove(&self, account_id: &str) {
        self.clients.write().await.remove(account_id);
    }

    pub fn encryption_key(&self) -> &[u8; 32] {
        &self.encryption_key
    }
}

impl GmailClient {
    /// Create a Gmail client by reading account credentials from the database.
    pub async fn from_account(
        db: &DbState,
        account_id: &str,
        encryption_key: [u8; 32],
    ) -> Result<Self, String> {
        let aid = account_id.to_string();
        let key = encryption_key;

        let (access_token, refresh_token, expires_at, client_id, client_secret) = db
            .with_conn(move |conn| {
                read_account_tokens(conn, &aid, &key)
            })
            .await?;

        let token_state = TokenState {
            access_token,
            refresh_token,
            expires_at,
        };

        Ok(Self {
            inner: Arc::new(ClientInner {
                http: reqwest::Client::new(),
                account_id: account_id.to_string(),
                token: RwLock::new(token_state),
                refresh_lock: Mutex::new(()),
                client_id,
                client_secret,
                encryption_key,
            }),
        })
    }

    #[allow(dead_code)] // Used by Phase 2 sync code
    pub fn account_id(&self) -> &str {
        &self.inner.account_id
    }

    /// Make an authenticated GET request to the Gmail API.
    pub async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        db: &DbState,
    ) -> Result<T, String> {
        let url = format!("{GMAIL_API_BASE}{path}");
        self.request::<T, ()>(&url, "GET", None, db).await
    }

    /// Make an authenticated POST request to the Gmail API.
    pub async fn post<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
        db: &DbState,
    ) -> Result<T, String> {
        let url = format!("{GMAIL_API_BASE}{path}");
        self.request(&url, "POST", Some(body), db).await
    }

    /// Make an authenticated PUT request to the Gmail API.
    pub async fn put<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
        db: &DbState,
    ) -> Result<T, String> {
        let url = format!("{GMAIL_API_BASE}{path}");
        self.request(&url, "PUT", Some(body), db).await
    }

    /// Make an authenticated PATCH request to the Gmail API.
    pub async fn patch<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
        db: &DbState,
    ) -> Result<T, String> {
        let url = format!("{GMAIL_API_BASE}{path}");
        self.request(&url, "PATCH", Some(body), db).await
    }

    /// Make an authenticated DELETE request to the Gmail API.
    /// Returns `()` — no response body expected.
    pub async fn delete(&self, path: &str, db: &DbState) -> Result<(), String> {
        let url = format!("{GMAIL_API_BASE}{path}");
        let access_token = self.ensure_valid_token(db).await?;
        let response = self
            .execute_with_retry(&url, "DELETE", None::<&()>, &access_token)
            .await?;

        if response.status().as_u16() == 401 {
            let new_token = self.force_refresh(db).await?;
            let retry = self
                .execute_with_retry(&url, "DELETE", None::<&()>, &new_token)
                .await?;
            check_response_status(retry).await?;
        } else {
            check_response_status(response).await?;
        }
        Ok(())
    }
}

// ── Private implementation ──────────────────────────────────

impl GmailClient {
    /// Core request method with token refresh and 429 retry.
    async fn request<T: DeserializeOwned, B: Serialize>(
        &self,
        url: &str,
        method: &str,
        body: Option<&B>,
        db: &DbState,
    ) -> Result<T, String> {
        let access_token = self.ensure_valid_token(db).await?;
        let response = self.execute_with_retry(url, method, body, &access_token).await?;

        if response.status().as_u16() == 401 {
            let new_token = self.force_refresh(db).await?;
            let retry = self.execute_with_retry(url, method, body, &new_token).await?;
            return parse_json_response(retry).await;
        }

        parse_json_response(response).await
    }

    /// Execute an HTTP request with retry on 429 (rate limit).
    async fn execute_with_retry<B: Serialize>(
        &self,
        url: &str,
        method: &str,
        body: Option<&B>,
        access_token: &str,
    ) -> Result<reqwest::Response, String> {
        let mut last_response = None;

        for attempt in 0..RETRY_CONFIG.max_attempts {
            let response = self.execute_once(url, method, body, access_token).await?;

            if response.status().as_u16() != 429 {
                return Ok(response);
            }

            last_response = Some(response);
            if attempt == RETRY_CONFIG.max_attempts - 1 {
                break;
            }

            let delay_ms = http::compute_retry_delay(
                last_response.as_ref(),
                attempt,
                &RETRY_CONFIG,
            );
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }

        last_response.ok_or_else(|| "No response received".to_string())
    }

    /// Execute a single HTTP request.
    async fn execute_once<B: Serialize>(
        &self,
        url: &str,
        method: &str,
        body: Option<&B>,
        access_token: &str,
    ) -> Result<reqwest::Response, String> {
        let mut builder = match method {
            "GET" => self.inner.http.get(url),
            "POST" => self.inner.http.post(url),
            "PUT" => self.inner.http.put(url),
            "PATCH" => self.inner.http.patch(url),
            "DELETE" => self.inner.http.delete(url),
            _ => return Err(format!("Unsupported HTTP method: {method}")),
        };

        builder = builder
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json");

        if let Some(b) = body {
            builder = builder.json(b);
        }

        builder
            .send()
            .await
            .map_err(|e| format!("Gmail API request failed: {e}"))
    }

    /// Get a valid access token, refreshing if needed.
    async fn ensure_valid_token(&self, db: &DbState) -> Result<String, String> {
        // Fast path: read lock, return if valid
        {
            let state = self.inner.token.read().await;
            if !state.needs_refresh() {
                return Ok(state.access_token.clone());
            }
        }

        self.do_refresh(db).await
    }

    /// Force a token refresh (e.g. after a 401 response).
    ///
    /// Sets the expiry to the past so the double-check inside `do_refresh`
    /// won't short-circuit with a stale (server-revoked) token.
    async fn force_refresh(&self, db: &DbState) -> Result<String, String> {
        {
            let mut state = self.inner.token.write().await;
            state.expires_at = 0;
        }
        self.do_refresh(db).await
    }

    /// Perform the actual token refresh, with mutex to coalesce concurrent refreshes.
    async fn do_refresh(&self, db: &DbState) -> Result<String, String> {
        // Acquire refresh lock — only one refresh at a time
        let _guard = self.inner.refresh_lock.lock().await;

        // Double-check: another task might have already refreshed
        {
            let state = self.inner.token.read().await;
            if !state.needs_refresh() {
                return Ok(state.access_token.clone());
            }
        }

        // Read current refresh token
        let refresh_token = self.inner.token.read().await.refresh_token.clone();

        // Call Google's token endpoint
        let result = token::refresh_google_token(
            &self.inner.http,
            &refresh_token,
            &self.inner.client_id,
            self.inner.client_secret.as_deref(),
        )
        .await?;

        // Update in-memory state
        let new_access_token = result.access_token.clone();
        {
            let mut state = self.inner.token.write().await;
            state.access_token = result.access_token;
            state.expires_at = result.expires_at;
        }

        // Persist encrypted token to DB
        persist_refreshed_token(
            db,
            &self.inner.account_id,
            &new_access_token,
            result.expires_at,
            &self.inner.encryption_key,
        )
        .await?;

        Ok(new_access_token)
    }
}

/// Read and decrypt account tokens from the database.
fn read_account_tokens(
    conn: &rusqlite::Connection,
    account_id: &str,
    key: &[u8; 32],
) -> Result<(String, String, i64, String, Option<String>), String> {
    let mut stmt = conn
        .prepare(
            "SELECT access_token, refresh_token, token_expires_at,
                    oauth_client_id
             FROM accounts WHERE id = ?1",
        )
        .map_err(|e| format!("prepare: {e}"))?;

    let row = stmt
        .query_row([account_id], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(|e| format!("Account {account_id} not found: {e}"))?;

    let enc_access = row.0.ok_or("No access_token for account")?;
    let enc_refresh = row.1.ok_or("No refresh_token for account")?;
    let expires_at = row.2.unwrap_or(0);

    let access_token = decrypt_or_raw(key, &enc_access);
    let refresh_token = decrypt_or_raw(key, &enc_refresh);

    // Client ID: from account's oauth_client_id, or fall back to global setting
    let client_id = if let Some(cid) = row.3 {
        if cid.is_empty() {
            read_global_client_id(conn, key)?
        } else {
            cid
        }
    } else {
        read_global_client_id(conn, key)?
    };

    // Client secret: read from global settings (optional)
    let client_secret = read_global_client_secret(conn, key);

    Ok((access_token, refresh_token, expires_at, client_id, client_secret))
}

/// Try to decrypt, falling back to raw value for pre-encryption data.
fn decrypt_or_raw(key: &[u8; 32], value: &str) -> String {
    if crypto::is_encrypted(value) {
        crypto::decrypt_value(key, value).unwrap_or_else(|_| value.to_string())
    } else {
        value.to_string()
    }
}

fn read_global_client_id(conn: &rusqlite::Connection, key: &[u8; 32]) -> Result<String, String> {
    let raw: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'google_client_id'",
            [],
            |row| row.get(0),
        )
        .map_err(|_| "No google_client_id configured in settings".to_string())?;
    Ok(decrypt_or_raw(key, &raw))
}

fn read_global_client_secret(conn: &rusqlite::Connection, key: &[u8; 32]) -> Option<String> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'google_client_secret'",
            [],
            |row| row.get(0),
        )
        .ok();
    raw.map(|r| decrypt_or_raw(key, &r))
}

/// Persist a refreshed access token (encrypted) to the database.
async fn persist_refreshed_token(
    db: &DbState,
    account_id: &str,
    access_token: &str,
    expires_at: i64,
    key: &[u8; 32],
) -> Result<(), String> {
    let encrypted = crypto::encrypt_value(key, access_token)?;
    let aid = account_id.to_string();

    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE accounts SET access_token = ?1, token_expires_at = ?2, updated_at = unixepoch() WHERE id = ?3",
            rusqlite::params![encrypted, expires_at, aid],
        )
        .map_err(|e| format!("Failed to persist refreshed token: {e}"))?;
        Ok(())
    })
    .await
}

/// Check HTTP response status, returning error details on failure.
async fn check_response_status(response: reqwest::Response) -> Result<(), String> {
    if response.status().is_success() || response.status().as_u16() == 204 {
        return Ok(());
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    Err(format!("Gmail API error: {status} {body}"))
}

/// Parse a JSON response, handling 204 No Content.
async fn parse_json_response<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, String> {
    let status = response.status();

    if !status.is_success() && status.as_u16() != 204 {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Gmail API error: {status} {body}"));
    }

    if status.as_u16() == 204 {
        // For 204 No Content, try to create a default value
        return serde_json::from_str("null")
            .map_err(|e| format!("Cannot deserialize null for 204 response: {e}"));
    }

    response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Gmail API response: {e}"))
}
