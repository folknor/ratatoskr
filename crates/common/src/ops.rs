use async_trait::async_trait;

use super::error::ProviderError;
use super::typed_ids::FolderId;
use super::types::{
    ActionProviderCtx, FetchedAttachment, LabelKind, ProviderCtx, ProviderParsedMessage,
    ProviderProfile, ProviderTestResult, SendIntent,
};

/// Common operations that every email provider must support.
///
/// This trait covers the action / send / draft / folder / profile /
/// connection methods shared across the four providers. Production sync
/// is driven by the Service-side Bifrost engine and consumer.
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
    async fn add_label(
        &self,
        ctx: &ActionProviderCtx<'_>,
        thread_id: &str,
        label: &LabelKind,
    ) -> Result<(), ProviderError>;
    async fn remove_label(
        &self,
        ctx: &ActionProviderCtx<'_>,
        thread_id: &str,
        label: &LabelKind,
    ) -> Result<(), ProviderError>;

    /// Set the server-side "MDN already sent" marker for a single
    /// message after we've responded to a read-receipt request.
    /// IMAP/JMAP set the `$MDNSent`/`$mdnsent` keyword. Gmail and
    /// Graph have no equivalent and the default no-op is correct
    /// for them.
    async fn mark_mdn_sent(
        &self,
        _ctx: &ProviderCtx<'_>,
        _message_id: &str,
    ) -> Result<(), ProviderError> {
        Ok(())
    }

    // ── Send + Drafts ───────────────────────────────────────────

    /// Returns the sent message ID.
    async fn send_email(
        &self,
        ctx: &ProviderCtx<'_>,
        raw_base64url: &str,
        thread_id: Option<&str>,
    ) -> Result<String, ProviderError>;

    async fn mark_send_intent(
        &self,
        _ctx: &ProviderCtx<'_>,
        _source_message_id: Option<&str>,
        _intent: SendIntent,
    ) -> Result<(), ProviderError> {
        Ok(())
    }

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

    // Folder/label OBJECT CRUD moved off `ProviderOps` onto the bifrost
    // engine's `container_*` primitives (B6b). The folder/label LIST sync
    // moved off `list_folders` onto `SyncEngine::containers_list`
    // (`bifrost::containers::sync_containers`, B6a).

    // ── Connection / Profile ────────────────────────────────────

    async fn test_connection(
        &self,
        ctx: &ProviderCtx<'_>,
    ) -> Result<ProviderTestResult, ProviderError>;
    async fn get_profile(&self, ctx: &ProviderCtx<'_>) -> Result<ProviderProfile, ProviderError>;
}
