//! Pinned-search Service handlers.
//!
//! Phase 6a: `pinned_search.kick` notification - `Drop`-class self-heal
//! driven by the UI's 5-min `Message::SyncTick` fan-out. Mirrors the
//! shape of `gal.kick` and `calendar.kick`: UI fires the cadence,
//! Service runs the work and gates by a staleness window. Missed kicks
//! self-heal on the next `SyncTick`. The work is one global DELETE
//! keyed on the 14-day staleness threshold (matches today's UI-side
//! `expire_stale_pinned_searches` call). Pinned searches are not
//! per-account; the table is global. The DELETE is idempotent so
//! concurrent kicks are harmless.
//!
//! Phase 6a-part-2: per-row CRUD - `pinned_search.create_or_update`,
//! `pinned_search.update`, `pinned_search.delete`,
//! `pinned_search.delete_all`. Each handler is the standard six-line
//! shape: read the write-side DB handle from `BootSharedState`, run
//! the sync helper inside one `with_conn`, return the named ack.
//! The four sync helpers in `db::pinned_searches` keep all multi-step
//! work (UPSERT + member-thread replacement, conflict-cleanup +
//! UPDATE) inside one transaction so a partial commit on Service
//! crash mid-write is impossible.
//!
//! No back-notification on the per-row writes today - the sidebar's
//! in-memory list refreshes on the next reload path (the same
//! convention `pinned_search.kick` uses). If a real freshness gap
//! shows up, a `pinned_search.changed` notification on the existing
//! `Drop`-class infrastructure is the documented escape hatch.

use std::sync::Arc;

use serde_json::Value;
use service_api::{
    PinnedSearchCreateOrUpdateAck, PinnedSearchCreateOrUpdateParams, PinnedSearchDeleteAck,
    PinnedSearchDeleteAllAck, PinnedSearchDeleteAllParams, PinnedSearchDeleteParams,
    PinnedSearchUpdateAck, PinnedSearchUpdateParams, ServiceError,
};

use crate::boot::BootSharedState;

/// Today's threshold from the UI-side caller: 14 days
/// (1_209_600 seconds). Held as a constant here so the relocation
/// preserves the historical behavior verbatim.
const STALENESS_SECS: i64 = 1_209_600;

pub(crate) async fn handle_kick(boot_state: &Arc<BootSharedState>) -> Result<(), String> {
    let write_db = match boot_state.write_db_state() {
        Ok(db) => db,
        Err(_) => {
            log::debug!("pinned_search.kick received before db_conn available; ignoring");
            return Ok(());
        }
    };
    let deleted = write_db
        .with_conn(move |conn| {
            db::db::pinned_searches::db_expire_stale_pinned_searches_sync(conn, STALENESS_SECS)
        })
        .await?;
    if deleted > 0 {
        log::info!("pinned_search.kick: expired {deleted} stale pinned searches");
    }
    Ok(())
}

pub(crate) async fn handle_create_or_update(
    boot_state: &Arc<BootSharedState>,
    params: PinnedSearchCreateOrUpdateParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    let id = write_db
        .with_conn(move |conn| {
            // Wire's PinnedThreadRef -> tuple expected by the DB
            // helper. Cheap allocation; the Vec is small (sidebar
            // rows). Keeping the wire type human-readable wins over a
            // pseudo-tuple.
            let thread_ids: Vec<(String, String)> = params
                .thread_ids
                .into_iter()
                .map(|t| (t.thread_id, t.account_id))
                .collect();
            db::db::pinned_searches::db_create_or_update_pinned_search_sync(
                conn,
                &params.query,
                &thread_ids,
                params.scope_account_id.as_deref(),
            )
        })
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(PinnedSearchCreateOrUpdateAck { id })
        .map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_update(
    boot_state: &Arc<BootSharedState>,
    params: PinnedSearchUpdateParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    write_db
        .with_conn(move |conn| {
            let thread_ids: Vec<(String, String)> = params
                .thread_ids
                .into_iter()
                .map(|t| (t.thread_id, t.account_id))
                .collect();
            db::db::pinned_searches::db_update_pinned_search_sync(
                conn,
                params.id,
                &params.query,
                &thread_ids,
                params.scope_account_id.as_deref(),
            )
        })
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(PinnedSearchUpdateAck)
        .map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_delete(
    boot_state: &Arc<BootSharedState>,
    params: PinnedSearchDeleteParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    write_db
        .with_conn(move |conn| {
            db::db::pinned_searches::db_delete_pinned_search_sync(conn, params.id)
        })
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(PinnedSearchDeleteAck)
        .map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_delete_all(
    boot_state: &Arc<BootSharedState>,
    _params: PinnedSearchDeleteAllParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    let deleted = write_db
        .with_conn(move |conn| {
            db::db::pinned_searches::db_delete_all_pinned_searches_sync(conn)
        })
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(PinnedSearchDeleteAllAck { deleted })
        .map_err(|e| ServiceError::Internal(e.to_string()))
}
