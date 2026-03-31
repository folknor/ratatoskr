use mail_parser::MimeHeaders;

use common::types::ProviderCtx;

use super::super::client::GraphClient;
use super::super::types::SingleValueExtendedProperty;
use super::PID_TAG_DEFERRED_SEND_TIME;

/// Send an email by creating a draft from raw MIME, then sending it.
pub(super) async fn send_via_draft(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
    raw_base64url: &str,
    thread_id: Option<&str>,
) -> Result<String, String> {
    log::info!("[Graph] Sending email for account {}", ctx.account_id);
    let draft_id = create_draft_impl(client, ctx, raw_base64url, thread_id).await?;
    // Send the draft — no response body (202 Accepted)
    let enc_draft_id = urlencoding::encode(&draft_id);
    let me = client.api_path_prefix();
    client
        .post_no_content::<()>(&format!("{me}/messages/{enc_draft_id}/send"), None, ctx.db)
        .await
        .map_err(|e| {
            log::error!(
                "[Graph] Send email failed for account {}: {e}",
                ctx.account_id
            );
            e
        })?;
    log::info!("[Graph] Email sent successfully, draft_id={draft_id}");
    Ok(draft_id)
}

/// Create a draft with the `PidTagDeferredSendTime` extended property set.
///
/// This is the same as `create_draft_impl` but injects the deferred send time
/// into the message body before creating it on the server.
pub(super) async fn create_draft_with_deferred_time(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
    raw_base64url: &str,
    _thread_id: Option<&str>,
    send_at_utc: &str,
) -> Result<String, String> {
    let raw_bytes = common::encoding::decode_base64url_nopad(raw_base64url)
        .map_err(|e| format!("Failed to decode base64url: {e}"))?;

    let parsed = mail_parser::MessageParser::default()
        .parse(&raw_bytes)
        .ok_or("Failed to parse MIME message")?;

    let mut create_msg = mime_to_graph_message(&parsed)?;

    // Inject the deferred send time extended property
    create_msg.single_value_extended_properties = Some(vec![SingleValueExtendedProperty {
        id: PID_TAG_DEFERRED_SEND_TIME.to_string(),
        value: send_at_utc.to_string(),
    }]);

    let me = client.api_path_prefix();
    let draft: serde_json::Value = client
        .post(&format!("{me}/messages"), &create_msg, ctx.db)
        .await?;

    let draft_id = draft
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("Draft response missing id")?
        .to_string();

    upload_attachments_from_mime(client, ctx, &draft_id, &parsed).await?;

    Ok(draft_id)
}

/// Create a draft message from raw MIME (base64url-encoded).
pub(super) async fn create_draft_impl(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
    raw_base64url: &str,
    _thread_id: Option<&str>,
) -> Result<String, String> {
    // Decode base64url → raw MIME bytes
    let raw_bytes = common::encoding::decode_base64url_nopad(raw_base64url)
        .map_err(|e| format!("Failed to decode base64url: {e}"))?;

    // Parse MIME using mail-parser
    let parsed = mail_parser::MessageParser::default()
        .parse(&raw_bytes)
        .ok_or("Failed to parse MIME message")?;

    // Build Graph message JSON from parsed MIME
    let create_msg = mime_to_graph_message(&parsed)?;

    // Create draft
    let me = client.api_path_prefix();
    let draft: serde_json::Value = client
        .post(&format!("{me}/messages"), &create_msg, ctx.db)
        .await?;

    let draft_id = draft
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("Draft response missing id")?
        .to_string();

    // Upload attachments if any
    upload_attachments_from_mime(client, ctx, &draft_id, &parsed).await?;

    Ok(draft_id)
}

/// Convert a parsed MIME message to a Graph create-message request.
pub(super) fn mime_to_graph_message(
    parsed: &mail_parser::Message<'_>,
) -> Result<super::super::types::GraphCreateMessage, String> {
    use super::super::types::{GraphBodyInput, GraphCreateMessage};

    let subject = parsed.subject().map(String::from);

    // Prefer HTML body, fall back to text
    let body = if let Some(html) = parsed.body_html(0) {
        Some(GraphBodyInput {
            content_type: "html".to_string(),
            content: html.to_string(),
        })
    } else {
        parsed.body_text(0).map(|text| GraphBodyInput {
            content_type: "text".to_string(),
            content: text.to_string(),
        })
    };

    let to = addr_to_recipients(parsed.to());
    let cc = addr_to_recipients(parsed.cc());
    let bcc = addr_to_recipients(parsed.bcc());
    let reply_to = addr_to_recipients(parsed.reply_to());

    fn addr_to_recipients(
        addr: Option<&mail_parser::Address<'_>>,
    ) -> Option<Vec<super::super::types::GraphRecipient>> {
        use super::super::types::{GraphEmailAddress, GraphRecipient};
        let addr = addr?;
        let recips: Vec<GraphRecipient> = addr
            .iter()
            .filter_map(|group| {
                group.address.as_ref().map(|email| GraphRecipient {
                    email_address: GraphEmailAddress {
                        name: group.name.as_ref().map(std::string::ToString::to_string),
                        address: email.to_string(),
                    },
                })
            })
            .collect();
        if recips.is_empty() {
            None
        } else {
            Some(recips)
        }
    }

    let message_id = parsed.message_id().map(String::from);

    Ok(GraphCreateMessage {
        subject,
        body,
        to_recipients: to,
        cc_recipients: cc,
        bcc_recipients: bcc,
        reply_to,
        importance: None,
        internet_message_id: message_id,
        single_value_extended_properties: None,
        from: None,
        sender: None,
        is_read_receipt_requested: Some(true),
    })
}

/// Graph API inline attachment size limit (3 MB).
/// Attachments larger than this must use a resumable upload session.
const GRAPH_INLINE_ATTACHMENT_LIMIT: usize = 3 * 1024 * 1024;

/// Chunk size for resumable upload sessions (4 MB, must be multiple of 320 KB per Microsoft docs).
const UPLOAD_CHUNK_SIZE: usize = 4 * 1024 * 1024;

/// Upload attachments from a parsed MIME message to a Graph draft.
async fn upload_attachments_from_mime(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
    draft_id: &str,
    parsed: &mail_parser::Message<'_>,
) -> Result<(), String> {
    use super::super::types::GraphAttachmentInput;

    let enc_draft_id = urlencoding::encode(draft_id);
    for attachment in parsed.attachments() {
        let name = attachment
            .attachment_name()
            .unwrap_or("attachment")
            .to_string();
        let content_type = attachment
            .content_type()
            .map(|ct| {
                if let Some(st) = ct.subtype() {
                    format!("{}/{st}", ct.ctype())
                } else {
                    ct.ctype().to_string()
                }
            })
            .unwrap_or_else(|| "application/octet-stream".to_string());
        let is_inline = attachment
            .content_disposition()
            .is_some_and(|d| d.ctype() == "inline");
        let raw_bytes = attachment.contents();

        if raw_bytes.len() > GRAPH_INLINE_ATTACHMENT_LIMIT {
            // Large attachment: use resumable upload session
            upload_large_attachment(
                client,
                ctx,
                &enc_draft_id,
                &name,
                &content_type,
                is_inline,
                raw_bytes,
            )
            .await?;
        } else {
            // Small attachment: inline base64
            let content_bytes = common::encoding::encode_base64_standard(raw_bytes);
            let content_id = attachment
                .content_id()
                .map(|id| id.trim_matches(&['<', '>'] as &[char]).to_string());

            let input = GraphAttachmentInput {
                odata_type: "#microsoft.graph.fileAttachment".to_string(),
                name,
                content_type,
                content_bytes,
                is_inline: if is_inline { Some(true) } else { None },
                content_id,
            };

            let me = client.api_path_prefix();
            let _: serde_json::Value = client
                .post(
                    &format!("{me}/messages/{enc_draft_id}/attachments"),
                    &input,
                    ctx.db,
                )
                .await?;
        }
    }

    Ok(())
}

/// Upload a large attachment using a resumable upload session (RFC-compliant chunked PUT).
async fn upload_large_attachment(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
    enc_draft_id: &str,
    name: &str,
    content_type: &str,
    is_inline: bool,
    data: &[u8],
) -> Result<(), String> {
    use super::super::types::{
        CreateUploadSessionRequest, UploadSession, UploadSessionAttachmentItem,
    };

    #[allow(clippy::cast_possible_wrap)]
    let size = data.len() as i64;

    // Step 1: Create upload session
    let session_req = CreateUploadSessionRequest {
        attachment_item: UploadSessionAttachmentItem {
            odata_type: "#microsoft.graph.fileAttachment".to_string(),
            name: name.to_string(),
            size,
            content_type: Some(content_type.to_string()),
            is_inline: if is_inline { Some(true) } else { None },
        },
    };

    let me = client.api_path_prefix();
    let session: UploadSession = client
        .post(
            &format!("{me}/messages/{enc_draft_id}/attachments/createUploadSession"),
            &session_req,
            ctx.db,
        )
        .await?;

    // Step 2: Upload in chunks
    let total = data.len();
    let mut offset = 0;
    while offset < total {
        let end = (offset + UPLOAD_CHUNK_SIZE).min(total);
        let chunk = &data[offset..end];

        client
            .put_bytes_range(&session.upload_url, chunk, offset, end - 1, total)
            .await?;

        offset = end;
    }

    log::info!("Uploaded large attachment '{name}' ({total} bytes) via resumable session");
    Ok(())
}

/// Upload attachments from a parsed MIME to a draft in a specific user's mailbox.
///
/// Similar to `upload_attachments_from_mime` but uses `/users/{user}/messages/{id}/attachments`
/// instead of `/me/messages/{id}/attachments`.
pub(super) async fn upload_attachments_to_user_mailbox(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
    enc_user: &str,
    draft_id: &str,
    parsed: &mail_parser::Message<'_>,
) -> Result<(), String> {
    use super::super::types::GraphAttachmentInput;

    let enc_draft_id = urlencoding::encode(draft_id);
    for attachment in parsed.attachments() {
        let name = attachment
            .attachment_name()
            .unwrap_or("attachment")
            .to_string();
        let content_type = attachment
            .content_type()
            .map(|ct| {
                if let Some(st) = ct.subtype() {
                    format!("{}/{st}", ct.ctype())
                } else {
                    ct.ctype().to_string()
                }
            })
            .unwrap_or_else(|| "application/octet-stream".to_string());
        let is_inline = attachment
            .content_disposition()
            .is_some_and(|d| d.ctype() == "inline");
        let raw_bytes = attachment.contents();

        // For shared mailbox drafts, only inline (base64) attachments are supported.
        // Large attachment upload sessions on /users/{id} require application
        // permissions, so we skip the resumable path here.
        let content_bytes = common::encoding::encode_base64_standard(raw_bytes);
        let content_id = attachment
            .content_id()
            .map(|id| id.trim_matches(&['<', '>'] as &[char]).to_string());

        let input = GraphAttachmentInput {
            odata_type: "#microsoft.graph.fileAttachment".to_string(),
            name,
            content_type,
            content_bytes,
            is_inline: if is_inline { Some(true) } else { None },
            content_id,
        };

        let _: serde_json::Value = client
            .post(
                &format!("/users/{enc_user}/messages/{enc_draft_id}/attachments"),
                &input,
                ctx.db,
            )
            .await?;
    }

    Ok(())
}
