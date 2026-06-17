//! `pending_ops.kick` notification handler.
//!
//! Phase 2 plan scope item 11 + task 18: the UI fires this when its
//! `Message::SyncTick` handler decides to nudge the Service into
//! draining `pending_operations`. The handler simply wakes the action
//! worker via `BootSharedState::notify_action_worker`; the worker
//! drains both the journal and the pending-ops retry queue on each
//! wakeup (see `crates/service/src/actions/worker.rs`).
//!
//! Returns immediately - the actual drain work is the worker's, not
//! this handler's. The Drop-class admission in the dispatch loop
//! (`NOTIFY_CAP = 4`) means a UI bug that fires kicks in a tight
//! loop is bounded server-side; the worker only wakes once per
//! arrival so spurious kicks are cheap.

use crate::boot::BootSharedState;
use db::db::pending_ops::db_pending_ops_cancel_for_resource_sync;
use std::sync::Arc;

pub(super) async fn handle(state: &Arc<BootSharedState>) -> Result<(), String> {
    log::debug!("pending_ops.kick received; signalling action worker");
    state.notify_action_worker();
    Ok(())
}

pub(super) async fn handle_cancel_for_resource(
    state: &Arc<BootSharedState>,
    account_id: String,
    resource_id: String,
    operation_type: String,
) -> Result<(), String> {
    let db = state
        .write_db_state()
        .map_err(|error| format!("boot context not populated: {error}"))?;
    db.with_write(move |conn| {
        db_pending_ops_cancel_for_resource_sync(conn, &account_id, &resource_id, &operation_type)
    })
    .await
}
