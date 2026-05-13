use async_trait::async_trait;

use super::error::ProviderError;
use super::typed_ids::{FolderId, TagId};
use super::types::{
    ActionProviderCtx, FetchedAttachment, ProviderCtx, ProviderFolderEntry, ProviderFolderMutation,
    ProviderParsedMessage, ProviderProfile, ProviderTestResult,
};

/// Common operations that every email provider must support.
///
/// This trait covers the action / send / draft / folder / profile /
/// connection methods shared across the four providers. The sync trait
/// (`provider_sync::ProviderSyncOps`) inherits this one, so any
/// `&dyn ProviderSyncOps` is also usable via this trait's methods.
/// Phase 6d-B carved the sync surface out of `common` so the crate no
/// longer needs to depend on `service-state`.
#[async_trait]
pub trait ProviderOps: Send + Sync {
    // ── Actions (thread-level) ──────────────────────────────────

    async fn archive(
        &self,
        ctx: &ActionProviderCtx<'_>,
        thread_id: &str,
    ) -> Result<(), ProviderError>;
    async fn trash(
        &self,
        ctx: &ActionProviderCtx<'_>,
        thread_id: &str,
    ) -> Result<(), ProviderError>;
    async fn permanent_delete(
        &self,
        ctx: &ActionProviderCtx<'_>,
        thread_id: &str,
    ) -> Result<(), ProviderError>;
    async fn mark_read(
        &self,
        ctx: &ActionProviderCtx<'_>,
        thread_id: &str,
        read: bool,
    ) -> Result<(), ProviderError>;
    async fn star(
        &self,
        ctx: &ActionProviderCtx<'_>,
        thread_id: &str,
        starred: bool,
    ) -> Result<(), ProviderError>;
    async fn spam(
        &self,
        ctx: &ActionProviderCtx<'_>,
        thread_id: &str,
        is_spam: bool,
    ) -> Result<(), ProviderError>;
    async fn move_to_folder(
        &self,
        ctx: &ActionProviderCtx<'_>,
        thread_id: &str,
        folder_id: &FolderId,
    ) -> Result<(), ProviderError>;
    async fn add_tag(
        &self,
        ctx: &ActionProviderCtx<'_>,
        thread_id: &str,
        tag_id: &TagId,
    ) -> Result<(), ProviderError>;
    async fn remove_tag(
        &self,
        ctx: &ActionProviderCtx<'_>,
        thread_id: &str,
        tag_id: &TagId,
    ) -> Result<(), ProviderError>;

    // ── Send + Drafts ───────────────────────────────────────────

    /// Returns the sent message ID.
    async fn send_email(
        &self,
        ctx: &ProviderCtx<'_>,
        raw_base64url: &str,
        thread_id: Option<&str>,
    ) -> Result<String, ProviderError>;

    /// Returns the draft ID.
    async fn create_draft(
        &self,
        ctx: &ProviderCtx<'_>,
        raw_base64url: &str,
        thread_id: Option<&str>,
    ) -> Result<String, ProviderError>;

    /// Returns the (possibly new) draft ID.
    async fn update_draft(
        &self,
        ctx: &ProviderCtx<'_>,
        draft_id: &str,
        raw_base64url: &str,
        thread_id: Option<&str>,
    ) -> Result<String, ProviderError>;

    async fn delete_draft(
        &self,
        ctx: &ProviderCtx<'_>,
        draft_id: &str,
    ) -> Result<(), ProviderError>;

    // ── Attachments ─────────────────────────────────────────────

    async fn fetch_attachment(
        &self,
        ctx: &ProviderCtx<'_>,
        message_id: &str,
        attachment_id: &str,
    ) -> Result<FetchedAttachment, ProviderError>;

    async fn fetch_message(
        &self,
        _ctx: &ProviderCtx<'_>,
        _message_id: &str,
    ) -> Result<ProviderParsedMessage, ProviderError> {
        Err(ProviderError::Client(
            "Fetching parsed messages is not supported for this provider.".to_string(),
        ))
    }

    async fn fetch_raw_message(
        &self,
        _ctx: &ProviderCtx<'_>,
        _message_id: &str,
    ) -> Result<String, ProviderError> {
        Err(ProviderError::Client(
            "Fetching raw messages is not supported for this provider.".to_string(),
        ))
    }

    // ── Folders ─────────────────────────────────────────────────

    async fn list_folders(
        &self,
        ctx: &ProviderCtx<'_>,
    ) -> Result<Vec<ProviderFolderEntry>, ProviderError>;
    async fn create_folder(
        &self,
        ctx: &ProviderCtx<'_>,
        name: &str,
        parent_id: Option<&FolderId>,
        text_color: Option<&str>,
        bg_color: Option<&str>,
    ) -> Result<ProviderFolderMutation, ProviderError>;
    async fn rename_folder(
        &self,
        ctx: &ProviderCtx<'_>,
        folder_id: &FolderId,
        new_name: &str,
        text_color: Option<&str>,
        bg_color: Option<&str>,
    ) -> Result<ProviderFolderMutation, ProviderError>;
    async fn delete_folder(
        &self,
        ctx: &ProviderCtx<'_>,
        folder_id: &FolderId,
    ) -> Result<(), ProviderError>;

    // ── Connection / Profile ────────────────────────────────────

    async fn test_connection(
        &self,
        ctx: &ProviderCtx<'_>,
    ) -> Result<ProviderTestResult, ProviderError>;
    async fn get_profile(&self, ctx: &ProviderCtx<'_>) -> Result<ProviderProfile, ProviderError>;
}
