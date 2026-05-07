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
    _boot_state: &Arc<BootSharedState>,
    _params: IndexRebuildParams,
) -> Result<Value, ServiceError> {
    // 7-9: spawn a tracked rebuild task and return its rebuild_id.
    // 7-4b stub returns a deterministic placeholder so the wire path
    // round-trips and tests can assert the response shape.
    Err(ServiceError::Internal(
        "index.rebuild not yet implemented (lands in phase 7-9)".into(),
    ))
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
///   `cached_at`, so a NULL here means a sync ordering bug or a
///   manually-injected row).
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
        if let Err(e) = runtime.enqueue(work).await {
            // Runtime closed or queue full. Both are recoverable -
            // the next kick will retry.
            log::warn!("extract.backfill_kick: enqueue failed: {e}");
            break;
        }
    }
    Ok(())
}

#[allow(dead_code)] // Used by the rebuild ack path once implemented.
pub(crate) fn make_rebuild_ack(rebuild_id: String) -> IndexRebuildAck {
    IndexRebuildAck { rebuild_id }
}
