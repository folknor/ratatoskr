#![allow(clippy::let_underscore_must_use)]

use std::collections::HashMap;

use tauri::State;

use crate::db::DbState;

use super::{FilterActions, FilterCriteria, FilterResult, FilterableMessage, evaluate_filters};

/// Evaluate enabled filters for an account against a set of messages.
/// Reads filter rules from DB, runs matching in Rust, returns per-thread actions.
/// The caller (TS) is responsible for applying the actions via emailActions.
#[tauri::command]
pub async fn filters_evaluate(
    state: State<'_, DbState>,
    account_id: String,
    messages: Vec<FilterableMessage>,
) -> Result<HashMap<String, FilterResult>, String> {
    if messages.is_empty() {
        return Ok(HashMap::new());
    }

    // Read enabled filters from DB
    let filters = state
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT criteria_json, actions_json FROM filter_rules \
                     WHERE account_id = ?1 AND is_enabled = 1 \
                     ORDER BY sort_order, created_at",
                )
                .map_err(|e| format!("prepare filters query: {e}"))?;

            let rows = stmt
                .query_map(rusqlite::params![account_id], |row| {
                    let criteria_json: String = row.get(0)?;
                    let actions_json: String = row.get(1)?;
                    Ok((criteria_json, actions_json))
                })
                .map_err(|e| format!("query filters: {e}"))?;

            let mut filters = Vec::new();
            for row in rows {
                let (criteria_json, actions_json) =
                    row.map_err(|e| format!("read filter row: {e}"))?;
                // Skip filters with invalid JSON (same as TS flatMap + try/catch)
                let criteria: FilterCriteria = match serde_json::from_str(&criteria_json) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                let actions: FilterActions = match serde_json::from_str(&actions_json) {
                    Ok(a) => a,
                    Err(_) => continue,
                };
                filters.push((criteria, actions));
            }

            Ok(filters)
        })
        .await?;

    if filters.is_empty() {
        return Ok(HashMap::new());
    }

    // Run matching on blocking thread (CPU-bound for large filter sets)
    tokio::task::spawn_blocking(move || Ok(evaluate_filters(&filters, &messages)))
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?
}
