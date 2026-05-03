//! `pending_ops.kick` notification handler.
//!
//! Phase 2 plan scope item 11 + task 18: the UI fires this when its
//! `Message::SyncTick` handler decides to nudge the Service into
//! draining `pending_operations`. Phase 2 lands the framing scaffold;
//! the actual periodic-drainer relocation lives in task 18 and will
//! call into the relocated `process_pending_ops` from here.
//!
//! For Phase 2 task 11b this is a stub: it logs the kick at debug
//! level and returns Ok. Task 18 fills in the body once the drainer
//! has moved Service-side.

use crate::boot::BootSharedState;
use std::sync::Arc;

pub(super) async fn handle(_state: &Arc<BootSharedState>) -> Result<(), String> {
    log::debug!("pending_ops.kick received (drainer wiring lands in task 18)");
    Ok(())
}
