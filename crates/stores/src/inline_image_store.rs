use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, params};

/// Max size for inline images stored in the SQLite blob store (256 KB).
/// Anything larger falls through to the file-based cache.
pub const MAX_INLINE_SIZE: usize = 256 * 1024;
/// Cap the inline image store so signature/logo blobs do not grow forever.
const MAX_INLINE_STORE_BYTES: u64 = 128 * 1024 * 1024;

/// Separate SQLite database for small inline images (signatures, logos).
///
/// Content-addressed by xxh3 hash. Identical blobs across messages share
/// one row. No compression — images are already compressed (PNG, JPEG, GIF).
#[derive(Clone)]
pub struct InlineImageStoreState {
    conn: Arc<Mutex<Connection>>,
}

/// A stored inline image blob.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineImage {
    pub content_hash: String,
    pub data: Vec<u8>,
    pub mime_type: String,
}

impl InlineImageStoreState {
    /// Open (or create) the inline image store database.
    pub fn init(app_data_dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(app_data_dir).map_err(|e| format!("create app dir: {e}"))?;

        let db_path = app_data_dir.join("inline_images.db");
        log::info!("Initializing inline image store at {}", db_path.display());
        let conn =
            Connection::open(&db_path).map_err(|e| format!("open inline image store: {e}"))?;

        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA temp_store = MEMORY;
             PRAGMA mmap_size = 268435456;
             PRAGMA cache_size = -16000;
             PRAGMA busy_timeout = 15000;",
        )
        .map_err(|e| format!("inline image store pragmas: {e}"))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS inline_images (
                content_hash TEXT PRIMARY KEY,
                data         BLOB NOT NULL,
                mime_type    TEXT NOT NULL,
                size         INTEGER NOT NULL,
                created_at   INTEGER NOT NULL DEFAULT (unixepoch())
             );",
        )
        .map_err(|e| format!("create inline_images table: {e}"))?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Run a closure on the connection via `spawn_blocking`.
    pub async fn with_conn<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&Connection) -> Result<T, String> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| format!("inline image store lock poisoned: {e}"))?;
            f(&conn)
        })
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?
    }

    /// Store an inline image by content hash. No-op if hash already exists.
    pub async fn put(
        &self,
        content_hash: String,
        data: Vec<u8>,
        mime_type: String,
    ) -> Result<(), String> {
        log::debug!("Storing inline image hash={} mime={} size={}", content_hash, mime_type, data.len());
        self.with_conn(move |conn| {
            #[allow(clippy::cast_possible_wrap)]
            let size = data.len() as i64;
            conn.execute(
                "INSERT OR IGNORE INTO inline_images (content_hash, data, mime_type, size)
                 VALUES (?1, ?2, ?3, ?4)",
                params![content_hash, data, mime_type, size],
            )
            .map_err(|e| format!("inline image put: {e}"))?;
            Ok(())
        })
        .await?;
        self.prune_to_size(MAX_INLINE_STORE_BYTES).await?;
        Ok(())
    }

    /// Store a batch of inline images in a single transaction.
    pub async fn put_batch(&self, images: Vec<InlineImage>) -> Result<(), String> {
        if images.is_empty() {
            return Ok(());
        }

        self.with_conn(move |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("inline image tx: {e}"))?;

            {
                let mut stmt = tx
                    .prepare(
                        "INSERT OR IGNORE INTO inline_images (content_hash, data, mime_type, size)
                         VALUES (?1, ?2, ?3, ?4)",
                    )
                    .map_err(|e| format!("prepare batch put: {e}"))?;

                for img in &images {
                    #[allow(clippy::cast_possible_wrap)]
                    let size = img.data.len() as i64;
                    stmt.execute(params![img.content_hash, img.data, img.mime_type, size])
                        .map_err(|e| format!("batch put row: {e}"))?;
                }
            }

            tx.commit()
                .map_err(|e| format!("inline image commit: {e}"))?;
            Ok(())
        })
        .await?;
        self.prune_to_size(MAX_INLINE_STORE_BYTES).await?;
        Ok(())
    }

    /// Retrieve an inline image by content hash.
    pub async fn get(&self, content_hash: String) -> Result<Option<(Vec<u8>, String)>, String> {
        log::debug!("Retrieving inline image hash={content_hash}");
        self.with_conn(move |conn| {
            let result = conn
                .query_row(
                    "SELECT data, mime_type FROM inline_images WHERE content_hash = ?1",
                    params![content_hash],
                    |row| {
                        let data: Vec<u8> = row.get("data")?;
                        let mime_type: String = row.get("mime_type")?;
                        Ok((data, mime_type))
                    },
                )
                .ok();
            Ok(result)
        })
        .await
    }

    /// Get storage statistics.
    pub async fn stats(&self) -> Result<InlineImageStats, String> {
        self.with_conn(|conn| {
            let count: i64 = conn
                .query_row("SELECT COUNT(*) AS cnt FROM inline_images", [], |row| row.get("cnt"))
                .map_err(|e| format!("count: {e}"))?;
            let total_bytes: i64 = conn
                .query_row(
                    "SELECT COALESCE(SUM(size), 0) AS total FROM inline_images",
                    [],
                    |row| row.get("total"),
                )
                .map_err(|e| format!("total size: {e}"))?;

            #[allow(clippy::cast_sign_loss)]
            Ok(InlineImageStats {
                image_count: count as u64,
                total_bytes: total_bytes as u64,
            })
        })
        .await
    }

    /// Clear all stored inline images.
    pub async fn clear(&self) -> Result<u64, String> {
        log::info!("Clearing all inline images");
        self.with_conn(|conn| {
            let deleted = conn
                .execute("DELETE FROM inline_images", [])
                .map_err(|e| format!("clear inline images: {e}"))?;
            #[allow(clippy::cast_sign_loss)]
            Ok(deleted as u64)
        })
        .await
    }

    /// Delete specific inline image blobs by content hash.
    pub async fn delete_hashes(&self, content_hashes: Vec<String>) -> Result<u64, String> {
        if content_hashes.is_empty() {
            return Ok(0);
        }

        self.with_conn(move |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("inline image delete tx: {e}"))?;
            let mut deleted = 0u64;

            for hash in content_hashes {
                let count = tx
                    .execute(
                        "DELETE FROM inline_images WHERE content_hash = ?1",
                        params![hash],
                    )
                    .map_err(|e| format!("delete inline image: {e}"))?;
                #[allow(clippy::cast_sign_loss)]
                {
                    deleted += count as u64;
                }
            }

            tx.commit()
                .map_err(|e| format!("inline image delete commit: {e}"))?;
            Ok(deleted)
        })
        .await
    }

    /// Evict oldest inline image blobs until the store fits under `max_bytes`.
    pub async fn prune_to_size(&self, max_bytes: u64) -> Result<u64, String> {
        self.with_conn(move |conn| {
            let total_bytes: i64 = conn
                .query_row(
                    "SELECT COALESCE(SUM(size), 0) AS total FROM inline_images",
                    [],
                    |row| row.get("total"),
                )
                .map_err(|e| format!("inline image total size: {e}"))?;

            #[allow(clippy::cast_possible_wrap)]
            let max_bytes_i64 = max_bytes as i64;
            if total_bytes <= max_bytes_i64 {
                return Ok(0);
            }

            log::warn!(
                "Inline image store size ({total_bytes} bytes) exceeds threshold ({max_bytes} bytes), pruning"
            );
            let excess = total_bytes - max_bytes_i64;
            let mut stmt = conn
                .prepare(
                    "SELECT content_hash, size
                     FROM inline_images
                     ORDER BY created_at ASC
                     LIMIT 512",
                )
                .map_err(|e| format!("prepare inline image prune query: {e}"))?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>("content_hash")?, row.get::<_, i64>("size")?))
                })
                .map_err(|e| format!("query inline images for pruning: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("collect inline images for pruning: {e}"))?;

            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("inline image prune tx: {e}"))?;
            let mut freed = 0i64;
            let mut deleted = 0u64;
            for (content_hash, size) in rows {
                if freed >= excess {
                    break;
                }
                let count = tx
                    .execute(
                        "DELETE FROM inline_images WHERE content_hash = ?1",
                        params![content_hash],
                    )
                    .map_err(|e| format!("delete inline image during prune: {e}"))?;
                freed += size.max(0);
                #[allow(clippy::cast_sign_loss)]
                {
                    deleted += count as u64;
                }
            }
            tx.commit()
                .map_err(|e| format!("inline image prune commit: {e}"))?;
            if deleted > 0 {
                log::info!("Pruned {deleted} inline images, freed {freed} bytes");
            }
            Ok(deleted)
        })
        .await
    }

    /// Delete inline image blobs that are no longer referenced by any attachment row.
    ///
    /// First call [`find_unreferenced_hashes`] with the main DB connection
    /// (inside a `with_conn` block) to get the orphaned hashes, then pass
    /// them here.
    pub async fn delete_unreferenced(
        &self,
        orphaned_hashes: Vec<String>,
    ) -> Result<u64, String> {
        self.delete_hashes(orphaned_hashes).await
    }
}

/// Given a set of content hashes, return only those that have zero remaining
/// references in the main database's `attachments` table.
pub fn find_unreferenced_hashes(
    main_db: &Connection,
    content_hashes: &[String],
) -> Result<Vec<String>, String> {
    let mut orphaned = Vec::new();
    for content_hash in content_hashes {
        let remaining_refs: i64 = main_db
            .query_row(
                "SELECT COUNT(*) AS cnt FROM attachments
                 WHERE is_inline = 1 AND content_hash = ?1",
                params![content_hash],
                |row| row.get("cnt"),
            )
            .map_err(|e| format!("count inline image refs: {e}"))?;
        if remaining_refs == 0 {
            orphaned.push(content_hash.clone());
        }
    }
    Ok(orphaned)
}

/// Collect all distinct inline content hashes for an account's messages.
///
/// Call this **before** deleting messages/accounts so cascade-deleted
/// attachment rows can still be queried. After deletion, pass the result
/// to `InlineImageStoreState::delete_unreferenced()`.
pub fn collect_inline_hashes_for_account(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT a.content_hash
             FROM attachments a
             JOIN messages m ON m.account_id = a.account_id AND m.id = a.message_id
             WHERE a.account_id = ?1 AND a.is_inline = 1 AND a.content_hash IS NOT NULL",
        )
        .map_err(|e| format!("prepare inline hash query: {e}"))?;
    let hashes = stmt
        .query_map(params![account_id], |row| row.get::<_, String>("content_hash"))
        .map_err(|e| format!("query inline hashes: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect inline hashes: {e}"))?;
    Ok(hashes)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineImageStats {
    pub image_count: u64,
    pub total_bytes: u64,
}
