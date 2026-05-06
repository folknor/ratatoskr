//! Account write handlers (Phase 6a).
//!
//! `account.update` and `account.reorder` are the small / non-envelope
//! surfaces. The bigger account operations live in their own modules:
//!
//! - `account.create` (Plaintext | Encrypted credential envelope)
//! - `account.delete` (cancel-and-await runner orchestration)
//!
//! ...both will land alongside their own ack types and timeouts so
//! the handler doc here stays scoped to the simple-write path.
//!
//! Today's caldav_password column stores the value verbatim (no
//! encryption); when `internal.encrypt_for_storage` lands the wire
//! shape stays unchanged but this handler can route the value through
//! the cipher before writing.

use std::sync::Arc;

use serde_json::Value;
use service_api::{
    AccountReorderAck, AccountReorderParams, AccountUpdateAck, AccountUpdateParams, ServiceError,
};

use crate::boot::BootSharedState;

pub(crate) async fn handle_update(
    boot_state: &Arc<BootSharedState>,
    params: AccountUpdateParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    write_db
        .with_conn(move |conn| {
            let id = params.id;
            let update = db::db::queries_extra::UpdateAccountParams {
                account_name: params.account_name,
                display_name: params.display_name,
                account_color: params.account_color,
                caldav_url: params.caldav_url,
                caldav_username: params.caldav_username,
                caldav_password: params.caldav_password,
            };
            db::db::queries_extra::update_account_sync(conn, &id, update)
        })
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(AccountUpdateAck)
        .map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_reorder(
    boot_state: &Arc<BootSharedState>,
    params: AccountReorderParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    write_db
        .with_conn(move |conn| {
            let updates: Vec<(String, i64)> = params
                .orders
                .into_iter()
                .map(|e| (e.account_id, e.sort_order))
                .collect();
            db::db::queries_extra::update_account_sort_order_sync(conn, &updates)
        })
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(AccountReorderAck)
        .map_err(|e| ServiceError::Internal(e.to_string()))
}
