//! Service handlers for label_group IPC.

use std::sync::Arc;

use serde_json::Value;
use service_api::{LabelGroupReorderAck, LabelGroupReorderParams, ServiceError};

use crate::boot::BootSharedState;

pub(crate) async fn handle_reorder(
    boot_state: &Arc<BootSharedState>,
    params: LabelGroupReorderParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    write_db
        .with_conn(move |conn| {
            db::db::queries_extra::label_groups::update_label_group_sort_order_sync(
                conn,
                &params.orders,
            )
        })
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(LabelGroupReorderAck)
        .map_err(|e| ServiceError::Internal(e.to_string()))
}
