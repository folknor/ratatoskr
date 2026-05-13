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
