//! `attachment.fetch` wire types.
//!
//! Cache-miss reads are Service-side: the UI ships `(account_id,
//! message_id, attachment_id)` and gets back the cache-relative path
//! the Service guarantees is present on disk. Bytes never cross the
//! IPC; the on-disk file is the contract.
//!
//! On the current flat cache (`attachment_cache/<content_hash>`), the
//! UI's open fd is the pin against eviction: `unlink` is fd-safe on
//! Linux, so a UI process holding the file open survives a
//! concurrent eviction sweep and the kernel reclaims space when the
//! last fd closes.
//!
//! When `PackStore` lands (attachments roadmap Phase 3), blobs live
//! inside pack files at `(pack_id, offset, length)`; there is no
//! user-readable file at a relative path. The handler bridges this
//! with **per-fetch transient extraction**: it copies the requested
//! blob from its pack to
//! `<app_data>/attachment_fetch_tmp/<content_hash>-<request_id>`
//! and returns that path in the ack. The UI opens the tmp file
//! positionally; the open fd remains the pin, just against the tmp
//! file rather than the pack. An idle cleanup pass reaps tmp entries
//! older than 10 minutes. No lease IDs.

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
