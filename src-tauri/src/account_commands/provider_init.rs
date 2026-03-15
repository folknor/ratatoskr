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

use ratatoskr_core::account::provider_init;

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
        .with_conn(move |conn| provider_init::check_gmail_duplicate(conn, &email_for_check))
        .await?;
    if let Some(existing_id) = duplicate {
        return Err(format!(
            "A Gmail account for {} already exists (id: {existing_id})",
            oauth.user_info.email
        ));
    }

    let encryption_key = *gmail.encryption_key();
    let account_id = uuid::Uuid::new_v4().to_string();
    let (access_token, refresh_token, expires_at) = provider_init::encrypt_oauth_tokens(
        &encryption_key,
        &oauth.tokens.access_token,
        oauth
            .tokens
            .refresh_token
            .as_deref()
            .ok_or("Google did not return a refresh token")?,
        oauth.tokens.expires_in,
    )?;

    let encrypted_client_id = encrypt_value(&encryption_key, &client_id)?;
    let encrypted_client_secret = if secret.is_empty() {
        None
    } else {
        Some(encrypt_value(&encryption_key, &secret)?)
    };

    let params = provider_init::InsertGmailAccountParams {
        account_id: account_id.clone(),
        email: oauth.user_info.email.clone(),
        display_name: Some(oauth.user_info.name.clone()),
        avatar_url: oauth.user_info.picture.clone(),
        access_token,
        refresh_token,
        expires_at,
        encrypted_client_id,
        encrypted_client_secret,
    };
    db.with_conn(move |conn| provider_init::insert_gmail_account(conn, &params))
        .await?;

    let client = GmailClient::from_account(&db, &account_id, encryption_key).await?;
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

    let encryption_key = *crypto.encryption_key();
    let (access_token, refresh_token, expires_at) = provider_init::encrypt_oauth_tokens(
        &encryption_key,
        &authorization.tokens.access_token,
        authorization
            .tokens
            .refresh_token
            .as_deref()
            .ok_or_else(|| "OAuth provider did not return a refresh token".to_string())?,
        authorization.tokens.expires_in,
    )?;
    let oauth_client_secret = request
        .oauth_client_secret
        .as_deref()
        .filter(|secret| !secret.is_empty())
        .map(|secret| encrypt_value(&encryption_key, secret))
        .transpose()?;

    let account_id = uuid::Uuid::new_v4().to_string();
    let params = provider_init::InsertImapOAuthAccountParams {
        account_id: account_id.clone(),
        email: request.email.clone(),
        display_name: request.display_name.clone(),
        avatar_url: request.avatar_url.clone(),
        access_token,
        refresh_token,
        expires_at,
        imap_host: request.imap_host,
        imap_port: request.imap_port,
        imap_security: request.imap_security,
        smtp_host: request.smtp_host,
        smtp_port: request.smtp_port,
        smtp_security: request.smtp_security,
        oauth_provider: request.oauth_provider,
        oauth_client_id: request.oauth_client_id,
        oauth_client_secret,
        oauth_token_url: request.oauth_token_url,
        imap_username: request.imap_username,
        accept_invalid_certs: request.accept_invalid_certs,
    };
    db.with_conn(move |conn| provider_init::insert_imap_oauth_account(conn, &params))
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

    let encryption_key = *graph.encryption_key();
    let account_id = uuid::Uuid::new_v4().to_string();
    let (access_token, refresh_token, expires_at) = provider_init::encrypt_oauth_tokens(
        &encryption_key,
        &oauth.tokens.access_token,
        oauth
            .tokens
            .refresh_token
            .as_deref()
            .ok_or("Microsoft did not return a refresh token")?,
        oauth.tokens.expires_in,
    )?;

    let actual_client_id =
        client_id.unwrap_or_else(|| crate::oauth::default_microsoft_client_id().to_string());
    let encrypted_client_id = encrypt_value(&encryption_key, &actual_client_id)?;

    let graph_params = provider_init::InsertGraphAccountParams {
        account_id: account_id.clone(),
        email: oauth.user_info.email.clone(),
        display_name: Some(oauth.user_info.name.clone()),
        access_token,
        refresh_token,
        expires_at,
        encrypted_client_id,
    };
    db.with_conn(move |conn| provider_init::insert_graph_account(conn, &graph_params))
        .await?;

    let init_result = async {
        let client = GraphClient::from_account(&db, &account_id, encryption_key).await?;
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
                    ratatoskr_core::account::delete::delete_account_row(conn, &cleanup_id)
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
        move |conn| provider_init::finalize_graph_profile(conn, &id, &email, &display_name)
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

    let (resolved_client_id, resolved_client_secret) = if let Some(ref cid) = client_id {
        (cid.clone(), client_secret.clone().unwrap_or_default())
    } else {
        let aid = account_id.clone();
        db.with_conn(move |conn| {
            provider_init::resolve_gmail_reauth_credentials(conn, &aid, &encryption_key)
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

    let refresh_raw = oauth
        .tokens
        .refresh_token
        .ok_or("Google did not return a refresh token. Please revoke app access and try again.")?;
    let (access_token, refresh_token, expires_at) = provider_init::encrypt_oauth_tokens(
        &encryption_key,
        &oauth.tokens.access_token,
        &refresh_raw,
        oauth.tokens.expires_in,
    )?;

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
            provider_init::update_gmail_reauth_tokens(
                conn,
                &id,
                &access_token,
                &refresh_token,
                expires_at,
                new_encrypted_cid.as_deref(),
                new_encrypted_cs.as_deref(),
            )
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
        let default_cid = crate::oauth::default_microsoft_client_id().to_string();
        db.with_conn(move |conn| {
            provider_init::resolve_graph_reauth_client_id(
                conn,
                &aid,
                &encryption_key,
                &default_cid,
            )
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

    let (access_token, refresh_token, expires_at) = provider_init::encrypt_oauth_tokens(
        &encryption_key,
        &oauth.tokens.access_token,
        oauth
            .tokens
            .refresh_token
            .as_deref()
            .ok_or("Microsoft did not return a refresh token")?,
        oauth.tokens.expires_in,
    )?;

    let new_encrypted_cid = if client_id.is_some() {
        Some(encrypt_value(&encryption_key, &resolved_client_id)?)
    } else {
        None
    };

    db.with_conn({
        let id = account_id.clone();
        move |conn| {
            provider_init::update_graph_reauth_tokens(
                conn,
                &id,
                &access_token,
                &refresh_token,
                expires_at,
                new_encrypted_cid.as_deref(),
            )
        }
    })
    .await?;

    graph.remove(&account_id).await;
    let client = GraphClient::from_account(&db, &account_id, encryption_key).await?;
    graph.insert(account_id, client).await;
    Ok(())
}
