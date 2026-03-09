use serde::{Deserialize, Serialize};
use specta::Type;

// ── Thread ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DbThread {
    pub id: String,
    pub account_id: String,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub last_message_at: Option<String>,
    pub message_count: i64,
    pub is_read: bool,
    pub is_starred: bool,
    pub is_important: bool,
    pub has_attachments: bool,
    pub is_snoozed: bool,
    pub snooze_until: Option<String>,
    pub is_pinned: bool,
    pub is_muted: bool,
    // Joined from latest message
    pub from_name: Option<String>,
    pub from_address: Option<String>,
}

// ── Message ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DbMessage {
    pub id: String,
    pub account_id: String,
    pub thread_id: String,
    pub from_address: Option<String>,
    pub from_name: Option<String>,
    pub to_addresses: Option<String>,
    pub cc_addresses: Option<String>,
    pub bcc_addresses: Option<String>,
    pub reply_to: Option<String>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub date: Option<String>,
    pub is_read: bool,
    pub is_starred: bool,
    pub body_html: Option<String>,
    pub body_text: Option<String>,
    pub body_cached: Option<bool>,
    pub raw_size: Option<i64>,
    pub internal_date: Option<String>,
    pub list_unsubscribe: Option<String>,
    pub list_unsubscribe_post: Option<String>,
    pub auth_results: Option<String>,
    pub message_id_header: Option<String>,
    pub references_header: Option<String>,
    pub in_reply_to_header: Option<String>,
    pub imap_uid: Option<i64>,
    pub imap_folder: Option<String>,
}

// ── Label ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DbLabel {
    pub id: String,
    pub account_id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub label_type: Option<String>,
    pub color_bg: Option<String>,
    pub color_fg: Option<String>,
    pub visible: bool,
    pub sort_order: i64,
    pub imap_folder_path: Option<String>,
    pub imap_special_use: Option<String>,
}

// ── Setting ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SettingRow {
    pub key: String,
    pub value: String,
}

// ── Thread category ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CategoryCount {
    pub category: Option<String>,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ThreadCategoryRow {
    pub thread_id: String,
    pub category: String,
}
