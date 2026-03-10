use std::collections::HashMap;

use jmap_client::email::{Email, Property};

use super::mailbox_mapper::{MailboxInfo, get_labels_for_email};

/// Parsed JMAP email ready for DB persistence.
///
/// Matches the shape written to the `messages` table + body store.
#[derive(Debug, Clone)]
pub struct ParsedJmapMessage {
    pub id: String,
    pub thread_id: String,
    pub from_address: Option<String>,
    pub from_name: Option<String>,
    pub to_addresses: Option<String>,
    pub cc_addresses: Option<String>,
    pub bcc_addresses: Option<String>,
    pub reply_to: Option<String>,
    pub subject: Option<String>,
    pub snippet: String,
    pub date: i64,
    pub is_read: bool,
    pub is_starred: bool,
    pub body_html: Option<String>,
    pub body_text: Option<String>,
    pub raw_size: i64,
    pub internal_date: i64,
    pub label_ids: Vec<String>,
    pub has_attachments: bool,
    pub attachments: Vec<ParsedJmapAttachment>,
    /// JMAP Message-ID header values
    pub message_id_header: Option<String>,
    /// JMAP References header values
    pub references_header: Option<String>,
    /// JMAP In-Reply-To header values
    pub in_reply_to_header: Option<String>,
}

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

    let raw_size = i64::try_from(email.size()).unwrap_or(i64::MAX);

    Ok(ParsedJmapMessage {
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
        attachments,
        message_id_header,
        references_header,
        in_reply_to_header,
    })
}

/// Format JMAP EmailAddress array to "Name <email>, ..." string.
fn format_addresses(addrs: Option<&[jmap_client::email::EmailAddress]>) -> Option<String> {
    let addrs = addrs?;
    if addrs.is_empty() {
        return None;
    }
    let formatted: Vec<String> = addrs
        .iter()
        .map(|a| {
            if let Some(name) = a.name() {
                format!("{name} <{}>", a.email())
            } else {
                a.email().to_string()
            }
        })
        .collect();
    Some(formatted.join(", "))
}

/// Extract body text or HTML from the email's bodyValues.
fn extract_body_value(email: &Email, html: bool) -> Option<String> {
    let parts = if html {
        email.html_body()?
    } else {
        email.text_body()?
    };

    let part = parts.first()?;
    let part_id = part.part_id()?;

    // Access bodyValues through the email's body_values field
    email.body_value(part_id).map(|bv| bv.value().to_string())
}
