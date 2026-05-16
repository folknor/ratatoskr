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
    pub is_deleting: bool,
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
    pub is_replied: bool,
    pub is_forwarded: bool,
    pub is_pinned: bool,
    pub is_muted: bool,
    pub has_attachments: bool,
    pub label_color_bgs: Vec<String>,
    pub from_name: Option<String>,
    pub from_address: Option<String>,
    /// Whether this is a local-only draft (not yet synced to server).
    pub is_local_draft: bool,
    /// Phase 7-8: which field carried the primary match for this
    /// search hit. `None` for non-search list paths (folder/label
    /// view) where attribution is meaningless. `Some(MatchKind::Body)`
    /// is the default for search results where no field outscored
    /// body (or where free_text was empty).
    pub match_kind: Option<rtsk::search::MatchKind>,
    /// Phase 7-8: secondary matches above the 50%-of-top-score
    /// threshold, score-descending. Empty for non-search paths.
    pub also_matched: Vec<rtsk::search::MatchKind>,
}

impl Thread {
    pub fn from_db_thread(t: rtsk::db::types::DbThread) -> Self {
        Self {
            id: t.id,
            account_id: t.account_id,
            subject: t.subject,
            snippet: t.snippet,
            last_message_at: t.last_message_at,
            message_count: t.message_count,
            is_read: t.is_read,
            is_starred: t.is_starred,
            is_replied: false,
            is_forwarded: false,
            is_pinned: t.is_pinned,
            is_muted: t.is_muted,
            has_attachments: t.has_attachments,
            label_color_bgs: Vec::new(),
            from_name: t.from_name,
            from_address: t.from_address,
            is_local_draft: false,
            match_kind: None,
            also_matched: Vec::new(),
        }
    }

    pub fn from_local_draft(d: rtsk::db::queries_extra::LocalDraftSummary) -> Self {
        Self {
            id: d.id,
            account_id: d.account_id,
            subject: d.subject,
            snippet: d.snippet,
            last_message_at: Some(d.updated_at),
            message_count: 1,
            is_read: true,
            is_starred: false,
            is_replied: false,
            is_forwarded: false,
            is_pinned: false,
            is_muted: false,
            has_attachments: false,
            label_color_bgs: Vec::new(),
            from_name: None,
            from_address: d.from_email,
            is_local_draft: true,
            match_kind: None,
            also_matched: Vec::new(),
        }
    }

    pub fn from_public_folder_item(item: rtsk::db::queries_extra::PublicFolderItem) -> Self {
        Self {
            id: item.item_id,
            account_id: item.account_id,
            subject: item.subject,
            snippet: item.body_preview,
            last_message_at: item.received_at,
            message_count: 1,
            is_read: item.is_read,
            is_starred: false,
            is_replied: false,
            is_forwarded: false,
            is_pinned: false,
            is_muted: false,
            has_attachments: false,
            label_color_bgs: Vec::new(),
            from_name: item.sender_name,
            from_address: item.sender_email,
            is_local_draft: false,
            match_kind: None,
            also_matched: Vec::new(),
        }
    }

    pub fn from_search_result(r: rtsk::search_pipeline::UnifiedSearchResult) -> Self {
        Self {
            id: r.thread_id,
            account_id: r.account_id,
            subject: r.subject,
            snippet: r.snippet,
            last_message_at: r.date,
            message_count: r.message_count.unwrap_or(1),
            is_read: r.is_read,
            is_starred: r.is_starred,
            is_replied: false,
            is_forwarded: false,
            is_pinned: false,
            is_muted: false,
            has_attachments: false,
            label_color_bgs: Vec::new(),
            from_name: r.from_name,
            from_address: r.from_address,
            is_local_draft: false,
            match_kind: Some(r.match_kind),
            also_matched: r.also_matched,
        }
    }
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
    pub message_id_header: Option<String>,
    /// Quote/signature-stripped summary for collapsed view.
    /// Falls back to snippet when loaded via legacy path.
    pub snippet: Option<String>,
    /// Full HTML body from the body store (decompressed).
    pub body_html: Option<String>,
    /// Full plain text body from the body store (decompressed).
    pub body_text: Option<String>,
    pub is_read: bool,
    pub is_starred: bool,
    pub is_replied: bool,
    pub is_forwarded: bool,
    /// Whether this message was sent by the account owner.
    pub is_own_message: bool,
}

#[derive(Debug, Clone)]
pub struct ThreadAttachment {
    pub id: String,
    pub message_id: String,
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
