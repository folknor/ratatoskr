//! `thread_ui_state.set` request handler (Phase 6a).
//!
//! Thin wrapper around `db::queries_extra::set_attachments_collapsed`.
//! Today's only field is `attachments_collapsed`; if the wire param's
//! `attachments_collapsed` is `Some(value)`, the handler upserts to
//! that value. `None` is reserved for partial-update extensibility
//! when more thread-scoped fields land here later (Phase 6c calendar
//! event mutations could land first; today's implementation just
//! returns success on a no-op `None`).

use std::sync::Arc;

use serde_json::Value;
use service_api::{ServiceError, ThreadUiStateSetAck, ThreadUiStateSetParams};

use crate::boot::BootSharedState;

pub(crate) async fn handle_set(
    boot_state: &Arc<BootSharedState>,
    params: ThreadUiStateSetParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    if let Some(collapsed) = params.attachments_collapsed {
        write_db
            .with_conn(move |conn| {
                db::db::queries_extra::set_attachments_collapsed(
                    conn,
                    &params.account_id,
                    &params.thread_id,
                    collapsed,
                )
            })
            .await
            .map_err(ServiceError::Internal)?;
    }
    serde_json::to_value(ThreadUiStateSetAck)
        .map_err(|e| ServiceError::Internal(e.to_string()))
}
