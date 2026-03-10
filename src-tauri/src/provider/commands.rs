#![allow(clippy::let_underscore_must_use)]

use tauri::{AppHandle, State};

use crate::body_store::BodyStoreState;
use crate::db::DbState;
use crate::gmail::client::GmailState;
use crate::graph::client::GraphState;
use crate::jmap::client::JmapState;
use crate::search::SearchState;

use super::router::{get_ops, get_provider_type};
use super::types::{AttachmentData, ProviderCtx, ProviderFolder, SyncResult};

// ── Sync ────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provider_sync_initial(
    account_id: String,
    days_back: Option<i64>,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let provider = get_provider_type(&db, &account_id).await?;
    let ops = get_ops(
        &provider,
        &account_id,
        &gmail,
        &jmap,
        &graph,
        *gmail.encryption_key(),
    )
    .await?;
    let ctx = ProviderCtx {
        account_id: &account_id,
        db: &db,
        body_store: &body_store,
        search: &search,
        app_handle: &app_handle,
    };
    ops.sync_initial(&ctx, days_back.unwrap_or(365)).await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provider_sync_delta(
    account_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<SyncResult, String> {
    let provider = get_provider_type(&db, &account_id).await?;
    let ops = get_ops(
        &provider,
        &account_id,
        &gmail,
        &jmap,
        &graph,
        *gmail.encryption_key(),
    )
    .await?;
    let ctx = ProviderCtx {
        account_id: &account_id,
        db: &db,
        body_store: &body_store,
        search: &search,
        app_handle: &app_handle,
    };
    ops.sync_delta(&ctx).await
}

// ── Actions (thread-level) ──────────────────────────────────

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provider_archive(
    account_id: String,
    thread_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let provider = get_provider_type(&db, &account_id).await?;
    let ops = get_ops(
        &provider,
        &account_id,
        &gmail,
        &jmap,
        &graph,
        *gmail.encryption_key(),
    )
    .await?;
    let ctx = ProviderCtx {
        account_id: &account_id,
        db: &db,
        body_store: &body_store,
        search: &search,
        app_handle: &app_handle,
    };
    ops.archive(&ctx, &thread_id).await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provider_trash(
    account_id: String,
    thread_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let provider = get_provider_type(&db, &account_id).await?;
    let ops = get_ops(
        &provider,
        &account_id,
        &gmail,
        &jmap,
        &graph,
        *gmail.encryption_key(),
    )
    .await?;
    let ctx = ProviderCtx {
        account_id: &account_id,
        db: &db,
        body_store: &body_store,
        search: &search,
        app_handle: &app_handle,
    };
    ops.trash(&ctx, &thread_id).await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provider_permanent_delete(
    account_id: String,
    thread_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let provider = get_provider_type(&db, &account_id).await?;
    let ops = get_ops(
        &provider,
        &account_id,
        &gmail,
        &jmap,
        &graph,
        *gmail.encryption_key(),
    )
    .await?;
    let ctx = ProviderCtx {
        account_id: &account_id,
        db: &db,
        body_store: &body_store,
        search: &search,
        app_handle: &app_handle,
    };
    ops.permanent_delete(&ctx, &thread_id).await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provider_mark_read(
    account_id: String,
    thread_id: String,
    read: bool,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let provider = get_provider_type(&db, &account_id).await?;
    let ops = get_ops(
        &provider,
        &account_id,
        &gmail,
        &jmap,
        &graph,
        *gmail.encryption_key(),
    )
    .await?;
    let ctx = ProviderCtx {
        account_id: &account_id,
        db: &db,
        body_store: &body_store,
        search: &search,
        app_handle: &app_handle,
    };
    ops.mark_read(&ctx, &thread_id, read).await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provider_star(
    account_id: String,
    thread_id: String,
    starred: bool,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let provider = get_provider_type(&db, &account_id).await?;
    let ops = get_ops(
        &provider,
        &account_id,
        &gmail,
        &jmap,
        &graph,
        *gmail.encryption_key(),
    )
    .await?;
    let ctx = ProviderCtx {
        account_id: &account_id,
        db: &db,
        body_store: &body_store,
        search: &search,
        app_handle: &app_handle,
    };
    ops.star(&ctx, &thread_id, starred).await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provider_spam(
    account_id: String,
    thread_id: String,
    is_spam: bool,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let provider = get_provider_type(&db, &account_id).await?;
    let ops = get_ops(
        &provider,
        &account_id,
        &gmail,
        &jmap,
        &graph,
        *gmail.encryption_key(),
    )
    .await?;
    let ctx = ProviderCtx {
        account_id: &account_id,
        db: &db,
        body_store: &body_store,
        search: &search,
        app_handle: &app_handle,
    };
    ops.spam(&ctx, &thread_id, is_spam).await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provider_move_to_folder(
    account_id: String,
    thread_id: String,
    folder_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let provider = get_provider_type(&db, &account_id).await?;
    let ops = get_ops(
        &provider,
        &account_id,
        &gmail,
        &jmap,
        &graph,
        *gmail.encryption_key(),
    )
    .await?;
    let ctx = ProviderCtx {
        account_id: &account_id,
        db: &db,
        body_store: &body_store,
        search: &search,
        app_handle: &app_handle,
    };
    ops.move_to_folder(&ctx, &thread_id, &folder_id).await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provider_add_tag(
    account_id: String,
    thread_id: String,
    tag_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let provider = get_provider_type(&db, &account_id).await?;
    let ops = get_ops(
        &provider,
        &account_id,
        &gmail,
        &jmap,
        &graph,
        *gmail.encryption_key(),
    )
    .await?;
    let ctx = ProviderCtx {
        account_id: &account_id,
        db: &db,
        body_store: &body_store,
        search: &search,
        app_handle: &app_handle,
    };
    ops.add_tag(&ctx, &thread_id, &tag_id).await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provider_remove_tag(
    account_id: String,
    thread_id: String,
    tag_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let provider = get_provider_type(&db, &account_id).await?;
    let ops = get_ops(
        &provider,
        &account_id,
        &gmail,
        &jmap,
        &graph,
        *gmail.encryption_key(),
    )
    .await?;
    let ctx = ProviderCtx {
        account_id: &account_id,
        db: &db,
        body_store: &body_store,
        search: &search,
        app_handle: &app_handle,
    };
    ops.remove_tag(&ctx, &thread_id, &tag_id).await
}

// ── Send + Drafts ───────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provider_send_email(
    account_id: String,
    raw_base64url: String,
    thread_id: Option<String>,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<String, String> {
    let provider = get_provider_type(&db, &account_id).await?;
    let ops = get_ops(
        &provider,
        &account_id,
        &gmail,
        &jmap,
        &graph,
        *gmail.encryption_key(),
    )
    .await?;
    let ctx = ProviderCtx {
        account_id: &account_id,
        db: &db,
        body_store: &body_store,
        search: &search,
        app_handle: &app_handle,
    };
    ops.send_email(&ctx, &raw_base64url, thread_id.as_deref())
        .await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provider_create_draft(
    account_id: String,
    raw_base64url: String,
    thread_id: Option<String>,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<String, String> {
    let provider = get_provider_type(&db, &account_id).await?;
    let ops = get_ops(
        &provider,
        &account_id,
        &gmail,
        &jmap,
        &graph,
        *gmail.encryption_key(),
    )
    .await?;
    let ctx = ProviderCtx {
        account_id: &account_id,
        db: &db,
        body_store: &body_store,
        search: &search,
        app_handle: &app_handle,
    };
    ops.create_draft(&ctx, &raw_base64url, thread_id.as_deref())
        .await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provider_update_draft(
    account_id: String,
    draft_id: String,
    raw_base64url: String,
    thread_id: Option<String>,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<String, String> {
    let provider = get_provider_type(&db, &account_id).await?;
    let ops = get_ops(
        &provider,
        &account_id,
        &gmail,
        &jmap,
        &graph,
        *gmail.encryption_key(),
    )
    .await?;
    let ctx = ProviderCtx {
        account_id: &account_id,
        db: &db,
        body_store: &body_store,
        search: &search,
        app_handle: &app_handle,
    };
    ops.update_draft(&ctx, &draft_id, &raw_base64url, thread_id.as_deref())
        .await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provider_delete_draft(
    account_id: String,
    draft_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let provider = get_provider_type(&db, &account_id).await?;
    let ops = get_ops(
        &provider,
        &account_id,
        &gmail,
        &jmap,
        &graph,
        *gmail.encryption_key(),
    )
    .await?;
    let ctx = ProviderCtx {
        account_id: &account_id,
        db: &db,
        body_store: &body_store,
        search: &search,
        app_handle: &app_handle,
    };
    ops.delete_draft(&ctx, &draft_id).await
}

// ── Attachments ─────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provider_fetch_attachment(
    account_id: String,
    message_id: String,
    attachment_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<AttachmentData, String> {
    let provider = get_provider_type(&db, &account_id).await?;
    let ops = get_ops(
        &provider,
        &account_id,
        &gmail,
        &jmap,
        &graph,
        *gmail.encryption_key(),
    )
    .await?;
    let ctx = ProviderCtx {
        account_id: &account_id,
        db: &db,
        body_store: &body_store,
        search: &search,
        app_handle: &app_handle,
    };
    ops.fetch_attachment(&ctx, &message_id, &attachment_id)
        .await
}

// ── Folders ─────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provider_list_folders(
    account_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<Vec<ProviderFolder>, String> {
    let provider = get_provider_type(&db, &account_id).await?;
    let ops = get_ops(
        &provider,
        &account_id,
        &gmail,
        &jmap,
        &graph,
        *gmail.encryption_key(),
    )
    .await?;
    let ctx = ProviderCtx {
        account_id: &account_id,
        db: &db,
        body_store: &body_store,
        search: &search,
        app_handle: &app_handle,
    };
    ops.list_folders(&ctx).await
}
