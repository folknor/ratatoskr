//! Smart-folder Service handler (Phase 6a-part-2).
//!
//! Single method - `smart_folder.create` - relocates the
//! "save current search as smart folder" path Service-side. Sibling
//! to `pinned_search` handlers; lives in its own module because
//! `smart_folders` is a different table and future smart-folder
//! surfaces (update / delete / icon-color edit) will land here.
//!
//! UUID minting moved Service-side from `app/src/db/pinned_searches.rs`.
//! The id is not UI-observable today (the UI re-lists after save), but
//! the wire ack carries it so future "open the new folder" affordances
//! can plug in without a second round-trip.

use std::sync::Arc;

use serde_json::Value;
use service_api::{ServiceError, SmartFolderCreateAck, SmartFolderCreateParams};

use crate::boot::BootSharedState;

pub(crate) async fn handle_create(
    boot_state: &Arc<BootSharedState>,
    params: SmartFolderCreateParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    let id = uuid::Uuid::new_v4().to_string();
    let id_for_db = id.clone();
    write_db
        .with_conn(move |conn| {
            // Default icon `"search"` and no color / no account scope
            // matches today's UI-side defaults (`app/src/db/
            // pinned_searches.rs:122` before this commit). Future
            // tightening: a `SmartFolderCreateParams` with optional
            // icon/color/account_id is a one-line wire change.
            db::db::queries_extra::db_insert_smart_folder_sync(
                conn,
                &id_for_db,
                &params.name,
                &params.query,
                None,
                None,
                None,
            )
        })
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(SmartFolderCreateAck { id })
        .map_err(|e| ServiceError::Internal(e.to_string()))
}
