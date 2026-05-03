//! `action.job_status` handler.
//!
//! Backs Phase 2 plan scope item 18d: the UI's `AckUnknown`
//! reconciliation path calls this after every `boot.ready` post-respawn
//! for plans whose `ActionPlanAck` was lost on the wire. The Service
//! looks up the journal row for `plan_id` and returns either
//! `JobStatusResponse::NotFound` (no row -> UI rolls back optimistic
//! state) or `JobStatusResponse::Journaled { status, summary }` (row
//! exists -> UI keeps optimistic state and lets journal-driven replay
//! drive completion).
//!
//! Read-only SELECT against `action_jobs`. Bypasses neither the
//! per-handler semaphore nor the dispatch admission cap - it's a fast
//! query and the contention is bounded by the cap itself.

use crate::boot::BootSharedState;
use db::db::action_journal::{JobStatus, query_job_status};
use serde_json::Value;
use service_api::{JobStatusResponse, PlanId, ServiceError, WireJobStatus};
use std::sync::Arc;

pub(super) async fn handle(
    state: &Arc<BootSharedState>,
    plan_id: PlanId,
) -> Result<Value, ServiceError> {
    let conn = state
        .db_conn()
        .ok_or_else(|| ServiceError::Internal("boot context not populated".into()))?;
    let job_id_bytes = *plan_id.0.as_bytes();

    let snapshot = tokio::task::spawn_blocking(move || {
        let conn = conn
            .lock()
            .map_err(|error| format!("db lock poisoned: {error}"))?;
        query_job_status(&conn, &job_id_bytes)
    })
    .await
    .map_err(|error| ServiceError::Internal(format!("spawn_blocking: {error}")))?
    .map_err(ServiceError::Internal)?;

    let response = match snapshot {
        None => JobStatusResponse::NotFound,
        Some(snap) => JobStatusResponse::Journaled {
            status: db_status_to_wire(snap.status),
            summary: snap.summary,
        },
    };

    serde_json::to_value(&response).map_err(|error| ServiceError::Internal(error.to_string()))
}

fn db_status_to_wire(status: JobStatus) -> WireJobStatus {
    match status {
        JobStatus::Queued => WireJobStatus::Queued,
        JobStatus::Leased => WireJobStatus::Leased,
        JobStatus::Executing => WireJobStatus::Executing,
        JobStatus::Completed => WireJobStatus::Completed,
        JobStatus::Partial => WireJobStatus::Partial,
        JobStatus::Failed => WireJobStatus::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_status_to_wire_round_trips_every_variant() {
        // Exhaustive match here is the regression guard: adding a
        // `JobStatus` variant without a `WireJobStatus` mirror is a
        // compile error. The test body just exercises the mapping.
        for (db, wire) in [
            (JobStatus::Queued, WireJobStatus::Queued),
            (JobStatus::Leased, WireJobStatus::Leased),
            (JobStatus::Executing, WireJobStatus::Executing),
            (JobStatus::Completed, WireJobStatus::Completed),
            (JobStatus::Partial, WireJobStatus::Partial),
            (JobStatus::Failed, WireJobStatus::Failed),
        ] {
            assert_eq!(db_status_to_wire(db), wire);
        }
    }
}
