use super::folder_mapper::FolderMap;
use super::types::{GraphMessage, GraphRecipient, LAST_VERB_EXECUTED_PROPERTY_ID, REACTIONS_GUID};
use common::email_parsing::format_address_list;
use common::encoding::decode_base64_standard;
use common::headers::find_header_value_case_insensitive;
use common::parsed_message::ParsedMessageBase;
use common::types::ImportanceLevel;

/// Parsed attachment metadata ready for DB persistence.
#[derive(Debug, Clone)]
pub struct ParsedGraphAttachment {
    pub id: String,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub size: Option<i64>,
    pub is_inline: bool,
    pub content_hash: Option<db::blob_hash::BlobHash>,
    pub inline_data: Option<Vec<u8>>,
    pub content_id: Option<String>,
}

/// Parsed Graph message ready for DB persistence.
///
/// Matches the shape written to the `messages` table + body store.
/// Analogous to `jmap/parse.rs::ParsedJmapMessage`.
#[derive(Debug, Clone)]
pub struct ParsedGraphMessage {
    /// Common fields shared with other providers.
    pub base: ParsedMessageBase,
    pub attachments: Vec<ParsedGraphAttachment>,
    /// Exchange categories assigned to this message (e.g. "Red Category", "Blue Category").
    pub categories: Vec<String>,
    /// The authenticated user's reaction emoji from Exchange extended properties.
    pub owner_reaction_type: Option<String>,
    /// Total reactions count from Exchange extended properties.
    pub reactions_count: Option<i64>,
}

common::impl_message_addresses!(ParsedGraphMessage);

/// Convert a Graph API message to our DB-ready struct.
///
/// Graph-specific parsing:
/// - Body comes as a string (`body.content`), not MIME parts
/// - Threading uses `conversationId` (provisional - see plan Open Question 3)
/// - Headers must be explicitly requested via `$select=internetMessageHeaders`
/// - Membership IDs derived from folder, categories, and importance
pub fn parse_graph_message(
    msg: &GraphMessage,
    folder_map: &FolderMap,
) -> Result<ParsedGraphMessage, String> {
    let id = msg.id.clone();

    // Thread ID: use conversationId (provisional - see plan Open Question 3)
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
    let (is_replied, is_forwarded) = extract_last_verb_state(msg);

    // Body: Graph provides body as a single content string (html or text)
    let (body_html, body_text) = extract_body(msg);

    // Membership IDs from folder, categories, and synthesized importance labels.
    let parent_folder = msg.parent_folder_id.as_deref().unwrap_or("");
    let categories = msg.categories.as_deref().unwrap_or(&[]);
    let mut label_ids =
        folder_map.get_folder_and_label_ids_for_message(parent_folder, categories)?;
    if let Some(label_id) = importance_label_id(msg.importance.as_deref()) {
        label_ids.push(label_id.to_string());
    }

    // Internet headers (must be explicitly requested via $select)
    let headers = &msg.internet_message_headers;
    let message_id_header = get_header(headers, "Message-ID");
    let references_header = get_header(headers, "References");
    let in_reply_to_header = get_header(headers, "In-Reply-To");
    let auth_results = get_header(headers, "Authentication-Results");
    let list_unsubscribe = get_header(headers, "List-Unsubscribe");
    let list_unsubscribe_post = get_header(headers, "List-Unsubscribe-Post");
    // Prefer native Graph boolean; fall back to header detection
    let mdn_requested = msg
        .is_read_receipt_requested
        .unwrap_or_else(|| get_header(headers, "Disposition-Notification-To").is_some());

    // Exchange reaction extended properties
    let (owner_reaction_type, reactions_count) = extract_reaction_properties(msg);

    // Attachments. Graph returns three @odata.type kinds:
    //   - `#microsoft.graph.fileAttachment` (raw bytes; supports `$value`)
    //   - `#microsoft.graph.itemAttachment` (nested item; the $value
    //     endpoint returns the nested message, not the kind of bytes
    //     the attachment store knows what to do with)
    //   - `#microsoft.graph.referenceAttachment` (URL to a cloud-hosted
    //     file; no bytes at all)
    //
    // Only fileAttachment has bytes the cache can store. Skip the
    // rest at parse time so PrefetchRuntime never tries `$value`
    // against a referenceAttachment (which Graph rejects) and the
    // attachments table doesn't accumulate rows no provider fetch
    // can satisfy. Cloud-link surfacing for reference attachments
    // lives in the cloud-attachments path (docs/cloud-attachments.md)
    // and is independent of this byte cache.
    let attachments: Vec<ParsedGraphAttachment> = msg
        .attachments
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .filter(|a| {
            let kind = a.odata_type.as_deref().unwrap_or("");
            // Treat missing/unknown discriminator as fileAttachment
            // for back-compat with any fixture or older payload that
            // didn't include @odata.type. Only known non-file kinds
            // are skipped.
            !matches!(
                kind,
                "#microsoft.graph.referenceAttachment" | "#microsoft.graph.itemAttachment"
            )
        })
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
                content_hash: inline_data.as_deref().map(db::blob_hash::BlobHash::hash),
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
            is_replied,
            is_forwarded,
            body_html,
            body_text,
            raw_size: 0, // Graph doesn't expose message size directly
            internal_date,
            label_ids,
            has_attachments,
            message_id_header,
            references_header,
            in_reply_to_header,
            auth_results,
            list_unsubscribe,
            list_unsubscribe_post,
            mdn_requested,
        },
        attachments,
        categories: categories.to_vec(),
        owner_reaction_type,
        reactions_count,
    })
}

fn importance_label_id(importance: Option<&str>) -> Option<&'static str> {
    importance
        .and_then(ImportanceLevel::from_graph_value)
        .map(ImportanceLevel::label_id)
}

fn extract_last_verb_state(msg: &GraphMessage) -> (bool, bool) {
    let props = match &msg.single_value_extended_properties {
        Some(p) if !p.is_empty() => p,
        _ => return (false, false),
    };

    let mut is_replied = false;
    let mut is_forwarded = false;
    for prop in props {
        if prop.id.eq_ignore_ascii_case(LAST_VERB_EXECUTED_PROPERTY_ID)
            && let Ok(value) = prop.value.trim().parse::<i64>()
        {
            is_replied = matches!(value, 102 | 103);
            is_forwarded = value == 104;
        }
    }
    (is_replied, is_forwarded)
}

/// Extract Exchange reaction extended properties from a Graph message.
///
/// Looks for `OwnerReactionType` (string) and `ReactionsCount` (integer)
/// under the GUID `{41F28F13-83F4-4114-A584-EEDB5A6B0BFF}`.
fn extract_reaction_properties(msg: &GraphMessage) -> (Option<String>, Option<i64>) {
    let props = match &msg.single_value_extended_properties {
        Some(p) if !p.is_empty() => p,
        _ => return (None, None),
    };

    let owner_reaction_id = format!("String {REACTIONS_GUID} Name OwnerReactionType");
    let reactions_count_id = format!("Integer {REACTIONS_GUID} Name ReactionsCount");

    let mut owner_reaction: Option<String> = None;
    let mut reactions_count: Option<i64> = None;

    for prop in props {
        if prop.id.eq_ignore_ascii_case(&owner_reaction_id) {
            let val = prop.value.trim();
            if !val.is_empty() {
                owner_reaction = Some(val.to_string());
            }
        } else if prop.id.eq_ignore_ascii_case(&reactions_count_id) {
            reactions_count = prop.value.trim().parse::<i64>().ok();
        }
    }

    (owner_reaction, reactions_count)
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
    if decoded.len() > store::inline_image_store::MAX_INLINE_SIZE {
        return None;
    }
    Some(decoded)
}

/// Parse an ISO 8601 date string to a `DateTime<Utc>`.
///
/// Tries RFC 3339 first (has timezone), then falls back to naive datetime
/// formats without timezone (assumed UTC). Graph API and EWS sometimes
/// return dates without a timezone suffix.
///
/// Callers choose their own resolution: `.timestamp_millis()` for message
/// dates, `.timestamp()` for sync timestamps, etc.
pub(crate) fn parse_iso_datetime(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    // Try RFC 3339 first (has timezone)
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.to_utc());
    }
    // Fallback: naive datetime without timezone (assume UTC), with fractional seconds
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
        return Some(dt.and_utc());
    }
    // Fallback: naive datetime without timezone (assume UTC), no fractional seconds
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Some(dt.and_utc());
    }
    None
}

/// Parse an ISO 8601 date string to epoch milliseconds.
fn parse_iso_date(s: &str) -> Option<i64> {
    parse_iso_datetime(s).map(|dt| dt.timestamp_millis())
}
