//! Per-account raw labels (settings > Mail Rules > Labels).
//!
//! Read path lives here. Writes (create / delete / rename / recolor) are
//! scaffolded as stub Tasks until the action service grows the matching
//! `label.create`, `label.delete`, `label.recolor`, `label.rename` actions.
//! Per the labels-unification redesign, every per-account label is keyed
//! on `(account_id, label_id)`; the pre-split `normalized_name` collapse
//! is gone (see `docs/labels-unification/redesign.md` "Reversal 2").

use std::sync::Arc;

use iced::Task;
use types::LabelId;

use crate::db::Db;

use super::LabelOp;

/// Load all raw provider labels grouped by account, sorted alphabetically.
/// The sidebar's LABELS section renders explicit `label_groups`, not the
/// per-account rows this settings view exposes.
pub fn load_visible_labels_async(db: &Arc<Db>) -> Task<LabelOp> {
    let db = Arc::clone(db);
    Task::perform(
        async move {
            let core_db = db.read_db_state();
            core_db
                .with_conn(move |conn| {
                    rtsk::db::queries_extra::navigation::query_labels_by_account(conn)
                })
                .await
        },
        |result| {
            if let Ok(ref groups) = result {
                let total: usize = groups.iter().map(|g| g.labels.len()).sum();
                log::info!(
                    "Labels loaded: {total} labels across {} accounts",
                    groups.len()
                );
            }
            LabelOp::Loaded(result)
        },
    )
}

// Write stubs.
//
// These return Task::done synthesising an Err result for now so the UI
// can surface "not yet implemented" without crashing. Replace with real
// Service IPC once the action handlers land.

#[allow(dead_code)] // wired in tier-2; kept for the call-site shape
pub fn create_label_async(account_id: &str, name: &str) -> Task<LabelOp> {
    log::warn!("create_label not implemented yet: account={account_id} name={name}");
    Task::done(LabelOp::CreatedAck(Err(
        "label creation not yet implemented".to_owned(),
    )))
}

#[allow(dead_code)]
pub fn delete_label_async(account_id: &str, label_id: &LabelId) -> Task<LabelOp> {
    log::warn!("delete_label not implemented yet: account={account_id} label={label_id}");
    Task::done(LabelOp::DeletedAck(Err(
        "label deletion not yet implemented".to_owned(),
    )))
}

#[allow(dead_code)]
pub fn rename_label_async(account_id: &str, label_id: &LabelId, new_name: &str) -> Task<LabelOp> {
    log::warn!(
        "rename_label not implemented yet: account={account_id} label={label_id} -> {new_name}"
    );
    Task::done(LabelOp::RenamedAck(Err(
        "label rename not yet implemented".to_owned(),
    )))
}

#[allow(dead_code)]
pub fn recolor_label_async(
    account_id: &str,
    label_id: &LabelId,
    color: label_colors::LabelStyleHex<'_>,
) -> Task<LabelOp> {
    log::warn!(
        "recolor_label not implemented yet: account={account_id} label={label_id} -> ({}, {})",
        color.bg(),
        color.fg(),
    );
    Task::done(LabelOp::RecoloredAck(Err(
        "label recolor not yet implemented".to_owned(),
    )))
}
