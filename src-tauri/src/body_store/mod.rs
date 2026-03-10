pub mod commands;

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, params};

/// Separate SQLite database for compressed email bodies.
///
/// Bodies are stored as zstd-compressed BLOBs, keeping the metadata DB small.
/// Same `Mutex<Connection>` + `spawn_blocking` pattern as `DbState`.
pub struct BodyStoreState {
    conn: Arc<Mutex<Connection>>,
}

/// A single body record returned to callers.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageBody {
    pub message_id: String,
    pub body_html: Option<String>,
    pub body_text: Option<String>,
}

/// Zstd compression level — 3 gives ~3-4x ratio with fast compression.
const ZSTD_LEVEL: i32 = 3;

fn compress(data: &str) -> Result<Vec<u8>, String> {
    zstd::encode_all(data.as_bytes(), ZSTD_LEVEL).map_err(|e| format!("zstd compress: {e}"))
}

fn decompress(data: &[u8]) -> Result<String, String> {
    let bytes = zstd::decode_all(data).map_err(|e| format!("zstd decompress: {e}"))?;
    String::from_utf8(bytes).map_err(|e| format!("utf8 decode: {e}"))
}

impl BodyStoreState {
    /// Open (or create) the body store database.
    pub fn init(app_data_dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(app_data_dir).map_err(|e| format!("create app dir: {e}"))?;

        let db_path = app_data_dir.join("bodies.db");
        let conn = Connection::open(&db_path).map_err(|e| format!("open body store: {e}"))?;

        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA temp_store = MEMORY;
             PRAGMA mmap_size = 2147483648;
             PRAGMA cache_size = -32000;
             PRAGMA busy_timeout = 15000;",
        )
        .map_err(|e| format!("body store pragmas: {e}"))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS bodies (
                message_id TEXT PRIMARY KEY,
                body_html  BLOB,
                body_text  BLOB
             );",
        )
        .map_err(|e| format!("create bodies table: {e}"))?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Run a closure on the body store connection via `spawn_blocking`.
    pub async fn with_conn<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&Connection) -> Result<T, String> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| format!("body store lock poisoned: {e}"))?;
            f(&conn)
        })
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?
    }

    /// Store a message body (compressed).
    pub async fn put(
        &self,
        message_id: String,
        body_html: Option<String>,
        body_text: Option<String>,
    ) -> Result<(), String> {
        self.with_conn(move |conn| {
            let html_blob = body_html.as_deref().map(compress).transpose()?;
            let text_blob = body_text.as_deref().map(compress).transpose()?;

            conn.execute(
                "INSERT OR REPLACE INTO bodies (message_id, body_html, body_text)
                 VALUES (?1, ?2, ?3)",
                params![message_id, html_blob, text_blob],
            )
            .map_err(|e| format!("body store put: {e}"))?;
            Ok(())
        })
        .await
    }

    /// Store multiple message bodies in a single transaction.
    pub async fn put_batch(&self, bodies: Vec<MessageBody>) -> Result<(), String> {
        self.with_conn(move |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("body store tx: {e}"))?;

            {
                let mut stmt = tx
                    .prepare(
                        "INSERT OR REPLACE INTO bodies (message_id, body_html, body_text)
                         VALUES (?1, ?2, ?3)",
                    )
                    .map_err(|e| format!("prepare batch put: {e}"))?;

                for body in &bodies {
                    let html_blob = body.body_html.as_deref().map(compress).transpose()?;
                    let text_blob = body.body_text.as_deref().map(compress).transpose()?;

                    stmt.execute(params![body.message_id, html_blob, text_blob])
                        .map_err(|e| format!("batch put row: {e}"))?;
                }
            }

            tx.commit().map_err(|e| format!("body store commit: {e}"))?;
            Ok(())
        })
        .await
    }

    /// Retrieve a single message body (decompressed).
    pub async fn get(&self, message_id: String) -> Result<Option<MessageBody>, String> {
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare("SELECT body_html, body_text FROM bodies WHERE message_id = ?1")
                .map_err(|e| format!("prepare get: {e}"))?;

            let result = stmt
                .query_row(params![message_id], |row| {
                    let html_blob: Option<Vec<u8>> = row.get(0)?;
                    let text_blob: Option<Vec<u8>> = row.get(1)?;
                    Ok((html_blob, text_blob))
                })
                .ok();

            match result {
                Some((html_blob, text_blob)) => {
                    let body_html = html_blob.map(|b| decompress(&b)).transpose()?;
                    let body_text = text_blob.map(|b| decompress(&b)).transpose()?;
                    Ok(Some(MessageBody {
                        message_id,
                        body_html,
                        body_text,
                    }))
                }
                None => Ok(None),
            }
        })
        .await
    }

    /// Retrieve multiple message bodies in a single query.
    pub async fn get_batch(&self, message_ids: Vec<String>) -> Result<Vec<MessageBody>, String> {
        if message_ids.is_empty() {
            return Ok(Vec::new());
        }

        self.with_conn(move |conn| {
            let mut results = Vec::with_capacity(message_ids.len());

            // Process in chunks of 100 to stay within SQLite variable limits
            for chunk in message_ids.chunks(100) {
                let placeholders: String = chunk
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(", ");

                let sql =
                    format!("SELECT message_id, body_html, body_text FROM bodies WHERE message_id IN ({placeholders})");

                let mut stmt = conn.prepare(&sql).map_err(|e| format!("prepare batch get: {e}"))?;

                let param_values: Vec<Box<dyn rusqlite::types::ToSql>> =
                    chunk.iter().map(|id| Box::new(id.clone()) as Box<dyn rusqlite::types::ToSql>).collect();
                let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                    param_values.iter().map(AsRef::as_ref).collect();

                let rows = stmt
                    .query_map(param_refs.as_slice(), |row| {
                        let mid: String = row.get(0)?;
                        let html_blob: Option<Vec<u8>> = row.get(1)?;
                        let text_blob: Option<Vec<u8>> = row.get(2)?;
                        Ok((mid, html_blob, text_blob))
                    })
                    .map_err(|e| format!("query batch get: {e}"))?;

                for row in rows {
                    let (mid, html_blob, text_blob) =
                        row.map_err(|e| format!("map row: {e}"))?;
                    let body_html = html_blob.map(|b| decompress(&b)).transpose()?;
                    let body_text = text_blob.map(|b| decompress(&b)).transpose()?;
                    results.push(MessageBody {
                        message_id: mid,
                        body_html,
                        body_text,
                    });
                }
            }

            Ok(results)
        })
        .await
    }

    /// Delete bodies for given message IDs.
    pub async fn delete(&self, message_ids: Vec<String>) -> Result<u64, String> {
        if message_ids.is_empty() {
            return Ok(0);
        }

        self.with_conn(move |conn| {
            let mut deleted: u64 = 0;

            for chunk in message_ids.chunks(100) {
                let placeholders: String = chunk
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(", ");

                let sql = format!("DELETE FROM bodies WHERE message_id IN ({placeholders})");
                let mut stmt = conn
                    .prepare(&sql)
                    .map_err(|e| format!("prepare delete: {e}"))?;

                let param_values: Vec<Box<dyn rusqlite::types::ToSql>> = chunk
                    .iter()
                    .map(|id| Box::new(id.clone()) as Box<dyn rusqlite::types::ToSql>)
                    .collect();
                let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                    param_values.iter().map(AsRef::as_ref).collect();

                let count = stmt
                    .execute(param_refs.as_slice())
                    .map_err(|e| format!("delete bodies: {e}"))?;

                #[allow(clippy::cast_sign_loss)]
                {
                    deleted += count as u64;
                }
            }

            Ok(deleted)
        })
        .await
    }

    /// Get storage statistics.
    pub async fn stats(&self) -> Result<BodyStoreStats, String> {
        self.with_conn(|conn| {
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM bodies", [], |row| row.get(0))
                .map_err(|e| format!("count: {e}"))?;

            let total_html_bytes: i64 = conn
                .query_row(
                    "SELECT COALESCE(SUM(LENGTH(body_html)), 0) FROM bodies",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| format!("html size: {e}"))?;

            let total_text_bytes: i64 = conn
                .query_row(
                    "SELECT COALESCE(SUM(LENGTH(body_text)), 0) FROM bodies",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| format!("text size: {e}"))?;

            #[allow(clippy::cast_sign_loss)]
            Ok(BodyStoreStats {
                message_count: count as u64,
                compressed_html_bytes: total_html_bytes as u64,
                compressed_text_bytes: total_text_bytes as u64,
            })
        })
        .await
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BodyStoreStats {
    pub message_count: u64,
    pub compressed_html_bytes: u64,
    pub compressed_text_bytes: u64,
}
