//! `settings.set` request handler (Phase 6a).
//!
//! Writes one or more settings rows in a single atomic transaction.
//! Per-variant `key()` and `render_for_storage()` live on the wire
//! type itself; the handler's job is the boundary crossing + the
//! transaction.

use std::sync::Arc;

use serde_json::Value;
use service_api::{ServiceError, SettingsSetAck, SettingsSetParams};

use crate::boot::BootSharedState;

pub(crate) async fn handle_set(
    boot_state: &Arc<BootSharedState>,
    params: SettingsSetParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    write_db
        .with_conn(move |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("settings.set begin tx: {e}"))?;
            for value in &params.values {
                let key = value.key();
                let storage_value = value.render_for_storage();
                rtsk::db::queries::set_setting(&tx, key, &storage_value)
                    .map_err(|e| format!("settings.set {key}: {e}"))?;
            }
            tx.commit()
                .map_err(|e| format!("settings.set commit: {e}"))?;
            Ok(())
        })
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(SettingsSetAck)
        .map_err(|e| ServiceError::Internal(e.to_string()))
}
