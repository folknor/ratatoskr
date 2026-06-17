//! Calendar wire types.
//!
//! Calendar sync lives in the Service alongside email sync. The shape
//! mirrors `sync`: UI dispatches `calendar.start_account_sync` and
//! waits on a `calendar.run_completed` notification correlated by
//! `CalendarRunId`.
//!
//! Calendar has a second view-reload notification:
//! `calendar.changed` (Coalesce), split out from the per-run
//! completion (MustDeliver). `CalendarRunCompleted` is routed to
//! per-run awaiters while `CalendarChanged` is routed to UI reload
//! handling.
//!
//! `CalendarChanged` fires whenever a run mutated calendar tables,
//! regardless of result: per-discovered-calendar upserts in
//! `crates/calendar/src/sync.rs` commit *before* later `?` failures
//! could occur, so a cancelled or failed run that already wrote a
//! batch must still trigger a UI reload.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::notification::WithGeneration;

// ---------------------------------------------------------------------------
// Identifiers
// ---------------------------------------------------------------------------

/// 128-bit time-ordered identifier for a single calendar sync run.
///
/// UUIDv7 so the per-account map's debug trace has good locality and
/// the run id is unique across Service respawns.
///
/// One `CalendarRunId` corresponds to exactly one runner-task lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CalendarRunId(pub Uuid);

impl CalendarRunId {
    /// Generate a fresh time-ordered run id. Service-side only.
    #[must_use]
    pub fn new_v7() -> Self {
        Self(Uuid::now_v7())
    }
}

impl std::fmt::Display for CalendarRunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

// ---------------------------------------------------------------------------
// Request params (calendar.start_account_sync / calendar.cancel_account_sync)
// ---------------------------------------------------------------------------

/// `calendar.start_account_sync` request body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarStartAccountSyncParams {
    pub account_id: String,
}

/// Synchronous response to `calendar.start_account_sync`. Mirrors
/// `SyncStartAck`: the handler acquires the per-account map, returns the
/// id of the existing or freshly-spawned runner, and acks. The runner
/// itself is fire-and-forget; per-run completion is delivered via the
/// `calendar.run_completed` notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarStartAck {
    pub account_id: String,
    pub run_id: CalendarRunId,
    pub already_in_flight: bool,
}

/// `calendar.cancel_account_sync` request body. Reserved for the
/// **explicit-request** cancel path (manual "Sync now", RSVP-then-resync).
/// Account deletion uses the piggyback inside `handle_cancel_account` -
/// see the account-deletion integration in the sync cancel handler.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarCancelAccountSyncParams {
    pub account_id: String,
}

/// Synchronous response to `calendar.cancel_account_sync`. `run_id` is
/// `Some` if a run was in flight (the caller can subscribe to that run's
/// `calendar.run_completed` to know when cancellation completed); `None`
/// if no runner was active.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarCancelAck {
    pub account_id: String,
    pub run_id: Option<CalendarRunId>,
    pub was_in_flight: bool,
}

// ---------------------------------------------------------------------------
// Request params (calendar.set_visibility)
// ---------------------------------------------------------------------------

/// `calendar.set_visibility` request body. The calendar visibility
/// toggle is the flat-boolean half of `db/calendar.rs`; event mutations
/// use the calendar action pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarSetVisibilityParams {
    pub calendar_id: String,
    pub visible: bool,
}

/// `calendar.set_visibility` ack. Empty struct; failure surfaces
/// through `ServiceResponse::Error`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarSetVisibilityAck;

// ---------------------------------------------------------------------------
// Notification payloads
// ---------------------------------------------------------------------------

/// Final result of a calendar sync run. Wire-narrow, like `SyncResult`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum CalendarSyncResult {
    Completed,
    Cancelled,
    Failed(String),
}

/// Per-run completion. `MustDeliver`: a dropped completion leaves any
/// `cancel_and_await`-shaped UI future hanging forever. Routed by
/// `run_id` so multiple waiters for the same run all resolve from one
/// broadcast.
///
/// `mutated` is informational on this notification - explicit-request
/// callers can use it to decide whether to skip a debounce. The UI's
/// view-reload signal is the separate `CalendarChanged` notification,
/// not this one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarRunCompleted {
    pub account_id: String,
    pub run_id: CalendarRunId,
    pub result: CalendarSyncResult,
    /// `true` if the runner committed at least one calendar-table write
    /// during the run. Set regardless of final `result` - cancellation
    /// or failure after a partial-batch commit still flips this true.
    pub mutated: bool,
    /// Cross-respawn drop tag. Service emits 0; UI's reader task
    /// overwrites at enqueue with `current_generation()`.
    pub service_generation: u32,
}

impl WithGeneration for CalendarRunCompleted {
    fn generation(&self) -> u32 {
        self.service_generation
    }
    fn set_generation(&mut self, generation: u32) {
        self.service_generation = generation;
    }
}

/// View-reload signal. `Coalesce` keyed per-account so N accounts
/// completing a kick batch produce a debounced reload, not a flood.
/// Emitted whenever a run set `mutated == true`, regardless of final
/// result. UI dispatches through a 250 ms trailing-edge debouncer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarChanged {
    pub account_id: String,
    /// Cross-respawn drop tag. Service emits 0; UI's reader task
    /// overwrites at enqueue with `current_generation()`.
    pub service_generation: u32,
}

impl WithGeneration for CalendarChanged {
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
    fn calendar_run_id_round_trips_through_serde() {
        let id = CalendarRunId::new_v7();
        let json = serde_json::to_value(id).expect("serialize");
        let recovered: CalendarRunId = serde_json::from_value(json).expect("deserialize");
        assert_eq!(id, recovered);
    }

    #[test]
    fn calendar_run_id_v7_is_time_ordered() {
        let a = CalendarRunId::new_v7();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = CalendarRunId::new_v7();
        assert!(a.0 < b.0, "expected v7 timestamps to monotonically advance");
    }

    #[test]
    fn calendar_start_account_sync_params_round_trips_through_serde() {
        let params = CalendarStartAccountSyncParams {
            account_id: "acc-7".into(),
        };
        let json = serde_json::to_value(&params).expect("serialize");
        let recovered: CalendarStartAccountSyncParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(params, recovered);
    }

    #[test]
    fn calendar_set_visibility_params_round_trips_through_serde() {
        let params = CalendarSetVisibilityParams {
            calendar_id: "cal-1".into(),
            visible: true,
        };
        let json = serde_json::to_value(&params).expect("serialize");
        let recovered: CalendarSetVisibilityParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(params, recovered);
    }

    #[test]
    fn calendar_set_visibility_ack_round_trips_through_serde() {
        let ack = CalendarSetVisibilityAck;
        let json = serde_json::to_value(&ack).expect("serialize");
        let _recovered: CalendarSetVisibilityAck =
            serde_json::from_value(json).expect("deserialize");
    }

    #[test]
    fn calendar_cancel_account_sync_params_round_trips_through_serde() {
        let params = CalendarCancelAccountSyncParams {
            account_id: "acc-8".into(),
        };
        let json = serde_json::to_value(&params).expect("serialize");
        let recovered: CalendarCancelAccountSyncParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(params, recovered);
    }

    #[test]
    fn calendar_start_ack_round_trips_through_serde() {
        let ack = CalendarStartAck {
            account_id: "acc-1".into(),
            run_id: CalendarRunId::new_v7(),
            already_in_flight: true,
        };
        let json = serde_json::to_value(&ack).expect("serialize");
        let recovered: CalendarStartAck = serde_json::from_value(json).expect("deserialize");
        assert_eq!(ack, recovered);
    }

    #[test]
    fn calendar_cancel_ack_round_trips_through_serde() {
        let cases = [
            CalendarCancelAck {
                account_id: "acc-1".into(),
                run_id: Some(CalendarRunId::new_v7()),
                was_in_flight: true,
            },
            CalendarCancelAck {
                account_id: "acc-2".into(),
                run_id: None,
                was_in_flight: false,
            },
        ];
        for ack in cases {
            let json = serde_json::to_value(&ack).expect("serialize");
            let recovered: CalendarCancelAck = serde_json::from_value(json).expect("deserialize");
            assert_eq!(ack, recovered);
        }
    }

    #[test]
    fn calendar_sync_result_round_trips_through_serde() {
        let cases = [
            CalendarSyncResult::Completed,
            CalendarSyncResult::Cancelled,
            CalendarSyncResult::Failed("provider 503".into()),
        ];
        for result in cases {
            let json = serde_json::to_value(&result).expect("serialize");
            let recovered: CalendarSyncResult = serde_json::from_value(json).expect("deserialize");
            assert_eq!(result, recovered);
        }
    }

    #[test]
    fn calendar_run_completed_round_trips_through_serde() {
        let completed = CalendarRunCompleted {
            account_id: "acc-1".into(),
            run_id: CalendarRunId::new_v7(),
            result: CalendarSyncResult::Completed,
            mutated: true,
            service_generation: 7,
        };
        let json = serde_json::to_value(&completed).expect("serialize");
        let recovered: CalendarRunCompleted = serde_json::from_value(json).expect("deserialize");
        assert_eq!(completed, recovered);
    }

    #[test]
    fn calendar_changed_round_trips_through_serde() {
        let changed = CalendarChanged {
            account_id: "acc-42".into(),
            service_generation: 13,
        };
        let json = serde_json::to_value(&changed).expect("serialize");
        let recovered: CalendarChanged = serde_json::from_value(json).expect("deserialize");
        assert_eq!(changed, recovered);
    }

    #[test]
    fn calendar_run_completed_with_generation_get_set_round_trips() {
        let mut completed = CalendarRunCompleted {
            account_id: "a".into(),
            run_id: CalendarRunId::new_v7(),
            result: CalendarSyncResult::Cancelled,
            mutated: false,
            service_generation: 1,
        };
        assert_eq!(completed.generation(), 1);
        completed.set_generation(99);
        assert_eq!(completed.generation(), 99);
        assert_eq!(completed.service_generation, 99);
    }

    #[test]
    fn calendar_changed_with_generation_get_set_round_trips() {
        let mut changed = CalendarChanged {
            account_id: "a".into(),
            service_generation: 1,
        };
        assert_eq!(changed.generation(), 1);
        changed.set_generation(99);
        assert_eq!(changed.generation(), 99);
        assert_eq!(changed.service_generation, 99);
    }
}
