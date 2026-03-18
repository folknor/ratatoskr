use serde::Deserialize;

const GOOGLE_TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";

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
/// optional — PKCE-only flows omit it.
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
