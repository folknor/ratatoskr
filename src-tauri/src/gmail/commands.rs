use tauri::{AppHandle, State};

use crate::body_store::BodyStoreState;
use crate::db::DbState;
use crate::search::SearchState;

use super::client::GmailState;
use super::parse::{parse_gmail_message, ParsedGmailMessage};
use super::sync::GmailSyncResult;
use super::types::{
    GmailAttachmentData, GmailDraft, GmailDraftStub, GmailHistoryResponse, GmailLabel,
    GmailMessage, GmailProfile, GmailSendAs, GmailThread, GmailThreadStub,
};

// ── Lifecycle ───────────────────────────────────────────────

/// Initialize a Gmail client for the given account.
/// Reads encrypted tokens from the database. Call after OAuth completes or on app startup.
#[tauri::command]
pub async fn gmail_init_client(
    account_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<(), String> {
    let client =
        super::client::GmailClient::from_account(&db, &account_id, *gmail.encryption_key())
            .await?;
    gmail.insert(account_id, client).await;
    Ok(())
}

/// Return a fresh access token for the given account, refreshing if needed.
/// Used by the TS calendar provider for direct Google Calendar API calls.
#[tauri::command]
pub async fn gmail_get_access_token(
    account_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<String, String> {
    let client = gmail.get(&account_id).await?;
    client.get_access_token(&db).await
}

/// Force-refresh the access token for the given account (e.g. after a 401).
#[tauri::command]
pub async fn gmail_force_refresh_token(
    account_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<String, String> {
    let client = gmail.get(&account_id).await?;
    client.force_refresh_token(&db).await
}

/// Remove the Gmail client for the given account (on deletion or re-auth).
#[tauri::command]
pub async fn gmail_remove_client(
    account_id: String,
    gmail: State<'_, GmailState>,
) -> Result<(), String> {
    gmail.remove(&account_id).await;
    Ok(())
}

/// Test that the Gmail client can connect and authenticate.
#[tauri::command]
pub async fn gmail_test_connection(
    account_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<GmailProfile, String> {
    let client = gmail.get(&account_id).await?;
    client.get_profile(&db).await
}

// ── Labels ──────────────────────────────────────────────────

#[tauri::command]
pub async fn gmail_list_labels(
    account_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<Vec<GmailLabel>, String> {
    let client = gmail.get(&account_id).await?;
    client.list_labels(&db).await
}

#[tauri::command]
pub async fn gmail_create_label(
    account_id: String,
    name: String,
    text_color: Option<String>,
    bg_color: Option<String>,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<GmailLabel, String> {
    let client = gmail.get(&account_id).await?;
    let color = match (&text_color, &bg_color) {
        (Some(tc), Some(bc)) => Some((tc.as_str(), bc.as_str())),
        _ => None,
    };
    client.create_label(&name, color, &db).await
}

#[tauri::command]
pub async fn gmail_update_label(
    account_id: String,
    label_id: String,
    name: Option<String>,
    text_color: Option<String>,
    bg_color: Option<String>,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<GmailLabel, String> {
    let client = gmail.get(&account_id).await?;
    let color = match (&text_color, &bg_color) {
        (Some(tc), Some(bc)) => Some(Some((tc.as_str(), bc.as_str()))),
        _ => None,
    };
    client
        .update_label(&label_id, name.as_deref(), color, &db)
        .await
}

#[tauri::command]
pub async fn gmail_delete_label(
    account_id: String,
    label_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<(), String> {
    let client = gmail.get(&account_id).await?;
    client.delete_label(&label_id, &db).await
}

// ── Threads ─────────────────────────────────────────────────

#[tauri::command]
pub async fn gmail_list_threads(
    account_id: String,
    query: Option<String>,
    max_results: Option<u32>,
    page_token: Option<String>,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<(Vec<GmailThreadStub>, Option<String>), String> {
    let client = gmail.get(&account_id).await?;
    client
        .list_threads(
            query.as_deref(),
            max_results,
            page_token.as_deref(),
            &db,
        )
        .await
}

#[tauri::command]
pub async fn gmail_get_thread(
    account_id: String,
    thread_id: String,
    format: Option<String>,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<GmailThread, String> {
    let client = gmail.get(&account_id).await?;
    let fmt = format.as_deref().unwrap_or("full");
    client.get_thread(&thread_id, fmt, &db).await
}

#[tauri::command]
pub async fn gmail_modify_thread(
    account_id: String,
    thread_id: String,
    add_labels: Vec<String>,
    remove_labels: Vec<String>,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<GmailThread, String> {
    let client = gmail.get(&account_id).await?;
    client
        .modify_thread(&thread_id, &add_labels, &remove_labels, &db)
        .await
}

#[tauri::command]
pub async fn gmail_delete_thread(
    account_id: String,
    thread_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<(), String> {
    let client = gmail.get(&account_id).await?;
    client.delete_thread(&thread_id, &db).await
}

// ── Messages ────────────────────────────────────────────────

#[tauri::command]
pub async fn gmail_get_message(
    account_id: String,
    message_id: String,
    format: Option<String>,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<GmailMessage, String> {
    let client = gmail.get(&account_id).await?;
    let fmt = format.as_deref().unwrap_or("full");
    client.get_message(&message_id, fmt, &db).await
}

/// Fetch a message and parse it into the internal format.
#[tauri::command]
pub async fn gmail_get_parsed_message(
    account_id: String,
    message_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<ParsedGmailMessage, String> {
    let client = gmail.get(&account_id).await?;
    let msg = client.get_message(&message_id, "full", &db).await?;
    Ok(parse_gmail_message(&msg))
}

#[tauri::command]
pub async fn gmail_send_email(
    account_id: String,
    raw: String,
    thread_id: Option<String>,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<GmailMessage, String> {
    let client = gmail.get(&account_id).await?;
    client
        .send_message(&raw, thread_id.as_deref(), &db)
        .await
}

#[tauri::command]
pub async fn gmail_fetch_attachment(
    account_id: String,
    message_id: String,
    attachment_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<GmailAttachmentData, String> {
    let client = gmail.get(&account_id).await?;
    client.get_attachment(&message_id, &attachment_id, &db).await
}

// ── History ─────────────────────────────────────────────────

#[tauri::command]
pub async fn gmail_get_history(
    account_id: String,
    start_history_id: String,
    page_token: Option<String>,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<GmailHistoryResponse, String> {
    let client = gmail.get(&account_id).await?;
    client
        .get_history(&start_history_id, page_token.as_deref(), &db)
        .await
}

// ── Drafts ──────────────────────────────────────────────────

#[tauri::command]
pub async fn gmail_create_draft(
    account_id: String,
    raw: String,
    thread_id: Option<String>,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<GmailDraft, String> {
    let client = gmail.get(&account_id).await?;
    client
        .create_draft(&raw, thread_id.as_deref(), &db)
        .await
}

#[tauri::command]
pub async fn gmail_update_draft(
    account_id: String,
    draft_id: String,
    raw: String,
    thread_id: Option<String>,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<GmailDraft, String> {
    let client = gmail.get(&account_id).await?;
    client
        .update_draft(&draft_id, &raw, thread_id.as_deref(), &db)
        .await
}

#[tauri::command]
pub async fn gmail_delete_draft(
    account_id: String,
    draft_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<(), String> {
    let client = gmail.get(&account_id).await?;
    client.delete_draft(&draft_id, &db).await
}

#[tauri::command]
pub async fn gmail_list_drafts(
    account_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<Vec<GmailDraftStub>, String> {
    let client = gmail.get(&account_id).await?;
    client.list_drafts(&db).await
}

// ── Send-as ─────────────────────────────────────────────────

#[tauri::command]
pub async fn gmail_fetch_send_as(
    account_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<Vec<GmailSendAs>, String> {
    let client = gmail.get(&account_id).await?;
    client.list_send_as(&db).await
}

// ── Sync ─────────────────────────────────────────────────────

/// Run initial Gmail sync: labels + thread list + parallel fetch.
#[tauri::command]
pub async fn gmail_sync_initial(
    account_id: String,
    days_back: Option<i64>,
    app: AppHandle,
    db: State<'_, DbState>,
    body_store: State<'_, BodyStoreState>,
    search: State<'_, SearchState>,
    gmail: State<'_, GmailState>,
) -> Result<(), String> {
    let client = gmail.get(&account_id).await?;
    let days = days_back.unwrap_or(365);
    super::sync::gmail_initial_sync(
        &client,
        &account_id,
        days,
        &db,
        &body_store,
        &search,
        &app,
    )
    .await
}

/// Run delta Gmail sync via History API.
#[tauri::command]
pub async fn gmail_sync_delta(
    account_id: String,
    app: AppHandle,
    db: State<'_, DbState>,
    body_store: State<'_, BodyStoreState>,
    search: State<'_, SearchState>,
    gmail: State<'_, GmailState>,
) -> Result<GmailSyncResult, String> {
    let client = gmail.get(&account_id).await?;
    super::sync::gmail_delta_sync(
        &client,
        &account_id,
        &db,
        &body_store,
        &search,
        &app,
    )
    .await
}
