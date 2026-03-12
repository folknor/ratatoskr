use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const OAUTH_CALLBACK_PORT: u16 = 17248;

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

fn random_base64url(size: usize) -> Result<String, String> {
    let mut buf = vec![0u8; size];
    getrandom::getrandom(&mut buf).map_err(|e| format!("Failed to generate random bytes: {e}"))?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

fn sha256_base64url(input: &[u8]) -> String {
    let digest = Sha256::digest(input);
    URL_SAFE_NO_PAD.encode(digest)
}
