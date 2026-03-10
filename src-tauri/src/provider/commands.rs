#![allow(clippy::let_underscore_must_use)]

use tauri::{AppHandle, Manager, State};

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
    // 1. Check cache
    if let Some(hit) = try_cache_hit(&db, &app_handle, &account_id, &message_id, &attachment_id).await? {
        return Ok(hit);
    }

    // 2. Cache miss — fetch from provider
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
    let result = ops
        .fetch_attachment(&ctx, &message_id, &attachment_id)
        .await?;

    // 3. Cache the result (fire-and-forget — don't delay response)
    cache_after_fetch(&db, &app_handle, &account_id, &message_id, &attachment_id, &result.data);

    Ok(result)
}

/// Check the content-addressed cache for a previously fetched attachment.
async fn try_cache_hit(
    db: &DbState,
    app_handle: &AppHandle,
    account_id: &str,
    message_id: &str,
    attachment_id: &str,
) -> Result<Option<AttachmentData>, String> {
    use crate::attachment_cache::{encode_base64, find_cache_info, read_cached};

    let app = app_handle.clone();
    let (acct, msg, att) = (
        account_id.to_string(),
        message_id.to_string(),
        attachment_id.to_string(),
    );

    db.with_conn(move |conn| {
        let info = find_cache_info(conn, &acct, &msg, &att)?;
        let Some(info) = info else { return Ok(None) };
        let Some(ref hash) = info.content_hash else {
            return Ok(None);
        };

        if let Some(bytes) = read_cached(&app, hash) {
            let size = bytes.len();
            let data = encode_base64(&bytes);
            return Ok(Some(AttachmentData { data, size }));
        }

        Ok(None)
    })
    .await
}

/// After a provider fetch, decode + hash + write to cache + update DB.
fn cache_after_fetch(
    _db: &DbState,
    app_handle: &AppHandle,
    account_id: &str,
    message_id: &str,
    attachment_id: &str,
    base64_data: &str,
) {
    use crate::attachment_cache::{
        decode_base64, find_cache_info, hash_bytes, update_cache_fields, write_cached,
    };

    let app = app_handle.clone();
    let (acct, msg, att, data) = (
        account_id.to_string(),
        message_id.to_string(),
        attachment_id.to_string(),
        base64_data.to_string(),
    );

    tokio::task::spawn(async move {
        let result: Result<(), String> = async {
            let bytes = decode_base64(&data)?;
            let content_hash = hash_bytes(&bytes);
            let local_path = write_cached(&app, &content_hash, &bytes)?;

            #[allow(clippy::cast_possible_wrap)]
            let cache_size = bytes.len() as i64;

            let db: tauri::State<'_, DbState> = app.state();
            db.with_conn(move |conn| {
                let info = find_cache_info(conn, &acct, &msg, &att)?;
                if let Some(info) = info {
                    update_cache_fields(
                        conn, &info.id, &local_path, cache_size, &content_hash,
                    )?;
                }
                Ok(())
            })
            .await
        }
        .await;

        if let Err(e) = result {
            log::warn!("Failed to cache attachment: {e}");
        }
    });
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
