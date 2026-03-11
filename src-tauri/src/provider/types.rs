use serde::Serialize;
use tauri::AppHandle;

use crate::body_store::BodyStoreState;
use crate::db::DbState;
use crate::inline_image_store::InlineImageStoreState;
use crate::search::SearchState;

/// Standardized sync result across all providers.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult {
    pub new_inbox_message_ids: Vec<String>,
    pub affected_thread_ids: Vec<String>,
}

/// Result from auto-selecting initial vs delta sync, including fallback info.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoSyncResult {
    pub new_inbox_message_ids: Vec<String>,
    pub affected_thread_ids: Vec<String>,
    pub was_delta: bool,
    pub fell_back_to_initial: bool,
}

/// Shared context for provider operations.
/// Bundles state references to stay under clippy's 7-arg limit.
pub struct ProviderCtx<'a> {
    pub account_id: &'a str,
    pub db: &'a DbState,
    pub body_store: &'a BodyStoreState,
    pub inline_images: &'a InlineImageStoreState,
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
    pub folder_type: String,
    pub special_use: Option<String>,
    pub delimiter: Option<String>,
    pub message_count: Option<u32>,
    pub unread_count: Option<u32>,
    pub color_bg: Option<String>,
    pub color_fg: Option<String>,
}

/// Provider-agnostic attachment data (base64-encoded).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentData {
    pub data: String,
    pub size: usize,
}

/// Provider-agnostic parsed attachment metadata.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderParsedAttachment {
    pub filename: String,
    pub mime_type: String,
    pub size: u32,
    pub gmail_attachment_id: String,
    pub content_id: Option<String>,
    pub is_inline: bool,
}

/// Provider-agnostic parsed message shape matching the frontend ParsedMessage contract.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderParsedMessage {
    pub id: String,
    pub thread_id: String,
    pub from_address: Option<String>,
    pub from_name: Option<String>,
    pub to_addresses: Option<String>,
    pub cc_addresses: Option<String>,
    pub bcc_addresses: Option<String>,
    pub reply_to: Option<String>,
    pub subject: Option<String>,
    pub snippet: String,
    pub date: i64,
    pub is_read: bool,
    pub is_starred: bool,
    pub body_html: Option<String>,
    pub body_text: Option<String>,
    pub raw_size: u32,
    pub internal_date: i64,
    pub label_ids: Vec<String>,
    pub has_attachments: bool,
    pub attachments: Vec<ProviderParsedAttachment>,
    pub list_unsubscribe: Option<String>,
    pub list_unsubscribe_post: Option<String>,
    pub auth_results: Option<String>,
}

/// Provider-agnostic connection test result.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTestResult {
    pub success: bool,
    pub message: String,
}

/// Provider-agnostic account profile.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderProfile {
    pub email: String,
    pub name: Option<String>,
}
