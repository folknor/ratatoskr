// Phase 7-4c: lots of items that 7-4d wires up in dispatch.rs. Tests
// inside this module exercise the lifecycle shape; the production
// callers land in the next slice.
#![allow(dead_code)]

//! Phase 7-4c / 7-7: ExtractRuntime - Service-side text-extraction
//! worker.
//!
//! Runtime owns the work queue + bounded-concurrency semaphore +
//! enqueue-dedupe HashSet + counters. The worker pops items, runs the
//! mime-routed extractor inside `tokio::task::spawn_blocking` with a
//! 30 s wallclock cap, persists the result to
//! `attachment_extracted_text`, sets `attachments.text_indexed_at` for
//! every row referencing the same content_hash, and (7-7) fans out
//! `WriterCommand::Index` for every message whose attachment list
//! changed.
//!
//! ## What this module does NOT do
//!
//! - **Boot wiring**: ExtractRuntime is constructed but only via tests
//!   today. The deferred 7-4d producer in `dispatch.rs` will pass
//!   clones of the boot-installed `SearchWriteHandle` and a
//!   `BodyStoreReadState` opened against `app_data_dir` into
//!   `ExtractRuntime::new`.
//! - **Cancellation budget**: `spawn_blocking` is uncancellable.
//!   Drain abandons in-flight extractions; idempotent backfill resumes
//!   from the dropped work next boot. Drain budget is for queue
//!   receiver + sender drops, not for waiting on extraction threads.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use search::{AttachmentDocFragment, SearchDocument};
use service_api::{ExtractCompleted, ExtractProgress, Notification};
use service_state::{SearchWriteHandle, WriteDbState};
use store::body_store::BodyStoreReadState;
use tokio::sync::{Mutex, Semaphore, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

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
    pub content_hash:  db::blob_hash::BlobHash,
    pub account_id:    String,
    pub message_id:    String,
    #[allow(dead_code)] // Surfaced in 7-7's re-index fan-out + 7-8's attribution.
    pub attachment_id: String,
}

pub(crate) struct ExtractRuntimeInner {
    closed: AtomicBool,
    in_flight_hashes: Mutex<HashSet<db::blob_hash::BlobHash>>,
    tx: mpsc::Sender<ExtractWork>,
    db: WriteDbState,
    read_db: db::db::ReadDbState,
    /// Service-side root, retained for diagnostic logging only.
    /// Attachments roadmap Phase 3 routed byte reads through
    /// `materialize_blob` against `pack_store` below, so the
    /// runtime no longer reads `attachment_cache/<hash>` directly.
    app_data_dir: PathBuf,
    /// PackStore handle for byte fetches. Populated at construction;
    /// `None` is only possible if the boot path's PackStore
    /// installation failed, which is a hard boot error elsewhere.
    pack_store: Option<Arc<store::PackStore>>,
    /// `BootSharedState` handle the materialize helper needs to read
    /// `app_data_dir` and `pack_store` together. Cheap clone.
    boot_state: Arc<crate::boot::BootSharedState>,
    notification_tx: NotificationSender,
    service_generation: u32,
    /// Phase 7-7: producer-side enrichment. After a successful
    /// extraction the worker reads canonical DB state for every
    /// message referencing the just-indexed `content_hash`, builds a
    /// fresh `SearchDocument` (body fields from `body_read`,
    /// scalar/attachment fields from `db`), and emits a single
    /// `WriterCommand::Index` via this handle. Sync's `Index` commands
    /// continue to emit thin docs with `attachments: Vec::new()`; this
    /// runtime is the only path that puts attachment text into the
    /// search index.
    search_write: SearchWriteHandle,
    body_read: BodyStoreReadState,
    /// Phase 7-4d: cancellation signal for the worker. `shutdown()`
    /// cancels this token; the worker's `tokio::select!` falls through
    /// to its `cancelled()` arm and exits, releasing the worker's
    /// `Arc<Inner>` clone. With both Arcs gone the inner drops, which
    /// drops the held `NotificationSender` + `SearchWriteHandle`
    /// clones - that's what lets the dispatch-side writer-task drain
    /// observe EOF.
    cancellation: CancellationToken,
    /// Phase 7-4d: worker `JoinHandle` so `shutdown()` can `.await`
    /// the worker's exit. Taken on first shutdown call; subsequent
    /// calls see `None` and return immediately. `std::sync::Mutex` -
    /// the access points (set during `new`, take during `shutdown`)
    /// are well-bounded and never held across an `.await`.
    worker_handle: std::sync::Mutex<Option<JoinHandle<()>>>,
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
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: WriteDbState,
        read_db: db::db::ReadDbState,
        app_data_dir: PathBuf,
        boot_state: Arc<crate::boot::BootSharedState>,
        search_write: SearchWriteHandle,
        body_read: BodyStoreReadState,
        notification_tx: NotificationSender,
        service_generation: u32,
        cancellation: CancellationToken,
    ) -> Self {
        let pack_store = boot_state.pack_store();
        let (tx, rx) = mpsc::channel::<ExtractWork>(COMMAND_QUEUE_CAPACITY);
        let inner = Arc::new(ExtractRuntimeInner {
            closed: AtomicBool::new(false),
            in_flight_hashes: Mutex::new(HashSet::new()),
            tx,
            db,
            read_db,
            app_data_dir,
            pack_store,
            boot_state,
            notification_tx,
            service_generation,
            search_write,
            body_read,
            cancellation,
            worker_handle: std::sync::Mutex::new(None),
            queue_depth: AtomicU64::new(0),
            indexed_count: AtomicU64::new(0),
            skipped_count: AtomicU64::new(0),
            failed_count: AtomicU64::new(0),
        });
        let runner_inner = Arc::clone(&inner);
        let handle = tokio::spawn(async move { run_worker(runner_inner, rx).await });
        inner
            .worker_handle
            .lock()
            .expect("worker_handle mutex poisoned at construction")
            .replace(handle);
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
            if !hashes.insert(work.content_hash) {
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

    /// L5 fix: non-blocking enqueue variant for callers that must not
    /// stall when the worker mpsc is full. Used by `attachment.fetch`
    /// (cache-miss / cache-hit paths) where blocking the user's UI
    /// fetch on indexing-queue capacity is the wrong trade-off. On
    /// queue-full or shut-down receiver, returns Ok(()) silently and
    /// rolls back the dedupe state - the next backfill kick will
    /// re-emit the row, so a missed enqueue self-heals on the hourly
    /// cadence.
    pub fn try_enqueue(&self, work: ExtractWork) -> Result<(), String> {
        if self.inner.closed.load(Ordering::Relaxed) {
            return Err("ExtractRuntime is shutting down".into());
        }

        let hash = work.content_hash;
        let inserted = match self.inner.in_flight_hashes.try_lock() {
            Ok(mut set) => set.insert(hash),
            Err(_) => {
                // Concurrent dedupe-set lock contention; treat as
                // "another caller is mid-enqueue" and skip.
                return Ok(());
            }
        };
        if !inserted {
            return Ok(());
        }
        self.inner.queue_depth.fetch_add(1, Ordering::Relaxed);
        match self.inner.tx.try_send(work) {
            Ok(()) => Ok(()),
            Err(_) => {
                self.inner.queue_depth.fetch_sub(1, Ordering::Relaxed);
                if let Ok(mut hashes) = self.inner.in_flight_hashes.try_lock() {
                    hashes.remove(&hash);
                }
                log::debug!(
                    "ExtractRuntime try_enqueue: queue full or receiver dropped for {hash} \
                     (rolled back; next backfill kick will re-emit)",
                );
                Ok(())
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
    /// fast; cancels the worker's cancellation token so its
    /// `tokio::select!` short-circuits to the `cancelled()` arm; awaits
    /// the worker `JoinHandle` so the dispatch-side drain genuinely
    /// observes worker termination.
    ///
    /// In-flight extractions inside `spawn_blocking` are uncancellable
    /// and are abandoned: the per-item task is dropped together with
    /// the worker, so the actual `spawn_blocking` thread continues to
    /// completion against the tokio blocking pool but its result is
    /// discarded. Idempotent backfill (7-6) re-extracts on next boot
    /// from canonical DB state. The drain budget is for the
    /// `JoinHandle::await`, not for awaiting in-flight extraction
    /// threads.
    pub async fn shutdown(&self) {
        self.inner.closed.store(true, Ordering::Relaxed);
        self.inner.cancellation.cancel();
        let handle = self
            .inner
            .worker_handle
            .lock()
            .expect("worker_handle mutex poisoned during shutdown")
            .take();
        if let Some(h) = handle
            && let Err(e) = h.await
        {
            log::warn!("ExtractRuntime worker join error during shutdown: {e}");
        }
    }
}

async fn run_worker(
    inner: Arc<ExtractRuntimeInner>,
    mut rx: mpsc::Receiver<ExtractWork>,
) {
    let semaphore = Arc::new(Semaphore::new(WORKER_CONCURRENCY));
    let cancellation = inner.cancellation.clone();
    // H1 fix: track per-item tasks in a JoinSet so the worker can
    // abort + await them on shutdown. Pre-fix the per-item spawns were
    // detached - shutdown only awaited the worker, but per-item futures
    // continued running with their own Arc<Inner> clones, blocking the
    // dispatch-side writer-task drain (which waits for every
    // SearchWriteHandle clone to drop before observing EOF). Aborting
    // the JoinSet drops the per-item futures at their next await
    // point, releasing Arc<Inner>; the underlying spawn_blocking
    // threads are still uncancellable and continue to completion in
    // the tokio blocking pool, but they don't hold Arc<Inner>, so the
    // writer-task drain is unblocked.
    //
    // Single-level spawn (collapsed from the prior outer-plus-inner
    // pattern): JoinSet's join_next returns Result<(), JoinError>, so
    // panic supervision is preserved at the worker level - a per-item
    // panic is logged and the worker keeps draining. The trade-off vs
    // the prior pattern: on panic we lose the content_hash bound to
    // the failed work, so finalize_item is not called - the hash sticks
    // in in_flight_hashes for the lifetime of this runtime instance.
    // Acceptable: in_flight_hashes is per-runtime, so the next boot
    // sees a fresh empty set; backfill re-enqueues the hash, and the
    // pre-flight either short-circuits (if a permanent row is now
    // present) or re-runs.
    let mut tasks: tokio::task::JoinSet<()> = tokio::task::JoinSet::new();

    loop {
        tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                log::debug!("ExtractRuntime worker cancelled, draining {} per-item tasks", tasks.len());
                break;
            }
            // Reap completed per-item tasks so the JoinSet doesn't
            // grow without bound while the worker keeps accepting
            // work. The if-guard prevents an empty JoinSet from
            // returning None and starving the recv arm.
            Some(result) = tasks.join_next(), if !tasks.is_empty() => {
                if let Err(e) = result {
                    if e.is_panic() {
                        log::error!(
                            "ExtractRuntime per-item task panicked: {e:?} \
                             (content_hash unrecoverable; hash remains in in_flight_hashes \
                             for the lifetime of this runtime, will be re-enqueued by next \
                             boot's backfill)",
                        );
                        inner.failed_count.fetch_add(1, Ordering::Relaxed);
                    } else {
                        log::warn!("ExtractRuntime per-item task aborted: {e:?}");
                    }
                }
            }
            maybe_work = rx.recv() => match maybe_work {
                Some(work) => {
                    let permit = match Arc::clone(&semaphore).acquire_owned().await {
                        Ok(p) => p,
                        Err(_) => break,
                    };
                    let inner_for_task = Arc::clone(&inner);
                    tasks.spawn(async move {
                        let _permit = permit;
                        process_one(inner_for_task, work).await;
                    });
                }
                None => {
                    log::debug!("ExtractRuntime worker rx closed, draining {} per-item tasks", tasks.len());
                    break;
                }
            }
        }
    }

    // Drain: abort any in-flight per-item tasks and await them so
    // their Arc<Inner> clones drop. shutdown() then observes the
    // worker JoinHandle's exit, by which point the only remaining
    // Arc<Inner> is the one stored in ExtractRuntime itself.
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}

async fn process_one(inner: Arc<ExtractRuntimeInner>, work: ExtractWork) {
    let outcome = run_extraction_pipeline(&inner, &work).await;
    match outcome {
        ExtractionOutcome::Indexed { .. } => {
            inner.indexed_count.fetch_add(1, Ordering::Relaxed);
            // Phase 7-7: re-index every message that references this
            // hash. Failures here log but do not retry; DB state is
            // idempotent and a future enqueue can re-emit.
            fan_out_reindex(&inner, &work.content_hash).await;
        }
        ExtractionOutcome::Skipped { .. } => {
            inner.skipped_count.fetch_add(1, Ordering::Relaxed);
        }
        ExtractionOutcome::Failed { .. } => {
            inner.failed_count.fetch_add(1, Ordering::Relaxed);
        }
        ExtractionOutcome::AlreadyResolved { .. } => {
            // C3 fix: pre-flight already handled text_indexed_at
            // UPDATE + fan_out_reindex (when applicable). The
            // original extraction's outcome already incremented one
            // of indexed/skipped/failed; counting it again here would
            // inflate the IPC totals. Intentional no-op.
        }
    }
    finalize_item(&inner, &work.content_hash).await;
}

async fn finalize_item(inner: &Arc<ExtractRuntimeInner>, content_hash: &db::blob_hash::BlobHash) {
    // M11 fix: read the in-flight set's len under the same lock that
    // does the remove, and gate completion on (new_depth == 0 &&
    // in_flight_empty). Pre-fix the gate was new_depth == 0 only,
    // which races against a concurrent enqueue parked at tx.send.await
    // (mpsc full): the new item's hashes.insert had already happened,
    // but its fetch_add hadn't, so depth read 0 momentarily and
    // fired ExtractCompleted while the new item sat in tx.send. The
    // hashes set is the canonical "items currently being processed
    // anywhere" signal because enqueue inserts BEFORE fetch_add and
    // BEFORE send, and finalize_item removes at terminal outcome -
    // a non-empty set means there's still work, regardless of which
    // bookkeeping queue holds it.
    let in_flight_empty = {
        let mut hashes = inner.in_flight_hashes.lock().await;
        hashes.remove(content_hash);
        hashes.is_empty()
    };
    // L10 fix: guard against an accidental double-finalize wrapping
    // the atomic to u64::MAX. fetch_update is the standard
    // compare-exchange-loop pattern; on prev==0 we skip the decrement
    // and log so a regression that introduced double-finalize would
    // show up. Today no double-finalize is provable from the code, so
    // this is defense in depth.
    let prev = inner
        .queue_depth
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
            if v > 0 { Some(v - 1) } else { None }
        });
    let new_depth = match prev {
        Ok(p) => p.saturating_sub(1),
        Err(_) => {
            log::debug!(
                "ExtractRuntime finalize_item: queue_depth was already 0 \
                 (double-finalize?); leaving at 0"
            );
            0
        }
    };

    // Emit per-item progress (Coalesce: latest-wins).
    let progress = Notification::ExtractProgress(ExtractProgress {
        service_generation: inner.service_generation,
        remaining: new_depth,
        indexed_in_session: inner.indexed_count.load(Ordering::Relaxed),
    });
    if let Err(e) = inner.notification_tx.send(progress).await {
        log::debug!("ExtractRuntime progress send failed: {e}");
    }

    // Emit ExtractCompleted when the queue drains AND nothing else
    // is mid-flight. Both checks required - see M11 fix above.
    if new_depth == 0 && in_flight_empty {
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
    let hash_for_check = work.content_hash;
    let existing = inner
        .read_db
        .with_conn(move |conn| {
            db::db::queries_extra::select_extracted_text_status(
                conn,
                &hash_for_check,
                i64::from(search::INDEX_SCHEMA_VERSION),
            )
        })
        .await
        .unwrap_or(None);
    if let Some(status) = existing
        && is_permanent_status(&status)
    {
        log::debug!(
            "ExtractRuntime pre-flight {} already resolved at status {status}",
            work.content_hash,
        );
        // Phase 7 (C3 fix): a permanent row already covers this hash.
        // Two side effects to keep the system honest:
        //
        // 1. UPDATE attachments.text_indexed_at for any rows that
        //    reference this content_hash with NULL. Without this the
        //    backfill SELECT (`find_unindexed_cached_attachments`,
        //    filtered on `text_indexed_at IS NULL`) keeps re-emitting
        //    the same rows on every kick - permanent skips churn
        //    forever and inflate `extract_status.skipped_total`.
        //
        // 2. When the permanent status is `indexed`, fan out a
        //    re-index for any newly-referencing messages. Without this
        //    a brand-new sync of a message that points at an
        //    already-extracted content_hash never gets attachment_text
        //    in its Tantivy doc - the worker short-circuits before
        //    `fan_out_reindex` and sync's thin doc never enriches via
        //    the extract path.
        let hash_for_update = work.content_hash;
        if let Err(e) = inner
            .db
            .with_conn(move |conn| {
                let now: i64 = chrono::Utc::now().timestamp();
                db::db::queries_extra::mark_attachment_text_indexed(conn, &hash_for_update, now)
            })
            .await
        {
            log::warn!(
                "ExtractRuntime pre-flight {} text_indexed_at update failed: {e}",
                work.content_hash,
            );
        }
        if status == "indexed" {
            fan_out_reindex(inner, &work.content_hash).await;
        }
        return ExtractionOutcome::AlreadyResolved { previous_status: status };
    }

    // Fetch metadata for this attachment (filename + mime), needed by
    // the dispatcher to canonicalize the mime.
    //
    // M5 fix: query by content_hash, not by the (account, message,
    // attachment_id) tuple. The specific work item's row may have been
    // deleted between enqueue and dequeue (account.delete, message
    // expire, sync purge); the same content_hash may still be
    // referenced by N other live attachments with valid filename + mime.
    // Pre-fix, a deleted-then-dequeued row produced None metadata,
    // which fell through to ("","") -> canonicalize_mime returns
    // Mime::Unknown -> permanent SkipReason::UnknownMime, poisoning
    // the content_hash for every other live attachment that shares it.
    // Picking ANY surviving row with non-empty filename keeps mime
    // dispatch deterministic and unblocks honest siblings.
    let hash_for_meta = work.content_hash;
    let meta = inner
        .read_db
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT filename, mime_type FROM attachments \
                     WHERE content_hash = ?1 AND filename IS NOT NULL AND filename != '' \
                     ORDER BY rowid LIMIT 1",
                )
                .map_err(|e| format!("prepare attachment meta: {e}"))?;
            let mut rows = stmt
                .query_map(
                    rusqlite::params![hash_for_meta],
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

    // Attachments roadmap Phase 3: route the byte read through
    // `materialize_blob` against PackStore. Tombstoned blobs return
    // None from `PackStore::get`, which the helper surfaces as
    // `ServiceError::Internal`; we translate that to a BytesGone
    // skip so the worker keeps draining instead of poisoning the
    // queue.
    let materialized = match crate::attachment_materialize::materialize_blob(
        &inner.boot_state,
        &work.content_hash,
    )
    .await
    {
        Ok(m) => m,
        Err(e) => {
            let msg = format!("materialize_blob {}: {e}", work.content_hash);
            log::debug!("{msg}");
            persist_outcome_row(
                inner,
                &work.content_hash,
                &mime_type,
                &ExtractionOutcome::Skipped { reason: SkipReason::BytesGone },
            )
            .await;
            return ExtractionOutcome::Skipped { reason: SkipReason::BytesGone };
        }
    };
    let bytes = match tokio::fs::read(&materialized.path).await {
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
            log::warn!("read {} failed: {e}", materialized.path.display());
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
    //
    // C3 fix: extended to fire for permanent skips too, not just
    // Indexed. A row with status `skipped:opaque` (or any other
    // permanent skip) is "decided" - re-extraction would produce the
    // same outcome. Leaving text_indexed_at NULL would make backfill
    // re-emit the row every kick, churning forever. Transient skips
    // (Timeout, BytesGone) and Failed leave it NULL so a future kick
    // retries.
    let should_mark_indexed = match &outcome {
        ExtractionOutcome::Indexed { .. } => true,
        ExtractionOutcome::Skipped { reason } => !reason.is_retry_eligible(),
        ExtractionOutcome::Failed { .. } => false,
        ExtractionOutcome::AlreadyResolved { .. } => false, // pre-flight handled
    };
    if should_mark_indexed {
        let hash = work.content_hash;
        let _ = inner
            .db
            .with_conn(move |conn| {
                let now: i64 = chrono::Utc::now().timestamp();
                db::db::queries_extra::mark_attachment_text_indexed(conn, &hash, now)
            })
            .await;
    }

    outcome
}

async fn persist_outcome_row(
    inner: &Arc<ExtractRuntimeInner>,
    content_hash: &db::blob_hash::BlobHash,
    mime_type: &str,
    outcome: &ExtractionOutcome,
) {
    let now: i64 = chrono::Utc::now().timestamp();
    let (status, text): (String, Option<String>) = match outcome {
        ExtractionOutcome::Indexed { text } => ("indexed".into(), Some(text.clone())),
        ExtractionOutcome::Skipped { reason } => (reason.status_string().to_string(), None),
        ExtractionOutcome::Failed { .. } => ("failed:transient".into(), None),
        ExtractionOutcome::AlreadyResolved { .. } => {
            // Pre-flight branch already verified the row exists; do
            // not rewrite it. Re-writing would update extracted_at
            // and confuse staleness sweeps that key on it.
            return;
        }
    };
    let hash = *content_hash;
    let mime = mime_type.to_string();
    let result = inner
        .db
        .with_conn(move |conn| {
            db::db::queries_extra::upsert_extracted_text_row(
                conn,
                &hash,
                &mime,
                text.as_deref(),
                &status,
                now,
                i64::from(search::INDEX_SCHEMA_VERSION),
            )
        })
        .await;
    if let Err(e) = result {
        log::warn!("persist_outcome_row {content_hash}: {e}");
    }
}

/// Maximum (account_id, message_id) pairs materialized into a single
/// `WriterCommand::Index` payload during `fan_out_reindex`. Mirrors
/// `rebuild::REBUILD_CHUNK_SIZE`. H2 fix: prior to chunking, a viral
/// content_hash referenced by N messages produced one writer command
/// carrying N * (subject + from + body_text + attachments) bytes - a
/// 1000-message viral attachment with 100 KB extracted_text per message
/// could push ~150 MB through a single mpsc command, blowing past the
/// writer's 64 MB heap budget. Chunking keeps each command bounded
/// (~30 MB worst case at 200 messages * 4 attachments * 100 KB) and
/// lets the bounded mpsc backpressure naturally between chunks.
const FANOUT_CHUNK_SIZE: usize = 200;

/// Phase 7-7: re-index every message that references the just-indexed
/// `content_hash`. Reads canonical DB state (messages + attachments +
/// extracted-text join) and the body store, builds a fresh
/// `SearchDocument` per message, and emits one `WriterCommand::Index`
/// per chunk of `FANOUT_CHUNK_SIZE` pairs.
///
/// Failures inside a chunk log a warning and proceed to the next
/// chunk: the `attachment_extracted_text` row + `text_indexed_at`
/// UPDATE are already committed, so a future enqueue of the same hash
/// (or an `extract.backfill_kick`) sees canonical state and re-emits.
async fn fan_out_reindex(inner: &Arc<ExtractRuntimeInner>, content_hash: &db::blob_hash::BlobHash) {
    let hash = *content_hash;
    let pairs_result = inner
        .read_db
        .with_conn(move |conn| {
            db::db::queries_extra::find_message_ids_referencing_content_hash(conn, &hash)
        })
        .await;
    let pairs = match pairs_result {
        Ok(p) if p.is_empty() => {
            log::debug!("fan_out_reindex {content_hash}: no messages reference this hash");
            return;
        }
        Ok(p) => p,
        Err(e) => {
            log::warn!("fan_out_reindex {content_hash}: find_message_ids: {e}");
            return;
        }
    };

    for chunk in pairs.chunks(FANOUT_CHUNK_SIZE) {
        if let Err(e) = fan_out_reindex_chunk(inner, content_hash, chunk).await {
            log::warn!("fan_out_reindex {content_hash}: chunk failed: {e}");
            // Continue to next chunk - partial fan-out beats no fan-out.
        }
    }
}

async fn fan_out_reindex_chunk(
    inner: &Arc<ExtractRuntimeInner>,
    content_hash: &db::blob_hash::BlobHash,
    pairs: &[(String, String)],
) -> Result<(), String> {
    let pairs_for_msgs = pairs.to_vec();
    let pairs_for_atts = pairs.to_vec();
    let messages_fut = inner.db.with_conn(move |conn| {
        db::db::queries_extra::select_messages_for_index_batch(conn, &pairs_for_msgs)
    });
    let attachments_fut = inner.db.with_read(move |conn| {
        db::db::queries_extra::select_attachment_fragments_batch(conn, &pairs_for_atts)
    });
    let message_ids: Vec<String> = pairs.iter().map(|(_, m)| m.clone()).collect();
    let bodies_fut = inner.body_read.get_batch(message_ids);

    let (messages, mut fragments, bodies) =
        match tokio::join!(messages_fut, attachments_fut, bodies_fut) {
            (Ok(m), Ok(a), Ok(b)) => (m, a, b),
            (m, a, b) => {
                return Err(format!(
                    "query failure (messages: {:?}, attachments: {:?}, bodies: {:?})",
                    m.as_ref().err(),
                    a.as_ref().err(),
                    b.as_ref().err(),
                ));
            }
        };

    // Index the bodies by message_id for cheap lookup.
    let mut body_by_mid: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::with_capacity(bodies.len());
    for b in bodies {
        body_by_mid.insert(b.message_id, b.body_text);
    }

    let mut docs: Vec<SearchDocument> = Vec::with_capacity(messages.len());
    for m in messages {
        let key = (m.account_id.clone(), m.message_id.clone());
        let attachment_rows = fragments.remove(&key).unwrap_or_default();
        let has_attachment = !attachment_rows.is_empty();
        let attachments: Vec<AttachmentDocFragment> = attachment_rows
            .into_iter()
            .map(|r| AttachmentDocFragment {
                attachment_id:  r.attachment_id,
                filename:       r.filename,
                mime:           r.mime_type,
                extracted_text: r.extracted_text,
            })
            .collect();
        // L10 fix: distinguish "row absent in body store" (skip the
        // doc - sync race, body hasn't committed yet, or store is in
        // an inconsistent state) from "row present with body_text =
        // None" (legitimate empty body). Pre-fix, both cases collapsed
        // to None and the writer applied an empty-body doc, briefly
        // dropping body_text from Tantivy until the next sync re-emit.
        let body_text = match body_by_mid.remove(&m.message_id) {
            Some(text) => text,
            None => {
                log::debug!(
                    "fan_out_reindex_chunk: body store row missing for {} \
                     (sync race or absent commit); skipping doc, next \
                     sync emit will re-add",
                    m.message_id,
                );
                continue;
            }
        };
        docs.push(SearchDocument {
            message_id: m.message_id,
            account_id: m.account_id,
            thread_id: m.thread_id,
            subject: m.subject,
            from_name: m.from_name,
            from_address: m.from_address,
            to_addresses: m.to_addresses,
            body_text,
            snippet: m.snippet,
            date: m.date,
            is_read: m.is_read,
            is_starred: m.is_starred,
            has_attachment,
            attachments,
        });
    }

    if docs.is_empty() {
        log::debug!("fan_out_reindex {content_hash}: chunk resolved but no messages found");
        return Ok(());
    }
    inner
        .search_write
        .index_messages_batch(docs)
        .await
        .map_err(|e| format!("index_messages_batch: {e}"))
}

/// Status strings that signal "do not retry on next enqueue."
/// L1 fix: delegates to `text_extract::is_retry_eligible_status_str` so
/// the partition lives in one place. Indexed is permanent (success +
/// fan-out emitted); transient failures and retry-eligible skips are
/// not.
fn is_permanent_status(status: &str) -> bool {
    !crate::text_extract::is_retry_eligible_status_str(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_state::WriterCommand;
    use tempfile::TempDir;

    /// Build a `BodyStoreReadState` against a fresh tempdir; returns the
    /// dir guard so the caller can keep it alive for the lifetime of
    /// the test.
    fn body_read_in_tempdir() -> (BodyStoreReadState, TempDir) {
        let tmp = TempDir::new().expect("tempdir");
        let body_read = BodyStoreReadState::init(tmp.path()).expect("body store init");
        (body_read, tmp)
    }

    /// Build an unused search-write handle whose receiver is held by
    /// the caller. Tests that don't exercise the writer drop the
    /// receiver immediately; tests that do keep it.
    fn dummy_search_write() -> (SearchWriteHandle, mpsc::Receiver<WriterCommand>) {
        let (tx, rx) = mpsc::channel::<WriterCommand>(8);
        (SearchWriteHandle::from_sender(tx), rx)
    }

    fn db_states_for_test() -> (WriteDbState, db::db::ReadDbState, TempDir) {
        let tmp = TempDir::new().expect("temp dir");
        let write = WriteDbState::from_pool(
            db::db::open_writer_pool(tmp.path()).expect("open writer pool"),
        );
        let read = db::db::open_reader_pool(tmp.path()).expect("open reader pool");
        (write, read, tmp)
    }

    /// Build a bare `BootSharedState` for tests that exercise the
    /// worker but don't need a PackStore installed. ExtractRuntime
    /// reads `boot_state.pack_store()` lazily; missing pack store is
    /// surfaced to materialize_blob as a `BytesGone`-shaped skip.
    fn dummy_boot_state() -> Arc<crate::boot::BootSharedState> {
        crate::boot::BootSharedState::new(
            std::path::PathBuf::from("."),
            crate::dispatch::DispatchConfig::default(),
        )
    }

    #[tokio::test]
    async fn enqueue_after_shutdown_returns_err() {
        // Build a runtime with a dummy DB - we only need the lifecycle
        // shape to be correct; no actual extraction runs because we
        // shut down before processing.
        let (db, read_db, _db_dir) = db_states_for_test();
        let (tx, _rx) = mpsc::channel::<Vec<u8>>(8);
        let notification_tx = NotificationSender::new(tx);
        let (search_write, _search_rx) = dummy_search_write();
        let (body_read, _body_dir) = body_read_in_tempdir();
        let runtime = ExtractRuntime::new(
            db,
            read_db,
            std::path::PathBuf::from("."),
            dummy_boot_state(),
            search_write,
            body_read,
            notification_tx,
            0,
            CancellationToken::new(),
        );
        runtime.shutdown().await;
        let result = runtime
            .enqueue(ExtractWork {
                content_hash: db::blob_hash::BlobHash::hash(b"abc"),
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
        let (db, read_db, _db_dir) = db_states_for_test();
        let (tx, _rx) = mpsc::channel::<Vec<u8>>(8);
        let notification_tx = NotificationSender::new(tx);
        let (search_write, _search_rx) = dummy_search_write();
        let (body_read, _body_dir) = body_read_in_tempdir();
        let runtime = ExtractRuntime::new(
            db,
            read_db,
            std::path::PathBuf::from("."),
            dummy_boot_state(),
            search_write,
            body_read,
            notification_tx,
            0,
            CancellationToken::new(),
        );

        // Pre-load the dedupe set so the second enqueue is dedupe'd.
        let hash_abc = db::blob_hash::BlobHash::hash(b"abc");
        runtime.inner.in_flight_hashes.lock().await.insert(hash_abc);
        let work = ExtractWork {
            content_hash: hash_abc,
            account_id: "acc".into(),
            message_id: "msg".into(),
            attachment_id: "att".into(),
        };
        let result = runtime.enqueue(work).await;
        assert!(result.is_ok(), "dedupe path should be Ok no-op");
        assert_eq!(runtime.status_snapshot().0, 0, "queue_depth should not increment on dedupe");
        runtime.shutdown().await;
    }

    /// Phase 7-7: build a runtime whose DB has one message + one
    /// attachment + one indexed extracted-text row, invoke
    /// `fan_out_reindex` directly, and assert the resulting
    /// `WriterCommand::Index` carries the doc with the attachment's
    /// extracted text inlined.
    #[tokio::test]
    async fn phase_7_7_indexed_outcome_emits_writer_command_index() {
        let (db, read_db, _db_dir) = db_states_for_test();
        let hash_a = db::blob_hash::BlobHash::hash(b"A");
        db.with_conn_sync(move |conn| {
            conn.execute(
                "INSERT INTO accounts (id, email, provider, is_active) \
                 VALUES ('acc1', 'acc1@example.com', 'gmail_api', 1)",
                [],
            )
            .map_err(|e| format!("insert account: {e}"))?;
            conn.execute(
                "INSERT INTO threads (account_id, id, subject, snippet, last_message_at) \
                 VALUES ('acc1', 'thr1', 'Quarterly report', 'snip', 1700000000)",
                [],
            )
            .map_err(|e| format!("insert thread: {e}"))?;
            conn.execute(
                "INSERT INTO messages (id, account_id, thread_id, subject, from_name,\
                    from_address, to_addresses, snippet, date, is_read, is_starred) \
                 VALUES ('msg1', 'acc1', 'thr1', 'Quarterly report', 'Alice', \
                         'a@example.com', 'b@example.com', 'snip', 1700000000, 0, 1)",
                [],
            )
            .map_err(|e| format!("insert msg: {e}"))?;
            conn.execute(
                "INSERT INTO attachments (id, message_id, account_id, filename, mime_type, content_hash) \
                 VALUES ('att1', 'msg1', 'acc1', 'report.pdf', 'application/pdf', ?1)",
                rusqlite::params![hash_a],
            )
            .map_err(|e| format!("insert att: {e}"))?;
            conn.execute(
                "INSERT INTO attachment_extracted_text \
                 (content_hash, mime_type, extracted_text, status, extracted_at, schema_version) \
                 VALUES (?1, 'application/pdf', 'pdf full text body', 'indexed', 1, 2)",
                rusqlite::params![hash_a],
            )
            .map_err(|e| format!("insert ext: {e}"))?;
            Ok(())
        })
        .expect("seed db");

        let (tx, _rx) = mpsc::channel::<Vec<u8>>(8);
        let notification_tx = NotificationSender::new(tx);
        let (search_write, mut search_rx) = dummy_search_write();
        let (body_read, _body_dir) = body_read_in_tempdir();
        // L10 fix: fan_out_reindex_chunk now skips docs whose body
        // store row is absent (treats absence as a sync race rather
        // than legit empty body). Populate the body store so the test
        // mirrors the production invariant - sync writes body BEFORE
        // emitting Index.
        body_read
            .put(
                "msg1".to_string(),
                None,
                Some("message body text".to_string()),
            )
            .await
            .expect("put body");
        let runtime = ExtractRuntime::new(
            db,
            read_db,
            std::path::PathBuf::from("."),
            dummy_boot_state(),
            search_write,
            body_read,
            notification_tx,
            0,
            CancellationToken::new(),
        );

        // `fan_out_reindex` sends a command and awaits its oneshot ack
        // before returning, so we must drive it concurrently with the
        // receiver that supplies the ack.
        let inner_for_task = Arc::clone(&runtime.inner);
        let extract_task = tokio::spawn(async move {
            fan_out_reindex(&inner_for_task, &hash_a).await;
        });

        let cmd = search_rx.recv().await.expect("writer command");
        match cmd {
            WriterCommand::Index { docs, ack } => {
                let _ = ack.send(Ok(()));
                assert_eq!(docs.len(), 1, "one doc per referenced message");
                let doc = &docs[0];
                assert_eq!(doc.message_id, "msg1");
                assert_eq!(doc.account_id, "acc1");
                assert_eq!(doc.subject.as_deref(), Some("Quarterly report"));
                assert!(doc.is_starred);
                assert!(doc.has_attachment);
                assert_eq!(doc.attachments.len(), 1);
                let att = &doc.attachments[0];
                assert_eq!(att.attachment_id, "att1");
                assert_eq!(att.filename, "report.pdf");
                assert_eq!(att.mime, "application/pdf");
                assert_eq!(att.extracted_text, "pdf full text body");
            }
            WriterCommand::Delete { .. } => panic!("expected Index, got Delete"),
            WriterCommand::Clear { .. } => panic!("expected Index, got Clear"),
            WriterCommand::FlushNow { .. } => panic!("expected Index, got FlushNow"),
        }
        extract_task.await.expect("fan_out_reindex task panicked");
        runtime.shutdown().await;
    }

    /// Phase 7-7: a content_hash that no message references is a
    /// logged no-op; no `WriterCommand` is sent.
    #[tokio::test]
    async fn phase_7_7_empty_fan_out_is_no_op() {
        let (db, read_db, _db_dir) = db_states_for_test();
        let (tx, _rx) = mpsc::channel::<Vec<u8>>(8);
        let notification_tx = NotificationSender::new(tx);
        let (search_write, mut search_rx) = dummy_search_write();
        let (body_read, _body_dir) = body_read_in_tempdir();
        let runtime = ExtractRuntime::new(
            db,
            read_db,
            std::path::PathBuf::from("."),
            dummy_boot_state(),
            search_write,
            body_read,
            notification_tx,
            0,
            CancellationToken::new(),
        );

        fan_out_reindex(&runtime.inner, &db::blob_hash::BlobHash::hash(b"no-such-hash")).await;

        // The worker task holds an Arc<inner> that keeps the
        // SearchWriteHandle alive, so `recv()` never returns None.
        // Use a short timeout instead and assert nothing arrived.
        let timed = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            search_rx.recv(),
        )
        .await;
        assert!(
            timed.is_err(),
            "no command should be sent for an empty fan-out (got {:?})",
            timed.ok().flatten().map(|_| "WriterCommand"),
        );
        runtime.shutdown().await;
    }
}
