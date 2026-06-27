use std::sync::Arc;

use serde_json::Value;
use service_api::{
    RescheduleSendParams, ScheduledSendAck, ScheduledSendHandleParams, ServiceError,
};

use crate::boot::BootSharedState;

pub(super) async fn handle_cancel(
    state: &Arc<BootSharedState>,
    params: ScheduledSendHandleParams,
) -> Result<Value, ServiceError> {
    let ctx = action_context(state)?;
    let sync_runtime = state.sync_runtime();
    match crate::actions::cancel_scheduled_send(
        &ctx,
        sync_runtime.as_deref(),
        &params.account_id,
        &params.remote_message_id,
    )
    .await
    {
        crate::actions::ActionOutcome::Success | crate::actions::ActionOutcome::NoOp => {
            serde_json::to_value(ScheduledSendAck { ok: true })
                .map_err(|error| ServiceError::Internal(error.to_string()))
        }
        crate::actions::ActionOutcome::Failed { error }
        | crate::actions::ActionOutcome::LocalOnly { reason: error, .. } => {
            Err(ServiceError::Internal(error.user_message()))
        }
    }
}

pub(super) async fn handle_reschedule(
    state: &Arc<BootSharedState>,
    params: RescheduleSendParams,
) -> Result<Value, ServiceError> {
    let ctx = action_context(state)?;
    let sync_runtime = state.sync_runtime();
    match crate::actions::reschedule_send(
        &ctx,
        sync_runtime.as_deref(),
        &params.account_id,
        &params.remote_message_id,
        params.scheduled_at,
    )
    .await
    {
        crate::actions::ActionOutcome::Success | crate::actions::ActionOutcome::NoOp => {
            serde_json::to_value(ScheduledSendAck { ok: true })
                .map_err(|error| ServiceError::Internal(error.to_string()))
        }
        crate::actions::ActionOutcome::Failed { error }
        | crate::actions::ActionOutcome::LocalOnly { reason: error, .. } => {
            Err(ServiceError::Internal(error.user_message()))
        }
    }
}

fn action_context(
    state: &Arc<BootSharedState>,
) -> Result<crate::actions::ActionContext, ServiceError> {
    crate::actions::worker::build_action_context(
        state.write_db_state()?,
        state
            .read_db_state()
            .ok_or_else(|| ServiceError::Internal("read DB is not initialized".into()))?,
        state
            .encryption_key()
            .ok_or_else(|| ServiceError::Internal("encryption key is not loaded".into()))?,
        state.app_data_dir(),
    )
    .map_err(ServiceError::Internal)
}
