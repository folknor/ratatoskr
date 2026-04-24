use serde::{Deserialize, Serialize};

// ---------- Namespace / ACL types (RFC 2342 / RFC 4314) ----------

/// Which IMAP namespace a folder belongs to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NamespaceType {
    Personal,
    OtherUsers,
    Shared,
}

/// A single entry from a NAMESPACE response section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceEntry {
    pub prefix: String,
    pub delimiter: Option<String>,
}

/// Parsed result of the IMAP NAMESPACE command (RFC 2342).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NamespaceInfo {
    pub personal: Vec<NamespaceEntry>,
    pub other_users: Vec<NamespaceEntry>,
    pub shared: Vec<NamespaceEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    pub security: String, // "tls", "starttls", "none"
    pub username: String,
    pub password: String,    // plaintext password or OAuth2 access token
    pub auth_method: String, // "password" or "oauth2"
    #[serde(default)]
    pub accept_invalid_certs: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImapFolder {
    pub path: String,     // decoded UTF-8 display name
    pub raw_path: String, // original modified UTF-7 path for IMAP commands
    pub name: String,     // decoded display name (last segment)
    pub delimiter: String,
    pub special_use: Option<String>, // "\Sent", "\Trash", "\Drafts", "\Junk", "\Archive", "\All"
    pub exists: u32,
    pub unseen: u32,
    /// Which IMAP namespace this folder belongs to (populated by `list_shared_folders`).
    pub namespace_type: Option<NamespaceType>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImapMessage {
    pub uid: u32,
    pub folder: String,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    pub from_address: Option<String>,
    pub from_name: Option<String>,
    pub to_addresses: Option<String>,
    pub cc_addresses: Option<String>,
    pub bcc_addresses: Option<String>,
    pub reply_to: Option<String>,
    pub subject: Option<String>,
    pub date: i64,
    pub is_read: bool,
    pub is_starred: bool,
    pub is_draft: bool,
    pub body_html: Option<String>,
    pub body_text: Option<String>,
    pub snippet: Option<String>,
    pub raw_size: u32,
    pub list_unsubscribe: Option<String>,
    pub list_unsubscribe_post: Option<String>,
    pub auth_results: Option<String>,
    pub mdn_requested: bool,
    pub attachments: Vec<ImapAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImapAttachment {
    pub part_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: u32,
    pub content_id: Option<String>,
    pub is_inline: bool,
    pub content_hash: Option<String>,
    /// Raw bytes for small inline images (≤ MAX_INLINE_SIZE).
    /// Only populated at IMAP parse time; `None` for non-inline or large parts.
    #[serde(skip)]
    pub inline_data: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImapFolderStatus {
    pub uidvalidity: u32,
    pub uidnext: u32,
    pub exists: u32,
    pub unseen: u32,
    pub highest_modseq: Option<u64>,
    /// Whether this folder's PERMANENTFLAGS includes `\*`, meaning the server
    /// allows clients to create arbitrary custom keywords (needed for category
    /// writeback via IMAP flags like `$label1`, `category_Work`, etc.).
    pub supports_custom_keywords: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImapFetchResult {
    pub messages: Vec<ImapMessage>,
    pub folder_status: ImapFolderStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImapFolderSyncResult {
    pub uids: Vec<u32>,
    pub messages: Vec<ImapMessage>,
    pub folder_status: ImapFolderStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImapFolderSearchResult {
    pub uids: Vec<u32>,
    pub folder_status: ImapFolderStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaCheckRequest {
    pub folder: String,
    pub last_uid: u32,
    pub uidvalidity: u32,
    /// Cached HIGHESTMODSEQ from the last sync. `None` if CONDSTORE not supported
    /// or first sync.
    pub last_modseq: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaCheckResult {
    pub folder: String,
    pub uidvalidity: u32,
    pub new_uids: Vec<u32>,
    pub uidvalidity_changed: bool,
    /// Server's HIGHESTMODSEQ from the SELECT response. `None` if the server
    /// does not support CONDSTORE or didn't return it for this mailbox.
    pub highest_modseq: Option<u64>,
    /// True when CONDSTORE is available and the server's HIGHESTMODSEQ matches
    /// the cached value - meaning no flag or metadata changes occurred.
    pub modseq_unchanged: bool,
    /// True when the server's HIGHESTMODSEQ is *lower* than our cached value
    /// while UIDVALIDITY is unchanged. This indicates a mod-sequence counter
    /// reset (server migration, mailbox repair, etc.) and requires a full
    /// flag resync - otherwise CHANGEDSINCE with the stale cached value would
    /// return no results, silently missing all updates.
    pub modseq_reset: bool,
    /// Whether PERMANENTFLAGS for this folder includes `\*` (arbitrary keywords).
    pub supports_custom_keywords: bool,
}

/// A flag change for a single message, returned by CHANGEDSINCE fetch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlagChange {
    pub uid: u32,
    pub is_read: bool,
    pub is_starred: bool,
    /// Custom keywords (non-standard flags like `$label1`, `project-alpha`).
    /// Empty when the server doesn't support custom keywords or none are set.
    #[serde(default)]
    pub keywords: Vec<String>,
}
