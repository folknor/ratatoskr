pub use ratatoskr_core::oauth::*;

use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

/// Run the OAuth authorization flow, opening the browser via Tauri's opener plugin.
pub async fn authorize_with_provider<P: OAuthIdentityProvider>(
    app: &AppHandle,
    provider: &P,
) -> Result<OAuthAuthorizationBundle, String> {
    // Clone the handle so the closure is 'static-friendly (OpenerExt is on AppHandle).
    let handle = app.clone();
    let open_url = move |url: &str| {
        handle
            .opener()
            .open_url(url, None::<&str>)
            .map_err(|e| format!("Failed to open browser for OAuth: {e}"))
    };

    ratatoskr_core::oauth::authorize_with_provider(provider, &open_url).await
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
    ratatoskr_core::oauth::oauth_exchange_token(
        token_url,
        code,
        client_id,
        redirect_uri,
        code_verifier,
        client_secret,
        scope,
    )
    .await
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
    ratatoskr_core::oauth::oauth_refresh_token(
        token_url,
        refresh_token,
        client_id,
        client_secret,
        scope,
    )
    .await
}
