//! Phase 7-9: `index.rebuild` Wipe path.
//!
//! `run_wipe_rebuild` is the body of the spawned task that
//! `handle_rebuild` registers on `BootSharedState`. The task:
//!
//! 1. Sends `WriterCommand::Clear` to truncate the Tantivy index.
//! 2. Resets the per-attachment extraction state in DB
//!    (`attachments.text_indexed_at = NULL`, truncate
//!    `attachment_extracted_text`).
//! 3. Iterates every message in fixed-size chunks and re-emits a
//!    `WriterCommand::Index` for the chunk. The full doc shape comes
//!    from the same DB queries 7-7's worker uses, plus the body
//!    store for `body_text`. Attachment fragments are read from DB
//!    at apply-time so a freshly-Clear'd index gets the canonical
//!    state.
//! 4. Emits `IndexRebuildProgress` per chunk (Coalesce by
//!    rebuild_id) and `IndexRebuildCompleted` at the end (MustDeliver).
//! 5. Triggers `extract.backfill_kick` so attachment text re-extracts
//!    against the now-empty index.
//!
//! Cancellation: a `CancellationToken::cancelled()` future is checked
//! between chunks. On cancel the task exits without emitting
//! `IndexRebuildCompleted` - the next boot will see partial-index
//! state and the user / palette can re-trigger.
//!
//! Drain: `dispatch::run_shutdown_drain` calls
//! `boot_state.take_rebuild_task()`, cancels the token, awaits the
//! handle. The same chunk-boundary cancellation check applies.

use std::sync::Arc;

use search::{AttachmentDocFragment, SearchDocument};
use service_api::{IndexRebuildCompleted, IndexRebuildProgress, Notification};
use service_state::SearchWriteHandle;
use store::body_store::BodyStoreReadState;
use tokio_util::sync::CancellationToken;

use crate::boot::BootSharedState;
use crate::boot_progress::NotificationSender;

/// Chunk size for the message-iteration step. Each chunk drives one
/// DB SELECT for messages + attachments and one
/// `index_messages_batch` send. Sized so the per-chunk payload stays
/// well under the 8 MB mpsc-batch ceiling the writer task expects.
const REBUILD_CHUNK_SIZE: usize = 200;

/// Wipe-path rebuild. Drives steps 1-5 above to completion (or to the
/// nearest chunk boundary on cancel).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_wipe_rebuild(
    boot_state: Arc<BootSharedState>,
    rebuild_id: String,
    cancel: CancellationToken,
    db: service_state::WriteDbState,
    search_write: SearchWriteHandle,
    body_read: BodyStoreReadState,
    notification_tx: NotificationSender,
    service_generation: u32,
) {
    let outcome = run_wipe_rebuild_inner(
        rebuild_id.clone(),
        cancel.clone(),
        db,
        search_write,
        body_read,
        notification_tx.clone(),
        service_generation,
    )
    .await;
    match outcome {
        Ok(()) => {
            log::info!("rebuild {rebuild_id}: completed");
            // C4 fix: record this rebuild_id as the last successfully-
            // completed rebuild. The schema-version dispatcher gates
            // its `.version` write on observing this matches the
            // rebuild_id it dispatched - so cancellation / drain /
            // error all leave the OLD `.version` on disk and the next
            // boot re-fires.
            boot_state.mark_rebuild_completed(rebuild_id.clone());
            // Trigger backfill so attachment text re-extracts against
            // the freshly-cleared index. Defensive against the kick
            // failing - the next hourly UI tick will retry.
            if let Err(e) = crate::handlers::extract::handle_backfill_kick(&boot_state).await {
                log::warn!("rebuild {rebuild_id}: post-rebuild backfill kick failed: {e}");
            }
        }
        Err(e) => {
            log::warn!("rebuild {rebuild_id}: aborted: {e}");
        }
    }
    // The slot is also consumed by drain (cancel + await); on a
    // graceful completion, clear the slot so a subsequent rebuild
    // request sees no in-flight rebuild.
    let _ = boot_state.take_rebuild_task();
}

/// PreserveExisting rebuild. Builds a staging index while the active
/// reader stays live, mirrors concurrent writes into that staging
/// writer, then flips the active-index pointer and routes future
/// writes to the rebuilt writer.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_preserve_existing_rebuild(
    boot_state: Arc<BootSharedState>,
    rebuild_id: String,
    cancel: CancellationToken,
    db: service_state::WriteDbState,
    live_search_write: SearchWriteHandle,
    body_read: BodyStoreReadState,
    notification_tx: NotificationSender,
    service_generation: u32,
) {
    let outcome = run_preserve_existing_rebuild_inner(
        Arc::clone(&boot_state),
        rebuild_id.clone(),
        cancel.clone(),
        db,
        live_search_write,
        body_read,
        notification_tx,
        service_generation,
    )
    .await;
    match outcome {
        Ok(()) => {
            log::info!("rebuild {rebuild_id}: preserve-existing completed");
            boot_state.mark_rebuild_completed(rebuild_id.clone());
        }
        Err(e) => {
            log::warn!("rebuild {rebuild_id}: preserve-existing aborted: {e}");
        }
    }
    let _ = boot_state.take_rebuild_task();
}

async fn run_wipe_rebuild_inner(
    rebuild_id: String,
    cancel: CancellationToken,
    db: service_state::WriteDbState,
    search_write: SearchWriteHandle,
    body_read: BodyStoreReadState,
    notification_tx: NotificationSender,
    service_generation: u32,
) -> Result<(), String> {
    if cancel.is_cancelled() {
        return Err("cancelled before start".into());
    }
    log::info!("rebuild {rebuild_id}: starting wipe");

    // Step 1: clear the Tantivy index.
    search_write
        .clear_index()
        .await
        .map_err(|e| format!("WriterCommand::Clear: {e}"))?;

    // Step 2: reset extraction state in DB.
    db.with_conn(db::db::queries_extra::reset_extracted_text_for_rebuild)
        .await
        .map_err(|e| format!("reset_extracted_text_for_rebuild: {e}"))?;

    if cancel.is_cancelled() {
        return Err("cancelled after clear".into());
    }

    // Step 3: enumerate all message identities.
    rebuild_all_messages(
        &rebuild_id,
        &cancel,
        &db,
        &search_write,
        &body_read,
        &notification_tx,
        service_generation,
    )
    .await?;

    emit_rebuild_completed(&rebuild_id, &notification_tx, service_generation).await;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_preserve_existing_rebuild_inner(
    boot_state: Arc<BootSharedState>,
    rebuild_id: String,
    cancel: CancellationToken,
    db: service_state::WriteDbState,
    live_search_write: SearchWriteHandle,
    body_read: BodyStoreReadState,
    notification_tx: NotificationSender,
    service_generation: u32,
) -> Result<(), String> {
    if cancel.is_cancelled() {
        return Err("cancelled before start".into());
    }
    let app_data_dir = boot_state.app_data_dir().to_path_buf();
    let staging_dir = search::staging_search_index_dir(&app_data_dir, &rebuild_id);
    log::info!(
        "rebuild {rebuild_id}: starting preserve-existing into {}",
        staging_dir.display(),
    );

    let staged_db_read = boot_state
        .read_db_state()
        .ok_or_else(|| "preserve-existing rebuild: db_conn missing".to_string())?;
    let (staged_write, staged_handle) = crate::search_writer::spawn_in_index_dir(
        &staging_dir,
        staged_db_read,
        notification_tx.clone(),
        service_generation,
    )
    .map_err(|e| format!("spawn staging search writer: {e}"))?;

    staged_write
        .clear_index()
        .await
        .map_err(|e| format!("staging clear_index: {e}"))?;
    live_search_write
        .mirror_to(&staged_write)
        .await
        .map_err(|e| format!("install staging mirror: {e}"))?;

    let rebuild_result = rebuild_all_messages(
        &rebuild_id,
        &cancel,
        &db,
        &staged_write,
        &body_read,
        &notification_tx,
        service_generation,
    )
    .await;
    if let Err(e) = rebuild_result {
        live_search_write.clear_mirror().await;
        drop(staged_write);
        staged_handle.abort();
        let _ = staged_handle.await;
        return Err(e);
    }
    if cancel.is_cancelled() {
        live_search_write.clear_mirror().await;
        drop(staged_write);
        staged_handle.abort();
        let _ = staged_handle.await;
        return Err("cancelled before cutover".into());
    }

    let mut pause = live_search_write.pause_writes().await;
    if let Err(e) = pause.flush_all().await {
        drop(pause);
        live_search_write.clear_mirror().await;
        drop(staged_write);
        staged_handle.abort();
        let _ = staged_handle.await;
        return Err(format!("flush before preserve cutover: {e}"));
    }
    if let Err(e) = crate::boot::write_search_index_version_at(&staging_dir) {
        drop(pause);
        live_search_write.clear_mirror().await;
        drop(staged_write);
        staged_handle.abort();
        let _ = staged_handle.await;
        return Err(e);
    }
    if let Err(e) = search::write_active_search_index_dir(&app_data_dir, &staging_dir) {
        drop(pause);
        live_search_write.clear_mirror().await;
        drop(staged_write);
        staged_handle.abort();
        let _ = staged_handle.await;
        return Err(e);
    }
    if let Err(e) = pause.set_primary_from(&staged_write).await {
        drop(pause);
        live_search_write.clear_mirror().await;
        drop(staged_write);
        staged_handle.abort();
        let _ = staged_handle.await;
        return Err(format!("route preserve cutover: {e}"));
    }
    let old_writer = boot_state.take_search_writer_handle();
    boot_state.install_search_writer_handle(staged_handle);
    drop(staged_write);
    drop(pause);

    if let Some(handle) = old_writer
        && let Err(e) = handle.await
    {
        log::warn!("old search writer join error after preserve cutover: {e}");
    }

    emit_rebuild_completed(&rebuild_id, &notification_tx, service_generation).await;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn rebuild_all_messages(
    rebuild_id: &str,
    cancel: &CancellationToken,
    db: &service_state::WriteDbState,
    search_write: &SearchWriteHandle,
    body_read: &BodyStoreReadState,
    notification_tx: &NotificationSender,
    service_generation: u32,
) -> Result<(), String> {
    let pairs = db
        .with_conn(db::db::queries_extra::select_all_message_ids_for_rebuild)
        .await
        .map_err(|e| format!("select_all_message_ids_for_rebuild: {e}"))?;
    let total = u64::try_from(pairs.len()).unwrap_or(u64::MAX);
    log::info!("rebuild {rebuild_id}: re-emitting {total} messages");

    let mut processed: u64 = 0;
    for chunk in pairs.chunks(REBUILD_CHUNK_SIZE) {
        if cancel.is_cancelled() {
            return Err("cancelled mid-iteration".into());
        }
        rebuild_chunk(db, search_write, body_read, chunk).await?;
        processed = processed.saturating_add(chunk.len() as u64);
        let progress = Notification::IndexRebuildProgress(IndexRebuildProgress {
            service_generation,
            rebuild_id: rebuild_id.to_string(),
            processed,
            total,
        });
        if let Err(e) = notification_tx.send(progress).await {
            log::debug!("rebuild progress send failed: {e}");
        }
    }

    Ok(())
}

async fn emit_rebuild_completed(
    rebuild_id: &str,
    notification_tx: &NotificationSender,
    service_generation: u32,
) {
    let completed = Notification::IndexRebuildCompleted(IndexRebuildCompleted {
        service_generation,
        rebuild_id: rebuild_id.to_string(),
    });
    if let Err(e) = notification_tx.send(completed).await {
        log::debug!("rebuild completed send failed: {e}");
    }
}

/// Build SearchDocuments for one chunk and send a single
/// `WriterCommand::Index`. Mirrors `extract::fan_out_reindex` shape
/// but operates on an arbitrary set of (account_id, message_id)
/// pairs rather than the per-content_hash fan-out.
async fn rebuild_chunk(
    db: &service_state::WriteDbState,
    search_write: &SearchWriteHandle,
    body_read: &BodyStoreReadState,
    pairs: &[(String, String)],
) -> Result<(), String> {
    let pairs_for_msgs = pairs.to_vec();
    let pairs_for_atts = pairs.to_vec();
    let messages_fut = db.with_conn(move |conn| {
        db::db::queries_extra::select_messages_for_index_batch(conn, &pairs_for_msgs)
    });
    let attachments_fut = db.with_read(move |conn| {
        db::db::queries_extra::select_attachment_fragments_batch(conn, &pairs_for_atts)
    });
    let message_ids: Vec<String> = pairs.iter().map(|(_, m)| m.clone()).collect();
    let bodies_fut = body_read.get_batch(message_ids);

    let (messages, mut fragments, bodies) =
        match tokio::join!(messages_fut, attachments_fut, bodies_fut) {
            (Ok(m), Ok(a), Ok(b)) => (m, a, b),
            (m, a, b) => {
                return Err(format!(
                    "rebuild chunk query failure (messages: {:?}, attachments: {:?}, bodies: {:?})",
                    m.as_ref().err(),
                    a.as_ref().err(),
                    b.as_ref().err(),
                ));
            }
        };

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
        let body_text = body_by_mid.remove(&m.message_id).unwrap_or(None);
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
        return Ok(());
    }
    search_write
        .index_messages_batch(docs)
        .await
        .map_err(|e| format!("index_messages_batch: {e}"))
}
