use serde::{Deserialize, Serialize};

// ── Thread ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingRow {
    pub key: String,
    pub value: String,
}

// ── Thread category ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryCount {
    pub category: Option<String>,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadCategoryRow {
    pub thread_id: String,
    pub category: String,
}

// ── Contact ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbContact {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub frequency: i64,
    pub last_contacted_at: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactStats {
    pub email_count: i64,
    pub first_email: Option<String>,
    pub last_email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SameDomainContact {
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactAttachmentRow {
    pub filename: String,
    pub mime_type: Option<String>,
    pub size: Option<i64>,
    pub date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentThread {
    pub thread_id: String,
    pub subject: Option<String>,
    pub last_message_at: Option<String>,
}

// ── Filter Rule ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbFilterRule {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub is_enabled: bool,
    pub criteria_json: String,
    pub actions_json: String,
    pub sort_order: i64,
    pub created_at: i64,
}

// ── Smart Folder ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbSmartFolder {
    pub id: String,
    pub account_id: Option<String>,
    pub name: String,
    pub query: String,
    pub icon: String,
    pub color: Option<String>,
    pub sort_order: i64,
    pub is_default: bool,
    pub created_at: i64,
}

// ── Smart Label Rule ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbSmartLabelRule {
    pub id: String,
    pub account_id: String,
    pub label_id: String,
    pub ai_description: String,
    pub criteria_json: Option<String>,
    pub is_enabled: bool,
    pub sort_order: i64,
    pub created_at: i64,
}

// ── Follow-Up Reminder ──────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbFollowUpReminder {
    pub id: String,
    pub account_id: String,
    pub thread_id: String,
    pub message_id: String,
    pub remind_at: i64,
    pub status: String,
    pub created_at: i64,
}

// ── Quick Step ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbQuickStep {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub description: Option<String>,
    pub shortcut: Option<String>,
    pub actions_json: String,
    pub icon: Option<String>,
    pub is_enabled: bool,
    pub continue_on_error: bool,
    pub sort_order: i64,
    pub created_at: i64,
}

// ── Triggered Follow-Up (returned by batch check) ──────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggeredFollowUp {
    pub id: String,
    pub account_id: String,
    pub thread_id: String,
    pub subject: String,
}

// ── Sort order helper ───────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SortOrderItem {
    pub id: String,
    pub sort_order: i64,
}

// ── Bundle Rule ─────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DbBundleRule {
    pub id: String,
    pub account_id: String,
    pub category: String,
    pub is_bundled: i64,
    pub delivery_enabled: i64,
    pub delivery_schedule: Option<String>,
    pub last_delivered_at: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BundleSummary {
    pub category: String,
    pub count: i64,
    pub latest_subject: Option<String>,
    pub latest_sender: Option<String>,
}

// ── Attachment ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbAttachment {
    pub id: String,
    pub message_id: String,
    pub account_id: String,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub size: Option<i64>,
    pub gmail_attachment_id: Option<String>,
    pub content_id: Option<String>,
    pub is_inline: bool,
    pub local_path: Option<String>,
}
