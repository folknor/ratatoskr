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

/// Load body text for a single message.
///
/// NOTE: Message bodies live in the body store (bodies.db), NOT in the
/// messages table. This function returns the `snippet` as a fallback
/// for pop-out windows. For full body content, use `BodyStoreState::get()`
/// from `crates/stores/`.
///
/// Returns `(body_text, body_html)`. body_html is always None here.
pub fn get_message_body(
    conn: &Connection,
    account_id: &str,
    message_id: &str,
) -> Result<(Option<String>, Option<String>), String> {
    let result = conn.query_row(
        "SELECT snippet FROM messages
         WHERE account_id = ?1 AND id = ?2",
        params![account_id, message_id],
        |row| row.get::<_, Option<String>>(0),
    );
    match result {
        Ok(snippet) => Ok((snippet, None)),
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
/// NOTE: Raw source is not stored in the messages table in the seed
/// database. This returns a placeholder. When real sync is running,
/// raw source would be stored in the body store or a dedicated column.
pub fn get_message_raw_source(
    conn: &Connection,
    account_id: &str,
    message_id: &str,
) -> Result<String, String> {
    // Check the message exists
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM messages WHERE account_id = ?1 AND id = ?2",
            params![account_id, message_id],
            |_| Ok(true),
        )
        .unwrap_or(false);

    if exists {
        Ok("(raw source not available - message bodies are stored in the body store)".to_string())
    } else {
        Err("Message not found".to_string())
    }
}
