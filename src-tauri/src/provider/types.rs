use serde::Serialize;
use tauri::AppHandle;

use crate::body_store::BodyStoreState;
use crate::db::DbState;
use crate::search::SearchState;

/// Standardized sync result across all providers.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult {
    pub new_inbox_message_ids: Vec<String>,
    pub affected_thread_ids: Vec<String>,
}

/// Shared context for provider operations.
/// Bundles state references to stay under clippy's 7-arg limit.
pub struct ProviderCtx<'a> {
    pub account_id: &'a str,
    pub db: &'a DbState,
    pub body_store: &'a BodyStoreState,
    pub search: &'a SearchState,
    pub app_handle: &'a AppHandle,
}

/// Provider-agnostic folder representation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderFolder {
    pub id: String,
    pub name: String,
    pub path: String,
    pub special_use: Option<String>,
}

/// Provider-agnostic attachment data (base64-encoded).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentData {
    pub data: String,
    pub size: usize,
}
