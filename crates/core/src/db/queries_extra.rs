use rusqlite::Connection;

// Re-export from the db crate so existing callers (`sync/notifications.rs`,
// `bundles_categories.rs`) can keep importing via this path.
pub use ratatoskr_db::db::queries::load_recent_rule_categorized_threads;

mod accounts_crud;
mod accounts_messages;
mod ai_state;
mod allowlists;
mod bundles_categories;
pub mod calendars;
pub(crate) mod compose;
pub mod contact_groups;
pub mod contacts;
mod filters_smart;
mod follow_up_quick;
mod labels_attachments;
mod misc;
pub mod navigation;
mod scoped_queries;
mod tasks;
pub mod thread_detail;
mod thread_ui_state;

pub use accounts_crud::*;
pub use accounts_messages::*;
pub use ai_state::*;
pub use allowlists::*;
pub use bundles_categories::*;
pub use calendars::*;
pub use compose::*;
pub use contact_groups::*;
pub use contacts::*;
pub use filters_smart::*;
pub use follow_up_quick::*;
pub use labels_attachments::*;
pub use misc::*;
pub use navigation::*;
pub use scoped_queries::*;
pub use tasks::*;
pub use thread_detail::*;
pub use thread_ui_state::*;

pub(super) fn dynamic_update(
    conn: &Connection,
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
