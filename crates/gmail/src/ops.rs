use ratatoskr_db::db::DbState;
use ratatoskr_provider_utils::encoding::encode_base64url_nopad;
use ratatoskr_provider_utils::error::ProviderError;
use ratatoskr_provider_utils::ops::ProviderOps;
use ratatoskr_provider_utils::typed_ids::{FolderId, TagId};
use ratatoskr_provider_utils::types::{
    AttachmentData, ProviderCtx, ProviderFolderEntry, ProviderFolderMutation, ProviderProfile,
    ProviderTestResult, SyncResult,
};
use async_trait::async_trait;

use super::client::GmailClient;

/// Gmail implementation of the provider operations trait.
pub struct GmailOps {
    pub(crate) client: GmailClient,
}

impl GmailOps {
    pub fn new(client: GmailClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl ProviderOps for GmailOps {
    async fn sync_initial(
        &self,
        ctx: &ProviderCtx<'_>,
        days_back: i64,
    ) -> Result<SyncResult, ProviderError> {
        super::sync::gmail_initial_sync(
            &self.client,
            ctx.account_id,
            days_back,
            ctx.db,
            ctx.body_store,
            ctx.inline_images,
            ctx.search,
            ctx.progress,
        )
        .await?;
        Ok(SyncResult::default())
    }

    async fn sync_delta(
        &self,
        ctx: &ProviderCtx<'_>,
        _days_back: Option<i64>,
    ) -> Result<SyncResult, ProviderError> {
        let result = super::sync::gmail_delta_sync(
            &self.client,
            ctx.account_id,
            ctx.db,
            ctx.body_store,
            ctx.inline_images,
            ctx.search,
            ctx.progress,
        )
        .await?;
        Ok(SyncResult {
            new_inbox_message_ids: result.new_inbox_message_ids,
            affected_thread_ids: result.affected_thread_ids,
        })
    }

    async fn archive(&self, ctx: &ProviderCtx<'_>, thread_id: &str) -> Result<(), ProviderError> {
        let remove = vec!["INBOX".to_string()];
        self.client
            .modify_thread(thread_id, &[], &remove, ctx.db)
            .await?;
        Ok(())
    }

    async fn trash(&self, ctx: &ProviderCtx<'_>, thread_id: &str) -> Result<(), ProviderError> {
        let add = vec!["TRASH".to_string()];
        let remove = vec!["INBOX".to_string()];
        self.client
            .modify_thread(thread_id, &add, &remove, ctx.db)
            .await?;
        Ok(())
    }

    async fn permanent_delete(&self, ctx: &ProviderCtx<'_>, thread_id: &str) -> Result<(), ProviderError> {
        self.client.delete_thread(thread_id, ctx.db).await?;
        Ok(())
    }

    async fn mark_read(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        read: bool,
    ) -> Result<(), ProviderError> {
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
    ) -> Result<(), ProviderError> {
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
    ) -> Result<(), ProviderError> {
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
        folder_id: &FolderId,
    ) -> Result<(), ProviderError> {
        let add = vec![folder_id.as_str().to_string()];
        self.client
            .modify_thread(thread_id, &add, &[], ctx.db)
            .await?;
        Ok(())
    }

    async fn add_tag(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        tag_id: &TagId,
    ) -> Result<(), ProviderError> {
        let add = vec![tag_id.as_str().to_string()];
        self.client
            .modify_thread(thread_id, &add, &[], ctx.db)
            .await?;
        Ok(())
    }

    async fn remove_tag(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        tag_id: &TagId,
    ) -> Result<(), ProviderError> {
        let remove = vec![tag_id.as_str().to_string()];
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
    ) -> Result<String, ProviderError> {
        log::info!("[Gmail] Sending email for account {}", ctx.account_id);
        let patched = ratatoskr_provider_utils::headers::inject_read_receipt_header_base64url(raw_base64url)?;
        let msg = self
            .client
            .send_message(&patched, thread_id, ctx.db)
            .await
            .map_err(|e| {
                log::error!("[Gmail] Send email failed for account {}: {e}", ctx.account_id);
                e
            })?;
        log::info!("[Gmail] Email sent successfully, message_id={}", msg.id);
        Ok(msg.id)
    }

    async fn create_draft(
        &self,
        ctx: &ProviderCtx<'_>,
        raw_base64url: &str,
        thread_id: Option<&str>,
    ) -> Result<String, ProviderError> {
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
    ) -> Result<String, ProviderError> {
        let draft = self
            .client
            .update_draft(draft_id, raw_base64url, thread_id, ctx.db)
            .await?;
        Ok(draft.id)
    }

    async fn delete_draft(&self, ctx: &ProviderCtx<'_>, draft_id: &str) -> Result<(), ProviderError> {
        self.client.delete_draft(draft_id, ctx.db).await?;
        Ok(())
    }

    async fn fetch_attachment(
        &self,
        ctx: &ProviderCtx<'_>,
        message_id: &str,
        attachment_id: &str,
    ) -> Result<AttachmentData, ProviderError> {
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

    async fn list_folders(
        &self,
        ctx: &ProviderCtx<'_>,
    ) -> Result<Vec<ProviderFolderEntry>, ProviderError> {
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
                    message_count: l.messages_total.map(|v| u32::try_from(v).unwrap_or(0)),
                    unread_count: l.messages_unread.map(|v| u32::try_from(v).unwrap_or(0)),
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
        parent_id: Option<&FolderId>,
        text_color: Option<&str>,
        bg_color: Option<&str>,
    ) -> Result<ProviderFolderMutation, ProviderError> {
        let full_name = parent_id.map_or_else(|| name.to_string(), |p| format!("{}/{name}", p.as_str()));
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
        folder_id: &FolderId,
        new_name: &str,
        text_color: Option<&str>,
        bg_color: Option<&str>,
    ) -> Result<ProviderFolderMutation, ProviderError> {
        let color = match (text_color, bg_color) {
            (Some(tc), Some(bc)) => Some(Some((tc, bc))),
            _ => None,
        };
        let label = self
            .client
            .update_label(folder_id.as_str(), Some(new_name), color, ctx.db)
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

    async fn delete_folder(&self, ctx: &ProviderCtx<'_>, folder_id: &FolderId) -> Result<(), ProviderError> {
        self.client.delete_label(folder_id.as_str(), ctx.db).await?;
        Ok(())
    }

    async fn test_connection(&self, ctx: &ProviderCtx<'_>) -> Result<ProviderTestResult, ProviderError> {
        let profile = self.client.get_profile(ctx.db).await?;
        Ok(ProviderTestResult {
            success: true,
            message: format!("Connected as {}", profile.email_address),
        })
    }

    async fn get_profile(&self, ctx: &ProviderCtx<'_>) -> Result<ProviderProfile, ProviderError> {
        let profile = self.client.get_profile(ctx.db).await?;
        Ok(ProviderProfile {
            email: profile.email_address,
            name: None,
        })
    }
}

// ── Gmail-specific operations (not part of ProviderOps) ─────

/// The MIME type Gmail uses to identify emoji reaction messages.
const REACTION_MIME_TYPE: &str = "text/vnd.google.email-reaction+json";

/// Build a raw RFC 2822 MIME message that Gmail will interpret as an emoji
/// reaction to an existing message.
///
/// The resulting message has a single MIME part with content type
/// `text/vnd.google.email-reaction+json` containing `{"emoji":"<emoji>","version":1}`.
fn build_reaction_mime(
    from: &str,
    to: &str,
    subject: &str,
    in_reply_to: &str,
    references: &str,
    emoji: &str,
) -> Vec<u8> {
    let payload = format!(r#"{{"emoji":"{emoji}","version":1}}"#);
    let boundary = format!("reaction_{:016x}", {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
    });

    let mut msg = String::with_capacity(512);
    msg.push_str(&format!("From: {from}\r\n"));
    msg.push_str(&format!("To: {to}\r\n"));
    msg.push_str(&format!("Subject: {subject}\r\n"));
    msg.push_str(&format!("In-Reply-To: {in_reply_to}\r\n"));
    msg.push_str(&format!("References: {references}\r\n"));
    msg.push_str("MIME-Version: 1.0\r\n");
    msg.push_str(&format!(
        "Content-Type: multipart/alternative; boundary=\"{boundary}\"\r\n"
    ));
    msg.push_str("\r\n");

    // The reaction MIME part
    msg.push_str(&format!("--{boundary}\r\n"));
    msg.push_str(&format!("Content-Type: {REACTION_MIME_TYPE}; charset=utf-8\r\n"));
    msg.push_str("\r\n");
    msg.push_str(&payload);
    msg.push_str("\r\n");

    // Closing boundary
    msg.push_str(&format!("--{boundary}--\r\n"));

    msg.into_bytes()
}

/// Send an emoji reaction to an existing Gmail message.
///
/// # Arguments
///
/// * `client` — Authenticated Gmail API client for the account.
/// * `db` — Database state (needed for token refresh).
/// * `from` — The sender address (the reacting user's email).
/// * `to` — The address of the original message's sender.
/// * `original_message_id` — RFC 2822 `Message-ID` of the message being reacted to
///   (e.g. `<CAB...@mail.gmail.com>`).
/// * `original_references` — The `References` header value from the original message,
///   or an empty string if none.
/// * `original_subject` — The `Subject` of the original message (used for `Re:` prefix).
/// * `thread_id` — Gmail thread ID to keep the reaction in the same thread.
/// * `emoji` — The emoji character to react with (e.g. "👍").
pub async fn send_reaction(
    client: &GmailClient,
    db: &DbState,
    from: &str,
    to: &str,
    original_message_id: &str,
    original_references: &str,
    original_subject: &str,
    thread_id: &str,
    emoji: &str,
) -> Result<String, String> {
    // Build References header: original references + the message being reacted to
    let references = if original_references.is_empty() {
        original_message_id.to_string()
    } else {
        format!("{original_references} {original_message_id}")
    };

    let subject = if original_subject.starts_with("Re:") || original_subject.starts_with("re:") {
        original_subject.to_string()
    } else {
        format!("Re: {original_subject}")
    };

    let raw_bytes = build_reaction_mime(from, to, &subject, original_message_id, &references, emoji);
    let raw_b64url = encode_base64url_nopad(&raw_bytes);

    let msg = client
        .send_message(&raw_b64url, Some(thread_id), db)
        .await?;
    Ok(msg.id)
}
