use std::sync::LazyLock;

use base64::Engine as _;
use regex::{Regex, RegexSet};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::db::DbState;
use crate::graph::client::GraphClient;

/// Cloud storage provider that hosts the linked file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudProvider {
    OneDrive,
    GoogleDrive,
    Dropbox,
    Box,
}

impl CloudProvider {
    /// Returns the string used in the `provider` column of `cloud_attachments`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OneDrive => "onedrive",
            Self::GoogleDrive => "google_drive",
            Self::Dropbox => "dropbox",
            Self::Box => "box",
        }
    }
}

/// A cloud storage link detected in an email body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudLink {
    pub url: String,
    pub provider: CloudProvider,
}

// Patterns are ordered to match CloudProvider variants via index ranges.
// Indices 0-2: OneDrive, 3-4: Google Drive, 5-6: Dropbox, 7: Box
static CLOUD_URL_PATTERNS: LazyLock<RegexSet> = LazyLock::new(|| {
    RegexSet::new([
        // OneDrive (indices 0-2)
        r"1drv\.ms",
        r"onedrive\.live\.com",
        r"sharepoint\.com",
        // Google Drive (indices 3-4)
        r"drive\.google\.com",
        r"docs\.google\.com/(?:document|spreadsheet|presentation|forms)",
        // Dropbox (indices 5-6)
        r"dropbox\.com/sh?/",
        r"dl\.dropboxusercontent\.com",
        // Box (index 7)
        r"app\.box\.com/s/",
    ])
    .expect("cloud URL patterns should be valid regexes")
});

/// Regex to extract URLs from href attributes in HTML.
static HREF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"href\s*=\s*["']([^"']+)["']"#)
        .expect("href regex should be valid")
});

fn provider_for_match_index(index: usize) -> Option<CloudProvider> {
    match index {
        0..=2 => Some(CloudProvider::OneDrive),
        3..=4 => Some(CloudProvider::GoogleDrive),
        5..=6 => Some(CloudProvider::Dropbox),
        7 => Some(CloudProvider::Box),
        _ => None,
    }
}

/// Scans an HTML body for cloud storage links and returns all detected links.
///
/// Extracts URLs from `href` attributes and matches each against known cloud
/// storage URL patterns. Each unique URL is returned at most once, tagged with
/// its provider.
pub fn detect_cloud_links(html_body: &str) -> Vec<CloudLink> {
    let mut results = Vec::new();
    let mut seen_urls = std::collections::HashSet::new();

    for cap in HREF_RE.captures_iter(html_body) {
        let Some(url) = cap.get(1).map(|m| m.as_str()) else {
            continue;
        };

        if !seen_urls.insert(url) {
            continue;
        }

        let matches: Vec<usize> = CLOUD_URL_PATTERNS.matches(url).into_iter().collect();
        if let Some(&first_match) = matches.first() {
            if let Some(provider) = provider_for_match_index(first_match) {
                results.push(CloudLink {
                    url: url.to_owned(),
                    provider,
                });
            }
        }
    }

    results
}

// ---------------------------------------------------------------------------
// Upload state machine
// ---------------------------------------------------------------------------

/// Upload lifecycle status for outgoing cloud attachments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadStatus {
    /// Queued, waiting for upload to start.
    Pending,
    /// Upload is in progress.
    Uploading,
    /// File fully uploaded to cloud storage (have drive item ID).
    Uploaded,
    /// Sharing link created and inserted into the email body.
    Linked,
    /// Email containing the link was sent successfully.
    Sent,
    /// Upload failed (may be retried).
    Failed,
}

impl UploadStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Uploading => "uploading",
            Self::Uploaded => "uploaded",
            Self::Linked => "linked",
            Self::Sent => "sent",
            Self::Failed => "failed",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "uploading" => Some(Self::Uploading),
            "uploaded" => Some(Self::Uploaded),
            "linked" => Some(Self::Linked),
            "sent" => Some(Self::Sent),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// A row from the `cloud_attachments` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudAttachment {
    pub id: i64,
    pub message_id: Option<String>,
    pub account_id: String,
    pub direction: String,
    pub provider: String,
    pub cloud_url: Option<String>,
    pub file_name: Option<String>,
    pub file_size: Option<i64>,
    pub mime_type: Option<String>,
    pub drive_item_id: Option<String>,
    pub upload_session_url: Option<String>,
    pub upload_status: String,
    pub bytes_uploaded: i64,
    pub retry_count: i32,
    pub created_at: i64,
}

fn row_to_cloud_attachment(row: &rusqlite::Row<'_>) -> Result<CloudAttachment, rusqlite::Error> {
    Ok(CloudAttachment {
        id: row.get("id")?,
        message_id: row.get("message_id")?,
        account_id: row.get("account_id")?,
        direction: row.get("direction")?,
        provider: row.get("provider")?,
        cloud_url: row.get("cloud_url")?,
        file_name: row.get("file_name")?,
        file_size: row.get("file_size")?,
        mime_type: row.get("mime_type")?,
        drive_item_id: row.get("drive_item_id")?,
        upload_session_url: row.get("upload_session_url")?,
        upload_status: row.get("upload_status")?,
        bytes_uploaded: row.get("bytes_uploaded")?,
        retry_count: row.get("retry_count")?,
        created_at: row.get("created_at")?,
    })
}

/// Get all pending uploads for an account (status = 'pending').
pub fn get_pending_uploads(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<CloudAttachment>, rusqlite::Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT * FROM cloud_attachments
         WHERE account_id = ?1 AND direction = 'outgoing' AND upload_status = 'pending'
         ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![account_id], row_to_cloud_attachment)?;
    rows.collect()
}

/// Transition an upload to a new status, optionally updating bytes_uploaded.
pub fn update_upload_status(
    conn: &Connection,
    id: i64,
    status: UploadStatus,
    bytes_uploaded: Option<i64>,
) -> Result<(), rusqlite::Error> {
    if let Some(bytes) = bytes_uploaded {
        conn.execute(
            "UPDATE cloud_attachments SET upload_status = ?1, bytes_uploaded = ?2 WHERE id = ?3",
            rusqlite::params![status.as_str(), bytes, id],
        )?;
    } else {
        conn.execute(
            "UPDATE cloud_attachments SET upload_status = ?1 WHERE id = ?2",
            rusqlite::params![status.as_str(), id],
        )?;
    }
    Ok(())
}

/// Mark an upload as failed, incrementing retry_count. If `retry` is true the
/// status is reset to `pending` so it will be picked up again; otherwise it
/// stays `failed`.
pub fn mark_upload_failed(
    conn: &Connection,
    id: i64,
    retry: bool,
) -> Result<(), rusqlite::Error> {
    let new_status = if retry {
        UploadStatus::Pending.as_str()
    } else {
        UploadStatus::Failed.as_str()
    };
    conn.execute(
        "UPDATE cloud_attachments
         SET upload_status = ?1, retry_count = retry_count + 1
         WHERE id = ?2",
        rusqlite::params![new_status, id],
    )?;
    Ok(())
}

/// On app restart: reset any rows stuck in `uploading` back to `pending` so
/// they will be retried. Returns the number of rows reset.
pub fn reset_interrupted_uploads(conn: &Connection) -> Result<usize, rusqlite::Error> {
    let count = conn.execute(
        "UPDATE cloud_attachments SET upload_status = 'pending' WHERE upload_status = 'uploading'",
        [],
    )?;
    Ok(count)
}

/// Create a new outgoing cloud attachment entry. Returns the row ID.
pub fn create_outgoing_upload(
    conn: &Connection,
    account_id: &str,
    file_name: &str,
    file_size: i64,
    mime_type: &str,
    provider: &str,
) -> Result<i64, rusqlite::Error> {
    conn.execute(
        "INSERT INTO cloud_attachments
            (account_id, direction, provider, file_name, file_size, mime_type, upload_status)
         VALUES (?1, 'outgoing', ?2, ?3, ?4, ?5, 'pending')",
        rusqlite::params![account_id, provider, file_name, file_size, mime_type],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Get uploads that have permanently failed (retry_count >= max_retries).
pub fn get_permanently_failed(
    conn: &Connection,
    max_retries: i32,
) -> Result<Vec<CloudAttachment>, rusqlite::Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT * FROM cloud_attachments
         WHERE upload_status = 'failed' AND retry_count >= ?1
         ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![max_retries], row_to_cloud_attachment)?;
    rows.collect()
}

/// Inserts detected incoming cloud links into the `cloud_attachments` table.
///
/// Each link is stored with `direction = 'incoming'` and
/// `upload_status = 'complete'` (the file already exists at the remote URL).
pub fn insert_incoming_cloud_links(
    conn: &Connection,
    message_id: &str,
    account_id: &str,
    links: &[CloudLink],
) -> Result<usize, rusqlite::Error> {
    if links.is_empty() {
        return Ok(0);
    }

    let mut stmt = conn.prepare_cached(
        "INSERT OR IGNORE INTO cloud_attachments
            (message_id, account_id, direction, provider, cloud_url, upload_status)
         VALUES (?1, ?2, 'incoming', ?3, ?4, 'complete')",
    )?;

    let mut count: usize = 0;
    for link in links {
        count += stmt.execute(rusqlite::params![
            message_id,
            account_id,
            link.provider.as_str(),
            link.url,
        ])?;
    }

    Ok(count)
}

// ---------------------------------------------------------------------------
// Metadata enrichment
// ---------------------------------------------------------------------------

/// Metadata fetched from a cloud provider about a shared file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CloudMetadata {
    pub file_name: Option<String>,
    pub file_size: Option<i64>,
    pub mime_type: Option<String>,
}

/// OneDrive/SharePoint sharing API response.
#[derive(Debug, Deserialize)]
struct SharesDriveItemResponse {
    name: Option<String>,
    size: Option<i64>,
    file: Option<SharesDriveItemFile>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SharesDriveItemFile {
    mime_type: Option<String>,
}

/// Fetch metadata for a OneDrive/SharePoint sharing link via the Graph API.
///
/// Uses the `GET /shares/{encoded}/driveItem` endpoint. The sharing URL is
/// encoded as `u!` + base64url(url).
pub async fn enrich_onedrive_link(
    client: &GraphClient,
    cloud_url: &str,
    db: &DbState,
) -> Result<CloudMetadata, String> {
    let encoded = encode_sharing_url(cloud_url);
    let path = format!("/shares/{encoded}/driveItem?$select=name,size,file");

    let item: SharesDriveItemResponse = client.get_json(&path, db).await?;

    Ok(CloudMetadata {
        file_name: item.name,
        file_size: item.size,
        mime_type: item.file.and_then(|f| f.mime_type),
    })
}

/// Encode a sharing URL for the Graph `/shares` endpoint.
///
/// Format: `u!` prefix + base64url-encoded URL (no padding).
fn encode_sharing_url(url: &str) -> String {
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(url);
    format!("u!{encoded}")
}

/// Google Drive file metadata response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GDriveFileResponse {
    name: Option<String>,
    size: Option<String>,
    mime_type: Option<String>,
}

/// Fetch metadata for a Google Drive file link.
///
/// Extracts the file ID from the URL, then calls
/// `GET /drive/v3/files/{id}?fields=name,size,mimeType`.
pub async fn enrich_gdrive_link(
    client: &reqwest::Client,
    cloud_url: &str,
    access_token: &str,
) -> Result<CloudMetadata, String> {
    let file_id = extract_gdrive_file_id(cloud_url)
        .ok_or_else(|| format!("Cannot extract Google Drive file ID from URL: {cloud_url}"))?;

    let url = format!(
        "https://www.googleapis.com/drive/v3/files/{file_id}?fields=name,size,mimeType&supportsAllDrives=true"
    );

    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {access_token}"))
        .send()
        .await
        .map_err(|e| format!("Google Drive metadata request failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Google Drive API error: {status} {body}"));
    }

    let file: GDriveFileResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Google Drive response: {e}"))?;

    Ok(CloudMetadata {
        file_name: file.name,
        file_size: file.size.and_then(|s| s.parse::<i64>().ok()),
        mime_type: file.mime_type,
    })
}

/// Regex for extracting file IDs from Google Drive and Google Docs URLs.
///
/// Matches:
/// - `drive.google.com/file/d/{id}/...`
/// - `docs.google.com/document/d/{id}/...`
/// - `docs.google.com/spreadsheets/d/{id}/...`
/// - `docs.google.com/presentation/d/{id}/...`
/// - `drive.google.com/open?id={id}`
static GDRIVE_ID_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:drive\.google\.com/file/d/|docs\.google\.com/(?:document|spreadsheets|presentation)/d/|drive\.google\.com/open\?id=)([a-zA-Z0-9_-]+)",
    )
    .expect("Google Drive ID regex should be valid")
});

/// Extract a Google Drive file ID from various URL formats.
///
/// Returns `None` if the URL does not match any known Google Drive pattern.
pub fn extract_gdrive_file_id(url: &str) -> Option<String> {
    GDRIVE_ID_RE
        .captures(url)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_owned())
}

/// Update the metadata columns of a `cloud_attachments` row.
pub fn update_cloud_attachment_metadata(
    conn: &Connection,
    id: i64,
    metadata: &CloudMetadata,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE cloud_attachments
         SET file_name = COALESCE(?1, file_name),
             file_size = COALESCE(?2, file_size),
             mime_type = COALESCE(?3, mime_type)
         WHERE id = ?4",
        rusqlite::params![metadata.file_name, metadata.file_size, metadata.mime_type, id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_onedrive_links() {
        let html = r#"<a href="https://1drv.ms/w/s!AjJkLmNoPqRs">doc</a>"#;
        let links = detect_cloud_links(html);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].provider, CloudProvider::OneDrive);
    }

    #[test]
    fn detects_sharepoint_link() {
        let html = r#"<a href="https://contoso.sharepoint.com/:w:/g/personal/file">file</a>"#;
        let links = detect_cloud_links(html);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].provider, CloudProvider::OneDrive);
    }

    #[test]
    fn detects_google_drive_link() {
        let html = r#"<a href="https://drive.google.com/file/d/abc123/view">file</a>"#;
        let links = detect_cloud_links(html);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].provider, CloudProvider::GoogleDrive);
    }

    #[test]
    fn detects_google_docs_link() {
        let html = r#"<a href="https://docs.google.com/spreadsheet/d/abc123/edit">sheet</a>"#;
        let links = detect_cloud_links(html);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].provider, CloudProvider::GoogleDrive);
    }

    #[test]
    fn detects_dropbox_link() {
        let html = r#"<a href="https://www.dropbox.com/s/abc123/file.pdf?dl=0">pdf</a>"#;
        let links = detect_cloud_links(html);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].provider, CloudProvider::Dropbox);
    }

    #[test]
    fn detects_dropbox_shared_link() {
        let html = r#"<a href="https://www.dropbox.com/sh/abc123/folder">folder</a>"#;
        let links = detect_cloud_links(html);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].provider, CloudProvider::Dropbox);
    }

    #[test]
    fn detects_box_link() {
        let html = r#"<a href="https://app.box.com/s/abc123def456">file</a>"#;
        let links = detect_cloud_links(html);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].provider, CloudProvider::Box);
    }

    #[test]
    fn deduplicates_urls() {
        let html = r#"
            <a href="https://drive.google.com/file/d/abc/view">link1</a>
            <a href="https://drive.google.com/file/d/abc/view">link2</a>
        "#;
        let links = detect_cloud_links(html);
        assert_eq!(links.len(), 1);
    }

    #[test]
    fn ignores_non_cloud_links() {
        let html = r#"
            <a href="https://example.com/page">normal</a>
            <a href="https://github.com/repo">github</a>
        "#;
        let links = detect_cloud_links(html);
        assert!(links.is_empty());
    }

    #[test]
    fn detects_multiple_providers() {
        let html = r#"
            <a href="https://1drv.ms/w/s!AjJk">onedrive</a>
            <a href="https://drive.google.com/file/d/abc/view">gdrive</a>
            <a href="https://app.box.com/s/xyz">box</a>
        "#;
        let links = detect_cloud_links(html);
        assert_eq!(links.len(), 3);
        assert_eq!(links[0].provider, CloudProvider::OneDrive);
        assert_eq!(links[1].provider, CloudProvider::GoogleDrive);
        assert_eq!(links[2].provider, CloudProvider::Box);
    }

    // ── Metadata enrichment tests ────────────────────────────

    #[test]
    fn encode_sharing_url_format() {
        let encoded = encode_sharing_url("https://1drv.ms/w/s!AjJk");
        assert!(encoded.starts_with("u!"));
        // Should be base64url with no padding
        let b64_part = &encoded[2..];
        assert!(!b64_part.contains('='));
        assert!(!b64_part.contains('+'));
        assert!(!b64_part.contains('/'));
    }

    #[test]
    fn extract_gdrive_file_id_from_file_url() {
        let url = "https://drive.google.com/file/d/1aBcDeFgHiJkLmNoPqRsTuVwXyZ/view?usp=sharing";
        assert_eq!(
            extract_gdrive_file_id(url),
            Some("1aBcDeFgHiJkLmNoPqRsTuVwXyZ".to_owned())
        );
    }

    #[test]
    fn extract_gdrive_file_id_from_docs_url() {
        let url = "https://docs.google.com/document/d/1aBcDeFgHiJk/edit";
        assert_eq!(
            extract_gdrive_file_id(url),
            Some("1aBcDeFgHiJk".to_owned())
        );
    }

    #[test]
    fn extract_gdrive_file_id_from_spreadsheets_url() {
        let url = "https://docs.google.com/spreadsheets/d/1aBcDeFgHiJk/edit#gid=0";
        assert_eq!(
            extract_gdrive_file_id(url),
            Some("1aBcDeFgHiJk".to_owned())
        );
    }

    #[test]
    fn extract_gdrive_file_id_from_open_url() {
        let url = "https://drive.google.com/open?id=1aBcDeFgHiJk";
        assert_eq!(
            extract_gdrive_file_id(url),
            Some("1aBcDeFgHiJk".to_owned())
        );
    }

    #[test]
    fn extract_gdrive_file_id_returns_none_for_non_drive_url() {
        assert_eq!(extract_gdrive_file_id("https://example.com/file"), None);
    }

    #[test]
    fn extract_gdrive_file_id_from_presentation_url() {
        let url = "https://docs.google.com/presentation/d/1aBcDeFgHiJk/edit";
        assert_eq!(
            extract_gdrive_file_id(url),
            Some("1aBcDeFgHiJk".to_owned())
        );
    }
}
