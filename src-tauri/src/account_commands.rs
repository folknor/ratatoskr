use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::db::DbState;
use crate::gmail::client::{GmailClient, GmailState};
use crate::graph::client::{GraphClient, GraphState};
use crate::graph::types::GraphProfile;
use crate::inline_image_store::InlineImageStoreState;
use crate::jmap::client::JmapState;
use crate::oauth::{
    GenericOAuthProvider, GoogleOAuthProvider, OAuthProviderAuthorizationRequest,
    PendingOAuthAuthorization, PendingOAuthAuthorizations,
};
use crate::provider::crypto::{AppCryptoState, decrypt_value, encrypt_value, is_encrypted};
use crate::sync::config;
use crate::{attachment_cache, body_store::BodyStoreState};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountResult {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub is_active: bool,
    pub provider: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthProviderAuthorizationResult {
    pub authorization_id: String,
    pub access_token: String,
    pub expires_in: u64,
    pub email: String,
    pub name: String,
    pub picture: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateImapOAuthAccountRequest {
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub imap_host: String,
    pub imap_port: i64,
    pub imap_security: String,
    pub smtp_host: String,
    pub smtp_port: i64,
    pub smtp_security: String,
    pub authorization_id: String,
    pub oauth_provider: String,
    pub oauth_client_id: String,
    pub oauth_client_secret: Option<String>,
    pub oauth_token_url: Option<String>,
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
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub provider: String,
    pub is_active: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountCaldavSettingsInfo {
    pub id: String,
    pub email: String,
    pub caldav_url: Option<String>,
    pub caldav_username: Option<String>,
    pub caldav_password: Option<String>,
    pub calendar_provider: Option<String>,
}

#[tauri::command]
pub async fn account_create_gmail_via_oauth(
    app: AppHandle,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<AccountResult, String> {
    let oauth = crate::oauth::authorize_with_provider(
        &app,
        &GoogleOAuthProvider::new(
            read_setting(&db, "google_client_id", gmail.encryption_key())
                .await?
                .ok_or("Google Client ID not configured. Go to Settings to set it up.")?,
            read_setting(&db, "google_client_secret", gmail.encryption_key())
                .await?
                .ok_or(
                    "Client Secret is not configured. Go to Settings -> Google API to add it.",
                )?,
        ),
    )
    .await?;

    let email_for_check = oauth.user_info.email.clone();
    let duplicate = db
        .with_conn(move |conn| {
            conn.query_row(
                "SELECT id FROM accounts WHERE email = ?1 AND provider = 'gmail_api' LIMIT 1",
                rusqlite::params![email_for_check],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| format!("Duplicate Gmail check failed: {e}"))
        })
        .await?;
    if let Some(existing_id) = duplicate {
        return Err(format!(
            "A Gmail account for {} already exists (id: {existing_id})",
            oauth.user_info.email
        ));
    }

    let account_id = uuid::Uuid::new_v4().to_string();
    let expires_at = chrono::Utc::now().timestamp()
        + i64::try_from(oauth.tokens.expires_in).map_err(|_| "Google token expiry overflow")?;
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

    Ok(AccountResult {
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
        Ok(
            config::calendar_provider_kind(&account).map(|provider| CalendarProviderInfo {
                provider: provider.to_string(),
            }),
        )
    })
    .await
}

#[tauri::command]
pub async fn account_get_caldav_connection_info(
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    account_id: String,
) -> Result<Option<CaldavConnectionInfo>, String> {
    let encryption_key = *gmail.encryption_key();
    db.with_conn(move |conn| {
        let Some(row) = conn
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
        else {
            return Ok(None);
        };

        let server_url = row
            .1
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "CalDAV credentials not configured".to_string())?;
        let password_raw = row
            .3
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "CalDAV credentials not configured".to_string())?;
        let password = if is_encrypted(&password_raw) {
            decrypt_value(&encryption_key, &password_raw).unwrap_or(password_raw)
        } else {
            password_raw
        };
        let username = row
            .2
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(row.0);

        Ok(Some(CaldavConnectionInfo {
            server_url,
            username,
            password,
        }))
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
            "SELECT id, email, display_name, avatar_url, provider, is_active FROM accounts WHERE id = ?1",
            rusqlite::params![account_id],
            |row| {
                Ok(AccountBasicInfo {
                    id: row.get(0)?,
                    email: row.get(1)?,
                    display_name: row.get(2)?,
                    avatar_url: row.get(3)?,
                    provider: row.get(4)?,
                    is_active: row.get::<_, i64>(5)? != 0,
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
            .prepare(
                "SELECT id, email, display_name, avatar_url, provider, is_active \
                 FROM accounts ORDER BY created_at ASC",
            )
            .map_err(|e| format!("prepare account list: {e}"))?;
        stmt.query_map([], |row| {
            Ok(AccountBasicInfo {
                id: row.get(0)?,
                email: row.get(1)?,
                display_name: row.get(2)?,
                avatar_url: row.get(3)?,
                provider: row.get(4)?,
                is_active: row.get::<_, i64>(5)? != 0,
            })
        })
        .map_err(|e| format!("query account list: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect account list: {e}"))
    })
    .await
}

#[tauri::command]
pub async fn account_delete(
    app_handle: AppHandle,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    inline_images: State<'_, InlineImageStoreState>,
    account_id: String,
) -> Result<(), String> {
    let (message_ids, cached_files, inline_hashes) = db
        .with_conn({
            let account_id = account_id.clone();
            move |conn| {
                let message_ids = {
                    let mut stmt = conn
                        .prepare("SELECT id FROM messages WHERE account_id = ?1")
                        .map_err(|e| format!("prepare account message query: {e}"))?;
                    stmt.query_map(rusqlite::params![&account_id], |row| {
                        row.get::<_, String>(0)
                    })
                    .map_err(|e| format!("query account message ids: {e}"))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("collect account message ids: {e}"))?
                };

                let cached_files = {
                    let mut stmt = conn
                        .prepare(
                            "SELECT DISTINCT local_path, content_hash
                             FROM attachments
                             WHERE account_id = ?1
                               AND cached_at IS NOT NULL
                               AND local_path IS NOT NULL
                               AND content_hash IS NOT NULL",
                        )
                        .map_err(|e| format!("prepare account cached attachment query: {e}"))?;
                    stmt.query_map(rusqlite::params![&account_id], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .map_err(|e| format!("query account cached attachments: {e}"))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("collect account cached attachments: {e}"))?
                };

                let inline_hashes = {
                    let mut stmt = conn
                        .prepare(
                            "SELECT DISTINCT content_hash
                             FROM attachments
                             WHERE account_id = ?1
                               AND is_inline = 1
                               AND content_hash IS NOT NULL",
                        )
                        .map_err(|e| format!("prepare account inline hash query: {e}"))?;
                    stmt.query_map(rusqlite::params![&account_id], |row| {
                        row.get::<_, String>(0)
                    })
                    .map_err(|e| format!("query account inline hashes: {e}"))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("collect account inline hashes: {e}"))?
                };

                Ok((message_ids, cached_files, inline_hashes))
            }
        })
        .await?;

    body_store.delete(message_ids).await?;

    db.with_conn({
        let account_id = account_id.clone();
        move |conn| {
            conn.execute(
                "DELETE FROM accounts WHERE id = ?1",
                rusqlite::params![account_id],
            )
            .map_err(|e| format!("delete account: {e}"))?;
            Ok(())
        }
    })
    .await?;

    for (local_path, content_hash) in cached_files {
        let remaining_refs: i64 = db
            .with_conn({
                let content_hash = content_hash.clone();
                move |conn| {
                    conn.query_row(
                        "SELECT COUNT(*) FROM attachments
                         WHERE content_hash = ?1 AND cached_at IS NOT NULL",
                        rusqlite::params![content_hash],
                        |row| row.get(0),
                    )
                    .map_err(|e| format!("count remaining cached attachment refs: {e}"))
                }
            })
            .await?;
        if remaining_refs == 0 {
            let app_data_dir = app_handle
                .path()
                .app_data_dir()
                .map_err(|e| format!("resolve app data dir: {e}"))?;
            let _ = attachment_cache::remove_cached_relative(&app_data_dir, &local_path);
        }
    }

    inline_images
        .delete_unreferenced(&db, inline_hashes)
        .await?;

    gmail.remove(&account_id).await;
    jmap.remove(&account_id).await;
    graph.remove(&account_id).await;
    Ok(())
}

#[tauri::command]
pub async fn account_get_caldav_settings_info(
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    account_id: String,
) -> Result<Option<AccountCaldavSettingsInfo>, String> {
    let encryption_key = *gmail.encryption_key();
    db.with_conn(move |conn| {
        conn.query_row(
            "SELECT id, email, caldav_url, caldav_username, caldav_password, calendar_provider \
             FROM accounts WHERE id = ?1",
            rusqlite::params![account_id],
            |row| {
                let password_raw: Option<String> = row.get(4)?;
                let caldav_password = password_raw.map(|raw| {
                    if is_encrypted(&raw) {
                        decrypt_value(&encryption_key, &raw).unwrap_or(raw)
                    } else {
                        raw
                    }
                });

                Ok(AccountCaldavSettingsInfo {
                    id: row.get(0)?,
                    email: row.get(1)?,
                    caldav_url: row.get(2)?,
                    caldav_username: row.get(3)?,
                    caldav_password,
                    calendar_provider: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(|e| format!("query account caldav settings info: {e}"))
    })
    .await
}

#[tauri::command]
pub async fn account_authorize_oauth_provider(
    app: AppHandle,
    request: OAuthProviderAuthorizationRequest,
    pending_oauth: State<'_, PendingOAuthAuthorizations>,
) -> Result<OAuthProviderAuthorizationResult, String> {
    let oauth =
        crate::oauth::authorize_with_provider(&app, &GenericOAuthProvider::from_request(request))
            .await?;
    let access_token = oauth.tokens.access_token.clone();
    let expires_in = oauth.tokens.expires_in;
    let authorization_id = uuid::Uuid::new_v4().to_string();
    pending_oauth
        .lock()
        .map_err(|_| "Pending OAuth store is poisoned".to_string())?
        .insert(
            authorization_id.clone(),
            PendingOAuthAuthorization {
                tokens: oauth.tokens,
            },
        );

    Ok(OAuthProviderAuthorizationResult {
        authorization_id,
        access_token,
        expires_in,
        email: oauth.user_info.email,
        name: oauth.user_info.name,
        picture: oauth.user_info.picture,
    })
}

#[tauri::command]
pub async fn account_create_imap_oauth(
    request: CreateImapOAuthAccountRequest,
    db: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
    pending_oauth: State<'_, PendingOAuthAuthorizations>,
) -> Result<AccountResult, String> {
    let authorization = pending_oauth
        .lock()
        .map_err(|_| "Pending OAuth store is poisoned".to_string())?
        .remove(&request.authorization_id)
        .ok_or_else(|| "OAuth authorization has expired or is invalid".to_string())?;
    let account_id = uuid::Uuid::new_v4().to_string();
    let access_token = encrypt_value(crypto.encryption_key(), &authorization.tokens.access_token)?;
    let refresh_token = encrypt_value(
        crypto.encryption_key(),
        authorization
            .tokens
            .refresh_token
            .as_deref()
            .ok_or_else(|| "OAuth provider did not return a refresh token".to_string())?,
    )?;
    let oauth_client_secret = request
        .oauth_client_secret
        .as_deref()
        .filter(|secret| !secret.is_empty())
        .map(|secret| encrypt_value(crypto.encryption_key(), secret))
        .transpose()?;

    db.with_conn({
        let account_id = account_id.clone();
        let email = request.email.clone();
        let display_name = request.display_name.clone();
        let avatar_url = request.avatar_url.clone();
        move |conn| {
            conn.execute(
                "INSERT INTO accounts (id, email, display_name, avatar_url, access_token, \
                 refresh_token, token_expires_at, provider, auth_method, imap_host, imap_port, \
                 imap_security, smtp_host, smtp_port, smtp_security, oauth_provider, \
                 oauth_client_id, oauth_client_secret, oauth_token_url, imap_username, \
                 accept_invalid_certs) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'imap', 'oauth2', ?8, ?9, ?10, ?11, ?12, \
                 ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
                rusqlite::params![
                    account_id,
                    email,
                    display_name,
                    avatar_url,
                    access_token,
                    refresh_token,
                    i64::try_from(authorization.tokens.expires_in)
                        .ok()
                        .and_then(|expires_in| chrono::Utc::now()
                            .timestamp()
                            .checked_add(expires_in))
                        .unwrap_or_else(|| chrono::Utc::now().timestamp()),
                    request.imap_host,
                    request.imap_port,
                    request.imap_security,
                    request.smtp_host,
                    request.smtp_port,
                    request.smtp_security,
                    request.oauth_provider,
                    request.oauth_client_id,
                    oauth_client_secret,
                    request.oauth_token_url,
                    request.imap_username,
                    if request.accept_invalid_certs { 1 } else { 0 },
                ],
            )
            .map_err(|e| format!("Failed to insert OAuth IMAP account: {e}"))?;
            Ok(())
        }
    })
    .await?;

    Ok(AccountResult {
        id: account_id,
        email: request.email,
        display_name: request.display_name.unwrap_or_default(),
        avatar_url: request.avatar_url,
        is_active: true,
        provider: "imap".to_string(),
    })
}

#[tauri::command]
pub async fn account_create_graph_via_oauth(
    app: AppHandle,
    db: State<'_, DbState>,
    graph: State<'_, GraphState>,
) -> Result<AccountResult, String> {
    let client_id = read_setting(&db, "microsoft_client_id", graph.encryption_key())
        .await?
        .ok_or("Microsoft Client ID not configured. Go to Settings to set it up.")?;

    let oauth = crate::oauth::authorize_with_provider(
        &app,
        &GenericOAuthProvider::microsoft_graph(client_id),
    )
    .await?;

    let account_id = uuid::Uuid::new_v4().to_string();
    let expires_at = chrono::Utc::now().timestamp() + oauth.tokens.expires_in as i64;
    let access_token = encrypt_value(graph.encryption_key(), &oauth.tokens.access_token)?;
    let refresh_token = encrypt_value(
        graph.encryption_key(),
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

    let init_result = async {
        let client = GraphClient::from_account(&db, &account_id, *graph.encryption_key()).await?;
        let profile: GraphProfile = client
            .get_json("/me?$select=displayName,mail,userPrincipalName", &db)
            .await?;
        Ok::<_, String>((client, profile))
    }
    .await;

    let (client, profile) = match init_result {
        Ok(pair) => pair,
        Err(e) => {
            let cleanup_id = account_id.clone();
            let _ = db
                .with_conn(move |conn| {
                    conn.execute(
                        "DELETE FROM accounts WHERE id = ?1",
                        rusqlite::params![cleanup_id],
                    )
                    .map_err(|de| format!("cleanup delete failed: {de}"))?;
                    Ok(())
                })
                .await;
            return Err(e);
        }
    };
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

    Ok(AccountResult {
        id: account_id,
        email,
        display_name,
        avatar_url: None,
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
    let oauth = crate::oauth::authorize_with_provider(
        &app,
        &GoogleOAuthProvider::new(
            read_setting(&db, "google_client_id", gmail.encryption_key())
                .await?
                .ok_or("Google Client ID not configured. Go to Settings to set it up.")?,
            read_setting(&db, "google_client_secret", gmail.encryption_key())
                .await?
                .ok_or(
                    "Client Secret is not configured. Go to Settings -> Google API to add it.",
                )?,
        ),
    )
    .await?;
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
    let expires_at = chrono::Utc::now().timestamp()
        + i64::try_from(oauth.tokens.expires_in).map_err(|_| "Google token expiry overflow")?;

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
