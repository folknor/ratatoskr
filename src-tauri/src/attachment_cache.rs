use std::path::PathBuf;

use base64::{Engine, engine::general_purpose::STANDARD};
use tauri::Manager;
use xxhash_rust::xxh3::xxh3_64;

const CACHE_DIR: &str = "attachment_cache";

/// Compute an xxh3 content hash from raw bytes, formatted as 16-char hex.
pub fn hash_bytes(data: &[u8]) -> String {
    format!("{:016x}", xxh3_64(data))
}

/// Resolve the attachment cache directory, creating it if needed.
fn cache_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    let base = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("resolve app data dir: {e}"))?;
    let dir = base.join(CACHE_DIR);
    if !dir.exists() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("create cache dir: {e}"))?;
    }
    Ok(dir)
}

/// Read cached attachment bytes by content hash. Returns `None` on miss.
pub fn read_cached(app_handle: &tauri::AppHandle, content_hash: &str) -> Option<Vec<u8>> {
    let path = cache_dir(app_handle).ok()?.join(content_hash);
    std::fs::read(&path).ok()
}

/// Write attachment bytes to the cache. Skips if file already exists (shared blob).
/// Returns the relative path for DB storage.
pub fn write_cached(
    app_handle: &tauri::AppHandle,
    content_hash: &str,
    data: &[u8],
) -> Result<String, String> {
    let path = cache_dir(app_handle)?.join(content_hash);
    if !path.exists() {
        std::fs::write(&path, data).map_err(|e| format!("write cache file: {e}"))?;
    }
    Ok(format!("{CACHE_DIR}/{content_hash}"))
}

/// Decode base64 (standard or URL-safe) to raw bytes.
pub fn decode_base64(data: &str) -> Result<Vec<u8>, String> {
    let normalized = data.replace('-', "+").replace('_', "/");
    STANDARD
        .decode(&normalized)
        .map_err(|e| format!("base64 decode: {e}"))
}

/// Encode raw bytes to standard base64.
pub fn encode_base64(data: &[u8]) -> String {
    STANDARD.encode(data)
}

// ── DB helpers (run inside with_conn closures) ──────────────

/// Look up an attachment's cache info by message + provider-agnostic remote attachment ID.
///
/// The attachments table still carries legacy per-provider columns, so this
/// helper checks both the Gmail and IMAP ID slots under the hood.
pub fn find_cache_info(
    conn: &rusqlite::Connection,
    account_id: &str,
    message_id: &str,
    remote_attachment_id: &str,
) -> Result<Option<CacheInfo>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, content_hash, mime_type \
             FROM attachments \
             WHERE account_id = ?1 AND message_id = ?2 \
               AND (gmail_attachment_id = ?3 OR imap_part_id = ?3) \
             LIMIT 1",
        )
        .map_err(|e| format!("prepare cache lookup: {e}"))?;

    let mut rows = stmt
        .query_map(
            rusqlite::params![account_id, message_id, remote_attachment_id],
            |row| {
                Ok(CacheInfo {
                    id: row.get(0)?,
                    content_hash: row.get(1)?,
                    mime_type: row.get(2)?,
                })
            },
        )
        .map_err(|e| format!("query cache lookup: {e}"))?;

    match rows.next() {
        Some(Ok(info)) => Ok(Some(info)),
        Some(Err(e)) => Err(format!("read cache row: {e}")),
        None => Ok(None),
    }
}

/// Update an attachment's cache fields after storing to disk.
pub fn update_cache_fields(
    conn: &rusqlite::Connection,
    attachment_id: &str,
    local_path: &str,
    cache_size: i64,
    content_hash: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE attachments \
         SET local_path = ?1, cached_at = unixepoch(), cache_size = ?2, content_hash = ?3 \
         WHERE id = ?4",
        rusqlite::params![local_path, cache_size, content_hash, attachment_id],
    )
    .map_err(|e| format!("update attachment cache: {e}"))?;
    Ok(())
}

/// Cache info for a single attachment row.
pub struct CacheInfo {
    pub id: String,
    pub content_hash: Option<String>,
    pub mime_type: Option<String>,
}
