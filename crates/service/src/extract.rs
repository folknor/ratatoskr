// Phase 7-4c: lots of items that 7-4d wires up in dispatch.rs. Tests
// inside this module exercise the lifecycle shape; the production
// callers land in the next slice.
#![allow(dead_code)]

//! Phase 7-4c: ExtractRuntime - Service-side text-extraction worker.
//!
//! Runtime owns the work queue + bounded-concurrency semaphore +
//! enqueue-dedupe HashSet + counters. The worker pops items, runs the
//! mime-routed extractor inside `tokio::task::spawn_blocking` with a
//! 30 s wallclock cap, persists the result to
//! `attachment_extracted_text`, and sets `attachments.text_indexed_at`
//! for every row referencing the same content_hash.
//!
//! ## What 7-4c does NOT do
//!
//! - **Re-index emission**: 7-4c does not yet emit
//!   `WriterCommand::Index` for messages whose attachments newly
//!   indexed. Phase 7-7 wires the search-writer fan-out. The DB
//!   state (`attachment_extracted_text` row + `text_indexed_at`)
//!   IS updated in 7-4c, so a future re-index reads canonical state.
//! - **Boot wiring**: ExtractRuntime is constructed but only via tests
//!   in 7-4c. The next slice (7-4d) wires construction into
//!   dispatch.rs's post-ready startup path and drain integration into
//!   lifecycle.rs.
//! - **Cancellation budget**: `spawn_blocking` is uncancellable.
//!   Drain abandons in-flight extractions; idempotent backfill resumes
//!   from the dropped work next boot. Drain budget is for queue
//!   receiver + sender drops, not for waiting on extraction threads.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use service_api::{ExtractCompleted, ExtractProgress, Notification};
use service_state::WriteDbState;
use tokio::sync::{Mutex, Semaphore, mpsc};

use crate::attachment_lock::SWEEP_LOCK;
use crate::boot_progress::NotificationSender;
use crate::text_extract::{ExtractionOutcome, MAX_INPUT_BYTES, PER_EXTRACTION_TIMEOUT_SECS,
    SkipReason, extract as run_extractor, truncate_on_char_boundary, MAX_EXTRACTED_TEXT_BYTES};

/// Bounded concurrency for in-flight extractions. Per the plan: cap 4
/// keeps PDF / OOXML CPU pressure manageable without serializing too
/// aggressively.
const WORKER_CONCURRENCY: usize = 4;

/// Bounded mpsc capacity. Backpressure: enqueuers (the
/// `attachment.fetch` handler and the `extract.backfill_kick` handler)
/// block on `send.await` when full. 256 lets a backfill emit a 1000-row
/// batch in chunks without dropping anything.
const COMMAND_QUEUE_CAPACITY: usize = 256;

/// Single extraction work item.
#[derive(Debug, Clone)]
pub(crate) struct ExtractWork {
    pub content_hash:  String,
    pub account_id:    String,
    pub message_id:    String,
    #[allow(dead_code)] // Surfaced in 7-7's re-index fan-out + 7-8's attribution.
    pub attachment_id: String,
}

pub(crate) struct ExtractRuntimeInner {
    closed: AtomicBool,
    in_flight_hashes: Mutex<HashSet<String>>,
    tx: mpsc::Sender<ExtractWork>,
    db: WriteDbState,
    app_data_dir: PathBuf,
    notification_tx: NotificationSender,
    service_generation: u32,
    /// Work-queue depth + in-flight extractions count. Decrements when
    /// a work item is fully processed (success or failure). When this
    /// reaches zero AND the queue receiver is empty we emit
    /// `ExtractCompleted`.
    queue_depth: AtomicU64,
    indexed_count: AtomicU64,
    skipped_count: AtomicU64,
    failed_count: AtomicU64,
}

#[derive(Clone)]
pub struct ExtractRuntime {
    inner: Arc<ExtractRuntimeInner>,
}

impl ExtractRuntime {
    /// Spawn the worker task and return a handle. The runtime owns
    /// the worker; dropping the last clone closes the mpsc which
    /// signals the worker to exit on its next `recv()`.
    pub fn new(
        db: WriteDbState,
        app_data_dir: PathBuf,
        notification_tx: NotificationSender,
        service_generation: u32,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<ExtractWork>(COMMAND_QUEUE_CAPACITY);
        let inner = Arc::new(ExtractRuntimeInner {
            closed: AtomicBool::new(false),
            in_flight_hashes: Mutex::new(HashSet::new()),
            tx,
            db,
            app_data_dir,
            notification_tx,
            service_generation,
            queue_depth: AtomicU64::new(0),
            indexed_count: AtomicU64::new(0),
            skipped_count: AtomicU64::new(0),
            failed_count: AtomicU64::new(0),
        });
        let runner_inner = Arc::clone(&inner);
        tokio::spawn(async move { run_worker(runner_inner, rx).await });
        Self { inner }
    }

    /// Enqueue an extraction work item. Idempotent against
    /// `in_flight_hashes`: a duplicate enqueue while the same hash is
    /// already in flight is a no-op. The worker's status-aware
    /// idempotency check separately covers DB-level deduplication
    /// (skip if `attachment_extracted_text` row exists at current
    /// schema_version with permanent status).
    pub async fn enqueue(&self, work: ExtractWork) -> Result<(), String> {
        if self.inner.closed.load(Ordering::Relaxed) {
            return Err("ExtractRuntime is shutting down".into());
        }

        // Dedupe: skip if this hash is already enqueued or in-flight.
        // This is the fast lane; the slow lane (DB-level idempotency)
        // is the worker's pre-flight check.
        {
            let mut hashes = self.inner.in_flight_hashes.lock().await;
            if !hashes.insert(work.content_hash.clone()) {
                return Ok(());
            }
        }

        self.inner.queue_depth.fetch_add(1, Ordering::Relaxed);
        match self.inner.tx.send(work.clone()).await {
            Ok(()) => Ok(()),
            Err(_) => {
                // Receiver dropped; runtime is gone. Roll back the
                // queue-depth + dedupe insertion so accounting stays
                // consistent.
                self.inner.queue_depth.fetch_sub(1, Ordering::Relaxed);
                self.inner.in_flight_hashes.lock().await.remove(&work.content_hash);
                Err("ExtractRuntime worker exited".into())
            }
        }
    }

    /// In-memory counter snapshot for `extract.status` IPC.
    pub fn status_snapshot(&self) -> (u64, u64, u64, u64) {
        (
            self.inner.queue_depth.load(Ordering::Relaxed),
            self.inner.indexed_count.load(Ordering::Relaxed),
            self.inner.skipped_count.load(Ordering::Relaxed),
            self.inner.failed_count.load(Ordering::Relaxed),
        )
    }

    /// Begin shutdown. Flips `closed` so future `enqueue` calls fail
    /// fast; the worker observes the closed mpsc sender on its next
    /// `recv()` and exits naturally. In-flight extractions continue
    /// to completion (they can't be cancelled - `spawn_blocking` is
    /// uncancellable). The drain budget is for the receiver/sender
    /// drop dance, not for awaiting in-flight work.
    pub fn shutdown(&self) {
        self.inner.closed.store(true, Ordering::Relaxed);
    }
}

async fn run_worker(
    inner: Arc<ExtractRuntimeInner>,
    mut rx: mpsc::Receiver<ExtractWork>,
) {
    let semaphore = Arc::new(Semaphore::new(WORKER_CONCURRENCY));

    while let Some(work) = rx.recv().await {
        let permit = match Arc::clone(&semaphore).acquire_owned().await {
            Ok(p) => p,
            Err(_) => return,
        };
        let inner_for_task = Arc::clone(&inner);
        tokio::spawn(async move {
            let _permit = permit;
            // Wrap individual-item processing in a spawned task so a
            // panic during extraction does not kill the whole worker.
            // JoinError::is_panic captures it for the supervisor log.
            let work_for_log = work.clone();
            let inner_for_inner = Arc::clone(&inner_for_task);
            let task = tokio::spawn(async move {
                process_one(inner_for_inner, work).await;
            });
            match task.await {
                Ok(()) => {}
                Err(e) if e.is_panic() => {
                    log::error!(
                        "ExtractRuntime worker panicked on hash {}: {e:?}",
                        work_for_log.content_hash,
                    );
                    inner_for_task.failed_count.fetch_add(1, Ordering::Relaxed);
                    finalize_item(&inner_for_task, &work_for_log.content_hash).await;
                }
                Err(e) => {
                    log::warn!(
                        "ExtractRuntime worker aborted on hash {}: {e:?}",
                        work_for_log.content_hash,
                    );
                    inner_for_task.failed_count.fetch_add(1, Ordering::Relaxed);
                    finalize_item(&inner_for_task, &work_for_log.content_hash).await;
                }
            }
        });
    }
    log::info!("ExtractRuntime worker exiting (rx closed)");
}

async fn process_one(inner: Arc<ExtractRuntimeInner>, work: ExtractWork) {
    let outcome = run_extraction_pipeline(&inner, &work).await;
    match outcome {
        ExtractionOutcome::Indexed { .. } => {
            inner.indexed_count.fetch_add(1, Ordering::Relaxed);
        }
        ExtractionOutcome::Skipped { .. } => {
            inner.skipped_count.fetch_add(1, Ordering::Relaxed);
        }
        ExtractionOutcome::Failed { .. } => {
            inner.failed_count.fetch_add(1, Ordering::Relaxed);
        }
    }
    finalize_item(&inner, &work.content_hash).await;
}

async fn finalize_item(inner: &Arc<ExtractRuntimeInner>, content_hash: &str) {
    inner.in_flight_hashes.lock().await.remove(content_hash);
    let prev = inner.queue_depth.fetch_sub(1, Ordering::Relaxed);
    let new_depth = prev.saturating_sub(1);

    // Emit per-item progress (Coalesce: latest-wins).
    let progress = Notification::ExtractProgress(ExtractProgress {
        service_generation: inner.service_generation,
        remaining: new_depth,
        indexed_in_session: inner.indexed_count.load(Ordering::Relaxed),
    });
    if let Err(e) = inner.notification_tx.send(progress).await {
        log::debug!("ExtractRuntime progress send failed: {e}");
    }

    // Emit ExtractCompleted when the queue drains.
    if new_depth == 0 {
        let completed = Notification::ExtractCompleted(ExtractCompleted {
            service_generation: inner.service_generation,
            indexed: inner.indexed_count.load(Ordering::Relaxed),
            skipped: inner.skipped_count.load(Ordering::Relaxed),
            failed: inner.failed_count.load(Ordering::Relaxed),
        });
        if let Err(e) = inner.notification_tx.send(completed).await {
            log::debug!("ExtractRuntime completed send failed: {e}");
        }
    }
}

async fn run_extraction_pipeline(
    inner: &Arc<ExtractRuntimeInner>,
    work: &ExtractWork,
) -> ExtractionOutcome {
    // Status-aware idempotency pre-flight: skip permanent statuses.
    let hash_for_check = work.content_hash.clone();
    let existing = inner
        .db
        .to_read_state()
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT status FROM attachment_extracted_text \
                     WHERE content_hash = ?1 AND schema_version = ?2",
                )
                .map_err(|e| format!("prepare idempotency check: {e}"))?;
            let mut rows = stmt
                .query_map(
                    rusqlite::params![hash_for_check, search::INDEX_SCHEMA_VERSION],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|e| format!("query idempotency check: {e}"))?;
            let first = rows.next().transpose().map_err(|e| e.to_string())?;
            Ok::<Option<String>, String>(first)
        })
        .await
        .unwrap_or(None);
    if let Some(status) = existing
        && is_permanent_status(&status)
    {
        log::debug!(
            "ExtractRuntime skip {} (already at permanent status {status})",
            work.content_hash,
        );
        return ExtractionOutcome::Skipped { reason: SkipReason::OpaqueMime };
    }

    // Fetch metadata for this attachment (filename + mime), needed by
    // the dispatcher to canonicalize the mime.
    let meta_account = work.account_id.clone();
    let meta_message = work.message_id.clone();
    let meta_attachment = work.attachment_id.clone();
    let meta = inner
        .db
        .to_read_state()
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT filename, mime_type FROM attachments \
                     WHERE account_id = ?1 AND message_id = ?2 AND id = ?3 LIMIT 1",
                )
                .map_err(|e| format!("prepare attachment meta: {e}"))?;
            let mut rows = stmt
                .query_map(
                    rusqlite::params![meta_account, meta_message, meta_attachment],
                    |row| Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    )),
                )
                .map_err(|e| format!("query attachment meta: {e}"))?;
            let first = rows.next().transpose().map_err(|e| e.to_string())?;
            Ok::<Option<(Option<String>, Option<String>)>, String>(first)
        })
        .await
        .unwrap_or(None);

    let (filename, mime_type) = meta
        .map(|(f, m)| (f.unwrap_or_default(), m.unwrap_or_default()))
        .unwrap_or_default();

    // Acquire SWEEP_LOCK.read() for the bytes-read window so eviction
    // cannot unlink the cache file mid-read.
    let _guard = SWEEP_LOCK.read().await;

    let cache_path = inner
        .app_data_dir
        .join("attachment_cache")
        .join(&work.content_hash);
    let bytes = match tokio::fs::read(&cache_path).await {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            persist_outcome_row(
                inner,
                &work.content_hash,
                &mime_type,
                &ExtractionOutcome::Skipped { reason: SkipReason::BytesGone },
            )
            .await;
            return ExtractionOutcome::Skipped { reason: SkipReason::BytesGone };
        }
        Err(e) => {
            log::warn!("read {} failed: {e}", cache_path.display());
            persist_outcome_row(
                inner,
                &work.content_hash,
                &mime_type,
                &ExtractionOutcome::Failed { error: format!("read: {e}") },
            )
            .await;
            return ExtractionOutcome::Failed { error: format!("read: {e}") };
        }
    };
    drop(_guard);

    if bytes.len() > MAX_INPUT_BYTES {
        let outcome = ExtractionOutcome::Skipped { reason: SkipReason::OversizeFile };
        persist_outcome_row(inner, &work.content_hash, &mime_type, &outcome).await;
        return outcome;
    }

    // Dispatch the CPU-bound extractor in spawn_blocking with a 30 s
    // wallclock cap. spawn_blocking is uncancellable; on timeout we
    // abandon the future (the thread continues in the background) and
    // record a Timeout outcome.
    let bytes_len = bytes.len();
    let mime_for_task = mime_type.clone();
    let filename_for_task = filename.clone();
    let extract_fut = tokio::task::spawn_blocking(move || {
        run_extractor(&bytes, &mime_for_task, &filename_for_task)
    });
    let outcome = match tokio::time::timeout(
        std::time::Duration::from_secs(PER_EXTRACTION_TIMEOUT_SECS),
        extract_fut,
    )
    .await
    {
        Ok(Ok(o)) => o,
        Ok(Err(join_err)) => {
            log::error!(
                "extractor JoinError on hash {} ({} bytes, mime {mime_type}): {join_err:?}",
                work.content_hash, bytes_len,
            );
            ExtractionOutcome::Failed { error: format!("join: {join_err}") }
        }
        Err(_) => {
            log::warn!(
                "extractor timeout on hash {} ({} bytes, mime {mime_type})",
                work.content_hash, bytes_len,
            );
            ExtractionOutcome::Skipped { reason: SkipReason::Timeout }
        }
    };

    // Truncate Indexed text to MAX_EXTRACTED_TEXT_BYTES on a UTF-8
    // char boundary before persisting.
    let outcome = match outcome {
        ExtractionOutcome::Indexed { text } => ExtractionOutcome::Indexed {
            text: truncate_on_char_boundary(text, MAX_EXTRACTED_TEXT_BYTES),
        },
        other => other,
    };

    persist_outcome_row(inner, &work.content_hash, &mime_type, &outcome).await;

    // Set attachments.text_indexed_at for every row referencing this
    // content_hash, so the backfill scan no longer picks them up.
    if matches!(outcome, ExtractionOutcome::Indexed { .. }) {
        let hash = work.content_hash.clone();
        let _ = inner
            .db
            .with_conn(move |conn| {
                let now: i64 = chrono::Utc::now().timestamp();
                conn.execute(
                    "UPDATE attachments SET text_indexed_at = ?1 \
                     WHERE content_hash = ?2 AND text_indexed_at IS NULL",
                    rusqlite::params![now, hash],
                )
                .map_err(|e| format!("update text_indexed_at: {e}"))?;
                Ok(())
            })
            .await;
    }

    outcome
}

async fn persist_outcome_row(
    inner: &Arc<ExtractRuntimeInner>,
    content_hash: &str,
    mime_type: &str,
    outcome: &ExtractionOutcome,
) {
    let now: i64 = chrono::Utc::now().timestamp();
    let (status, text): (String, Option<String>) = match outcome {
        ExtractionOutcome::Indexed { text } => ("indexed".into(), Some(text.clone())),
        ExtractionOutcome::Skipped { reason } => (reason.status_string().to_string(), None),
        ExtractionOutcome::Failed { .. } => ("failed:transient".into(), None),
    };
    let hash = content_hash.to_string();
    let mime = mime_type.to_string();
    let result = inner
        .db
        .with_conn(move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO attachment_extracted_text \
                 (content_hash, mime_type, extracted_text, status, extracted_at, schema_version) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![hash, mime, text, status, now, search::INDEX_SCHEMA_VERSION],
            )
            .map_err(|e| format!("upsert attachment_extracted_text: {e}"))?;
            Ok(())
        })
        .await;
    if let Err(e) = result {
        log::warn!("persist_outcome_row {content_hash}: {e}");
    }
}

/// Status strings that signal "do not retry on next enqueue."
fn is_permanent_status(status: &str) -> bool {
    matches!(
        status,
        "indexed"
            | "skipped:opaque"
            | "skipped:encrypted"
            | "skipped:oversize"
            | "skipped:encoding"
            | "skipped:empty"
            | "skipped:ocr"
            | "skipped:unknown_mime"
            | "skipped:privacy"
            | "skipped:zipbomb"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn enqueue_after_shutdown_returns_err() {
        // Build a runtime with a dummy DB - we only need the lifecycle
        // shape to be correct; no actual extraction runs because we
        // shut down before processing.
        let conn = rusqlite::Connection::open_in_memory().expect("conn");
        let db = WriteDbState::from_arc(Arc::new(std::sync::Mutex::new(conn)));
        let (tx, _rx) = mpsc::channel::<Vec<u8>>(8);
        let notification_tx = NotificationSender::new(tx);
        let runtime = ExtractRuntime::new(
            db,
            std::path::PathBuf::from("."),
            notification_tx,
            0,
        );
        runtime.shutdown();
        let result = runtime
            .enqueue(ExtractWork {
                content_hash: "abc".into(),
                account_id: "acc".into(),
                message_id: "msg".into(),
                attachment_id: "att".into(),
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn enqueue_dedupes_concurrent_same_hash() {
        // Build a runtime; enqueue the same hash twice without letting
        // the worker process. The second call should be a no-op
        // (Ok with no extra item enqueued).
        let conn = rusqlite::Connection::open_in_memory().expect("conn");
        // Schema with attachment_extracted_text so the worker pre-
        // flight query doesn't error. The test doesn't drive the
        // worker to completion - shutdown after the dedupe assertion.
        conn.execute_batch(
            "CREATE TABLE attachment_extracted_text (\
                content_hash TEXT PRIMARY KEY, mime_type TEXT, \
                extracted_text TEXT, status TEXT NOT NULL, \
                extracted_at INTEGER NOT NULL, schema_version INTEGER NOT NULL);",
        ).expect("schema");
        let db = WriteDbState::from_arc(Arc::new(std::sync::Mutex::new(conn)));
        let (tx, _rx) = mpsc::channel::<Vec<u8>>(8);
        let notification_tx = NotificationSender::new(tx);
        let runtime = ExtractRuntime::new(
            db,
            std::path::PathBuf::from("."),
            notification_tx,
            0,
        );

        // Pre-load the dedupe set so the second enqueue is dedupe'd.
        runtime.inner.in_flight_hashes.lock().await.insert("abc".into());
        let work = ExtractWork {
            content_hash: "abc".into(),
            account_id: "acc".into(),
            message_id: "msg".into(),
            attachment_id: "att".into(),
        };
        let result = runtime.enqueue(work).await;
        assert!(result.is_ok(), "dedupe path should be Ok no-op");
        assert_eq!(runtime.status_snapshot().0, 0, "queue_depth should not increment on dedupe");
        runtime.shutdown();
    }
}
