#![allow(clippy::needless_pass_by_value)]

use super::{CategorizationInput, categorize_batch, categorize_by_rules};
use crate::db::DbState;
use tauri::State;

/// Categorize a single thread by deterministic rules.
/// Returns the category string ("Primary", "Updates", "Promotions", "Social", "Newsletters").
#[tauri::command]
pub fn categorize_thread_by_rules(input: CategorizationInput) -> String {
    categorize_by_rules(&input).as_str().to_string()
}

/// Batch-categorize multiple threads by deterministic rules.
/// Returns category strings in the same order as the inputs.
#[tauri::command]
pub fn categorize_threads_by_rules(inputs: Vec<CategorizationInput>) -> Vec<String> {
    categorize_batch(&inputs)
        .into_iter()
        .map(|c| c.as_str().to_string())
        .collect()
}

/// Apply AI categorization results while respecting existing manual overrides.
#[tauri::command]
pub async fn categorization_apply_ai_results(
    db: State<'_, DbState>,
    account_id: String,
    categories: Vec<(String, String)>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        for (thread_id, category) in &categories {
            tx.execute(
                "INSERT INTO thread_categories (account_id, thread_id, category, is_manual)
                 VALUES (?1, ?2, ?3, 0)
                 ON CONFLICT(account_id, thread_id) DO UPDATE SET
                   category = ?3
                 WHERE is_manual = 0",
                rusqlite::params![account_id, thread_id, category],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}
