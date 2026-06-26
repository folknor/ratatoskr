use serde::Serialize;

use db::db::ReadDbState;
use db::progress::ProgressReporter;

pub use ::types::{
    FolderKind, GmailSystemLabelId, ImportanceLevel, LabelKind, MailProviderKind, SendIntent,
    SystemFolderId,
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

/// Shared context for non-sync, non-action provider operations.
///
/// Phase 3 task 5 narrows this from the pre-Phase-3 wide shape (which
/// also carried `&BodyStoreReadState`, `&InlineImageStoreReadState`,
/// `&SearchReadState`). Those store handles only ever served sync
/// methods; the non-sync methods (folder mutations, `fetch_*`,
/// `get_profile`, `test_connection`, `list_folders`) never read or
/// wrote them.
///
/// Action methods take `ActionProviderCtx`. This narrow `ProviderCtx`
/// covers non-action provider calls such as attachment fetches and
/// provider metadata.
pub struct ProviderCtx<'a> {
    pub account_id: &'a str,
    pub db: &'a ReadDbState,
    pub progress: &'a dyn ProgressReporter,
}

/// Narrower context for `ProviderOps` action methods (Phase 2 task 7).
///
/// Action methods (`archive`, `trash`, `mark_read`, `star`, `spam`,
/// `move_to_folder`, `add_label`, `remove_label`, `permanent_delete`) issue
/// HTTP requests to the provider; the local DB write happens UI-side
/// (now Service-side after task 9) BEFORE the provider call. They
/// don't need `body_store` / `inline_images` / `search` - those exist
/// on `ProviderCtx` for the sync-side consumers. Dropping the unused
/// fields keeps the action-side ctx narrower and means
/// service-state's writers stay unreachable through this surface
/// (the type only carries `&ReadDbState`, which `common` already
/// depends on).
pub struct ActionProviderCtx<'a> {
    pub account_id: &'a str,
    pub db: &'a ReadDbState,
    pub progress: &'a dyn ProgressReporter,
}

/// Provider-agnostic folder representation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderFolderEntry {
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

/// Provider-agnostic folder creation/rename result.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderFolderMutation {
    pub id: String,
    pub name: String,
    pub path: String,
    pub folder_type: String,
    pub special_use: Option<String>,
    pub delimiter: Option<String>,
    pub color_bg: Option<String>,
    pub color_fg: Option<String>,
}

/// Raw attachment bytes returned by a provider's `fetch_attachment` impl.
/// Bytes never round-trip through base64 inside the Service.
#[derive(Debug, Clone)]
pub struct FetchedAttachment {
    pub bytes: Vec<u8>,
    pub size: u64,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Phase 2 task 16 regression guard: action-side `ProviderCtx`
    /// does not expose `&SearchReadState`. The action methods on
    /// `ProviderOps` take `ActionProviderCtx`, and Phase 2
    /// deliberately defers the Tantivy writer relocation to Phase 3 -
    /// so any action-time search write would be a type error. The
    /// destructure below is exhaustive (no `..` rest pattern): a
    /// future `search` field on `ActionProviderCtx` fails to compile,
    /// forcing the design conversation.
    #[allow(dead_code)]
    fn action_provider_ctx_destructure_is_exhaustive(ctx: &ActionProviderCtx<'_>) {
        let ActionProviderCtx {
            account_id: _,
            db: _,
            progress: _,
        } = ctx;
    }
}
