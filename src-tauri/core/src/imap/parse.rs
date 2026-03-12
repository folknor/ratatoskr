use std::collections::HashMap;

use mail_parser::{MessageParser, MimeHeaders};
use xxhash_rust::xxh3::xxh3_64;

use super::types::*;

/// Detect special-use attribute from IMAP folder attributes and name heuristics.
pub fn detect_special_use(name: &async_imap::types::Name) -> Option<String> {
    use async_imap::types::NameAttribute;

    // Check RFC 6154 attributes first
    for attr in name.attributes() {
        let special = match attr {
            NameAttribute::Sent => Some("\\Sent"),
            NameAttribute::Trash => Some("\\Trash"),
            NameAttribute::Drafts => Some("\\Drafts"),
            NameAttribute::Junk => Some("\\Junk"),
            NameAttribute::Archive => Some("\\Archive"),
            NameAttribute::All => Some("\\All"),
            NameAttribute::Flagged => Some("\\Flagged"),
            _ => None,
        };
        if let Some(s) = special {
            return Some(s.to_string());
        }
    }

    // Heuristic fallback based on common folder names
    let lower = name.name().to_lowercase();
    match lower.as_str() {
        "inbox" => Some("\\Inbox".to_string()),
        "sent" | "sent messages" | "sent items" | "[gmail]/sent mail" => Some("\\Sent".to_string()),
        "trash" | "deleted" | "deleted items" | "deleted messages" | "bin" | "corbeille"
        | "unsolbox" | "[gmail]/trash" => Some("\\Trash".to_string()),
        "drafts" | "draft" | "draftbox" | "brouillons" | "[gmail]/drafts" => {
            Some("\\Drafts".to_string())
        }
        "junk" | "spam" | "junk e-mail" | "[gmail]/spam" => Some("\\Junk".to_string()),
        "archive" | "archives" | "[gmail]/all mail" => Some("\\Archive".to_string()),
        _ => None,
    }
}

/// Parse a raw email message into our ImapMessage struct.
///
/// `internal_date`: optional INTERNALDATE timestamp from the IMAP server,
/// used as fallback when the Date header cannot be parsed.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub fn parse_message(
    parser: &MessageParser,
    raw: &[u8],
    uid: u32,
    folder: &str,
    raw_size: u32,
    is_read: bool,
    is_starred: bool,
    is_draft: bool,
    internal_date: Option<i64>,
) -> Result<ImapMessage, String> {
    let message = parser.parse(raw).ok_or("Failed to parse MIME message")?;

    let message_id = message.message_id().map(ToString::to_string);
    let subject = message.subject().map(ToString::to_string);
    let date = message
        .date()
        .map(mail_parser::DateTime::to_timestamp)
        .or(internal_date)
        .unwrap_or(0);

    // In-Reply-To
    let in_reply_to = match message.in_reply_to() {
        mail_parser::HeaderValue::Text(t) => Some(t.to_string()),
        mail_parser::HeaderValue::TextList(list) => list.first().map(ToString::to_string),
        _ => None,
    };

    // References (space-separated message IDs)
    let references = match message.references() {
        mail_parser::HeaderValue::Text(t) => Some(t.to_string()),
        mail_parser::HeaderValue::TextList(list) => {
            if list.is_empty() {
                None
            } else {
                Some(list.iter().map(AsRef::as_ref).collect::<Vec<_>>().join(" "))
            }
        }
        _ => None,
    };

    // Addresses
    let (from_address, from_name) = extract_first_address(message.from());
    let to_addresses = format_address_list(message.to());
    let cc_addresses = format_address_list(message.cc());
    let bcc_addresses = format_address_list(message.bcc());
    let reply_to = format_address_list(message.reply_to());

    // Body
    let body_text = message.body_text(0).map(|s| s.to_string());
    let body_html = message.body_html(0).map(|s| s.to_string());

    // Generate snippet from text body (truncate at char boundary)
    let snippet = body_text.as_ref().map(|text| {
        let cleaned: String = text
            .chars()
            .map(|c| if c.is_whitespace() { ' ' } else { c })
            .collect();
        let trimmed = cleaned.trim();
        if trimmed.chars().count() > 200 {
            let end: String = trimmed.chars().take(200).collect();
            format!("{end}...")
        } else {
            trimmed.to_string()
        }
    });

    // List-Unsubscribe headers
    let list_unsubscribe =
        extract_header_text(message.header(mail_parser::HeaderName::ListUnsubscribe));
    let list_unsubscribe_post = extract_header_text(message.header(
        mail_parser::HeaderName::Other("List-Unsubscribe-Post".into()),
    ));

    // Authentication-Results header
    let auth_results = extract_header_text(message.header(mail_parser::HeaderName::Other(
        "Authentication-Results".into(),
    )));

    // Build a map from mail-parser part index → IMAP MIME section path.
    // IMAP numbers children of multipart containers starting at 1 (e.g. "1", "2", "1.2.3").
    // mail-parser stores all parts flat in a Vec, with Multipart variants holding child indices.
    let section_map = build_imap_section_map(&message);

    log::debug!(
        "IMAP parse UID {uid}: {} parts, {} attachment indices {:?}, section_map: {:?}",
        message.parts.len(),
        message.attachments.len(),
        message.attachments,
        section_map,
    );

    // Attachments — deduplicated by xxh3 content hash to collapse identical inline
    // parts that appear under different MIME sections (e.g. multipart/related + mixed).
    let attachments: Vec<ImapAttachment> = {
        let all: Vec<(u64, ImapAttachment)> = message
            .attachments
            .iter()
            .filter_map(|&part_idx| {
                let att = message.parts.get(part_idx as usize)?;
                let section = match section_map.get(&(part_idx as usize)) {
                    Some(s) => s.clone(),
                    None => {
                        log::warn!(
                            "IMAP UID {uid}: attachment at part index {part_idx} not found in section map (map has {} entries)",
                            section_map.len(),
                        );
                        return None;
                    }
                };

                let mime_type = att
                    .content_type()
                    .map(|ct| {
                        let ctype = ct.ctype();
                        let subtype = ct.subtype().unwrap_or("octet-stream");
                        format!("{ctype}/{subtype}")
                    })
                    .unwrap_or_else(|| "application/octet-stream".to_string());

                let contents = att.contents();
                let raw_hash = xxh3_64(contents);
                let content_hash = format!("{raw_hash:016x}");

                #[allow(clippy::cast_possible_truncation)]
                let size = att.len() as u32;
                let is_inline = att.content_disposition().is_some_and(mail_parser::ContentType::is_inline);

                // Carry raw bytes for small inline images so we can store them
                // in the inline image SQLite cache during sync.
                let inline_data = if is_inline
                    && (size as usize) <= crate::inline_image_store::MAX_INLINE_SIZE
                    && mime_type.starts_with("image/")
                {
                    Some(contents.to_vec())
                } else {
                    None
                };

                let attachment = ImapAttachment {
                    part_id: section,
                    filename: att
                        .attachment_name()
                        .unwrap_or("attachment")
                        .to_string(),
                    mime_type,
                    size,
                    content_id: att.content_id().map(ToString::to_string),
                    is_inline,
                    content_hash: Some(content_hash),
                    inline_data,
                };
                Some((raw_hash, attachment))
            })
            .collect();

        dedup_attachments_by_hash(all)
    };

    Ok(ImapMessage {
        uid,
        folder: folder.to_string(),
        message_id,
        in_reply_to,
        references,
        from_address,
        from_name,
        to_addresses,
        cc_addresses,
        bcc_addresses,
        reply_to,
        subject,
        date,
        is_read,
        is_starred,
        is_draft,
        body_html,
        body_text,
        snippet,
        raw_size,
        list_unsubscribe,
        list_unsubscribe_post,
        auth_results,
        attachments,
    })
}

/// Build a mapping from mail-parser part index → IMAP MIME section path string.
///
/// IMAP section numbering: children of a multipart container are numbered 1, 2, 3, ...
/// Nested multipart children get dot-separated paths (e.g., "1.2" for the 2nd child of the 1st child).
/// For non-multipart messages, the single body is section "1".
pub fn build_imap_section_map(
    message: &mail_parser::Message,
) -> std::collections::HashMap<usize, String> {
    use mail_parser::PartType;

    let mut map = std::collections::HashMap::new();

    fn walk(
        parts: &[mail_parser::MessagePart],
        part_idx: usize,
        prefix: &str,
        map: &mut std::collections::HashMap<usize, String>,
    ) {
        if let Some(part) = parts.get(part_idx) {
            if let PartType::Multipart(children) = &part.body {
                for (i, &child_idx) in children.iter().enumerate() {
                    let section = if prefix.is_empty() {
                        format!("{}", i + 1)
                    } else {
                        format!("{}.{}", prefix, i + 1)
                    };
                    walk(parts, child_idx as usize, &section, map);
                }
            } else {
                // Leaf part — use the section path as-is
                let section = if prefix.is_empty() {
                    // Non-multipart message: the body is section "1"
                    "1".to_string()
                } else {
                    prefix.to_string()
                };
                map.insert(part_idx, section);
            }
        }
    }

    // Start from part 0 (root) with empty prefix
    if !message.parts.is_empty() {
        walk(&message.parts, 0, "", &mut map);
    }

    map
}

/// Deduplicate attachments by content hash.
///
/// When multiple MIME parts have identical bytes (same xxh3 hash), keep only one.
/// Prefer the record with a real filename over "attachment", and prefer one with
/// a `content_id` so CID references in the HTML body resolve correctly.
fn dedup_attachments_by_hash(parts: Vec<(u64, ImapAttachment)>) -> Vec<ImapAttachment> {
    let mut seen: HashMap<u64, ImapAttachment> = HashMap::new();
    for (hash, mut att) in parts {
        seen.entry(hash)
            .and_modify(|existing| {
                // Prefer whichever has a real filename
                let existing_has_name = existing.filename != "attachment";
                let new_has_name = att.filename != "attachment";
                if !existing_has_name && new_has_name {
                    existing.filename = att.filename.clone();
                }
                // Prefer whichever has a content_id
                if existing.content_id.is_none() && att.content_id.is_some() {
                    existing.content_id.clone_from(&att.content_id);
                }
                // Mark as inline if either copy is
                if att.is_inline {
                    existing.is_inline = true;
                }
                // Keep inline_data if either copy has it
                if existing.inline_data.is_none() && att.inline_data.is_some() {
                    existing.inline_data = att.inline_data.take();
                }
            })
            .or_insert(att);
    }
    seen.into_values().collect()
}

/// Extract a text value from a HeaderValue, if present.
fn extract_header_text(hv: Option<&mail_parser::HeaderValue>) -> Option<String> {
    match hv {
        Some(mail_parser::HeaderValue::Text(t)) => Some(t.to_string()),
        Some(mail_parser::HeaderValue::TextList(list)) => Some(
            list.iter()
                .map(AsRef::as_ref)
                .collect::<Vec<_>>()
                .join(", "),
        ),
        _ => None,
    }
}

/// Extract the first address (email, display name) from an Address field.
fn extract_first_address(addr: Option<&mail_parser::Address>) -> (Option<String>, Option<String>) {
    let addr = match addr {
        Some(a) => a,
        None => return (None, None),
    };

    if let Some(first) = addr.first() {
        let email = first.address.as_ref().map(ToString::to_string);
        let name = first.name.as_ref().map(ToString::to_string);
        (email, name)
    } else {
        (None, None)
    }
}

/// Format an address list as a comma-separated string of "Name <email>" or "email".
fn format_address_list(addr: Option<&mail_parser::Address>) -> Option<String> {
    let addr = addr?;

    let parts: Vec<String> = addr
        .iter()
        .map(|a| {
            let email = a.address.as_deref().unwrap_or("");
            match a.name.as_deref() {
                Some(name) if !name.is_empty() => format!("{name} <{email}>"),
                _ => email.to_string(),
            }
        })
        .collect();

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}
