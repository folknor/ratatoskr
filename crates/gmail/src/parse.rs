use serde::{Deserialize, Serialize};

use common::attachment_dedup::{
    dedup_by_key, prefer_missing_clone, prefer_non_placeholder_filename,
};
use common::email_parsing::parse_single_address_header;
use common::encoding::decode_base64url_nopad;
use common::headers::find_header_value_case_insensitive;
use common::parsed_message::ParsedMessageBase;

use super::auth_parser::parse_authentication_results;
use super::types::{GmailHeader, GmailMessage, GmailPayload};

/// The MIME type used by Gmail for emoji reaction messages.
const REACTION_MIME_TYPE: &str = "text/vnd.google.email-reaction+json";

/// JSON payload inside a Gmail reaction MIME part.
#[derive(Debug, Deserialize)]
struct GmailReactionPayload {
    emoji: String,
    // version field is present but unused
}

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
    /// Common fields shared with other providers.
    #[serde(flatten)]
    pub base: ParsedMessageBase,
    pub attachments: Vec<ParsedAttachment>,
    /// True when this message is a Gmail emoji reaction (not a real message).
    pub is_reaction: bool,
    /// The emoji from a reaction message, if any.
    pub reaction_emoji: Option<String>,
}

common::impl_message_addresses!(ParsedGmailMessage);

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

    // Detect Gmail emoji reaction messages
    let reaction_emoji = payload.and_then(extract_reaction_emoji);
    let is_reaction = reaction_emoji.is_some();

    let internal_date = msg
        .internal_date
        .as_deref()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);

    ParsedGmailMessage {
        base: ParsedMessageBase {
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
            list_unsubscribe: get_header(headers, "List-Unsubscribe"),
            list_unsubscribe_post: get_header(headers, "List-Unsubscribe-Post"),
            auth_results,
            mdn_requested: get_header(headers, "Disposition-Notification-To").is_some(),
            message_id_header: get_header(headers, "Message-ID")
                .or_else(|| get_header(headers, "Message-Id")),
            references_header: get_header(headers, "References"),
            in_reply_to_header: get_header(headers, "In-Reply-To"),
        },
        attachments,
        is_reaction,
        reaction_emoji,
    }
}

fn get_header(headers: &[GmailHeader], name: &str) -> Option<String> {
    find_header_value_case_insensitive(headers, name, |h| h.name.as_str(), |h| h.value.as_str())
}

/// Recursively extract a body part matching the given MIME type.
///
/// Skips `text/x-amp-html` parts - AMP emails contain tracking-heavy
/// interactive content that should never be selected as the body.
fn extract_body(part: &GmailPayload, mime_type: &str) -> Option<String> {
    if !common::email_parsing::is_amp_content_type(&part.mime_type)
        && part.mime_type == mime_type
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
                    .filter(|data| data.len() <= store::inline_image_store::MAX_INLINE_SIZE)
            } else {
                None
            };
            let content_hash = inline_data
                .as_deref()
                .map(store::attachment_cache::hash_bytes);

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

/// Recursively search for a `text/vnd.google.email-reaction+json` MIME part
/// and parse the emoji from its JSON body.
fn extract_reaction_emoji(part: &GmailPayload) -> Option<String> {
    if part.mime_type == REACTION_MIME_TYPE {
        if let Some(body) = &part.body
            && let Some(data) = &body.data
            && let Some(decoded) = decode_base64url(data)
            && let Ok(payload) = serde_json::from_str::<GmailReactionPayload>(&decoded)
        {
            return Some(payload.emoji);
        }
        return None;
    }

    for child in &part.parts {
        if let Some(emoji) = extract_reaction_emoji(child) {
            return Some(emoji);
        }
    }

    None
}

/// Decode Gmail's base64url-encoded body data to a UTF-8 string.
fn decode_base64url(data: &str) -> Option<String> {
    let bytes = decode_base64url_bytes(data)?;
    String::from_utf8(bytes).ok()
}

fn decode_base64url_bytes(data: &str) -> Option<Vec<u8>> {
    decode_base64url_nopad(data).ok()
}
