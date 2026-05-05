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
    let account_id = params.account_id.clone();
    let ack = runtime.start_account(params.account_id).await;

    // Phase 4 task 6: opportunistic push start. Detached:
    // PushRuntime::start_account does TLS+HTTPS+OAuth-refresh, which
    // would violate the 5s sync.start_account IPC contract if awaited.
    // Failure is logged inside the runtime, not surfaced through the
    // SyncStartAck. PushRuntime::start_account is idempotent and
    // self-policing (no-ops for non-JMAP accounts), so the handler
    // doesn't need to know which provider an account uses.
    if let Some(push_runtime) = boot_state.push_runtime() {
        tokio::spawn(async move {
            if let Err(e) = push_runtime.start_account(account_id.clone()).await {
                log::debug!("[push] piggyback start_account({account_id}) failed: {e}");
            }
        });
    }

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
    let account_id = params.account_id.clone();
    let ack = runtime.cancel_account(&params.account_id).await;

    // Phase 4 task 6: symmetric push cancel piggyback. Detached for the
    // same 5s-IPC-contract reason as the start side; the bridge runs
    // manager.stop_push().await on its exit path which itself awaits a
    // WebSocket close. cancel_account returns false if no bridge was
    // registered (account is non-JMAP, never started, or already
    // cancelled), which is a normal no-op.
    if let Some(push_runtime) = boot_state.push_runtime() {
        tokio::spawn(async move {
            push_runtime.cancel_account(&account_id).await;
        });
    }

    serde_json::to_value(ack).map_err(|e| ServiceError::Internal(e.to_string()))
}
