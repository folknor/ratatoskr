//! `cal_action.execute_plan` handler.
//!
//! Phase 6c task 6c-4: validate the incoming `CalendarActionPlan`, journal
//! it atomically into `action_jobs` (kind = `'calendar_plan'`) plus
//! `action_job_ops`, and return `CalendarActionPlanAck { plan_id, journaled:
//! true }`. Mirror of the mail-side `action.execute_plan` handler in
//! `handlers/action.rs` - the dispatch loop sends the JSON-RPC response
//! only after the handler future returns, so the handler must NOT drive
//! the per-op dispatcher itself; it's just enqueue + ack. The action
//! worker (Phase 6c-7) wakes up on the signal and routes calendar_plan
//! kinds through `service::cal_actions::batch_execute`.
//!
//! `action_job_ops.thread_id` is `NOT NULL`, but calendar ops have
//! no thread. We overload the column to carry the calendar event id
//! per op (empty string for `CreateEvent`, where the id isn't
//! known until execution mints it). The contract is documented at
//! `db::action_journal::insert_calendar_plan` and at the wire type
//! site (`service-api::cal_action::CalendarActionWireOperation`).

use crate::boot::BootSharedState;
use db::db::action_journal::{PlanOpInsert, insert_calendar_plan};
use serde_json::Value;
use service_api::{CalendarActionPlan, CalendarActionPlanAck, ServiceError, WireCalendarOperation};
use std::sync::Arc;

pub(super) async fn handle(
    state: &Arc<BootSharedState>,
    plan: CalendarActionPlan,
) -> Result<Value, ServiceError> {
    validate_plan(&plan)?;
    let plan_id = plan.plan_id;
    let account_id = plan.operations[0].account_id.clone();
    let db = state.write_db_state()?;
    let plan_id_bytes = *plan_id.0.as_bytes();
    let ops = serialize_ops(plan)?;

    db.with_conn(move |conn| insert_calendar_plan(conn, &plan_id_bytes, &account_id, &ops))
    .await
    .map_err(ServiceError::Internal)?;

    state.notify_action_worker();

    let ack = CalendarActionPlanAck {
        plan_id,
        journaled: true,
    };
    serde_json::to_value(&ack).map_err(|e| ServiceError::Internal(e.to_string()))
}

/// Same shape as the mail-side `validate_plan`, narrowed for calendar:
/// at-least-one operation, single-account scope, unique operation_ids.
fn validate_plan(plan: &CalendarActionPlan) -> Result<(), ServiceError> {
    if plan.operations.is_empty() {
        return Err(ServiceError::InvalidParams {
            method: "cal_action.execute_plan".into(),
            message: "plan has no operations".into(),
        });
    }
    let account_id = &plan.operations[0].account_id;
    let mut seen_op_ids = std::collections::HashSet::new();
    for op in &plan.operations {
        if &op.account_id != account_id {
            return Err(ServiceError::InvalidParams {
                method: "cal_action.execute_plan".into(),
                message: format!(
                    "plan crosses accounts: {} != {}",
                    op.account_id, account_id
                ),
            });
        }
        if !seen_op_ids.insert(op.operation_id) {
            return Err(ServiceError::InvalidParams {
                method: "cal_action.execute_plan".into(),
                message: format!(
                    "duplicate operation_id {:?} in plan",
                    op.operation_id
                ),
            });
        }
    }
    Ok(())
}

/// Serialize each op into a `PlanOpInsert`. The `thread_id` column is
/// overloaded (see module doc): we store the calendar event id where
/// the op carries one, otherwise the empty string.
fn serialize_ops(plan: CalendarActionPlan) -> Result<Vec<PlanOpInsert>, ServiceError> {
    let mut out = Vec::with_capacity(plan.operations.len());
    for (ordinal, op) in plan.operations.into_iter().enumerate() {
        let event_id_for_op = match &op.operation {
            WireCalendarOperation::CreateEvent { input, .. } => {
                // Round-trip the input through the wire_input_to_domain
                // converter to enforce the mirror contract; we drop
                // the result. Mirrors mail's `wire_to_mail` step in
                // `actions/serialize_ops`. A stale wire variant
                // missing a required field would otherwise journal a
                // blob the worker can't reconstruct.
                let _ = crate::cal_actions::wire_input_to_domain(input);
                String::new()
            }
            WireCalendarOperation::UpdateEvent { event_id, input } => {
                let _ = crate::cal_actions::wire_input_to_domain(input);
                event_id.clone()
            }
            WireCalendarOperation::DeleteEvent { event_id } => event_id.clone(),
        };
        let blob = serde_json::to_vec(&op.operation).map_err(|e| {
            ServiceError::Internal(format!(
                "serialize WireCalendarOperation for op {:?}: {e}",
                op.operation_id
            ))
        })?;
        let ordinal_u32 = u32::try_from(ordinal).map_err(|_| {
            ServiceError::Internal(format!(
                "plan has more than u32::MAX operations ({ordinal})",
            ))
        })?;
        out.push(PlanOpInsert {
            operation_id: op.operation_id.0,
            ordinal: ordinal_u32,
            thread_id: event_id_for_op,
            operation_blob: blob,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_api::{
        CalendarActionWireOperation, OperationId, PlanId, WireCalendarEventInput,
    };

    fn op(account: &str, op_id: u32, op: WireCalendarOperation) -> CalendarActionWireOperation {
        CalendarActionWireOperation {
            operation_id: OperationId(op_id),
            account_id: account.into(),
            operation: op,
        }
    }

    fn make_input() -> WireCalendarEventInput {
        WireCalendarEventInput {
            title: "T".into(),
            description: String::new(),
            location: String::new(),
            start_time: 0,
            end_time: 0,
            is_all_day: false,
            timezone: None,
            recurrence_rule: None,
            availability: None,
            visibility: None,
        }
    }

    #[test]
    fn validate_rejects_empty_plan() {
        let plan = CalendarActionPlan {
            plan_id: PlanId::new_v7(),
            operations: vec![],
        };
        let err = validate_plan(&plan).expect_err("empty plan should be rejected");
        match err {
            ServiceError::InvalidParams { message, .. } => {
                assert!(message.contains("no operations"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn validate_rejects_cross_account_plan() {
        let plan = CalendarActionPlan {
            plan_id: PlanId::new_v7(),
            operations: vec![
                op(
                    "acc-1",
                    0,
                    WireCalendarOperation::DeleteEvent {
                        event_id: "e1".into(),
                    },
                ),
                op(
                    "acc-2",
                    1,
                    WireCalendarOperation::DeleteEvent {
                        event_id: "e2".into(),
                    },
                ),
            ],
        };
        let err = validate_plan(&plan).expect_err("cross-account plan should be rejected");
        match err {
            ServiceError::InvalidParams { message, .. } => {
                assert!(message.contains("crosses accounts"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn validate_rejects_duplicate_operation_ids() {
        let plan = CalendarActionPlan {
            plan_id: PlanId::new_v7(),
            operations: vec![
                op(
                    "acc-1",
                    0,
                    WireCalendarOperation::DeleteEvent {
                        event_id: "e1".into(),
                    },
                ),
                op(
                    "acc-1",
                    0,
                    WireCalendarOperation::DeleteEvent {
                        event_id: "e2".into(),
                    },
                ),
            ],
        };
        let err = validate_plan(&plan).expect_err("duplicate op_id should be rejected");
        match err {
            ServiceError::InvalidParams { message, .. } => {
                assert!(message.contains("duplicate operation_id"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn validate_accepts_well_formed_plan() {
        let plan = CalendarActionPlan {
            plan_id: PlanId::new_v7(),
            operations: vec![
                op(
                    "acc-1",
                    0,
                    WireCalendarOperation::CreateEvent {
                        calendar_remote_id: "primary".into(),
                        input: make_input(),
                    },
                ),
                op(
                    "acc-1",
                    1,
                    WireCalendarOperation::UpdateEvent {
                        event_id: "e1".into(),
                        input: make_input(),
                    },
                ),
            ],
        };
        validate_plan(&plan).expect("well-formed plan");
    }

    #[test]
    fn serialize_ops_overloads_thread_id_with_event_id() {
        let plan = CalendarActionPlan {
            plan_id: PlanId::new_v7(),
            operations: vec![
                op(
                    "acc-1",
                    0,
                    WireCalendarOperation::CreateEvent {
                        calendar_remote_id: "primary".into(),
                        input: make_input(),
                    },
                ),
                op(
                    "acc-1",
                    1,
                    WireCalendarOperation::UpdateEvent {
                        event_id: "evt-42".into(),
                        input: make_input(),
                    },
                ),
                op(
                    "acc-1",
                    2,
                    WireCalendarOperation::DeleteEvent {
                        event_id: "evt-43".into(),
                    },
                ),
            ],
        };
        let inserts = serialize_ops(plan).expect("serialize");
        assert_eq!(inserts.len(), 3);
        assert_eq!(inserts[0].thread_id, "");
        assert_eq!(inserts[1].thread_id, "evt-42");
        assert_eq!(inserts[2].thread_id, "evt-43");
    }
}
