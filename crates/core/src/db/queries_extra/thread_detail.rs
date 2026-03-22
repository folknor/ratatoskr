//! Thread detail data layer — everything needed for the conversation view.

use std::collections::{HashMap, HashSet};

/// Map from message ID to (text_body, html_body).
type BodyMap = HashMap<String, (Option<String>, Option<String>)>;

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::db::queries_extra::thread_ui_state::get_attachments_collapsed;
use crate::label_colors::resolve_label_color;
use crate::provider::folder_roles::SYSTEM_FOLDER_ROLES;

// ── Return types ────────────────────────────────────────────

/// A single message within a thread detail response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadDetailMessage {
    pub id: String,
    pub thread_id: String,
    pub account_id: String,

    // Sender
    pub from_address: Option<String>,
    pub from_name: Option<String>,

    // Recipients (raw JSON strings from the messages table)
    pub to_addresses: Option<String>,
    pub cc_addresses: Option<String>,
    pub bcc_addresses: Option<String>,

    // Timestamps
    pub date: i64,

    // Content
    pub subject: Option<String>,
    pub body_html: Option<String>,
    pub body_text: Option<String>,

    // Flags
    pub is_read: bool,
    pub is_starred: bool,

    // Computed fields
    pub is_own_message: bool,
    pub collapsed_summary: Option<String>,
}

/// Labels with resolved colors, for the thread header label toggles
/// and thread card label dots.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadLabel {
    pub label_id: String,
    pub name: String,
    pub color_bg: String,
    pub color_fg: String,
}

/// Attachments grouped for the conversation view's attachment panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadAttachment {
    pub id: String,
    pub message_id: String,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub size: Option<i64>,
    pub content_id: Option<String>,
    pub is_inline: bool,
    pub local_path: Option<String>,
    pub content_hash: Option<String>,
    pub gmail_attachment_id: Option<String>,
    // Context from parent message
    pub from_name: Option<String>,
    pub from_address: Option<String>,
    pub date: i64,
}

/// Complete thread detail response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadDetail {
    pub thread_id: String,
    pub account_id: String,
    pub subject: Option<String>,
    pub is_starred: bool,
    pub is_snoozed: bool,
    pub is_pinned: bool,
    pub is_muted: bool,

    /// Messages ordered newest-first (conversation view order).
    pub messages: Vec<ThreadDetailMessage>,

    /// Labels with resolved colors.
    pub labels: Vec<ThreadLabel>,

    /// Non-inline attachments across all messages, ordered by date desc.
    pub attachments: Vec<ThreadAttachment>,

    /// Whether the attachment group is collapsed (persisted per-thread).
    pub attachments_collapsed: bool,
}

// ── Thread metadata (from threads table) ────────────────────

struct ThreadMeta {
    subject: Option<String>,
    is_starred: bool,
    is_snoozed: bool,
    is_pinned: bool,
    is_muted: bool,
}

fn query_thread_meta(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
) -> Result<ThreadMeta, String> {
    conn.query_row(
        "SELECT subject, is_starred, is_snoozed, is_pinned, is_muted \
         FROM threads WHERE account_id = ?1 AND id = ?2",
        params![account_id, thread_id],
        |row| {
            Ok(ThreadMeta {
                subject: row.get("subject")?,
                is_starred: row.get::<_, i64>("is_starred")? != 0,
                is_snoozed: row.get::<_, i64>("is_snoozed")? != 0,
                is_pinned: row.get::<_, i64>("is_pinned")? != 0,
                is_muted: row.get::<_, i64>("is_muted")? != 0,
            })
        },
    )
    .map_err(|e| format!("query thread meta: {e}"))
}

// ── Messages ────────────────────────────────────────────────

fn query_messages(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
) -> Result<Vec<ThreadDetailMessage>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, thread_id, account_id, from_address, from_name, \
                    to_addresses, cc_addresses, bcc_addresses, \
                    subject, date, is_read, is_starred \
             FROM messages \
             WHERE account_id = ?1 AND thread_id = ?2 \
             ORDER BY date DESC",
        )
        .map_err(|e| format!("prepare messages: {e}"))?;

    let rows = stmt
        .query_map(params![account_id, thread_id], |row| {
            Ok(ThreadDetailMessage {
                id: row.get("id")?,
                thread_id: row.get("thread_id")?,
                account_id: row.get("account_id")?,
                from_address: row.get("from_address")?,
                from_name: row.get("from_name")?,
                to_addresses: row.get("to_addresses")?,
                cc_addresses: row.get("cc_addresses")?,
                bcc_addresses: row.get("bcc_addresses")?,
                date: row.get("date")?,
                subject: row.get("subject")?,
                is_read: row.get::<_, i64>("is_read")? != 0,
                is_starred: row.get::<_, i64>("is_starred")? != 0,
                // Populated later
                body_html: None,
                body_text: None,
                is_own_message: false,
                collapsed_summary: None,
            })
        })
        .map_err(|e| format!("query messages: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("map messages: {e}"))?;

    Ok(rows)
}

// ── Body store (zstd-compressed bodies from separate DB) ────

fn decompress_body(data: &[u8]) -> Result<String, String> {
    let bytes = zstd::decode_all(data).map_err(|e| format!("zstd decompress: {e}"))?;
    String::from_utf8(bytes).map_err(|e| format!("utf8 decode: {e}"))
}

fn fetch_bodies(
    body_store_conn: &Connection,
    message_ids: &[String],
) -> Result<BodyMap, String> {
    let mut body_map: BodyMap = HashMap::new();
    if message_ids.is_empty() {
        return Ok(body_map);
    }

    for chunk in message_ids.chunks(100) {
        fetch_bodies_chunk(body_store_conn, chunk, &mut body_map)?;
    }

    Ok(body_map)
}

fn fetch_bodies_chunk(
    conn: &Connection,
    chunk: &[String],
    body_map: &mut BodyMap,
) -> Result<(), String> {
    let placeholders: String = chunk
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(", ");

    let sql = format!(
        "SELECT message_id, body_html, body_text FROM bodies \
         WHERE message_id IN ({placeholders})"
    );

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("prepare body batch: {e}"))?;

    let param_values: Vec<Box<dyn rusqlite::types::ToSql>> = chunk
        .iter()
        .map(|id| Box::new(id.clone()) as Box<dyn rusqlite::types::ToSql>)
        .collect();
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(AsRef::as_ref).collect();

    let rows = stmt
        .query_map(param_refs.as_slice(), |row| {
            let mid: String = row.get("message_id")?;
            let html_blob: Option<Vec<u8>> = row.get("body_html")?;
            let text_blob: Option<Vec<u8>> = row.get("body_text")?;
            Ok((mid, html_blob, text_blob))
        })
        .map_err(|e| format!("query body batch: {e}"))?;

    for row in rows {
        let (mid, html_blob, text_blob) = row.map_err(|e| format!("map body row: {e}"))?;
        let body_html = html_blob.map(|b| decompress_body(&b)).transpose()?;
        let body_text = text_blob.map(|b| decompress_body(&b)).transpose()?;
        body_map.insert(mid, (body_html, body_text));
    }

    Ok(())
}

// ── Identity detection ──────────────────────────────────────

fn query_identity_emails(
    conn: &Connection,
    account_id: &str,
) -> Result<HashSet<String>, String> {
    let mut emails = HashSet::new();

    let mut stmt = conn
        .prepare(
            "SELECT email FROM send_identities WHERE account_id = ?1 \
             UNION \
             SELECT email FROM send_as_aliases WHERE account_id = ?1 \
             UNION \
             SELECT email FROM accounts WHERE id = ?1",
        )
        .map_err(|e| format!("prepare identity emails: {e}"))?;

    let rows = stmt
        .query_map(params![account_id], |row| row.get::<_, String>(0))
        .map_err(|e| format!("query identity emails: {e}"))?;

    for row in rows {
        let email = row.map_err(|e| format!("map identity email: {e}"))?;
        emails.insert(email.to_lowercase());
    }

    Ok(emails)
}

// ── Collapsed summary ───────────────────────────────────────

/// Maximum length of a collapsed summary before truncation.
const SUMMARY_MAX_LEN: usize = 60;

/// Strip quoted lines and signature blocks, return the first ~60 chars
/// of meaningful body text.
fn make_collapsed_summary(
    body_text: Option<&str>,
    body_html: Option<&str>,
) -> Option<String> {
    let source = match (body_text, body_html) {
        (Some(text), _) if !text.trim().is_empty() => text.to_string(),
        (_, Some(html)) if !html.trim().is_empty() => strip_html_tags(html),
        _ => return None,
    };

    let meaningful = extract_meaningful_lines(&source);
    if meaningful.is_empty() {
        return None;
    }

    Some(truncate_summary(&meaningful))
}

/// Minimal HTML tag stripper for summary generation.
fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;

    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                result.push(' ');
            }
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }

    decode_html_entities(&result)
}

/// Decode common HTML entities.
fn decode_html_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

/// Extract non-quoted, non-signature lines from body text.
fn extract_meaningful_lines(text: &str) -> String {
    let mut parts = Vec::new();

    for line in text.lines() {
        // Stop at signature delimiter (RFC 3676: "-- " with trailing space)
        if line == "-- " || line.starts_with("-- \n") {
            break;
        }
        // Skip quoted lines (leading `>` after optional whitespace)
        let trimmed = line.trim_start();
        if trimmed.starts_with('>') {
            continue;
        }
        // Skip blank lines
        if trimmed.is_empty() {
            continue;
        }
        parts.push(trimmed);
    }

    // Collapse into a single line
    let joined = parts.join(" ");
    // Collapse runs of whitespace
    joined.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Truncate to `SUMMARY_MAX_LEN` chars, appending `...` if needed.
fn truncate_summary(text: &str) -> String {
    if text.len() <= SUMMARY_MAX_LEN {
        return text.to_string();
    }

    // Find a break point near the limit (don't split mid-word if easy)
    let truncated: String = text.chars().take(SUMMARY_MAX_LEN).collect();
    format!("{truncated}...")
}

// ── Labels with resolved colors ─────────────────────────────

fn query_thread_labels(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
) -> Result<Vec<ThreadLabel>, String> {
    let system_ids: HashSet<&str> = SYSTEM_FOLDER_ROLES
        .iter()
        .map(|r| r.label_id)
        .collect();

    let mut stmt = conn
        .prepare(
            "SELECT l.id, l.name, l.color_bg, l.color_fg, l.account_id \
             FROM thread_labels tl \
             JOIN labels l ON l.account_id = tl.account_id AND l.id = tl.label_id \
             WHERE tl.account_id = ?1 AND tl.thread_id = ?2 \
             ORDER BY l.sort_order ASC, l.name ASC",
        )
        .map_err(|e| format!("prepare thread labels: {e}"))?;

    let rows = stmt
        .query_map(params![account_id, thread_id], |row| {
            Ok(LabelRow {
                id: row.get("id")?,
                name: row.get("name")?,
                color_bg: row.get("color_bg")?,
                color_fg: row.get("color_fg")?,
                account_id: row.get("account_id")?,
            })
        })
        .map_err(|e| format!("query thread labels: {e}"))?;

    let mut labels = Vec::new();
    for row in rows {
        let lr = row.map_err(|e| format!("map label row: {e}"))?;

        // Filter out system labels
        if system_ids.contains(lr.id.as_str()) {
            continue;
        }

        // Build a minimal DbLabel to pass to resolve_label_color
        let db_label = crate::db::types::DbLabel {
            id: lr.id.clone(),
            account_id: lr.account_id,
            name: lr.name.clone(),
            label_type: None,
            label_kind: "container".to_string(),
            color_bg: lr.color_bg,
            color_fg: lr.color_fg,
            visible: true,
            sort_order: 0,
            imap_folder_path: None,
            imap_special_use: None,
            parent_label_id: None,
            right_read: None,
            right_add: None,
            right_remove: None,
            right_set_seen: None,
            right_set_keywords: None,
            right_create_child: None,
            right_rename: None,
            right_delete: None,
            right_submit: None,
        };

        let (bg, fg) = resolve_label_color(&db_label);
        labels.push(ThreadLabel {
            label_id: lr.id,
            name: lr.name,
            color_bg: bg.to_string(),
            color_fg: fg.to_string(),
        });
    }

    Ok(labels)
}

/// Intermediate row for label queries.
struct LabelRow {
    id: String,
    name: String,
    color_bg: Option<String>,
    color_fg: Option<String>,
    account_id: String,
}

// ── Attachments with message context ────────────────────────

fn query_thread_attachments(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
) -> Result<Vec<ThreadAttachment>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT a.id, a.message_id, a.filename, a.mime_type, a.size, \
                    a.content_id, a.is_inline, a.local_path, a.content_hash, \
                    a.gmail_attachment_id, \
                    m.from_name, m.from_address, m.date \
             FROM attachments a \
             JOIN messages m ON a.message_id = m.id AND a.account_id = m.account_id \
             WHERE a.account_id = ?1 AND m.thread_id = ?2 \
               AND a.is_inline = 0 \
               AND a.filename IS NOT NULL AND a.filename != '' \
             ORDER BY m.date DESC",
        )
        .map_err(|e| format!("prepare attachments: {e}"))?;

    let rows = stmt
        .query_map(params![account_id, thread_id], |row| {
            Ok(ThreadAttachment {
                id: row.get("id")?,
                message_id: row.get("message_id")?,
                filename: row.get("filename")?,
                mime_type: row.get("mime_type")?,
                size: row.get("size")?,
                content_id: row.get("content_id")?,
                is_inline: row.get::<_, i64>("is_inline")? != 0,
                local_path: row.get("local_path")?,
                content_hash: row.get("content_hash")?,
                gmail_attachment_id: row.get("gmail_attachment_id")?,
                from_name: row.get("from_name")?,
                from_address: row.get("from_address")?,
                date: row.get("date")?,
            })
        })
        .map_err(|e| format!("query attachments: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("map attachments: {e}"))?;

    Ok(rows)
}

// ── Main entry point ────────────────────────────────────────

/// Fetch everything needed to render the conversation view for a thread.
///
/// Joins thread metadata, messages (with bodies from BodyStore), labels
/// (with resolved colors), attachments, and attachment collapse state.
/// Each message is annotated with `is_own_message` (ownership detection)
/// and `collapsed_summary` (quote/signature-stripped preview).
pub fn get_thread_detail(
    conn: &Connection,
    body_store_conn: &Connection,
    account_id: &str,
    thread_id: &str,
) -> Result<ThreadDetail, String> {
    log::debug!("Loading thread detail: thread_id={thread_id}, account_id={account_id}");
    let meta = query_thread_meta(conn, account_id, thread_id).map_err(|e| {
        log::error!("Failed to load thread meta: thread_id={thread_id}, error={e}");
        e
    })?;
    let mut messages = query_messages(conn, account_id, thread_id)?;

    // Fetch and attach bodies
    let message_ids: Vec<String> = messages.iter().map(|m| m.id.clone()).collect();
    let body_map = fetch_bodies(body_store_conn, &message_ids)?;
    for msg in &mut messages {
        if let Some((html, text)) = body_map.get(&msg.id) {
            msg.body_html.clone_from(html);
            msg.body_text.clone_from(text);
        }
    }

    // Detect ownership and generate summaries
    let identity_emails = query_identity_emails(conn, account_id)?;
    annotate_messages(&mut messages, &identity_emails);

    let labels = query_thread_labels(conn, account_id, thread_id)?;
    let attachments = query_thread_attachments(conn, account_id, thread_id)?;
    let attachments_collapsed = get_attachments_collapsed(conn, account_id, thread_id)?;

    let detail = ThreadDetail {
        thread_id: thread_id.to_string(),
        account_id: account_id.to_string(),
        subject: meta.subject,
        is_starred: meta.is_starred,
        is_snoozed: meta.is_snoozed,
        is_pinned: meta.is_pinned,
        is_muted: meta.is_muted,
        messages,
        labels,
        attachments,
        attachments_collapsed,
    };
    log::debug!(
        "Thread detail loaded: thread_id={thread_id}, messages={}, labels={}, attachments={}",
        detail.messages.len(),
        detail.labels.len(),
        detail.attachments.len(),
    );
    Ok(detail)
}

/// Set `is_own_message` and `collapsed_summary` on each message.
fn annotate_messages(
    messages: &mut [ThreadDetailMessage],
    identity_emails: &HashSet<String>,
) {
    for msg in messages.iter_mut() {
        // Ownership detection
        msg.is_own_message = msg
            .from_address
            .as_ref()
            .is_some_and(|addr| identity_emails.contains(&addr.to_lowercase()));

        // Collapsed summary
        msg.collapsed_summary = make_collapsed_summary(
            msg.body_text.as_deref(),
            msg.body_html.as_deref(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Summary generation tests ────────────────────────────

    #[test]
    fn summary_strips_quotes() {
        let text = "> This is quoted\nHello, this is the actual message.";
        let summary = make_collapsed_summary(Some(text), None);
        assert_eq!(
            summary.as_deref(),
            Some("Hello, this is the actual message.")
        );
    }

    #[test]
    fn summary_strips_signature() {
        let text = "Main content here.\n-- \nJohn Doe\njohn@example.com";
        let summary = make_collapsed_summary(Some(text), None);
        assert_eq!(summary.as_deref(), Some("Main content here."));
    }

    #[test]
    fn summary_truncates_long_text() {
        let long_text = "A".repeat(100);
        let summary = make_collapsed_summary(Some(&long_text), None);
        let s = summary.expect("should produce summary");
        assert!(s.ends_with("..."));
        // 60 chars + "..."
        assert_eq!(s.len(), 63);
    }

    #[test]
    fn summary_prefers_text_over_html() {
        let text = "Plain text version.";
        let html = "<p>HTML version.</p>";
        let summary = make_collapsed_summary(Some(text), Some(html));
        assert_eq!(summary.as_deref(), Some("Plain text version."));
    }

    #[test]
    fn summary_falls_back_to_html() {
        let html = "<p>HTML <b>content</b> here.</p>";
        let summary = make_collapsed_summary(None, Some(html));
        assert_eq!(summary.as_deref(), Some("HTML content here."));
    }

    #[test]
    fn summary_returns_none_for_empty() {
        assert!(make_collapsed_summary(None, None).is_none());
        assert!(make_collapsed_summary(Some(""), None).is_none());
        assert!(make_collapsed_summary(Some("  "), None).is_none());
    }

    #[test]
    fn summary_handles_only_quotes() {
        let text = "> quoted line 1\n> quoted line 2";
        assert!(make_collapsed_summary(Some(text), None).is_none());
    }

    #[test]
    fn html_entity_decoding() {
        let html = "<p>&amp; &lt;tag&gt; &quot;quoted&quot; &nbsp;</p>";
        let stripped = strip_html_tags(html);
        assert!(stripped.contains("& <tag>"));
        assert!(stripped.contains("\"quoted\""));
    }
}
