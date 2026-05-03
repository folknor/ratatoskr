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

/// Serialize and enqueue a `boot.progress` notification onto the outbound
/// writer queue. Best-effort: a `try_send` failure is logged at warn and
/// dropped. The OUTBOUND_QUEUE_CAP (1024) far exceeds the bounded number of
/// boot-sequence emissions, so a drop here means something is genuinely
/// wrong with the writer task or the consumer; the boot sequence proceeds
/// either way.
///
/// Phase 1.5 commit 5 lands the helper. Commits 6 through 9 call it from
/// the per-phase boot work that those commits introduce.
pub(crate) fn emit(out_tx: &mpsc::Sender<Vec<u8>>, phase: BootPhase, message: Option<String>) {
    let notification = Notification::BootProgress(BootProgress {
        phase,
        message,
        service_generation: 0,
    });
    let bytes = match serialize_notification(&notification) {
        Ok(bytes) => bytes,
        Err(error) => {
            log::warn!("failed to serialize boot.progress: {error}");
            return;
        }
    };
    if let Err(error) = out_tx.try_send(bytes) {
        log::warn!("failed to enqueue boot.progress notification: {error}");
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
