use serde::{Deserialize, Serialize};

// ── Account Scope ───────────────────────────────────────────

/// Specifies which accounts to include in a cross-account query.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum AccountScope {
    /// A single account (equivalent to the existing `account_id` parameter).
    Single(String),
    /// A specific set of accounts.
    Multiple(Vec<String>),
    /// All accounts in the database.
    All,
}

// ── Account ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbAccount {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub token_expires_at: Option<i64>,
    pub history_id: Option<String>,
    pub initial_sync_completed: i64,
    pub last_sync_at: Option<i64>,
    pub is_active: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub provider: String,
    pub imap_host: Option<String>,
    pub imap_port: Option<i64>,
    pub imap_security: Option<String>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<i64>,
    pub smtp_security: Option<String>,
    pub auth_method: String,
    pub imap_password: Option<String>,
    pub oauth_provider: Option<String>,
    pub oauth_client_id: Option<String>,
    pub oauth_client_secret: Option<String>,
    pub imap_username: Option<String>,
    pub smtp_username: Option<String>,
    pub smtp_password: Option<String>,
    pub caldav_url: Option<String>,
    pub caldav_username: Option<String>,
    pub caldav_password: Option<String>,
    pub caldav_principal_url: Option<String>,
    pub caldav_home_url: Option<String>,
    pub calendar_provider: Option<String>,
    pub accept_invalid_certs: i64,
    pub jmap_url: Option<String>,
    pub account_color: Option<String>,
    pub account_name: Option<String>,
    pub sort_order: i64,
}

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
    pub date: i64,
    pub is_read: bool,
    pub is_starred: bool,
    pub body_html: Option<String>,
    pub body_text: Option<String>,
    pub body_cached: Option<bool>,
    pub raw_size: Option<i64>,
    pub internal_date: Option<i64>,
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
    pub parent_label_id: Option<String>,
}

// ── Category ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbCategory {
    pub id: String,
    pub account_id: String,
    pub display_name: String,
    pub color_preset: Option<String>,
    pub color_bg: Option<String>,
    pub color_fg: Option<String>,
    pub provider_id: Option<String>,
    pub sync_state: String,
    pub sort_order: i64,
}

// ── Setting ──────────────────────────────────────────────────

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

// ── Contact Group ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbContactGroup {
    pub id: String,
    pub name: String,
    pub member_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbContactGroupMember {
    pub member_type: String,
    pub member_value: String,
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

// ── Bundle Summary (single category) ───────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BundleSummarySingle {
    pub count: i64,
    pub latest_subject: Option<String>,
    pub latest_sender: Option<String>,
}

// ── Thread category with manual flag ───────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ThreadCategoryWithManual {
    pub category: String,
    pub is_manual: bool,
}

// ── Thread info for categorization ─────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ThreadInfoRow {
    pub id: String,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub from_address: Option<String>,
}

// ── Calendar ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbCalendar {
    pub id: String,
    pub account_id: String,
    pub provider: String,
    pub remote_id: String,
    pub display_name: Option<String>,
    pub color: Option<String>,
    pub is_primary: i64,
    pub is_visible: i64,
    pub sync_token: Option<String>,
    pub ctag: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

// ── Calendar Event ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbCalendarEvent {
    pub id: String,
    pub account_id: String,
    pub google_event_id: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub start_time: i64,
    pub end_time: i64,
    pub is_all_day: i64,
    pub status: String,
    pub organizer_email: Option<String>,
    pub attendees_json: Option<String>,
    pub html_link: Option<String>,
    pub updated_at: i64,
    pub calendar_id: Option<String>,
    pub remote_event_id: Option<String>,
    pub etag: Option<String>,
    pub ical_data: Option<String>,
    pub uid: Option<String>,
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
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UncachedAttachment {
    pub id: String,
    pub message_id: String,
    pub account_id: String,
    pub size: i64,
    pub gmail_attachment_id: Option<String>,
    pub imap_part_id: Option<String>,
}

// ── Writing Style Profile ──────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbWritingStyleProfile {
    pub id: String,
    pub account_id: String,
    pub profile_text: String,
    pub sample_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

// ── Folder Sync State ──────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbFolderSyncState {
    pub account_id: String,
    pub folder_path: String,
    pub uidvalidity: Option<i64>,
    pub last_uid: i64,
    pub modseq: Option<i64>,
    pub last_sync_at: Option<i64>,
}

// ── Notification VIP ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbNotificationVip {
    pub id: String,
    pub account_id: String,
    pub email_address: String,
    pub display_name: Option<String>,
    pub created_at: i64,
}

// ── Image Allowlist ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbAllowlistEntry {
    pub id: String,
    pub account_id: String,
    pub sender_address: String,
    pub created_at: i64,
}

// ── Phishing Allowlist ──────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbPhishingAllowlistEntry {
    pub id: String,
    pub sender_address: String,
    pub created_at: i64,
}

// ── Template ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbTemplate {
    pub id: String,
    pub account_id: Option<String>,
    pub name: String,
    pub subject: Option<String>,
    pub body_html: String,
    pub shortcut: Option<String>,
    pub sort_order: i64,
    pub created_at: i64,
}

// ── Signature ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbSignature {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub body_html: String,
    pub is_default: i64,
    pub sort_order: i64,
}

// ── Send-As Alias ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbSendAsAlias {
    pub id: String,
    pub account_id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub reply_to_address: Option<String>,
    pub signature_id: Option<String>,
    pub is_primary: i64,
    pub is_default: i64,
    pub treat_as_alias: i64,
    pub verification_status: String,
    pub created_at: i64,
}

// ── Local Draft ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbLocalDraft {
    pub id: String,
    pub account_id: String,
    pub to_addresses: Option<String>,
    pub cc_addresses: Option<String>,
    pub bcc_addresses: Option<String>,
    pub subject: Option<String>,
    pub body_html: Option<String>,
    pub reply_to_message_id: Option<String>,
    pub thread_id: Option<String>,
    pub from_email: Option<String>,
    pub signature_id: Option<String>,
    pub remote_draft_id: Option<String>,
    pub attachments: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub sync_status: String,
}

// ── Scheduled Email ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbScheduledEmail {
    pub id: String,
    pub account_id: String,
    pub to_addresses: String,
    pub cc_addresses: Option<String>,
    pub bcc_addresses: Option<String>,
    pub subject: Option<String>,
    pub body_html: String,
    pub reply_to_message_id: Option<String>,
    pub thread_id: Option<String>,
    pub scheduled_at: i64,
    pub signature_id: Option<String>,
    pub attachment_paths: Option<String>,
    pub status: String,
    pub created_at: i64,
    // v43 delegation columns
    pub delegation: String,
    pub remote_message_id: Option<String>,
    pub remote_status: Option<String>,
    pub timezone: Option<String>,
    pub from_email: Option<String>,
    pub error_message: Option<String>,
    pub retry_count: i64,
}

// ── Attachment with context (for library view) ─────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentWithContext {
    pub id: String,
    pub message_id: String,
    pub account_id: String,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub size: Option<i64>,
    pub gmail_attachment_id: Option<String>,
    pub content_id: Option<String>,
    pub is_inline: i64,
    pub local_path: Option<String>,
    pub content_hash: Option<String>,
    pub from_address: Option<String>,
    pub from_name: Option<String>,
    pub date: Option<i64>,
    pub subject: Option<String>,
    pub thread_id: Option<String>,
}

// ── Attachment sender (grouped by sender) ──────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentSender {
    pub from_address: String,
    pub from_name: Option<String>,
    pub count: i64,
}

// ── Label sort order item ──────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelSortOrderItem {
    pub id: String,
    pub sort_order: i64,
}

// ── Task ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbTask {
    pub id: String,
    pub account_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub priority: String,
    pub is_completed: i64,
    pub completed_at: Option<i64>,
    pub due_date: Option<i64>,
    pub parent_id: Option<String>,
    pub thread_id: Option<String>,
    pub thread_account_id: Option<String>,
    pub sort_order: i64,
    pub recurrence_rule: Option<String>,
    pub next_recurrence_at: Option<i64>,
    pub tags_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbTaskTag {
    pub tag: String,
    pub account_id: Option<String>,
    pub color: Option<String>,
    pub sort_order: i64,
    pub created_at: i64,
}

// ── Folder Unread Count ─────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderUnreadCount {
    pub folder_id: String,
    pub unread_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderAccountUnreadCount {
    pub folder_id: String,
    pub account_id: String,
    pub unread_count: i64,
}

// ── Snoozed thread ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnoozedThread {
    pub id: String,
    pub account_id: String,
}

// ── Subscription entry (unsubscribe manager) ───────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionEntry {
    pub from_address: String,
    pub from_name: Option<String>,
    pub latest_unsubscribe_header: String,
    pub latest_unsubscribe_post: Option<String>,
    pub message_count: i64,
    pub latest_date: i64,
    pub status: Option<String>,
}

// ── IMAP message info ──────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImapMessageRow {
    pub id: String,
    pub imap_uid: Option<i64>,
    pub imap_folder: Option<String>,
}

// ── Special folder lookup ──────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecialFolderRow {
    pub imap_folder_path: Option<String>,
    pub name: String,
}

// ── Cached attachment info (for eviction) ──────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedAttachmentRow {
    pub id: String,
    pub local_path: String,
    pub cache_size: i64,
    pub content_hash: Option<String>,
}

// ── Backfill row (smart label backfill) ────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackfillRow {
    pub thread_id: String,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub from_address: Option<String>,
    pub from_name: Option<String>,
    pub to_addresses: Option<String>,
    pub has_attachments: i64,
    pub id: String,
}
