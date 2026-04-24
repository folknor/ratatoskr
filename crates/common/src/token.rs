use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use serde::Deserialize;

const GOOGLE_TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const MICROSOFT_TOKEN_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";
const FASTMAIL_TOKEN_URL: &str = "https://api.fastmail.com/oauth/token";
const YAHOO_TOKEN_URL: &str = "https://api.login.yahoo.com/oauth2/get_token";

/// In-memory token state for an OAuth2 account.
pub struct TokenState {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
}

impl TokenState {
    /// Returns `true` if the token expires within the next 5 minutes.
    pub fn needs_refresh(&self) -> bool {
        let now = chrono::Utc::now().timestamp();
        self.expires_at - now < 300
    }
}

/// Result from a successful token refresh.
pub struct TokenRefreshResult {
    pub access_token: String,
    pub expires_at: i64,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: i64,
}

/// Refresh an OAuth2 access token against any RFC 6749 token endpoint.
///
/// Works with any provider that accepts a standard `grant_type=refresh_token`
/// form POST (Google, Microsoft, Fastmail, etc.). The `client_secret` is
/// optional - PKCE-only flows omit it.
pub async fn refresh_oauth_token(
    http: &reqwest::Client,
    token_endpoint: &str,
    refresh_token: &str,
    client_id: &str,
    client_secret: Option<&str>,
) -> Result<TokenRefreshResult, String> {
    let mut params = vec![
        ("refresh_token", refresh_token),
        ("client_id", client_id),
        ("grant_type", "refresh_token"),
    ];
    if let Some(secret) = client_secret
        && !secret.is_empty()
    {
        params.push(("client_secret", secret));
    }

    let response = http
        .post(token_endpoint)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Token refresh request failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Token refresh failed ({status}): {body}"));
    }

    let resp: TokenResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse token response: {e}"))?;

    let now = chrono::Utc::now().timestamp();
    Ok(TokenRefreshResult {
        access_token: resp.access_token,
        expires_at: now + resp.expires_in,
    })
}

/// Convenience wrapper: refresh via Google's OAuth2 token endpoint.
pub async fn refresh_google_token(
    http: &reqwest::Client,
    refresh_token: &str,
    client_id: &str,
    client_secret: Option<&str>,
) -> Result<TokenRefreshResult, String> {
    refresh_oauth_token(
        http,
        GOOGLE_TOKEN_ENDPOINT,
        refresh_token,
        client_id,
        client_secret,
    )
    .await
}

// ---------------------------------------------------------------------------
// Per-account refresh lock registry
// ---------------------------------------------------------------------------

/// Global per-account refresh lock registry.
///
/// Prevents concurrent token refreshes for the same account across any
/// provider (JMAP, IMAP, etc.). Each account ID maps to a shared async mutex.
static REFRESH_LOCKS: OnceLock<StdMutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    OnceLock::new();

/// Get (or create) the per-account refresh lock for the given account ID.
///
/// Used by providers that read token state from the DB and need to serialize
/// refresh attempts across concurrent tasks for the same account.
pub fn get_refresh_lock(account_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    let map = REFRESH_LOCKS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut guard = map.lock().expect("refresh lock map poisoned");
    Arc::clone(
        guard
            .entry(account_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
    )
}

// ---------------------------------------------------------------------------
// OAuth token endpoint resolution
// ---------------------------------------------------------------------------

/// Resolve the OAuth2 token endpoint URL for a provider.
///
/// If a `stored_url` is present and non-empty, it takes precedence. Otherwise,
/// the provider ID is matched against known providers (Microsoft, Google,
/// Fastmail, Yahoo). Returns an error for unknown providers without a stored URL.
pub fn oauth_token_endpoint(provider_id: &str, stored_url: Option<&str>) -> Result<String, String> {
    if let Some(url) = stored_url.filter(|u| !u.is_empty()) {
        return Ok(url.to_string());
    }
    match provider_id {
        "microsoft" | "microsoft_graph" => Ok(MICROSOFT_TOKEN_URL.to_string()),
        "google" | "gmail" => Ok(GOOGLE_TOKEN_ENDPOINT.to_string()),
        "fastmail" | "jmap" => Ok(FASTMAIL_TOKEN_URL.to_string()),
        "yahoo" => Ok(YAHOO_TOKEN_URL.to_string()),
        other => Err(format!(
            "Unsupported OAuth provider: {other}. \
             Set oauth_token_url in the account record."
        )),
    }
}
