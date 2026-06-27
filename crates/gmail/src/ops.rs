use async_trait::async_trait;
use common::encoding::encode_base64url_nopad;
use common::error::ProviderError;
use common::ops::ProviderOps;
use common::typed_ids::FolderId;
use common::types::{
    ActionProviderCtx, FetchedAttachment, LabelKind, ProviderCtx, ProviderProfile,
    ProviderTestResult,
};
use db::db::ReadDbState;

use super::client::GmailClient;

/// Gmail implementation of the provider operations trait.
pub struct GmailOps {
    pub client: GmailClient,
}

impl GmailOps {
    pub fn new(client: GmailClient) -> Self {
        Self { client }
    }
}

// Gmail mail sync now runs through the service-owned bifrost runner; this
// type remains the Gmail action surface.
#[async_trait]
impl ProviderOps for GmailOps {
    async fn archive(
        &self,
        ctx: &ActionProviderCtx<'_>,
        thread_id: &str,
    ) -> Result<(), ProviderError> {
        let remove = vec!["INBOX".to_string()];
        self.client
            .modify_thread(thread_id, &[], &remove, ctx.db)
            .await?;
        Ok(())
    }

    async fn trash(
        &self,
        ctx: &ActionProviderCtx<'_>,
        thread_id: &str,
    ) -> Result<(), ProviderError> {
        let add = vec!["TRASH".to_string()];
        let remove = vec!["INBOX".to_string()];
        self.client
            .modify_thread(thread_id, &add, &remove, ctx.db)
            .await?;
        Ok(())
    }

    async fn permanent_delete(
        &self,
        ctx: &ActionProviderCtx<'_>,
        thread_id: &str,
    ) -> Result<(), ProviderError> {
        self.client.delete_thread(thread_id, ctx.db).await?;
        Ok(())
    }

    async fn mark_read(
        &self,
        ctx: &ActionProviderCtx<'_>,
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
        ctx: &ActionProviderCtx<'_>,
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
        ctx: &ActionProviderCtx<'_>,
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
        ctx: &ActionProviderCtx<'_>,
        thread_id: &str,
        folder_id: &FolderId,
    ) -> Result<(), ProviderError> {
        let add = vec![folder_id.as_str().to_string()];
        self.client
            .modify_thread(thread_id, &add, &[], ctx.db)
            .await?;
        Ok(())
    }

    async fn add_label(
        &self,
        ctx: &ActionProviderCtx<'_>,
        thread_id: &str,
        label: &LabelKind,
    ) -> Result<(), ProviderError> {
        let LabelKind::GmailUser(label_id) = label else {
            return Err(ProviderError::Client(format!(
                "Gmail add_label received non-Gmail label kind: {label:?}"
            )));
        };
        let add = vec![label_id.as_str().to_string()];
        self.client
            .modify_thread(thread_id, &add, &[], ctx.db)
            .await?;
        Ok(())
    }

    async fn remove_label(
        &self,
        ctx: &ActionProviderCtx<'_>,
        thread_id: &str,
        label: &LabelKind,
    ) -> Result<(), ProviderError> {
        let LabelKind::GmailUser(label_id) = label else {
            return Err(ProviderError::Client(format!(
                "Gmail remove_label received non-Gmail label kind: {label:?}"
            )));
        };
        let remove = vec![label_id.as_str().to_string()];
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
        let patched = common::headers::inject_read_receipt_header_base64url(raw_base64url)?;
        let msg = self
            .client
            .send_message(&patched, thread_id, ctx.db)
            .await
            .map_err(|e| {
                log::error!(
                    "[Gmail] Send email failed for account {}: {e}",
                    ctx.account_id
                );
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

    async fn delete_draft(
        &self,
        ctx: &ProviderCtx<'_>,
        draft_id: &str,
    ) -> Result<(), ProviderError> {
        self.client.delete_draft(draft_id, ctx.db).await?;
        Ok(())
    }

    async fn fetch_attachment(
        &self,
        ctx: &ProviderCtx<'_>,
        message_id: &str,
        attachment_id: &str,
    ) -> Result<FetchedAttachment, ProviderError> {
        let att = self
            .client
            .get_attachment(message_id, attachment_id, ctx.db)
            .await?;
        let bytes =
            common::encoding::decode_base64url_nopad(&att.data).map_err(ProviderError::Client)?;
        let size = bytes.len() as u64;
        Ok(FetchedAttachment { bytes, size })
    }

    async fn test_connection(
        &self,
        ctx: &ProviderCtx<'_>,
    ) -> Result<ProviderTestResult, ProviderError> {
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
    msg.push_str(&format!(
        "Content-Type: {REACTION_MIME_TYPE}; charset=utf-8\r\n"
    ));
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
/// * `client` - Authenticated Gmail API client for the account.
/// * `db` - Database state (needed for token refresh).
/// * `from` - The sender address (the reacting user's email).
/// * `to` - The address of the original message's sender.
/// * `original_message_id` - RFC 2822 `Message-ID` of the message being reacted to
///   (e.g. `<CAB...@mail.gmail.com>`).
/// * `original_references` - The `References` header value from the original message,
///   or an empty string if none.
/// * `original_subject` - The `Subject` of the original message (used for `Re:` prefix).
/// * `thread_id` - Gmail thread ID to keep the reaction in the same thread.
/// * `emoji` - The emoji character to react with (e.g. a thumbs-up).
// TODO(refactor): wrap headers/threading fields in a ReactionMessage struct.
#[allow(clippy::too_many_arguments)]
pub async fn send_reaction(
    client: &GmailClient,
    db: &ReadDbState,
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

    let raw_bytes =
        build_reaction_mime(from, to, &subject, original_message_id, &references, emoji);
    let raw_b64url = encode_base64url_nopad(&raw_bytes);

    let msg = client
        .send_message(&raw_b64url, Some(thread_id), db)
        .await?;
    Ok(msg.id)
}
