use super::folder_mapper::FolderMap;
use super::types::{GraphMessage, GraphRecipient};
use crate::provider::email_parsing::format_address_list;
use crate::provider::encoding::decode_base64_standard;
use crate::provider::headers::find_header_value_case_insensitive;

/// Parsed attachment metadata ready for DB persistence.
#[derive(Debug, Clone)]
pub struct ParsedGraphAttachment {
    pub id: String,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub size: Option<i64>,
    pub is_inline: bool,
    pub content_hash: Option<String>,
    pub inline_data: Option<Vec<u8>>,
    pub content_id: Option<String>,
}

/// Parsed Graph message ready for DB persistence.
///
/// Matches the shape written to the `messages` table + body store.
/// Analogous to `jmap/parse.rs::ParsedJmapMessage`.
#[derive(Debug, Clone)]
pub struct ParsedGraphMessage {
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
    pub internal_date: i64,
    pub label_ids: Vec<String>,
    pub has_attachments: bool,
    pub message_id_header: Option<String>,
    pub references_header: Option<String>,
    pub in_reply_to_header: Option<String>,
    pub auth_results: Option<String>,
    pub list_unsubscribe: Option<String>,
    pub list_unsubscribe_post: Option<String>,
    pub attachments: Vec<ParsedGraphAttachment>,
}

impl crate::seen_addresses::MessageAddresses for ParsedGraphMessage {
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

/// Convert a Graph API message to our DB-ready struct.
///
/// Graph-specific parsing:
/// - Body comes as a string (`body.content`), not MIME parts
/// - Threading uses `conversationId` (provisional — see plan Open Question 3)
/// - Headers must be explicitly requested via `$select=internetMessageHeaders`
/// - Labels derived from folder + categories + read/starred flags
pub fn parse_graph_message(
    msg: &GraphMessage,
    folder_map: &FolderMap,
) -> Result<ParsedGraphMessage, String> {
    let id = msg.id.clone();

    // Thread ID: use conversationId (provisional — see plan Open Question 3)
    let thread_id = msg
        .conversation_id
        .clone()
        .unwrap_or_else(|| msg.id.clone());

    // Sender
    let from_address = msg.from.as_ref().map(|r| r.email_address.address.clone());
    let from_name = msg.from.as_ref().and_then(|r| r.email_address.name.clone());

    // Recipients
    let to_addresses = format_recipients(msg.to_recipients.as_deref());
    let cc_addresses = format_recipients(msg.cc_recipients.as_deref());
    let bcc_addresses = format_recipients(msg.bcc_recipients.as_deref());
    let reply_to = format_recipients(msg.reply_to.as_deref());

    let subject = msg.subject.clone();
    let snippet = msg.body_preview.clone().unwrap_or_default();

    // Dates: ISO 8601 → epoch milliseconds
    let sent_date = msg.sent_date_time.as_deref().and_then(parse_iso_date);
    let received_date = msg.received_date_time.as_deref().and_then(parse_iso_date);
    let date = sent_date.or(received_date).unwrap_or(0);
    let internal_date = received_date.unwrap_or(date);

    // Flags
    let is_read = msg.is_read.unwrap_or(false);
    let is_starred = msg
        .flag
        .as_ref()
        .is_some_and(|f| f.flag_status == "flagged");
    let has_attachments = msg.has_attachments.unwrap_or(false);

    // Body: Graph provides body as a single content string (html or text)
    let (body_html, body_text) = extract_body(msg);

    // Labels from folder + categories + flags
    let parent_folder = msg.parent_folder_id.as_deref().unwrap_or("");
    let categories = msg.categories.as_deref().unwrap_or(&[]);
    let flag_status = msg
        .flag
        .as_ref()
        .map(|f| f.flag_status.as_str())
        .unwrap_or("notFlagged");
    let label_ids =
        folder_map.get_labels_for_message(parent_folder, categories, is_read, flag_status);

    // Internet headers (must be explicitly requested via $select)
    let headers = &msg.internet_message_headers;
    let message_id_header = get_header(headers, "Message-ID");
    let references_header = get_header(headers, "References");
    let in_reply_to_header = get_header(headers, "In-Reply-To");
    let auth_results = get_header(headers, "Authentication-Results");
    let list_unsubscribe = get_header(headers, "List-Unsubscribe");
    let list_unsubscribe_post = get_header(headers, "List-Unsubscribe-Post");

    // Attachments
    let attachments: Vec<ParsedGraphAttachment> = msg
        .attachments
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .map(|a| {
            let is_inline = a.is_inline.unwrap_or(false);
            let mime_type = a.content_type.clone();
            let inline_data = if is_inline
                && mime_type
                    .as_deref()
                    .is_some_and(|v| v.starts_with("image/"))
            {
                a.content_bytes.as_deref().and_then(decode_inline_bytes)
            } else {
                None
            };

            ParsedGraphAttachment {
                content_hash: inline_data
                    .as_deref()
                    .map(crate::attachment_cache::hash_bytes),
                inline_data,
                id: a.id.clone(),
                filename: a.name.clone(),
                mime_type,
                size: a.size,
                is_inline,
                content_id: a.content_id.clone(),
            }
        })
        .collect();

    Ok(ParsedGraphMessage {
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
        internal_date,
        label_ids,
        has_attachments,
        message_id_header,
        references_header,
        in_reply_to_header,
        auth_results,
        list_unsubscribe,
        list_unsubscribe_post,
        attachments,
    })
}

/// Extract body HTML and text from a Graph message.
///
/// Graph returns a single `body` with `contentType` of "html" or "text".
/// Unlike Gmail (base64-encoded MIME parts) or JMAP (bodyValues), this is
/// a plain string.
fn extract_body(msg: &GraphMessage) -> (Option<String>, Option<String>) {
    let mut html = None;
    let mut text = None;

    if let Some(body) = &msg.body {
        match body.content_type.as_str() {
            "html" => html = Some(body.content.clone()),
            "text" => text = Some(body.content.clone()),
            _ => {}
        }
    }

    (html, text)
}

/// Look up a header value by name (case-insensitive).
fn get_header(
    headers: &Option<Vec<super::types::GraphInternetHeader>>,
    name: &str,
) -> Option<String> {
    find_header_value_case_insensitive(
        headers.as_ref()?,
        name,
        |h| h.name.as_str(),
        |h| h.value.as_str(),
    )
}

/// Format Graph recipients to "Name <email>, ..." string.
fn format_recipients(recipients: Option<&[GraphRecipient]>) -> Option<String> {
    let recipients = recipients?;
    format_address_list(recipients.iter().map(|r| {
        (
            r.email_address.name.clone(),
            r.email_address.address.clone(),
        )
    }))
}

fn decode_inline_bytes(data: &str) -> Option<Vec<u8>> {
    let decoded = decode_base64_standard(data).ok()?;
    if decoded.len() > crate::inline_image_store::MAX_INLINE_SIZE {
        return None;
    }
    Some(decoded)
}

/// Parse an ISO 8601 date string to epoch milliseconds.
///
/// Tries RFC 3339 first (has timezone), then falls back to naive datetime
/// formats without timezone (assumed UTC). Graph API sometimes returns dates
/// without a timezone suffix.
fn parse_iso_date(s: &str) -> Option<i64> {
    // Try RFC 3339 first (has timezone)
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }
    // Fallback: naive datetime without timezone (assume UTC), with fractional seconds
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
        return Some(dt.and_utc().timestamp_millis());
    }
    // Fallback: naive datetime without timezone (assume UTC), no fractional seconds
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Some(dt.and_utc().timestamp_millis());
    }
    None
}
