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
    pub token_expires_at: Option<i64>,
    pub is_active: bool,
    pub sort_order: i64,
}

/// A shared/delegated mailbox discovered via Autodiscover.
#[derive(Debug, Clone)]
pub struct SharedMailbox {
    /// The SMTP address of the shared mailbox (e.g., "support@contoso.com").
    pub mailbox_id: String,
    /// Display name from Autodiscover or admin config.
    pub display_name: Option<String>,
    /// The parent account ID (the user's personal account).
    pub account_id: String,
    /// Whether sync is enabled for this shared mailbox.
    pub is_sync_enabled: bool,
    /// Last successful sync timestamp.
    pub last_synced_at: Option<i64>,
    /// Last sync error, if any.
    pub sync_error: Option<String>,
}

/// A pinned public folder for sidebar display.
#[derive(Debug, Clone)]
pub struct PinnedPublicFolder {
    /// The EWS FolderId or IMAP folder path.
    pub folder_id: String,
    /// Display name for the sidebar.
    pub display_name: String,
    /// The parent account ID.
    pub account_id: String,
    /// Whether offline sync is enabled for this pin.
    pub sync_enabled: bool,
    /// Sidebar ordering position.
    pub position: i64,
    /// Unread count (from last hierarchy fetch).
    pub unread_count: i64,
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
    pub is_pinned: bool,
    pub is_muted: bool,
    pub has_attachments: bool,
    pub from_name: Option<String>,
    pub from_address: Option<String>,
    /// Whether this is a local-only draft (not yet synced to server).
    pub is_local_draft: bool,
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
    pub cc_addresses: Option<String>,
    pub date: Option<i64>,
    pub subject: Option<String>,
    /// Quote/signature-stripped summary for collapsed view.
    /// Falls back to snippet when loaded via legacy path.
    pub snippet: Option<String>,
    /// Full HTML body from the body store (decompressed).
    pub body_html: Option<String>,
    /// Full plain text body from the body store (decompressed).
    pub body_text: Option<String>,
    pub is_read: bool,
    pub is_starred: bool,
    /// Whether this message was sent by the account owner.
    pub is_own_message: bool,
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

/// Calendar event data (subset of DbCalendarEvent for app-layer use).
#[derive(Debug, Clone)]
pub struct CalendarEvent {
    pub id: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub start_time: i64,
    pub end_time: i64,
    pub is_all_day: bool,
    pub calendar_id: Option<String>,
    pub account_id: String,
    pub timezone: Option<String>,
    pub recurrence_rule: Option<String>,
    pub organizer_name: Option<String>,
    pub organizer_email: Option<String>,
    pub rsvp_status: Option<String>,
    pub availability: Option<String>,
    pub visibility: Option<String>,
}
