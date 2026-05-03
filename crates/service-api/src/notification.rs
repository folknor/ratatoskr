use crate::action::{ActionCompleted, OperationOutcome};
use crate::boot::{BootPhaseKind, BootProgress};
use serde::{Deserialize, Serialize};

/// Marker + accessor trait for notification payloads that carry a
/// Service-incarnation generation tag for cross-respawn dispatch
/// filtering. Implement on every payload struct whose parent
/// `Notification` arm needs the cross-respawn drop guarantee
/// (`BootProgress` today; future `ActionCompleted`, `PushEvent`,
/// `OperationOutcome`, etc.).
///
/// The trait exists so that `Notification::service_generation()` and
/// `Notification::set_service_generation()` dispatch through a method
/// rather than naming a struct field. A future contributor adding a
/// new tagged payload must implement the trait, which forces them to
/// (a) put the `service_generation: u32` field on the payload and
/// (b) decide that the variant is in fact tagged. Test-only /
/// informational notifications that carry no UI state effect (e.g.
/// `TestEcho`) intentionally do NOT implement this trait; their
/// `service_generation()` arm returns `None` and dispatch never filters
/// them.
pub trait WithGeneration {
    fn generation(&self) -> u32;
    fn set_generation(&mut self, generation: u32);
}

impl WithGeneration for BootProgress {
    fn generation(&self) -> u32 {
        self.service_generation
    }
    fn set_generation(&mut self, generation: u32) {
        self.service_generation = generation;
    }
}

// `OperationOutcome` and `ActionCompleted` `WithGeneration` impls live
// in `crate::action` alongside the type definitions; see that module.

/// Per-class coalesce key. The queue compares `CoalesceKey` for equality
/// when deciding whether a new entry replaces an existing one; the type is
/// constructed at enqueue time, never serialized onto the wire (the wire
/// notification carries the typed payload instead).
///
/// Each production variant is a discrete coalesce bucket. `BootProgress`
/// keys off `BootPhaseKind` so each phase coalesces independently and the
/// ordered phase sequence reaches the UI even under back-pressure.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum CoalesceKey {
    BootProgress(BootPhaseKind),
    /// Synthetic key used only by in-process queue tests in consumer crates.
    /// The queue's per-class enqueue logic is generic over `Classifiable`,
    /// and tests construct mock items with arbitrary string-keyed coalesce
    /// keys. Production code paths never construct `Test`.
    Test(String),
}

impl CoalesceKey {
    /// Construct a `Test` coalesce key. Test-only - the constructor name
    /// reflects that.
    pub fn test(value: impl Into<String>) -> Self {
        Self::Test(value.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationClass {
    Coalesce { key: CoalesceKey },
    Drop,
    MustDeliver,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum Notification {
    /// Service boot progress. Emitted during the boot sequence so the UI
    /// splash can render phase + per-migration progress while migrations,
    /// pending-ops recovery, drafts sweep, and thread-participants backfill
    /// run.
    #[serde(rename = "boot.progress")]
    BootProgress(BootProgress),
    /// Per-operation outcome from the action worker. `MustDeliver`: a
    /// dropped outcome desynchronises the UI's per-plan
    /// `applied_outcomes` set vs the journal. Cross-respawn safety via
    /// `service_generation`.
    #[serde(rename = "action.operation_outcome")]
    OperationOutcome(OperationOutcome),
    /// Per-plan completion from the action worker. `MustDeliver`: a
    /// dropped completion leaves the UI's `in_flight_plans` entry alive
    /// forever. Emitted after the per-plan transaction has committed and
    /// is observable from a fresh read connection.
    #[serde(rename = "action.completed")]
    ActionCompleted(ActionCompleted),
    /// Test-only variant. Lets the wire round-trip be exercised when no
    /// production payload happens to match a test's needs. Compiled out of
    /// release builds via `#[cfg(test)]`.
    #[cfg(test)]
    #[serde(rename = "test.echo")]
    TestEcho { value: String },
}

impl Notification {
    pub fn class(&self) -> NotificationClass {
        match self {
            Self::BootProgress(progress) => NotificationClass::Coalesce {
                key: CoalesceKey::BootProgress(progress.phase.coalesce_discriminant()),
            },
            // `MustDeliver` for both action notifications: the UI's
            // applied-outcome dedupe and the in-flight-plans completion
            // unwinder both need every event to land. Coalescing would
            // collapse outcomes for different operations within a plan
            // (each carries a distinct OperationId), and dropping under
            // pressure would leak `in_flight_plans` entries forever.
            Self::OperationOutcome(_) => NotificationClass::MustDeliver,
            Self::ActionCompleted(_) => NotificationClass::MustDeliver,
            #[cfg(test)]
            Self::TestEcho { .. } => NotificationClass::Coalesce {
                key: CoalesceKey::test("test.echo"),
            },
        }
    }

    pub fn method_name(&self) -> &'static str {
        match self {
            Self::BootProgress(_) => "boot.progress",
            Self::OperationOutcome(_) => "action.operation_outcome",
            Self::ActionCompleted(_) => "action.completed",
            #[cfg(test)]
            Self::TestEcho { .. } => "test.echo",
        }
    }

    /// The Service-incarnation generation tag carried by this notification,
    /// if the variant has one. The reader task overwrites this value with
    /// its own captured generation at enqueue time
    /// (`set_service_generation` below); the dispatch side compares against
    /// the live `ServiceClient::current_generation` and drops mismatches
    /// (scope item 20 of `phase-1.5-plan.md`). Variants that have no need
    /// for the cross-respawn discriminator (currently only the test
    /// variant) return `None`, which the dispatch side treats as
    /// "always dispatch".
    ///
    /// **Phase 2+ contract**: every state-changing notification variant
    /// MUST return `Some(generation)` and have a matching arm in
    /// `set_service_generation` that calls `.set_generation()` on the
    /// payload. Side-effecting notifications (e.g. the upcoming
    /// `action.completed`, `push.event`, `OperationOutcome`) from a
    /// dying Service incarnation must not be applied to UI state
    /// belonging to the new incarnation - they would, for example, mark
    /// an action complete that the respawned action service never
    /// dispatched. Returning `None` from such a variant silently
    /// disables the cross-respawn guard and reintroduces the race scope
    /// item 20 closed. Routing through the `WithGeneration` trait means
    /// the payload struct must opt in (which forces the field to exist)
    /// and the get/set pair lives in adjacent methods so they cannot
    /// drift. The compiler enforces exhaustive match here, so adding a
    /// new variant without an arm is a compile error; choosing the
    /// wrong arm is a contract violation, not a compile error - hence
    /// this doc-comment gate.
    pub fn service_generation(&self) -> Option<u32> {
        match self {
            Self::BootProgress(progress) => Some(progress.generation()),
            Self::OperationOutcome(outcome) => Some(outcome.generation()),
            Self::ActionCompleted(completed) => Some(completed.generation()),
            #[cfg(test)]
            Self::TestEcho { .. } => None,
        }
    }

    /// Overwrite the Service-incarnation generation tag on this
    /// notification. Mirrors `service_generation` and **must** stay
    /// in sync with it: every variant whose `service_generation()` returns
    /// `Some(_)` must here delegate to `WithGeneration::set_generation`
    /// on the payload. The reader task in
    /// `crates/app/src/service_client.rs` calls this on every notification
    /// before enqueue.
    ///
    /// Variants that don't carry a generation field (test-only) are no-ops.
    pub fn set_service_generation(&mut self, generation: u32) {
        match self {
            Self::BootProgress(progress) => progress.set_generation(generation),
            Self::OperationOutcome(outcome) => outcome.set_generation(generation),
            Self::ActionCompleted(completed) => completed.set_generation(generation),
            #[cfg(test)]
            Self::TestEcho { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boot::BootPhase;
    use crate::framing::{ParsedServiceMessage, parse_service_message};

    #[test]
    fn test_echo_round_trips_through_serde() {
        let original = Notification::TestEcho {
            value: "hello".to_string(),
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: Notification = serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn test_echo_round_trips_through_parse_service_message() {
        // The wire envelope is `{jsonrpc, method, params}`; parse_service_message
        // takes the line and reconstructs `Notification`. Verifies that the
        // synthetic JSON object the parser builds matches the
        // `tag = "method", content = "params"` shape that serde expects on the
        // way back in.
        let line = r#"{"jsonrpc":"2.0","method":"test.echo","params":{"value":"hi"}}"#;
        let parsed = parse_service_message(line).expect("parse");
        match parsed {
            ParsedServiceMessage::Notification(Notification::TestEcho { value }) => {
                assert_eq!(value, "hi");
            }
            other => panic!("expected TestEcho notification, got {other:?}"),
        }
    }

    #[test]
    fn test_echo_classifies_as_coalesce() {
        let notification = Notification::TestEcho {
            value: "x".to_string(),
        };
        match notification.class() {
            NotificationClass::Coalesce { key } => assert_eq!(key, CoalesceKey::test("test.echo")),
            other => panic!("expected Coalesce, got {other:?}"),
        }
    }

    #[test]
    fn test_echo_method_name_is_dotted() {
        let notification = Notification::TestEcho {
            value: "x".to_string(),
        };
        assert_eq!(notification.method_name(), "test.echo");
    }

    #[test]
    fn boot_progress_round_trips_through_parse_service_message() {
        let line = r#"{"jsonrpc":"2.0","method":"boot.progress","params":{"phase":{"Migrating":{"current":3,"total":10}},"service_generation":7}}"#;
        let parsed = parse_service_message(line).expect("parse");
        match parsed {
            ParsedServiceMessage::Notification(Notification::BootProgress(progress)) => {
                assert_eq!(
                    progress.phase,
                    BootPhase::Migrating {
                        current: 3,
                        total: 10,
                    },
                );
                assert_eq!(progress.message, None);
                assert_eq!(progress.service_generation, 7);
            }
            other => panic!("expected BootProgress notification, got {other:?}"),
        }
    }

    #[test]
    fn boot_progress_classifies_per_phase() {
        let migrating_a = Notification::BootProgress(BootProgress {
            phase: BootPhase::Migrating {
                current: 1,
                total: 10,
            },
            message: None,
            service_generation: 0,
        });
        let migrating_b = Notification::BootProgress(BootProgress {
            phase: BootPhase::Migrating {
                current: 5,
                total: 10,
            },
            message: None,
            service_generation: 0,
        });
        let loading_key = Notification::BootProgress(BootProgress {
            phase: BootPhase::LoadingKey,
            message: None,
            service_generation: 0,
        });

        // Two `Migrating` updates collapse onto the same key, so the queue
        // replaces the older one with the newer.
        assert_eq!(migrating_a.class(), migrating_b.class());

        // `LoadingKey` keys onto a distinct discriminant; queue keeps both
        // entries.
        assert_ne!(migrating_a.class(), loading_key.class());
    }

    #[test]
    fn boot_progress_method_name_is_dotted() {
        let progress = Notification::BootProgress(BootProgress {
            phase: BootPhase::OpeningDatabase,
            message: None,
            service_generation: 0,
        });
        assert_eq!(progress.method_name(), "boot.progress");
    }

    #[test]
    fn service_generation_returns_payload_value_for_boot_progress() {
        let n = Notification::BootProgress(BootProgress {
            phase: BootPhase::OpeningDatabase,
            message: None,
            service_generation: 13,
        });
        assert_eq!(n.service_generation(), Some(13));
    }

    #[test]
    fn service_generation_is_none_for_variants_without_the_field() {
        let n = Notification::TestEcho {
            value: "x".to_string(),
        };
        assert_eq!(n.service_generation(), None);
    }

    #[test]
    fn operation_outcome_classifies_as_must_deliver() {
        use crate::action::{OperationId, OperationResult, PlanId};
        let outcome = Notification::OperationOutcome(crate::action::OperationOutcome {
            plan_id: PlanId::new_v7(),
            operation_id: OperationId(0),
            result: OperationResult::Success,
            service_generation: 0,
        });
        assert!(matches!(outcome.class(), NotificationClass::MustDeliver));
    }

    #[test]
    fn action_completed_classifies_as_must_deliver() {
        use crate::action::PlanId;
        let completed = Notification::ActionCompleted(crate::action::ActionCompleted {
            plan_id: PlanId::new_v7(),
            summary: crate::action::PlanSummary::default(),
            service_generation: 0,
        });
        assert!(matches!(completed.class(), NotificationClass::MustDeliver));
    }

    #[test]
    fn operation_outcome_method_name_is_dotted() {
        use crate::action::{OperationId, OperationResult, PlanId};
        let outcome = Notification::OperationOutcome(crate::action::OperationOutcome {
            plan_id: PlanId::new_v7(),
            operation_id: OperationId(0),
            result: OperationResult::Success,
            service_generation: 0,
        });
        assert_eq!(outcome.method_name(), "action.operation_outcome");
    }

    #[test]
    fn action_completed_method_name_is_dotted() {
        use crate::action::PlanId;
        let completed = Notification::ActionCompleted(crate::action::ActionCompleted {
            plan_id: PlanId::new_v7(),
            summary: crate::action::PlanSummary::default(),
            service_generation: 0,
        });
        assert_eq!(completed.method_name(), "action.completed");
    }

    #[test]
    fn operation_outcome_service_generation_round_trips() {
        use crate::action::{OperationId, OperationResult, PlanId};
        let mut n = Notification::OperationOutcome(crate::action::OperationOutcome {
            plan_id: PlanId::new_v7(),
            operation_id: OperationId(0),
            result: OperationResult::Success,
            service_generation: 1,
        });
        assert_eq!(n.service_generation(), Some(1));
        n.set_service_generation(99);
        assert_eq!(n.service_generation(), Some(99));
    }

    #[test]
    fn action_completed_service_generation_round_trips() {
        use crate::action::PlanId;
        let mut n = Notification::ActionCompleted(crate::action::ActionCompleted {
            plan_id: PlanId::new_v7(),
            summary: crate::action::PlanSummary::default(),
            service_generation: 1,
        });
        assert_eq!(n.service_generation(), Some(1));
        n.set_service_generation(99);
        assert_eq!(n.service_generation(), Some(99));
    }

    #[test]
    fn operation_outcome_round_trips_through_parse_service_message() {
        use crate::action::{OperationId, OperationResult, PlanId};

        let plan_id = PlanId::new_v7();
        let original = Notification::OperationOutcome(crate::action::OperationOutcome {
            plan_id,
            operation_id: OperationId(2),
            result: OperationResult::Success,
            service_generation: 5,
        });
        let json = serde_json::to_string(&original).expect("serialize");
        let line = format!(r#"{{"jsonrpc":"2.0",{}}}"#, &json[1..json.len() - 1]);
        let parsed = parse_service_message(&line).expect("parse");
        match parsed {
            ParsedServiceMessage::Notification(Notification::OperationOutcome(outcome)) => {
                assert_eq!(outcome.plan_id, plan_id);
                assert_eq!(outcome.operation_id, OperationId(2));
                assert_eq!(outcome.result, OperationResult::Success);
                assert_eq!(outcome.service_generation, 5);
            }
            other => panic!("expected OperationOutcome notification, got {other:?}"),
        }
    }

    #[test]
    fn coalesce_key_round_trips_through_serde() {
        let key = CoalesceKey::BootProgress(BootPhaseKind::Migrating);
        let json = serde_json::to_value(&key).expect("serialize");
        let recovered: CoalesceKey = serde_json::from_value(json).expect("deserialize");
        assert_eq!(key, recovered);
    }
}
