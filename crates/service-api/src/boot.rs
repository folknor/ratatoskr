use serde::{Deserialize, Serialize};

/// Fatal Service-side boot exit codes. Values are picked outside the clap
/// (=2), Rust panic (=101), and shell-signal (137 / 143) ranges so the UI's
/// `wait().status.code()` mapping cannot ambiguously confuse a real boot
/// failure with a runtime-induced exit. Variants are wire-stable; renumbering
/// breaks the cross-process contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(i32)]
pub enum BootExitCode {
    /// Service-side handshake/protocol mismatch detected before reaching
    /// `boot.ready`. Reserved; not emitted by Phase 1.5 since the existing
    /// `health.ping` version check covers protocol-version mismatch.
    HandshakeFailure = 70,
    /// Single-instance lock contended; another Service is holding the lock
    /// for this data dir.
    AnotherInstanceRunning = 71,
    /// Schema migration (or the velo-&gt;ratatoskr rename that precedes it)
    /// failed. Includes partial-rename recovery failures.
    MigrationFailure = 72,
    /// `ratatoskr.key` could not be loaded. Phase 1 silently fell back to a
    /// zero key; Phase 1.5 makes this fatal so auto-respawn never widens the
    /// window where data lands under the zero key.
    KeyLoadFailure = 73,
}

impl BootExitCode {
    pub fn as_i32(self) -> i32 {
        self as i32
    }

    pub fn from_i32(code: i32) -> Option<Self> {
        match code {
            70 => Some(Self::HandshakeFailure),
            71 => Some(Self::AnotherInstanceRunning),
            72 => Some(Self::MigrationFailure),
            73 => Some(Self::KeyLoadFailure),
            _ => None,
        }
    }
}

/// Classification the UI computes from the dying child's exit code.
///
/// The mapping (per `phase-1.5-plan.md` scope item 7):
/// * `code == 0` AND `BootReady` already observed: clean shutdown - no
///   classification produced (handled by the caller's running-state).
/// * `code == 0` AND `BootReady` not yet observed: `UnexpectedExit { code:
///   Some(0) }`. A Service that exits 0 before answering `boot.ready` is
///   broken.
/// * `code == None` (signal-killed / SIGABRT from `panic = "abort"`):
///   `UnexpectedExit { code: None }`.
/// * `code == Some(n)` matching a `BootExitCode`: `BootFailure { code }`.
/// * `code == Some(n)` not in the variant set: `UnexpectedExit { code:
///   Some(n) }`.
///
/// `BootFailure` is terminal: the UI logs and `iced::exit()`s without
/// respawning. `UnexpectedExit` is also terminal in Phase 1.5 (it covers
/// runtime crashes that the per-PID log naming would otherwise turn into a
/// directory full of one-second-apart logs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BootClassification {
    BootFailure { code: BootExitCode },
    UnexpectedExit { code: Option<i32> },
}

impl BootClassification {
    /// Compute the classification for a child that exited before its first
    /// `BootReady` was observed. Callers must NOT invoke this once `BootReady`
    /// has fired - a clean shutdown after readiness is not a boot failure.
    pub fn from_exit_code(code: Option<i32>) -> Self {
        match code {
            Some(value) => match BootExitCode::from_i32(value) {
                Some(boot_code) => Self::BootFailure { code: boot_code },
                None => Self::UnexpectedExit { code: Some(value) },
            },
            None => Self::UnexpectedExit { code: None },
        }
    }
}

/// Discriminant of `BootPhase`, used as the per-phase coalesce key. Each
/// variant collapses independently in the queue: `Migrating { 1, 10 }` and
/// `Migrating { 5, 10 }` collapse to the latest, but `LoadingKey` and
/// `OpeningDatabase` remain independent entries that retain wire order. A
/// single coalesce key would let the latest phase clobber unrendered earlier
/// phases and break the integration test that asserts ordered delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BootPhaseKind {
    LoadingKey,
    OpeningDatabase,
    Migrating,
    RecoveringPendingOps,
    SweepingQueuedDrafts,
    BackfillingThreadParticipants,
}

/// Phase markers emitted by the Service during the boot sequence so the UI
/// splash can render progress while migrations run.
///
/// `AcquiringLock` is deliberately absent: lock acquisition runs before the
/// writer task is alive, so the notification cannot be emitted; if the lock
/// contends, the Service exits with `BootExitCode::AnotherInstanceRunning`
/// instead, which the UI surfaces directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BootPhase {
    LoadingKey,
    OpeningDatabase,
    Migrating { current: u32, total: u32 },
    RecoveringPendingOps,
    SweepingQueuedDrafts,
    BackfillingThreadParticipants,
}

impl BootPhase {
    /// Discriminant used as the coalesce key. Keep this in sync with
    /// `BootPhaseKind` - exhaustive match keeps that contract compiler-
    /// enforced.
    pub fn coalesce_discriminant(&self) -> BootPhaseKind {
        match self {
            Self::LoadingKey => BootPhaseKind::LoadingKey,
            Self::OpeningDatabase => BootPhaseKind::OpeningDatabase,
            Self::Migrating { .. } => BootPhaseKind::Migrating,
            Self::RecoveringPendingOps => BootPhaseKind::RecoveringPendingOps,
            Self::SweepingQueuedDrafts => BootPhaseKind::SweepingQueuedDrafts,
            Self::BackfillingThreadParticipants => BootPhaseKind::BackfillingThreadParticipants,
        }
    }
}

/// Payload of the `boot.progress` notification.
///
/// `service_generation` is tagged on the UI side at reader-task enqueue time,
/// not by the Service. It exists so the App's notification dispatcher can
/// drop `BootProgress` (and any future `MustDeliver` payloads) emitted by a
/// dying Service whose new incarnation has already been spawned. The Service
/// emits `0` (or any value); the UI overwrites with its current
/// `service_generation` counter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootProgress {
    pub phase: BootPhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub service_generation: u32,
}

/// Response payload for `boot.ready`. Phase 1.5 carries schema state plus the
/// migrations-applied count; Phase 3 will extend with `tantivy_ready` and
/// Phase 6 with the writers-initialized confirmation. Each future addition is
/// one new field, not a new shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootReadyResponse {
    pub ready: bool,
    pub schema_version: u32,
    pub migrations_applied: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_exit_code_round_trips_through_i32() {
        for code in [
            BootExitCode::HandshakeFailure,
            BootExitCode::AnotherInstanceRunning,
            BootExitCode::MigrationFailure,
            BootExitCode::KeyLoadFailure,
        ] {
            assert_eq!(BootExitCode::from_i32(code.as_i32()), Some(code));
        }
        assert_eq!(BootExitCode::from_i32(0), None);
        assert_eq!(BootExitCode::from_i32(101), None);
        assert_eq!(BootExitCode::from_i32(137), None);
    }

    #[test]
    fn boot_exit_codes_are_outside_runtime_ranges() {
        // clap exits 2; rust panic = abort yields 101 on Linux; shell signals
        // 137 / 143. Keep those bands clear.
        for code in [
            BootExitCode::HandshakeFailure,
            BootExitCode::AnotherInstanceRunning,
            BootExitCode::MigrationFailure,
            BootExitCode::KeyLoadFailure,
        ] {
            let value = code.as_i32();
            assert_ne!(value, 0);
            assert_ne!(value, 2);
            assert_ne!(value, 101);
            assert_ne!(value, 137);
            assert_ne!(value, 143);
        }
    }

    #[test]
    fn boot_classification_maps_known_codes_to_boot_failure() {
        assert_eq!(
            BootClassification::from_exit_code(Some(73)),
            BootClassification::BootFailure {
                code: BootExitCode::KeyLoadFailure,
            },
        );
        assert_eq!(
            BootClassification::from_exit_code(Some(71)),
            BootClassification::BootFailure {
                code: BootExitCode::AnotherInstanceRunning,
            },
        );
    }

    #[test]
    fn boot_classification_maps_unknown_codes_to_unexpected_exit() {
        assert_eq!(
            BootClassification::from_exit_code(Some(0)),
            BootClassification::UnexpectedExit { code: Some(0) },
        );
        assert_eq!(
            BootClassification::from_exit_code(Some(42)),
            BootClassification::UnexpectedExit { code: Some(42) },
        );
        assert_eq!(
            BootClassification::from_exit_code(None),
            BootClassification::UnexpectedExit { code: None },
        );
    }

    #[test]
    fn boot_phase_coalesce_discriminant_is_exhaustive() {
        let cases = [
            (BootPhase::LoadingKey, BootPhaseKind::LoadingKey),
            (BootPhase::OpeningDatabase, BootPhaseKind::OpeningDatabase),
            (
                BootPhase::Migrating {
                    current: 3,
                    total: 10,
                },
                BootPhaseKind::Migrating,
            ),
            (
                BootPhase::RecoveringPendingOps,
                BootPhaseKind::RecoveringPendingOps,
            ),
            (
                BootPhase::SweepingQueuedDrafts,
                BootPhaseKind::SweepingQueuedDrafts,
            ),
            (
                BootPhase::BackfillingThreadParticipants,
                BootPhaseKind::BackfillingThreadParticipants,
            ),
        ];
        for (phase, expected) in cases {
            assert_eq!(phase.coalesce_discriminant(), expected);
        }
    }

    #[test]
    fn migrating_collapses_under_kind_but_other_phases_do_not() {
        // Two Migrating updates with different (current, total) collapse to
        // the same kind.
        let a = BootPhase::Migrating {
            current: 1,
            total: 10,
        };
        let b = BootPhase::Migrating {
            current: 5,
            total: 10,
        };
        assert_eq!(a.coalesce_discriminant(), b.coalesce_discriminant());

        // LoadingKey and OpeningDatabase resolve to distinct kinds so they
        // are NOT collapsed together; the queue keeps them as independent
        // entries that retain wire order.
        assert_ne!(
            BootPhase::LoadingKey.coalesce_discriminant(),
            BootPhase::OpeningDatabase.coalesce_discriminant(),
        );
    }

    #[test]
    fn boot_progress_round_trips_through_serde() {
        let original = BootProgress {
            phase: BootPhase::Migrating {
                current: 2,
                total: 7,
            },
            message: Some("Applying migration 2 of 7".to_string()),
            service_generation: 4,
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: BootProgress = serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn boot_progress_omits_absent_message_field() {
        let payload = BootProgress {
            phase: BootPhase::LoadingKey,
            message: None,
            service_generation: 0,
        };
        let json = serde_json::to_string(&payload).expect("serialize");
        assert!(
            !json.contains("message"),
            "absent message must be omitted from the wire payload, got: {json}",
        );
    }

    #[test]
    fn boot_ready_response_round_trips_through_serde() {
        let original = BootReadyResponse {
            ready: true,
            schema_version: 100,
            migrations_applied: 1,
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: BootReadyResponse = serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }
}
