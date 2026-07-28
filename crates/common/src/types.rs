use serde::Serialize;

pub use ::types::{
    FolderKind, GmailSystemLabelId, ImportanceLevel, LabelKind, MailProviderKind,
    NamespaceAttribution, NamespaceKind, SendIntent, SystemFolderId,
};

/// Standardized sync result across all providers.
#[derive(Debug, Clone, Default, Serialize)]
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

/// Provider-agnostic parsed attachment metadata.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderParsedAttachment {
    pub filename: String,
    pub mime_type: String,
    pub size: u32,
    pub attachment_id: String,
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
