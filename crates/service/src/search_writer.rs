//! Service-side Tantivy writer task.
//!
//! Phase 3 of `docs/service/phase-3-plan.md` relocates Tantivy writer
//! ownership into a Service-internal task. The task body lives here
//! (alongside `NotificationSender` from `service::boot_progress`); the
//! public handle (`service-state::SearchWriteHandle`) is the cheap
//! `mpsc::Sender` consumers see.
//!
//! ## Why a dedicated task
//!
//! Three reasons (per `phase-3-plan.md` § "Search writer task"):
//!
//! 1. **Tantivy parallelises adds internally.** Wrapping `IndexWriter`
//!    in `Arc<Mutex<_>>` would serialise adds across concurrent
//!    producers - a Phase 5 multi-account scaling cliff.
//! 2. **Cadence policy belongs in one place.** "Every caller commits"
//!    forces repetition; the task owns the size + time + `FlushNow`
//!    triggers and emits one `IndexCommitted` notification per commit.
//! 3. **Backpressure works correctly.** A bounded mpsc fills under
//!    sustained pressure; `index_messages_batch` parks on
//!    `send.await`; sync slows naturally rather than the writer
//!    falling behind silently.
//!
//! ## Cadence
//!
//! Commit when:
//! - `pending_docs >= COMMIT_DOC_THRESHOLD` (1000 docs).
//! - `Instant::now() - first_uncommitted >= COMMIT_TIME_THRESHOLD` (2 s).
//! - A `FlushNow` command arrives.
//! - A `Clear` command arrives (clear is a forced commit by definition).
//! - The mpsc sender count drops to zero (drain path; commit any
//!   straggler docs from runners that finished mid-flight).
//!
//! ## Notifications
//!
//! After every successful `commit()` the task awaits a
//! `Notification::IndexCommitted` send via `send_timeout(30 s, …)`.
//! On timeout: log a warning and drop the notification. The signal is
//! advisory (the next commit will fire another); a wedged consumer
//! must not park the writer indefinitely. See `phase-3-plan.md` H5.
//!
//! ## Runtime flavor
//!
//! `tokio::task::block_in_place` panics on a `current_thread` runtime,
//! and the synchronous `IndexWriter::commit` / `add_document` calls
//! happen inside `block_in_place`. The Service's main runtime is
//! constructed `multi_thread`; `spawn` asserts this at construction.

use std::path::Path;
use std::time::{Duration, Instant};

use tantivy::{IndexWriter, Term};
use tokio::sync::mpsc;

use search::{Fields, build_search_doc, open_or_create_search_index};
use service_api::{IndexCommitted, Notification};
use service_state::search_write::{
    SearchWriteHandle, WriterCommand,
    cadence::{
        COMMAND_QUEUE_CAPACITY, COMMIT_DOC_THRESHOLD, COMMIT_TIME_THRESHOLD,
        INDEX_COMMITTED_SEND_TIMEOUT,
    },
};

use crate::boot_progress::NotificationSender;

/// Heap budget for the Tantivy writer. 64 MB is the same value the
/// pre-Phase-3 unified `SearchState::init` used.
const WRITER_HEAP_BYTES: usize = 64 * 1024 * 1024;

/// Construct + spawn the Service-side search writer task.
///
/// Asserts the current tokio runtime is multi-threaded (the writer
/// needs `block_in_place`); returns `Err` otherwise.
///
/// `app_data_dir` is the same directory the UI's `SearchReadState`
/// will open later in the boot sequence. The boot ordering contract
/// (Phase 3 task 12): the Service spawns this task in
/// `BootPhase::OpeningSearchIndex` *before* `boot.ready`, so the
/// directory + initial empty segment exist by the time the UI tries
/// to construct its reader.
///
/// `notification_tx` is the Service's notification queue (the
/// generation-tagged sender from `boot_progress`); the task captures
/// it and the writer's static `service_generation` to emit
/// `Notification::IndexCommitted` after each commit.
pub fn spawn(
    app_data_dir: &Path,
    notification_tx: NotificationSender,
    service_generation: u32,
) -> Result<SearchWriteHandle, String> {
    let handle = tokio::runtime::Handle::try_current()
        .map_err(|_| "search writer requires a tokio runtime".to_string())?;
    if !matches!(
        handle.runtime_flavor(),
        tokio::runtime::RuntimeFlavor::MultiThread
    ) {
        return Err("search writer requires multi-threaded tokio runtime".to_string());
    }

    let (index, schema) = open_or_create_search_index(app_data_dir)?;
    let writer: IndexWriter = index
        .writer(WRITER_HEAP_BYTES)
        .map_err(|e| format!("create search writer: {e}"))?;
    let fields = Fields::from_schema(&schema);
    drop(index);

    let (tx, rx) = mpsc::channel::<WriterCommand>(COMMAND_QUEUE_CAPACITY);
    tokio::spawn(run_writer_task(
        writer,
        fields,
        rx,
        notification_tx,
        service_generation,
    ));
    Ok(SearchWriteHandle::from_sender(tx))
}

/// Runner body. Owns the `IndexWriter` and `Fields`; processes
/// `WriterCommand`s sequentially; commits on cadence triggers.
async fn run_writer_task(
    mut writer: IndexWriter,
    fields: Fields,
    mut rx: mpsc::Receiver<WriterCommand>,
    notification_tx: NotificationSender,
    service_generation: u32,
) {
    let mut pending_docs: u64 = 0;
    let mut first_uncommitted: Option<Instant> = None;

    loop {
        // Sleep deadline: COMMIT_TIME_THRESHOLD past the first
        // uncommitted doc, or far in the future if nothing is queued.
        // The "1 hour" sentinel just keeps the select! arm valid; the
        // recv branch dominates when the queue is idle.
        let deadline = first_uncommitted
            .map(|t| t + COMMIT_TIME_THRESHOLD)
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(3600));

        tokio::select! {
            cmd = rx.recv() => match cmd {
                Some(c) => {
                    apply_command(
                        &mut writer,
                        &fields,
                        c,
                        &mut pending_docs,
                        &mut first_uncommitted,
                        &notification_tx,
                        service_generation,
                    )
                    .await;
                }
                None => {
                    // All senders dropped (drain path). Commit any
                    // straggler docs and exit.
                    if pending_docs > 0 {
                        let _ = commit_and_notify(
                            &mut writer,
                            &notification_tx,
                            service_generation,
                        )
                        .await;
                    }
                    log::info!("Search writer task exiting cleanly");
                    return;
                }
            },
            () = tokio::time::sleep_until(deadline.into()) => {
                if pending_docs > 0 {
                    if let Err(e) = commit_and_notify(
                        &mut writer,
                        &notification_tx,
                        service_generation,
                    )
                    .await
                    {
                        log::warn!("Time-triggered commit failed: {e}");
                    }
                    pending_docs = 0;
                    first_uncommitted = None;
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn apply_command(
    writer: &mut IndexWriter,
    fields: &Fields,
    cmd: WriterCommand,
    pending_docs: &mut u64,
    first_uncommitted: &mut Option<Instant>,
    notification_tx: &NotificationSender,
    service_generation: u32,
) {
    match cmd {
        WriterCommand::Index { docs, ack } => {
            let result = tokio::task::block_in_place(|| {
                for doc in &docs {
                    let tantivy_doc = build_search_doc(fields, doc);
                    // Replace any existing doc for this message_id by
                    // first deleting then adding.
                    writer.delete_term(Term::from_field_text(fields.message_id, &doc.message_id));
                    writer
                        .add_document(tantivy_doc)
                        .map_err(|e| format!("add document: {e}"))?;
                }
                Ok::<(), String>(())
            });
            #[allow(clippy::cast_possible_truncation)]
            if result.is_ok() {
                *pending_docs += docs.len() as u64;
                if first_uncommitted.is_none() {
                    *first_uncommitted = Some(Instant::now());
                }
            }
            let _ = ack.send(result);
        }
        WriterCommand::Delete { ids, ack } => {
            let result = tokio::task::block_in_place(|| {
                for id in &ids {
                    writer.delete_term(Term::from_field_text(fields.message_id, id));
                }
                Ok::<(), String>(())
            });
            #[allow(clippy::cast_possible_truncation)]
            if result.is_ok() {
                *pending_docs += ids.len() as u64;
                if first_uncommitted.is_none() {
                    *first_uncommitted = Some(Instant::now());
                }
            }
            let _ = ack.send(result);
        }
        WriterCommand::Clear { ack } => {
            let result = tokio::task::block_in_place(|| {
                writer
                    .delete_all_documents()
                    .map_err(|e| format!("clear index: {e}"))?;
                Ok::<(), String>(())
            });
            if result.is_ok() {
                let commit_result =
                    commit_and_notify(writer, notification_tx, service_generation).await;
                let final_result = result.and(commit_result);
                *pending_docs = 0;
                *first_uncommitted = None;
                let _ = ack.send(final_result);
            } else {
                let _ = ack.send(result);
            }
            return;
        }
        WriterCommand::FlushNow { ack } => {
            let commit_result =
                commit_and_notify(writer, notification_tx, service_generation).await;
            *pending_docs = 0;
            *first_uncommitted = None;
            let _ = ack.send(commit_result);
            return;
        }
    }

    // Size-triggered commit, applied uniformly after every Index/Delete.
    if *pending_docs >= COMMIT_DOC_THRESHOLD {
        if let Err(e) = commit_and_notify(writer, notification_tx, service_generation).await {
            log::warn!("Size-triggered commit failed: {e}");
        }
        *pending_docs = 0;
        *first_uncommitted = None;
    }
}

async fn commit_and_notify(
    writer: &mut IndexWriter,
    notification_tx: &NotificationSender,
    service_generation: u32,
) -> Result<(), String> {
    let commit_result =
        tokio::task::block_in_place(|| writer.commit().map_err(|e| format!("commit: {e}")));
    if commit_result.is_ok() {
        let notif = Notification::IndexCommitted(IndexCommitted { service_generation });
        match tokio::time::timeout(INDEX_COMMITTED_SEND_TIMEOUT, notification_tx.send(notif)).await
        {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                log::warn!(
                    "IndexCommitted send: notification queue closed (UI is probably gone)"
                );
            }
            Err(_) => {
                log::warn!(
                    "IndexCommitted send timed out after {} s; UI consumer wedged. Dropping; the next IndexCommitted will catch up.",
                    INDEX_COMMITTED_SEND_TIMEOUT.as_secs(),
                );
            }
        }
    }
    commit_result.map(|_| ())
}
