use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::body_store::BodyStoreState;
use crate::db::DbState;
use crate::search::SearchState;

use super::auto_discovery::{discover_jmap_url, JmapDiscoveryResult};
use super::client::{JmapClient, JmapState};
use super::mailbox_mapper::{
    find_mailbox_id_by_role, label_id_to_mailbox_id, map_mailbox_to_label,
};
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
// Email action commands (thread-level, called by TS queue processor)
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn jmap_archive(
    account_id: String,
    thread_id: String,
    jmap: State<'_, JmapState>,
) -> Result<(), String> {
    let client = jmap.get(&account_id).await?;
    let mailboxes = get_mailbox_list(&client).await?;
    let inbox_id = find_mailbox_id_by_role(&mailboxes, "inbox")
        .ok_or("No inbox mailbox found")?;
    let archive_id = find_mailbox_id_by_role(&mailboxes, "archive");

    let email_ids = query_thread_email_ids(&client, &thread_id).await?;
    for eid in &email_ids {
        client.inner().email_set_mailbox(eid, &inbox_id, false).await
            .map_err(|e| format!("archive remove inbox: {e}"))?;
        if let Some(ref aid) = archive_id {
            client.inner().email_set_mailbox(eid, aid, true).await
                .map_err(|e| format!("archive add archive: {e}"))?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn jmap_trash(
    account_id: String,
    thread_id: String,
    jmap: State<'_, JmapState>,
) -> Result<(), String> {
    let client = jmap.get(&account_id).await?;
    let mailboxes = get_mailbox_list(&client).await?;
    let trash_id = find_mailbox_id_by_role(&mailboxes, "trash")
        .ok_or("No trash mailbox found")?;
    let inbox_id = find_mailbox_id_by_role(&mailboxes, "inbox");

    let email_ids = query_thread_email_ids(&client, &thread_id).await?;
    for eid in &email_ids {
        client.inner().email_set_mailbox(eid, &trash_id, true).await
            .map_err(|e| format!("trash add: {e}"))?;
        if let Some(ref iid) = inbox_id {
            client.inner().email_set_mailbox(eid, iid, false).await
                .map_err(|e| format!("trash remove inbox: {e}"))?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn jmap_permanent_delete(
    account_id: String,
    email_ids: Vec<String>,
    jmap: State<'_, JmapState>,
) -> Result<(), String> {
    let client = jmap.get(&account_id).await?;
    for eid in &email_ids {
        client.inner().email_destroy(eid).await
            .map_err(|e| format!("permanent delete: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn jmap_mark_read(
    account_id: String,
    thread_id: String,
    read: bool,
    jmap: State<'_, JmapState>,
) -> Result<(), String> {
    let client = jmap.get(&account_id).await?;
    let email_ids = query_thread_email_ids(&client, &thread_id).await?;
    for eid in &email_ids {
        client.inner().email_set_keyword(eid, "$seen", read).await
            .map_err(|e| format!("mark read: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn jmap_star(
    account_id: String,
    thread_id: String,
    starred: bool,
    jmap: State<'_, JmapState>,
) -> Result<(), String> {
    let client = jmap.get(&account_id).await?;
    let email_ids = query_thread_email_ids(&client, &thread_id).await?;
    for eid in &email_ids {
        client.inner().email_set_keyword(eid, "$flagged", starred).await
            .map_err(|e| format!("star: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn jmap_spam(
    account_id: String,
    thread_id: String,
    is_spam: bool,
    jmap: State<'_, JmapState>,
) -> Result<(), String> {
    let client = jmap.get(&account_id).await?;
    let mailboxes = get_mailbox_list(&client).await?;
    let junk_id = find_mailbox_id_by_role(&mailboxes, "junk")
        .ok_or("No junk/spam mailbox found")?;
    let inbox_id = find_mailbox_id_by_role(&mailboxes, "inbox")
        .ok_or("No inbox mailbox found")?;

    let email_ids = query_thread_email_ids(&client, &thread_id).await?;
    for eid in &email_ids {
        if is_spam {
            client.inner().email_set_mailbox(eid, &junk_id, true).await
                .map_err(|e| format!("spam add junk: {e}"))?;
            client.inner().email_set_mailbox(eid, &inbox_id, false).await
                .map_err(|e| format!("spam remove inbox: {e}"))?;
        } else {
            client.inner().email_set_mailbox(eid, &inbox_id, true).await
                .map_err(|e| format!("not-spam add inbox: {e}"))?;
            client.inner().email_set_mailbox(eid, &junk_id, false).await
                .map_err(|e| format!("not-spam remove junk: {e}"))?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn jmap_move_to_folder(
    account_id: String,
    thread_id: String,
    folder_id: String,
    jmap: State<'_, JmapState>,
) -> Result<(), String> {
    let client = jmap.get(&account_id).await?;
    let target_id = resolve_mailbox_id(&client, &folder_id).await?;

    let email_ids = query_thread_email_ids(&client, &thread_id).await?;
    for eid in &email_ids {
        // Set this as the only mailbox
        client.inner().email_set_mailboxes(eid, vec![target_id.clone()]).await
            .map_err(|e| format!("move to folder: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn jmap_add_label(
    account_id: String,
    thread_id: String,
    label_id: String,
    jmap: State<'_, JmapState>,
) -> Result<(), String> {
    let client = jmap.get(&account_id).await?;
    let mailbox_id = resolve_mailbox_id(&client, &label_id).await?;
    let email_ids = query_thread_email_ids(&client, &thread_id).await?;
    for eid in &email_ids {
        client.inner().email_set_mailbox(eid, &mailbox_id, true).await
            .map_err(|e| format!("add label: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn jmap_remove_label(
    account_id: String,
    thread_id: String,
    label_id: String,
    jmap: State<'_, JmapState>,
) -> Result<(), String> {
    let client = jmap.get(&account_id).await?;
    let mailbox_id = resolve_mailbox_id(&client, &label_id).await?;
    let email_ids = query_thread_email_ids(&client, &thread_id).await?;
    for eid in &email_ids {
        client.inner().email_set_mailbox(eid, &mailbox_id, false).await
            .map_err(|e| format!("remove label: {e}"))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Send + draft commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn jmap_send_email(
    account_id: String,
    raw_base64url: String,
    _thread_id: Option<String>,
    jmap: State<'_, JmapState>,
) -> Result<JmapSendResult, String> {
    let client = jmap.get(&account_id).await?;
    let raw_bytes = URL_SAFE_NO_PAD
        .decode(&raw_base64url)
        .map_err(|e| format!("base64url decode: {e}"))?;

    // Import the message, then create a submission
    let mut email = client
        .inner()
        .email_import(
            raw_bytes,
            Vec::<String>::new(),
            Some(vec!["$seen".to_string()]),
            None,
        )
        .await
        .map_err(|e| format!("Email/import: {e}"))?;

    let email_id = email.take_id();

    // Get the first identity for submission
    let identity_id = get_first_identity_id(client.inner()).await?;

    let _ = client
        .inner()
        .email_submission_create(&email_id, &identity_id)
        .await
        .map_err(|e| format!("EmailSubmission/set: {e}"))?;

    // Remove $draft keyword after successful submission
    let _ = client.inner().email_set_keyword(&email_id, "$draft", false).await;
    let _ = client.inner().email_set_keyword(&email_id, "$seen", true).await;

    Ok(JmapSendResult { id: email_id })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JmapSendResult {
    pub id: String,
}

#[tauri::command]
pub async fn jmap_create_draft(
    account_id: String,
    raw_base64url: String,
    _thread_id: Option<String>,
    jmap: State<'_, JmapState>,
) -> Result<JmapDraftResult, String> {
    let client = jmap.get(&account_id).await?;
    let raw_bytes = URL_SAFE_NO_PAD
        .decode(&raw_base64url)
        .map_err(|e| format!("base64url decode: {e}"))?;

    // Find drafts mailbox
    let mailboxes = get_mailbox_list(&client).await?;
    let drafts_id = find_mailbox_id_by_role(&mailboxes, "drafts")
        .ok_or("No drafts mailbox found")?;

    let mut email = client
        .inner()
        .email_import(
            raw_bytes,
            vec![drafts_id],
            Some(vec!["$draft".to_string(), "$seen".to_string()]),
            None,
        )
        .await
        .map_err(|e| format!("Email/import draft: {e}"))?;

    Ok(JmapDraftResult {
        draft_id: email.take_id(),
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JmapDraftResult {
    pub draft_id: String,
}

#[tauri::command]
pub async fn jmap_update_draft(
    account_id: String,
    draft_id: String,
    raw_base64url: String,
    thread_id: Option<String>,
    jmap: State<'_, JmapState>,
) -> Result<JmapDraftResult, String> {
    // JMAP has no draft mutation — delete old, create new
    let client = jmap.get(&account_id).await?;
    client.inner().email_destroy(&draft_id).await
        .map_err(|e| format!("delete old draft: {e}"))?;
    drop(client);

    jmap_create_draft(account_id, raw_base64url, thread_id, jmap).await
}

#[tauri::command]
pub async fn jmap_delete_draft(
    account_id: String,
    draft_id: String,
    jmap: State<'_, JmapState>,
) -> Result<(), String> {
    let client = jmap.get(&account_id).await?;
    client.inner().email_destroy(&draft_id).await
        .map_err(|e| format!("delete draft: {e}"))?;
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
async fn query_thread_email_ids(
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
async fn get_mailbox_list(
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
async fn get_first_identity_id(client: &jmap_client::client::Client) -> Result<String, String> {
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
async fn resolve_mailbox_id(
    client: &JmapClient,
    label_id: &str,
) -> Result<String, String> {
    let mailboxes = get_mailbox_list(client).await?;
    label_id_to_mailbox_id(label_id, &mailboxes)
        .ok_or_else(|| format!("Cannot resolve label \"{label_id}\" to JMAP mailbox"))
}
