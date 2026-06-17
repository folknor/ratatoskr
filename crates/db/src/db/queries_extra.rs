use crate::db::WriteTarget;

// Re-export from the db queries module so siblings can use `super::load_recent_rule_bundled_threads`.
pub use super::queries::load_recent_rule_bundled_threads;

mod account_delete;
pub mod account_sync_writes;
mod accounts_crud;
mod accounts_messages;
pub mod action_helpers;
mod ai_state;
mod allowlists;
pub mod auto_responses;
pub mod bimi;
mod bundles;
pub mod calendar_contacts_writes;
pub mod calendars;
pub mod chat;
pub mod clean_shutdown_cursors;
pub mod cloud_attachments;
pub mod command_palette;
pub(crate) mod compose;
pub mod contact_carddav;
pub mod contact_groups;
pub mod contact_persistence;
pub mod contact_photos;
pub mod contact_search;
pub mod contacts;
pub mod draft_lifecycle;
pub mod email_actions;
pub mod extract_reindex;
mod filters_smart;
mod follow_up_quick;
pub mod label_groups;
pub mod label_intent;
pub mod label_persistence;
mod labels_attachments;
pub mod mdn;
pub mod message_membership;
pub mod message_persistence;
pub mod message_queries;
mod misc;
pub mod provider_sync_writes;
pub mod scoped_queries;
pub mod search_fallback;
pub mod send_identity;
mod tasks;
pub mod thread_detail;
pub mod thread_persistence;
pub(crate) mod thread_ui_state;

pub use account_delete::*;
pub use account_sync_writes::*;
pub use accounts_crud::*;
pub use accounts_messages::*;
pub use action_helpers::*;
pub use ai_state::*;
pub use allowlists::*;
pub use auto_responses::*;
pub use bimi::*;
pub use bundles::*;
pub use calendar_contacts_writes::*;
pub use calendars::*;
pub use chat::*;
pub use clean_shutdown_cursors::*;
pub use cloud_attachments::*;
pub use command_palette::*;
pub use compose::*;
pub use contact_carddav::*;
pub use contact_groups::*;
pub use contact_persistence::*;
pub use contact_photos::*;
pub use contact_search::*;
pub use contacts::*;
pub use draft_lifecycle::*;
pub use email_actions::*;
pub use extract_reindex::*;
pub use filters_smart::*;
pub use follow_up_quick::*;
pub use label_intent::*;
pub use label_persistence::*;
pub use labels_attachments::*;
pub use mdn::*;
pub use message_membership::*;
pub use message_persistence::*;
pub use message_queries::*;
pub use misc::*;
pub use provider_sync_writes::*;
pub use scoped_queries::*;
pub use search_fallback::*;
pub use send_identity::*;
pub use tasks::*;
pub use thread_detail::*;
pub use thread_persistence::*;
pub use thread_ui_state::*;

pub(super) fn dynamic_update(
    conn: &impl WriteTarget,
    table: &str,
    id_col: &str,
    id_val: &str,
    sets: Vec<(&str, Box<dyn rusqlite::types::ToSql>)>,
) -> Result<(), String> {
    if sets.is_empty() {
        return Ok(());
    }
    let mut placeholders = Vec::new();
    let mut param_vals: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    for (i, (col, val)) in sets.into_iter().enumerate() {
        placeholders.push(format!("{col} = ?{}", i + 1));
        param_vals.push(val);
    }
    let id_idx = param_vals.len() + 1;
    param_vals.push(Box::new(id_val.to_owned()));
    let sql = format!(
        "UPDATE {table} SET {} WHERE {id_col} = ?{id_idx}",
        placeholders.join(", ")
    );
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        param_vals.iter().map(AsRef::as_ref).collect();
    conn.execute(&sql, param_refs.as_slice())
        .map_err(|e| e.to_string())?;
    Ok(())
}
