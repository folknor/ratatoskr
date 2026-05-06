//! Calendar action dispatcher - the Service-side write path for calendar
//! event mutations.
//!
//! Phase 6c task 6c-6: this module is the calendar pipeline's
//! `batch_execute`. The action worker (Phase 6c-7) reads the journaled
//! `kind = 'calendar_plan'` row, deserialises each
//! `WireCalendarOperation` blob, builds a `CalendarActionContext` from
//! the boot-shared writer-half + encryption key, and calls
//! `batch_execute`. Per-op `CalendarOperationOutcome` notifications and
//! the per-plan `CalendarActionCompleted` are emitted on the way back.
//!
//! `CalendarOperationOutcome` is `MustDeliver` class (see
//! `service-api::Notification`); the UI's `pending_calendar_action_plans`
//! map (Phase 6c-9) keys on `plan_id` and unblocks the awaiting caller
//! when the matching `CalendarActionCompleted` arrives. Phase 5 used
//! the latch pattern for `CalendarRunCompleted` to dodge the
//! late-subscriber race; this pipeline reuses the same shape.
//!
//! `ActionOutcome` is the in-process domain type returned by
//! `cal::actions::*`; it is converted to the wire-narrow
//! `CalendarOperationResult` at the IPC boundary inside this
//! function. Mail's `OperationResult` has the rich
//! `RemoteFailure { http_status, retryable }` taxonomy because the
//! mail action pipeline classifies provider errors that way; the
//! calendar action pipeline returns `ActionOutcome::LocalOnly { reason,
//! retryable }` on provider failure for `CreateEvent` and a flat
//! `ActionOutcome::Failed { error }` for `Update` / `Delete`.
//! `CalendarOperationResult` mirrors that narrower taxonomy.

use action_types::{ActionOutcome, CalendarActionContext};
use cal::actions::{
    CalendarEventInput, create_calendar_event, delete_calendar_event, update_calendar_event,
};
use service_api::{
    CalendarActionWireOperation, CalendarOperationResult, WireCalendarEventInput,
    WireCalendarOperation,
};

/// Run every operation in `ops` sequentially, returning per-op
/// results in original order.
///
/// Calendar plans today are 1:1 (one user intent = one operation),
/// so the sequential loop is exactly right. The shape mirrors mail's
/// `batch_execute` so that the future Phase 6d work (RSVP /
/// series-vs-occurrence) can layer in N-op plans without a structural
/// refactor.
pub async fn batch_execute(
    ctx: &CalendarActionContext,
    ops: Vec<CalendarActionWireOperation>,
) -> Vec<CalendarOperationResult> {
    let mut out = Vec::with_capacity(ops.len());
    for op in ops {
        let outcome = run_one(ctx, &op).await;
        out.push(outcome_to_wire(outcome));
    }
    out
}

/// Dispatch one operation to the matching `cal::actions::*` function.
async fn run_one(
    ctx: &CalendarActionContext,
    op: &CalendarActionWireOperation,
) -> ActionOutcome {
    match &op.operation {
        WireCalendarOperation::CreateEvent {
            calendar_remote_id,
            input,
        } => {
            create_calendar_event(
                ctx,
                &op.account_id,
                calendar_remote_id,
                wire_input_to_domain(input),
            )
            .await
        }
        WireCalendarOperation::UpdateEvent { event_id, input } => {
            update_calendar_event(ctx, &op.account_id, event_id, wire_input_to_domain(input)).await
        }
        WireCalendarOperation::DeleteEvent { event_id } => {
            delete_calendar_event(ctx, &op.account_id, event_id).await
        }
    }
}

/// Convert the wire shape to the in-process domain shape. The two
/// have identical fields - the wire mirror exists so service-api
/// stays free of cal's transitive provider-trait graph.
fn wire_input_to_domain(input: &WireCalendarEventInput) -> CalendarEventInput {
    CalendarEventInput {
        title: input.title.clone(),
        description: input.description.clone(),
        location: input.location.clone(),
        start_time: input.start_time,
        end_time: input.end_time,
        is_all_day: input.is_all_day,
        timezone: input.timezone.clone(),
        recurrence_rule: input.recurrence_rule.clone(),
        availability: input.availability.clone(),
        visibility: input.visibility.clone(),
    }
}

/// Convert the in-process `ActionOutcome` to the wire-narrow
/// `CalendarOperationResult`. `LocalOnly` is reachable only for
/// `CreateEvent`; the mapping is exhaustive so a future variant on
/// either side surfaces here as a compile error.
fn outcome_to_wire(outcome: ActionOutcome) -> CalendarOperationResult {
    match outcome {
        ActionOutcome::Success | ActionOutcome::NoOp => CalendarOperationResult::Success,
        ActionOutcome::LocalOnly { reason, .. } => CalendarOperationResult::LocalOnly {
            reason: reason.user_message(),
        },
        ActionOutcome::Failed { error } => CalendarOperationResult::Failed {
            error: error.user_message(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use action_types::ActionError;

    #[test]
    fn outcome_to_wire_maps_success() {
        let result = outcome_to_wire(ActionOutcome::Success);
        assert!(matches!(result, CalendarOperationResult::Success));
    }

    #[test]
    fn outcome_to_wire_maps_local_only_with_reason() {
        let result = outcome_to_wire(ActionOutcome::LocalOnly {
            reason: ActionError::remote("provider 503"),
            retryable: true,
        });
        match result {
            CalendarOperationResult::LocalOnly { reason } => {
                assert!(reason.contains("provider 503"));
            }
            other => panic!("unexpected mapping: {other:?}"),
        }
    }

    #[test]
    fn outcome_to_wire_maps_failed_to_user_message() {
        let result = outcome_to_wire(ActionOutcome::Failed {
            error: ActionError::not_found("calendar 404"),
        });
        assert!(matches!(result, CalendarOperationResult::Failed { .. }));
    }
}
