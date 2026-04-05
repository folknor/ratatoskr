use rusqlite::Connection;

// Re-export from the db queries module so siblings can use `super::load_recent_rule_bundled_threads`.
pub use super::queries::load_recent_rule_bundled_threads;

mod accounts_crud;
mod account_delete;
mod accounts_messages;
mod ai_state;
mod allowlists;
mod bundles;
pub mod calendars;
pub(crate) mod compose;
pub mod contact_groups;
pub mod contacts;
mod filters_smart;
mod follow_up_quick;
mod labels_attachments;
pub mod message_queries;
mod misc;
pub mod scoped_queries;
mod tasks;

pub use accounts_crud::*;
pub use account_delete::*;
pub use accounts_messages::*;
pub use ai_state::*;
pub use allowlists::*;
pub use bundles::*;
pub use calendars::*;
pub use compose::*;
pub use contact_groups::*;
pub use contacts::*;
pub use filters_smart::*;
pub use follow_up_quick::*;
pub use labels_attachments::*;
pub use message_queries::*;
pub use misc::*;
pub use scoped_queries::*;
pub use tasks::*;

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
