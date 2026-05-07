//! Phase 7-4: handlers for `extract.status` / `index.rebuild` IPC and
//! the `extract.backfill_kick` client notification.
//!
//! 7-4b ships these as stubs that return reasonable placeholders or
//! errors so the dispatch enums are exhaustive without committing to
//! the real ExtractRuntime wiring (which lands in 7-4c). The wire
//! envelope and IPC catalog tests pass at this layer; the actual
//! extract pipeline is plumbed in the next slice.

use std::sync::Arc;

use serde_json::Value;
use service_api::{
    ExtractStatusAck, ExtractStatusParams, IndexRebuildAck, IndexRebuildParams, ServiceError,
};

use crate::boot::BootSharedState;

#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn handle_status(
    _boot_state: &Arc<BootSharedState>,
    _params: ExtractStatusParams,
) -> Result<Value, ServiceError> {
    // 7-4c: read counters from ExtractRuntime once it lands.
    let ack = ExtractStatusAck {
        queue_depth: 0,
        indexed_total: 0,
        skipped_total: 0,
        failed_total: 0,
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

pub(crate) async fn handle_backfill_kick(
    _boot_state: &Arc<BootSharedState>,
) -> Result<(), String> {
    // 7-6: scan attachments WHERE cached_at IS NOT NULL AND
    // text_indexed_at IS NULL LIMIT 1000 and enqueue each into the
    // ExtractRuntime. 7-4b is a no-op until the runtime exists.
    Ok(())
}

#[allow(dead_code)] // Used by the rebuild ack path once implemented.
pub(crate) fn make_rebuild_ack(rebuild_id: String) -> IndexRebuildAck {
    IndexRebuildAck { rebuild_id }
}
