//! Contact-group write handlers (Phase 6a).
//!
//! Two methods - `contacts.group_save` and `contacts.group_delete` -
//! relocate the user-facing group editor's writes Service-side. Both
//! delegate to the existing `*_sync` helpers in
//! `db::queries_extra::contact_groups`; the helpers handle the
//! transactional UPSERT + member replace and the cascading delete.
//!
//! Second CRUD instance after `signature.*` - the shape is locked in
//! by the per-surface checklist: thin handler, one
//! `WriteDbState::with_conn`, named ack struct, no Message variant
//! reuse on the UI side.
//!
//! `save_contact` (Phase 6a-part-2): UPSERT one contact row. Both
//! the UI single-contact save handler and the bulk-import path
//! issue this IPC, one call per contact. Provider write-back
//! (Google / Graph / CardDAV) still routes through the action
//! service - this surface only covers the local-DB UPSERT that the
//! pre-relocation `Db::save_contact` was performing UI-side.
//!
//! `delete_contact` is out of scope for this handler: it routes
//! through the action service for provider write-back and needs a
//! different relocation pattern than the simple-write surfaces in 6a.

use std::sync::Arc;

use serde_json::Value;
use service_api::{
    ContactGroupDeleteAck, ContactGroupDeleteParams, ContactGroupSaveAck, ContactGroupSaveParams,
    ContactSaveAck, ContactSaveParams, ServiceError,
};

use crate::boot::BootSharedState;

pub(crate) async fn handle_group_save(
    boot_state: &Arc<BootSharedState>,
    params: ContactGroupSaveParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    write_db
        .with_conn(move |conn| {
            let entry = db::db::queries_extra::GroupSettingsEntry {
                id: params.id,
                name: params.name,
                member_count: params.member_count,
                created_at: params.created_at,
                updated_at: params.updated_at,
            };
            db::db::queries_extra::save_group_sync(conn, &entry, &params.member_emails)
        })
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(ContactGroupSaveAck)
        .map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_group_delete(
    boot_state: &Arc<BootSharedState>,
    params: ContactGroupDeleteParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    write_db
        .with_conn(move |conn| db::db::queries_extra::delete_group_sync(conn, &params.id))
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(ContactGroupDeleteAck)
        .map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_contact_save(
    boot_state: &Arc<BootSharedState>,
    params: ContactSaveParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    write_db
        .with_conn(move |conn| {
            let entry = db::db::queries_extra::ContactSettingsEntry {
                id: params.id,
                email: params.email,
                display_name: params.display_name,
                email2: params.email2,
                phone: params.phone,
                company: params.company,
                notes: params.notes,
                account_id: params.account_id,
                account_color: params.account_color,
                groups: params.groups,
                source: params.source,
                server_id: params.server_id,
            };
            db::db::queries_extra::save_contact_sync(conn, &entry)
        })
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(ContactSaveAck).map_err(|e| ServiceError::Internal(e.to_string()))
}
