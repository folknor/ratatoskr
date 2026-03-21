//! Single-message queries for pop-out windows and Save As.
//!
//! These provide per-message body, attachment, and raw source data.
//! Thread-level queries live in `thread_detail.rs`; these are for
//! individual messages in pop-out windows.

use rusqlite::{Connection, params};

/// Attachment metadata for a single message (pop-out view).
#[derive(Debug, Clone)]
pub struct MessageAttachment {
    pub id: String,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub size: Option<i64>,
}

/// Load body text and HTML for a single message.
///
/// Returns `(body_text, body_html)`. Returns `(None, None)` if the
/// message is not found rather than erroring.
pub fn get_message_body(
    conn: &Connection,
    account_id: &str,
    message_id: &str,
) -> Result<(Option<String>, Option<String>), String> {
    let result = conn.query_row(
        "SELECT body_text, body_html FROM messages
         WHERE account_id = ?1 AND id = ?2",
        params![account_id, message_id],
        |row| {
            Ok((
                row.get::<_, Option<String>>("body_text")?,
                row.get::<_, Option<String>>("body_html")?,
            ))
        },
    );
    match result {
        Ok(pair) => Ok(pair),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok((None, None)),
        Err(e) => Err(e.to_string()),
    }
}

/// Load attachments for a single message (pop-out view).
pub fn get_message_attachments(
    conn: &Connection,
    account_id: &str,
    message_id: &str,
) -> Result<Vec<MessageAttachment>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, filename, mime_type, size
             FROM attachments
             WHERE account_id = ?1 AND message_id = ?2
             ORDER BY id ASC",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_map(params![account_id, message_id], |row| {
        Ok(MessageAttachment {
            id: row.get("id")?,
            filename: row.get("filename")?,
            mime_type: row.get("mime_type")?,
            size: row.get("size")?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

/// Load raw email source for a message (Source view / Save As).
///
/// Returns a placeholder string if the message has no source stored,
/// and an error if the message is not found at all.
pub fn get_message_raw_source(
    conn: &Connection,
    account_id: &str,
    message_id: &str,
) -> Result<String, String> {
    let result = conn.query_row(
        "SELECT raw_source FROM messages
         WHERE account_id = ?1 AND id = ?2",
        params![account_id, message_id],
        |row| row.get::<_, Option<String>>(0),
    );
    match result {
        Ok(Some(source)) => Ok(source),
        Ok(None) => Ok("(no source available)".to_string()),
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            Err("Message not found".to_string())
        }
        Err(e) => Err(e.to_string()),
    }
}
