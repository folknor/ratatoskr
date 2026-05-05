//! Sync wire types.
//!
//! Phase 3 of `docs/service/phase-3-plan.md` relocates JMAP delta sync
//! into the Service. The UI dispatches `sync.start_account` and waits
//! for a `sync.completed` notification correlated by `SyncRunId`. The
//! Service may also emit `index.committed` notifications when the
//! Tantivy writer commits a batch; the UI uses those to drive a
//! debounced `IndexReader` reload.
//!
//! Wire types live here (not `crates/core/`) so `service-api` stays
//! lightweight - core depends on providers/search/store and pulling
//! that graph into every consumer of `service-api` would defeat the
//! "wire crate" framing. Conversion happens at the app/service edge.
//!
//! `SyncResult` on the wire is `Completed | Cancelled | Failed(String)`
//! only. UI-side `ClientError::ServiceCrashed` is *never* on the wire -
//! it's a synthesized condition the UI surfaces when the Service
//! pipe breaks. `AlreadyInFlight` is also not on the wire - the
//! `SyncStartAck.already_in_flight: bool` flag carries that signal.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::notification::WithGeneration;

// ---------------------------------------------------------------------------
// Identifiers
// ---------------------------------------------------------------------------

/// 128-bit time-ordered identifier for a single sync run.
///
/// UUIDv7: high 48 bits are a millisecond timestamp, the rest is
/// random. Time-ordered insertion gives the per-account map's
/// debugging-trace good locality, and the 128-bit width survives a
/// Service restart resetting any in-process counter.
///
/// One `SyncRunId` corresponds to exactly one runner-task lifetime:
/// `start_account` either returns the existing run's id (when
/// `already_in_flight = true`) or generates a fresh one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SyncRunId(pub Uuid);

impl SyncRunId {
    /// Generate a fresh time-ordered run id. Service-side only;
    /// the UI never constructs one.
    #[must_use]
    pub fn new_v7() -> Self {
        Self(Uuid::now_v7())
    }
}

impl std::fmt::Display for SyncRunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

// ---------------------------------------------------------------------------
// Request params (sync.start_account / sync.cancel_account)
// ---------------------------------------------------------------------------

/// `sync.start_account` request body. The Service either spawns a fresh
/// runner for this account or returns the existing run's id with
/// `already_in_flight = true`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncStartAccountParams {
    pub account_id: String,
}

/// Synchronous response to `sync.start_account`. The handler returns
/// within microseconds: it acquires the per-account map lock, checks
/// for an existing in-flight runner, spawns one if needed, and acks.
/// The actual sync work runs in the spawned task; per the
/// handler/worker split established in Phase 2, the JSON-RPC ack is
/// sent before the runner does any work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncStartAck {
    pub account_id: String,
    /// The run id under which `sync.completed` will eventually be
    /// emitted. Multiple callers receiving the same id (the
    /// `already_in_flight` case) all subscribe to the same
    /// completion notification.
    pub run_id: SyncRunId,
    /// `true` if a runner for this account was already executing when
    /// the request arrived. The caller may still subscribe to the
    /// returned `run_id`'s completion - both the original caller and
    /// the duplicate caller resolve from the same broadcast.
    pub already_in_flight: bool,
}

/// `sync.cancel_account` request body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncCancelAccountParams {
    pub account_id: String,
}

/// Synchronous response to `sync.cancel_account`. Always returns
/// promptly; the actual cancellation propagates through the runner's
/// `CancellationToken` and may take up to 5 seconds on a healthy
/// connection (longer if a single in-flight network call is stuck).
///
/// `run_id` is `Some` if a sync run was in flight (the caller can then
/// subscribe to that run's `sync.completed` to know when cancellation
/// completed); `None` if no sync runner was active.
///
/// Phase 5 task 9: `calendar_run_id` is the calendar runner's run id
/// when `handle_cancel_account` piggybacks calendar cancel alongside
/// sync cancel (account-deletion path). `None` when no calendar runner
/// was active OR when the request didn't piggyback calendar cancel
/// (e.g., the explicit-request path uses `calendar.cancel_account_sync`
/// directly and ignores this field's content). The
/// `cancel_and_await` UI path uses both `run_id` and `calendar_run_id`
/// to await both terminal completions before issuing the DB DELETE.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncCancelAck {
    pub account_id: String,
    pub run_id: Option<SyncRunId>,
    pub was_in_flight: bool,
    /// Phase 5 task 9: piggyback calendar cancel run id. Defaults to
    /// `None` if the field is missing on the wire (older Service or
    /// the call didn't piggyback).
    #[serde(default)]
    pub calendar_run_id: Option<crate::calendar::CalendarRunId>,
}

// ---------------------------------------------------------------------------
// Notification payloads (sync.completed / index.committed)
// ---------------------------------------------------------------------------

/// Final result of a sync run. Wire-narrow: `ServiceCrashed` and
/// `AlreadyInFlight` are deliberately omitted - the former is a
/// synthesized UI-side `ClientError`, and the latter is a flag on
/// `SyncStartAck`.
///
/// `Failed` carries the error message verbatim from the runner so the
/// UI can render it in toasts. The string is not stable for branching
/// or substring matching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum SyncResult {
    Completed,
    Cancelled,
    Failed(String),
}

/// Emitted once per run after the runner has finished (or panicked,
/// via the supervisor's synthetic emission). `MustDeliver` notification
/// (per `docs/service/problem-statement.md` § IPC "notification class
/// taxonomy"): a dropped completion would leave the UI's pending future
/// hanging forever.
///
/// Routing uses `run_id` as the correlation key so multiple waiters per
/// run (user-initiated `start_sync` + an account-delete
/// `cancel_and_await` + a duplicate tick-driven kick) all resolve
/// cleanly via the `pending_syncs` broadcast map.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncCompleted {
    pub account_id: String,
    pub run_id: SyncRunId,
    pub result: SyncResult,
    /// Cross-respawn drop tag. Service emits 0; UI's reader task
    /// overwrites at enqueue with `current_generation()`.
    pub service_generation: u32,
}

impl WithGeneration for SyncCompleted {
    fn generation(&self) -> u32 {
        self.service_generation
    }
    fn set_generation(&mut self, generation: u32) {
        self.service_generation = generation;
    }
}

/// Emitted by the search-writer task after a successful Tantivy commit.
///
/// `MustDeliver` (with a 30 s send-deadline degrade per Phase 3 plan
/// H5): a dropped commit notification leaves the UI's `IndexReader`
/// stale until the next commit arrives. The advisory nature of the
/// signal (next commit catches up) is what makes the deadline-on-send
/// safe for forward progress; the wire-level taxonomy still classifies
/// as `MustDeliver` until a future `BestEffort` class is added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexCommitted {
    /// Cross-respawn drop tag. Captured by the writer task at spawn
    /// time and emitted on every commit. UI's reader task overwrites
    /// at enqueue.
    pub service_generation: u32,
}

impl WithGeneration for IndexCommitted {
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
    fn sync_run_id_round_trips_through_serde() {
        let id = SyncRunId::new_v7();
        let json = serde_json::to_value(id).expect("serialize");
        let recovered: SyncRunId = serde_json::from_value(json).expect("deserialize");
        assert_eq!(id, recovered);
    }

    #[test]
    fn sync_run_id_v7_is_time_ordered() {
        let a = SyncRunId::new_v7();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = SyncRunId::new_v7();
        assert!(a.0 < b.0, "expected v7 timestamps to monotonically advance");
    }

    #[test]
    fn sync_start_ack_round_trips_through_serde() {
        let ack = SyncStartAck {
            account_id: "acc-1".into(),
            run_id: SyncRunId::new_v7(),
            already_in_flight: true,
        };
        let json = serde_json::to_value(&ack).expect("serialize");
        let recovered: SyncStartAck = serde_json::from_value(json).expect("deserialize");
        assert_eq!(ack, recovered);
    }

    #[test]
    fn sync_cancel_ack_round_trips_through_serde() {
        let cases = [
            SyncCancelAck {
                account_id: "acc-1".into(),
                run_id: Some(SyncRunId::new_v7()),
                was_in_flight: true,
                calendar_run_id: Some(crate::calendar::CalendarRunId::new_v7()),
            },
            SyncCancelAck {
                account_id: "acc-2".into(),
                run_id: None,
                was_in_flight: false,
                calendar_run_id: None,
            },
        ];
        for ack in cases {
            let json = serde_json::to_value(&ack).expect("serialize");
            let recovered: SyncCancelAck = serde_json::from_value(json).expect("deserialize");
            assert_eq!(ack, recovered);
        }
    }

    #[test]
    fn sync_result_round_trips_through_serde() {
        let cases = [
            SyncResult::Completed,
            SyncResult::Cancelled,
            SyncResult::Failed("provider 503".into()),
        ];
        for result in cases {
            let json = serde_json::to_value(&result).expect("serialize");
            let recovered: SyncResult = serde_json::from_value(json).expect("deserialize");
            assert_eq!(result, recovered);
        }
    }

    #[test]
    fn sync_completed_round_trips_through_serde() {
        let completed = SyncCompleted {
            account_id: "acc-1".into(),
            run_id: SyncRunId::new_v7(),
            result: SyncResult::Completed,
            service_generation: 7,
        };
        let json = serde_json::to_value(&completed).expect("serialize");
        let recovered: SyncCompleted = serde_json::from_value(json).expect("deserialize");
        assert_eq!(completed, recovered);
    }

    #[test]
    fn index_committed_round_trips_through_serde() {
        let committed = IndexCommitted {
            service_generation: 13,
        };
        let json = serde_json::to_value(&committed).expect("serialize");
        let recovered: IndexCommitted = serde_json::from_value(json).expect("deserialize");
        assert_eq!(committed, recovered);
    }

    #[test]
    fn sync_completed_with_generation_get_set_round_trips() {
        let mut completed = SyncCompleted {
            account_id: "a".into(),
            run_id: SyncRunId::new_v7(),
            result: SyncResult::Cancelled,
            service_generation: 1,
        };
        assert_eq!(completed.generation(), 1);
        completed.set_generation(99);
        assert_eq!(completed.generation(), 99);
        assert_eq!(completed.service_generation, 99);
    }

    #[test]
    fn index_committed_with_generation_get_set_round_trips() {
        let mut committed = IndexCommitted {
            service_generation: 1,
        };
        assert_eq!(committed.generation(), 1);
        committed.set_generation(99);
        assert_eq!(committed.generation(), 99);
        assert_eq!(committed.service_generation, 99);
    }
}
