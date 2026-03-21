//! Signature CRUD handlers for the app crate.
//!
//! These functions delegate to core CRUD functions in
//! `ratatoskr_core::db::queries_extra::compose` rather than using raw SQL.
//! The core functions handle transactional default-clearing properly.

use std::sync::Arc;

use iced::Task;

use crate::db::Db;
use crate::ui::settings::SignatureEntry;

/// Save a signature (insert or update) via core CRUD functions.
///
/// When `is_default` is true, the core functions clear `is_default` on all
/// other signatures for the same account in a transaction. Same for
/// `is_reply_default`. Auto-generates `body_text` from `body_html`.
pub fn handle_save_signature(
    db: &Arc<Db>,
    req: crate::ui::settings::SignatureSaveRequest,
) -> Task<super::SignatureResult> {
    let db = Arc::clone(db);
    Task::perform(
        async move {
            let body_text = html_to_plain_text(&req.body_html);
            let core_db = ratatoskr_core::db::DbState::from_arc(db.write_conn_arc());

            if let Some(ref id) = req.id {
                // Update existing signature via core CRUD.
                let params = ratatoskr_core::db::queries_extra::UpdateSignatureParams {
                    id: id.clone(),
                    name: Some(req.name),
                    body_html: Some(req.body_html),
                    body_text: Some(Some(body_text)),
                    is_default: Some(req.is_default),
                    is_reply_default: Some(req.is_reply_default),
                };
                ratatoskr_core::db::queries_extra::db_update_signature(&core_db, params).await
            } else {
                // Insert new signature via core CRUD.
                let params = ratatoskr_core::db::queries_extra::InsertSignatureParams {
                    account_id: req.account_id,
                    name: req.name,
                    body_html: req.body_html,
                    body_text: Some(body_text),
                    is_default: req.is_default,
                    is_reply_default: req.is_reply_default,
                };
                ratatoskr_core::db::queries_extra::db_insert_signature(&core_db, params)
                    .await
                    .map(|_id| ())
            }
        },
        |result| {
            if let Err(ref e) = result {
                log::error!("Failed to save signature: {e}");
            } else {
                log::info!("Signature saved");
            }
            super::SignatureResult::Saved(result)
        },
    )
}

/// Delete a signature by ID via core CRUD.
pub fn handle_delete_signature(
    db: &Arc<Db>,
    sig_id: String,
) -> Task<super::SignatureResult> {
    let db = Arc::clone(db);
    Task::perform(
        async move {
            let core_db = ratatoskr_core::db::DbState::from_arc(db.write_conn_arc());
            ratatoskr_core::db::queries_extra::db_delete_signature(&core_db, sig_id).await
        },
        |result| {
            if let Err(ref e) = result {
                log::error!("Failed to delete signature: {e}");
            } else {
                log::info!("Signature deleted");
            }
            super::SignatureResult::Deleted(result)
        },
    )
}

/// Load all signatures from the DB asynchronously via core CRUD.
pub fn load_signatures_async(
    db: &Arc<Db>,
) -> Task<super::SignatureResult> {
    let db = Arc::clone(db);
    Task::perform(
        async move {
            let core_db = ratatoskr_core::db::DbState::from_arc(db.conn_arc());
            let db_sigs = ratatoskr_core::db::queries_extra::db_get_all_signatures(&core_db).await?;
            // Convert DbSignature to the app's SignatureEntry type.
            let entries = db_sigs
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

/// Reorder signatures by updating sort_order via core CRUD.
pub fn handle_reorder_signatures(
    db: &Arc<Db>,
    ordered_ids: Vec<String>,
) -> Task<super::SignatureResult> {
    let db = Arc::clone(db);
    Task::perform(
        async move {
            let core_db = ratatoskr_core::db::DbState::from_arc(db.write_conn_arc());
            ratatoskr_core::db::queries_extra::db_reorder_signatures(&core_db, ordered_ids).await
        },
        |result| {
            if let Err(ref e) = result {
                eprintln!("Failed to reorder signatures: {e}");
            }
            // Reload after reorder.
            super::SignatureResult::Saved(result)
        },
    )
}

// ── HTML-to-plain-text ──────────────────────────────────

/// Strip HTML tags to produce a plain-text fallback for the signature.
///
/// Block elements insert newlines; inline elements are dropped.
fn html_to_plain_text(html: &str) -> String {
    // Delegate to the core implementation.
    ratatoskr_core::db::queries_extra::html_to_plain_text(html)
}
