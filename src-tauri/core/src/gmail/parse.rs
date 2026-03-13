use serde::Serialize;

use crate::provider::attachment_dedup::{
    dedup_by_key, prefer_missing_clone, prefer_non_placeholder_filename,
};
use crate::provider::email_parsing::parse_single_address_header;
use crate::provider::encoding::decode_base64url_nopad;
use crate::provider::headers::find_header_value_case_insensitive;

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
    pub content_hash: Option<String>,
    #[serde(skip_serializing)]
    pub inline_data: Option<Vec<u8>>,
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

impl crate::seen_addresses::MessageAddresses for ParsedGmailMessage {
    fn sender_address(&self) -> Option<&str> {
        self.from_address.as_deref()
    }
    fn sender_name(&self) -> Option<&str> {
        self.from_name.as_deref()
    }
    fn to_addresses(&self) -> Option<&str> {
        self.to_addresses.as_deref()
    }
    fn cc_addresses(&self) -> Option<&str> {
        self.cc_addresses.as_deref()
    }
    fn bcc_addresses(&self) -> Option<&str> {
        self.bcc_addresses.as_deref()
    }
    fn msg_date_ms(&self) -> i64 {
        self.date
    }
}

/// Parse a Gmail API message into the internal representation.
pub fn parse_gmail_message(msg: &GmailMessage) -> ParsedGmailMessage {
    let payload = msg.payload.as_ref();
    let headers = payload.map_or(&[] as &[GmailHeader], |p| &p.headers);

    let from_raw = get_header(headers, "From");
    let (from_name, from_address) = parse_single_address_header(from_raw.as_deref());

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
    find_header_value_case_insensitive(headers, name, |h| h.name.as_str(), |h| h.value.as_str())
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
    dedup_by_key(
        attachments,
        |att| att.gmail_attachment_id.clone(),
        |existing, att| {
            let existing_is_placeholder =
                existing.filename == existing.content_id.as_deref().unwrap_or("inline");
            let new_is_placeholder = att.filename == att.content_id.as_deref().unwrap_or("inline");
            prefer_non_placeholder_filename(
                &mut existing.filename,
                &att.filename,
                existing_is_placeholder,
                new_is_placeholder,
            );
            prefer_missing_clone(&mut existing.content_id, &att.content_id);
            if att.is_inline {
                existing.is_inline = true;
            }
            if existing.inline_data.is_none() && att.inline_data.is_some() {
                existing.inline_data.clone_from(&att.inline_data);
                existing.content_hash.clone_from(&att.content_hash);
            }
        },
    )
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
            let inline_data = if is_inline && !has_filename && part.mime_type.starts_with("image/")
            {
                body.data
                    .as_deref()
                    .and_then(decode_base64url_bytes)
                    .filter(|data| data.len() <= crate::inline_image_store::MAX_INLINE_SIZE)
            } else {
                None
            };
            let content_hash = inline_data
                .as_deref()
                .map(crate::attachment_cache::hash_bytes);

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
                content_hash,
                inline_data,
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
    let bytes = decode_base64url_bytes(data)?;
    String::from_utf8(bytes).ok()
}

fn decode_base64url_bytes(data: &str) -> Option<Vec<u8>> {
    decode_base64url_nopad(data).ok()
}
