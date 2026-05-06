//! `attachment.fetch` wire types (Phase 6b).
//!
//! Cache-miss reads relocate Service-side: the UI ships `(account_id,
//! message_id, attachment_id)` and gets back the cache-relative path
//! the Service guarantees is present on disk. Bytes never cross the
//! IPC (phase-1.5-plan.md backpressure policy); the cache file is
//! the contract.
//!
//! On Linux, `unlink` does not invalidate already-open file
//! descriptors. A UI process that has the cache file open survives a
//! concurrent eviction sweep cleanly: the UI keeps reading from its
//! open fd; the file is removed from the directory; the kernel
//! reclaims disk space when the last fd closes. The race that pack-
//! aware reads have (in-pack offset moved by repack, frame-orphan GC
//! marking unreachable) does not exist on the flat cache because
//! each file is one blob and `unlink` is fd-safe. The read pin is
//! the open fd itself - no lease IDs.
//!
//! When Phase 1a lands (pack store + `pack_index`), the lease design
//! returns: pack-aware reads cannot use "open fd" as the pin because
//! eviction may rewrite the *file* and the UI's offset becomes
//! meaningless. The future revision pass adds `lease_id` +
//! `PackStore::get_with_lease` + active-lease counter on
//! `pack_index`. Until then, no leases.

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
/// `relative_path` is rooted at `<app_data>/`, matching the existing
/// `attachment_cache::write_cached` return shape
/// (`attachment_cache/<content_hash>`). UI re-opens the file
/// positionally; the open fd is the pin against eviction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentFetchAck {
    pub content_hash: String,
    pub size_bytes: u64,
    pub relative_path: String,
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
            relative_path: "attachment_cache/deadbeefcafebabe".into(),
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: AttachmentFetchAck = serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }
}
