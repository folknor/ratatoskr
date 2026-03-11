use crate::provider::ops::ProviderOps;
use crate::provider::types::{
    AttachmentData, ProviderCtx, ProviderFolderEntry, ProviderFolderMutation, ProviderProfile,
    ProviderTestResult, SyncResult,
};
use async_trait::async_trait;

use super::client::GmailClient;

/// Gmail implementation of the provider operations trait.
pub struct GmailOps {
    pub(crate) client: GmailClient,
}

#[async_trait]
impl ProviderOps for GmailOps {
    async fn sync_initial(
        &self,
        ctx: &ProviderCtx<'_>,
        days_back: i64,
    ) -> Result<SyncResult, String> {
        super::sync::gmail_initial_sync(
            &self.client,
            ctx.account_id,
            days_back,
            ctx.db,
            ctx.body_store,
            ctx.search,
            ctx.app_handle,
        )
        .await?;
        Ok(SyncResult::default())
    }

    async fn sync_delta(
        &self,
        ctx: &ProviderCtx<'_>,
        _days_back: Option<i64>,
    ) -> Result<SyncResult, String> {
        let result = super::sync::gmail_delta_sync(
            &self.client,
            ctx.account_id,
            ctx.db,
            ctx.body_store,
            ctx.search,
            ctx.app_handle,
        )
        .await?;
        Ok(SyncResult {
            new_inbox_message_ids: result.new_inbox_message_ids,
            affected_thread_ids: result.affected_thread_ids,
        })
    }

    async fn archive(&self, ctx: &ProviderCtx<'_>, thread_id: &str) -> Result<(), String> {
        let remove = vec!["INBOX".to_string()];
        self.client
            .modify_thread(thread_id, &[], &remove, ctx.db)
            .await?;
        Ok(())
    }

    async fn trash(&self, ctx: &ProviderCtx<'_>, thread_id: &str) -> Result<(), String> {
        let add = vec!["TRASH".to_string()];
        let remove = vec!["INBOX".to_string()];
        self.client
            .modify_thread(thread_id, &add, &remove, ctx.db)
            .await?;
        Ok(())
    }

    async fn permanent_delete(&self, ctx: &ProviderCtx<'_>, thread_id: &str) -> Result<(), String> {
        self.client.delete_thread(thread_id, ctx.db).await
    }

    async fn mark_read(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        read: bool,
    ) -> Result<(), String> {
        let (add, remove) = if read {
            (vec![], vec!["UNREAD".to_string()])
        } else {
            (vec!["UNREAD".to_string()], vec![])
        };
        self.client
            .modify_thread(thread_id, &add, &remove, ctx.db)
            .await?;
        Ok(())
    }

    async fn star(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        starred: bool,
    ) -> Result<(), String> {
        let (add, remove) = if starred {
            (vec!["STARRED".to_string()], vec![])
        } else {
            (vec![], vec!["STARRED".to_string()])
        };
        self.client
            .modify_thread(thread_id, &add, &remove, ctx.db)
            .await?;
        Ok(())
    }

    async fn spam(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        is_spam: bool,
    ) -> Result<(), String> {
        let (add, remove) = if is_spam {
            (vec!["SPAM".to_string()], vec!["INBOX".to_string()])
        } else {
            (vec!["INBOX".to_string()], vec!["SPAM".to_string()])
        };
        self.client
            .modify_thread(thread_id, &add, &remove, ctx.db)
            .await?;
        Ok(())
    }

    async fn move_to_folder(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        folder_id: &str,
    ) -> Result<(), String> {
        let add = vec![folder_id.to_string()];
        self.client
            .modify_thread(thread_id, &add, &[], ctx.db)
            .await?;
        Ok(())
    }

    async fn add_tag(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        tag_id: &str,
    ) -> Result<(), String> {
        let add = vec![tag_id.to_string()];
        self.client
            .modify_thread(thread_id, &add, &[], ctx.db)
            .await?;
        Ok(())
    }

    async fn remove_tag(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        tag_id: &str,
    ) -> Result<(), String> {
        let remove = vec![tag_id.to_string()];
        self.client
            .modify_thread(thread_id, &[], &remove, ctx.db)
            .await?;
        Ok(())
    }

    async fn send_email(
        &self,
        ctx: &ProviderCtx<'_>,
        raw_base64url: &str,
        thread_id: Option<&str>,
    ) -> Result<String, String> {
        let msg = self
            .client
            .send_message(raw_base64url, thread_id, ctx.db)
            .await?;
        Ok(msg.id)
    }

    async fn create_draft(
        &self,
        ctx: &ProviderCtx<'_>,
        raw_base64url: &str,
        thread_id: Option<&str>,
    ) -> Result<String, String> {
        let draft = self
            .client
            .create_draft(raw_base64url, thread_id, ctx.db)
            .await?;
        Ok(draft.id)
    }

    async fn update_draft(
        &self,
        ctx: &ProviderCtx<'_>,
        draft_id: &str,
        raw_base64url: &str,
        thread_id: Option<&str>,
    ) -> Result<String, String> {
        let draft = self
            .client
            .update_draft(draft_id, raw_base64url, thread_id, ctx.db)
            .await?;
        Ok(draft.id)
    }

    async fn delete_draft(&self, ctx: &ProviderCtx<'_>, draft_id: &str) -> Result<(), String> {
        self.client.delete_draft(draft_id, ctx.db).await
    }

    async fn fetch_attachment(
        &self,
        ctx: &ProviderCtx<'_>,
        message_id: &str,
        attachment_id: &str,
    ) -> Result<AttachmentData, String> {
        let att = self
            .client
            .get_attachment(message_id, attachment_id, ctx.db)
            .await?;
        Ok(AttachmentData {
            data: att.data,
            size: att.size.map_or(0, |s| {
                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                let sz = s as usize;
                sz
            }),
        })
    }

    async fn list_folders(&self, ctx: &ProviderCtx<'_>) -> Result<Vec<ProviderFolderEntry>, String> {
        let labels = self.client.list_labels(ctx.db).await?;
        Ok(labels
            .into_iter()
            .map(|l| {
                let special = match l.id.as_str() {
                    "INBOX" => Some("inbox"),
                    "SENT" => Some("sent"),
                    "TRASH" => Some("trash"),
                    "SPAM" => Some("spam"),
                    "DRAFT" => Some("drafts"),
                    _ => None,
                };
                ProviderFolderEntry {
                    id: l.id.clone(),
                    name: l.name.clone(),
                    path: l.name,
                    folder_type: if l.label_type.as_deref() == Some("system") {
                        "system".to_string()
                    } else {
                        "user".to_string()
                    },
                    special_use: special.map(String::from),
                    delimiter: Some("/".to_string()),
                    message_count: l.messages_total.map(|v| v as u32),
                    unread_count: l.messages_unread.map(|v| v as u32),
                    color_bg: l.color.as_ref().map(|c| c.background_color.clone()),
                    color_fg: l.color.as_ref().map(|c| c.text_color.clone()),
                }
            })
            .collect())
    }

    async fn create_folder(
        &self,
        ctx: &ProviderCtx<'_>,
        name: &str,
        parent_id: Option<&str>,
        text_color: Option<&str>,
        bg_color: Option<&str>,
    ) -> Result<ProviderFolderMutation, String> {
        let full_name = parent_id.map_or_else(|| name.to_string(), |p| format!("{p}/{name}"));
        let color = match (text_color, bg_color) {
            (Some(tc), Some(bc)) => Some((tc, bc)),
            _ => None,
        };
        let label = self.client.create_label(&full_name, color, ctx.db).await?;
        Ok(ProviderFolderMutation {
            id: label.id,
            name: label.name.clone(),
            path: label.name,
            folder_type: "user".to_string(),
            special_use: None,
            delimiter: Some("/".to_string()),
            color_bg: label.color.as_ref().map(|c| c.background_color.clone()),
            color_fg: label.color.as_ref().map(|c| c.text_color.clone()),
        })
    }

    async fn rename_folder(
        &self,
        ctx: &ProviderCtx<'_>,
        folder_id: &str,
        new_name: &str,
        text_color: Option<&str>,
        bg_color: Option<&str>,
    ) -> Result<ProviderFolderMutation, String> {
        let color = match (text_color, bg_color) {
            (Some(tc), Some(bc)) => Some(Some((tc, bc))),
            _ => None,
        };
        let label = self
            .client
            .update_label(folder_id, Some(new_name), color, ctx.db)
            .await?;
        Ok(ProviderFolderMutation {
            id: label.id,
            name: label.name.clone(),
            path: label.name,
            folder_type: if label.label_type.as_deref() == Some("system") {
                "system".to_string()
            } else {
                "user".to_string()
            },
            special_use: None,
            delimiter: Some("/".to_string()),
            color_bg: label.color.as_ref().map(|c| c.background_color.clone()),
            color_fg: label.color.as_ref().map(|c| c.text_color.clone()),
        })
    }

    async fn delete_folder(&self, ctx: &ProviderCtx<'_>, folder_id: &str) -> Result<(), String> {
        self.client.delete_label(folder_id, ctx.db).await
    }

    async fn test_connection(&self, ctx: &ProviderCtx<'_>) -> Result<ProviderTestResult, String> {
        let profile = self.client.get_profile(ctx.db).await?;
        Ok(ProviderTestResult {
            success: true,
            message: format!("Connected as {}", profile.email_address),
        })
    }

    async fn get_profile(&self, ctx: &ProviderCtx<'_>) -> Result<ProviderProfile, String> {
        let profile = self.client.get_profile(ctx.db).await?;
        Ok(ProviderProfile {
            email: profile.email_address,
            name: None,
        })
    }
}
