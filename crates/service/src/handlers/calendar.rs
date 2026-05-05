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
    CalendarCancelAccountSyncParams, CalendarStartAccountSyncParams, ServiceError,
};

use crate::boot::BootSharedState;

/// Staleness threshold for the kick-driven path. Accounts whose last
/// `CalendarRuntime` run completed within this window are skipped on
/// `calendar.kick` (the explicit-request path bypasses the gate).
///
/// 1 hour matches the deleted UI-side `Message::GalRefreshTick` cadence
/// the kick replaces; the actual cadence stays UI-driven on the 5-min
/// `SyncTick`, with this staleness check enforcing the effective hourly
/// rate Service-side. See `docs/service/phase-5-plan.md` § "Cadence-
/// driven kicks" for the design.
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

    let account_ids = list_all_account_ids(boot_state).await?;
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

/// Read all account ids from the DB. Inline because the existing
/// `accounts_crud` module has many narrower lookups but no "list all
/// account ids" helper; lifting one would touch read-side query
/// surfaces unrelated to Phase 5.
async fn list_all_account_ids(boot_state: &Arc<BootSharedState>) -> Result<Vec<String>, String> {
    let Some(conn) = boot_state.db_conn() else {
        return Err("calendar.kick: db connection not available".into());
    };
    let read_db = db::db::ReadDbState::from_arc(conn);
    read_db
        .with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT id FROM accounts ORDER BY sort_order")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            Ok(rows)
        })
        .await
}
