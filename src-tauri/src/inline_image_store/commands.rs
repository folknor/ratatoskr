#![allow(clippy::let_underscore_must_use)]

use tauri::State;

use super::{InlineImageStats, InlineImageStoreState};

/// Retrieve an inline image by content hash, returned as base64.
#[tauri::command]
pub async fn inline_image_get(
    state: State<'_, InlineImageStoreState>,
    content_hash: String,
) -> Result<Option<InlineImageResult>, String> {
    let result = state.get(content_hash).await?;
    Ok(result.map(|(data, mime_type)| {
        use base64::{Engine, engine::general_purpose::STANDARD};
        InlineImageResult {
            data: STANDARD.encode(&data),
            mime_type,
            size: data.len(),
        }
    }))
}

/// Get inline image store statistics.
#[tauri::command]
pub async fn inline_image_stats(
    state: State<'_, InlineImageStoreState>,
) -> Result<InlineImageStats, String> {
    state.stats().await
}

/// Clear all stored inline images.
#[tauri::command]
pub async fn inline_image_clear(state: State<'_, InlineImageStoreState>) -> Result<u64, String> {
    state.clear().await
}

/// Prune the inline image store to a given size limit (in bytes).
///
/// Evicts oldest entries until total size fits under `max_bytes`.
/// Use this when the user changes the size limit in settings.
#[tauri::command]
pub async fn inline_image_prune(
    state: State<'_, InlineImageStoreState>,
    max_bytes: u64,
) -> Result<u64, String> {
    state.prune_to_size(max_bytes).await
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineImageResult {
    pub data: String,
    pub mime_type: String,
    pub size: usize,
}
