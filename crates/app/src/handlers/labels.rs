//! Cross-account labels (settings > Mail Rules > Labels, sidebar section 4).
//!
//! Read path lives here. Writes (create / delete / rename / recolor) are
//! scaffolded as stub Tasks until the action service grows the matching
//! `label.create`, `label.delete`, `label.recolor`, `label.rename` actions.

use std::sync::Arc;

use iced::Task;

use crate::db::Db;

use super::LabelOp;

/// Load all cross-account labels (`label_kind = 'tag'`) grouped by
/// normalized name, sorted alphabetically. Drives both the settings tab
/// and sidebar section 4.
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

// ── Write stubs ─────────────────────────────────────────────
//
// These return Task::done synthesising an Err result for now so the UI
// can surface "not yet implemented" without crashing. Replace with real
// Service IPC once the action handlers land.

#[allow(dead_code)] // wired in tier-2; kept for the call-site shape
pub fn create_label_async(normalized_name: &str) -> Task<LabelOp> {
    log::warn!("create_label not implemented yet: {normalized_name}");
    Task::done(LabelOp::CreatedAck(Err(
        "label creation not yet implemented".to_owned(),
    )))
}

#[allow(dead_code)]
pub fn delete_label_async(normalized_name: &str) -> Task<LabelOp> {
    log::warn!("delete_label not implemented yet: {normalized_name}");
    Task::done(LabelOp::DeletedAck(Err(
        "label deletion not yet implemented".to_owned(),
    )))
}

#[allow(dead_code)]
pub fn rename_label_async(from: &str, to: &str) -> Task<LabelOp> {
    log::warn!("rename_label not implemented yet: {from} -> {to}");
    Task::done(LabelOp::RenamedAck(Err(
        "label rename not yet implemented".to_owned(),
    )))
}

#[allow(dead_code)]
pub fn recolor_label_async(
    normalized_name: &str,
    color_bg: &str,
    color_fg: &str,
) -> Task<LabelOp> {
    log::warn!(
        "recolor_label not implemented yet: {normalized_name} -> ({color_bg}, {color_fg})"
    );
    Task::done(LabelOp::RecoloredAck(Err(
        "label recolor not yet implemented".to_owned(),
    )))
}
