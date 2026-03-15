use super::DbState;
use super::sql_fragments::LATEST_MESSAGE_SUBQUERY;
use super::types::{
    DbFilterRule, DbFollowUpReminder, DbQuickStep, DbSmartFolder, DbSmartLabelRule, ThreadInfoRow,
};
use rusqlite::{Connection, Row, params};

mod accounts_messages;
mod ai_state;
mod allowlists;
mod bundles_categories;
mod calendars;
pub(crate) mod compose;
mod contact_groups;
mod contacts;
mod filters_smart;
mod follow_up_quick;
mod labels_attachments;
mod misc;
pub mod navigation;
mod scoped_queries;
mod tasks;

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

pub fn load_recent_rule_categorized_threads(
    conn: &Connection,
    account_id: &str,
    limit: i64,
) -> Result<Vec<ThreadInfoRow>, String> {
    let sql = format!(
        "SELECT t.id, t.subject, t.snippet, m.from_address
         FROM threads t
         INNER JOIN thread_labels tl ON tl.account_id = t.account_id AND tl.thread_id = t.id
         INNER JOIN thread_categories tc ON tc.account_id = t.account_id AND tc.thread_id = t.id
         LEFT JOIN ({LATEST_MESSAGE_SUBQUERY}
         ) m ON m.account_id = t.account_id AND m.thread_id = t.id
         WHERE t.account_id = ?1 AND tl.label_id = 'INBOX' AND tc.is_manual = 0
         ORDER BY t.last_message_at DESC
         LIMIT ?2"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    stmt.query_map(params![account_id, limit], |row| {
        Ok(ThreadInfoRow {
            id: row.get(0)?,
            subject: row.get(1)?,
            snippet: row.get(2)?,
            from_address: row.get(3)?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

pub(super) fn row_to_filter(row: &Row<'_>) -> rusqlite::Result<DbFilterRule> {
    Ok(DbFilterRule {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        name: row.get("name")?,
        is_enabled: row.get::<_, i64>("is_enabled")? != 0,
        criteria_json: row.get("criteria_json")?,
        actions_json: row.get("actions_json")?,
        sort_order: row.get("sort_order")?,
        created_at: row.get("created_at")?,
    })
}

pub(super) fn row_to_smart_folder(row: &Row<'_>) -> rusqlite::Result<DbSmartFolder> {
    Ok(DbSmartFolder {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        name: row.get("name")?,
        query: row.get("query")?,
        icon: row.get("icon")?,
        color: row.get("color")?,
        sort_order: row.get("sort_order")?,
        is_default: row.get::<_, i64>("is_default")? != 0,
        created_at: row.get("created_at")?,
    })
}

pub(super) fn row_to_smart_label_rule(row: &Row<'_>) -> rusqlite::Result<DbSmartLabelRule> {
    Ok(DbSmartLabelRule {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        label_id: row.get("label_id")?,
        ai_description: row.get("ai_description")?,
        criteria_json: row.get("criteria_json")?,
        is_enabled: row.get::<_, i64>("is_enabled")? != 0,
        sort_order: row.get("sort_order")?,
        created_at: row.get("created_at")?,
    })
}

#[allow(dead_code)]
fn _keep_types_alive(_: (&DbState, &DbFollowUpReminder, &DbQuickStep)) {}
