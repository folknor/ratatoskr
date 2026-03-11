use serde::{Deserialize, Serialize};

use crate::smart_labels::AppliedSmartLabelMatch;

/// Progress event emitted during sync.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncProgressEvent {
    pub account_id: String,
    /// "folders" | "messages" | "threading" | "storing_threads" | "done"
    pub phase: String,
    pub current: u64,
    pub total: u64,
    pub folder: Option<String>,
}

/// Completion event with summary of what changed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct SyncCompleteEvent {
    pub account_id: String,
    pub new_message_count: u64,
    /// IDs of new inbox messages — TS uses for notification + filter hooks.
    pub new_inbox_message_ids: Vec<String>,
    /// Thread IDs of affected threads — TS uses for UI refresh.
    pub affected_thread_ids: Vec<String>,
}

/// Error event emitted when sync fails.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct SyncErrorEvent {
    pub account_id: String,
    pub error: String,
}

/// Generic sync lifecycle event emitted for queued/manual sync runs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncStatus {
    Syncing,
    Done,
    Error,
}

/// Generic sync lifecycle event emitted for queued/manual sync runs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatusDonePayload {
    pub should_sync_calendar: bool,
    pub new_inbox_message_ids: Vec<String>,
    pub affected_thread_ids: Vec<String>,
    pub criteria_smart_label_matches: Vec<AppliedSmartLabelMatch>,
    pub notifications_to_queue: Vec<NotificationCandidate>,
}

/// Generic sync lifecycle event emitted for queued/manual sync runs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatusEvent {
    pub account_id: String,
    pub provider: String,
    pub status: SyncStatus,
    pub error: Option<String>,
    pub result: Option<SyncStatusDonePayload>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationCandidate {
    pub thread_id: String,
    pub from_name: Option<String>,
    pub from_address: Option<String>,
    pub subject: Option<String>,
}

/// Lightweight metadata kept in memory during sync for the threading pass.
/// Bodies and full ParsedMessage data are already written to DB at this point.
#[derive(Debug, Clone)]
pub struct MessageMeta {
    pub id: String,
    pub rfc_message_id: String,
    pub label_ids: Vec<String>,
    pub is_read: bool,
    pub is_starred: bool,
    pub has_attachments: bool,
    pub subject: Option<String>,
    pub snippet: String,
    pub date: i64,
}

/// Result of the IMAP sync command returned to TS.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImapSyncResult {
    /// Number of messages stored.
    pub stored_count: u64,
    /// Number of thread groups created.
    pub thread_count: u64,
    /// IDs of new inbox messages (for filters/notifications on TS side).
    pub new_inbox_message_ids: Vec<String>,
    /// Thread IDs of all affected threads.
    pub affected_thread_ids: Vec<String>,
}
