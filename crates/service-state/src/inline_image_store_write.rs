//! Service-only writer half of the inline image store.
//!
//! Phase 3 mirror of `body_store_write.rs`: read state lives in
//! `crates/stores/`, write state lives here. Both halves open their
//! own SQLite connection against the same on-disk
//! `inline_images.db` file; SQLite WAL handles
//! multi-reader-single-writer concurrency.

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, params};

pub use store::inline_image_store::{InlineImage, MAX_INLINE_STORE_BYTES};

/// Writer-side state for the on-disk inline image store.
#[derive(Clone)]
pub struct InlineImageStoreWriteState {
    conn: Arc<Mutex<Connection>>,
}

impl InlineImageStoreWriteState {
    /// Open the writer-side connection. Service constructs this in
    /// `BootPhase::OpeningBodyAndInlineStores` (added in Phase 3
    /// task 12).
    pub fn init(app_data_dir: &Path) -> Result<Self, String> {
        let conn = store::inline_image_store::open_inline_image_store_connection(app_data_dir)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Run a closure on the writer connection via `spawn_blocking`.
    pub async fn with_conn<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&Connection) -> Result<T, String> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| format!("inline image write lock poisoned: {e}"))?;
            f(&conn)
        })
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?
    }

    /// Store a single inline image. No-op if the content hash already
    /// exists (content-addressed; identical blobs share one row).
    pub async fn put(
        &self,
        content_hash: String,
        data: Vec<u8>,
        mime_type: String,
    ) -> Result<(), String> {
        log::debug!(
            "Storing inline image hash={} mime={} size={}",
            content_hash,
            mime_type,
            data.len()
        );
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

    /// Delete the supplied content hashes. Returns the row count
    /// removed.
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

    /// Evict oldest inline image blobs until the store fits under
    /// `max_bytes`.
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
                    Ok((
                        row.get::<_, String>("content_hash")?,
                        row.get::<_, i64>("size")?,
                    ))
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

    /// Clear every blob from the store. Used for the debug "wipe
    /// inline cache" surface in dev tools.
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn put_then_separately_opened_reader_finds_blob() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let writer = InlineImageStoreWriteState::init(tmp.path()).expect("writer init");
        let reader = store::inline_image_store::InlineImageStoreReadState::init(tmp.path())
            .expect("reader init");

        writer
            .put("h1".into(), vec![1, 2, 3], "image/png".into())
            .await
            .expect("put");

        let got = reader.get("h1".into()).await.expect("get");
        assert!(got.is_some());
        let (data, mime) = got.expect("get");
        assert_eq!(data, vec![1, 2, 3]);
        assert_eq!(mime, "image/png");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delete_hashes_removes_rows() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let writer = InlineImageStoreWriteState::init(tmp.path()).expect("writer init");
        let reader = store::inline_image_store::InlineImageStoreReadState::init(tmp.path())
            .expect("reader init");

        writer
            .put_batch(vec![InlineImage {
                content_hash: "h1".into(),
                data: vec![9],
                mime_type: "image/jpeg".into(),
            }])
            .await
            .expect("put_batch");

        let removed = writer
            .delete_hashes(vec!["h1".into()])
            .await
            .expect("delete");
        assert_eq!(removed, 1);

        let got = reader.get("h1".into()).await.expect("get");
        assert!(got.is_none(), "row should be gone");
    }
}
