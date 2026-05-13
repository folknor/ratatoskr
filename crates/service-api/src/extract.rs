//! Phase 7-4: text-extraction + index-rebuild wire types.
//!
//! ## Notifications (Service -> UI)
//!
//! - `extract.progress` (Coalesce, key = `()`): per-tick remaining +
//!   indexed counts for the status-bar indicator.
//! - `extract.completed` (MustDeliver): fired once when the
//!   ExtractRuntime queue drains to zero.
//! - `index.rebuild_progress` (Coalesce, key = `rebuild_id`): per-batch
//!   processed/total counts during a Wipe or PreserveExisting rebuild.
//! - `index.rebuild_completed` (MustDeliver): fired once per rebuild
//!   when the tracked task exits (success or cancel).
//!
//! Every state-changing notification carries `service_generation`
//! per the `notification.rs:245` contract; cross-respawn dispatch
//! filtering would otherwise apply stale notifications to a fresh UI.
//!
//! ## Requests (UI -> Service)
//!
//! - `extract.status`: read the ExtractRuntime's running totals for
//!   status-bar polling. Cheap; no DB round-trip beyond the in-memory
//!   counters.
//! - `index.rebuild`: trigger a rebuild. Tracked-task spawned by the
//!   handler; the IPC ack returns immediately with the `rebuild_id`
//!   the UI subscribes to via `index.rebuild_progress` / `_completed`.
//!
//! ## Client kicks (UI -> Service)
//!
//! - `extract.backfill_kick` (Drop): UI fans this out once on
//!   `boot.ready` and on a separate hourly subscription. The Service
//!   handler scans cached + unindexed attachments and enqueues each
//!   into the ExtractRuntime. Capped at 1000 rows per kick so a
//!   100k-attachment backlog doesn't blow the mpsc.

use serde::{Deserialize, Serialize};

use crate::notification::WithGeneration;

// ---------------------------------------------------------------------------
// extract.status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ExtractStatusParams {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractStatusAck {
    pub queue_depth:    u64,
    pub indexed_total:  u64,
    pub skipped_total:  u64,
    pub failed_total:   u64,
}

// ---------------------------------------------------------------------------
// index.rebuild
// ---------------------------------------------------------------------------

/// Rebuild flavor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebuildPolicy {
    /// Clear the active index and repopulate it in place. Search is
    /// briefly unavailable while the rebuild runs.
    Wipe,
    /// Build a staging index while the existing reader stays live,
    /// mirror concurrent writes into staging, then atomically update
    /// the active-index pointer and rebind the UI reader on completion.
    PreserveExisting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexRebuildParams {
    pub policy: RebuildPolicy,
    pub force:  bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexRebuildAck {
    pub rebuild_id: String,
}

// ---------------------------------------------------------------------------
// Notification payloads
// ---------------------------------------------------------------------------

/// Per-tick extraction progress. `Coalesce { key: () }`: latest-wins
/// for the status-bar indicator. Several extractions completing
/// rapidly produce one indicator update, not N.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractProgress {
    pub service_generation:  u32,
    /// Items still queued or in-flight in the ExtractRuntime.
    pub remaining:           u64,
    /// Items extracted (status='indexed') in this Service-incarnation.
    pub indexed_in_session:  u64,
}

impl WithGeneration for ExtractProgress {
    fn generation(&self) -> u32 { self.service_generation }
    fn set_generation(&mut self, generation: u32) { self.service_generation = generation; }
}

/// Final extraction summary. Fired once when the ExtractRuntime queue
/// drains to zero (no in-flight, no enqueued items remain). MustDeliver:
/// the UI's status-bar dismiss logic awaits this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractCompleted {
    pub service_generation: u32,
    pub indexed: u64,
    pub skipped: u64,
    pub failed:  u64,
}

impl WithGeneration for ExtractCompleted {
    fn generation(&self) -> u32 { self.service_generation }
    fn set_generation(&mut self, generation: u32) { self.service_generation = generation; }
}

/// Phase 4 (attachments roadmap): per-tick `PrefetchRuntime` progress.
/// `Coalesce { key: () }`: latest-wins for the status-bar "Caching
/// attachments... N / M" indicator. Mirrors `ExtractProgress`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrefetchProgress {
    pub service_generation: u32,
    /// Items still queued or in-flight across both priority queues.
    pub remaining:          u64,
    /// Items whose bytes were successfully written to PackStore in
    /// this Service-incarnation.
    pub fetched_in_session: u64,
}

impl WithGeneration for PrefetchProgress {
    fn generation(&self) -> u32 { self.service_generation }
    fn set_generation(&mut self, generation: u32) { self.service_generation = generation; }
}

/// Phase 4 (attachments roadmap): `PrefetchRuntime` queue-drained-to-
/// zero summary. Fired once when both queues are empty and nothing is
/// in-flight. `MustDeliver`: the UI's status-bar dismiss logic awaits
/// this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrefetchCompleted {
    pub service_generation: u32,
    pub fetched: u64,
    pub skipped: u64,
    pub failed:  u64,
}

impl WithGeneration for PrefetchCompleted {
    fn generation(&self) -> u32 { self.service_generation }
    fn set_generation(&mut self, generation: u32) { self.service_generation = generation; }
}

/// Per-rebuild progress. `Coalesce { key: rebuild_id }`: a viral
/// progress emission collapses to the latest-per-rebuild, but
/// concurrent rebuilds (rare) don't collide.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexRebuildProgress {
    pub service_generation: u32,
    pub rebuild_id:         String,
    pub processed:          u64,
    pub total:              u64,
}

impl WithGeneration for IndexRebuildProgress {
    fn generation(&self) -> u32 { self.service_generation }
    fn set_generation(&mut self, generation: u32) { self.service_generation = generation; }
}

/// Per-rebuild completion. Fired once when the tracked rebuild task
/// exits (success or cancel). MustDeliver: the UI's status-bar
/// dismiss logic awaits this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexRebuildCompleted {
    pub service_generation: u32,
    pub rebuild_id:         String,
}

impl WithGeneration for IndexRebuildCompleted {
    fn generation(&self) -> u32 { self.service_generation }
    fn set_generation(&mut self, generation: u32) { self.service_generation = generation; }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebuild_policy_round_trips() {
        let p = RebuildPolicy::Wipe;
        let json = serde_json::to_value(p).expect("serialize");
        let recovered: RebuildPolicy = serde_json::from_value(json).expect("deserialize");
        assert_eq!(p, recovered);
        let p = RebuildPolicy::PreserveExisting;
        let json = serde_json::to_value(p).expect("serialize");
        let recovered: RebuildPolicy = serde_json::from_value(json).expect("deserialize");
        assert_eq!(p, recovered);
    }

    #[test]
    fn extract_status_ack_round_trips() {
        let ack = ExtractStatusAck {
            queue_depth: 5,
            indexed_total: 100,
            skipped_total: 3,
            failed_total: 1,
        };
        let json = serde_json::to_value(&ack).expect("serialize");
        let recovered: ExtractStatusAck = serde_json::from_value(json).expect("deserialize");
        assert_eq!(ack, recovered);
    }

    #[test]
    fn index_rebuild_params_round_trips() {
        let cases = [
            IndexRebuildParams { policy: RebuildPolicy::Wipe, force: false },
            IndexRebuildParams { policy: RebuildPolicy::Wipe, force: true },
            IndexRebuildParams { policy: RebuildPolicy::PreserveExisting, force: false },
            IndexRebuildParams { policy: RebuildPolicy::PreserveExisting, force: true },
        ];
        for params in cases {
            let json = serde_json::to_value(&params).expect("serialize");
            let recovered: IndexRebuildParams = serde_json::from_value(json).expect("deserialize");
            assert_eq!(params, recovered);
        }
    }

    #[test]
    fn extract_progress_with_generation_round_trips() {
        let mut progress = ExtractProgress {
            service_generation: 3,
            remaining: 12,
            indexed_in_session: 47,
        };
        assert_eq!(progress.generation(), 3);
        progress.set_generation(99);
        assert_eq!(progress.service_generation, 99);
        let json = serde_json::to_value(&progress).expect("serialize");
        let recovered: ExtractProgress = serde_json::from_value(json).expect("deserialize");
        assert_eq!(progress, recovered);
    }

    #[test]
    fn extract_completed_with_generation_round_trips() {
        let mut completed = ExtractCompleted {
            service_generation: 1,
            indexed: 100, skipped: 5, failed: 2,
        };
        assert_eq!(completed.generation(), 1);
        completed.set_generation(7);
        assert_eq!(completed.service_generation, 7);
        let json = serde_json::to_value(&completed).expect("serialize");
        let recovered: ExtractCompleted = serde_json::from_value(json).expect("deserialize");
        assert_eq!(completed, recovered);
    }

    #[test]
    fn index_rebuild_progress_with_generation_round_trips() {
        let mut progress = IndexRebuildProgress {
            service_generation: 2,
            rebuild_id: "rb-1".into(),
            processed: 1000,
            total: 5000,
        };
        progress.set_generation(8);
        assert_eq!(progress.service_generation, 8);
        let json = serde_json::to_value(&progress).expect("serialize");
        let recovered: IndexRebuildProgress = serde_json::from_value(json).expect("deserialize");
        assert_eq!(progress, recovered);
    }

    #[test]
    fn index_rebuild_completed_with_generation_round_trips() {
        let mut c = IndexRebuildCompleted {
            service_generation: 4,
            rebuild_id: "rb-2".into(),
        };
        c.set_generation(11);
        assert_eq!(c.service_generation, 11);
        let json = serde_json::to_value(&c).expect("serialize");
        let recovered: IndexRebuildCompleted = serde_json::from_value(json).expect("deserialize");
        assert_eq!(c, recovered);
    }
}
