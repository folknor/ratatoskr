use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;
use std::time::Duration;
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const OAUTH_CALLBACK_PORT: u16 = 17248;
const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo";
const GOOGLE_SCOPES: &str = concat!(
    "https://www.googleapis.com/auth/gmail.readonly ",
    "https://www.googleapis.com/auth/gmail.modify ",
    "https://www.googleapis.com/auth/gmail.send ",
    "https://www.googleapis.com/auth/gmail.labels ",
    "https://www.googleapis.com/auth/userinfo.email ",
    "https://www.googleapis.com/auth/userinfo.profile ",
    "https://www.googleapis.com/auth/calendar.readonly ",
    "https://www.googleapis.com/auth/calendar.events"
);
const MICROSOFT_GRAPH_AUTH_URL: &str =
    "https://login.microsoftonline.com/common/oauth2/v2.0/authorize";
const MICROSOFT_GRAPH_TOKEN_URL: &str =
    "https://login.microsoftonline.com/common/oauth2/v2.0/token";
const MICROSOFT_GRAPH_SCOPES: [&str; 7] = [
    "Mail.ReadWrite",
    "Mail.Send",
    "MailboxSettings.ReadWrite",
    "offline_access",
    "openid",
    "profile",
    "User.Read",
];

type OAuthFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

fn shared_http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

#[derive(Serialize)]
pub struct OAuthResult {
    pub code: String,
    pub state: String,
    pub actual_port: u16,
}

pub struct OAuthAuthorizationRequest {
    pub auth_url: String,
    pub client_id: String,
    pub scopes: Vec<String>,
    pub use_pkce: bool,
    pub extra_auth_params: Vec<(String, String)>,
}

pub struct OAuthAuthorizationFlow {
    pub code: String,
    pub redirect_uri: String,
    pub code_verifier: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthProviderAuthorizationRequest {
    pub provider_id: String,
    pub auth_url: String,
    pub token_url: String,
    pub scopes: Vec<String>,
    pub user_info_url: Option<String>,
    pub use_pkce: bool,
    pub client_id: String,
    pub client_secret: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OAuthUserInfo {
    pub email: String,
    pub name: String,
    pub picture: Option<String>,
}

pub struct OAuthAuthorizationBundle {
    pub tokens: TokenExchangeResult,
    pub user_info: OAuthUserInfo,
}

pub struct PendingOAuthAuthorization {
    pub tokens: TokenExchangeResult,
}

pub type PendingOAuthAuthorizations =
    std::sync::Mutex<std::collections::HashMap<String, PendingOAuthAuthorization>>;

pub trait OAuthIdentityProvider {
    fn authorization_request(&self) -> OAuthAuthorizationRequest;
    fn token_request(&self) -> OAuthTokenRequest;
    fn fetch_user_info<'a>(
        &'a self,
        tokens: &'a TokenExchangeResult,
    ) -> OAuthFuture<'a, Result<OAuthUserInfo, String>>;
}

pub struct OAuthTokenRequest {
    pub token_url: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub scope: Option<String>,
}

pub struct GoogleOAuthProvider {
    client_id: String,
    client_secret: String,
}

impl GoogleOAuthProvider {
    pub fn new(client_id: String, client_secret: String) -> Self {
        Self {
            client_id,
            client_secret,
        }
    }
}

pub struct GenericOAuthProvider {
    provider_id: String,
    auth_url: String,
    token_url: String,
    scopes: Vec<String>,
    user_info_url: Option<String>,
    use_pkce: bool,
    client_id: String,
    client_secret: Option<String>,
}

impl GenericOAuthProvider {
    pub fn from_request(request: OAuthProviderAuthorizationRequest) -> Self {
        Self {
            provider_id: request.provider_id,
            auth_url: request.auth_url,
            token_url: request.token_url,
            scopes: request.scopes,
            user_info_url: request.user_info_url,
            use_pkce: request.use_pkce,
            client_id: request.client_id,
            client_secret: request.client_secret,
        }
    }

    pub fn microsoft_graph(client_id: String) -> Self {
        Self {
            provider_id: "microsoft_graph".to_string(),
            auth_url: MICROSOFT_GRAPH_AUTH_URL.to_string(),
            token_url: MICROSOFT_GRAPH_TOKEN_URL.to_string(),
            scopes: MICROSOFT_GRAPH_SCOPES
                .iter()
                .map(|scope| (*scope).to_string())
                .collect(),
            user_info_url: None,
            use_pkce: true,
            client_id,
            client_secret: None,
        }
    }

    fn is_microsoft(&self) -> bool {
        self.provider_id == "microsoft" || self.provider_id == "microsoft_graph"
    }
}

pub async fn authorize_with_provider<P: OAuthIdentityProvider>(
    app: &AppHandle,
    provider: &P,
) -> Result<OAuthAuthorizationBundle, String> {
    let auth = run_oauth_authorization_flow(app, provider.authorization_request()).await?;
    let token_request = provider.token_request();
    let tokens = oauth_exchange_token(
        token_request.token_url,
        auth.code,
        token_request.client_id,
        auth.redirect_uri,
        auth.code_verifier,
        token_request.client_secret,
        token_request.scope,
    )
    .await?;
    let user_info = provider.fetch_user_info(&tokens).await?;

    Ok(OAuthAuthorizationBundle { tokens, user_info })
}

pub async fn run_oauth_authorization_flow(
    app: &AppHandle,
    request: OAuthAuthorizationRequest,
) -> Result<OAuthAuthorizationFlow, String> {
    let code_verifier = if request.use_pkce {
        Some(random_base64url(32)?)
    } else {
        None
    };
    let code_challenge = code_verifier
        .as_deref()
        .map(|verifier| sha256_base64url(verifier.as_bytes()));
    let state = random_base64url(32)?;

    let (listener, actual_port) = bind_oauth_listener(OAUTH_CALLBACK_PORT).await?;
    let redirect_uri = format!("http://127.0.0.1:{actual_port}");

    let mut params = vec![
        ("client_id".to_string(), request.client_id),
        ("redirect_uri".to_string(), redirect_uri.clone()),
        ("response_type".to_string(), "code".to_string()),
        ("scope".to_string(), request.scopes.join(" ")),
        ("state".to_string(), state.clone()),
    ];
    if let Some(challenge) = code_challenge {
        params.push(("code_challenge".to_string(), challenge));
        params.push(("code_challenge_method".to_string(), "S256".to_string()));
    }
    params.extend(request.extra_auth_params);

    let auth_url = format!(
        "{}?{}",
        request.auth_url,
        params
            .into_iter()
            .map(|(key, value)| format!(
                "{}={}",
                urlencoding::encode(&key),
                urlencoding::encode(&value)
            ))
            .collect::<Vec<_>>()
            .join("&")
    );

    app.opener()
        .open_url(auth_url, None::<&str>)
        .map_err(|e| format!("Failed to open browser for OAuth: {e}"))?;
    let result = await_oauth_callback(listener, &state).await?;

    Ok(OAuthAuthorizationFlow {
        code: result.code,
        redirect_uri,
        code_verifier,
    })
}

impl OAuthIdentityProvider for GoogleOAuthProvider {
    fn authorization_request(&self) -> OAuthAuthorizationRequest {
        OAuthAuthorizationRequest {
            auth_url: GOOGLE_AUTH_URL.to_string(),
            client_id: self.client_id.clone(),
            scopes: vec![GOOGLE_SCOPES.to_string()],
            use_pkce: true,
            extra_auth_params: vec![
                ("access_type".to_string(), "offline".to_string()),
                ("prompt".to_string(), "consent".to_string()),
            ],
        }
    }

    fn token_request(&self) -> OAuthTokenRequest {
        OAuthTokenRequest {
            token_url: GOOGLE_TOKEN_URL.to_string(),
            client_id: self.client_id.clone(),
            client_secret: Some(self.client_secret.clone()),
            scope: None,
        }
    }

    fn fetch_user_info<'a>(
        &'a self,
        tokens: &'a TokenExchangeResult,
    ) -> OAuthFuture<'a, Result<OAuthUserInfo, String>> {
        Box::pin(async move { fetch_google_userinfo(&tokens.access_token).await })
    }
}

impl OAuthIdentityProvider for GenericOAuthProvider {
    fn authorization_request(&self) -> OAuthAuthorizationRequest {
        let extra_auth_params = if self.is_microsoft() {
            vec![
                ("prompt".to_string(), "consent".to_string()),
                ("response_mode".to_string(), "query".to_string()),
            ]
        } else {
            vec![("access_type".to_string(), "offline".to_string())]
        };

        OAuthAuthorizationRequest {
            auth_url: self.auth_url.clone(),
            client_id: self.client_id.clone(),
            scopes: self.scopes.clone(),
            use_pkce: self.use_pkce,
            extra_auth_params,
        }
    }

    fn token_request(&self) -> OAuthTokenRequest {
        OAuthTokenRequest {
            token_url: self.token_url.clone(),
            client_id: self.client_id.clone(),
            client_secret: self.client_secret.clone(),
            scope: self.is_microsoft().then(|| self.scopes.join(" ")),
        }
    }

    fn fetch_user_info<'a>(
        &'a self,
        tokens: &'a TokenExchangeResult,
    ) -> OAuthFuture<'a, Result<OAuthUserInfo, String>> {
        Box::pin(async move { fetch_provider_userinfo(self, tokens).await })
    }
}

/// Bind to a localhost port for an OAuth callback. Tries the given port first,
/// falls back to nearby ports if taken. Returns the listener and the port it bound to.
pub async fn bind_oauth_listener(port: u16) -> Result<(TcpListener, u16), String> {
    let mut listener = None;
    for p in [port, port + 1, port + 2, port + 3] {
        match TcpListener::bind(format!("127.0.0.1:{p}")).await {
            Ok(l) => {
                listener = Some(l);
                break;
            }
            Err(_) => continue,
        }
    }

    let listener = listener.ok_or("Failed to bind to any port for OAuth callback")?;
    let actual_port = listener
        .local_addr()
        .map_err(|e| format!("Failed to get OAuth listener addr: {e}"))?
        .port();

    log::info!("OAuth callback server listening on port {actual_port}");
    Ok((listener, actual_port))
}

/// Wait for a single OAuth callback on an already-bound listener. Validates the
/// CSRF state, sends a success page, and returns the auth code.
pub async fn await_oauth_callback(
    listener: TcpListener,
    state: &str,
) -> Result<OAuthResult, String> {
    let actual_port = listener
        .local_addr()
        .map_err(|e| format!("Failed to get OAuth listener addr: {e}"))?
        .port();

    let (mut stream, _) = tokio::time::timeout(Duration::from_secs(300), listener.accept())
        .await
        .map_err(|_| "OAuth timed out — please try again".to_string())?
        .map_err(|e| format!("Failed to accept OAuth connection: {e}"))?;

    let mut buf = vec![0u8; 4096];
    let n = stream
        .read(&mut buf)
        .await
        .map_err(|e| format!("Failed to read OAuth request: {e}"))?;
    let request = String::from_utf8_lossy(&buf[..n]);

    let (code, returned_state) = parse_auth_code_and_state(&request)?;

    if returned_state != state {
        return Err("OAuth state mismatch — possible CSRF attack".to_string());
    }

    let html = r#"<!DOCTYPE html>
<html>
<head><title>Ratatoskr</title></head>
<body style="font-family: -apple-system, sans-serif; display: flex; align-items: center; justify-content: center; height: 100vh; margin: 0; background: #0f172a; color: #e2e8f0;">
<div style="text-align: center;">
<h1 style="margin-bottom: 8px;">Account Connected!</h1>
<p style="opacity: 0.7;">You can close this tab and return to Ratatoskr.</p>
</div>
</body>
</html>"#;

    let html_len = html.len();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {html_len}\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nConnection: close\r\n\r\n{html}"
    );

    _ = stream.write_all(response.as_bytes()).await;
    _ = stream.flush().await;

    Ok(OAuthResult {
        code,
        state: returned_state,
        actual_port,
    })
}

fn parse_auth_code_and_state(request: &str) -> Result<(String, String), String> {
    let first_line = request.lines().next().ok_or("Empty request")?;

    let path = first_line
        .split_whitespace()
        .nth(1)
        .ok_or("No path in request")?;

    if path.contains("error=") {
        let params = parse_query_string(path);
        let error = params.get("error").cloned().unwrap_or_default();
        return Err(format!("OAuth error: {error}"));
    }

    let params = parse_query_string(path);
    let code = params
        .get("code")
        .cloned()
        .ok_or_else(|| "No auth code in redirect".to_string())?;
    let state = params
        .get("state")
        .cloned()
        .ok_or_else(|| "No state in redirect".to_string())?;
    Ok((code, state))
}

fn parse_query_string(path: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    if let Some(query) = path.split('?').nth(1) {
        for pair in query.split('&') {
            let mut kv = pair.splitn(2, '=');
            if let (Some(key), Some(value)) = (kv.next(), kv.next()) {
                params.insert(key.to_string(), urlencoding_decode(value));
            }
        }
    }
    params
}

fn urlencoding_decode(s: &str) -> String {
    let mut result = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(byte) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            result.push(byte);
            i += 3;
            continue;
        }
        if bytes[i] == b'+' {
            result.push(b' ');
        } else {
            result.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8(result).unwrap_or_else(|_| s.to_string())
}

#[derive(Serialize, Deserialize)]
pub struct TokenExchangeResult {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: u64,
    pub token_type: String,
    pub scope: Option<String>,
    pub id_token: Option<String>,
}

/// Exchange an OAuth authorization code for tokens via Rust HTTP client (avoids CORS).
#[tauri::command]
pub async fn oauth_exchange_token(
    token_url: String,
    code: String,
    client_id: String,
    redirect_uri: String,
    code_verifier: Option<String>,
    client_secret: Option<String>,
    scope: Option<String>,
) -> Result<TokenExchangeResult, String> {
    let mut params = vec![
        ("code", code),
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        ("grant_type", "authorization_code".to_string()),
    ];
    if let Some(verifier) = code_verifier {
        params.push(("code_verifier", verifier));
    }
    if let Some(secret) = client_secret
        && !secret.is_empty()
    {
        params.push(("client_secret", secret));
    }
    if let Some(s) = scope {
        params.push(("scope", s));
    }

    let response = shared_http_client()
        .post(&token_url)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Token exchange request failed: {e}"))?;

    if !response.status().is_success() {
        let error = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(format!("Token exchange failed: {error}"));
    }

    response
        .json::<TokenExchangeResult>()
        .await
        .map_err(|e| format!("Failed to parse token response: {e}"))
}

/// Refresh an OAuth token via Rust HTTP client (avoids CORS).
#[tauri::command]
pub async fn oauth_refresh_token(
    token_url: String,
    refresh_token: String,
    client_id: String,
    client_secret: Option<String>,
    scope: Option<String>,
) -> Result<TokenExchangeResult, String> {
    let mut params = vec![
        ("refresh_token", refresh_token),
        ("client_id", client_id),
        ("grant_type", "refresh_token".to_string()),
    ];
    if let Some(secret) = client_secret
        && !secret.is_empty()
    {
        params.push(("client_secret", secret));
    }
    if let Some(s) = scope {
        params.push(("scope", s));
    }

    let response = shared_http_client()
        .post(&token_url)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Token refresh request failed: {e}"))?;

    if !response.status().is_success() {
        let error = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(format!("Token refresh failed: {error}"));
    }

    response
        .json::<TokenExchangeResult>()
        .await
        .map_err(|e| format!("Failed to parse token response: {e}"))
}

async fn fetch_google_userinfo(access_token: &str) -> Result<OAuthUserInfo, String> {
    let response = shared_http_client()
        .get(GOOGLE_USERINFO_URL)
        .header(AUTHORIZATION, format!("Bearer {access_token}"))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch Google user info: {e}"))?;

    if !response.status().is_success() {
        return Err("Failed to fetch Google user info".to_string());
    }

    response
        .json::<GoogleUserInfo>()
        .await
        .map(|user| OAuthUserInfo {
            email: user.email,
            name: user.name,
            picture: user.picture,
        })
        .map_err(|e| format!("Failed to parse Google user info: {e}"))
}

async fn fetch_provider_userinfo(
    provider: &GenericOAuthProvider,
    tokens: &TokenExchangeResult,
) -> Result<OAuthUserInfo, String> {
    if provider.is_microsoft() {
        return parse_microsoft_userinfo(tokens);
    }

    let user_info_url = provider.user_info_url.as_deref().ok_or_else(|| {
        format!(
            "Provider {} has no user info endpoint",
            provider.provider_id
        )
    })?;
    let response = shared_http_client()
        .get(user_info_url)
        .header(AUTHORIZATION, format!("Bearer {}", tokens.access_token))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch provider user info: {e}"))?;

    if !response.status().is_success() {
        let error = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(format!("Failed to fetch provider user info: {error}"));
    }

    let data = response
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("Failed to parse provider user info: {e}"))?;

    let email = data
        .get("email")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            format!(
                "Provider {} did not return an email address",
                provider.provider_id
            )
        })?
        .to_string();

    Ok(OAuthUserInfo {
        email,
        name: data
            .get("name")
            .and_then(serde_json::Value::as_str)
            .or_else(|| data.get("nickname").and_then(serde_json::Value::as_str))
            .unwrap_or_default()
            .to_string(),
        picture: data
            .get("picture")
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string),
    })
}

fn parse_microsoft_userinfo(tokens: &TokenExchangeResult) -> Result<OAuthUserInfo, String> {
    let id_token = tokens
        .id_token
        .as_deref()
        .ok_or("Microsoft OAuth response did not include an ID token")?;
    let payload = id_token
        .split('.')
        .nth(1)
        .ok_or("Invalid ID token format")?;
    let decoded = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|e| format!("Failed to decode ID token payload: {e}"))?;
    let claims = serde_json::from_slice::<serde_json::Value>(&decoded)
        .map_err(|e| format!("Failed to parse ID token payload: {e}"))?;

    let email = claims
        .get("email")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            claims
                .get("preferred_username")
                .and_then(serde_json::Value::as_str)
        })
        .filter(|s| !s.is_empty())
        .ok_or("Microsoft ID token did not contain an email or preferred_username claim")?
        .to_string();

    Ok(OAuthUserInfo {
        email,
        name: claims
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        picture: None,
    })
}

#[derive(Debug, Deserialize)]
struct GoogleUserInfo {
    email: String,
    name: String,
    picture: Option<String>,
}

fn random_base64url(size: usize) -> Result<String, String> {
    let mut buf = vec![0u8; size];
    getrandom::getrandom(&mut buf).map_err(|e| format!("Failed to generate random bytes: {e}"))?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

fn sha256_base64url(input: &[u8]) -> String {
    let digest = Sha256::digest(input);
    URL_SAFE_NO_PAD.encode(digest)
}
