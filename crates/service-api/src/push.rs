//! JMAP push wire types.
//!
//! Phase 4 of `docs/service/phase-4-plan.md` relocates the JMAP push
//! WebSocket loop into the Service. The Service-internal bridge task
//! calls `SyncRuntime::start_account` directly on each debounced
//! StateChange burst and emits a `push.event` notification afterwards
//! so the UI's status bar can surface "new mail arrived" indicators.
//!
//! Class is `Coalesce { key: PushEvent(account_id) }` - status-bar
//! semantics are latest-wins per account; nobody waits on a `PushEvent`
//! future, so drop-on-overflow is benign. `MustDeliver` would
//! backpressure the bridge task on send and delay the next StateChange's
//! sync kick, which would invert the priority (sync correctness is
//! `MustDeliver`'s job, not status-bar updates).

use crate::notification::WithGeneration;
use serde::{Deserialize, Serialize};

/// Service-side JMAP push event for one account.
///
/// Emitted from the per-account bridge task after a debounced
/// StateChange burst kicks `SyncRuntime::start_account`. Carries the
/// account so the UI can update the right per-account status indicator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushEvent {
    pub account_id: String,
    /// Cross-respawn drop tag. Service emits 0; UI's reader task
    /// overwrites at enqueue with `current_generation()`.
    pub service_generation: u32,
}

impl WithGeneration for PushEvent {
    fn generation(&self) -> u32 {
        self.service_generation
    }
    fn set_generation(&mut self, generation: u32) {
        self.service_generation = generation;
    }
}
