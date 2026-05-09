//! `calendar.start_account_sync` + `calendar.cancel_account_sync` request
//! handlers and the `calendar.kick` notification handler.
//!
//! Phase 5 task 4. Forwards the wire request into the `CalendarRuntime`
//! installed on `BootSharedState` by the post-ready runtime task. The
//! `CalendarRuntime` itself spawns the runner under a panic supervisor
//! and emits the dual notifications (`CalendarRunCompleted` +
//! `CalendarChanged`); the handler's responsibility is purely to
//! translate the request into a runtime call and serialize the ack.
//!
//! Reachability: in production the UI parks on `boot.ready` before
//! issuing any calendar request, so the "runtime not yet installed"
//! branch should be unreachable - an `Internal` error is a debug aid.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use service_api::{
    CalendarCancelAccountSyncParams, CalendarSetVisibilityAck, CalendarSetVisibilityParams,
    CalendarStartAccountSyncParams, ServiceError,
};

use crate::boot::BootSharedState;

/// Staleness threshold for the kick-driven path. Accounts whose last
/// `CalendarRuntime` run completed within this window are skipped on
/// `calendar.kick` (the explicit-request path bypasses the gate).
///
/// 1 hour matches the deleted UI-side `Message::GalRefreshTick` cadence
/// the kick replaces; the actual cadence stays UI-driven on the 5-min
/// `SyncTick`, with this staleness check enforcing the effective hourly
/// rate Service-side.
const CALENDAR_STALENESS: Duration = Duration::from_secs(60 * 60);

pub(crate) async fn handle_start_account_sync(
    boot_state: &Arc<BootSharedState>,
    params: CalendarStartAccountSyncParams,
) -> Result<Value, ServiceError> {
    let runtime = boot_state.calendar_runtime().ok_or_else(|| {
        ServiceError::Internal(
            "calendar.start_account_sync received before CalendarRuntime was \
             installed; UI must wait for boot.ready"
                .into(),
        )
    })?;
    let ack = runtime
        .start_account(params.account_id)
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(ack).map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_cancel_account_sync(
    boot_state: &Arc<BootSharedState>,
    params: CalendarCancelAccountSyncParams,
) -> Result<Value, ServiceError> {
    let runtime = boot_state.calendar_runtime().ok_or_else(|| {
        ServiceError::Internal(
            "calendar.cancel_account_sync received before CalendarRuntime was \
             installed; UI must wait for boot.ready"
                .into(),
        )
    })?;
    let ack = runtime.cancel_account(&params.account_id).await;
    serde_json::to_value(ack).map_err(|e| ServiceError::Internal(e.to_string()))
}

/// `calendar.set_visibility` request handler (Phase 6a). Toggles the
/// `is_visible` flag on a single `calendars` row. Thin wrapper around
/// `set_calendar_visibility_sync`; calendar event mutations stay
/// UI-side until Phase 6c. `WriteDbState` comes from
/// `BootSharedState::write_db_state()` so the boilerplate stays in one
/// place across the Phase 6a write-surface handlers.
pub(crate) async fn handle_set_visibility(
    boot_state: &Arc<BootSharedState>,
    params: CalendarSetVisibilityParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    write_db
        .with_conn(move |conn| {
            db::db::queries_extra::calendars::set_calendar_visibility_sync(
                conn,
                &params.calendar_id,
                params.visible,
            )
        })
        .await
        .map_err(ServiceError::Internal)?;
    // Always go through `to_value` (even for unit-struct acks) so
    // adding a field to the ack later is a single-site edit.
    serde_json::to_value(CalendarSetVisibilityAck)
        .map_err(|e| ServiceError::Internal(e.to_string()))
}

/// `calendar.kick` notification handler. Enumerates accounts and starts
/// runners for those whose `last_calendar_sync` is more than
/// `CALENDAR_STALENESS` old. The runtime's `start_account` is
/// idempotent - already-in-flight accounts are no-ops, the per-runtime
/// semaphore caps concurrent runners.
pub(crate) async fn handle_calendar_kick(
    boot_state: &Arc<BootSharedState>,
) -> Result<(), String> {
    let Some(runtime) = boot_state.calendar_runtime() else {
        log::debug!("calendar.kick received before CalendarRuntime installed; ignoring");
        return Ok(());
    };

    let account_ids = list_calendar_capable_account_ids(boot_state).await?;
    if account_ids.is_empty() {
        return Ok(());
    }

    let stale = runtime
        .accounts_due_for_sync(account_ids, CALENDAR_STALENESS)
        .await;

    log::debug!(
        "calendar.kick: {} accounts past staleness threshold",
        stale.len()
    );

    for account_id in stale {
        if let Err(e) = runtime.start_account(account_id.clone()).await {
            log::warn!("[calendar] kick start failed for {account_id}: {e}");
        }
    }
    Ok(())
}

/// Read calendar-capable account ids from the DB. Filtering at
/// enumeration is what keeps the kick handler from re-failing
/// IMAP/JMAP-only accounts every hour through the
/// `"No calendar provider configured"` path. The shared helper lives in
/// `db::queries_extra::list_calendar_capable_account_ids_sync`.
async fn list_calendar_capable_account_ids(
    boot_state: &Arc<BootSharedState>,
) -> Result<Vec<String>, String> {
    let Some(conn) = boot_state.db_conn() else {
        return Err("calendar.kick: db connection not available".into());
    };
    let read_db = db::db::ReadDbState::from_arc(conn);
    read_db
        .with_conn(db::db::queries_extra::list_calendar_capable_account_ids_sync)
        .await
}
