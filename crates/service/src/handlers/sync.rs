//! `sync.start_account` + `sync.cancel_account` JSON-RPC handlers.
//!
//! Phase 3 task 9: forwards the wire request into the
//! `SyncRuntime` installed on `BootSharedState` by the boot task. The
//! `SyncRuntime` itself spawns the runner under a panic supervisor and
//! emits the terminal `sync.completed` notification; the handler's
//! responsibility is purely to translate the request into a runtime
//! call and serialize the ack.
//!
//! Both handlers return a `ServiceError::Internal` if the runtime is
//! not yet installed. In production the UI parks on `boot.ready`
//! before issuing any sync request, so this branch should not be
//! reachable; an Internal error is a debug aid for a UI bug or a
//! future test that races boot.

use crate::boot::BootSharedState;
use serde_json::Value;
use service_api::{ServiceError, SyncCancelAccountParams, SyncStartAccountParams};
use std::sync::Arc;

pub(crate) async fn handle_start_account(
    boot_state: &Arc<BootSharedState>,
    params: SyncStartAccountParams,
) -> Result<Value, ServiceError> {
    let runtime = boot_state.sync_runtime().ok_or_else(|| {
        ServiceError::Internal(
            "sync.start_account received before SyncRuntime was installed; \
             UI must wait for boot.ready"
                .into(),
        )
    })?;
    let ack = runtime.start_account(params.account_id).await;

    serde_json::to_value(ack).map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_cancel_account(
    boot_state: &Arc<BootSharedState>,
    params: SyncCancelAccountParams,
) -> Result<Value, ServiceError> {
    let runtime = boot_state.sync_runtime().ok_or_else(|| {
        ServiceError::Internal(
            "sync.cancel_account received before SyncRuntime was installed; \
             UI must wait for boot.ready"
                .into(),
        )
    })?;
    let mut ack = runtime.cancel_account(&params.account_id).await;

    if let Err(error) = runtime.detach_resident_account(&params.account_id).await {
        log::debug!(
            "sync.cancel_account: resident detach for {} returned {error}",
            params.account_id
        );
    }

    // Phase 5 task 9: piggyback calendar cancel server-side, mirroring
    // the push pattern. Stamps `calendar_run_id` on the ack so the UI's
    // cancel_and_await path can subscribe to CalendarRunCompleted for
    // the cancelled run before issuing the DB DELETE. Calendar tables
    // CASCADE from `accounts` (db/src/db/schema/05_calendar.sql:5);
    // without this piggyback a calendar runner with an open
    // WriteDbState write borrow could race the DELETE FROM accounts.
    if let Some(calendar_runtime) = boot_state.calendar_runtime() {
        let cal_ack = calendar_runtime.cancel_account(&params.account_id).await;
        ack.calendar_run_id = cal_ack.run_id;
    }

    serde_json::to_value(ack).map_err(|e| ServiceError::Internal(e.to_string()))
}
