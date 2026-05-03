//! `action.execute_plan` handler.
//!
//! Phase 2 task 9b: validate the incoming `ActionWirePlan`, journal it
//! atomically into `action_jobs` + `action_job_ops`, and return
//! `ActionPlanAck { plan_id, journaled: true }`. This is the handler
//! half of the handler-vs-worker split (per Phase 2 plan scope item 3):
//! the dispatch loop sends the JSON-RPC response only after the
//! handler future returns, so the handler must NOT drive
//! `batch_execute` itself - it's just enqueue + ack.
//!
//! The worker that actually executes ops lands in task 9c. Until then
//! the handler journals plans and they accumulate; the in-process
//! tests cover the handler's contract (validate + journal + ack)
//! independently of execution. The wakeup signal gets wired in when
//! the worker lands; handler is forward-compatible (it just needs to
//! call `boot_state.notify_action_worker()` on success).

use crate::actions::wire_conversion::wire_to_mail;
use crate::boot::BootSharedState;
use db::db::action_journal::{PlanOpInsert, insert_mail_plan};
use serde_json::Value;
use service_api::{ActionPlanAck, ActionWirePlan, ServiceError};
use std::sync::Arc;

pub(super) async fn handle(
    state: &Arc<BootSharedState>,
    plan: ActionWirePlan,
) -> Result<Value, ServiceError> {
    validate_plan(&plan)?;
    let plan_id = plan.plan_id;
    let account_id = plan.operations[0].account_id.clone();
    let conn = state
        .db_conn()
        .ok_or_else(|| ServiceError::Internal("boot context not populated".into()))?;
    let plan_id_bytes = *plan_id.0.as_bytes();
    let ops = serialize_ops(plan)?;

    tokio::task::spawn_blocking(move || {
        let conn = conn
            .lock()
            .map_err(|e| format!("db lock poisoned: {e}"))?;
        insert_mail_plan(&conn, &plan_id_bytes, &account_id, false, &ops)
    })
    .await
    .map_err(|e| ServiceError::Internal(format!("spawn_blocking: {e}")))?
    .map_err(ServiceError::Internal)?;

    // Worker wakeup wires in with task 9c. For now, the journal rows
    // sit until the worker is added.
    state.notify_action_worker();

    let ack = ActionPlanAck {
        plan_id,
        journaled: true,
    };
    serde_json::to_value(&ack).map_err(|e| ServiceError::Internal(e.to_string()))
}

/// Validate that the plan is internally consistent before journaling.
///
/// Per the Phase 2 plan, an `action.execute_plan` request:
/// - must have at least one operation (an empty plan is meaningless),
/// - must have all ops scoped to a single account (per-plan invariant
///   that the worker's per-account semaphore policy and the journal's
///   `account_id` column rely on),
/// - must have unique `operation_id` per plan (the journal's PK
///   `(job_id, operation_id)` would also catch this, but a clean
///   InvalidParams beats a SQL constraint violation in the response).
fn validate_plan(plan: &ActionWirePlan) -> Result<(), ServiceError> {
    if plan.operations.is_empty() {
        return Err(ServiceError::InvalidParams {
            method: "action.execute_plan".into(),
            message: "plan has no operations".into(),
        });
    }
    let account_id = &plan.operations[0].account_id;
    let mut seen_op_ids = std::collections::HashSet::new();
    for op in &plan.operations {
        if &op.account_id != account_id {
            return Err(ServiceError::InvalidParams {
                method: "action.execute_plan".into(),
                message: format!(
                    "plan crosses accounts: {} != {}",
                    op.account_id, account_id
                ),
            });
        }
        if !seen_op_ids.insert(op.operation_id) {
            return Err(ServiceError::InvalidParams {
                method: "action.execute_plan".into(),
                message: format!(
                    "duplicate operation_id {:?} in plan",
                    op.operation_id
                ),
            });
        }
    }
    Ok(())
}

/// Convert each `ActionWireOperation` into a `PlanOpInsert` carrying
/// the serialized `WireMailOperation` blob the journal stores.
///
/// The conversion goes wire -> domain -> wire so the
/// `wire_conversion::wire_to_mail` exhaustive-match guard is exercised
/// at handler time: a future `WireMailOperation` variant added without
/// a matching `MailOperation` variant fails to compile here, not later
/// at the worker. The journaled blob is the wire form (which is
/// what the worker re-deserializes).
fn serialize_ops(plan: ActionWirePlan) -> Result<Vec<PlanOpInsert>, ServiceError> {
    let mut out = Vec::with_capacity(plan.operations.len());
    for op in plan.operations {
        // Round-trip through wire_to_mail to enforce the mirror
        // contract; we drop the result (the journal stores the wire
        // form). This is cheap (each variant is a few bytes) and
        // ensures we never journal a plan that the worker can't
        // deserialize.
        let _ = wire_to_mail(op.operation.clone());

        let blob = serde_json::to_vec(&op.operation).map_err(|e| {
            ServiceError::Internal(format!(
                "serialize WireMailOperation for op {:?}: {e}",
                op.operation_id
            ))
        })?;
        out.push(PlanOpInsert {
            operation_id: op.operation_id.0,
            ordinal: op.operation_id.0,
            thread_id: op.thread_id,
            operation_blob: blob,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_api::{
        ActionWireOperation, OperationId, PlanId, WireFolderId, WireMailOperation,
    };

    fn op(account: &str, op_id: u32, op: WireMailOperation) -> ActionWireOperation {
        ActionWireOperation {
            operation_id: OperationId(op_id),
            account_id: account.into(),
            thread_id: format!("thr-{op_id}"),
            operation: op,
        }
    }

    #[test]
    fn validate_rejects_empty_plan() {
        let plan = ActionWirePlan {
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
        let plan = ActionWirePlan {
            plan_id: PlanId::new_v7(),
            operations: vec![
                op("acc-1", 0, WireMailOperation::Archive),
                op("acc-2", 1, WireMailOperation::Archive),
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
        let plan = ActionWirePlan {
            plan_id: PlanId::new_v7(),
            operations: vec![
                op("acc-1", 0, WireMailOperation::Archive),
                op("acc-1", 0, WireMailOperation::Trash),
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
        let plan = ActionWirePlan {
            plan_id: PlanId::new_v7(),
            operations: vec![
                op("acc-1", 0, WireMailOperation::Archive),
                op("acc-1", 1, WireMailOperation::Trash),
                op(
                    "acc-1",
                    2,
                    WireMailOperation::MoveToFolder {
                        dest: WireFolderId("inbox".into()),
                        source: None,
                    },
                ),
            ],
        };
        validate_plan(&plan).expect("well-formed plan");
    }

    #[test]
    fn serialize_ops_round_trips_through_wire_form() {
        let plan = ActionWirePlan {
            plan_id: PlanId::new_v7(),
            operations: vec![op("acc-1", 0, WireMailOperation::Archive)],
        };
        let inserts = serialize_ops(plan).expect("serialize");
        assert_eq!(inserts.len(), 1);
        let recovered: WireMailOperation =
            serde_json::from_slice(&inserts[0].operation_blob).expect("deserialize");
        assert_eq!(recovered, WireMailOperation::Archive);
    }
}
