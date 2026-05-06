//! Signature CRUD UI handlers.
//!
//! Phase 6a: relocated to the Service. This module now only fires
//! IPC requests via `ServiceClient` and routes the typed acks back
//! through dedicated `SignatureResult` variants - one per IPC method,
//! per the per-surface checklist. The HTML-to-plain-text conversion
//! that previously ran here moved Service-side too: the Service
//! handler derives `body_text` from `body_html` when the wire's
//! `body_text` is `None`, so the UI does not need a local copy of
//! that helper.
//!
//! Read paths (`load_signatures_async`) stay UI-side - the Service
//! is the write boundary, but reads continue to flow through `Db`.

use std::sync::Arc;

use iced::Task;
use service_api::{SignatureCreateParams, SignatureUpdateParams};

use crate::db::Db;
use crate::service_client::ServiceClient;
use crate::ui::settings::SignatureEntry;

type ClientHandle = Option<Arc<ServiceClient>>;

/// Save a signature (insert or update) via Service IPC.
///
/// `req.id` set means update; unset means create. Failure is logged
/// at error level and surfaces through the dedicated ack variant; the
/// caller's `handle_signature_op` arm then triggers a re-list so the
/// settings UI reflects the canonical Service-committed state.
pub fn handle_save_signature(
    client: ClientHandle,
    req: crate::ui::settings::SignatureSaveRequest,
) -> Task<super::SignatureResult> {
    let Some(client) = client else {
        log::warn!("signature.save: no ServiceClient yet; ignoring save");
        // Map to the right ack variant so the dispatch arm still
        // routes the failure to the same re-list / status path.
        return if req.id.is_some() {
            Task::done(super::SignatureResult::UpdatedAck(Err(
                "Service not ready".to_string(),
            )))
        } else {
            Task::done(super::SignatureResult::CreatedAck(Err(
                "Service not ready".to_string(),
            )))
        };
    };
    if let Some(id) = req.id {
        let params = SignatureUpdateParams {
            id,
            name: Some(req.name),
            body_html: Some(req.body_html),
            // body_text=None on the wire = "Service derives from
            // body_html." UI doesn't need to run the strip-HTML
            // conversion locally anymore.
            body_text: None,
            is_default: Some(req.is_default),
            is_reply_default: Some(req.is_reply_default),
        };
        Task::perform(
            async move {
                client
                    .update_signature(params)
                    .await
                    .map_err(|e| e.to_string())
            },
            |result| {
                if let Err(ref e) = result {
                    log::error!("Failed to update signature: {e}");
                } else {
                    log::info!("Signature updated");
                }
                super::SignatureResult::UpdatedAck(result)
            },
        )
    } else {
        let params = SignatureCreateParams {
            account_id: req.account_id,
            name: req.name,
            body_html: req.body_html,
            body_text: None,
            is_default: req.is_default,
            is_reply_default: req.is_reply_default,
        };
        Task::perform(
            async move {
                client
                    .create_signature(params)
                    .await
                    .map_err(|e| e.to_string())
            },
            |result| {
                if let Err(ref e) = result {
                    log::error!("Failed to create signature: {e}");
                } else {
                    log::info!("Signature created");
                }
                super::SignatureResult::CreatedAck(result)
            },
        )
    }
}

/// Delete a signature by id via Service IPC.
pub fn handle_delete_signature(
    client: ClientHandle,
    sig_id: String,
) -> Task<super::SignatureResult> {
    let Some(client) = client else {
        log::warn!("signature.delete: no ServiceClient yet; ignoring delete");
        return Task::done(super::SignatureResult::DeletedAck(Err(
            "Service not ready".to_string(),
        )));
    };
    Task::perform(
        async move {
            client
                .delete_signature(sig_id)
                .await
                .map_err(|e| e.to_string())
        },
        |result| {
            if let Err(ref e) = result {
                log::error!("Failed to delete signature: {e}");
            } else {
                log::info!("Signature deleted");
            }
            super::SignatureResult::DeletedAck(result)
        },
    )
}

/// Load all signatures from the DB asynchronously via core CRUD.
///
/// Read path stays UI-side: the Service is the write boundary, but
/// the read still flows through `Db` -> `db_get_all_signatures`.
pub fn load_signatures_async(db: &Arc<Db>) -> Task<super::SignatureResult> {
    let db = Arc::clone(db);
    Task::perform(
        async move {
            let core_db = db.read_db_state();
            let db_sigs = rtsk::db::queries_extra::db_get_all_signatures(&core_db).await?;
            let entries: Vec<SignatureEntry> = db_sigs
                .into_iter()
                .map(|s| SignatureEntry {
                    id: s.id,
                    account_id: s.account_id,
                    name: s.name,
                    body_html: s.body_html,
                    body_text: s.body_text,
                    is_default: s.is_default != 0,
                    is_reply_default: s.is_reply_default != 0,
                })
                .collect();
            Ok(entries)
        },
        |result| {
            if let Ok(ref sigs) = result {
                log::info!("Signatures loaded: {} entries", sigs.len());
            }
            super::SignatureResult::Loaded(result)
        },
    )
}

/// Reorder signatures via Service IPC.
///
/// Per-account ordering hazard: rapid drag-reorder clicks can land
/// out of order at the Service if the blocking pool is not
/// order-preserving. Today the staleness is tolerable because the
/// next list reload picks up the canonical order; if a user-visible
/// bug shows up, the documented escape hatch is a generation token
/// on `SignatureReorderParams`.
pub fn handle_reorder_signatures(
    client: ClientHandle,
    ordered_ids: Vec<String>,
) -> Task<super::SignatureResult> {
    let Some(client) = client else {
        log::warn!("signature.reorder: no ServiceClient yet; ignoring reorder");
        return Task::done(super::SignatureResult::ReorderedAck(Err(
            "Service not ready".to_string(),
        )));
    };
    Task::perform(
        async move {
            client
                .reorder_signatures(ordered_ids)
                .await
                .map_err(|e| e.to_string())
        },
        |result| {
            if let Err(ref e) = result {
                log::error!("Failed to reorder signatures: {e}");
            }
            super::SignatureResult::ReorderedAck(result)
        },
    )
}
