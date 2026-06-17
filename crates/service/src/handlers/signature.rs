//! Signature CRUD request handlers (Phase 6a).
//!
//! Four methods - `signature.create`, `signature.update`,
//! `signature.delete`, `signature.reorder` - establish the CRUD shape
//! that contacts/groups will copy. Each handler is a thin wrapper:
//! convert the wire `Params` into the underlying `*_sync` DB
//! function's argument shape, run inside one `WriteDbState::with_write`,
//! return the named ack struct.
//!
//! The per-account "exactly one is_default / is_reply_default"
//! invariant is enforced inside the same DB transaction by the sync
//! helpers, so callers cannot observe a partial commit even if the
//! Service crashes mid-write.

use std::sync::Arc;

use serde_json::Value;
use service_api::{
    ServiceError, SignatureCreateAck, SignatureCreateParams, SignatureDeleteAck,
    SignatureDeleteParams, SignatureReorderAck, SignatureReorderParams, SignatureUpdateAck,
    SignatureUpdateParams,
};

use crate::boot::BootSharedState;

pub(crate) async fn handle_create(
    boot_state: &Arc<BootSharedState>,
    params: SignatureCreateParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    let id = write_db
        .with_write(move |conn| {
            // body_text=None on the wire means "let the Service derive
            // it from body_html" - the strip-HTML helper lives in rtsk
            // and is reused here so callers get identical text fallback
            // regardless of which path created the signature.
            let body_text = params
                .body_text
                .or_else(|| Some(db::db::queries_extra::html_to_plain_text(&params.body_html)));
            let p = db::db::queries_extra::InsertSignatureParams {
                account_id: params.account_id,
                name: params.name,
                body_html: params.body_html,
                body_text,
                is_default: params.is_default,
                is_reply_default: params.is_reply_default,
            };
            db::db::queries_extra::db_insert_signature_sync(conn, &p)
        })
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(SignatureCreateAck { id })
        .map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_update(
    boot_state: &Arc<BootSharedState>,
    params: SignatureUpdateParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    write_db
        .with_write(move |conn| {
            // The wire's `body_text: Option<String>` collapses the
            // underlying API's `Option<Option<String>>` to "no change"
            // / "set to text." Set-to-NULL is not exposed because
            // today's UI never needs it; if it ever does, add a
            // sentinel like `clear_body_text: bool`.
            let p = db::db::queries_extra::UpdateSignatureParams {
                id: params.id,
                name: params.name,
                body_html: params.body_html,
                body_text: params.body_text.map(Some),
                is_default: params.is_default,
                is_reply_default: params.is_reply_default,
            };
            db::db::queries_extra::db_update_signature_sync(conn, p)
        })
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(SignatureUpdateAck).map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_delete(
    boot_state: &Arc<BootSharedState>,
    params: SignatureDeleteParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    write_db
        .with_write(move |conn| db::db::queries_extra::db_delete_signature_sync(conn, &params.id))
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(SignatureDeleteAck).map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_reorder(
    boot_state: &Arc<BootSharedState>,
    params: SignatureReorderParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    write_db
        .with_write(move |conn| {
            db::db::queries_extra::db_reorder_signatures_sync(conn, &params.ordered_ids)
        })
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(SignatureReorderAck).map_err(|e| ServiceError::Internal(e.to_string()))
}
