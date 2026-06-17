//! `attachment.fetch` wire types.
//!
//! The UI ships `(account_id, message_id, attachment_id)` and the
//! Service returns the path of a freshly-materialized tmp file under
//! `<app_data>/attachment_fetch_tmp/<content_hash>-<request_id>`.
//! Bytes never cross the IPC; the tmp file is the contract.
//!
//! The pack store (attachments roadmap Phase 2) is the source of
//! truth for cached blobs. On a cache hit the Service writes the
//! blob bytes out to a unique tmp file (`<hash>-<uuid>.part` then
//! atomic rename) and returns the path. On a cache miss the Service
//! runs the full pipeline: provider fetch → BLAKE3 →
//! `PackStore::put` → materialize → ack. The UI re-opens the tmp
//! file positionally; the open fd is the pin (`unlink` is fd-safe on
//! Linux so a concurrent reap survives an in-flight read). An idle
//! cleanup pass (`attachment.tmp_cleanup_kick`) reaps tmp entries
//! older than 10 minutes.

use serde::{Deserialize, Serialize};

/// `attachment.fetch` request body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentFetchParams {
    pub account_id: String,
    pub message_id: String,
    pub attachment_id: String,
}

/// `attachment.fetch` ack.
///
/// `relative_path` is rooted at `<app_data>/` and takes the form
/// `attachment_fetch_tmp/<content_hash>-<request_id>`. The UI re-opens
/// the file positionally; the open fd is the pin against the idle
/// cleanup kick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentFetchAck {
    pub content_hash: String,
    pub size_bytes: u64,
    pub relative_path: String,
}

/// Attachments roadmap Phase 6: `attachment.cache_size` request body.
/// Global readout - no parameters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AttachmentCacheSizeParams {}

/// Attachments roadmap Phase 6: `attachment.cache_size` ack.
///
/// `live_bytes` sums `attachment_blobs.length` where `tombstoned_at IS
/// NULL` - the on-disk size users care about for the settings readout.
/// `tombstoned_bytes` is the wasted space reclaimable by the next GC
/// repack; surfaced separately so the UI can show both ("Cache using
/// X.Y GB, Y.Z MB reclaimable on next cleanup").
///
/// Snapshot semantics: the values reflect the SQLite index at the
/// instant of the query. A racing PackStore write or tombstone moves
/// the truth out from under the response; UI should treat as
/// fresh-enough rather than authoritative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentCacheSizeAck {
    pub live_bytes: u64,
    pub tombstoned_bytes: u64,
}

/// Attachments roadmap Phase 8a: emitted at the end of every
/// retention-window eviction sweep (startup, post-sync, window-shrink).
/// `MustDeliver`: harness scripts and future UI hooks await this for a
/// deterministic completion signal. The `trigger` field lets observers
/// distinguish which pass fired.
///
/// `superseded = true` means a later trigger (typically a follow-up
/// window-shrink) bumped the eviction epoch mid-sweep, so this pass
/// bailed before draining its window. The next pass's
/// `EvictionCompleted` will reflect the up-to-date window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvictionCompleted {
    pub service_generation: u32,
    pub trigger: String,
    pub blobs_tombstoned: u64,
    pub pages_walked: u64,
    pub superseded: bool,
}

impl crate::notification::WithGeneration for EvictionCompleted {
    fn generation(&self) -> u32 {
        self.service_generation
    }
    fn set_generation(&mut self, generation: u32) {
        self.service_generation = generation;
    }
}

/// Attachments roadmap Phase 8c: `attachment.clear_cache` request
/// body. No parameters; the action is global "tombstone every live
/// blob and reclaim the bytes".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AttachmentClearCacheParams {}

/// Attachments roadmap Phase 8c: `attachment.clear_cache` ack.
///
/// `blobs_tombstoned` is the count of attachment_blobs rows the
/// bulk-tombstone UPDATE flipped from `tombstoned_at IS NULL` to a
/// timestamp. `bytes_reclaimed` is the post-tombstone GC pass's
/// physical-bytes-freed count - what `size_breakdown().tombstoned`
/// dropped by. Both can be zero (already-cleared cache).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentClearCacheAck {
    pub blobs_tombstoned: u64,
    pub bytes_reclaimed: u64,
}

/// Attachments roadmap Phase 8b: physical GC pack-repack completion.
/// `MustDeliver`: harness scripts and the future cache-size UI await
/// this. Fires once per GC pass (startup or post-eviction chain).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcCompleted {
    pub service_generation: u32,
    pub trigger: String,
    pub packs_compacted: u32,
    pub blobs_dropped: u64,
    pub bytes_reclaimed: u64,
}

impl crate::notification::WithGeneration for GcCompleted {
    fn generation(&self) -> u32 {
        self.service_generation
    }
    fn set_generation(&mut self, generation: u32) {
        self.service_generation = generation;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_round_trip() {
        let original = AttachmentFetchParams {
            account_id: "acct-1".into(),
            message_id: "msg-1".into(),
            attachment_id: "att-1".into(),
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: AttachmentFetchParams = serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn ack_round_trip() {
        let original = AttachmentFetchAck {
            content_hash: "deadbeefcafebabe".into(),
            size_bytes: 12345,
            relative_path: "attachment_fetch_tmp/deadbeefcafebabe-deadbeef".into(),
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: AttachmentFetchAck = serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }
}
