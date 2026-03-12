use std::path::{Path, PathBuf};

use base64::{Engine, engine::general_purpose::STANDARD};
use rusqlite::OptionalExtension;
use xxhash_rust::xxh3::xxh3_64;

use crate::db::DbState;

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
    std::fs::read(&path).ok()
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

    let path = app_data_dir.join(relative_path);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("remove cache file: {error}")),
    }
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

#[derive(Debug)]
struct CachedAttachmentRow {
    id: String,
    local_path: String,
    content_hash: Option<String>,
}

async fn attachment_cache_max_bytes(db: &DbState) -> Result<i64, String> {
    db.with_conn(|conn| {
        let raw: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'attachment_cache_max_mb'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("query attachment cache limit: {e}"))?;
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

    loop {
        let current_size: i64 = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT COALESCE(SUM(cache_size), 0) FROM attachments WHERE cached_at IS NOT NULL",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| format!("query attachment cache size: {e}"))
            })
            .await?;
        if current_size <= max_bytes {
            return Ok(());
        }

        let oldest: Option<CachedAttachmentRow> = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT id, local_path, content_hash
                     FROM attachments
                     WHERE cached_at IS NOT NULL
                     ORDER BY cached_at ASC
                     LIMIT 1",
                    [],
                    |row| {
                        Ok(CachedAttachmentRow {
                            id: row.get(0)?,
                            local_path: row.get(1)?,
                            content_hash: row.get(2)?,
                        })
                    },
                )
                .optional()
                .map_err(|e| format!("query oldest cached attachment: {e}"))
            })
            .await?;

        let Some(row) = oldest else {
            return Ok(());
        };

        let remaining_refs: i64 = db
            .with_conn({
                let attachment_id = row.id.clone();
                let content_hash = row.content_hash.clone();
                move |conn| {
                    conn.execute(
                        "UPDATE attachments
                         SET local_path = NULL, cached_at = NULL, cache_size = NULL
                         WHERE id = ?1",
                        rusqlite::params![attachment_id],
                    )
                    .map_err(|e| format!("clear attachment cache entry: {e}"))?;

                    if let Some(hash) = content_hash {
                        return conn
                            .query_row(
                                "SELECT COUNT(*) FROM attachments
                                 WHERE content_hash = ?1 AND cached_at IS NOT NULL",
                                rusqlite::params![hash],
                                |db_row| db_row.get(0),
                            )
                            .map_err(|e| format!("count remaining cache refs: {e}"));
                    }

                    Ok(0)
                }
            })
            .await?;

        if remaining_refs == 0 {
            remove_cached_relative(app_data_dir, &row.local_path)?;
        }
    }
}
