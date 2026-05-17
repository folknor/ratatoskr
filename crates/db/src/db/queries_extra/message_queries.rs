//! Single-message queries for pop-out windows and Save As.
//!
//! These provide per-message body, attachment, and raw source data.
//! Thread-level queries live in `thread_detail.rs`; these are for
//! individual messages in pop-out windows.

use rusqlite::{Connection, params};

use crate::db::ReadConn;

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
/// for pop-out windows. For full body content, use `BodyStoreReadState::get()`
/// from `crates/stores/`.
///
/// Returns `(body_text, body_html)`. body_html is always None here.
pub fn get_message_body(
    conn: &ReadConn<'_>,
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
        Err(crate::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => {
            Ok((None, None))
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Load attachments for a single message (pop-out view).
pub fn get_message_attachments(
    conn: &ReadConn<'_>,
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

/// Phase 8-2: given a slice of candidate `message_id`s, return the
/// subset that is **not** present in the `messages` table. Used by the
/// startup invariant pass to find orphan body / search rows whose
/// underlying message has been deleted.
///
/// Chunked to stay under SQLite's host-parameter cap (default 999).
pub fn find_unreferenced_message_ids(
    conn: &Connection,
    candidates: &[String],
) -> Result<Vec<String>, String> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let mut orphans = Vec::new();
    for chunk in candidates.chunks(500) {
        let placeholders = (1..=chunk.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!("SELECT id FROM messages WHERE id IN ({placeholders})");
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| format!("prepare unreferenced lookup: {e}"))?;
        let params: Vec<&dyn rusqlite::ToSql> = chunk
            .iter()
            .map(|s| s as &dyn rusqlite::ToSql)
            .collect();
        let live: std::collections::HashSet<String> = stmt
            .query_map(params.as_slice(), |r| r.get::<_, String>(0))
            .map_err(|e| format!("query unreferenced lookup: {e}"))?
            .collect::<Result<_, _>>()
            .map_err(|e| format!("collect unreferenced lookup: {e}"))?;
        for id in chunk {
            if !live.contains(id) {
                orphans.push(id.clone());
            }
        }
    }
    Ok(orphans)
}

/// Phase 8-2: enumerate `messages.id` for a given account. Used by the
/// startup invariant pass to build the live-set passed to
/// `SearchReadState::find_orphan_message_ids_for_account`.
pub fn list_message_ids_for_account(
    conn: &Connection,
    account_id: &str,
) -> Result<std::collections::HashSet<String>, String> {
    let mut stmt = conn
        .prepare("SELECT id FROM messages WHERE account_id = ?1")
        .map_err(|e| format!("prepare list ids: {e}"))?;
    let rows = stmt
        .query_map(params![account_id], |r| r.get::<_, String>(0))
        .map_err(|e| format!("query list ids: {e}"))?;
    let mut out = std::collections::HashSet::new();
    for r in rows {
        out.insert(r.map_err(|e| format!("collect id: {e}"))?);
    }
    Ok(out)
}

/// Load raw email source for a message (Source view / Save As).
///
/// NOTE: Raw source is not stored in the messages table in the seed
/// database. This returns a placeholder. When real sync is running,
/// raw source would be stored in the body store or a dedicated column.
pub fn get_message_raw_source(
    conn: &ReadConn<'_>,
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
