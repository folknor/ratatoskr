use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::db::DbState;
use crate::gmail::client::{GmailClient, GmailState};
use crate::provider::crypto::{decrypt_value, encrypt_value, is_encrypted};

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo";
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

fn random_base64url(size: usize) -> Result<String, String> {
    let mut buf = vec![0u8; size];
    getrandom::getrandom(&mut buf).map_err(|e| format!("Failed to generate random bytes: {e}"))?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

fn sha256_base64url(input: &[u8]) -> String {
    let digest = Sha256::digest(input);
    URL_SAFE_NO_PAD.encode(digest)
}
