// tauri::command macro generates code that trips let_underscore_must_use
#![allow(clippy::let_underscore_must_use)]

use tauri::State;

use super::{BodyStoreState, BodyStoreStats, MessageBody};

/// Store a single message body (zstd-compressed).
#[tauri::command]
pub async fn body_store_put(
    state: State<'_, BodyStoreState>,
    message_id: String,
    body_html: Option<String>,
    body_text: Option<String>,
) -> Result<(), String> {
    state.put(message_id, body_html, body_text).await
}

/// Store multiple message bodies in a single transaction.
#[tauri::command]
pub async fn body_store_put_batch(
    state: State<'_, BodyStoreState>,
    bodies: Vec<MessageBody>,
) -> Result<(), String> {
    state.put_batch(bodies).await
}

/// Retrieve a single message body (decompressed).
#[tauri::command]
pub async fn body_store_get(
    state: State<'_, BodyStoreState>,
    message_id: String,
) -> Result<Option<MessageBody>, String> {
    state.get(message_id).await
}

/// Retrieve multiple message bodies (decompressed).
#[tauri::command]
pub async fn body_store_get_batch(
    state: State<'_, BodyStoreState>,
    message_ids: Vec<String>,
) -> Result<Vec<MessageBody>, String> {
    state.get_batch(message_ids).await
}

/// Delete bodies for given message IDs.
#[tauri::command]
pub async fn body_store_delete(
    state: State<'_, BodyStoreState>,
    message_ids: Vec<String>,
) -> Result<u64, String> {
    state.delete(message_ids).await
}

/// Get body store statistics (count, compressed sizes).
#[tauri::command]
pub async fn body_store_stats(state: State<'_, BodyStoreState>) -> Result<BodyStoreStats, String> {
    state.stats().await
}

/// Migrate existing bodies from the metadata DB into the body store.
///
/// No-op: body_html/body_text columns have been dropped from the messages table.
/// Kept for backward compatibility with Tauri command registration.
#[tauri::command]
pub async fn body_store_migrate(
    _state: State<'_, BodyStoreState>,
    _db_state: State<'_, crate::db::DbState>,
) -> Result<u64, String> {
    log::info!("Body store migration: columns dropped, migration no longer needed");
    Ok(0)
}
