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

use db::db::ReadDbState;
use tantivy::{IndexWriter, Term};
use tokio::sync::mpsc;

use search::{AttachmentDocFragment, Fields, build_search_doc, open_or_create_search_index};
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
    db_read: ReadDbState,
    notification_tx: NotificationSender,
    service_generation: u32,
) -> Result<(SearchWriteHandle, tokio::task::JoinHandle<()>), String> {
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
    // Phase 4 review-pass fix: capture and return the writer task's
    // JoinHandle so the consolidated drain can await it after every
    // SearchWriteHandle clone has been dropped. Pre-Phase-4 the handle
    // was discarded; the task exited "by accident" because every
    // run_sync exit path called flush_now() (which round-trips through
    // an oneshot ack), so by SyncRuntime::shutdown return the writer
    // was queued-empty. Undocumented invariant; one stray future
    // change to a sync exit path that skips flush_now would re-open a
    // sentinel-before-flush race with no test catching it. Now the
    // drain explicitly observes the writer's exit.
    let task_handle = tokio::spawn(run_writer_task(
        writer,
        fields,
        rx,
        db_read,
        notification_tx,
        service_generation,
    ));
    Ok((SearchWriteHandle::from_sender(tx), task_handle))
}

/// Runner body. Owns the `IndexWriter` and `Fields`; processes
/// `WriterCommand`s sequentially; commits on cadence triggers.
async fn run_writer_task(
    mut writer: IndexWriter,
    fields: Fields,
    mut rx: mpsc::Receiver<WriterCommand>,
    db_read: ReadDbState,
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
                        &db_read,
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
    db_read: &ReadDbState,
    notification_tx: &NotificationSender,
    service_generation: u32,
) {
    match cmd {
        WriterCommand::Index { mut docs, ack } => {
            // Phase 7 (C2 fix): writer-side DB enrichment for thin
            // docs. Provider sync emits `attachments: Vec::new()`; if
            // we trusted that as authoritative, the sync re-emit of a
            // message whose attachments were already extracted would
            // wipe `attachment_text` from Tantivy (last-writer-wins on
            // delete_term + add_document). Enrich at apply time so
            // both producers (sync's thin doc + ExtractRuntime's
            // populated doc) converge to the same canonical shape.
            //
            // Trigger: `has_attachment && attachments.is_empty()`. A
            // populated `attachments` field is trusted (ExtractRuntime
            // already joined the same DB tables to build it).
            // `has_attachment == false` means the message has no
            // attachment rows; no DB read needed.
            enrich_thin_docs(&mut docs, db_read).await;
            // Phase 7 (M6 fix): drop docs whose message_id is no
            // longer in the canonical messages table at apply time.
            // Closes the index-after-delete race: sync emits a Delete
            // command, then extract's fan_out_reindex emits an Index
            // command for the same message_id; without this filter
            // the writer would re-create the just-deleted doc via
            // delete_term + add_document. One batched SELECT per
            // Index command, no per-doc lookup.
            filter_deleted_messages(&mut docs, db_read).await;
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

/// Drop docs whose `message_id` no longer exists in the `messages`
/// table at apply time. M6 fix: Sync's Delete and Extract's Index can
/// race for the same message_id (Sync deletes the message + emits
/// Delete; Extract's fan_out_reindex took its DB snapshot before the
/// delete and emits Index). Writer FIFO doesn't enforce DB-vs-Index
/// ordering across runtimes, so a stale Index after a fresh Delete
/// would resurrect the doc via delete_term (no-op) + add_document.
/// Single batched SELECT message_id FROM messages WHERE message_id IN
/// (...) - any doc whose id isn't in the result set is dropped.
///
/// On DB-read failure we keep all docs (degraded but not broken):
/// false-negative Delete handling is preferable to false-positive doc
/// drops.
async fn filter_deleted_messages(docs: &mut Vec<search::SearchDocument>, db_read: &ReadDbState) {
    if docs.is_empty() {
        return;
    }
    let ids: Vec<String> = docs.iter().map(|d| d.message_id.clone()).collect();
    let result = db_read
        .with_conn(move |conn| {
            let mut surviving: std::collections::HashSet<String> =
                std::collections::HashSet::with_capacity(ids.len());
            for chunk in ids.chunks(500) {
                let placeholders: String = chunk
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!("SELECT id FROM messages WHERE id IN ({placeholders})");
                let mut stmt = conn
                    .prepare(&sql)
                    .map_err(|e| format!("prepare filter_deleted: {e}"))?;
                let param_values: Vec<Box<dyn rusqlite::types::ToSql>> = chunk
                    .iter()
                    .map(|id| Box::new(id.clone()) as Box<dyn rusqlite::types::ToSql>)
                    .collect();
                let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                    param_values.iter().map(AsRef::as_ref).collect();
                let rows = stmt
                    .query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))
                    .map_err(|e| format!("query filter_deleted: {e}"))?;
                for row in rows {
                    surviving.insert(row.map_err(|e| format!("row filter_deleted: {e}"))?);
                }
            }
            Ok::<_, String>(surviving)
        })
        .await;
    let surviving = match result {
        Ok(s) => s,
        Err(e) => {
            log::warn!(
                "search_writer filter_deleted_messages: SELECT failed: {e} \
                 (proceeding with all docs; index-after-delete race window briefly open)"
            );
            return;
        }
    };
    let before = docs.len();
    docs.retain(|d| surviving.contains(&d.message_id));
    let dropped = before - docs.len();
    if dropped > 0 {
        log::debug!(
            "search_writer filter_deleted_messages: dropped {dropped} docs whose message_id \
             no longer exists in `messages` (Index-after-Delete race)"
        );
    }
}

/// Populate `doc.attachments` from DB for any doc where
/// `has_attachment && attachments.is_empty()` (sync's thin-doc shape).
/// Docs that already carry attachments are trusted - ExtractRuntime's
/// `fan_out_reindex` builds them from the same DB tables.
async fn enrich_thin_docs(docs: &mut [search::SearchDocument], db_read: &ReadDbState) {
    let pairs: Vec<(String, String)> = docs
        .iter()
        .filter(|d| d.has_attachment && d.attachments.is_empty())
        .map(|d| (d.account_id.clone(), d.message_id.clone()))
        .collect();
    if pairs.is_empty() {
        return;
    }
    let pairs_for_query = pairs.clone();
    let result = db_read
        .with_conn(move |conn| {
            db::db::queries_extra::select_attachment_fragments_batch(conn, &pairs_for_query)
        })
        .await;
    let mut fragments = match result {
        Ok(f) => f,
        Err(e) => {
            log::warn!(
                "search_writer enrich_thin_docs: select_attachment_fragments_batch failed: {e} \
                 (proceeding with thin docs; attachment search will be stale until next emit)"
            );
            return;
        }
    };
    for doc in docs.iter_mut() {
        if !(doc.has_attachment && doc.attachments.is_empty()) {
            continue;
        }
        let key = (doc.account_id.clone(), doc.message_id.clone());
        let rows = fragments.remove(&key).unwrap_or_default();
        doc.attachments = rows
            .into_iter()
            .map(|r| AttachmentDocFragment {
                attachment_id:  r.attachment_id,
                filename:       r.filename,
                mime:           r.mime_type,
                extracted_text: r.extracted_text,
            })
            .collect();
    }
}
