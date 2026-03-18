use ratatoskr_db::db::DbState;
use ratatoskr_provider_utils::encoding::encode_base64url_nopad;
use ratatoskr_provider_utils::ops::ProviderOps;
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
    ) -> Result<SyncResult, String> {
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
    ) -> Result<SyncResult, String> {
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

    async fn apply_category(
        &self,
        ctx: &ProviderCtx<'_>,
        message_id: &str,
        category_name: &str,
    ) -> Result<(), String> {
        let label_id = find_label_id_by_name(&self.client, ctx, category_name).await?;
        let add = vec![label_id];
        self.client
            .modify_message(message_id, &add, &[], ctx.db)
            .await?;
        Ok(())
    }

    async fn remove_category(
        &self,
        ctx: &ProviderCtx<'_>,
        message_id: &str,
        category_name: &str,
    ) -> Result<(), String> {
        let label_id = find_label_id_by_name(&self.client, ctx, category_name).await?;
        let remove = vec![label_id];
        self.client
            .modify_message(message_id, &[], &remove, ctx.db)
            .await?;
        Ok(())
    }

    async fn send_email(
        &self,
        ctx: &ProviderCtx<'_>,
        raw_base64url: &str,
        thread_id: Option<&str>,
        _mentions: &[(String, String)],
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
        _mentions: &[(String, String)],
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

    async fn list_folders(
        &self,
        ctx: &ProviderCtx<'_>,
    ) -> Result<Vec<ProviderFolderEntry>, String> {
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

// ── Helpers ──────────────────────────────────────────────────

/// Find a Gmail label ID by its display name (case-insensitive).
async fn find_label_id_by_name(
    client: &GmailClient,
    ctx: &ProviderCtx<'_>,
    name: &str,
) -> Result<String, String> {
    let labels = client.list_labels(ctx.db).await?;
    let lower = name.to_lowercase();
    labels
        .into_iter()
        .find(|l| l.name.to_lowercase() == lower)
        .map(|l| l.id)
        .ok_or_else(|| format!("No Gmail label found matching category name '{name}'"))
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
            .map_or(0, |d| d.as_nanos() as u64)
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
