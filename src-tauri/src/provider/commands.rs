#![allow(clippy::let_underscore_must_use)]

use tauri::{AppHandle, Manager, State};

use crate::body_store::BodyStoreState;
use crate::db::DbState;
use crate::gmail::client::GmailState;
use crate::graph::client::GraphState;
use crate::inline_image_store::InlineImageStoreState;
use crate::jmap::client::JmapState;
use crate::progress::{self, ProgressReporter, TauriProgressReporter};
use crate::search::SearchState;
use crate::state::AppState;
use crate::sync::{self, SyncState};

use super::ops::ProviderOps;
use super::registry::ProviderRegistry;
use super::router::{get_ops, get_provider_type};
use super::types::{
    AttachmentData, AutoSyncResult, ProviderCtx, ProviderFolderEntry, ProviderFolderMutation,
    ProviderParsedMessage, ProviderProfile, ProviderTestResult, SyncResult,
};
use crate::sync::types::SyncProgressEvent;

#[allow(clippy::too_many_arguments)]
async fn resolve_provider_command<'a>(
    provider: Option<&str>,
    account_id: &'a str,
    db: &'a DbState,
    gmail: &'a GmailState,
    jmap: &'a JmapState,
    graph: &'a GraphState,
    body_store: &'a BodyStoreState,
    inline_images: &'a InlineImageStoreState,
    search: &'a SearchState,
    progress: &'a dyn ProgressReporter,
) -> Result<(Box<dyn ProviderOps>, ProviderCtx<'a>), String> {
    let provider = match provider {
        Some(provider) => provider.to_string(),
        None => get_provider_type(db, account_id).await?,
    };
    let ops = get_ops(
        &provider,
        account_id,
        gmail,
        jmap,
        graph,
        *gmail.encryption_key(),
    )
    .await?;
    let ctx = ProviderCtx {
        account_id,
        db,
        body_store,
        inline_images,
        search,
        progress,
    };
    Ok((ops, ctx))
}

async fn resolve_provider_command_with_registry<'a>(
    provider: Option<&str>,
    account_id: &'a str,
    registry: &'a dyn ProviderRegistry,
    db: &'a DbState,
    body_store: &'a BodyStoreState,
    inline_images: &'a InlineImageStoreState,
    search: &'a SearchState,
    progress: &'a dyn ProgressReporter,
) -> Result<(Box<dyn ProviderOps>, ProviderCtx<'a>), String> {
    let provider = match provider {
        Some(provider) => provider.to_string(),
        None => get_provider_type(db, account_id).await?,
    };
    let ops = registry.get_ops(&provider, account_id).await?;
    let ctx = ProviderCtx {
        account_id,
        db,
        body_store,
        inline_images,
        search,
        progress,
    };
    Ok((ops, ctx))
}

// ── Sync ────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub(crate) async fn provider_sync_auto_for_provider(
    account_id: &str,
    provider: &str,
    initial_sync_completed: bool,
    sync_days: i64,
    db: &DbState,
    registry: &dyn ProviderRegistry,
    body_store: &BodyStoreState,
    inline_images: &InlineImageStoreState,
    search: &SearchState,
    progress: &dyn ProgressReporter,
) -> Result<AutoSyncResult, String> {
    let (ops, ctx) = resolve_provider_command_with_registry(
        Some(provider),
        account_id,
        registry,
        db,
        body_store,
        inline_images,
        search,
        progress,
    )
    .await?;

    let fallback_marker = if provider == "gmail_api" {
        Some("HISTORY_EXPIRED")
    } else if provider == "jmap" {
        Some("JMAP_STATE_EXPIRED")
    } else if provider == "graph" {
        Some("GRAPH_NO_DELTA_STATE")
    } else {
        None
    };

    if initial_sync_completed {
        match ops.sync_delta(&ctx, Some(sync_days)).await {
            Ok(result) => {
                return Ok(AutoSyncResult {
                    new_inbox_message_ids: result.new_inbox_message_ids,
                    affected_thread_ids: result.affected_thread_ids,
                    was_delta: true,
                    fell_back_to_initial: false,
                });
            }
            Err(err)
                if should_fallback_to_initial(&err, fallback_marker) || err == "JMAP_NO_STATE" =>
            {
                emit_fallback_progress(progress, provider, account_id);
                let result = ops.sync_initial(&ctx, sync_days).await?;
                return Ok(AutoSyncResult {
                    new_inbox_message_ids: result.new_inbox_message_ids,
                    affected_thread_ids: result.affected_thread_ids,
                    was_delta: true,
                    fell_back_to_initial: true,
                });
            }
            Err(err) => return Err(err),
        }
    }

    let result = ops.sync_initial(&ctx, sync_days).await?;
    Ok(AutoSyncResult {
        new_inbox_message_ids: result.new_inbox_message_ids,
        affected_thread_ids: result.affected_thread_ids,
        was_delta: false,
        fell_back_to_initial: false,
    })
}

fn emit_fallback_progress(progress: &dyn ProgressReporter, provider: &str, account_id: &str) {
    let event_name = match provider {
        "gmail_api" => "gmail-sync-progress",
        "imap" => "imap-sync-progress",
        "jmap" => "jmap-sync-progress",
        "graph" => "graph-sync-progress",
        _ => return,
    };

    let event = SyncProgressEvent {
        account_id: account_id.to_string(),
        phase: "fallback".to_string(),
        current: 0,
        total: 1,
        folder: None,
    };

    progress::emit_event(progress, event_name, &event);
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn provider_sync_auto_impl(
    account_id: &str,
    app_state: &AppState,
    progress: &dyn ProgressReporter,
) -> Result<AutoSyncResult, String> {
    let account_id_owned = account_id.to_string();
    let sync_config = app_state
        .db
        .with_conn(move |conn| sync::config::get_auto_sync_config(conn, &account_id_owned))
        .await?;
    provider_sync_auto_for_provider(
        account_id,
        &sync_config.provider,
        sync_config.initial_sync_completed,
        sync_config.sync_period_days,
        &app_state.db,
        &app_state.providers,
        &app_state.body_store,
        &app_state.inline_images,
        &app_state.search,
        progress,
    )
    .await
}

#[tauri::command]
pub async fn provider_prepare_full_sync(
    db: State<'_, DbState>,
    account_ids: Vec<String>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        for account_id in account_ids {
            crate::sync::pipeline::clear_account_history_id(conn, &account_id)?;
        }
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn provider_prepare_account_resync(
    db: State<'_, DbState>,
    body_store: State<'_, BodyStoreState>,
    account_id: String,
) -> Result<(), String> {
    let message_ids = db
        .with_conn({
            let account_id = account_id.clone();
            move |conn| {
                let mut stmt = conn
                    .prepare("SELECT id FROM messages WHERE account_id = ?1")
                    .map_err(|e| format!("prepare resync message query: {e}"))?;
                stmt.query_map(rusqlite::params![account_id], |row| row.get::<_, String>(0))
                    .map_err(|e| format!("query resync message ids: {e}"))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("collect resync message ids: {e}"))
            }
        })
        .await?;

    body_store.delete(message_ids).await?;

    db.with_conn(move |conn| {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("begin resync transaction: {e}"))?;
        tx.execute(
            "DELETE FROM threads WHERE account_id = ?1",
            rusqlite::params![account_id],
        )
        .map_err(|e| format!("delete threads for account: {e}"))?;
        crate::sync::pipeline::clear_account_history_id(&tx, &account_id)?;
        crate::sync::pipeline::clear_all_folder_sync_states(&tx, &account_id)?;
        tx.commit()
            .map_err(|e| format!("commit resync transaction: {e}"))?;
        Ok(())
    })
    .await
}

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
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    let (ops, ctx) = resolve_provider_command(
        None,
        &account_id,
        &db,
        &gmail,
        &jmap,
        &graph,
        &body_store,
        &inline_images,
        &search,
        &reporter,
    )
    .await?;
    let _ = ops.sync_initial(&ctx, days_back.unwrap_or(365)).await?;
    Ok(())
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
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<SyncResult, String> {
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    let (ops, ctx) = resolve_provider_command(
        None,
        &account_id,
        &db,
        &gmail,
        &jmap,
        &graph,
        &body_store,
        &inline_images,
        &search,
        &reporter,
    )
    .await?;
    ops.sync_delta(&ctx, None).await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provider_sync_auto(
    account_id: String,
    sync_state: State<'_, SyncState>,
    app_state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<AutoSyncResult, String> {
    if !sync_state.try_lock_account(&account_id) {
        return Err("Sync already in progress for this account".to_string());
    }

    let reporter = TauriProgressReporter::from_ref(&app_handle);
    let result = provider_sync_auto_impl(&account_id, &app_state, &reporter).await;
    sync_state.unlock_account(&account_id);
    result
}

fn should_fallback_to_initial(err: &str, fallback_marker: Option<&str>) -> bool {
    fallback_marker
        .map(|marker| err == marker || err.contains(marker))
        .unwrap_or(false)
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
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    let (ops, ctx) = resolve_provider_command(
        None,
        &account_id,
        &db,
        &gmail,
        &jmap,
        &graph,
        &body_store,
        &inline_images,
        &search,
        &reporter,
    )
    .await?;
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
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    let (ops, ctx) = resolve_provider_command(
        None,
        &account_id,
        &db,
        &gmail,
        &jmap,
        &graph,
        &body_store,
        &inline_images,
        &search,
        &reporter,
    )
    .await?;
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
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    let (ops, ctx) = resolve_provider_command(
        None,
        &account_id,
        &db,
        &gmail,
        &jmap,
        &graph,
        &body_store,
        &inline_images,
        &search,
        &reporter,
    )
    .await?;
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
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    let (ops, ctx) = resolve_provider_command(
        None,
        &account_id,
        &db,
        &gmail,
        &jmap,
        &graph,
        &body_store,
        &inline_images,
        &search,
        &reporter,
    )
    .await?;
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
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    let (ops, ctx) = resolve_provider_command(
        None,
        &account_id,
        &db,
        &gmail,
        &jmap,
        &graph,
        &body_store,
        &inline_images,
        &search,
        &reporter,
    )
    .await?;
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
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    let (ops, ctx) = resolve_provider_command(
        None,
        &account_id,
        &db,
        &gmail,
        &jmap,
        &graph,
        &body_store,
        &inline_images,
        &search,
        &reporter,
    )
    .await?;
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
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    let (ops, ctx) = resolve_provider_command(
        None,
        &account_id,
        &db,
        &gmail,
        &jmap,
        &graph,
        &body_store,
        &inline_images,
        &search,
        &reporter,
    )
    .await?;
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
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    let (ops, ctx) = resolve_provider_command(
        None,
        &account_id,
        &db,
        &gmail,
        &jmap,
        &graph,
        &body_store,
        &inline_images,
        &search,
        &reporter,
    )
    .await?;
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
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    let (ops, ctx) = resolve_provider_command(
        None,
        &account_id,
        &db,
        &gmail,
        &jmap,
        &graph,
        &body_store,
        &inline_images,
        &search,
        &reporter,
    )
    .await?;
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
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<String, String> {
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    let (ops, ctx) = resolve_provider_command(
        None,
        &account_id,
        &db,
        &gmail,
        &jmap,
        &graph,
        &body_store,
        &inline_images,
        &search,
        &reporter,
    )
    .await?;
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
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<String, String> {
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    let (ops, ctx) = resolve_provider_command(
        None,
        &account_id,
        &db,
        &gmail,
        &jmap,
        &graph,
        &body_store,
        &inline_images,
        &search,
        &reporter,
    )
    .await?;
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
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<String, String> {
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    let (ops, ctx) = resolve_provider_command(
        None,
        &account_id,
        &db,
        &gmail,
        &jmap,
        &graph,
        &body_store,
        &inline_images,
        &search,
        &reporter,
    )
    .await?;
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
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    let (ops, ctx) = resolve_provider_command(
        None,
        &account_id,
        &db,
        &gmail,
        &jmap,
        &graph,
        &body_store,
        &inline_images,
        &search,
        &reporter,
    )
    .await?;
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
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<AttachmentData, String> {
    // 1. Check inline image store (fast SQLite lookup for small images)
    if let Some(hit) = try_inline_image_hit(
        &db,
        &inline_images,
        &account_id,
        &message_id,
        &attachment_id,
    )
    .await?
    {
        return Ok(hit);
    }

    // 2. Check file-based cache
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("resolve app data dir: {e}"))?;
    if let Some(hit) =
        try_cache_hit(&db, &app_data_dir, &account_id, &message_id, &attachment_id).await?
    {
        return Ok(hit);
    }

    // 3. Cache miss — fetch from provider
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    let (ops, ctx) = resolve_provider_command(
        None,
        &account_id,
        &db,
        &gmail,
        &jmap,
        &graph,
        &body_store,
        &inline_images,
        &search,
        &reporter,
    )
    .await?;
    let result = ops
        .fetch_attachment(&ctx, &message_id, &attachment_id)
        .await?;

    // 4. Cache the result (fire-and-forget — don't delay response)
    cache_after_fetch(
        &db,
        &inline_images,
        &app_data_dir,
        &account_id,
        &message_id,
        &attachment_id,
        &result.data,
    );

    Ok(result)
}

/// Check the inline image SQLite store for small cached images.
async fn try_inline_image_hit(
    db: &DbState,
    inline_images: &InlineImageStoreState,
    account_id: &str,
    message_id: &str,
    attachment_id: &str,
) -> Result<Option<AttachmentData>, String> {
    use crate::attachment_cache::{encode_base64, find_cache_info};

    let (acct, msg, att) = (
        account_id.to_string(),
        message_id.to_string(),
        attachment_id.to_string(),
    );

    let hash = db
        .with_conn(move |conn| {
            let info = find_cache_info(conn, &acct, &msg, &att)?;
            Ok(info.and_then(|i| i.content_hash))
        })
        .await?;

    let Some(hash) = hash else { return Ok(None) };

    let result = inline_images.get(hash).await?;
    Ok(result.map(|(bytes, _mime)| {
        let size = bytes.len();
        let data = encode_base64(&bytes);
        AttachmentData { data, size }
    }))
}

/// Check the content-addressed file cache for a previously fetched attachment.
async fn try_cache_hit(
    db: &DbState,
    app_data_dir: &std::path::Path,
    account_id: &str,
    message_id: &str,
    attachment_id: &str,
) -> Result<Option<AttachmentData>, String> {
    use crate::attachment_cache::{encode_base64, find_cache_info, read_cached};

    let dir = app_data_dir.to_path_buf();
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

        if let Some(bytes) = read_cached(&dir, hash) {
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
    db: &DbState,
    inline_images: &InlineImageStoreState,
    app_data_dir: &std::path::Path,
    account_id: &str,
    message_id: &str,
    attachment_id: &str,
    base64_data: &str,
) {
    use crate::attachment_cache::{
        decode_base64, enforce_cache_limit, find_cache_info, hash_bytes, update_cache_fields,
        write_cached,
    };
    use crate::inline_image_store::MAX_INLINE_SIZE;

    let db = db.clone();
    let inline_store = inline_images.clone();
    let dir = app_data_dir.to_path_buf();
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

            // Small inline images → SQLite blob store
            if bytes.len() <= MAX_INLINE_SIZE {
                let mime = {
                    let (a, m, at) = (acct.clone(), msg.clone(), att.clone());
                    db.with_conn(move |conn| {
                        let info = find_cache_info(conn, &a, &m, &at)?;
                        Ok(info.and_then(|i| i.mime_type))
                    })
                    .await?
                };
                if let Some(ref mime) = mime {
                    if mime.starts_with("image/") {
                        inline_store
                            .put(content_hash.clone(), bytes.clone(), mime.clone())
                            .await?;
                    }
                }
            }

            // File-based cache for all sizes
            let local_path = write_cached(&dir, &content_hash, &bytes)?;

            #[allow(clippy::cast_possible_wrap)]
            let cache_size = bytes.len() as i64;

            db.with_conn(move |conn| {
                let info = find_cache_info(conn, &acct, &msg, &att)?;
                if let Some(info) = info {
                    update_cache_fields(conn, &info.id, &local_path, cache_size, &content_hash)?;
                }
                Ok(())
            })
            .await?;

            enforce_cache_limit(&db, &dir).await
        }
        .await;

        if let Err(e) = result {
            log::warn!("Failed to cache attachment: {e}");
        }
    });
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provider_fetch_message(
    account_id: String,
    message_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<ProviderParsedMessage, String> {
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    let (ops, ctx) = resolve_provider_command(
        None,
        &account_id,
        &db,
        &gmail,
        &jmap,
        &graph,
        &body_store,
        &inline_images,
        &search,
        &reporter,
    )
    .await?;
    ops.fetch_message(&ctx, &message_id).await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provider_fetch_raw_message(
    account_id: String,
    message_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<String, String> {
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    let (ops, ctx) = resolve_provider_command(
        None,
        &account_id,
        &db,
        &gmail,
        &jmap,
        &graph,
        &body_store,
        &inline_images,
        &search,
        &reporter,
    )
    .await?;
    ops.fetch_raw_message(&ctx, &message_id).await
}

// ── Folders ─────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provider_test_connection(
    account_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<ProviderTestResult, String> {
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    let (ops, ctx) = resolve_provider_command(
        None,
        &account_id,
        &db,
        &gmail,
        &jmap,
        &graph,
        &body_store,
        &inline_images,
        &search,
        &reporter,
    )
    .await?;
    match ops.test_connection(&ctx).await {
        Ok(result) => Ok(result),
        Err(e) => Ok(ProviderTestResult {
            success: false,
            message: e,
        }),
    }
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provider_get_profile(
    account_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<ProviderProfile, String> {
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    let (ops, ctx) = resolve_provider_command(
        None,
        &account_id,
        &db,
        &gmail,
        &jmap,
        &graph,
        &body_store,
        &inline_images,
        &search,
        &reporter,
    )
    .await?;
    ops.get_profile(&ctx).await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provider_list_folders(
    account_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<Vec<ProviderFolderEntry>, String> {
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    let (ops, ctx) = resolve_provider_command(
        None,
        &account_id,
        &db,
        &gmail,
        &jmap,
        &graph,
        &body_store,
        &inline_images,
        &search,
        &reporter,
    )
    .await?;
    ops.list_folders(&ctx).await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provider_create_folder(
    account_id: String,
    name: String,
    parent_id: Option<String>,
    text_color: Option<String>,
    bg_color: Option<String>,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<ProviderFolderMutation, String> {
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    let (ops, ctx) = resolve_provider_command(
        None,
        &account_id,
        &db,
        &gmail,
        &jmap,
        &graph,
        &body_store,
        &inline_images,
        &search,
        &reporter,
    )
    .await?;
    ops.create_folder(
        &ctx,
        &name,
        parent_id.as_deref(),
        text_color.as_deref(),
        bg_color.as_deref(),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provider_rename_folder(
    account_id: String,
    folder_id: String,
    new_name: String,
    text_color: Option<String>,
    bg_color: Option<String>,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<ProviderFolderMutation, String> {
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    let (ops, ctx) = resolve_provider_command(
        None,
        &account_id,
        &db,
        &gmail,
        &jmap,
        &graph,
        &body_store,
        &inline_images,
        &search,
        &reporter,
    )
    .await?;
    ops.rename_folder(
        &ctx,
        &folder_id,
        &new_name,
        text_color.as_deref(),
        bg_color.as_deref(),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provider_delete_folder(
    account_id: String,
    folder_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    jmap: State<'_, JmapState>,
    graph: State<'_, GraphState>,
    body_store: State<'_, BodyStoreState>,
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let reporter = TauriProgressReporter::from_ref(&app_handle);
    let (ops, ctx) = resolve_provider_command(
        None,
        &account_id,
        &db,
        &gmail,
        &jmap,
        &graph,
        &body_store,
        &inline_images,
        &search,
        &reporter,
    )
    .await?;
    ops.delete_folder(&ctx, &folder_id).await
}
