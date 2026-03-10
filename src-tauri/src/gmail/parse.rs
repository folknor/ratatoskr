use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;

use super::auth_parser::parse_authentication_results;
use super::types::{GmailHeader, GmailMessage, GmailPayload};

/// A parsed attachment extracted from a Gmail message.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedAttachment {
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    pub gmail_attachment_id: String,
    pub content_id: Option<String>,
    pub is_inline: bool,
}

/// A fully parsed Gmail message ready for DB storage.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedGmailMessage {
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
    pub attachments: Vec<ParsedAttachment>,
    pub list_unsubscribe: Option<String>,
    pub list_unsubscribe_post: Option<String>,
    pub auth_results: Option<String>,
    pub message_id_header: Option<String>,
    pub references_header: Option<String>,
    pub in_reply_to_header: Option<String>,
}

/// Parse a Gmail API message into the internal representation.
pub fn parse_gmail_message(msg: &GmailMessage) -> ParsedGmailMessage {
    let payload = msg.payload.as_ref();
    let headers = payload.map_or(&[] as &[GmailHeader], |p| &p.headers);

    let from_raw = get_header(headers, "From");
    let (from_name, from_address) = parse_email_address(from_raw.as_deref());

    let body_html = payload
        .and_then(|p| extract_body(p, "text/html"))
        .and_then(|d| decode_base64url(&d));
    let body_text = payload
        .and_then(|p| extract_body(p, "text/plain"))
        .and_then(|d| decode_base64url(&d));

    let attachments = payload.map_or_else(Vec::new, extract_attachments);

    let auth_results =
        parse_authentication_results(headers).and_then(|r| serde_json::to_string(&r).ok());

    let internal_date = msg
        .internal_date
        .as_deref()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);

    ParsedGmailMessage {
        id: msg.id.clone(),
        thread_id: msg.thread_id.clone(),
        from_address,
        from_name,
        to_addresses: get_header(headers, "To"),
        cc_addresses: get_header(headers, "Cc"),
        bcc_addresses: get_header(headers, "Bcc"),
        reply_to: get_header(headers, "Reply-To"),
        subject: get_header(headers, "Subject"),
        snippet: msg.snippet.clone(),
        date: internal_date,
        is_read: !msg.label_ids.contains(&"UNREAD".to_string()),
        is_starred: msg.label_ids.contains(&"STARRED".to_string()),
        body_html,
        body_text,
        raw_size: msg.size_estimate.unwrap_or(0),
        internal_date,
        label_ids: msg.label_ids.clone(),
        has_attachments: !attachments.is_empty(),
        attachments,
        list_unsubscribe: get_header(headers, "List-Unsubscribe"),
        list_unsubscribe_post: get_header(headers, "List-Unsubscribe-Post"),
        auth_results,
        message_id_header: get_header(headers, "Message-ID")
            .or_else(|| get_header(headers, "Message-Id")),
        references_header: get_header(headers, "References"),
        in_reply_to_header: get_header(headers, "In-Reply-To"),
    }
}

fn get_header(headers: &[GmailHeader], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .map(|h| h.value.clone())
}

fn parse_email_address(raw: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(raw) = raw else {
        return (None, None);
    };

    // Format: "Display Name <email@example.com>" or "\"Name\" <email>"
    if let Some(angle_start) = raw.rfind('<')
        && let Some(angle_end) = raw[angle_start..].find('>')
    {
        let address = raw[angle_start + 1..angle_start + angle_end].trim();
        let name_part = raw[..angle_start].trim().trim_matches('"').trim();
        let name = if name_part.is_empty() || name_part == address {
            None
        } else {
            Some(name_part.to_string())
        };
        return (name, Some(address.to_string()));
    }

    // Bare email
    (None, Some(raw.trim().to_string()))
}

/// Recursively extract a body part matching the given MIME type.
fn extract_body(part: &GmailPayload, mime_type: &str) -> Option<String> {
    if part.mime_type == mime_type
        && let Some(body) = &part.body
        && let Some(data) = &body.data
    {
        return Some(data.clone());
    }

    for child in &part.parts {
        if let Some(result) = extract_body(child, mime_type) {
            return Some(result);
        }
    }

    None
}

/// Recursively extract attachments from a message payload, deduplicated by attachment ID.
///
/// Gmail can expose the same blob under multiple MIME parts (e.g. inline in
/// multipart/related AND as a named attachment in multipart/mixed). Since each
/// blob has a unique `attachment_id`, we collapse duplicates and merge metadata.
fn extract_attachments(part: &GmailPayload) -> Vec<ParsedAttachment> {
    let mut results = Vec::new();
    collect_attachments(part, &mut results);
    dedup_by_attachment_id(results)
}

/// Collapse attachments that share the same `gmail_attachment_id`.
/// Prefers the entry with a real filename and preserves `content_id`.
fn dedup_by_attachment_id(attachments: Vec<ParsedAttachment>) -> Vec<ParsedAttachment> {
    use std::collections::HashMap;

    let mut seen: HashMap<String, ParsedAttachment> = HashMap::new();
    for att in attachments {
        seen.entry(att.gmail_attachment_id.clone())
            .and_modify(|existing| {
                // Prefer whichever has a real filename
                if existing.filename == existing.content_id.as_deref().unwrap_or("inline")
                    && att.filename != att.content_id.as_deref().unwrap_or("inline")
                {
                    existing.filename.clone_from(&att.filename);
                }
                // Preserve content_id for CID resolution
                if existing.content_id.is_none() && att.content_id.is_some() {
                    existing.content_id.clone_from(&att.content_id);
                }
                // Mark as inline if either copy is
                if att.is_inline {
                    existing.is_inline = true;
                }
            })
            .or_insert(att);
    }
    seen.into_values().collect()
}

fn collect_attachments(part: &GmailPayload, results: &mut Vec<ParsedAttachment>) {
    if let Some(body) = &part.body
        && let Some(attachment_id) = &body.attachment_id
    {
        let content_id_header = part
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("content-id"));
        let content_disposition = part
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("content-disposition"));

        let has_filename = !part.filename.is_empty();
        let has_cid = content_id_header.is_some();
        let is_inline =
            content_disposition.is_some_and(|h| h.value.to_lowercase().starts_with("inline"));

        // Collect parts with a filename or Content-ID (inline images)
        if has_filename || has_cid {
            let cid = content_id_header
                .map(|h| h.value.trim_matches(|c| c == '<' || c == '>').to_string());

            let filename = if has_filename {
                part.filename.clone()
            } else {
                cid.clone().unwrap_or_else(|| "inline".to_string())
            };

            results.push(ParsedAttachment {
                filename,
                mime_type: part.mime_type.clone(),
                size: body.size,
                gmail_attachment_id: attachment_id.clone(),
                content_id: cid,
                is_inline: is_inline && !has_filename,
            });
        }
    }

    for child in &part.parts {
        collect_attachments(child, results);
    }
}

/// Decode Gmail's base64url-encoded body data to a UTF-8 string.
fn decode_base64url(data: &str) -> Option<String> {
    let bytes = URL_SAFE_NO_PAD.decode(data).ok()?;
    String::from_utf8(bytes).ok()
}
