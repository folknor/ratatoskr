//! Google Drive resumable file upload and sharing via the Drive API v3.
//!
//! Uploads files to the user's Google Drive using the resumable upload protocol,
//! then creates sharing permissions. Uses `reqwest::Client` directly (not
//! `GmailClient`) since the Drive API has a different base URL.

use serde::{Deserialize, Serialize};

/// Default chunk size: 5 MiB (must be a multiple of 256 KiB per Google's spec).
const GDRIVE_CHUNK_SIZE: usize = 5 * 1024 * 1024;

/// Minimum alignment for upload chunks (256 KiB per Google Drive API spec).
const GDRIVE_CHUNK_ALIGN: usize = 256 * 1024;

// ── Types ────────────────────────────────────────────────

/// An active Google Drive resumable upload session.
#[derive(Debug)]
pub struct GDriveUploadSession {
    pub upload_url: String,
}

/// The completed file metadata returned after a successful upload.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GDriveFileResponse {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub web_view_link: Option<String>,
    #[serde(default)]
    pub size: Option<String>,
}

/// Controls who a file is shared with.
#[derive(Debug)]
pub enum GDriveSharingScope {
    /// Anyone with the link can view.
    Anyone,
    /// Only users in the specified domain can view.
    Domain(String),
}

/// Request body for file metadata when creating an upload session.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileMetadata {
    name: String,
    mime_type: String,
}

/// Response from the permissions endpoint.
#[derive(Debug, Deserialize)]
struct PermissionResponse {
    #[allow(dead_code)]
    id: String,
}

/// Response from getting file metadata with webViewLink.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileWebLinkResponse {
    web_view_link: String,
}

// ── Public API ───────────────────────────────────────────

/// Create a resumable upload session for a file in Google Drive.
///
/// Returns the upload URL from the `Location` header. The upload URL is
/// pre-authenticated - subsequent PUT requests do not need a Bearer token.
pub async fn create_upload_session(
    http: &reqwest::Client,
    access_token: &str,
    file_name: &str,
    mime_type: &str,
    file_size: u64,
) -> Result<GDriveUploadSession, String> {
    let metadata = FileMetadata {
        name: file_name.to_string(),
        mime_type: mime_type.to_string(),
    };

    let response = http
        .post("https://www.googleapis.com/upload/drive/v3/files?uploadType=resumable")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json")
        .header("X-Upload-Content-Type", mime_type)
        .header("X-Upload-Content-Length", file_size.to_string())
        .json(&metadata)
        .send()
        .await
        .map_err(|e| format!("Failed to create upload session: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        log::error!("[GDrive] Failed to create upload session for '{file_name}': {status}");
        return Err(format!("Failed to create upload session: {status} {body}"));
    }

    log::debug!("[GDrive] Created upload session for '{file_name}' ({file_size} bytes)");
    let upload_url = response
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| "Upload session response missing Location header".to_string())?
        .to_string();

    Ok(GDriveUploadSession { upload_url })
}

/// Upload a file in chunks using a resumable upload session.
///
/// `chunk_size` must be a multiple of 256 KiB (`GDRIVE_CHUNK_ALIGN`).
/// The upload URL is pre-authenticated (no Bearer token needed), so this
/// uses a raw `reqwest::Client`.
///
/// Returns the Drive file metadata of the completed upload.
pub async fn upload_file_chunked(
    http: &reqwest::Client,
    upload_url: &str,
    data: &[u8],
    chunk_size: usize,
) -> Result<GDriveFileResponse, String> {
    if chunk_size == 0 || !chunk_size.is_multiple_of(GDRIVE_CHUNK_ALIGN) {
        return Err(format!(
            "chunk_size must be a positive multiple of {GDRIVE_CHUNK_ALIGN}, got {chunk_size}"
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
            // 200 or 201 = final chunk accepted, response contains file metadata
            200 | 201 => {
                let file: GDriveFileResponse = response
                    .json()
                    .await
                    .map_err(|e| format!("Failed to parse completed upload response: {e}"))?;
                return Ok(file);
            }
            // 308 Resume Incomplete = more chunks needed
            308 => {
                // Parse Range header to find how many bytes the server received
                if let Some(range) = response.headers().get("range")
                    && let Ok(range_str) = range.to_str()
                    && let Some(end_str) = range_str.strip_prefix("bytes=0-")
                    && let Ok(last_byte) = end_str.parse::<usize>()
                {
                    offset = last_byte + 1;
                    continue;
                }
                // If we can't parse the Range header, advance by chunk size
                offset = end;
            }
            _ => {
                let body = response.text().await.unwrap_or_default();
                return Err(format!("Upload chunk failed: {status} {body}"));
            }
        }
    }

    Err("Upload completed without receiving a file response".to_string())
}

/// Resume an interrupted upload by querying the server for what has been received.
///
/// Sends an empty PUT with `Content-Range: bytes */{total}` to probe the upload
/// status. Returns the next byte offset to resume from.
pub async fn resume_upload(
    http: &reqwest::Client,
    upload_url: &str,
    total_size: u64,
) -> Result<u64, String> {
    let content_range = format!("bytes */{total_size}");

    let response = http
        .put(upload_url)
        .header("Content-Range", &content_range)
        .header("Content-Length", "0")
        .body(Vec::<u8>::new())
        .send()
        .await
        .map_err(|e| format!("Failed to query upload status: {e}"))?;

    let status = response.status().as_u16();
    match status {
        // 308 = upload incomplete, Range header tells us what was received
        308 => {
            let range = response
                .headers()
                .get("range")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| {
                    "Resume response missing Range header - no bytes received yet".to_string()
                })?;

            // Format: "bytes=0-{last_byte_received}"
            let last_byte_str = range
                .strip_prefix("bytes=0-")
                .ok_or_else(|| format!("Unexpected Range header format: {range}"))?;

            let last_byte: u64 = last_byte_str
                .parse()
                .map_err(|e| format!("Failed to parse Range header value: {e}"))?;

            Ok(last_byte + 1)
        }
        // 200 or 201 = upload already completed
        200 | 201 => Ok(total_size),
        404 => Err("Upload session has expired".to_string()),
        _ => {
            let body = response.text().await.unwrap_or_default();
            Err(format!("Upload status query failed: {status} {body}"))
        }
    }
}

/// Create a sharing permission on an uploaded file.
///
/// After creating the permission, fetches and returns the web view link
/// for the file.
pub async fn create_sharing_permission(
    http: &reqwest::Client,
    access_token: &str,
    file_id: &str,
    scope: GDriveSharingScope,
) -> Result<String, String> {
    let body = match &scope {
        GDriveSharingScope::Anyone => serde_json::json!({
            "role": "reader",
            "type": "anyone",
        }),
        GDriveSharingScope::Domain(domain) => serde_json::json!({
            "role": "reader",
            "type": "domain",
            "domain": domain,
        }),
    };

    let url = format!("https://www.googleapis.com/drive/v3/files/{file_id}/permissions?fields=id");

    let response = http
        .post(&url)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Failed to create sharing permission: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        log::error!("[GDrive] Failed to create sharing permission for file {file_id}: {status}");
        return Err(format!(
            "Failed to create sharing permission: {status} {body}"
        ));
    }
    log::info!("[GDrive] Created sharing permission for file {file_id}");

    let _perm: PermissionResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse permission response: {e}"))?;

    // Fetch the web view link for the shared file
    get_file_web_link(http, access_token, file_id).await
}

/// Get the web view link for a file.
///
/// Returns the shareable URL suitable for insertion into email bodies.
pub async fn get_file_web_link(
    http: &reqwest::Client,
    access_token: &str,
    file_id: &str,
) -> Result<String, String> {
    let url = format!("https://www.googleapis.com/drive/v3/files/{file_id}?fields=webViewLink");

    let response = http
        .get(&url)
        .header("Authorization", format!("Bearer {access_token}"))
        .send()
        .await
        .map_err(|e| format!("Failed to get file web link: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Failed to get file web link: {status} {body}"));
    }

    let file: FileWebLinkResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse file web link response: {e}"))?;

    Ok(file.web_view_link)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_size_validation() {
        // GDRIVE_CHUNK_SIZE must be a multiple of GDRIVE_CHUNK_ALIGN
        assert_eq!(GDRIVE_CHUNK_SIZE % GDRIVE_CHUNK_ALIGN, 0);
        // 5 MiB = 5 * 1024 * 1024
        assert_eq!(GDRIVE_CHUNK_SIZE, 5 * 1024 * 1024);
    }

    #[test]
    fn chunk_range_calculation() {
        let total = 12 * 1024 * 1024; // 12 MiB
        let chunk_size = GDRIVE_CHUNK_SIZE; // 5 MiB

        // First chunk: 0 to 5MiB-1
        let offset = 0;
        let end = (offset + chunk_size).min(total);
        let range = format!("bytes {offset}-{}/{total}", end - 1);
        assert_eq!(range, format!("bytes 0-{}/{total}", 5 * 1024 * 1024 - 1));

        // Second chunk: 5MiB to 10MiB-1
        let offset = end;
        let end = (offset + chunk_size).min(total);
        let range = format!("bytes {offset}-{}/{total}", end - 1);
        assert_eq!(
            range,
            format!("bytes {}-{}/{total}", 5 * 1024 * 1024, 10 * 1024 * 1024 - 1)
        );

        // Third (final) chunk: 10MiB to 12MiB-1
        let offset = end;
        let end = (offset + chunk_size).min(total);
        let range = format!("bytes {offset}-{}/{total}", end - 1);
        assert_eq!(
            range,
            format!(
                "bytes {}-{}/{total}",
                10 * 1024 * 1024,
                12 * 1024 * 1024 - 1
            )
        );
    }

    #[test]
    fn file_response_deserialization() {
        let json = r#"{
            "id": "abc123",
            "name": "report.pdf",
            "webViewLink": "https://drive.google.com/file/d/abc123/view",
            "size": "1048576"
        }"#;

        let file: GDriveFileResponse = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(file.id, "abc123");
        assert_eq!(file.name.as_deref(), Some("report.pdf"));
        assert_eq!(
            file.web_view_link.as_deref(),
            Some("https://drive.google.com/file/d/abc123/view")
        );
        assert_eq!(file.size.as_deref(), Some("1048576"));
    }

    #[test]
    fn file_response_deserialization_minimal() {
        let json = r#"{"id": "xyz789"}"#;

        let file: GDriveFileResponse = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(file.id, "xyz789");
        assert!(file.name.is_none());
        assert!(file.web_view_link.is_none());
        assert!(file.size.is_none());
    }

    #[test]
    fn permission_request_body_anyone() {
        let body = serde_json::json!({
            "role": "reader",
            "type": "anyone",
        });
        assert_eq!(body["role"], "reader");
        assert_eq!(body["type"], "anyone");
    }

    #[test]
    fn permission_request_body_domain() {
        let domain = "example.com";
        let body = serde_json::json!({
            "role": "reader",
            "type": "domain",
            "domain": domain,
        });
        assert_eq!(body["role"], "reader");
        assert_eq!(body["type"], "domain");
        assert_eq!(body["domain"], "example.com");
    }

    #[test]
    fn resume_content_range_header() {
        let total_size: u64 = 10_000_000;
        let content_range = format!("bytes */{total_size}");
        assert_eq!(content_range, "bytes */10000000");
    }
}
