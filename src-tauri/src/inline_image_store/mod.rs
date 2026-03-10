pub mod commands;

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, params};

/// Max size for inline images stored in the SQLite blob store (256 KB).
/// Anything larger falls through to the file-based cache.
pub const MAX_INLINE_SIZE: usize = 256 * 1024;

/// Separate SQLite database for small inline images (signatures, logos).
///
/// Content-addressed by xxh3 hash. Identical blobs across messages share
/// one row. No compression — images are already compressed (PNG, JPEG, GIF).
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
        let conn = Connection::open(&db_path)
            .map_err(|e| format!("open inline image store: {e}"))?;

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
        .await
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
        .await
    }

    /// Retrieve an inline image by content hash.
    pub async fn get(&self, content_hash: String) -> Result<Option<(Vec<u8>, String)>, String> {
        self.with_conn(move |conn| {
            let result = conn
                .query_row(
                    "SELECT data, mime_type FROM inline_images WHERE content_hash = ?1",
                    params![content_hash],
                    |row| {
                        let data: Vec<u8> = row.get(0)?;
                        let mime_type: String = row.get(1)?;
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
                .query_row("SELECT COUNT(*) FROM inline_images", [], |row| row.get(0))
                .map_err(|e| format!("count: {e}"))?;
            let total_bytes: i64 = conn
                .query_row(
                    "SELECT COALESCE(SUM(size), 0) FROM inline_images",
                    [],
                    |row| row.get(0),
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
        self.with_conn(|conn| {
            let deleted = conn
                .execute("DELETE FROM inline_images", [])
                .map_err(|e| format!("clear inline images: {e}"))?;
            #[allow(clippy::cast_sign_loss)]
            Ok(deleted as u64)
        })
        .await
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineImageStats {
    pub image_count: u64,
    pub total_bytes: u64,
}
