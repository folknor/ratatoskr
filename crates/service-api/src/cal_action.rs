//! Calendar action wire types.
//!
//! Phase 6c relocates calendar event mutations Service-side. The UI
//! sends a `CalendarActionPlan` over IPC (`cal_action.execute_plan`);
//! the Service journals it as a `kind = 'calendar_plan'` row in
//! `action_jobs` (Phase 6c-1 widened the CHECK constraint), and the
//! action worker dispatches each op to `service::cal_actions::batch_execute`,
//! emitting per-op `CalendarOperationOutcome` notifications and a final
//! `CalendarActionCompleted` per plan.
//!
//! ## Why a sibling pipeline rather than folding into `WireMailOperation`
//!
//! The mail wire types are exhaustively matched at seven sites
//! (`completion_behavior`, `dispatch_with_provider`, `op_local`,
//! `enqueue_params`, `op_name`, `to_wire_op`, `wire_to_mail`). Folding
//! the calendar union into `WireMailOperation` forces every mail-side
//! site to add calendar arms that fall through. Calendar mutations
//! also have semantics email mutations do not (provider-first vs
//! local-first per-variant; ETag-based concurrency; future
//! series-vs-occurrence and RSVP semantics in Phase 6d), and forcing
//! them through the email-shaped pipeline would be a tax on every
//! mail-side site without buying anything.
//!
//! Mail and calendar share the `action_jobs` journal via the existing
//! `kind` CHECK constraint - the SQL layer treats both kinds
//! identically; only the worker dispatch and the wire-frame `method`
//! name differ. The journal's `account_id` column carries the
//! account scope; the wire `CalendarOperation` carries the calendar
//! event id where applicable (`UpdateEvent` / `DeleteEvent`).
//!
//! `account_id` is `String`s; the mail-side typed-newtype mirrors
//! (`WireFolderId`, `WireLabelId`) have no analogue here because
//! calendar refs are opaque provider strings rather than enum-able
//! ids.

use serde::{Deserialize, Serialize};

use crate::action::PlanId;
use crate::notification::WithGeneration;

// ---------------------------------------------------------------------------
// Calendar event input
// ---------------------------------------------------------------------------

/// Wire-side mirror of `cal::actions::CalendarEventInput`.
///
/// Wire-only mirror so `service-api` does not pull in the calendar
/// crate's transitive provider-trait graph. The Service handler
/// converts to the in-process domain type before dispatching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireCalendarEventInput {
    pub title: String,
    pub description: String,
    pub location: String,
    pub start_time: i64,
    pub end_time: i64,
    pub is_all_day: bool,
    pub timezone: Option<String>,
    pub recurrence_rule: Option<String>,
    pub availability: Option<String>,
    pub visibility: Option<String>,
}

// ---------------------------------------------------------------------------
// Calendar operation
// ---------------------------------------------------------------------------

/// Per-op variant carried inside a `CalendarActionPlan`.
///
/// Each variant maps 1:1 to a `cal::actions::*` function in the
/// pre-Phase-6c surface:
/// - `CreateEvent` -> `create_calendar_event(ctx, account_id, calendar_remote_id, input)`
/// - `UpdateEvent` -> `update_calendar_event(ctx, account_id, event_id, input)`
/// - `DeleteEvent` -> `delete_calendar_event(ctx, account_id, event_id)`
///
/// **`LocalOnly` reachability** (see `CalendarOperationResult`) differs
/// per variant:
/// - `CreateEvent`: writes locally first, provider second; if the
///   provider fails, the local row persists and the wire result is
///   `LocalOnly`. The user sees the event with a "not synced"
///   indicator.
/// - `UpdateEvent` / `DeleteEvent`: provider-first for synced events
///   (the ones with `remote_event_id`). For these, the wire result
///   is `Success | Failed`, never `LocalOnly`. Unsynced events (no
///   `remote_event_id`) shortcut to local-only writes; their wire
///   result for `Update` / `Delete` is `Success` if the local write
///   succeeds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum WireCalendarOperation {
    /// Create a new calendar event on the named calendar. Local-first:
    /// writes the local row, then dispatches to the provider. Provider
    /// failure surfaces as `LocalOnly`.
    CreateEvent {
        calendar_remote_id: String,
        input: WireCalendarEventInput,
    },
    /// Update an existing calendar event by local id. Provider-first
    /// for synced events; falls back to local-only update for events
    /// with no `remote_event_id`.
    UpdateEvent {
        event_id: String,
        input: WireCalendarEventInput,
    },
    /// Delete an existing calendar event by local id. Provider-first
    /// for synced events; falls back to local-only delete for events
    /// with no `remote_event_id`.
    DeleteEvent { event_id: String },
}

// ---------------------------------------------------------------------------
// Plan wire types (request side)
// ---------------------------------------------------------------------------

/// One operation inside a `CalendarActionPlan`. Carries the typed
/// `WireCalendarOperation` plus the `account_id` target.
///
/// `operation_id` is the per-plan ordinal correlation key. Calendar
/// plans today are 1:1 (one user intent = one operation), but the
/// shape mirrors the mail action plan so the journal write path can
/// stay shared and so future RSVP / series-vs-occurrence intents in
/// Phase 6d can layer in N-op plans without changing the wire frame.
///
/// `event_id` (the local calendar event id) is overloaded into the
/// journal's `action_job_ops.thread_id` column at handler time:
/// calendar ops have no thread, but the column is `NOT NULL` so we
/// reuse it for per-op correlation. For `CreateEvent`, the event id
/// is not known until execution mints it; the journal stores the
/// empty string in that case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarActionWireOperation {
    pub operation_id: crate::action::OperationId,
    pub account_id: String,
    pub operation: WireCalendarOperation,
}

/// Resolved-and-planned calendar action ready for IPC dispatch.
///
/// The UI builds this from a calendar workflow (today's
/// `handle_save_event` / `DeleteEvent` paths). The Service journals
/// it as a `kind = 'calendar_plan'` row + N `action_job_ops` children.
/// Subsequent `CalendarOperationOutcome` notifications (one per op)
/// and a final `CalendarActionCompleted` (one per plan) drive the UI's
/// completion handling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarActionPlan {
    pub plan_id: PlanId,
    pub operations: Vec<CalendarActionWireOperation>,
}

/// Synchronous response to `cal_action.execute_plan`. Matches the
/// shape of `ActionPlanAck` for symmetry: the Service has validated
/// and journaled the plan; the worker will pick it up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarActionPlanAck {
    pub plan_id: PlanId,
    pub journaled: bool,
}

// ---------------------------------------------------------------------------
// Outcome wire types (notification side)
// ---------------------------------------------------------------------------

/// Per-op result emitted by the Service-side calendar dispatcher.
///
/// **`LocalOnly` is reachable only for `CreateEvent`.** `UpdateEvent`
/// and `DeleteEvent` are provider-first for synced events and return
/// `Success | Failed`. The mapping to the in-process
/// `cal::actions::ActionOutcome` happens at the IPC boundary inside
/// `service::cal_actions::batch_execute`.
///
/// This is a sibling of `crate::action::OperationResult`, intentionally
/// narrower: calendar dispatch has no `RemoteFailure { http_status,
/// retryable }` taxonomy because the existing `cal::actions` code
/// returns `ActionOutcome::LocalOnly { reason, retryable }` on
/// provider failure rather than the rich `RemoteFailure`
/// classification that mail uses. Phase 6d will re-evaluate when RSVP
/// / series semantics arrive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum CalendarOperationResult {
    /// Local DB mutation + provider call both succeeded (or the
    /// operation was a local-only update / delete on an unsynced event).
    Success,
    /// Local DB mutation succeeded; provider call failed. Reachable
    /// only for `CreateEvent`. The local row persists; the user sees
    /// the event with a "not synced" indicator.
    LocalOnly { reason: String },
    /// Operation failed without producing an observable local state
    /// change. The UI shows the failure and does NOT close any
    /// optimistic-applied UI.
    Failed { error: String },
}

/// Worker emits one of these per operation in a calendar plan.
/// `MustDeliver` notification class; cross-respawn safety via
/// `service_generation`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarOperationOutcome {
    pub plan_id: PlanId,
    pub operation_id: crate::action::OperationId,
    pub result: CalendarOperationResult,
    /// Cross-respawn drop tag. Service emits 0; UI's reader task
    /// overwrites at enqueue with `current_generation()`. See
    /// `crate::action::OperationOutcome::service_generation` for the
    /// full rationale.
    pub service_generation: u32,
}

impl WithGeneration for CalendarOperationOutcome {
    fn generation(&self) -> u32 {
        self.service_generation
    }
    fn set_generation(&mut self, generation: u32) {
        self.service_generation = generation;
    }
}

/// Worker emits this once per calendar plan after every op has reached
/// a terminal status. Mirror of `crate::action::ActionCompleted` for
/// the calendar pipeline. `MustDeliver`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarActionCompleted {
    pub plan_id: PlanId,
    pub results: Vec<CalendarOperationOutcome>,
    /// Cross-respawn drop tag. See `CalendarOperationOutcome::service_generation`.
    pub service_generation: u32,
}

impl WithGeneration for CalendarActionCompleted {
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
    use crate::action::OperationId;

    #[test]
    fn calendar_operation_round_trips_through_serde() {
        let op = WireCalendarOperation::CreateEvent {
            calendar_remote_id: "primary".to_string(),
            input: WireCalendarEventInput {
                title: "Standup".to_string(),
                description: String::new(),
                location: String::new(),
                start_time: 1_700_000_000,
                end_time: 1_700_003_600,
                is_all_day: false,
                timezone: Some("UTC".to_string()),
                recurrence_rule: None,
                availability: None,
                visibility: None,
            },
        };
        let json = serde_json::to_value(&op).expect("serialize");
        let recovered: WireCalendarOperation = serde_json::from_value(json).expect("deserialize");
        assert_eq!(op, recovered);
    }

    #[test]
    fn calendar_action_plan_round_trips_through_serde() {
        let plan = CalendarActionPlan {
            plan_id: PlanId::new_v7(),
            operations: vec![CalendarActionWireOperation {
                operation_id: OperationId(0),
                account_id: "acc-1".to_string(),
                operation: WireCalendarOperation::DeleteEvent {
                    event_id: "evt-1".to_string(),
                },
            }],
        };
        let json = serde_json::to_value(&plan).expect("serialize");
        let recovered: CalendarActionPlan = serde_json::from_value(json).expect("deserialize");
        assert_eq!(plan, recovered);
    }

    #[test]
    fn calendar_operation_result_serializes_local_only_with_reason() {
        let result = CalendarOperationResult::LocalOnly {
            reason: "provider HTTP 503".to_string(),
        };
        let json = serde_json::to_value(&result).expect("serialize");
        let recovered: CalendarOperationResult = serde_json::from_value(json).expect("deserialize");
        assert_eq!(result, recovered);
    }
}
