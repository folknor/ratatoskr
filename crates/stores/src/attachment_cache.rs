use std::path::{Path, PathBuf};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use xxhash_rust::xxh3::xxh3_64;

use db::db::DbState;

use crate::inline_image_store::InlineImageStoreState;

/// Data returned when an attachment is fetched (base64-encoded body + size).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentData {
    pub data: String,
    pub size: usize,
}

const CACHE_DIR: &str = "attachment_cache";

/// Compute an xxh3 content hash from raw bytes, formatted as 16-char hex.
pub fn hash_bytes(data: &[u8]) -> String {
    format!("{:016x}", xxh3_64(data))
}

/// Resolve the attachment cache directory, creating it if needed.
fn cache_dir(app_data_dir: &Path) -> Result<PathBuf, String> {
    let dir = app_data_dir.join(CACHE_DIR);
    if !dir.exists() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("create cache dir: {e}"))?;
    }
    Ok(dir)
}

/// Read cached attachment bytes by content hash. Returns `None` on miss.
pub fn read_cached(app_data_dir: &Path, content_hash: &str) -> Option<Vec<u8>> {
    let path = cache_dir(app_data_dir).ok()?.join(content_hash);
    match std::fs::read(&path) {
        Ok(data) => {
            log::debug!("Attachment cache hit for hash={content_hash}");
            Some(data)
        }
        Err(_) => {
            log::debug!("Attachment cache miss for hash={content_hash}");
            None
        }
    }
}

/// Write attachment bytes to the cache. Skips if file already exists (shared blob).
/// Returns the relative path for DB storage.
pub fn write_cached(
    app_data_dir: &Path,
    content_hash: &str,
    data: &[u8],
) -> Result<String, String> {
    let path = cache_dir(app_data_dir)?.join(content_hash);
    if !path.exists() {
        std::fs::write(&path, data).map_err(|e| format!("write cache file: {e}"))?;
    }
    Ok(format!("{CACHE_DIR}/{content_hash}"))
}

/// Delete a cached attachment file by its DB-relative path.
pub fn remove_cached_relative(app_data_dir: &Path, relative_path: &str) -> Result<(), String> {
    if !relative_path.starts_with(&format!("{CACHE_DIR}/")) {
        return Err(format!("invalid attachment cache path: {relative_path}"));
    }

    let full_path = app_data_dir.join(relative_path);
    let canonical =
        full_path.canonicalize().map_err(|e| format!("canonicalize cache path: {e}"))?;
    let cache_dir = app_data_dir
        .join(CACHE_DIR)
        .canonicalize()
        .map_err(|e| format!("canonicalize cache dir: {e}"))?;
    if !canonical.starts_with(&cache_dir) {
        return Err(format!("path escapes cache directory: {relative_path}"));
    }

    match std::fs::remove_file(&canonical) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("remove cache file: {error}")),
    }
}

/// Decode base64 (standard or URL-safe) to raw bytes.
pub(crate) fn decode_base64(data: &str) -> Result<Vec<u8>, String> {
    let normalized = data.replace('-', "+").replace('_', "/");
    STANDARD
        .decode(&normalized)
        .map_err(|e| format!("base64 decode: {e}"))
}

/// Encode raw bytes to standard base64.
pub(crate) fn encode_base64(data: &[u8]) -> String {
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
                    id: row.get("id")?,
                    content_hash: row.get("content_hash")?,
                    mime_type: row.get("mime_type")?,
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

async fn attachment_cache_max_bytes(db: &DbState) -> Result<i64, String> {
    db.with_conn(|conn| {
        let raw = db::db::queries::get_setting(conn, "attachment_cache_max_mb")
            .unwrap_or(None);
        let max_mb = raw
            .as_deref()
            .unwrap_or("500")
            .parse::<i64>()
            .unwrap_or(500);
        Ok(max_mb.saturating_mul(1024 * 1024))
    })
    .await
}

/// Enforce the configured attachment cache size limit by evicting oldest entries.
pub async fn enforce_cache_limit(db: &DbState, app_data_dir: &Path) -> Result<(), String> {
    let max_bytes = attachment_cache_max_bytes(db).await?;
    if max_bytes <= 0 {
        return Ok(());
    }
    log::debug!("Enforcing attachment cache limit: max {max_bytes} bytes");

    // Collect all eviction candidates and clear their DB rows in one lock acquisition,
    // then delete files outside the lock.
    let files_to_delete: Vec<String> = db
        .with_conn(move |conn| {
            let current_size: i64 = conn
                .query_row(
                    "SELECT COALESCE(SUM(cache_size), 0) AS total \
                     FROM attachments WHERE cached_at IS NOT NULL",
                    [],
                    |row| row.get("total"),
                )
                .map_err(|e| format!("query attachment cache size: {e}"))?;

            if current_size <= max_bytes {
                return Ok(Vec::new());
            }

            // Fetch oldest-first cached attachments so we can walk them until
            // we've freed enough space.
            let mut stmt = conn
                .prepare(
                    "SELECT id, local_path, content_hash, cache_size \
                     FROM attachments \
                     WHERE cached_at IS NOT NULL \
                     ORDER BY cached_at ASC",
                )
                .map_err(|e| format!("prepare eviction query: {e}"))?;

            let rows: Vec<(String, String, Option<String>, i64)> = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>("id")?,
                        row.get::<_, String>("local_path")?,
                        row.get::<_, Option<String>>("content_hash")?,
                        row.get::<_, i64>("cache_size")?,
                    ))
                })
                .map_err(|e| format!("query eviction candidates: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("read eviction row: {e}"))?;

            let mut freed: i64 = 0;
            let excess = current_size - max_bytes;
            let mut ids_to_clear = Vec::new();
            let mut candidates = Vec::new(); // (local_path, content_hash)

            for (id, local_path, content_hash, size) in rows {
                if freed >= excess {
                    break;
                }
                freed = freed.saturating_add(size);
                ids_to_clear.push(id);
                candidates.push((local_path, content_hash));
            }

            // Batch-clear all evicted rows in one statement.
            if !ids_to_clear.is_empty() {
                let placeholders: String = ids_to_clear
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(", ");

                let sql = format!(
                    "UPDATE attachments \
                     SET local_path = NULL, cached_at = NULL, cache_size = NULL \
                     WHERE id IN ({placeholders})"
                );

                let params: Vec<&dyn rusqlite::types::ToSql> = ids_to_clear
                    .iter()
                    .map(|s| s as &dyn rusqlite::types::ToSql)
                    .collect();

                conn.execute(&sql, params.as_slice())
                    .map_err(|e| format!("batch clear attachment cache: {e}"))?;
            }

            // Determine which files to actually delete (skip if another row
            // still references the same content hash).
            let mut files = Vec::new();
            for (local_path, content_hash) in candidates {
                let still_referenced = if let Some(ref hash) = content_hash {
                    let count: i64 = conn
                        .query_row(
                            "SELECT COUNT(*) AS cnt FROM attachments \
                             WHERE content_hash = ?1 AND cached_at IS NOT NULL",
                            rusqlite::params![hash],
                            |row| row.get("cnt"),
                        )
                        .map_err(|e| format!("count remaining cache refs: {e}"))?;
                    count > 0
                } else {
                    false
                };
                if !still_referenced {
                    files.push(local_path);
                }
            }

            Ok(files)
        })
        .await?;

    // Delete files outside the DB lock.
    for path in &files_to_delete {
        remove_cached_relative(app_data_dir, path)?;
    }

    Ok(())
}

// ── Cache orchestration for attachment fetches ──────────────

/// Check the inline image SQLite store for small cached images.
pub async fn try_inline_image_hit(
    db: &DbState,
    inline_images: &InlineImageStoreState,
    account_id: &str,
    message_id: &str,
    attachment_id: &str,
) -> Result<Option<AttachmentData>, String> {
    let (acct, msg, att) = (
        account_id.to_string(),
        message_id.to_string(),
        attachment_id.to_string(),
    );

    let hash = db
        .with_conn(move |conn| {
            let info = find_cache_info(conn, &acct, &msg, &att)?;
            Ok(info.and_then(|i| i.content_hash))
        })
        .await?;

    let Some(hash) = hash else { return Ok(None) };

    let result = inline_images.get(hash).await?;
    Ok(result.map(|(bytes, _mime)| {
        let size = bytes.len();
        let data = encode_base64(&bytes);
        AttachmentData { data, size }
    }))
}

/// Check the content-addressed file cache for a previously fetched
/// attachment.
pub async fn try_cache_hit(
    db: &DbState,
    app_data_dir: &Path,
    account_id: &str,
    message_id: &str,
    attachment_id: &str,
) -> Result<Option<AttachmentData>, String> {
    let dir = app_data_dir.to_path_buf();
    let (acct, msg, att) = (
        account_id.to_string(),
        message_id.to_string(),
        attachment_id.to_string(),
    );

    db.with_conn(move |conn| {
        let info = find_cache_info(conn, &acct, &msg, &att)?;
        let Some(info) = info else { return Ok(None) };
        let Some(ref hash) = info.content_hash else {
            return Ok(None);
        };

        if let Some(bytes) = read_cached(&dir, hash) {
            let size = bytes.len();
            let data = encode_base64(&bytes);
            return Ok(Some(AttachmentData { data, size }));
        }

        Ok(None)
    })
    .await
}

/// After a provider fetch, decode + hash + write to cache + update DB.
///
/// Spawns a background task so the caller is not blocked.
pub fn cache_after_fetch(
    db: &DbState,
    inline_images: &InlineImageStoreState,
    app_data_dir: &Path,
    account_id: &str,
    message_id: &str,
    attachment_id: &str,
    base64_data: &str,
) {
    let db = db.clone();
    let inline_store = inline_images.clone();
    let dir = app_data_dir.to_path_buf();
    let (acct, msg, att, data) = (
        account_id.to_string(),
        message_id.to_string(),
        attachment_id.to_string(),
        base64_data.to_string(),
    );

    tokio::task::spawn(async move {
        let result: Result<(), String> = async {
            let bytes = decode_base64(&data)?;
            let content_hash = hash_bytes(&bytes);

            // Small inline images -> SQLite blob store
            if bytes.len() <= crate::inline_image_store::MAX_INLINE_SIZE {
                let mime = {
                    let (a, m, at) = (acct.clone(), msg.clone(), att.clone());
                    db.with_conn(move |conn| {
                        let info = find_cache_info(conn, &a, &m, &at)?;
                        Ok(info.and_then(|i| i.mime_type))
                    })
                    .await?
                };
                if let Some(ref mime) = mime
                    && mime.starts_with("image/")
                {
                    inline_store
                        .put(content_hash.clone(), bytes.clone(), mime.clone())
                        .await?;
                }
            }

            // File-based cache for all sizes
            let local_path = write_cached(&dir, &content_hash, &bytes)?;

            #[allow(clippy::cast_possible_wrap)]
            let cache_size = bytes.len() as i64;

            db.with_conn(move |conn| {
                let info = find_cache_info(conn, &acct, &msg, &att)?;
                if let Some(info) = info {
                    update_cache_fields(conn, &info.id, &local_path, cache_size, &content_hash)?;
                }
                Ok(())
            })
            .await?;

            enforce_cache_limit(&db, &dir).await
        }
        .await;

        if let Err(e) = result {
            log::warn!("Failed to cache attachment: {e}");
        }
    });
}
