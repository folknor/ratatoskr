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
    /// Non-contention IO failure on the instance lock file (disk full,
    /// permission flip on the data dir, etc.). Distinct from the generic
    /// runtime exit-code-1 (`UnexpectedExit { Some(1) }`) so the UI can
    /// surface a path-specific message naming `<app_data>/ratatoskr.lock`
    /// rather than the catch-all "Service exited unexpectedly".
    LockIoFailure = 74,
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
            74 => Some(Self::LockIoFailure),
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
    /// Map a child process's exit code to a classification.
    ///
    /// Always produces a classification - the caller decides whether to
    /// surface it as fatal. The two production call sites:
    /// - `elevate_initial_boot_error` (pre-BootReady): a child that exited
    ///   before answering boot.ready. Code 0 here is `UnexpectedExit { Some(0) }`
    ///   per scope item 7 of phase-1.5-plan.md - a Service that exits 0
    ///   before answering boot.ready is broken.
    /// - `handle_crash` crashloop branch (post-BootReady): the Service
    ///   crashed unexpectedly after going Ready. Reaching this path
    ///   implies the crash was not a clean shutdown (clean shutdown sets
    ///   `is_shutting_down = true`, which `handle_crash` checks for and
    ///   bails before reaching the classification step). Code 0 here is
    ///   still `UnexpectedExit { Some(0) }` and still surfaces fatally,
    ///   because a Service that exited 0 without going through
    ///   `client.shutdown()` is broken.
    ///
    /// The "do not call after a clean shutdown" contract lives at the call
    /// sites, not here: this function is total over `Option<i32>`.
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
    OpeningBodyAndInlineStores,
    OpeningSearchIndex,
    RunningInvariantPass,
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
    OpeningBodyAndInlineStores,
    OpeningSearchIndex,
    RunningInvariantPass,
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
            Self::OpeningBodyAndInlineStores => BootPhaseKind::OpeningBodyAndInlineStores,
            Self::OpeningSearchIndex => BootPhaseKind::OpeningSearchIndex,
            Self::RunningInvariantPass => BootPhaseKind::RunningInvariantPass,
        }
    }
}

/// Payload of the `boot.progress` notification.
///
/// `service_generation` is **wire-reserved for the UI**: the Service emits
/// `0`, the UI's reader task overwrites it with its current
/// `ServiceClient::current_generation` at enqueue time, and the App's
/// notification dispatcher drops payloads whose tag does not match the live
/// generation. This closes the cross-respawn race where a dying Service can
/// enqueue a stale `BootProgress` (or in Phase 2+, a stale `MustDeliver`
/// notification) that arrives after the replacement Service is established.
/// The field defaults to 0 on deserialize so a future Service that omits it
/// still parses cleanly; the UI's reader task always overwrites the value
/// regardless of what arrived on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootProgress {
    pub phase: BootPhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default)]
    pub service_generation: u32,
}

/// Response payload for `boot.ready`. Phase 1.5 carries schema state plus the
/// migrations-applied count; Phase 3 will extend with `tantivy_ready` and
/// Phase 6 with the writers-initialized confirmation. Each future addition is
/// one new field, not a new shape.
///
/// `recovery_warnings` carries non-fatal failures from the boot recovery
/// steps (pending-ops recovery, queued-drafts sweep, thread-participants
/// backfill). These are documented in `phase-1.5-plan.md` scope items 4 / 5
/// / 5a as state-repair, not correctness gates - a failure leaves the DB in
/// the same state the previous boot left it in. Surfacing them on the
/// response gives the UI an observable signal for diagnostics
/// ("boot ok but recovery had issues; check the logs") instead of forcing
/// operators to grep `service.<pid>.log` for warn lines.
///
/// Empty Vec on a healthy boot. Skipped on the wire when empty so the
/// payload doesn't grow on the common path. Each entry is a short
/// human-readable label (e.g., "pending-ops recovery", "queued-drafts
/// sweep") - the actual error detail stays in the rolling log file under
/// the sensitive-value policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootReadyResponse {
    pub ready: bool,
    pub schema_version: u32,
    pub migrations_applied: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recovery_warnings: Vec<String>,
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
            BootExitCode::LockIoFailure,
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
            BootExitCode::LockIoFailure,
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

    /// Code 0 is `UnexpectedExit { Some(0) }` regardless of whether the
    /// caller is reasoning about pre- or post-BootReady. The "post-BootReady
    /// clean shutdown" case is filtered at the call sites (handle_crash
    /// bails on `is_shutting_down`); reaching from_exit_code with code 0
    /// post-BootReady means the Service exited 0 unexpectedly, which is
    /// still broken. Locks in the contract from the doc-comment - without
    /// this test, a refactor that special-cased Some(0) to "no
    /// classification" would silently break the post-BootReady crashloop
    /// branch's terminal-failure surfacing.
    #[test]
    fn boot_classification_zero_is_unexpected_exit_in_both_phases() {
        // Pre-BootReady code 0: per scope item 7 of phase-1.5-plan.md, a
        // Service that exits 0 before answering boot.ready is broken.
        let pre = BootClassification::from_exit_code(Some(0));
        assert_eq!(pre, BootClassification::UnexpectedExit { code: Some(0) });

        // Post-BootReady code 0: the same classification is produced. The
        // clean-shutdown filter lives at the call site, not in this
        // function. Both phases share the encoding so terminal-failure
        // surfacing is uniform across the spawn flow.
        let post = BootClassification::from_exit_code(Some(0));
        assert_eq!(post, BootClassification::UnexpectedExit { code: Some(0) });
        assert_eq!(pre, post);
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
            recovery_warnings: Vec::new(),
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: BootReadyResponse = serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    /// `recovery_warnings` round-trips when populated and is omitted from
    /// the wire payload when empty (the common-case healthy boot path).
    #[test]
    fn boot_ready_response_recovery_warnings_round_trip() {
        let with_warnings = BootReadyResponse {
            ready: true,
            schema_version: 100,
            migrations_applied: 0,
            recovery_warnings: vec![
                "pending-ops recovery".to_string(),
                "thread-participants backfill".to_string(),
            ],
        };
        let json = serde_json::to_value(&with_warnings).expect("serialize");
        let recovered: BootReadyResponse = serde_json::from_value(json).expect("deserialize");
        assert_eq!(with_warnings, recovered);

        let empty = BootReadyResponse {
            ready: true,
            schema_version: 100,
            migrations_applied: 0,
            recovery_warnings: Vec::new(),
        };
        let json_str = serde_json::to_string(&empty).expect("serialize");
        assert!(
            !json_str.contains("recovery_warnings"),
            "empty recovery_warnings must be omitted from the wire payload, got: {json_str}",
        );
    }

    /// Older Service builds omit the field entirely. Defaulting to an empty
    /// Vec keeps the deserialize-side robust against forward/backward
    /// shipping skew during a UI/Service upgrade.
    #[test]
    fn boot_ready_response_recovery_warnings_default_when_absent() {
        let json = serde_json::json!({
            "ready": true,
            "schema_version": 100,
            "migrations_applied": 0
        });
        let recovered: BootReadyResponse = serde_json::from_value(json).expect("deserialize");
        assert!(recovered.recovery_warnings.is_empty());
    }
}
