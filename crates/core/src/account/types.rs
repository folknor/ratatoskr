use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountResult {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub is_active: bool,
    pub provider: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthProviderAuthorizationResult {
    pub authorization_id: String,
    pub access_token: String,
    pub expires_in: u64,
    pub email: String,
    pub name: String,
    pub picture: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateImapOAuthAccountRequest {
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub imap_host: String,
    pub imap_port: i64,
    pub imap_security: String,
    pub smtp_host: String,
    pub smtp_port: i64,
    pub smtp_security: String,
    pub authorization_id: String,
    pub oauth_provider: String,
    pub oauth_client_id: String,
    pub oauth_client_secret: Option<String>,
    pub oauth_token_url: Option<String>,
    pub imap_username: Option<String>,
    pub accept_invalid_certs: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarProviderInfo {
    pub provider: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaldavConnectionInfo {
    pub server_url: String,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountBasicInfo {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub provider: String,
    pub is_active: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountCaldavSettingsInfo {
    pub id: String,
    pub email: String,
    pub caldav_url: Option<String>,
    pub caldav_username: Option<String>,
    pub caldav_password: Option<String>,
    pub calendar_provider: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountOAuthCredentials {
    pub client_id: String,
    pub client_secret: Option<String>,
}

/// Data collected from the DB before account deletion for cleanup purposes.
pub struct AccountDeletionData {
    pub message_ids: Vec<String>,
    pub cached_files: Vec<(String, String)>,
    pub inline_hashes: Vec<String>,
}

/// Result of the synchronous phase of account deletion: gathered data plus
/// the sets of hashes that are still referenced by other accounts.
pub struct AccountDeletionPlan {
    pub data: AccountDeletionData,
    /// Cached-file content hashes referenced by at least one other account.
    pub shared_cache_hashes: std::collections::HashSet<String>,
    /// Inline-image content hashes referenced by at least one other account.
    pub shared_inline_hashes: std::collections::HashSet<String>,
}

/// Report of best-effort cleanup after account deletion.
#[derive(Debug, Default)]
pub struct AccountDeletionCleanupReport {
    pub bodies_deleted: u64,
    pub inline_images_deleted: u64,
    pub cache_files_deleted: u64,
    pub cache_file_errors: Vec<(String, String)>,
    pub search_cleaned: bool,
}
