//! OneDrive resumable file upload via the Microsoft Graph API.
//!
//! Uploads files to a "Ratatoskr Attachments" folder in the user's OneDrive,
//! then creates a sharing link. Uses the resumable upload session protocol
//! with 320 KiB-aligned chunks.

use serde::{Deserialize, Serialize};

use ratatoskr_db::db::DbState;

use super::client::GraphClient;

/// Default chunk size: 5 MiB (must be a multiple of 320 KiB).
pub const DEFAULT_CHUNK_SIZE: usize = 5 * 1024 * 1024;

/// Minimum alignment for upload chunks (320 KiB per Graph API spec).
const CHUNK_ALIGNMENT: usize = 320 * 1024;

// ── Types ────────────────────────────────────────────────

/// An active OneDrive resumable upload session.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadSession {
    pub upload_url: String,
    #[serde(default)]
    pub expiration_date_time: String,
}

/// Request body for creating an upload session via `createUploadSession`.
#[derive(Debug, Serialize)]
struct CreateUploadSessionRequest {
    item: DriveItemUploadable,
    #[serde(rename = "@microsoft.graph.conflictBehavior")]
    conflict_behavior: String,
}

/// Properties for the file being uploaded.
#[derive(Debug, Serialize)]
struct DriveItemUploadable {
    name: String,
}

/// The completed drive item returned after the final upload chunk.
#[derive(Debug, Deserialize)]
struct DriveItemResponse {
    id: String,
}

/// Response from creating a sharing link.
#[derive(Debug, Deserialize)]
struct CreateLinkResponse {
    link: SharingLink,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SharingLink {
    web_url: String,
}

/// Response from querying upload status (for resume).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadStatusResponse {
    #[serde(default)]
    next_expected_ranges: Vec<String>,
}

// ── Public API ───────────────────────────────────────────

/// Create an upload session for a file in the "Ratatoskr Attachments" folder.
///
/// The folder is created implicitly by OneDrive when the first file is uploaded.
/// Uses `rename` conflict behavior so uploads never overwrite existing files.
pub async fn create_upload_session(
    client: &GraphClient,
    filename: &str,
    db: &DbState,
) -> Result<UploadSession, String> {
    log::debug!("[OneDrive] Creating upload session for '{filename}'");
    let encoded_path = encode_onedrive_path(filename);
    let path = format!(
        "/me/drive/root:/Ratatoskr Attachments/{encoded_path}:/createUploadSession"
    );

    let body = CreateUploadSessionRequest {
        item: DriveItemUploadable {
            name: filename.to_string(),
        },
        conflict_behavior: "rename".to_string(),
    };

    client.post(&path, &body, db).await
}

/// Upload a file in chunks using a resumable upload session.
///
/// `chunk_size` must be a multiple of 320 KiB (`CHUNK_ALIGNMENT`).
/// Returns the OneDrive drive item ID of the completed upload.
///
/// The upload URL is pre-authenticated (no Bearer token needed), so this
/// uses a raw `reqwest::Client` rather than `GraphClient`.
pub async fn upload_file_chunked(
    http: &reqwest::Client,
    upload_url: &str,
    data: &[u8],
    chunk_size: usize,
) -> Result<String, String> {
    if chunk_size == 0 || !chunk_size.is_multiple_of(CHUNK_ALIGNMENT) {
        return Err(format!(
            "chunk_size must be a positive multiple of {CHUNK_ALIGNMENT}, got {chunk_size}"
        ));
    }

    let total = data.len();
    if total == 0 {
        return Err("Cannot upload empty file".to_string());
    }

    let mut offset = 0;
    while offset < total {
        let end = (offset + chunk_size).min(total);
        let chunk = &data[offset..end];
        let content_range = format!("bytes {offset}-{}/{total}", end - 1);

        let response = http
            .put(upload_url)
            .header("Content-Range", &content_range)
            .header("Content-Length", chunk.len().to_string())
            .body(chunk.to_vec())
            .send()
            .await
            .map_err(|e| format!("Upload chunk failed: {e}"))?;

        let status = response.status().as_u16();
        match status {
            // 200 or 201 = final chunk accepted, response contains the drive item
            200 | 201 => {
                let item: DriveItemResponse = response
                    .json()
                    .await
                    .map_err(|e| format!("Failed to parse completed upload response: {e}"))?;
                return Ok(item.id);
            }
            // 202 = more chunks expected
            202 => {}
            _ => {
                let body = response.text().await.unwrap_or_default();
                return Err(format!("Upload chunk failed: {status} {body}"));
            }
        }

        offset = end;
    }

    Err("Upload completed without receiving a drive item response".to_string())
}

/// Resume an interrupted upload by querying what byte ranges the server still expects.
///
/// Returns a list of `(start, end)` byte ranges that need to be uploaded.
/// An empty `end` in the Graph API response (e.g. `"128-"`) is represented
/// as `u64::MAX`, meaning "from start to the end of the file."
pub async fn resume_upload(
    http: &reqwest::Client,
    upload_url: &str,
) -> Result<Vec<(u64, u64)>, String> {
    let response = http
        .get(upload_url)
        .send()
        .await
        .map_err(|e| format!("Failed to query upload status: {e}"))?;

    let status = response.status().as_u16();
    if status == 404 {
        return Err("Upload session has expired".to_string());
    }
    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Upload status query failed: {status} {body}"));
    }

    let status_resp: UploadStatusResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse upload status: {e}"))?;

    let mut ranges = Vec::new();
    for range_str in &status_resp.next_expected_ranges {
        let parts: Vec<&str> = range_str.split('-').collect();
        let start: u64 = parts
            .first()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("Invalid range: {range_str}"))?;
        let end: u64 = parts
            .get(1)
            .and_then(|s| if s.is_empty() { None } else { s.parse().ok() })
            .unwrap_or(u64::MAX);
        ranges.push((start, end));
    }

    Ok(ranges)
}

/// Create a sharing link for an uploaded drive item.
///
/// `scope` should be `"organization"` (tenant-only) or `"anonymous"` (anyone with the link).
/// Returns the web URL of the sharing link.
pub async fn create_sharing_link(
    client: &GraphClient,
    item_id: &str,
    scope: &str,
    db: &DbState,
) -> Result<String, String> {
    let path = format!("/me/drive/items/{item_id}/createLink");

    let body = serde_json::json!({
        "type": "view",
        "scope": scope,
    });

    let response: CreateLinkResponse = client.post(&path, &body, db).await?;
    Ok(response.link.web_url)
}

// ── Helpers ──────────────────────────────────────────────

/// Percent-encode characters that are invalid in OneDrive path segments.
///
/// OneDrive path-based addressing requires encoding `#`, `%`, and `?`.
/// Spaces are allowed in OneDrive paths and kept as-is (the Graph API
/// handles them in the URL template).
fn encode_onedrive_path(filename: &str) -> String {
    filename
        .replace('%', "%25")
        .replace('#', "%23")
        .replace('?', "%3F")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_size_validation() {
        // DEFAULT_CHUNK_SIZE must be a multiple of CHUNK_ALIGNMENT
        assert_eq!(DEFAULT_CHUNK_SIZE % CHUNK_ALIGNMENT, 0);
        // 5 MiB = 5 * 1024 * 1024
        assert_eq!(DEFAULT_CHUNK_SIZE, 5 * 1024 * 1024);
    }

    #[test]
    fn encode_special_chars() {
        assert_eq!(encode_onedrive_path("file.txt"), "file.txt");
        assert_eq!(encode_onedrive_path("file #1.txt"), "file %231.txt");
        assert_eq!(
            encode_onedrive_path("100% done?.txt"),
            "100%25 done%3F.txt"
        );
    }
}
