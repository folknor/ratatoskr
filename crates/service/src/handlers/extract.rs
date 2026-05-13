//! Phase 7-4: handlers for `extract.status` / `index.rebuild` IPC and
//! the `extract.backfill_kick` client notification.
//!
//! 7-4b shipped these as stubs. 7-4d wired ExtractRuntime construction
//! into the post-ready spawn, so `handle_status` now returns live
//! counters and 7-6's `handle_backfill_kick` enqueues unindexed-cached
//! attachments. `handle_rebuild` remains a stub until 7-9.

use std::sync::Arc;

use serde_json::Value;
use service_api::{
    ExtractStatusAck, ExtractStatusParams, IndexRebuildAck, IndexRebuildParams, ServiceError,
};

use crate::boot::BootSharedState;
use crate::extract::ExtractWork;

const BACKFILL_KICK_LIMIT: usize = 1000;

#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn handle_status(
    boot_state: &Arc<BootSharedState>,
    _params: ExtractStatusParams,
) -> Result<Value, ServiceError> {
    let ack = if let Some(runtime) = boot_state.extract_runtime() {
        let (queue_depth, indexed_total, skipped_total, failed_total) = runtime.status_snapshot();
        ExtractStatusAck { queue_depth, indexed_total, skipped_total, failed_total }
    } else {
        // Pre-7-4d (no runtime installed) or post-shutdown.
        ExtractStatusAck {
            queue_depth: 0,
            indexed_total: 0,
            skipped_total: 0,
            failed_total: 0,
        }
    };
    serde_json::to_value(ack).map_err(|e| ServiceError::Internal(e.to_string()))
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn handle_rebuild(
    boot_state: &Arc<BootSharedState>,
    params: IndexRebuildParams,
) -> Result<Value, ServiceError> {
    let search_write = boot_state
        .search_write()
        .or_else(|| boot_state.sync_runtime().map(|runtime| runtime.search_write()))
        .ok_or_else(|| {
            ServiceError::Internal("index.rebuild: search_write missing post-boot.ready".into())
        })?;

    // Reject (or pre-empt) a rebuild that's already in flight.
    if let Some(in_flight_id) = boot_state.rebuild_in_flight_id() {
        if !params.force {
            return Err(ServiceError::Internal(format!(
                "index.rebuild already in flight (rebuild_id={in_flight_id}); pass force=true to pre-empt"
            )));
        }
        // force=true: cancel + await the previous rebuild before
        // installing the new one. The rebuild task respects
        // CancellationToken between chunks.
        if let Some(prev) = boot_state.take_rebuild_task() {
            log::info!("index.rebuild: pre-empting previous rebuild {}", prev.rebuild_id);
            prev.cancel.cancel();
            // Don't .await on prev.handle here - the handler is
            // user-facing and shouldn't block on the previous
            // rebuild's chunk. abort drops the future at the next
            // await point.
            prev.handle.abort();
            search_write.clear_mirror().await;
        }
    }

    // Resolve the runtime dependencies. Errors here are programming
    // bugs (boot completes before any handler is dispatched).
    let db_conn = boot_state.db_conn().ok_or_else(|| {
        ServiceError::Internal("index.rebuild: db_conn missing post-boot.ready".into())
    })?;
    let body_read =
        store::body_store::BodyStoreReadState::init(boot_state.app_data_dir()).map_err(|e| {
            ServiceError::Internal(format!("index.rebuild: body_store init failed: {e}"))
        })?;
    let notification_tx = boot_state.notification_sender().ok_or_else(|| {
        ServiceError::Internal(
            "index.rebuild: out_tx slot empty (boot incomplete or shutting down)".into(),
        )
    })?;

    let rebuild_id = uuid::Uuid::new_v4().to_string();
    let cancel = tokio_util::sync::CancellationToken::new();

    match params.policy {
        service_api::RebuildPolicy::Wipe => {
            let db_state = service_state::WriteDbState::from_arc(db_conn);
            let boot_state_for_task = Arc::clone(boot_state);
            let cancel_for_task = cancel.clone();
            let id_for_task = rebuild_id.clone();
            let handle = tokio::spawn(async move {
                crate::rebuild::run_wipe_rebuild(
                    boot_state_for_task,
                    id_for_task,
                    cancel_for_task,
                    db_state,
                    search_write,
                    body_read,
                    notification_tx,
                    // service_generation is overwritten by the UI's
                    // reader task at enqueue time per the
                    // WithGeneration trait contract; emit 0 here.
                    0,
                )
                .await;
            });
            boot_state.install_rebuild_task(crate::boot::RebuildTaskState {
                rebuild_id: rebuild_id.clone(),
                cancel,
                handle,
            });
        }
        service_api::RebuildPolicy::PreserveExisting => {
            let db_state = service_state::WriteDbState::from_arc(db_conn);
            let boot_state_for_task = Arc::clone(boot_state);
            let cancel_for_task = cancel.clone();
            let id_for_task = rebuild_id.clone();
            let handle = tokio::spawn(async move {
                crate::rebuild::run_preserve_existing_rebuild(
                    boot_state_for_task,
                    id_for_task,
                    cancel_for_task,
                    db_state,
                    search_write,
                    body_read,
                    notification_tx,
                    // service_generation is overwritten by the UI's
                    // reader task at enqueue time per the
                    // WithGeneration trait contract; emit 0 here.
                    0,
                )
                .await;
            });
            boot_state.install_rebuild_task(crate::boot::RebuildTaskState {
                rebuild_id: rebuild_id.clone(),
                cancel,
                handle,
            });
        }
    }

    let ack = make_rebuild_ack(rebuild_id);
    serde_json::to_value(ack).map_err(|e| ServiceError::Internal(e.to_string()))
}


/// Phase 7-6: post-boot backfill. Selects up to
/// `BACKFILL_KICK_LIMIT` attachment rows that are cached but
/// unindexed, and enqueues each into the installed `ExtractRuntime`.
/// Idempotent on repeat: the runtime's `in_flight_hashes` dedupe
/// rejects duplicates while extraction is in progress, and the
/// worker's status-aware skip handles already-extracted rows. A
/// second kick after the first finishes returns 0 rows from the
/// SELECT and is therefore a no-op.
///
/// Skips:
/// - rows with no `content_hash` (the worker can't extract without
///   one; sync's normal write path always populates the hash before
///   the matching `attachment_blobs` row, so a NULL here means a sync
///   ordering bug or a manually-injected row).
/// - the call entirely if no `ExtractRuntime` is installed - this is
///   the case during shutdown and during the brief window before the
///   post-ready spawn finishes installing the runtime.
pub(crate) async fn handle_backfill_kick(
    boot_state: &Arc<BootSharedState>,
) -> Result<(), String> {
    let Some(runtime) = boot_state.extract_runtime() else {
        log::debug!("extract.backfill_kick: ExtractRuntime not installed, skipping");
        return Ok(());
    };
    let Some(db_conn) = boot_state.db_conn() else {
        log::debug!("extract.backfill_kick: db_conn missing, skipping");
        return Ok(());
    };
    let db = service_state::WriteDbState::from_arc(db_conn);
    let rows = db
        .with_conn(move |conn| {
            db::db::queries_extra::find_unindexed_cached_attachments(conn, BACKFILL_KICK_LIMIT)
        })
        .await
        .map_err(|e| format!("extract.backfill_kick: query failed: {e}"))?;
    if rows.is_empty() {
        log::debug!("extract.backfill_kick: no unindexed cached attachments");
        return Ok(());
    }
    log::info!("extract.backfill_kick: enqueuing {} attachments", rows.len());
    for row in rows {
        let Some(content_hash) = row.content_hash else {
            log::debug!(
                "extract.backfill_kick: skipping {} (no content_hash)",
                row.attachment_id
            );
            continue;
        };
        let work = ExtractWork {
            content_hash,
            account_id: row.account_id,
            message_id: row.message_id,
            attachment_id: row.attachment_id,
        };
        // Q2 close: try_enqueue instead of awaiting send. Backfill is
        // a Drop-class trigger - the work is idempotent, the next
        // hourly tick re-emits any rows the queue couldn't accept.
        // Pre-fix the await on a bounded mpsc could park the handler
        // for tens of minutes (1000 BACKFILL_KICK_LIMIT items at a
        // 256-mpsc capacity with a 30s p95 per item under
        // WORKER_CONCURRENCY=4). The handler is async and shouldn't
        // hold execution that long; subsequent UI kicks (post-boot
        // catch-up + hourly ticker) would queue up serially behind it.
        // try_enqueue fills the queue up to capacity per kick and
        // drops the rest; with rows now staying out of the backfill
        // SELECT once their text_indexed_at is set (C3 fix),
        // steady-state backlog shrinks fast.
        if let Err(e) = runtime.try_enqueue(work) {
            // Runtime closed. Recoverable - the next kick will retry.
            log::warn!("extract.backfill_kick: try_enqueue failed: {e}");
            break;
        }
    }
    Ok(())
}

#[allow(dead_code)] // Used by the rebuild ack path once implemented.
pub(crate) fn make_rebuild_ack(rebuild_id: String) -> IndexRebuildAck {
    IndexRebuildAck { rebuild_id }
}
