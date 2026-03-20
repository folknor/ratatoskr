use rusqlite::Row;

// ── Date display mode ───────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateDisplay {
    /// Absolute date + relative offset from first message ("+14d")
    RelativeOffset,
    /// "Mar 12, 2026 at 2:34 PM"
    Absolute,
}

// ── Types (subset of src-tauri/src/db/types.rs) ─────────────

#[derive(Debug, Clone)]
pub struct Account {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub provider: String,
    pub account_name: Option<String>,
    pub account_color: Option<String>,
    pub last_sync_at: Option<i64>,
    pub sort_order: i64,
}

#[derive(Debug, Clone)]
pub struct Thread {
    pub id: String,
    pub account_id: String,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub last_message_at: Option<i64>,
    pub message_count: i64,
    pub is_read: bool,
    pub is_starred: bool,
    pub has_attachments: bool,
    pub from_name: Option<String>,
    pub from_address: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Label {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct ThreadMessage {
    pub id: String,
    pub thread_id: String,
    pub account_id: String,
    pub from_name: Option<String>,
    pub from_address: Option<String>,
    pub to_addresses: Option<String>,
    pub date: Option<i64>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub is_read: bool,
    pub is_starred: bool,
}

#[derive(Debug, Clone)]
pub struct ThreadAttachment {
    pub id: String,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub size: Option<i64>,
    pub from_name: Option<String>,
    pub date: Option<i64>,
}

/// Attachment data for a single message in a pop-out view.
#[derive(Debug, Clone)]
pub struct MessageViewAttachment {
    pub id: String,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub size: Option<i64>,
}

pub(crate) fn row_to_thread(row: &Row<'_>) -> rusqlite::Result<Thread> {
    Ok(Thread {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        subject: row.get("subject")?,
        snippet: row.get("snippet")?,
        last_message_at: row.get("last_message_at")?,
        message_count: row.get("message_count")?,
        is_read: row.get::<_, i64>("is_read")? != 0,
        is_starred: row.get::<_, i64>("is_starred")? != 0,
        has_attachments: row.get::<_, i64>("has_attachments")? != 0,
        from_name: row.get("from_name")?,
        from_address: row.get("from_address")?,
    })
}
