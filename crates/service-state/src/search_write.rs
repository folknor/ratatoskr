//! Service-only handle to the Tantivy search writer task.
//!
//! Phase 3 of `docs/service/phase-3-plan.md` relocates Tantivy writer
//! ownership into a Service-internal task. The public type is a cheap
//! `Clone` mpsc handle - the actual `IndexWriter` lives inside
//! `crates/service/src/search_writer.rs::run_writer_task` and never
//! escapes that task. This is the architectural fix for the Phase 3
//! plan's revision-1 launch-blocker (`block_on(send)` from inside
//! `IndexWriter::commit()` would deadlock when reached from an async
//! call site like `crates/sync/src/persistence.rs:58`).
//!
//! The handle's methods send a `WriterCommand` over the mpsc and
//! await an `oneshot` ack. The writer task processes commands
//! sequentially, applying tantivy mutations via
//! `tokio::task::block_in_place` (multi-thread runtime cooperative
//! bridge from async to sync) and committing on cadence triggers.

use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};

pub use search::SearchDocument;

/// Initial commit-cadence parameters. The Service-side writer task
/// uses these as defaults; tests can override via the constructor.
pub mod cadence {
    use std::time::Duration;

    /// Commit when this many docs have been queued since the last
    /// commit. Picked to amortise fsync cost over ~10 JMAP
    /// `SYNC_BATCH_SIZE = 100` batches.
    pub const COMMIT_DOC_THRESHOLD: u64 = 1000;
    /// Commit when this much wall-clock time has passed since the
    /// first uncommitted doc. Bounds search-result staleness.
    pub const COMMIT_TIME_THRESHOLD: Duration = Duration::from_millis(2000);
    /// `mpsc` capacity. Backpressure: a producer that fills the queue
    /// (e.g., heavy sync flooding the writer) blocks on `send.await`.
    /// 256 lets a single sync run buffer ~25 batches' worth of
    /// commands without stalling.
    pub const COMMAND_QUEUE_CAPACITY: usize = 256;
    /// `IndexCommitted` notification send-deadline. The plan H5 fix:
    /// a wedged UI consumer cannot park the writer indefinitely. On
    /// timeout the notification is dropped with a warning; the next
    /// commit will fire another one.
    pub const INDEX_COMMITTED_SEND_TIMEOUT: Duration = Duration::from_secs(30);
}

/// Internal command queue payload. `pub` so the writer task body
/// (in `service`) can pattern-match against the variants without a
/// circular dep between `service-state` and `service`.
pub enum WriterCommand {
    Index {
        docs: Vec<SearchDocument>,
        ack: oneshot::Sender<Result<(), String>>,
    },
    Delete {
        ids: Vec<String>,
        ack: oneshot::Sender<Result<(), String>>,
    },
    Clear {
        ack: oneshot::Sender<Result<(), String>>,
    },
    FlushNow {
        ack: oneshot::Sender<Result<(), String>>,
    },
}

/// Cheap `Clone` handle to the Service-side search writer task. The
/// `Arc<mpsc::Sender>` is the only piece of state; `clone()` is one
/// pointer copy.
#[derive(Clone)]
pub struct SearchWriteHandle {
    tx: Arc<mpsc::Sender<WriterCommand>>,
}

impl SearchWriteHandle {
    /// Construct from a raw sender (used by `service::search_writer::spawn`).
    pub fn from_sender(tx: mpsc::Sender<WriterCommand>) -> Self {
        Self { tx: Arc::new(tx) }
    }

    /// Index a batch of documents. The writer task may or may not
    /// commit before acking - cadence triggers (size, time, `FlushNow`)
    /// drive commit timing. The ack confirms the docs are *queued*
    /// (and that any prior batch has not been lost).
    pub async fn index_messages_batch(&self, docs: Vec<SearchDocument>) -> Result<(), String> {
        if docs.is_empty() {
            return Ok(());
        }
        let (ack_tx, ack_rx) = oneshot::channel();
        self.tx
            .send(WriterCommand::Index { docs, ack: ack_tx })
            .await
            .map_err(|_| "search writer task gone".to_string())?;
        ack_rx
            .await
            .map_err(|_| "search writer ack dropped".to_string())?
    }

    /// Delete documents by message_id. Same cadence rules as
    /// `index_messages_batch`.
    pub async fn delete_messages_batch(&self, ids: Vec<String>) -> Result<(), String> {
        if ids.is_empty() {
            return Ok(());
        }
        let (ack_tx, ack_rx) = oneshot::channel();
        self.tx
            .send(WriterCommand::Delete { ids, ack: ack_tx })
            .await
            .map_err(|_| "search writer task gone".to_string())?;
        ack_rx
            .await
            .map_err(|_| "search writer ack dropped".to_string())?
    }

    /// Convenience wrapper for single-message deletion.
    pub async fn delete_message(&self, id: String) -> Result<(), String> {
        self.delete_messages_batch(vec![id]).await
    }

    /// Drop every document and force a commit. Used by the Phase 7
    /// "rebuild index" debug path and by tests.
    pub async fn clear_index(&self) -> Result<(), String> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.tx
            .send(WriterCommand::Clear { ack: ack_tx })
            .await
            .map_err(|_| "search writer task gone".to_string())?;
        ack_rx
            .await
            .map_err(|_| "search writer ack dropped".to_string())?
    }

    /// Force an immediate commit even if cadence triggers haven't
    /// fired. Used by `sync.completed` handler before the runner emits
    /// its terminal notification, and by the drain path before the
    /// Service exits.
    pub async fn flush_now(&self) -> Result<(), String> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.tx
            .send(WriterCommand::FlushNow { ack: ack_tx })
            .await
            .map_err(|_| "search writer task gone".to_string())?;
        ack_rx
            .await
            .map_err(|_| "search writer ack dropped".to_string())?
    }
}
