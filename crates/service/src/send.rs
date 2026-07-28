//! Core email send pipeline helpers.
//!
//! This module provides:
//! - [`SendRequest`] - all data needed to send an email
//! - [`to_bifrost_send_request`] - maps the app-facing request to bifrost
//! - Draft lifecycle helpers - `delete_local_draft`, `mark_draft_failed`

use std::time::SystemTime;

use bifrost_types::{Address, AttachmentInline};
use mail_parser::MessageParser;
pub use service_api::actions::{SendAttachment, SendIntent, SendRequest};
use service_state::WriteDbState;

use crate::actions::ActionError;

/// Errors that can occur during the send pipeline.
#[derive(Debug, Clone)]
pub enum SendError {
    /// The request could not be mapped to bifrost's structured send shape.
    Build(String),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Build(msg) => write!(f, "MIME build error: {msg}"),
        }
    }
}

impl std::error::Error for SendError {}

/// Parse `"alice@example.com"` or `"Alice <alice@example.com>"` into a
/// bifrost `Address`.
///
/// B15 dropped the `lettre` dependency, whose `Mailbox` parser did this
/// before. `mail_parser` is deliberately lenient - it will happily hand back
/// a bare token as an "address" - so the addr-spec shape is checked here
/// rather than assumed, preserving the rejection the `lettre` parse gave us.
fn parse_address(addr: &str) -> Result<Address, SendError> {
    let invalid = || SendError::Build(format!("Invalid address '{addr}'"));
    let raw = format!("From: {addr}\r\n\r\n");
    let (name, email) = MessageParser::default()
        .parse(&raw)
        .and_then(|message| {
            message
                .from()
                .and_then(|addresses| addresses.first().cloned())
        })
        .and_then(|address| address.address.map(|email| (address.name, email)))
        .ok_or_else(invalid)?;
    let email = email.into_owned();
    let (local, domain) = email.split_once('@').ok_or_else(invalid)?;
    if local.is_empty()
        || domain.is_empty()
        || domain.contains('@')
        || email.chars().any(char::is_whitespace)
    {
        return Err(invalid());
    }
    Ok(match name {
        Some(name) => Address::named(name.into_owned(), email),
        None => Address::bare(email),
    })
}

fn parse_addresses(values: &[String]) -> Result<Vec<Address>, SendError> {
    values.iter().map(|addr| parse_address(addr)).collect()
}

/// Map ratatoskr's consumer-facing send request to bifrost's structured
/// `SendRequest`. Each `to` / `cc` / `bcc` entry is parsed as one full RFC
/// 5322 address. Do not comma-split these Vec entries.
pub fn to_bifrost_send_request(
    req: &SendRequest,
    scheduled: Option<SystemTime>,
) -> Result<bifrost_types::SendRequest, ActionError> {
    // `bifrost_types::SendRequest` is `#[non_exhaustive]`, so it cannot be
    // built with a struct literal (even with `..Default::default()`) from
    // outside its crate. Construct the default and assign the public fields.
    // The unset fields (`identity`, `reply_to`, `attachments_uploaded`,
    // `save_to_sent`, `send_as`) keep their protocol defaults:
    // `save_to_sent = None` preserves today's behavior (the sent copy
    // returns via sync); `send_as`/`identity` stay default until B12/B13.
    let mut bifrost_request = bifrost_types::SendRequest::default();
    bifrost_request.from =
        Some(parse_address(&req.from).map_err(|e| ActionError::build(format!("{e}")))?);
    bifrost_request.to =
        parse_addresses(&req.to).map_err(|e| ActionError::build(format!("{e}")))?;
    bifrost_request.cc =
        parse_addresses(&req.cc).map_err(|e| ActionError::build(format!("{e}")))?;
    bifrost_request.bcc =
        parse_addresses(&req.bcc).map_err(|e| ActionError::build(format!("{e}")))?;
    bifrost_request.subject = req.subject.clone();
    bifrost_request.body_text = Some(req.body_text.clone());
    bifrost_request.body_html = Some(req.body_html.clone());
    bifrost_request.attachments_inline = req
        .attachments
        .iter()
        .map(|att| AttachmentInline {
            filename: att.filename.clone(),
            mime: att.mime_type.clone(),
            // `att.data` is already `Bytes`; cloning it is an O(1) ref-count
            // bump that shares the one heap buffer with the consumer-facing
            // request, NOT a re-copy of the payload. A 50 MB attachment is
            // therefore resident exactly once across both request shapes
            // (was `Bytes::copy_from_slice`, a full second allocation + memcpy
            // held live alongside the original for the whole send).
            data: att.data.clone(),
            inline: att.content_id.is_some(),
            content_id: att.content_id.clone(),
        })
        .collect();
    bifrost_request.in_reply_to = req.in_reply_to.clone();
    bifrost_request.references = req
        .references
        .as_deref()
        .map(|refs| refs.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default();
    bifrost_request.scheduled = scheduled;
    // Preserve the unconditional outgoing read-receipt request that every
    // pre-Bifrost send implementation injected.
    bifrost_request.request_read_receipt = true;
    Ok(bifrost_request)
}

// ── Draft lifecycle ──────────────────────────────────────────

/// Remove the local draft after a successful provider send. The sent
/// message arrives via provider sync as a regular thread in the Sent
/// folder; the `local_drafts` row has no further purpose. Leaving it
/// would surface a phantom entry in the Drafts pane permanently.
pub async fn delete_local_draft(db: &WriteDbState, draft_id: String) -> Result<(), String> {
    db.with_write(move |conn| {
        db::db::queries_extra::draft_lifecycle::delete_draft_sync(conn, &draft_id)
    })
    .await
}

/// Transition a local draft to `'failed'` status after a send error.
pub async fn mark_draft_failed(db: &WriteDbState, draft_id: String) -> Result<(), String> {
    db.with_write(move |conn| {
        db::db::queries_extra::draft_lifecycle::mark_draft_failed_sync(conn, &draft_id)
    })
    .await
}

pub async fn mark_send_intent_local(
    db: &WriteDbState,
    account_id: String,
    source_message_id: Option<String>,
    intent: SendIntent,
) -> Result<(), String> {
    let Some(message_id) = source_message_id else {
        return Ok(());
    };

    let (is_replied, is_forwarded) = match intent {
        SendIntent::New => return Ok(()),
        SendIntent::Reply => (true, false),
        SendIntent::Forward => (false, true),
    };

    db.with_write(move |conn| {
        conn.execute(
            "UPDATE messages \
             SET is_replied = CASE WHEN ?3 THEN 1 ELSE is_replied END, \
                 is_forwarded = CASE WHEN ?4 THEN 1 ELSE is_forwarded END \
             WHERE account_id = ?1 AND id = ?2",
            rusqlite::params![account_id, message_id, is_replied, is_forwarded],
        )
        .map_err(|e| format!("mark send intent local: {e}"))?;
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
            source_message_id: None,
            intent: SendIntent::New,
        }
    }

    #[test]
    fn to_bifrost_send_request_maps_addresses_without_comma_splitting() {
        let req = minimal_request();
        let mapped = to_bifrost_send_request(&req, None).expect("map");
        assert_eq!(mapped.to.len(), 1);
        assert_eq!(mapped.to[0].address, "bob@example.com");
        assert_eq!(mapped.subject.as_deref(), Some("Test subject"));
        assert_eq!(mapped.body_text.as_deref(), Some("Hello"));
        assert_eq!(mapped.body_html.as_deref(), Some("<p>Hello</p>"));
        assert!(mapped.request_read_receipt);
    }

    #[test]
    fn to_bifrost_send_request_carries_inline_content_id() {
        let mut req = minimal_request();
        req.attachments.push(SendAttachment {
            filename: "test.txt".to_string(),
            mime_type: "text/plain".to_string(),
            data: b"file content".to_vec().into(),
            content_id: None,
        });

        req.attachments[0].content_id = Some("cid-1".to_string());
        let mapped = to_bifrost_send_request(&req, None).expect("map");
        assert_eq!(mapped.attachments_inline.len(), 1);
        assert_eq!(
            mapped.attachments_inline[0].content_id.as_deref(),
            Some("cid-1")
        );
        assert!(mapped.attachments_inline[0].inline);
    }

    #[test]
    fn to_bifrost_send_request_carries_reply_headers_and_schedule() {
        let mut req = minimal_request();
        req.in_reply_to = Some("<original@example.com>".to_string());
        req.references = Some("<root@example.com> <original@example.com>".to_string());
        let scheduled = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(42);

        let mapped = to_bifrost_send_request(&req, Some(scheduled)).expect("map");
        assert_eq!(
            mapped.in_reply_to.as_deref(),
            Some("<original@example.com>")
        );
        assert_eq!(
            mapped.references,
            vec!["<root@example.com>", "<original@example.com>"]
        );
        assert_eq!(mapped.scheduled, Some(scheduled));
    }

    #[test]
    fn to_bifrost_send_request_preserves_display_names() {
        let mut req = minimal_request();
        req.from = "Alice Smith <alice@example.com>".to_string();
        req.to = vec!["Bob Jones <bob@example.com>".to_string()];

        let mapped = to_bifrost_send_request(&req, None).expect("map");
        let from = mapped.from.expect("from");
        assert_eq!(from.name.as_deref(), Some("Alice Smith"));
        assert_eq!(mapped.to[0].name.as_deref(), Some("Bob Jones"));
    }

    #[test]
    fn test_invalid_from_address() {
        let mut req = minimal_request();
        req.from = "not an email".to_string();
        let result = to_bifrost_send_request(&req, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_to_address() {
        let mut req = minimal_request();
        req.to = vec!["not an email".to_string()];
        let result = to_bifrost_send_request(&req, None);
        assert!(result.is_err());
    }

    /// The `lettre::Mailbox` parse B15 removed rejected these; `mail_parser`
    /// alone does not, so the addr-spec guard in `parse_address` has to.
    #[test]
    fn parse_address_rejects_non_addr_spec_shapes() {
        for addr in ["bob", "bob@", "@example.com", "bob@a@b.com", ""] {
            assert!(
                parse_address(addr).is_err(),
                "expected {addr:?} to be rejected"
            );
        }
        assert_eq!(
            parse_address("Bob <bob@example.com>")
                .expect("valid")
                .address,
            "bob@example.com"
        );
    }
}
