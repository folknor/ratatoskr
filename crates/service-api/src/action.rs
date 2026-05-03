//! Action wire types.
//!
//! Phase 2 introduces the action service relocation: the UI sends an
//! `ActionWirePlan` over IPC; the Service journals it (per the
//! sibling-job model in `docs/service/phase-2-plan.md` scope item 18a)
//! and a worker drives execution with per-operation `OperationOutcome`
//! notifications and a final `ActionCompleted`.
//!
//! The wire types live here rather than in `crates/core/src/actions/` so
//! that `service-api` stays lightweight - core depends on
//! providers/search/store, and pulling that graph into every consumer
//! of `service-api` would defeat the "wire crate" framing. The mirror
//! types are 1:1 with their `core::actions` counterparts; conversion
//! happens at the app/service edge in a bridge layer (added when the
//! action service relocates in task 9).
//!
//! `core::actions::MailOperation` itself is NOT serializable (no serde
//! derives). The wire mirror `WireMailOperation` is the canonical
//! serializable form. Adding a variant to `MailOperation` without a
//! matching arm here is a contract violation - task 9 lands a
//! conversion function in the bridge crate whose exhaustive match
//! enforces this at compile time.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::notification::WithGeneration;

// ---------------------------------------------------------------------------
// Identifiers
// ---------------------------------------------------------------------------

/// 128-bit time-ordered identifier for an action plan.
///
/// UUIDv7: the high 48 bits are a millisecond timestamp, the rest is
/// random. Time-ordered insertion gives the journal partial-index
/// scheduler good locality, and 128-bit width survives a UI restart
/// resetting any in-process counter (a u64 counter reset across
/// restart could collide with a journal row from the previous
/// incarnation; UUIDv7 cannot).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PlanId(pub Uuid);

impl PlanId {
    /// Generate a fresh time-ordered plan id. UI calls this when
    /// resolving an `ActionExecutionPlan` into an `ActionWirePlan`.
    #[must_use]
    pub fn new_v7() -> Self {
        Self(Uuid::now_v7())
    }
}

impl std::fmt::Display for PlanId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Per-plan ordinal identifier for an operation. Generated UI-side
/// alongside the plan; the Service uses `(plan_id, operation_id)`
/// as the journal's `action_job_ops` primary key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OperationId(pub u32);

// ---------------------------------------------------------------------------
// Typed-ID wire wrappers
// ---------------------------------------------------------------------------

/// Wire-side folder identifier. Mirrors `common::typed_ids::FolderId` as
/// a serializable newtype that does not depend on `common`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WireFolderId(pub String);

/// Wire-side tag/label identifier. Mirrors `common::typed_ids::TagId`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WireTagId(pub String);

// ---------------------------------------------------------------------------
// MailOperation wire mirror
// ---------------------------------------------------------------------------

/// 1:1 mirror of `core::actions::MailOperation` with serde derives.
/// Variants must match `MailOperation` exactly, including all fields.
/// Field-loss bugs are caught at compile time by the conversion
/// function the action-service relocation adds in task 9 (an
/// exhaustive match without `_` wildcards).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum WireMailOperation {
    Archive,
    Trash,
    PermanentDelete,
    SetSpam { to: bool },
    SetStarred { to: bool },
    SetRead { to: bool },
    SetPinned { to: bool },
    SetMuted { to: bool },
    MoveToFolder {
        dest: WireFolderId,
        source: Option<WireFolderId>,
    },
    AddLabel { label_id: WireTagId },
    RemoveLabel { label_id: WireTagId },
    Snooze { until: i64 },
}

// ---------------------------------------------------------------------------
// Plan wire types (request side)
// ---------------------------------------------------------------------------

/// One operation inside an `ActionWirePlan`. Carries the typed
/// `WireMailOperation` plus the `(account_id, thread_id)` target. The
/// Service journals one `action_job_ops` row per `ActionWireOperation`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionWireOperation {
    pub operation_id: OperationId,
    pub account_id: String,
    pub thread_id: String,
    pub operation: WireMailOperation,
}

/// Resolved-and-planned action ready for IPC dispatch. The UI builds
/// this from an `ActionExecutionPlan`; the UI metadata (toast text,
/// auto-advance hints, completion-behavior policy) stays UI-side in
/// `in_flight_plans`, keyed by `plan_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionWirePlan {
    pub plan_id: PlanId,
    pub operations: Vec<ActionWireOperation>,
}

/// Synchronous response to `action.execute_plan`. The Service has
/// validated the plan and journaled it in `action_jobs` +
/// `action_job_ops`; from this point a Service crash does NOT lose
/// the plan - the worker picks it up after respawn and the journal
/// drives replay.
///
/// `journaled = true` is the common case; `false` is reserved for
/// future shapes (e.g. a hypothetical "validate-only" mode) and
/// always indicates the plan will NOT be executed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionPlanAck {
    pub plan_id: PlanId,
    pub journaled: bool,
}

// ---------------------------------------------------------------------------
// Outcome wire types (notification side)
// ---------------------------------------------------------------------------

/// Per-operation result emitted by the worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum OperationResult {
    /// Local DB mutation + provider call both succeeded.
    Success,
    /// Local DB mutation succeeded; provider call deferred to
    /// `pending_operations` (transient retryable failure). The UI's
    /// optimistic state stays applied; the periodic drainer retries
    /// the provider call.
    LocalOnly,
    /// Provider rejected the call. `retryable = false` means the
    /// pending-ops queue does NOT re-enqueue (the action is lost).
    RemoteFailure { failure: RemoteFailure },
    /// The local state was incompatible with the operation (e.g.
    /// archiving a thread that's already archived). UI's optimistic
    /// update was already a no-op; the worker emits this so the UI
    /// can surface a toast if it wants.
    ConflictRejected { detail: String },
}

/// Provider-side failure detail. Provider-specific error variants from
/// `core::actions::ActionError` collapse into this one shape on the
/// wire to keep `service-api` decoupled from provider error
/// taxonomies. The retryable flag drives `pending_operations`
/// re-enqueue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteFailure {
    pub provider_message: String,
    pub http_status: Option<u16>,
    pub retryable: bool,
}

/// Worker emits one of these per operation. `MustDeliver` notification
/// (per `docs/service/problem-statement.md` § IPC "notification class
/// taxonomy"); cross-respawn safety via `service_generation`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationOutcome {
    pub plan_id: PlanId,
    pub operation_id: OperationId,
    pub result: OperationResult,
    /// Set by the UI's reader task at enqueue time; the dispatch side
    /// drops mismatches against the live incarnation.
    pub service_generation: u32,
}

impl WithGeneration for OperationOutcome {
    fn generation(&self) -> u32 {
        self.service_generation
    }
    fn set_generation(&mut self, generation: u32) {
        self.service_generation = generation;
    }
}

/// Aggregate counts populated by the worker after a plan reaches a
/// terminal status. Mirrors the journaled `action_jobs.summary` blob.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PlanSummary {
    pub total: u32,
    pub local_only: u32,
    pub remote_succeeded: u32,
    pub remote_failed: u32,
    pub conflicts: u32,
}

/// Worker emits this once per plan after the per-plan transaction has
/// committed and the result is observable from a fresh read connection.
/// `MustDeliver` notification with cross-respawn safety via
/// `service_generation`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionCompleted {
    pub plan_id: PlanId,
    pub summary: PlanSummary,
    pub service_generation: u32,
}

impl WithGeneration for ActionCompleted {
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
    fn plan_id_round_trips_through_serde() {
        let id = PlanId::new_v7();
        let json = serde_json::to_value(id).expect("serialize");
        let recovered: PlanId = serde_json::from_value(json).expect("deserialize");
        assert_eq!(id, recovered);
    }

    #[test]
    fn plan_id_v7_is_time_ordered() {
        // UUIDv7 high bits are a ms timestamp; ids generated in order
        // sort in order. Sanity check rather than a strict guarantee.
        let a = PlanId::new_v7();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = PlanId::new_v7();
        assert!(a.0 < b.0, "expected v7 timestamps to monotonically advance");
    }

    #[test]
    fn wire_mail_operation_round_trips_through_serde() {
        let cases = [
            WireMailOperation::Archive,
            WireMailOperation::Trash,
            WireMailOperation::PermanentDelete,
            WireMailOperation::SetSpam { to: true },
            WireMailOperation::SetStarred { to: false },
            WireMailOperation::SetRead { to: true },
            WireMailOperation::SetPinned { to: false },
            WireMailOperation::SetMuted { to: true },
            WireMailOperation::MoveToFolder {
                dest: WireFolderId("inbox".into()),
                source: Some(WireFolderId("archive".into())),
            },
            WireMailOperation::MoveToFolder {
                dest: WireFolderId("inbox".into()),
                source: None,
            },
            WireMailOperation::AddLabel {
                label_id: WireTagId("work".into()),
            },
            WireMailOperation::RemoveLabel {
                label_id: WireTagId("personal".into()),
            },
            WireMailOperation::Snooze { until: 1_700_000_000 },
        ];
        for op in cases {
            let json = serde_json::to_value(&op).expect("serialize");
            let recovered: WireMailOperation =
                serde_json::from_value(json).expect("deserialize");
            assert_eq!(op, recovered);
        }
    }

    #[test]
    fn action_wire_plan_round_trips_through_serde() {
        let plan = ActionWirePlan {
            plan_id: PlanId::new_v7(),
            operations: vec![
                ActionWireOperation {
                    operation_id: OperationId(0),
                    account_id: "acc-1".into(),
                    thread_id: "thr-9".into(),
                    operation: WireMailOperation::Archive,
                },
                ActionWireOperation {
                    operation_id: OperationId(1),
                    account_id: "acc-1".into(),
                    thread_id: "thr-10".into(),
                    operation: WireMailOperation::SetStarred { to: true },
                },
            ],
        };
        let json = serde_json::to_value(&plan).expect("serialize");
        let recovered: ActionWirePlan = serde_json::from_value(json).expect("deserialize");
        assert_eq!(plan, recovered);
    }

    #[test]
    fn action_plan_ack_round_trips_through_serde() {
        let ack = ActionPlanAck {
            plan_id: PlanId::new_v7(),
            journaled: true,
        };
        let json = serde_json::to_value(&ack).expect("serialize");
        let recovered: ActionPlanAck = serde_json::from_value(json).expect("deserialize");
        assert_eq!(ack, recovered);
    }

    #[test]
    fn operation_outcome_round_trips_through_serde() {
        let cases = [
            OperationOutcome {
                plan_id: PlanId::new_v7(),
                operation_id: OperationId(0),
                result: OperationResult::Success,
                service_generation: 7,
            },
            OperationOutcome {
                plan_id: PlanId::new_v7(),
                operation_id: OperationId(1),
                result: OperationResult::LocalOnly,
                service_generation: 7,
            },
            OperationOutcome {
                plan_id: PlanId::new_v7(),
                operation_id: OperationId(2),
                result: OperationResult::RemoteFailure {
                    failure: RemoteFailure {
                        provider_message: "rate limited".into(),
                        http_status: Some(429),
                        retryable: true,
                    },
                },
                service_generation: 7,
            },
            OperationOutcome {
                plan_id: PlanId::new_v7(),
                operation_id: OperationId(3),
                result: OperationResult::ConflictRejected {
                    detail: "thread already archived".into(),
                },
                service_generation: 7,
            },
        ];
        for outcome in cases {
            let json = serde_json::to_value(&outcome).expect("serialize");
            let recovered: OperationOutcome =
                serde_json::from_value(json).expect("deserialize");
            assert_eq!(outcome, recovered);
        }
    }

    #[test]
    fn action_completed_round_trips_through_serde() {
        let completed = ActionCompleted {
            plan_id: PlanId::new_v7(),
            summary: PlanSummary {
                total: 5,
                local_only: 1,
                remote_succeeded: 3,
                remote_failed: 1,
                conflicts: 0,
            },
            service_generation: 11,
        };
        let json = serde_json::to_value(&completed).expect("serialize");
        let recovered: ActionCompleted = serde_json::from_value(json).expect("deserialize");
        assert_eq!(completed, recovered);
    }

    #[test]
    fn with_generation_get_set_round_trips_for_operation_outcome() {
        let mut outcome = OperationOutcome {
            plan_id: PlanId::new_v7(),
            operation_id: OperationId(0),
            result: OperationResult::Success,
            service_generation: 1,
        };
        assert_eq!(outcome.generation(), 1);
        outcome.set_generation(42);
        assert_eq!(outcome.generation(), 42);
        assert_eq!(outcome.service_generation, 42);
    }

    #[test]
    fn with_generation_get_set_round_trips_for_action_completed() {
        let mut completed = ActionCompleted {
            plan_id: PlanId::new_v7(),
            summary: PlanSummary::default(),
            service_generation: 1,
        };
        assert_eq!(completed.generation(), 1);
        completed.set_generation(42);
        assert_eq!(completed.generation(), 42);
        assert_eq!(completed.service_generation, 42);
    }
}
