//! Core email send pipeline — shared infrastructure for building and sending
//! RFC 2822 MIME messages.
//!
//! This module provides:
//! - [`SendRequest`] — all data needed to send an email
//! - [`build_mime_message`] — assembles a proper MIME message and returns raw bytes
//! - Draft lifecycle helpers — `mark_draft_sending`, `mark_draft_sent`, `mark_draft_failed`
//!
//! Provider-specific send logic lives in each provider crate. This module
//! produces the raw RFC 2822 bytes that providers consume (Gmail uploads raw
//! MIME, SMTP sends raw MIME, JMAP may use parts differently).

use chrono::Utc;
use lettre::message::{
    Attachment, Mailbox, MessageBuilder, MultiPart, SinglePart,
    header::ContentType,
};
use rusqlite::params;

use crate::db::DbState;
use crate::provider::encoding::encode_base64url_nopad;

// ── Types ────────────────────────────────────────────────────

/// Attachment payload for outgoing messages.
#[derive(Debug, Clone)]
pub struct SendAttachment {
    /// Original filename (e.g. `report.pdf`).
    pub filename: String,
    /// MIME type (e.g. `application/pdf`).
    pub mime_type: String,
    /// Raw file bytes.
    pub data: Vec<u8>,
    /// Optional Content-ID for inline images (without angle brackets).
    pub content_id: Option<String>,
}

/// Everything needed to send a single email.
///
/// The UI (compose window) populates this from the local draft and
/// finalized HTML/plain-text bodies.
#[derive(Debug, Clone)]
pub struct SendRequest {
    /// Local draft ID (from `local_drafts` table).
    pub draft_id: String,
    /// Account this message is sent from.
    pub account_id: String,
    /// RFC 5322 `From` address (e.g. `"Alice <alice@example.com>"`).
    pub from: String,
    /// RFC 5322 `To` addresses.
    pub to: Vec<String>,
    /// RFC 5322 `Cc` addresses.
    pub cc: Vec<String>,
    /// RFC 5322 `Bcc` addresses.
    pub bcc: Vec<String>,
    /// Subject line.
    pub subject: Option<String>,
    /// HTML body (finalized via `finalize_compose_html`).
    pub body_html: String,
    /// Plain-text body (finalized via `finalize_compose_plain_text`).
    pub body_text: String,
    /// File attachments.
    pub attachments: Vec<SendAttachment>,
    /// `In-Reply-To` header value (Message-ID of the message being replied to).
    pub in_reply_to: Option<String>,
    /// `References` header value (space-separated Message-IDs).
    pub references: Option<String>,
    /// Provider thread ID (for threading on send).
    pub thread_id: Option<String>,
}

/// Errors that can occur during the send pipeline.
#[derive(Debug, Clone)]
pub enum SendError {
    /// The MIME message could not be assembled.
    Build(String),
    /// A provider-level error (network, auth, etc.).
    Provider(String),
    /// The draft was not found or is in an invalid state.
    InvalidDraft(String),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Build(msg) => write!(f, "MIME build error: {msg}"),
            Self::Provider(msg) => write!(f, "Provider error: {msg}"),
            Self::InvalidDraft(msg) => write!(f, "Invalid draft: {msg}"),
        }
    }
}

impl std::error::Error for SendError {}

/// The result of a successful send: the provider-assigned message ID.
#[derive(Debug, Clone)]
pub struct SentMessageId(pub String);

// ── MIME construction ────────────────────────────────────────

/// Parse an address string into a lettre `Mailbox`.
///
/// Accepts both `"alice@example.com"` and `"Alice <alice@example.com>"` forms.
fn parse_mailbox(addr: &str) -> Result<Mailbox, SendError> {
    addr.parse::<Mailbox>()
        .map_err(|e| SendError::Build(format!("Invalid address '{addr}': {e}")))
}

/// Build an RFC 2822 MIME message from a [`SendRequest`].
///
/// Returns the raw message bytes suitable for:
/// - Gmail raw upload (base64url-encode the bytes)
/// - SMTP transport (send directly)
/// - JMAP blob upload
///
/// The message uses `multipart/alternative` for text/html bodies, wrapped in
/// `multipart/mixed` when attachments are present.
pub fn build_mime_message(req: &SendRequest) -> Result<Vec<u8>, SendError> {
    // ── Headers ──────────────────────────────────────────
    let from_mbox = parse_mailbox(&req.from)?;
    let mut builder: MessageBuilder = lettre::Message::builder()
        .from(from_mbox)
        .date_now();

    for addr in &req.to {
        builder = builder.to(parse_mailbox(addr)?);
    }
    for addr in &req.cc {
        builder = builder.cc(parse_mailbox(addr)?);
    }
    for addr in &req.bcc {
        builder = builder.bcc(parse_mailbox(addr)?);
    }

    builder = builder.subject(
        req.subject.as_deref().unwrap_or("").to_string()
    );

    if let Some(ref in_reply_to) = req.in_reply_to {
        builder = builder.in_reply_to(in_reply_to.clone());
    }
    if let Some(ref refs) = req.references {
        // References header contains space-separated Message-IDs.
        // lettre's .references() adds one ID at a time.
        for msg_id_ref in refs.split_whitespace() {
            builder = builder.references(msg_id_ref.to_string());
        }
    }

    // Generate a unique Message-ID
    let msg_id = format!(
        "<{}.{}@ratatoskr>",
        uuid::Uuid::new_v4(),
        Utc::now().timestamp()
    );
    builder = builder.message_id(Some(msg_id));

    // ── Body ─────────────────────────────────────────────
    let text_part = SinglePart::builder()
        .header(ContentType::TEXT_PLAIN)
        .body(req.body_text.clone());

    let html_part = SinglePart::builder()
        .header(ContentType::TEXT_HTML)
        .body(req.body_html.clone());

    let alternative = MultiPart::alternative()
        .singlepart(text_part)
        .singlepart(html_part);

    let message = if req.attachments.is_empty() {
        builder
            .multipart(alternative)
            .map_err(|e| SendError::Build(format!("Failed to build message: {e}")))?
    } else {
        // Wrap in multipart/mixed with attachments
        let mut mixed = MultiPart::mixed().multipart(alternative);

        for att in &req.attachments {
            let content_type = att
                .mime_type
                .parse::<ContentType>()
                .or_else(|_| ContentType::parse("application/octet-stream"))
                .map_err(|e| SendError::Build(format!("Content-Type parse error: {e}")))?;

            let attachment = if let Some(ref cid) = att.content_id {
                // Inline attachment (e.g. embedded image referenced by cid)
                Attachment::new_inline(cid.clone())
                    .body(att.data.clone(), content_type)
            } else {
                Attachment::new(att.filename.clone())
                    .body(att.data.clone(), content_type)
            };

            mixed = mixed.singlepart(attachment);
        }

        builder
            .multipart(mixed)
            .map_err(|e| SendError::Build(format!("Failed to build message: {e}")))?
    };

    Ok(message.formatted())
}

/// Build a MIME message and return it as a base64url-encoded string (no padding).
///
/// This is the format expected by `ProviderOps::send_email` and the Gmail API.
pub fn build_mime_message_base64url(req: &SendRequest) -> Result<String, SendError> {
    let raw = build_mime_message(req)?;
    Ok(encode_base64url_nopad(&raw))
}

// ── Draft lifecycle ──────────────────────────────────────────

/// Transition a local draft to `'sending'` status.
///
/// Called at the start of the send pipeline, before the provider call.
/// Prevents duplicate sends if the user triggers send again while in flight.
pub async fn mark_draft_sending(db: &DbState, draft_id: String) -> Result<(), String> {
    db.with_conn(move |conn| {
        let rows = conn
            .execute(
                "UPDATE local_drafts SET sync_status = 'sending' \
                 WHERE id = ?1 AND sync_status IN ('pending', 'synced', 'failed')",
                params![draft_id],
            )
            .map_err(|e| e.to_string())?;
        if rows == 0 {
            return Err(format!(
                "Draft {draft_id} not found or already sending/sent"
            ));
        }
        Ok(())
    })
    .await
}

/// Transition a local draft to `'sent'` status after successful provider send.
///
/// The `sent_message_id` is the provider-assigned ID for the sent message.
pub async fn mark_draft_sent(
    db: &DbState,
    draft_id: String,
    sent_message_id: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE local_drafts SET sync_status = 'sent', remote_draft_id = ?1 \
             WHERE id = ?2",
            params![sent_message_id, draft_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

/// Transition a local draft to `'failed'` status after a send error.
///
/// The error message is stored so the UI can display it.
pub async fn mark_draft_failed(
    db: &DbState,
    draft_id: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE local_drafts SET sync_status = 'failed' WHERE id = ?1",
            params![draft_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_request() -> SendRequest {
        SendRequest {
            draft_id: "draft-1".to_string(),
            account_id: "acct-1".to_string(),
            from: "alice@example.com".to_string(),
            to: vec!["bob@example.com".to_string()],
            cc: vec![],
            bcc: vec![],
            subject: Some("Test subject".to_string()),
            body_html: "<p>Hello</p>".to_string(),
            body_text: "Hello".to_string(),
            attachments: vec![],
            in_reply_to: None,
            references: None,
            thread_id: None,
        }
    }

    #[test]
    fn test_build_mime_no_attachments() {
        let req = minimal_request();
        let raw = build_mime_message(&req).expect("should build");
        let text = String::from_utf8_lossy(&raw);
        assert!(text.contains("From: alice@example.com"));
        assert!(text.contains("To: bob@example.com"));
        assert!(text.contains("Subject: Test subject"));
        assert!(text.contains("multipart/alternative"));
        assert!(text.contains("Hello"));
        assert!(text.contains("<p>Hello</p>"));
    }

    #[test]
    fn test_build_mime_with_attachments() {
        let mut req = minimal_request();
        req.attachments.push(SendAttachment {
            filename: "test.txt".to_string(),
            mime_type: "text/plain".to_string(),
            data: b"file content".to_vec(),
            content_id: None,
        });

        let raw = build_mime_message(&req).expect("should build");
        let text = String::from_utf8_lossy(&raw);
        assert!(text.contains("multipart/mixed"));
        assert!(text.contains("test.txt"));
    }

    #[test]
    fn test_build_mime_with_reply_headers() {
        let mut req = minimal_request();
        req.in_reply_to = Some("<original@example.com>".to_string());
        req.references = Some("<root@example.com> <original@example.com>".to_string());

        let raw = build_mime_message(&req).expect("should build");
        let text = String::from_utf8_lossy(&raw);
        assert!(text.contains("In-Reply-To:"));
        assert!(text.contains("References:"));
    }

    #[test]
    fn test_build_mime_with_display_name() {
        let mut req = minimal_request();
        req.from = "Alice Smith <alice@example.com>".to_string();
        req.to = vec!["Bob Jones <bob@example.com>".to_string()];

        let raw = build_mime_message(&req).expect("should build");
        let text = String::from_utf8_lossy(&raw);
        assert!(text.contains("Alice Smith"));
    }

    #[test]
    fn test_build_mime_cc_bcc() {
        let mut req = minimal_request();
        req.cc = vec!["carol@example.com".to_string()];
        req.bcc = vec!["secret@example.com".to_string()];

        let raw = build_mime_message(&req).expect("should build");
        let text = String::from_utf8_lossy(&raw);
        assert!(text.contains("Cc: carol@example.com"));
        // BCC should be in headers (for envelope extraction) but providers
        // typically strip it before delivery
        assert!(text.contains("Bcc:"));
    }

    #[test]
    fn test_build_mime_base64url() {
        let req = minimal_request();
        let encoded = build_mime_message_base64url(&req).expect("should encode");
        // Should be valid base64url (no +, /, or = characters)
        assert!(!encoded.contains('+'));
        assert!(!encoded.contains('/'));
        assert!(!encoded.contains('='));
    }

    #[test]
    fn test_build_mime_message_id_present() {
        let req = minimal_request();
        let raw = build_mime_message(&req).expect("should build");
        let text = String::from_utf8_lossy(&raw);
        assert!(text.contains("Message-ID:"));
        assert!(text.contains("@ratatoskr>"));
    }

    #[test]
    fn test_invalid_from_address() {
        let mut req = minimal_request();
        req.from = "not an email".to_string();
        let result = build_mime_message(&req);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_to_address() {
        let mut req = minimal_request();
        req.to = vec!["not an email".to_string()];
        let result = build_mime_message(&req);
        assert!(result.is_err());
    }
}
