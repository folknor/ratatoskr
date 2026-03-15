use std::path::Path;

use tauri::{AppHandle, Manager, State};

use crate::body_store::BodyStoreState;
use crate::db::DbState;
use crate::gmail::client::GmailState;
use crate::graph::client::GraphState;
use crate::inline_image_store::InlineImageStoreState;
use crate::jmap::client::JmapState;
use crate::provider::crypto::AppCryptoState;

use ratatoskr_core::account::{delete, info};
use ratatoskr_core::attachment_cache;

use super::types::{
    AccountBasicInfo, AccountCaldavSettingsInfo, AccountOAuthCredentials, CaldavConnectionInfo,
    CalendarProviderInfo,
};

#[tauri::command]
pub async fn account_get_calendar_provider_info(
    db: State<'_, DbState>,
    account_id: String,
) -> Result<Option<CalendarProviderInfo>, String> {
    db.with_conn(move |conn| info::get_calendar_provider_info(conn, &account_id))
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
        info::get_caldav_connection_info(conn, &account_id, &encryption_key)
    })
    .await
}

#[tauri::command]
pub async fn account_get_basic_info(
    db: State<'_, DbState>,
    account_id: String,
) -> Result<Option<AccountBasicInfo>, String> {
    db.with_conn(move |conn| info::get_basic_info(conn, &account_id))
        .await
}

#[tauri::command]
pub async fn account_list_basic_info(
    db: State<'_, DbState>,
) -> Result<Vec<AccountBasicInfo>, String> {
    db.with_conn(info::list_basic_info).await
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
    let deletion_data = db
        .with_conn({
            let account_id = account_id.clone();
            move |conn| delete::gather_deletion_data(conn, &account_id)
        })
        .await?;

    body_store.delete(deletion_data.message_ids).await?;

    db.with_conn({
        let account_id = account_id.clone();
        move |conn| delete::delete_account_row(conn, &account_id)
    })
    .await?;

    cleanup_cached_files(
        &app_handle,
        &db,
        deletion_data.cached_files,
    )
    .await?;

    inline_images
        .delete_unreferenced(&db, deletion_data.inline_hashes)
        .await?;

    gmail.remove(&account_id).await;
    jmap.remove(&account_id).await;
    graph.remove(&account_id).await;
    Ok(())
}

async fn cleanup_cached_files(
    app_handle: &AppHandle,
    db: &DbState,
    cached_files: Vec<(String, String)>,
) -> Result<(), String> {
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("resolve app data dir: {e}"))?;
    for (local_path, content_hash) in cached_files {
        let remaining_refs = count_refs(db, &content_hash).await?;
        if remaining_refs == 0 {
            remove_cached(&app_data_dir, &local_path);
        }
    }
    Ok(())
}

async fn count_refs(db: &DbState, content_hash: &str) -> Result<i64, String> {
    let hash = content_hash.to_string();
    db.with_conn(move |conn| delete::count_cached_refs(conn, &hash))
        .await
}

fn remove_cached(app_data_dir: &Path, local_path: &str) {
    let _ = attachment_cache::remove_cached_relative(app_data_dir, local_path);
}

#[tauri::command]
pub async fn account_get_caldav_settings_info(
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
    account_id: String,
) -> Result<Option<AccountCaldavSettingsInfo>, String> {
    let encryption_key = *gmail.encryption_key();
    db.with_conn(move |conn| {
        info::get_caldav_settings_info(conn, &account_id, &encryption_key)
    })
    .await
}

#[tauri::command]
pub async fn account_get_oauth_credentials(
    db: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
    account_id: String,
) -> Result<Option<AccountOAuthCredentials>, String> {
    let encryption_key = *crypto.encryption_key();
    db.with_conn(move |conn| {
        info::get_oauth_credentials(conn, &account_id, &encryption_key)
    })
    .await
}
