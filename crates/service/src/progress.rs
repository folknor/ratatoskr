//! Service-side `db::ProgressReporter` impl that posts events onto the
//! outbound IPC writer queue as `Notification::SyncProgress` frames.
//!
//! Phase 2 introduces this so the relocated action service (task 9)
//! and the Phase 3 sync paths can keep their existing
//! `&dyn ProgressReporter` plumbing without knowing about the IPC
//! envelope. Each emit becomes one wire notification keyed for
//! per-account latest-wins coalescing.

use db::progress::ProgressReporter;
use service_api::{Notification, SyncProgress};
use tokio::sync::mpsc;

use crate::boot_progress::enqueue_notification;

/// `ProgressReporter` backed by the IPC outbound queue.
///
/// Each instance is bound to a single `account_id`. Emissions wrap
/// the `(event_name, json)` pair into a `Notification::SyncProgress`
/// carrying the bound account id and enqueue via the same
/// `try_send` path that `boot.progress` uses. The queue's
/// `Coalesce { key: SyncProgress(account_id) }` policy collapses
/// repeats per account so a chatty per-row sync emission cannot
/// flood the wire.
///
/// `service_generation` is always `0` from this side; the UI's
/// reader task overwrites it with the live generation at enqueue
/// time so the App's notification dispatcher can drop stale
/// notifications from a dying Service incarnation (per Phase 1.5
/// scope item 20).
#[derive(Clone)]
pub struct IpcProgressReporter {
    out_tx: mpsc::Sender<Vec<u8>>,
    account_id: String,
}

impl IpcProgressReporter {
    pub fn new(out_tx: mpsc::Sender<Vec<u8>>, account_id: impl Into<String>) -> Self {
        Self {
            out_tx,
            account_id: account_id.into(),
        }
    }
}

impl ProgressReporter for IpcProgressReporter {
    fn emit_json(&self, event_name: &str, json: serde_json::Value) {
        let notification = Notification::SyncProgress(SyncProgress {
            account_id: self.account_id.clone(),
            event_name: event_name.to_string(),
            payload: json,
            service_generation: 0,
        });
        enqueue_notification(&self.out_tx, &notification);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_api::{ParsedServiceMessage, parse_service_message};

    fn parse_frame(bytes: &[u8]) -> ParsedServiceMessage {
        let line = std::str::from_utf8(bytes).expect("utf-8 frame");
        assert!(line.ends_with('\n'), "frame must be newline-terminated");
        parse_service_message(line.trim_end_matches('\n')).expect("parse frame")
    }

    #[tokio::test]
    async fn emit_json_enqueues_a_well_formed_sync_progress_frame() {
        let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(8);
        let reporter = IpcProgressReporter::new(out_tx, "acc-1");
        reporter.emit_json("thread.imported", serde_json::json!({ "count": 42 }));
        let bytes = out_rx.recv().await.expect("frame on out_rx");
        match parse_frame(&bytes) {
            ParsedServiceMessage::Notification(Notification::SyncProgress(progress)) => {
                assert_eq!(progress.account_id, "acc-1");
                assert_eq!(progress.event_name, "thread.imported");
                assert_eq!(progress.payload, serde_json::json!({ "count": 42 }));
                assert_eq!(progress.service_generation, 0);
            }
            other => panic!("expected SyncProgress, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn emit_json_drops_when_outbound_queue_is_full() {
        let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(1);
        out_tx.try_send(vec![0u8]).expect("pre-fill");
        let reporter = IpcProgressReporter::new(out_tx, "acc-1");
        // Must not panic, must not block.
        reporter.emit_json("any", serde_json::json!({}));
        let _ = out_rx.recv().await.expect("pre-fill drained");
        // No second frame arrived (try_send dropped on full).
        let nothing = tokio::time::timeout(std::time::Duration::from_millis(50), out_rx.recv()).await;
        assert!(nothing.is_err());
    }

    #[tokio::test]
    async fn each_reporter_carries_its_own_account_id() {
        let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(8);
        let reporter_a = IpcProgressReporter::new(out_tx.clone(), "acc-a");
        let reporter_b = IpcProgressReporter::new(out_tx, "acc-b");
        reporter_a.emit_json("evt", serde_json::json!({}));
        reporter_b.emit_json("evt", serde_json::json!({}));
        let mut accounts = Vec::new();
        for _ in 0..2 {
            let bytes = out_rx.recv().await.expect("frame on out_rx");
            match parse_frame(&bytes) {
                ParsedServiceMessage::Notification(Notification::SyncProgress(progress)) => {
                    accounts.push(progress.account_id);
                }
                other => panic!("expected SyncProgress, got {other:?}"),
            }
        }
        accounts.sort();
        assert_eq!(accounts, vec!["acc-a".to_string(), "acc-b".to_string()]);
    }
}
