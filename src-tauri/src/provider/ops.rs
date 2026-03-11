use async_trait::async_trait;

use super::types::{
    AttachmentData, ProviderCtx, ProviderFolder, ProviderParsedMessage, ProviderProfile,
    ProviderTestResult, SyncResult,
};

/// Common operations that every email provider must support.
///
/// This trait covers the ~17 operations shared across Gmail, JMAP, and Graph.
/// It does NOT unify state ownership, auth lifecycle, or provider-specific APIs.
/// Each provider keeps its own `*State` as a Tauri managed state.
#[async_trait]
pub trait ProviderOps: Send + Sync {
    // ── Sync ────────────────────────────────────────────────────

    async fn sync_initial(&self, ctx: &ProviderCtx<'_>, days_back: i64) -> Result<(), String>;
    async fn sync_delta(&self, ctx: &ProviderCtx<'_>) -> Result<SyncResult, String>;

    // ── Actions (thread-level) ──────────────────────────────────

    async fn archive(&self, ctx: &ProviderCtx<'_>, thread_id: &str) -> Result<(), String>;
    async fn trash(&self, ctx: &ProviderCtx<'_>, thread_id: &str) -> Result<(), String>;
    async fn permanent_delete(&self, ctx: &ProviderCtx<'_>, thread_id: &str) -> Result<(), String>;
    async fn mark_read(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        read: bool,
    ) -> Result<(), String>;
    async fn star(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        starred: bool,
    ) -> Result<(), String>;
    async fn spam(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        is_spam: bool,
    ) -> Result<(), String>;
    async fn move_to_folder(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        folder_id: &str,
    ) -> Result<(), String>;
    async fn add_tag(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        tag_id: &str,
    ) -> Result<(), String>;
    async fn remove_tag(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        tag_id: &str,
    ) -> Result<(), String>;

    // ── Send + Drafts ───────────────────────────────────────────

    /// Returns the sent message ID.
    async fn send_email(
        &self,
        ctx: &ProviderCtx<'_>,
        raw_base64url: &str,
        thread_id: Option<&str>,
    ) -> Result<String, String>;

    /// Returns the draft ID.
    async fn create_draft(
        &self,
        ctx: &ProviderCtx<'_>,
        raw_base64url: &str,
        thread_id: Option<&str>,
    ) -> Result<String, String>;

    /// Returns the (possibly new) draft ID.
    async fn update_draft(
        &self,
        ctx: &ProviderCtx<'_>,
        draft_id: &str,
        raw_base64url: &str,
        thread_id: Option<&str>,
    ) -> Result<String, String>;

    async fn delete_draft(&self, ctx: &ProviderCtx<'_>, draft_id: &str) -> Result<(), String>;

    // ── Attachments ─────────────────────────────────────────────

    async fn fetch_attachment(
        &self,
        ctx: &ProviderCtx<'_>,
        message_id: &str,
        attachment_id: &str,
    ) -> Result<AttachmentData, String>;

    async fn fetch_message(
        &self,
        _ctx: &ProviderCtx<'_>,
        _message_id: &str,
    ) -> Result<ProviderParsedMessage, String> {
        Err("Fetching parsed messages is not supported for this provider.".to_string())
    }

    async fn fetch_raw_message(
        &self,
        _ctx: &ProviderCtx<'_>,
        _message_id: &str,
    ) -> Result<String, String> {
        Err("Fetching raw messages is not supported for this provider.".to_string())
    }

    // ── Folders ─────────────────────────────────────────────────

    async fn list_folders(&self, ctx: &ProviderCtx<'_>) -> Result<Vec<ProviderFolder>, String>;
    async fn create_folder(
        &self,
        ctx: &ProviderCtx<'_>,
        name: &str,
        parent_id: Option<&str>,
        text_color: Option<&str>,
        bg_color: Option<&str>,
    ) -> Result<ProviderFolder, String>;
    async fn rename_folder(
        &self,
        ctx: &ProviderCtx<'_>,
        folder_id: &str,
        new_name: &str,
        text_color: Option<&str>,
        bg_color: Option<&str>,
    ) -> Result<ProviderFolder, String>;
    async fn delete_folder(&self, ctx: &ProviderCtx<'_>, folder_id: &str) -> Result<(), String>;

    // ── Connection / Profile ────────────────────────────────────

    async fn test_connection(&self, ctx: &ProviderCtx<'_>) -> Result<ProviderTestResult, String>;
    async fn get_profile(&self, ctx: &ProviderCtx<'_>) -> Result<ProviderProfile, String>;
}
