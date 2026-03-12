use serde::{Deserialize, Serialize};

// ── Gmail API response types ────────────────────────────────
//
// These match the Gmail REST API JSON shapes (camelCase).
// Used for deserializing API responses and serializing back to TS via Tauri IPC.

// ── Message ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailMessage {
    pub id: String,
    pub thread_id: String,
    #[serde(default)]
    pub label_ids: Vec<String>,
    #[serde(default)]
    pub snippet: String,
    pub history_id: Option<String>,
    pub internal_date: Option<String>,
    pub payload: Option<GmailPayload>,
    pub size_estimate: Option<i64>,
    pub raw: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailPayload {
    pub part_id: Option<String>,
    pub mime_type: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub headers: Vec<GmailHeader>,
    pub body: Option<GmailBody>,
    #[serde(default)]
    pub parts: Vec<GmailPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GmailHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailBody {
    pub attachment_id: Option<String>,
    #[serde(default)]
    pub size: i64,
    pub data: Option<String>,
}

// ── Thread ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailThread {
    pub id: String,
    pub history_id: Option<String>,
    #[serde(default)]
    pub messages: Vec<GmailMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailThreadStub {
    pub id: String,
    pub snippet: Option<String>,
    pub history_id: Option<String>,
}

// ── Label ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailLabel {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub label_type: Option<String>,
    pub message_list_visibility: Option<String>,
    pub label_list_visibility: Option<String>,
    pub messages_total: Option<i64>,
    pub messages_unread: Option<i64>,
    pub threads_total: Option<i64>,
    pub threads_unread: Option<i64>,
    pub color: Option<GmailLabelColor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailLabelColor {
    pub text_color: String,
    pub background_color: String,
}

// ── History ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailHistoryResponse {
    #[serde(default)]
    pub history: Vec<GmailHistoryItem>,
    pub history_id: String,
    pub next_page_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailHistoryItem {
    pub id: String,
    #[serde(default)]
    pub messages: Vec<GmailMessage>,
    #[serde(default)]
    pub messages_added: Vec<GmailHistoryMessageWrapper>,
    #[serde(default)]
    pub messages_deleted: Vec<GmailHistoryMessageWrapper>,
    #[serde(default)]
    pub labels_added: Vec<GmailHistoryLabelWrapper>,
    #[serde(default)]
    pub labels_removed: Vec<GmailHistoryLabelWrapper>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GmailHistoryMessageWrapper {
    pub message: GmailMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailHistoryLabelWrapper {
    pub message: GmailMessage,
    pub label_ids: Vec<String>,
}

// ── List responses ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListThreadsResponse {
    #[serde(default)]
    pub threads: Vec<GmailThreadStub>,
    pub next_page_token: Option<String>,
    pub result_size_estimate: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListLabelsResponse {
    #[serde(default)]
    pub labels: Vec<GmailLabel>,
}

// ── Attachment ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailAttachmentData {
    pub attachment_id: Option<String>,
    pub size: Option<i64>,
    pub data: String,
}

// ── Draft ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GmailDraft {
    pub id: String,
    pub message: GmailMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailDraftStub {
    pub id: String,
    pub message: GmailDraftMessageRef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailDraftMessageRef {
    pub id: String,
    pub thread_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListDraftsResponse {
    #[serde(default)]
    pub drafts: Vec<GmailDraftStub>,
}

// ── Send-as ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailSendAs {
    pub send_as_email: String,
    pub display_name: Option<String>,
    pub is_default: Option<bool>,
    pub is_primary: Option<bool>,
    pub treat_as_alias: Option<bool>,
    pub verification_status: Option<String>,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSendAsResponse {
    #[serde(default)]
    pub send_as: Vec<GmailSendAs>,
}

// ── Profile ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailProfile {
    pub email_address: String,
    pub messages_total: Option<i64>,
    pub threads_total: Option<i64>,
    pub history_id: String,
}
