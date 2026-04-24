use std::collections::HashMap;

use jmap_client::email::{Email, Header, HeaderValue, Property};

use common::email_parsing::format_address_list;
use common::parsed_message::ParsedMessageBase;

use super::mailbox_mapper::{MailboxInfo, get_labels_for_email};

/// Parsed JMAP email ready for DB persistence.
///
/// Matches the shape written to the `messages` table + body store.
#[derive(Debug, Clone)]
pub struct ParsedJmapMessage {
    /// Common fields shared with other providers.
    pub base: ParsedMessageBase,
    pub attachments: Vec<ParsedJmapAttachment>,
    /// Non-system JMAP keywords (those not starting with `$`) that map to categories.
    pub keyword_categories: Vec<String>,
}

common::impl_message_addresses!(ParsedJmapMessage);

#[derive(Debug, Clone)]
pub struct ParsedJmapAttachment {
    pub blob_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    pub content_id: Option<String>,
    pub is_inline: bool,
}

/// Properties to request when fetching emails for sync.
pub fn email_get_properties() -> Vec<Property> {
    vec![
        Property::Id,
        Property::BlobId,
        Property::ThreadId,
        Property::MailboxIds,
        Property::Keywords,
        Property::Size,
        Property::ReceivedAt,
        Property::MessageId,
        Property::InReplyTo,
        Property::References,
        Property::From,
        Property::To,
        Property::Cc,
        Property::Bcc,
        Property::ReplyTo,
        Property::Subject,
        Property::SentAt,
        Property::HasAttachment,
        Property::Preview,
        Property::TextBody,
        Property::HtmlBody,
        Property::Attachments,
        Property::Header(Header::as_text("List-Unsubscribe", false)),
        Property::Header(Header::as_text("List-Unsubscribe-Post", false)),
        Property::Header(Header::as_text("Disposition-Notification-To", false)),
    ]
}

/// Convert a `jmap-client` Email response into our internal DB-ready struct.
///
/// Unlike Gmail parsing, JMAP provides:
/// - Bodies inline (no base64 decode / MIME walk)
/// - Typed address arrays (no string parsing)
/// - Native `threadId`
/// - No `Authentication-Results` header
pub fn parse_jmap_email(
    email: &Email,
    mailbox_map: &HashMap<String, MailboxInfo>,
) -> Result<ParsedJmapMessage, String> {
    let id = email.id().ok_or("Email missing id")?.to_string();
    let thread_id = email
        .thread_id()
        .ok_or("Email missing threadId")?
        .to_string();

    let from = email.from().and_then(|addrs| addrs.first());
    let from_address = from.map(|a| a.email().to_string());
    let from_name = from.and_then(|a| a.name()).map(String::from);

    let to_addresses = format_addresses(email.to());
    let cc_addresses = format_addresses(email.cc());
    let bcc_addresses = format_addresses(email.bcc());
    let reply_to = format_addresses(email.reply_to());

    let subject = email.subject().map(String::from);
    let snippet = email.preview().unwrap_or("").to_string();

    let sent_at = email.sent_at().unwrap_or(0);
    let received_at = email.received_at().unwrap_or(0);
    let date = if sent_at > 0 {
        sent_at * 1000
    } else {
        received_at * 1000
    };
    let internal_date = received_at * 1000;

    let keywords = email.keywords();
    let is_read = keywords.contains(&"$seen");
    let is_starred = keywords.contains(&"$flagged");

    // Non-system keywords are user-defined categories (system keywords start with '$')
    let keyword_categories: Vec<String> = keywords
        .iter()
        .filter(|kw| !kw.starts_with('$'))
        .map(|kw| (*kw).to_string())
        .collect();

    let mailbox_ids = email.mailbox_ids();
    let label_ids = get_labels_for_email(&mailbox_ids, &keywords, mailbox_map);

    let has_attachments = email.has_attachment();

    // Body extraction from bodyValues
    let body_html = extract_body_value(email, true);
    let body_text = extract_body_value(email, false);

    // Attachments
    let attachments = email
        .attachments()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| {
                    let blob_id = part.blob_id()?.to_string();
                    Some(ParsedJmapAttachment {
                        blob_id,
                        filename: part.name().unwrap_or("attachment").to_string(),
                        mime_type: part
                            .content_type()
                            .unwrap_or("application/octet-stream")
                            .to_string(),
                        size: i64::try_from(part.size()).unwrap_or(i64::MAX),
                        content_id: part.content_id().map(String::from),
                        is_inline: part.content_disposition() == Some("inline"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // Header arrays → joined strings for DB storage
    let message_id_header = email.message_id().map(|ids| ids.join(" "));
    let references_header = email.references().map(|refs| refs.join(" "));
    let in_reply_to_header = email.in_reply_to().map(|ids| ids.join(" "));

    // List-Unsubscribe headers (RFC 2369 / RFC 8058)
    let list_unsubscribe =
        extract_header_text(email.header(&Header::as_text("List-Unsubscribe", false)));
    let list_unsubscribe_post =
        extract_header_text(email.header(&Header::as_text("List-Unsubscribe-Post", false)));

    // MDN request detection (Disposition-Notification-To)
    let mdn_requested = email
        .header(&Header::as_text("Disposition-Notification-To", false))
        .is_some();

    let raw_size = i64::try_from(email.size()).unwrap_or(i64::MAX);

    Ok(ParsedJmapMessage {
        base: ParsedMessageBase {
            id,
            thread_id,
            from_address,
            from_name,
            to_addresses,
            cc_addresses,
            bcc_addresses,
            reply_to,
            subject,
            snippet,
            date,
            is_read,
            is_starred,
            body_html,
            body_text,
            raw_size,
            internal_date,
            label_ids,
            has_attachments,
            message_id_header,
            references_header,
            in_reply_to_header,
            list_unsubscribe,
            list_unsubscribe_post,
            auth_results: None,
            mdn_requested,
        },
        attachments,
        keyword_categories,
    })
}

/// Format JMAP EmailAddress array to "Name <email>, ..." string.
fn format_addresses(addrs: Option<&[jmap_client::email::EmailAddress]>) -> Option<String> {
    let addrs = addrs?;
    format_address_list(
        addrs
            .iter()
            .map(|a| (a.name().map(ToString::to_string), a.email().to_string())),
    )
}

/// Extract a text header value from a JMAP `HeaderValue`.
fn extract_header_text(hv: Option<&HeaderValue>) -> Option<String> {
    match hv {
        Some(HeaderValue::AsText(t)) => Some(t.clone()),
        _ => None,
    }
}

/// Extract body text or HTML from the email's bodyValues.
///
/// When extracting HTML, skips any `text/x-amp-html` parts - AMP emails contain
/// tracking-heavy interactive content. Falls through to subsequent parts so the
/// caller gets `text/html` if available.
fn extract_body_value(email: &Email, html: bool) -> Option<String> {
    let parts = if html {
        email.html_body()?
    } else {
        email.text_body()?
    };

    for part in parts {
        // Skip AMP body parts - prefer regular text/html
        if html
            && let Some(ct) = part.content_type()
            && common::email_parsing::is_amp_content_type(ct)
        {
            continue;
        }

        let part_id = part.part_id()?;
        if let Some(bv) = email.body_value(part_id) {
            return Some(bv.value().to_string());
        }
    }

    None
}
