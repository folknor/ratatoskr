use rusqlite::OptionalExtension;
use tauri::{AppHandle, State};

use crate::db::DbState;
use crate::gmail::client::{GmailClient, GmailState};
use crate::graph::client::{GraphClient, GraphState};
use crate::graph::types::GraphProfile;
use crate::oauth::{
    GenericOAuthProvider, GoogleOAuthProvider, OAuthProviderAuthorizationRequest,
    PendingOAuthAuthorization, PendingOAuthAuthorizations,
};
use crate::provider::crypto::{AppCryptoState, encrypt_value};

use super::types::{
    AccountResult, CreateImapOAuthAccountRequest, OAuthProviderAuthorizationResult,
};

#[tauri::command]
pub async fn account_create_gmail_via_oauth(
    app: AppHandle,
    client_id: String,
    client_secret: Option<String>,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<AccountResult, String> {
    let secret = client_secret.as_deref().unwrap_or_default().to_string();
    let oauth = crate::oauth::authorize_with_provider(
        &app,
        &GoogleOAuthProvider::new(client_id.clone(), secret.clone()),
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

    let encrypted_client_id = encrypt_value(gmail.encryption_key(), &client_id)?;
    let encrypted_client_secret = if secret.is_empty() {
        None
    } else {
        Some(encrypt_value(gmail.encryption_key(), &secret)?)
    };

    db.with_conn({
        let id = account_id.clone();
        let email = oauth.user_info.email.clone();
        let display_name = oauth.user_info.name.clone();
        let avatar_url = oauth.user_info.picture.clone();
        move |conn| {
            conn.execute(
                "INSERT INTO accounts (id, email, display_name, avatar_url, access_token, \
                 refresh_token, token_expires_at, provider, auth_method, oauth_client_id, \
                 oauth_client_secret) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'gmail_api', 'oauth2', ?8, ?9)",
                rusqlite::params![
                    id,
                    email,
                    display_name,
                    avatar_url,
                    access_token,
                    refresh_token,
                    expires_at,
                    encrypted_client_id,
                    encrypted_client_secret,
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
    client_id: Option<String>,
    db: State<'_, DbState>,
    graph: State<'_, GraphState>,
) -> Result<AccountResult, String> {
    let provider = match &client_id {
        Some(id) if !id.is_empty() => GenericOAuthProvider::microsoft_graph(id.clone()),
        _ => GenericOAuthProvider::microsoft_graph_default(),
    };
    let oauth = crate::oauth::authorize_with_provider(&app, &provider).await?;

    let account_id = uuid::Uuid::new_v4().to_string();
    let expires_at = chrono::Utc::now().timestamp()
        + i64::try_from(oauth.tokens.expires_in).map_err(|_| "Microsoft token expiry overflow")?;
    let access_token = encrypt_value(graph.encryption_key(), &oauth.tokens.access_token)?;
    let refresh_token = encrypt_value(
        graph.encryption_key(),
        oauth
            .tokens
            .refresh_token
            .as_deref()
            .ok_or("Microsoft did not return a refresh token")?,
    )?;

    let actual_client_id = client_id.unwrap_or_else(|| crate::oauth::default_microsoft_client_id().to_string());
    let encrypted_client_id = encrypt_value(graph.encryption_key(), &actual_client_id)?;

    db.with_conn({
        let id = account_id.clone();
        let email = oauth.user_info.email.clone();
        let display_name = oauth.user_info.name.clone();
        move |conn| {
            conn.execute(
                "INSERT INTO accounts (id, email, display_name, avatar_url, access_token, \
                 refresh_token, token_expires_at, provider, auth_method, oauth_client_id) \
                 VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, 'graph', 'oauth2', ?7)",
                rusqlite::params![
                    id,
                    email,
                    display_name,
                    access_token,
                    refresh_token,
                    expires_at,
                    encrypted_client_id,
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
    client_id: Option<String>,
    client_secret: Option<String>,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<(), String> {
    let encryption_key = *gmail.encryption_key();

    // Use provided credentials, or read existing per-account credentials
    let (resolved_client_id, resolved_client_secret) = if let Some(ref cid) = client_id {
        (cid.clone(), client_secret.clone().unwrap_or_default())
    } else {
        let aid = account_id.clone();
        db.with_conn(move |conn| {
            conn.query_row(
                "SELECT oauth_client_id, oauth_client_secret FROM accounts WHERE id = ?1",
                rusqlite::params![aid],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .map_err(|e| format!("Failed to read account credentials: {e}"))
            .and_then(|(cid, cs)| {
                let cid = cid.filter(|s| !s.is_empty()).ok_or_else(|| {
                    "Account has no stored OAuth credentials. Provide client_id to reauthorize."
                        .to_string()
                })?;
                let cid = if crate::provider::crypto::is_encrypted(&cid) {
                    crate::provider::crypto::decrypt_value(&encryption_key, &cid).unwrap_or(cid)
                } else {
                    cid
                };
                let cs = cs
                    .filter(|s| !s.is_empty())
                    .map(|s| {
                        if crate::provider::crypto::is_encrypted(&s) {
                            crate::provider::crypto::decrypt_value(&encryption_key, &s).unwrap_or(s)
                        } else {
                            s
                        }
                    })
                    .unwrap_or_default();
                Ok((cid, cs))
            })
        })
        .await?
    };

    let oauth = crate::oauth::authorize_with_provider(
        &app,
        &GoogleOAuthProvider::new(resolved_client_id.clone(), resolved_client_secret.clone()),
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
    let access_token = encrypt_value(&encryption_key, &oauth.tokens.access_token)?;
    let refresh_token = encrypt_value(&encryption_key, &refresh_token)?;
    let expires_at = chrono::Utc::now().timestamp()
        + i64::try_from(oauth.tokens.expires_in).map_err(|_| "Google token expiry overflow")?;

    // If new credentials were provided, update them on the account
    let new_encrypted_cid = if client_id.is_some() {
        Some(encrypt_value(&encryption_key, &resolved_client_id)?)
    } else {
        None
    };
    let new_encrypted_cs = if client_id.is_some() && !resolved_client_secret.is_empty() {
        Some(encrypt_value(&encryption_key, &resolved_client_secret)?)
    } else {
        None
    };

    db.with_conn({
        let id = account_id.clone();
        move |conn| {
            if let Some(enc_cid) = new_encrypted_cid {
                conn.execute(
                    "UPDATE accounts SET access_token = ?1, refresh_token = ?2, \
                     token_expires_at = ?3, oauth_client_id = ?4, oauth_client_secret = ?5, \
                     updated_at = unixepoch() WHERE id = ?6",
                    rusqlite::params![
                        access_token,
                        refresh_token,
                        expires_at,
                        enc_cid,
                        new_encrypted_cs,
                        id
                    ],
                )
                .map_err(|e| format!("Failed to update Gmail account tokens: {e}"))?;
            } else {
                conn.execute(
                    "UPDATE accounts SET access_token = ?1, refresh_token = ?2, \
                     token_expires_at = ?3, updated_at = unixepoch() WHERE id = ?4",
                    rusqlite::params![access_token, refresh_token, expires_at, id],
                )
                .map_err(|e| format!("Failed to update Gmail account tokens: {e}"))?;
            }
            Ok(())
        }
    })
    .await?;

    gmail.remove(&account_id).await;
    let client = GmailClient::from_account(&db, &account_id, encryption_key).await?;
    gmail.insert(account_id, client).await;
    Ok(())
}

#[tauri::command]
pub async fn account_reauthorize_graph(
    app: AppHandle,
    account_id: String,
    expected_email: String,
    client_id: Option<String>,
    db: State<'_, DbState>,
    graph: State<'_, GraphState>,
) -> Result<(), String> {
    let encryption_key = *graph.encryption_key();

    let resolved_client_id = if let Some(cid) = client_id.clone() {
        cid
    } else {
        let aid = account_id.clone();
        db.with_conn(move |conn| {
            conn.query_row(
                "SELECT oauth_client_id FROM accounts WHERE id = ?1",
                rusqlite::params![aid],
                |row| row.get::<_, Option<String>>(0),
            )
            .map_err(|e| format!("Failed to read account credentials: {e}"))
            .map(|cid| {
                match cid.filter(|s| !s.is_empty()) {
                    Some(encrypted) => {
                        if crate::provider::crypto::is_encrypted(&encrypted) {
                            crate::provider::crypto::decrypt_value(&encryption_key, &encrypted).unwrap_or(encrypted)
                        } else {
                            encrypted
                        }
                    }
                    None => crate::oauth::default_microsoft_client_id().to_string(),
                }
            })
        })
        .await?
    };

    let oauth = crate::oauth::authorize_with_provider(
        &app,
        &GenericOAuthProvider::microsoft_graph(resolved_client_id.clone()),
    )
    .await?;
    if !oauth.user_info.email.eq_ignore_ascii_case(&expected_email) {
        return Err(format!(
            "Signed in as {}, but expected {}. Please sign in with the correct account.",
            oauth.user_info.email, expected_email
        ));
    }

    let access_token = encrypt_value(&encryption_key, &oauth.tokens.access_token)?;
    let refresh_token = encrypt_value(
        &encryption_key,
        oauth
            .tokens
            .refresh_token
            .as_deref()
            .ok_or("Microsoft did not return a refresh token")?,
    )?;
    let expires_at = chrono::Utc::now().timestamp()
        + i64::try_from(oauth.tokens.expires_in).map_err(|_| "Microsoft token expiry overflow")?;

    let new_encrypted_cid = if client_id.is_some() {
        Some(encrypt_value(&encryption_key, &resolved_client_id)?)
    } else {
        None
    };

    db.with_conn({
        let id = account_id.clone();
        move |conn| {
            if let Some(enc_cid) = new_encrypted_cid {
                conn.execute(
                    "UPDATE accounts SET access_token = ?1, refresh_token = ?2, \
                     token_expires_at = ?3, oauth_client_id = ?4, \
                     updated_at = unixepoch() WHERE id = ?5",
                    rusqlite::params![access_token, refresh_token, expires_at, enc_cid, id],
                )
                .map_err(|e| format!("Failed to update Graph account tokens: {e}"))?;
            } else {
                conn.execute(
                    "UPDATE accounts SET access_token = ?1, refresh_token = ?2, \
                     token_expires_at = ?3, updated_at = unixepoch() WHERE id = ?4",
                    rusqlite::params![access_token, refresh_token, expires_at, id],
                )
                .map_err(|e| format!("Failed to update Graph account tokens: {e}"))?;
            }
            Ok(())
        }
    })
    .await?;

    graph.remove(&account_id).await;
    let client = GraphClient::from_account(&db, &account_id, encryption_key).await?;
    graph.insert(account_id, client).await;
    Ok(())
}
