#![allow(clippy::let_underscore_must_use)]

use base64::Engine;
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::body_store::BodyStoreState;
use crate::db::DbState;
use crate::progress::TauriProgressReporter;
use crate::search::SearchState;

use super::client::{JmapClient, JmapState};
use super::mailbox_mapper::{label_id_to_mailbox_id, map_mailbox_to_label};
use super::sync::{JmapSyncResult, jmap_delta_sync, jmap_initial_sync};

// ---------------------------------------------------------------------------
// Lifecycle commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn jmap_init_client(
    account_id: String,
    db: State<'_, DbState>,
    jmap: State<'_, JmapState>,
) -> Result<(), String> {
    let client = JmapClient::from_account(&db, &account_id, jmap.encryption_key()).await?;
    jmap.insert(account_id, client).await;
    Ok(())
}

#[tauri::command]
pub async fn jmap_remove_client(
    account_id: String,
    jmap: State<'_, JmapState>,
) -> Result<(), String> {
    jmap.remove(&account_id).await;
    Ok(())
}

#[tauri::command]
pub async fn jmap_test_connection(
    account_id: String,
    jmap: State<'_, JmapState>,
) -> Result<JmapTestResult, String> {
    let jmap_client = jmap.get(&account_id).await?;
    let inner = jmap_client.inner();
    let session = inner.session();
    Ok(JmapTestResult {
        success: true,
        message: format!("Connected as {}", session.username()),
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JmapTestResult {
    pub success: bool,
    pub message: String,
}

#[tauri::command]
pub async fn jmap_get_profile(
    account_id: String,
    jmap: State<'_, JmapState>,
) -> Result<JmapProfile, String> {
    let jmap_client = jmap.get(&account_id).await?;
    let inner = jmap_client.inner();
    let session = inner.session();
    Ok(JmapProfile {
        email: session.username().to_string(),
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JmapProfile {
    pub email: String,
}

// ---------------------------------------------------------------------------
// Sync commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn jmap_sync_initial(
    account_id: String,
    days_back: Option<i64>,
    db: State<'_, DbState>,
    body_store: State<'_, BodyStoreState>,
    inline_images: State<'_, crate::inline_image_store::InlineImageStoreState>,
    search: State<'_, SearchState>,
    jmap: State<'_, JmapState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let client = jmap.get(&account_id).await?;
    let days = days_back.unwrap_or(365);
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    jmap_initial_sync(
        &client,
        &account_id,
        days,
        &db,
        &body_store,
        &inline_images,
        &search,
        &reporter,
    )
    .await
}

#[tauri::command]
pub async fn jmap_sync_delta(
    account_id: String,
    db: State<'_, DbState>,
    body_store: State<'_, BodyStoreState>,
    inline_images: State<'_, crate::inline_image_store::InlineImageStoreState>,
    search: State<'_, SearchState>,
    jmap: State<'_, JmapState>,
    app_handle: AppHandle,
) -> Result<JmapSyncResult, String> {
    let client = jmap.get(&account_id).await?;
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    jmap_delta_sync(
        &client,
        &account_id,
        &db,
        &body_store,
        &inline_images,
        &search,
        &reporter,
    )
    .await
}

// ---------------------------------------------------------------------------
// Folder (mailbox) commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn jmap_list_folders(
    account_id: String,
    jmap: State<'_, JmapState>,
) -> Result<Vec<JmapFolder>, String> {
    let client = jmap.get(&account_id).await?;

    use jmap_client::mailbox::Role;
    let mailboxes = super::sync::fetch_all_mailboxes(&client).await?;

    let mut folders = Vec::new();
    for mb in &mailboxes {
        let Some(id) = mb.id() else { continue };
        let name = mb.name().unwrap_or("(unnamed)");
        let role = mb.role();
        let role_str = if role == Role::None {
            None
        } else {
            Some(super::sync::role_to_str(&role))
        };
        let mapping = map_mailbox_to_label(role_str, id, name);

        folders.push(JmapFolder {
            id: mapping.label_id,
            name: mapping.label_name,
            path: name.to_string(),
            folder_type: mapping.label_type.to_string(),
            special_use: role_str.map(String::from),
            message_count: mb.total_emails(),
            unread_count: mb.unread_emails(),
        });
    }

    Ok(folders)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JmapFolder {
    pub id: String,
    pub name: String,
    pub path: String,
    pub folder_type: String,
    pub special_use: Option<String>,
    pub message_count: usize,
    pub unread_count: usize,
}

#[tauri::command]
pub async fn jmap_create_folder(
    account_id: String,
    name: String,
    parent_id: Option<String>,
    jmap: State<'_, JmapState>,
) -> Result<JmapFolder, String> {
    let jmap_client = jmap.get(&account_id).await?;

    use jmap_client::mailbox::Role;
    let inner = jmap_client.inner();
    let mut mb = inner
        .mailbox_create(&name, parent_id, Role::None)
        .await
        .map_err(|e| format!("Mailbox/set create: {e}"))?;

    let id = mb.take_id();
    Ok(JmapFolder {
        id: format!("jmap-{id}"),
        name: name.clone(),
        path: name,
        folder_type: "user".to_string(),
        special_use: None,
        message_count: 0,
        unread_count: 0,
    })
}

#[tauri::command]
pub async fn jmap_rename_folder(
    account_id: String,
    folder_id: String,
    new_name: String,
    jmap: State<'_, JmapState>,
) -> Result<(), String> {
    let jmap_client = jmap.get(&account_id).await?;
    let mailbox_id = resolve_mailbox_id(&jmap_client, &folder_id).await?;
    let inner = jmap_client.inner();
    inner
        .mailbox_rename(&mailbox_id, &new_name)
        .await
        .map_err(|e| format!("Mailbox/set rename: {e}"))?;
    Ok(())
}

#[tauri::command]
pub async fn jmap_delete_folder(
    account_id: String,
    folder_id: String,
    jmap: State<'_, JmapState>,
) -> Result<(), String> {
    let jmap_client = jmap.get(&account_id).await?;
    let mailbox_id = resolve_mailbox_id(&jmap_client, &folder_id).await?;
    let inner = jmap_client.inner();
    inner
        .mailbox_destroy(&mailbox_id, true)
        .await
        .map_err(|e| format!("Mailbox/set destroy: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Attachment command
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn jmap_fetch_attachment(
    account_id: String,
    _email_id: String,
    blob_id: String,
    jmap: State<'_, JmapState>,
) -> Result<JmapAttachmentData, String> {
    let jmap_client = jmap.get(&account_id).await?;

    let inner = jmap_client.inner();
    let data = inner
        .download(&blob_id)
        .await
        .map_err(|e| format!("Blob download: {e}"))?;

    Ok(JmapAttachmentData {
        data: base64::engine::general_purpose::STANDARD.encode(&data),
        size: data.len(),
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JmapAttachmentData {
    pub data: String,
    pub size: usize,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Get the full mailbox list as (id, role, name) tuples.
pub(crate) async fn get_mailbox_list(
    client: &JmapClient,
) -> Result<Vec<(String, Option<String>, String)>, String> {
    use jmap_client::mailbox::Role;

    let mailboxes = super::sync::fetch_all_mailboxes(client).await?;

    let mut result = Vec::new();
    for mb in &mailboxes {
        let Some(id) = mb.id() else { continue };
        let name = mb.name().unwrap_or("(unnamed)");
        let role = mb.role();
        let role_str = if role == Role::None {
            None
        } else {
            Some(super::sync::role_to_str(&role).to_string())
        };
        result.push((id.to_string(), role_str, name.to_string()));
    }
    Ok(result)
}

/// Resolve a Gmail-style label ID to a JMAP mailbox ID.
pub(crate) async fn resolve_mailbox_id(
    client: &JmapClient,
    label_id: &str,
) -> Result<String, String> {
    let mailboxes = get_mailbox_list(client).await?;
    label_id_to_mailbox_id(label_id, &mailboxes)
        .ok_or_else(|| format!("Cannot resolve label \"{label_id}\" to JMAP mailbox"))
}
