//! Contact + contact-group write handlers.
//!
//! Phase 6a (`group_save`, `group_delete`): user-facing group editor.
//! Phase 6a-part-2 (`contact_save`): local-only contact UPSERT, used by
//! the bulk-import path.
//! Phase 6d-A (`contact_save_with_writeback`, `contact_delete`): full
//! single-contact pipeline including provider write-back to JMAP /
//! Google People / Graph for synced contacts. Replaces the pre-6d
//! `service::actions::contacts::*` UI-side calls that ran through the
//! `action_ctx` field.
//!
//! The 6d-A handlers reuse `service::actions::contacts::save_contact`
//! and `delete_contact` (which carry the local-DB write +
//! `MutationLog` emission + provider dispatch). The handler builds an
//! `ActionContext` from `BootSharedState` (same construction as
//! `actions::worker::build_action_context`) and converts the resulting
//! `ActionOutcome` into the wire `WritebackOutcome` at the IPC
//! boundary - `ActionOutcome` lives in `action-types` and is not
//! serde-derive, so it never crosses the wire.

use std::sync::Arc;

use serde_json::Value;
use service_api::{
    ContactDeleteAck, ContactDeleteParams, ContactGroupDeleteAck, ContactGroupDeleteParams,
    ContactGroupSaveAck, ContactGroupSaveParams, ContactSaveAck, ContactSaveParams,
    ContactSaveWithWritebackAck, ServiceError, WritebackOutcome,
};

use crate::actions::worker::build_action_context;
use crate::boot::BootSharedState;
use action_types::ActionOutcome;

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

pub(crate) async fn handle_contact_save_with_writeback(
    boot_state: &Arc<BootSharedState>,
    params: ContactSaveParams,
) -> Result<Value, ServiceError> {
    let ctx = action_context(boot_state)?;
    // `account_color` and `groups` are dropped at this boundary -
    // matches the pre-6d UI-side path. Group membership is managed
    // separately through `contacts.group_save`; account_color is a
    // join-time read concern, not a column on `contacts`.
    let input = crate::actions::contacts::ContactSaveInput {
        id: params.id,
        email: params.email,
        display_name: params.display_name,
        email2: params.email2,
        phone: params.phone,
        company: params.company,
        notes: params.notes,
        account_id: params.account_id,
        source: params.source,
        server_id: params.server_id,
    };
    let outcome = crate::actions::contacts::save_contact(&ctx, input).await;
    let ack = ContactSaveWithWritebackAck {
        writeback: outcome_to_writeback(outcome)?,
    };
    serde_json::to_value(ack).map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_contact_delete(
    boot_state: &Arc<BootSharedState>,
    params: ContactDeleteParams,
) -> Result<Value, ServiceError> {
    let ctx = action_context(boot_state)?;
    let outcome = crate::actions::contacts::delete_contact(&ctx, &params.id).await;
    let ack = ContactDeleteAck {
        writeback: outcome_to_writeback(outcome)?,
    };
    serde_json::to_value(ack).map_err(|e| ServiceError::Internal(e.to_string()))
}

fn action_context(
    boot_state: &Arc<BootSharedState>,
) -> Result<action_types::ActionContext, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    let encryption_key = boot_state.encryption_key().ok_or_else(|| {
        ServiceError::Internal(
            "request received before encryption_key available; UI must wait for boot.ready".into(),
        )
    })?;
    let read_db = boot_state.read_db_state().ok_or_else(|| {
        ServiceError::Internal(
            "request received before read db available; UI must wait for boot.ready".into(),
        )
    })?;
    build_action_context(write_db, read_db, encryption_key, boot_state.app_data_dir())
        .map_err(ServiceError::Internal)
}

/// Map an `ActionOutcome` from `service::actions::contacts::*` to the
/// wire-friendly `WritebackOutcome`. `Failed` becomes a `ServiceError`
/// (the local DB write itself failed - the caller never sees a
/// `WritebackOutcome::Failed`); `Success` and `LocalOnly` map 1:1.
/// `NoOp` is unreachable from the contacts pipeline today but is
/// folded into `Success` for forward-compatibility.
fn outcome_to_writeback(outcome: ActionOutcome) -> Result<WritebackOutcome, ServiceError> {
    match outcome {
        ActionOutcome::Success | ActionOutcome::NoOp => Ok(WritebackOutcome::Success),
        ActionOutcome::LocalOnly { reason, retryable } => Ok(WritebackOutcome::LocalOnly {
            reason: reason.user_message(),
            retryable,
        }),
        ActionOutcome::Failed { error } => Err(ServiceError::Internal(error.user_message())),
    }
}
