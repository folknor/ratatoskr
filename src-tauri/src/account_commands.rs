use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::db::DbState;
use crate::gmail::client::{GmailClient, GmailState};
use crate::graph::client::{GraphClient, GraphState};
use crate::graph::types::GraphProfile;
use crate::provider::crypto::{decrypt_value, encrypt_value, is_encrypted};
use crate::sync::config;

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo";
const MICROSOFT_GRAPH_AUTH_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/authorize";
const MICROSOFT_GRAPH_TOKEN_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";
const OAUTH_CALLBACK_PORT: u16 = 17248;
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
const MICROSOFT_GRAPH_SCOPES: [&str; 6] = [
    "Mail.ReadWrite",
    "Mail.Send",
    "MailboxSettings.ReadWrite",
    "offline_access",
    "openid",
    "profile",
];

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailAccountResult {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub avatar_url: String,
    pub is_active: bool,
    pub provider: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphAccountResult {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub avatar_url: String,
    pub is_active: bool,
    pub provider: String,
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthProviderAuthorizationResult {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: u64,
    pub token_type: String,
    pub scope: Option<String>,
    pub id_token: Option<String>,
    pub email: String,
    pub name: String,
    pub picture: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateImapOAuthAccountRequest {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub imap_host: String,
    pub imap_port: i64,
    pub imap_security: String,
    pub smtp_host: String,
    pub smtp_port: i64,
    pub smtp_security: String,
    pub access_token: String,
    pub refresh_token: String,
    pub token_expires_at: i64,
    pub oauth_provider: String,
    pub oauth_client_id: String,
    pub oauth_client_secret: Option<String>,
    pub imap_username: Option<String>,
    pub accept_invalid_certs: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarProviderInfo {
    pub provider: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaldavConnectionInfo {
    pub server_url: String,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountBasicInfo {
    pub id: String,
    pub email: String,
    pub provider: String,
    pub is_active: bool,
}

#[derive(Debug, Deserialize)]
struct GoogleTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleUserInfo {
    email: String,
    name: String,
    picture: String,
}

#[derive(Debug)]
struct OAuthProviderUserInfo {
    email: String,
    name: String,
    picture: Option<String>,
}

#[tauri::command]
pub async fn account_create_gmail_via_oauth(
    app: AppHandle,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<GmailAccountResult, String> {
    let oauth = perform_google_oauth(&app, &db, gmail.encryption_key()).await?;
    let account_id = uuid::Uuid::new_v4().to_string();
    let expires_at = chrono::Utc::now().timestamp() + oauth.tokens.expires_in;
    let access_token = encrypt_value(gmail.encryption_key(), &oauth.tokens.access_token)?;
    let refresh_token = encrypt_value(
        gmail.encryption_key(),
        oauth
            .tokens
            .refresh_token
            .as_deref()
            .ok_or("Google did not return a refresh token")?,
    )?;

    db.with_conn({
        let id = account_id.clone();
        let email = oauth.user_info.email.clone();
        let display_name = oauth.user_info.name.clone();
        let avatar_url = oauth.user_info.picture.clone();
        move |conn| {
            conn.execute(
                "INSERT INTO accounts (id, email, display_name, avatar_url, access_token, \
                 refresh_token, token_expires_at, provider, auth_method) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'gmail_api', 'oauth2')",
                rusqlite::params![
                    id,
                    email,
                    display_name,
                    avatar_url,
                    access_token,
                    refresh_token,
                    expires_at
                ],
            )
            .map_err(|e| format!("Failed to insert Gmail account: {e}"))?;
            Ok(())
        }
    })
    .await?;

    let client = GmailClient::from_account(&db, &account_id, *gmail.encryption_key()).await?;
    gmail.insert(account_id.clone(), client).await;

    Ok(GmailAccountResult {
        id: account_id,
        email: oauth.user_info.email,
        display_name: oauth.user_info.name,
        avatar_url: oauth.user_info.picture,
        is_active: true,
        provider: "gmail_api".to_string(),
    })
}

#[tauri::command]
pub async fn account_get_calendar_provider_info(
    db: State<'_, DbState>,
    account_id: String,
) -> Result<Option<CalendarProviderInfo>, String> {
    db.with_conn(move |conn| {
        let account = config::get_account(conn, &account_id)?;
        Ok(config::calendar_provider_kind(&account).map(|provider| CalendarProviderInfo {
            provider: provider.to_string(),
        }))
    })
    .await
}

#[tauri::command]
pub async fn account_get_caldav_connection_info(
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    account_id: String,
) -> Result<CaldavConnectionInfo, String> {
    let encryption_key = *gmail.encryption_key();
    db.with_conn(move |conn| {
        let row = conn
            .query_row(
                "SELECT email, caldav_url, caldav_username, caldav_password FROM accounts WHERE id = ?1",
                rusqlite::params![account_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| format!("query caldav account: {e}"))?
            .ok_or_else(|| "Account not found".to_string())?;

        let server_url = row
            .1
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "CalDAV credentials not configured".to_string())?;
        let password_raw = row
            .3
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "CalDAV credentials not configured".to_string())?;
        let password = if is_encrypted(&password_raw) {
            decrypt_value(&encryption_key, &password_raw)?
        } else {
            password_raw
        };
        let username = row
            .2
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(row.0);

        Ok(CaldavConnectionInfo {
            server_url,
            username,
            password,
        })
    })
    .await
}

#[tauri::command]
pub async fn account_get_basic_info(
    db: State<'_, DbState>,
    account_id: String,
) -> Result<Option<AccountBasicInfo>, String> {
    db.with_conn(move |conn| {
        conn.query_row(
            "SELECT id, email, provider, is_active FROM accounts WHERE id = ?1",
            rusqlite::params![account_id],
            |row| {
                Ok(AccountBasicInfo {
                    id: row.get(0)?,
                    email: row.get(1)?,
                    provider: row.get(2)?,
                    is_active: row.get::<_, i64>(3)? != 0,
                })
            },
        )
        .optional()
        .map_err(|e| format!("query account basic info: {e}"))
    })
    .await
}

#[tauri::command]
pub async fn account_list_basic_info(
    db: State<'_, DbState>,
) -> Result<Vec<AccountBasicInfo>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare("SELECT id, email, provider, is_active FROM accounts ORDER BY created_at ASC")
            .map_err(|e| format!("prepare account list: {e}"))?;
        stmt.query_map([], |row| {
            Ok(AccountBasicInfo {
                id: row.get(0)?,
                email: row.get(1)?,
                provider: row.get(2)?,
                is_active: row.get::<_, i64>(3)? != 0,
            })
        })
        .map_err(|e| format!("query account list: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect account list: {e}"))
    })
    .await
}

#[tauri::command]
pub async fn account_authorize_oauth_provider(
    app: AppHandle,
    request: OAuthProviderAuthorizationRequest,
) -> Result<OAuthProviderAuthorizationResult, String> {
    let oauth = perform_provider_oauth(&app, &request).await?;

    Ok(OAuthProviderAuthorizationResult {
        access_token: oauth.tokens.access_token,
        refresh_token: oauth.tokens.refresh_token,
        expires_in: oauth.tokens.expires_in,
        token_type: oauth.tokens.token_type,
        scope: oauth.tokens.scope,
        id_token: oauth.tokens.id_token,
        email: oauth.user_info.email,
        name: oauth.user_info.name,
        picture: oauth.user_info.picture,
    })
}

#[tauri::command]
pub async fn account_create_imap_oauth(
    request: CreateImapOAuthAccountRequest,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<(), String> {
    let access_token = encrypt_value(gmail.encryption_key(), &request.access_token)?;
    let refresh_token = encrypt_value(gmail.encryption_key(), &request.refresh_token)?;
    let oauth_client_secret = request
        .oauth_client_secret
        .as_deref()
        .filter(|secret| !secret.is_empty())
        .map(|secret| encrypt_value(gmail.encryption_key(), secret))
        .transpose()?;

    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO accounts (id, email, display_name, avatar_url, access_token, \
             refresh_token, token_expires_at, provider, auth_method, imap_host, imap_port, \
             imap_security, smtp_host, smtp_port, smtp_security, oauth_provider, \
             oauth_client_id, oauth_client_secret, imap_username, accept_invalid_certs) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'imap', 'oauth2', ?8, ?9, ?10, ?11, ?12, \
             ?13, ?14, ?15, ?16, ?17, ?18)",
            rusqlite::params![
                request.id,
                request.email,
                request.display_name,
                request.avatar_url,
                access_token,
                refresh_token,
                request.token_expires_at,
                request.imap_host,
                request.imap_port,
                request.imap_security,
                request.smtp_host,
                request.smtp_port,
                request.smtp_security,
                request.oauth_provider,
                request.oauth_client_id,
                oauth_client_secret,
                request.imap_username,
                if request.accept_invalid_certs { 1 } else { 0 },
            ],
        )
        .map_err(|e| format!("Failed to insert OAuth IMAP account: {e}"))?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn account_create_graph_via_oauth(
    app: AppHandle,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    graph: State<'_, GraphState>,
) -> Result<GraphAccountResult, String> {
    let client_id = read_setting(&db, "microsoft_client_id", gmail.encryption_key())
        .await?
        .ok_or("Microsoft Client ID not configured. Go to Settings to set it up.")?;

    let oauth = perform_provider_oauth(
        &app,
        &OAuthProviderAuthorizationRequest {
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
        },
    )
    .await?;

    if oauth.user_info.email.is_empty() {
        return Err("Could not determine email address from Microsoft account".to_string());
    }

    let account_id = uuid::Uuid::new_v4().to_string();
    let expires_at = chrono::Utc::now().timestamp() + oauth.tokens.expires_in as i64;
    let access_token = encrypt_value(gmail.encryption_key(), &oauth.tokens.access_token)?;
    let refresh_token = encrypt_value(
        gmail.encryption_key(),
        oauth
            .tokens
            .refresh_token
            .as_deref()
            .ok_or("Microsoft did not return a refresh token")?,
    )?;

    db.with_conn({
        let id = account_id.clone();
        let email = oauth.user_info.email.clone();
        let display_name = oauth.user_info.name.clone();
        move |conn| {
            conn.execute(
                "INSERT INTO accounts (id, email, display_name, avatar_url, access_token, \
                 refresh_token, token_expires_at, provider, auth_method) \
                 VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, 'graph', 'oauth2')",
                rusqlite::params![
                    id,
                    email,
                    display_name,
                    access_token,
                    refresh_token,
                    expires_at
                ],
            )
            .map_err(|e| format!("Failed to insert Graph account: {e}"))?;
            Ok(())
        }
    })
    .await?;

    let client = GraphClient::from_account(&db, &account_id, *graph.encryption_key()).await?;
    let profile: GraphProfile = client
        .get_json("/me?$select=displayName,mail,userPrincipalName", &db)
        .await?;
    graph.insert(account_id.clone(), client).await;

    let email = profile
        .mail
        .or(profile.user_principal_name)
        .filter(|value| !value.is_empty())
        .unwrap_or(oauth.user_info.email);
    let display_name = profile
        .display_name
        .filter(|value| !value.is_empty())
        .unwrap_or(oauth.user_info.name);

    db.with_conn({
        let id = account_id.clone();
        let email = email.clone();
        let display_name = display_name.clone();
        move |conn| {
            conn.execute(
                "UPDATE accounts SET email = ?1, display_name = ?2, updated_at = unixepoch() \
                 WHERE id = ?3",
                rusqlite::params![email, display_name, id],
            )
            .map_err(|e| format!("Failed to finalize Graph account profile: {e}"))?;
            Ok(())
        }
    })
    .await?;

    Ok(GraphAccountResult {
        id: account_id,
        email,
        display_name,
        avatar_url: String::new(),
        is_active: true,
        provider: "graph".to_string(),
    })
}

#[tauri::command]
pub async fn account_reauthorize_gmail(
    app: AppHandle,
    account_id: String,
    expected_email: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<(), String> {
    let oauth = perform_google_oauth(&app, &db, gmail.encryption_key()).await?;
    if !oauth.user_info.email.eq_ignore_ascii_case(&expected_email) {
        return Err(format!(
            "Signed in as {}, but expected {}. Please sign in with the correct account.",
            oauth.user_info.email, expected_email
        ));
    }

    let refresh_token = oauth
        .tokens
        .refresh_token
        .ok_or("Google did not return a refresh token. Please revoke app access and try again.")?;
    let access_token = encrypt_value(gmail.encryption_key(), &oauth.tokens.access_token)?;
    let refresh_token = encrypt_value(gmail.encryption_key(), &refresh_token)?;
    let expires_at = chrono::Utc::now().timestamp() + oauth.tokens.expires_in;

    db.with_conn({
        let id = account_id.clone();
        move |conn| {
            conn.execute(
                "UPDATE accounts SET access_token = ?1, refresh_token = ?2, \
                 token_expires_at = ?3, updated_at = unixepoch() WHERE id = ?4",
                rusqlite::params![access_token, refresh_token, expires_at, id],
            )
            .map_err(|e| format!("Failed to update Gmail account tokens: {e}"))?;
            Ok(())
        }
    })
    .await?;

    gmail.remove(&account_id).await;
    let client = GmailClient::from_account(&db, &account_id, *gmail.encryption_key()).await?;
    gmail.insert(account_id, client).await;
    Ok(())
}

struct GoogleOAuthBundle {
    tokens: GoogleTokenResponse,
    user_info: GoogleUserInfo,
}

struct OAuthProviderBundle {
    tokens: crate::oauth::TokenExchangeResult,
    user_info: OAuthProviderUserInfo,
}

async fn perform_google_oauth(
    app: &AppHandle,
    db: &DbState,
    encryption_key: &[u8; 32],
) -> Result<GoogleOAuthBundle, String> {
    let client_id = read_setting(db, "google_client_id", encryption_key)
        .await?
        .ok_or("Google Client ID not configured. Go to Settings to set it up.")?;
    let client_secret = read_setting(db, "google_client_secret", encryption_key)
        .await?
        .ok_or("Client Secret is not configured. Go to Settings -> Google API to add it.")?;

    let code_verifier = random_base64url(32)?;
    let code_challenge = sha256_base64url(code_verifier.as_bytes());
    let state = random_base64url(32)?;
    let redirect_uri = format!("http://127.0.0.1:{OAUTH_CALLBACK_PORT}");
    let auth_url = format!(
        "{GOOGLE_AUTH_URL}?client_id={}&redirect_uri={}&response_type=code&scope={}&code_challenge={}&code_challenge_method=S256&access_type=offline&prompt=consent&state={}",
        urlencoding::encode(&client_id),
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(GOOGLE_SCOPES),
        urlencoding::encode(&code_challenge),
        urlencoding::encode(&state),
    );

    let server = crate::oauth::start_oauth_server(OAUTH_CALLBACK_PORT, state);
    app.opener()
        .open_url(auth_url, None::<&str>)
        .map_err(|e| format!("Failed to open browser for OAuth: {e}"))?;
    let result = server.await?;

    let tokens = exchange_google_code(
        &result.code,
        &client_id,
        &client_secret,
        &redirect_uri,
        &code_verifier,
    )
    .await?;
    let user_info = fetch_google_userinfo(&tokens.access_token).await?;

    Ok(GoogleOAuthBundle { tokens, user_info })
}

async fn perform_provider_oauth(
    app: &AppHandle,
    request: &OAuthProviderAuthorizationRequest,
) -> Result<OAuthProviderBundle, String> {
    let code_verifier = if request.use_pkce {
        Some(random_base64url(32)?)
    } else {
        None
    };
    let code_challenge = code_verifier
        .as_deref()
        .map(|verifier| sha256_base64url(verifier.as_bytes()));
    let state = random_base64url(32)?;
    let redirect_uri = format!("http://localhost:{OAUTH_CALLBACK_PORT}");
    let mut params = vec![
        ("client_id".to_string(), request.client_id.clone()),
        ("redirect_uri".to_string(), redirect_uri.clone()),
        ("response_type".to_string(), "code".to_string()),
        ("scope".to_string(), request.scopes.join(" ")),
        ("state".to_string(), state.clone()),
    ];

    if let Some(challenge) = code_challenge {
        params.push(("code_challenge".to_string(), challenge));
        params.push(("code_challenge_method".to_string(), "S256".to_string()));
    }

    if request.provider_id == "microsoft" || request.provider_id == "microsoft_graph" {
        params.push(("prompt".to_string(), "consent".to_string()));
        params.push(("response_mode".to_string(), "query".to_string()));
    }

    let auth_url = format!(
        "{}?{}",
        request.auth_url,
        params
            .into_iter()
            .map(|(key, value)| format!("{}={}", urlencoding::encode(&key), urlencoding::encode(&value)))
            .collect::<Vec<_>>()
            .join("&")
    );

    let server = crate::oauth::start_oauth_server(OAUTH_CALLBACK_PORT, state);
    app.opener()
        .open_url(auth_url, None::<&str>)
        .map_err(|e| format!("Failed to open browser for OAuth: {e}"))?;
    let result = server.await?;

    let tokens = crate::oauth::oauth_exchange_token(
        request.token_url.clone(),
        result.code,
        request.client_id.clone(),
        redirect_uri,
        code_verifier.filter(|_| request.use_pkce),
        request.client_secret.clone(),
        if request.provider_id == "microsoft" || request.provider_id == "microsoft_graph" {
            Some(request.scopes.join(" "))
        } else {
            None
        },
    )
    .await?;
    let user_info = fetch_provider_userinfo(request, &tokens).await?;

    Ok(OAuthProviderBundle { tokens, user_info })
}

async fn read_setting(
    db: &DbState,
    key: &str,
    encryption_key: &[u8; 32],
) -> Result<Option<String>, String> {
    let key_name = key.to_string();
    let value = db
        .with_conn(move |conn| {
            conn.query_row(
                "SELECT value FROM settings WHERE key = ?1",
                rusqlite::params![key_name],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| format!("Failed to read setting: {e}"))
        })
        .await?;

    Ok(value.map(|raw| {
        if is_encrypted(&raw) {
            decrypt_value(encryption_key, &raw).unwrap_or(raw)
        } else {
            raw
        }
    }))
}

async fn exchange_google_code(
    code: &str,
    client_id: &str,
    client_secret: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> Result<GoogleTokenResponse, String> {
    let response = reqwest::Client::new()
        .post(GOOGLE_TOKEN_URL)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .form(&[
            ("code", code),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
            ("code_verifier", code_verifier),
        ])
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
        .json::<GoogleTokenResponse>()
        .await
        .map_err(|e| format!("Failed to parse token response: {e}"))
}

async fn fetch_google_userinfo(access_token: &str) -> Result<GoogleUserInfo, String> {
    let response = reqwest::Client::new()
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
        .map_err(|e| format!("Failed to parse Google user info: {e}"))
}

async fn fetch_provider_userinfo(
    request: &OAuthProviderAuthorizationRequest,
    tokens: &crate::oauth::TokenExchangeResult,
) -> Result<OAuthProviderUserInfo, String> {
    if request.provider_id == "microsoft" || request.provider_id == "microsoft_graph" {
        return parse_microsoft_userinfo(tokens);
    }

    let user_info_url = request
        .user_info_url
        .as_deref()
        .ok_or_else(|| format!("Provider {} has no user info endpoint", request.provider_id))?;
    let response = reqwest::Client::new()
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

    Ok(OAuthProviderUserInfo {
        email: data
            .get("email")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
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

fn parse_microsoft_userinfo(
    tokens: &crate::oauth::TokenExchangeResult,
) -> Result<OAuthProviderUserInfo, String> {
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

    Ok(OAuthProviderUserInfo {
        email: claims
            .get("email")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                claims
                    .get("preferred_username")
                    .and_then(serde_json::Value::as_str)
            })
            .unwrap_or_default()
            .to_string(),
        name: claims
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        picture: None,
    })
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
