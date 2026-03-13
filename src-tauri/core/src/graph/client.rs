use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::{Mutex, RwLock, Semaphore};

use crate::db::DbState;
use crate::provider::crypto;
use crate::provider::http::RetryConfig;
use crate::provider::token::{self, TokenState};

use super::folder_mapper::FolderMap;

const GRAPH_API_BASE: &str = "https://graph.microsoft.com/v1.0";
const MS_TOKEN_ENDPOINT: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";

/// Graph allows max 4 concurrent requests per app per mailbox.
/// Reserve 1 for user-initiated actions during sync.
const CONCURRENCY_LIMIT: usize = 3;

const RETRY_CONFIG: RetryConfig = RetryConfig {
    max_attempts: 3,
    initial_backoff_ms: 1000,
};

/// Per-account Microsoft Graph API client.
///
/// Internally reference-counted — cloning is cheap (Arc increment).
/// All API methods take `&self`, supporting concurrent use.
#[derive(Clone)]
pub struct GraphClient {
    inner: Arc<ClientInner>,
}

struct ClientInner {
    http: reqwest::Client,
    account_id: String,
    token: RwLock<TokenState>,
    refresh_lock: Mutex<()>,
    client_id: String,
    encryption_key: [u8; 32],
    semaphore: Arc<Semaphore>,
    folder_map: RwLock<Option<FolderMap>>,
    folder_map_last_sync: RwLock<Option<std::time::Instant>>,
    sync_cycle_counter: AtomicU32,
}

/// Tauri-managed state holding all Graph clients and the encryption key.
#[derive(Clone)]
pub struct GraphState {
    clients: Arc<RwLock<HashMap<String, GraphClient>>>,
    encryption_key: [u8; 32],
}

impl GraphState {
    pub fn new(encryption_key: [u8; 32]) -> Self {
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
            encryption_key,
        }
    }

    pub async fn get(&self, account_id: &str) -> Result<GraphClient, String> {
        self.clients
            .read()
            .await
            .get(account_id)
            .cloned()
            .ok_or_else(|| format!("Graph client not initialized for account {account_id}"))
    }

    pub async fn insert(&self, account_id: String, client: GraphClient) {
        self.clients.write().await.insert(account_id, client);
    }

    pub async fn remove(&self, account_id: &str) {
        self.clients.write().await.remove(account_id);
    }

    pub fn encryption_key(&self) -> &[u8; 32] {
        &self.encryption_key
    }
}

impl GraphClient {
    /// Create a Graph client by reading account credentials from the database.
    pub async fn from_account(
        db: &DbState,
        account_id: &str,
        encryption_key: [u8; 32],
    ) -> Result<Self, String> {
        let aid = account_id.to_string();
        let key = encryption_key;

        let (access_token, refresh_token, expires_at, client_id) = db
            .with_conn(move |conn| read_account_tokens(conn, &aid, &key))
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
                encryption_key,
                semaphore: Arc::new(Semaphore::new(CONCURRENCY_LIMIT)),
                folder_map: RwLock::new(None),
                folder_map_last_sync: RwLock::new(None),
                sync_cycle_counter: AtomicU32::new(0),
            }),
        })
    }

    /// Get or rebuild the cached folder map.
    pub async fn folder_map(&self) -> Option<FolderMap> {
        self.inner.folder_map.read().await.clone()
    }

    /// Store a new folder map.
    pub async fn set_folder_map(&self, map: FolderMap) {
        *self.inner.folder_map.write().await = Some(map);
    }

    /// How long ago the folder map was last synced from the server.
    pub async fn folder_map_age(&self) -> Option<std::time::Duration> {
        self.inner
            .folder_map_last_sync
            .read()
            .await
            .map(|t| t.elapsed())
    }

    /// Atomically increment the sync cycle counter and return the new value.
    pub fn increment_sync_cycle(&self) -> u32 {
        self.inner
            .sync_cycle_counter
            .fetch_add(1, Ordering::Relaxed)
            + 1
    }

    /// Record that the folder map was just synced from the server.
    pub async fn set_folder_map_synced(&self) {
        *self.inner.folder_map_last_sync.write().await = Some(std::time::Instant::now());
    }

    // ── HTTP methods ────────────────────────────────────────

    /// Authenticated GET against the Graph API.
    pub async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        db: &DbState,
    ) -> Result<T, String> {
        let url = format!("{GRAPH_API_BASE}{path}");
        self.request::<T, ()>(&url, "GET", None, db).await
    }

    /// Authenticated GET returning raw bytes (for attachment `/$value`).
    pub async fn get_bytes(&self, path: &str, db: &DbState) -> Result<Vec<u8>, String> {
        let url = format!("{GRAPH_API_BASE}{path}");
        self.request_bytes(&url, db).await
    }

    /// Authenticated POST against the Graph API.
    pub async fn post<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
        db: &DbState,
    ) -> Result<T, String> {
        let url = format!("{GRAPH_API_BASE}{path}");
        self.request(&url, "POST", Some(body), db).await
    }

    /// Authenticated POST with no response body expected.
    pub async fn post_no_content<B: Serialize>(
        &self,
        path: &str,
        body: Option<&B>,
        db: &DbState,
    ) -> Result<(), String> {
        let url = format!("{GRAPH_API_BASE}{path}");
        let access_token = self.ensure_valid_token(db).await?;
        let _permit = self
            .inner
            .semaphore
            .acquire()
            .await
            .map_err(|e| format!("Semaphore closed: {e}"))?;
        let response = self
            .execute_with_retry(&url, "POST", body, &access_token)
            .await?;

        if response.status().as_u16() == 401 {
            let new_token = self.force_refresh(db).await?;
            let retry = self
                .execute_with_retry(&url, "POST", body, &new_token)
                .await?;
            return check_response_status(retry).await;
        }
        check_response_status(response).await
    }

    /// Authenticated PATCH against the Graph API.
    pub async fn patch<B: Serialize>(
        &self,
        path: &str,
        body: &B,
        db: &DbState,
    ) -> Result<(), String> {
        let url = format!("{GRAPH_API_BASE}{path}");
        let access_token = self.ensure_valid_token(db).await?;
        let _permit = self
            .inner
            .semaphore
            .acquire()
            .await
            .map_err(|e| format!("Semaphore closed: {e}"))?;
        let response = self
            .execute_with_retry(&url, "PATCH", Some(body), &access_token)
            .await?;

        if response.status().as_u16() == 401 {
            let new_token = self.force_refresh(db).await?;
            let retry = self
                .execute_with_retry(&url, "PATCH", Some(body), &new_token)
                .await?;
            return check_response_status(retry).await;
        }
        check_response_status(response).await
    }

    /// Authenticated DELETE against the Graph API.
    pub async fn delete(&self, path: &str, db: &DbState) -> Result<(), String> {
        let url = format!("{GRAPH_API_BASE}{path}");
        let access_token = self.ensure_valid_token(db).await?;
        let _permit = self
            .inner
            .semaphore
            .acquire()
            .await
            .map_err(|e| format!("Semaphore closed: {e}"))?;
        let response = self
            .execute_with_retry(&url, "DELETE", None::<&()>, &access_token)
            .await?;

        if response.status().as_u16() == 401 {
            let new_token = self.force_refresh(db).await?;
            let retry = self
                .execute_with_retry(&url, "DELETE", None::<&()>, &new_token)
                .await?;
            return check_response_status(retry).await;
        }
        check_response_status(response).await
    }

    /// Execute a batch of up to 20 requests in a single `POST /$batch` call.
    ///
    /// Returns per-request results. Callers should check each `BatchResponseItem.status`
    /// for individual failures. Graph allows max 20 requests per batch.
    pub async fn post_batch(
        &self,
        batch: &super::types::BatchRequest,
        db: &DbState,
    ) -> Result<super::types::BatchResponse, String> {
        self.post("/$batch", batch, db).await
    }

    /// GET a URL by full absolute URL (for OData pagination links).
    pub async fn get_absolute<T: DeserializeOwned>(
        &self,
        url: &str,
        db: &DbState,
    ) -> Result<T, String> {
        self.request::<T, ()>(url, "GET", None, db).await
    }
}

// ── Private implementation ──────────────────────────────────

impl GraphClient {
    /// Core request method with semaphore, token refresh, and 429 retry.
    async fn request<T: DeserializeOwned, B: Serialize>(
        &self,
        url: &str,
        method: &str,
        body: Option<&B>,
        db: &DbState,
    ) -> Result<T, String> {
        let access_token = self.ensure_valid_token(db).await?;
        let _permit = self
            .inner
            .semaphore
            .acquire()
            .await
            .map_err(|e| format!("Semaphore closed: {e}"))?;
        let response = self
            .execute_with_retry(url, method, body, &access_token)
            .await?;

        if response.status().as_u16() == 401 {
            let new_token = self.force_refresh(db).await?;
            let retry = self
                .execute_with_retry(url, method, body, &new_token)
                .await?;
            return parse_json_response(retry).await;
        }

        parse_json_response(response).await
    }

    /// Request returning raw bytes (for `/$value` endpoints).
    async fn request_bytes(&self, url: &str, db: &DbState) -> Result<Vec<u8>, String> {
        let access_token = self.ensure_valid_token(db).await?;
        let _permit = self
            .inner
            .semaphore
            .acquire()
            .await
            .map_err(|e| format!("Semaphore closed: {e}"))?;

        let response = self
            .execute_with_retry(url, "GET", None::<&()>, &access_token)
            .await?;

        if response.status().as_u16() == 401 {
            let new_token = self.force_refresh(db).await?;
            let retry = self
                .execute_with_retry(url, "GET", None::<&()>, &new_token)
                .await?;
            return parse_bytes_response(retry).await;
        }

        parse_bytes_response(response).await
    }

    /// Execute with retry on 429 (rate limit).
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

            let delay_ms = crate::provider::http::compute_retry_delay(
                last_response.as_ref(),
                attempt,
                &RETRY_CONFIG,
            );
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }

        last_response.ok_or_else(|| "No response received".to_string())
    }

    /// Execute a single HTTP request.
    #[allow(clippy::cognitive_complexity)]
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
            .map_err(|e| format!("Graph API request failed: {e}"))
    }

    /// Get a valid access token, refreshing if needed.
    async fn ensure_valid_token(&self, db: &DbState) -> Result<String, String> {
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

    /// Perform the actual token refresh, coalesced via mutex.
    async fn do_refresh(&self, db: &DbState) -> Result<String, String> {
        let _guard = self.inner.refresh_lock.lock().await;

        // Double-check: another task might have already refreshed
        {
            let state = self.inner.token.read().await;
            if !state.needs_refresh() {
                return Ok(state.access_token.clone());
            }
        }

        let refresh_token = self.inner.token.read().await.refresh_token.clone();

        let result = token::refresh_oauth_token(
            &self.inner.http,
            MS_TOKEN_ENDPOINT,
            &refresh_token,
            &self.inner.client_id,
            None, // PKCE flow — no client secret
        )
        .await?;

        let new_access_token = result.access_token.clone();
        {
            let mut state = self.inner.token.write().await;
            state.access_token = result.access_token;
            state.expires_at = result.expires_at;
        }

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
) -> Result<(String, String, i64, String), String> {
    let mut stmt = conn
        .prepare(
            "SELECT access_token, refresh_token, token_expires_at, \
                    oauth_client_id \
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

    let enc_access = row.0.ok_or("No access_token for Graph account")?;
    let enc_refresh = row.1.ok_or("No refresh_token for Graph account")?;
    let expires_at = row.2.unwrap_or(0);

    let access_token = decrypt_or_raw(key, &enc_access);
    let refresh_token = decrypt_or_raw(key, &enc_refresh);

    let client_id = row
        .3
        .filter(|s| !s.is_empty())
        .map(|s| decrypt_or_raw(key, &s))
        .ok_or_else(|| "Account missing OAuth credentials — reauthorize to fix".to_string())?;

    Ok((access_token, refresh_token, expires_at, client_id))
}

fn decrypt_or_raw(key: &[u8; 32], value: &str) -> String {
    if crypto::is_encrypted(value) {
        crypto::decrypt_value(key, value).unwrap_or_else(|_| value.to_string())
    } else {
        value.to_string()
    }
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
            "UPDATE accounts SET access_token = ?1, token_expires_at = ?2, \
             updated_at = unixepoch() WHERE id = ?3",
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
    Err(format!("Graph API error: {status} {body}"))
}

/// Parse a JSON response, handling 204 No Content.
async fn parse_json_response<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, String> {
    let status = response.status();

    if !status.is_success() && status.as_u16() != 204 {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Graph API error: {status} {body}"));
    }

    if status.as_u16() == 204 {
        return serde_json::from_str("null")
            .map_err(|e| format!("Cannot deserialize null for 204 response: {e}"));
    }

    response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Graph API response: {e}"))
}

/// Parse a bytes response.
async fn parse_bytes_response(response: reqwest::Response) -> Result<Vec<u8>, String> {
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Graph API error: {status} {body}"));
    }
    response
        .bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("Failed to read Graph API response bytes: {e}"))
}
