//! Service-only writer half of the body store.
//!
//! Phase 3 of `docs/service/phase-3-plan.md` introduces a read/write
//! split at the type level: `store::body_store::BodyStoreReadState`
//! lives in `crates/stores/` (UI-visible); `BodyStoreWriteState` lives
//! here (Service-only). The split is enforced by Cargo dependency
//! graph - the `app` crate does not depend on `service-state`, so
//! `BodyStoreWriteState` cannot be reached from UI source files even
//! with `pub` visibility.
//!
//! Both halves open their own SQLite connection against the same
//! on-disk `bodies.db` file. SQLite's WAL handles
//! multi-reader-single-writer concurrency; the Rust types enforce
//! *which side* of the API surface a consumer sees.
//!
//! For Phase 3 the writer half hosts `put_batch` (the canonical sync
//! persistence path) plus `put` and `delete` (used by janitor /
//! prune-on-resync paths today). When task 5/7 narrows the
//! `SyncProviderCtx` to take `&BodyStoreWriteState` and the existing
//! write methods on `BodyStoreReadState` are removed, this type
//! becomes the only path through which body bytes can be written.

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, params};

pub use store::body_store::MessageBody;

/// Writer-side state for the on-disk body store.
///
/// Holds an `Arc<Mutex<Connection>>` distinct from the read half's
/// connection. SQLite WAL handles the file-level coordination: the
/// writer's `BEGIN IMMEDIATE` blocks until any in-flight read finishes;
/// readers do not block writers thanks to the WAL.
#[derive(Clone)]
pub struct BodyStoreWriteState {
    conn: Arc<Mutex<Connection>>,
}

impl BodyStoreWriteState {
    /// Open the writer-side connection. Service constructs this in
    /// `BootPhase::OpeningBodyAndInlineStores` (added in Phase 3
    /// task 12).
    pub fn init(app_data_dir: &Path) -> Result<Self, String> {
        let conn = store::body_store::open_body_store_connection(app_data_dir)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Construct from an already-open connection. Useful for tests
    /// and for the boot path's deferred initialization.
    pub fn from_arc(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Run a closure on the writer connection via `spawn_blocking`.
    /// Test helper / janitor escape hatch; production paths should
    /// prefer the typed methods below.
    pub async fn with_conn<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&Connection) -> Result<T, String> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| format!("body store write lock poisoned: {e}"))?;
            f(&conn)
        })
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?
    }

    /// Store a single message body (compressed). Provided for parity
    /// with the read state's pre-Phase-3 surface; production paths use
    /// `put_batch`.
    pub async fn put(
        &self,
        message_id: String,
        body_html: Option<String>,
        body_text: Option<String>,
    ) -> Result<(), String> {
        log::debug!("Storing body for message_id={message_id}");
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let html_blob = body_html
                .as_deref()
                .map(store::body_store::compress_body)
                .transpose()?;
            let text_blob = body_text
                .as_deref()
                .map(store::body_store::compress_body)
                .transpose()?;

            let conn = conn
                .lock()
                .map_err(|e| format!("body store write lock poisoned: {e}"))?;
            conn.execute(
                "INSERT OR REPLACE INTO bodies (message_id, body_html, body_text)
                 VALUES (?1, ?2, ?3)",
                params![message_id, html_blob, text_blob],
            )
            .map_err(|e| format!("body store put: {e}"))?;
            Ok(())
        })
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?
    }

    /// Store multiple message bodies in a single transaction. The
    /// canonical sync persistence path.
    pub async fn put_batch(&self, bodies: Vec<MessageBody>) -> Result<(), String> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            #[allow(clippy::type_complexity)]
            let compressed: Vec<(String, Option<Vec<u8>>, Option<Vec<u8>>)> = bodies
                .iter()
                .map(|body| {
                    let html_blob = body
                        .body_html
                        .as_deref()
                        .map(store::body_store::compress_body)
                        .transpose()?;
                    let text_blob = body
                        .body_text
                        .as_deref()
                        .map(store::body_store::compress_body)
                        .transpose()?;
                    Ok((body.message_id.clone(), html_blob, text_blob))
                })
                .collect::<Result<Vec<_>, String>>()?;

            let conn = conn
                .lock()
                .map_err(|e| format!("body store write lock poisoned: {e}"))?;
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
                for (message_id, html_blob, text_blob) in &compressed {
                    stmt.execute(params![message_id, html_blob, text_blob])
                        .map_err(|e| format!("batch put row: {e}"))?;
                }
            }
            tx.commit().map_err(|e| format!("body store commit: {e}"))?;
            Ok(())
        })
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?
    }

    /// Delete bodies for the given message IDs. Returns the number of
    /// rows actually removed.
    pub async fn delete(&self, message_ids: Vec<String>) -> Result<u64, String> {
        if message_ids.is_empty() {
            return Ok(0);
        }
        log::debug!("Pruning {} bodies from body store", message_ids.len());
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn put_batch_writes_then_read_state_reads_them_back() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let writer = BodyStoreWriteState::init(tmp.path()).expect("writer init");
        let reader = store::body_store::BodyStoreReadState::init(tmp.path()).expect("reader init");

        let bodies = vec![
            MessageBody {
                message_id: "m1".into(),
                body_html: Some("<p>one</p>".into()),
                body_text: Some("one".into()),
            },
            MessageBody {
                message_id: "m2".into(),
                body_html: None,
                body_text: Some("two".into()),
            },
        ];
        writer.put_batch(bodies).await.expect("put_batch");

        let got = reader
            .get_batch(vec!["m1".into(), "m2".into()])
            .await
            .expect("get_batch");
        assert_eq!(got.len(), 2);
        // Order is not guaranteed.
        let m1 = got.iter().find(|b| b.message_id == "m1").expect("m1");
        let m2 = got.iter().find(|b| b.message_id == "m2").expect("m2");
        assert_eq!(m1.body_html.as_deref(), Some("<p>one</p>"));
        assert_eq!(m2.body_html, None);
        assert_eq!(m2.body_text.as_deref(), Some("two"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delete_removes_rows() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let writer = BodyStoreWriteState::init(tmp.path()).expect("writer init");
        let reader = store::body_store::BodyStoreReadState::init(tmp.path()).expect("reader init");

        writer
            .put_batch(vec![MessageBody {
                message_id: "m1".into(),
                body_html: None,
                body_text: Some("body".into()),
            }])
            .await
            .expect("put_batch");

        let removed = writer
            .delete(vec!["m1".into()])
            .await
            .expect("delete");
        assert_eq!(removed, 1);
        let got = reader.get_batch(vec!["m1".into()]).await.expect("get_batch");
        assert!(got.is_empty(), "row should be gone");
    }
}
