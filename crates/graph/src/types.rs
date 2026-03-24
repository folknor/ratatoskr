use serde::{Deserialize, Serialize};

/// Generic OData collection wrapper for all list/delta endpoints.
#[derive(Debug, Deserialize)]
pub struct ODataCollection<T> {
    pub value: Vec<T>,
    #[serde(rename = "@odata.nextLink")]
    pub next_link: Option<String>,
    #[serde(rename = "@odata.deltaLink")]
    pub delta_link: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphMessage {
    pub id: String,
    pub conversation_id: Option<String>,
    pub subject: Option<String>,
    pub body_preview: Option<String>,
    pub body: Option<GraphBody>,
    pub from: Option<GraphRecipient>,
    pub to_recipients: Option<Vec<GraphRecipient>>,
    pub cc_recipients: Option<Vec<GraphRecipient>>,
    pub bcc_recipients: Option<Vec<GraphRecipient>>,
    pub reply_to: Option<Vec<GraphRecipient>>,
    pub received_date_time: Option<String>,
    pub sent_date_time: Option<String>,
    pub is_read: Option<bool>,
    pub has_attachments: Option<bool>,
    pub parent_folder_id: Option<String>,
    pub categories: Option<Vec<String>>,
    pub flag: Option<GraphFlag>,
    pub inference_classification: Option<String>,
    pub is_read_receipt_requested: Option<bool>,
    pub internet_message_headers: Option<Vec<GraphInternetHeader>>,
    pub attachments: Option<Vec<GraphAttachment>>,
    pub single_value_extended_properties: Option<Vec<SingleValueExtendedProperty>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphBody {
    pub content_type: String, // "html" or "text"
    pub content: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphRecipient {
    pub email_address: GraphEmailAddress,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEmailAddress {
    pub name: Option<String>,
    pub address: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphFlag {
    pub flag_status: String, // "notFlagged", "flagged", "complete"
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphInternetHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphMailFolder {
    pub id: String,
    pub display_name: String,
    pub child_folder_count: Option<i32>,
    pub parent_folder_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphCreateFolderRequest {
    pub display_name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphRenameFolderRequest {
    pub display_name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphAttachment {
    pub id: String,
    pub name: Option<String>,
    pub content_type: Option<String>,
    pub size: Option<i64>,
    pub is_inline: Option<bool>,
    pub content_id: Option<String>,
    /// base64-encoded, only populated for small attachments.
    pub content_bytes: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphProfile {
    pub display_name: Option<String>,
    pub mail: Option<String>,
    pub user_principal_name: Option<String>,
}

// ── Request body types ──────────────────────────────────────

/// For creating/updating Graph messages (drafts).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphCreateMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<GraphBodyInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_recipients: Option<Vec<GraphRecipient>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cc_recipients: Option<Vec<GraphRecipient>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bcc_recipients: Option<Vec<GraphRecipient>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<Vec<GraphRecipient>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub importance: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub internet_message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub single_value_extended_properties: Option<Vec<SingleValueExtendedProperty>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<GraphRecipient>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender: Option<GraphRecipient>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_read_receipt_requested: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphBodyInput {
    pub content_type: String,
    pub content: String,
}

/// A single-value legacy extended property (MAPI named property).
///
/// Used for Exchange-specific features like `PidTagDeferredSendTime` (tag 0x3FEF).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SingleValueExtendedProperty {
    pub id: String,
    pub value: String,
}

/// For PATCH updates to existing messages.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphMessagePatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_read: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flag: Option<GraphFlagInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub single_value_extended_properties: Option<Vec<SingleValueExtendedProperty>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphFlagInput {
    pub flag_status: String,
}

/// For move operations.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphMoveRequest {
    pub destination_id: String,
}

/// For creating attachments on a draft.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphAttachmentInput {
    #[serde(rename = "@odata.type")]
    pub odata_type: String,
    pub name: String,
    pub content_type: String,
    pub content_bytes: String, // base64
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_inline: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_id: Option<String>,
}

/// The `$select` fields we request for sync messages.
pub(crate) const MESSAGE_SELECT: &str = "\
id,conversationId,subject,bodyPreview,body,uniqueBody,from,\
toRecipients,ccRecipients,bccRecipients,replyTo,\
receivedDateTime,sentDateTime,isRead,isDraft,hasAttachments,\
importance,parentFolderId,categories,flag,\
inferenceClassification,isReadReceiptRequested,internetMessageHeaders,internetMessageId";

/// GUID for Exchange reaction extended properties.
pub(crate) const REACTIONS_GUID: &str = "{41F28F13-83F4-4114-A584-EEDB5A6B0BFF}";

/// `$expand` clause to fetch Exchange reaction extended properties alongside messages.
///
/// - `OwnerReactionType` — the authenticated user's reaction emoji (string)
/// - `ReactionsCount` — total number of reactions on the message (integer)
pub(crate) const REACTIONS_EXPAND: &str = "\
singleValueExtendedProperties(\
$filter=id eq 'String {41F28F13-83F4-4114-A584-EEDB5A6B0BFF} Name OwnerReactionType' \
or id eq 'Integer {41F28F13-83F4-4114-A584-EEDB5A6B0BFF} Name ReactionsCount'\
)";

// ── Large attachment upload session types ─────────────────

/// Request body for `POST /me/messages/{id}/attachments/createUploadSession`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateUploadSessionRequest {
    pub attachment_item: UploadSessionAttachmentItem,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadSessionAttachmentItem {
    #[serde(rename = "@odata.type")]
    pub odata_type: String,
    pub name: String,
    pub size: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_inline: Option<bool>,
}

/// Response from `POST createUploadSession`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadSession {
    pub upload_url: String,
}

// ── Batch request types ──────────────────────────────────

/// A single request within a `POST /$batch` call.
#[derive(Debug, Clone, Serialize)]
pub struct BatchRequestItem {
    pub id: String,
    pub method: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<std::collections::HashMap<String, String>>,
}

/// The top-level `POST /$batch` request body.
#[derive(Debug, Serialize)]
pub struct BatchRequest {
    pub requests: Vec<BatchRequestItem>,
}

/// A single response from a `POST /$batch` call.
#[derive(Debug, Deserialize)]
pub struct BatchResponseItem {
    pub id: String,
    pub status: u16,
    pub body: Option<serde_json::Value>,
}

/// The top-level `POST /$batch` response body.
#[derive(Debug, Deserialize)]
pub struct BatchResponse {
    pub responses: Vec<BatchResponseItem>,
}

// ── Contact types ─────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphContact {
    pub id: String,
    pub display_name: Option<String>,
    pub email_addresses: Option<Vec<GraphContactEmail>>,
    pub parent_folder_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphContactEmail {
    pub name: Option<String>,
    pub address: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphContactFolder {
    pub id: String,
    pub display_name: String,
}

/// The `$select` fields we request for contact sync.
pub const CONTACT_SELECT: &str = "id,displayName,emailAddresses,parentFolderId";
