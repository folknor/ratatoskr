use base64::Engine;
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::body_store::BodyStoreState;
use crate::db::DbState;
use crate::search::SearchState;

use super::auto_discovery::{discover_jmap_url, JmapDiscoveryResult};
use super::client::{JmapClient, JmapState};
use super::mailbox_mapper::{label_id_to_mailbox_id, map_mailbox_to_label};
use super::sync::{jmap_delta_sync, jmap_initial_sync, JmapSyncResult};

// ---------------------------------------------------------------------------
// Lifecycle commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn jmap_init_client(
    account_id: String,
    db: State<'_, DbState>,
    jmap: State<'_, JmapState>,
) -> Result<(), String> {
    let client =
        JmapClient::from_account(&db, &account_id, jmap.encryption_key()).await?;
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
    let client = jmap.get(&account_id).await?;
    let session = client.inner().session();
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
pub async fn jmap_discover_url(email: String) -> Result<Option<JmapDiscoveryResult>, String> {
    Ok(discover_jmap_url(&email).await)
}

#[tauri::command]
pub async fn jmap_get_profile(
    account_id: String,
    jmap: State<'_, JmapState>,
) -> Result<JmapProfile, String> {
    let client = jmap.get(&account_id).await?;
    let session = client.inner().session();
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
    search: State<'_, SearchState>,
    jmap: State<'_, JmapState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let client = jmap.get(&account_id).await?;
    let days = days_back.unwrap_or(365);
    jmap_initial_sync(&client, &account_id, days, &db, &body_store, &search, &app_handle).await
}

#[tauri::command]
pub async fn jmap_sync_delta(
    account_id: String,
    db: State<'_, DbState>,
    body_store: State<'_, BodyStoreState>,
    search: State<'_, SearchState>,
    jmap: State<'_, JmapState>,
    app_handle: AppHandle,
) -> Result<JmapSyncResult, String> {
    let client = jmap.get(&account_id).await?;
    jmap_delta_sync(&client, &account_id, &db, &body_store, &search, &app_handle).await
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
        let role_str = if role == Role::None { None } else { Some(super::sync::role_to_str(&role)) };
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
    let client = jmap.get(&account_id).await?;

    use jmap_client::mailbox::Role;
    let mut mb = client
        .inner()
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
    let client = jmap.get(&account_id).await?;
    let mailbox_id = resolve_mailbox_id(&client, &folder_id).await?;
    client
        .inner()
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
    let client = jmap.get(&account_id).await?;
    let mailbox_id = resolve_mailbox_id(&client, &folder_id).await?;
    client
        .inner()
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
    let client = jmap.get(&account_id).await?;

    let data = client
        .inner()
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

/// Get all email IDs in a JMAP thread.
pub(crate) async fn query_thread_email_ids(
    client: &JmapClient,
    thread_id: &str,
) -> Result<Vec<String>, String> {
    use jmap_client::email;

    let filter: jmap_client::core::query::Filter<email::query::Filter> =
        email::query::Filter::in_thread(thread_id).into();
    let result = client
        .inner()
        .email_query(
            Some(filter),
            None::<Vec<_>>,
        )
        .await
        .map_err(|e| format!("Email/query inThread: {e}"))?;

    Ok(result.ids().to_vec())
}

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
        let role_str = if role == Role::None { None } else { Some(super::sync::role_to_str(&role).to_string()) };
        result.push((id.to_string(), role_str, name.to_string()));
    }
    Ok(result)
}

/// Get the first identity ID for email submission.
pub(crate) async fn get_first_identity_id(
    client: &jmap_client::client::Client,
) -> Result<String, String> {
    let mut request = client.build();
    request.get_identity();
    let response = request
        .send()
        .await
        .map_err(|e| format!("Identity/get: {e}"))?;

    response
        .unwrap_method_responses()
        .pop()
        .and_then(|r| r.unwrap_get_identity().ok())
        .and_then(|mut r| {
            r.take_list()
                .into_iter()
                .next()
                .and_then(|mut i| Some(i.take_id()))
        })
        .ok_or_else(|| "No identity found for email submission".to_string())
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
