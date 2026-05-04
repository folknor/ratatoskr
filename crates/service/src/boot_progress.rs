//! `boot.progress` notification emission helpers.
//!
//! The Service emits `BootProgress` notifications during the long boot
//! sequence (key load, DB open + migrations, pending-ops recovery, queued-
//! drafts sweep, thread-participants backfill) so the UI splash can render
//! progress while migrations run.
//!
//! Wire format: `{"jsonrpc":"2.0","method":"boot.progress","params":<BootProgress>}`.
//! Each notification is one newline-terminated frame.
//!
//! `service_generation` on the wire payload is always `0` from the Service's
//! perspective; the UI's reader task overwrites it with the current
//! generation at enqueue time so the App's notification dispatcher can drop
//! stale notifications from a dying Service incarnation. The Service has no
//! view of the UI's generation counter (per scope item 20 of
//! `phase-1.5-plan.md`).

use serde_json::Value;
use service_api::{BootPhase, BootProgress, MAX_FRAME_BYTES, Notification};
use std::io;
use tokio::sync::mpsc;

/// Awaitable wrapper around the outbound writer mpsc that serializes
/// + frames a `Notification` before sending.
///
/// Phase 3 task 4 introduces this so the search writer task can drive
/// `MustDeliver` notifications with `tokio::time::timeout` against
/// `NotificationSender::send` (the H5 30 s send-deadline) without
/// re-implementing the serialize step.
///
/// Cheap `Clone` (the inner `mpsc::Sender` is `Arc`-backed). Mirrors
/// the existing `try_send`-style helpers above for callers that need
/// awaitable backpressure.
#[derive(Clone)]
pub struct NotificationSender {
    out_tx: mpsc::Sender<Vec<u8>>,
}

impl NotificationSender {
    pub fn new(out_tx: mpsc::Sender<Vec<u8>>) -> Self {
        Self { out_tx }
    }

    /// Awaitable send. Backpressure-aware (`mpsc::Sender::send`).
    pub async fn send(
        &self,
        notification: Notification,
    ) -> Result<(), mpsc::error::SendError<Vec<u8>>> {
        let bytes = match serialize_notification(&notification) {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!(
                    "failed to serialize {}: {error}",
                    notification.method_name()
                );
                return Ok(());
            }
        };
        self.out_tx.send(bytes).await
    }
}

/// Serialize and enqueue a `boot.progress` notification onto the outbound
/// writer queue. Best-effort: a `try_send` failure is logged at warn and
/// dropped.
///
/// Why try_send-and-drop is safe in Phase 1.5: the dispatch loop's
/// OUTBOUND_QUEUE_CAP is 1024 entries, and the entire Phase 1.5 boot
/// sequence emits a bounded number of frames (one per BootPhase, plus
/// 2*N for an N-migration run via the before/after-COMMIT callback).
/// That assumption holds today for any plausible migration count. If a
/// future phase adds a chatty `MustDeliver` notification during boot,
/// or an unbounded-size loop emits per-row progress, this helper must
/// switch to `send().await` (with a path back to the dispatch loop's
/// out_tx) - or the queue cap must grow. The contract:
/// `OUTBOUND_QUEUE_CAP` must remain much larger than the total number of
/// Phase-1.5 boot.progress frames. Land any new emitter behind a
/// regression test that verifies this still holds.
pub(crate) fn emit(out_tx: &mpsc::Sender<Vec<u8>>, phase: BootPhase, message: Option<String>) {
    let notification = Notification::BootProgress(BootProgress {
        phase,
        message,
        service_generation: 0,
    });
    enqueue_notification(out_tx, &notification);
}

/// Serialize a notification and enqueue it on the outbound writer
/// queue with `try_send` semantics.
///
/// Lifted out of `emit` so that other Service-side notification
/// emitters (the relocated action service's `IpcProgressReporter`
/// in `crate::progress`, future sync paths) share the same
/// serialization + enqueue logic. Same caveat as `emit`: the
/// `try_send` drop-on-full policy is safe today only because the
/// total Phase 1.5 frame count fits comfortably under
/// `OUTBOUND_QUEUE_CAP`. Phase 2's `MustDeliver` notifications
/// (action.operation_outcome, action.completed) MUST NOT use this
/// helper and must instead be emitted via an awaited `send`; the
/// drop-on-full policy is incompatible with `MustDeliver` semantics.
/// The action handler+worker (Phase 2 task 9) lands the awaited-send
/// path. Coalesce-class notifications (boot.progress, sync.progress)
/// can keep using this helper because the queue's coalesce policy
/// already collapses repeats, so a try_send drop in the rare full-
/// queue case is no worse than a coalesce hit.
pub(crate) fn enqueue_notification(
    out_tx: &mpsc::Sender<Vec<u8>>,
    notification: &Notification,
) {
    let method_name = notification.method_name();
    let bytes = match serialize_notification(notification) {
        Ok(bytes) => bytes,
        Err(error) => {
            log::warn!("failed to serialize {method_name}: {error}");
            return;
        }
    };
    if let Err(error) = out_tx.try_send(bytes) {
        log::warn!("failed to enqueue {method_name} notification: {error}");
    }
}

/// Enqueue a `MustDeliver`-class notification onto the outbound writer
/// queue, blocking on full-queue backpressure until the receiver
/// drains a slot.
///
/// `MustDeliver` semantics (per `Notification::class()` in service-api)
/// are incompatible with `try_send`'s drop-on-full policy: dropping an
/// `OperationOutcome` desyncs the UI's optimistic state from the
/// journal (the outcome row has `outcome IS NOT NULL` so
/// `replay_unemitted` skips it on respawn); dropping an
/// `ActionCompleted` permanently leaks the UI's `pending_action_plans`
/// entry (the post-respawn reconcile only sweeps `AckUnknown`). The
/// caller MUST be on an async path; the awaited send is the contract.
///
/// A failure here means the receiver is gone, which only happens on
/// Service teardown - log at warn and return; the dispatch loop is
/// already on its way out.
pub(crate) async fn send_must_deliver_notification(
    out_tx: &mpsc::Sender<Vec<u8>>,
    notification: &Notification,
) {
    let method_name = notification.method_name();
    let bytes = match serialize_notification(notification) {
        Ok(bytes) => bytes,
        Err(error) => {
            log::warn!("failed to serialize {method_name}: {error}");
            return;
        }
    };
    if let Err(error) = out_tx.send(bytes).await {
        log::warn!(
            "MustDeliver {method_name} dropped (receiver gone): {error}"
        );
    }
}

/// Build a single newline-terminated JSON-RPC frame for a `Notification`.
/// `Notification` serializes as `{"method":..., "params":...}` via its
/// `tag = "method", content = "params"` shape; we splice in the JSON-RPC
/// 2.0 envelope on top.
fn serialize_notification(notification: &Notification) -> io::Result<Vec<u8>> {
    let mut value = serde_json::to_value(notification).map_err(io::Error::other)?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| io::Error::other("notification did not serialize to object"))?;
    object.insert("jsonrpc".to_string(), Value::String("2.0".to_string()));
    let mut bytes = serde_json::to_vec(&value).map_err(io::Error::other)?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "serialized notification frame exceeds maximum size",
        ));
    }
    bytes.push(b'\n');
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_api::{ParsedServiceMessage, parse_service_message};

    fn parse_frame(bytes: &[u8]) -> ParsedServiceMessage {
        let line = std::str::from_utf8(bytes).expect("utf-8 frame");
        assert!(
            line.ends_with('\n'),
            "frame must be newline-terminated, got {line:?}"
        );
        parse_service_message(line.trim_end_matches('\n')).expect("parse frame")
    }

    #[tokio::test]
    async fn emit_enqueues_a_well_formed_boot_progress_frame() {
        let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(8);
        emit(
            &out_tx,
            BootPhase::Migrating {
                current: 3,
                total: 10,
            },
            Some("Applying migration 3 of 10".to_string()),
        );
        let bytes = out_rx.recv().await.expect("frame on out_rx");
        let parsed = parse_frame(&bytes);
        match parsed {
            ParsedServiceMessage::Notification(Notification::BootProgress(progress)) => {
                assert_eq!(
                    progress.phase,
                    BootPhase::Migrating {
                        current: 3,
                        total: 10,
                    },
                );
                assert_eq!(
                    progress.message.as_deref(),
                    Some("Applying migration 3 of 10")
                );
                // Service-side always emits 0; the UI overwrites at enqueue.
                assert_eq!(progress.service_generation, 0);
            }
            other => panic!("expected BootProgress notification, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn emit_serialises_unit_phases_without_a_message_field() {
        let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(8);
        emit(&out_tx, BootPhase::LoadingKey, None);
        let bytes = out_rx.recv().await.expect("frame on out_rx");
        let line = std::str::from_utf8(&bytes).expect("utf-8");
        // BootProgress::message is `skip_serializing_if = "Option::is_none"`,
        // so absent messages must not appear in the wire payload.
        assert!(
            !line.contains("\"message\""),
            "absent message must be omitted, got: {line}"
        );
        let parsed = parse_frame(&bytes);
        match parsed {
            ParsedServiceMessage::Notification(Notification::BootProgress(progress)) => {
                assert_eq!(progress.phase, BootPhase::LoadingKey);
                assert_eq!(progress.message, None);
            }
            other => panic!("expected BootProgress notification, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn emit_drops_when_outbound_queue_is_full() {
        // Queue capacity 1; pre-fill it. The next emit must not panic and
        // must not block (the helper uses try_send by design).
        let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(1);
        out_tx.try_send(vec![0u8]).expect("pre-fill");
        emit(&out_tx, BootPhase::OpeningDatabase, None);
        // Drain the pre-fill so the receiver mpsc isn't dropped before the
        // sender finishes its try_send.
        let _ = out_rx.recv().await.expect("pre-fill drained");
        // No second frame should arrive - the emit was dropped.
        let nothing = tokio::time::timeout(std::time::Duration::from_millis(50), out_rx.recv())
            .await;
        assert!(nothing.is_err(), "no frame should follow the dropped emit");
    }

    #[test]
    fn serialize_notification_produces_jsonrpc_envelope() {
        let notification = Notification::BootProgress(BootProgress {
            phase: BootPhase::RecoveringPendingOps,
            message: None,
            service_generation: 0,
        });
        let bytes = serialize_notification(&notification).expect("serialize");
        let line = std::str::from_utf8(&bytes).expect("utf-8");
        assert!(
            line.contains("\"jsonrpc\":\"2.0\""),
            "frame must carry jsonrpc 2.0 envelope, got: {line}"
        );
        assert!(
            line.contains("\"method\":\"boot.progress\""),
            "frame must carry method=boot.progress, got: {line}"
        );
    }
}
