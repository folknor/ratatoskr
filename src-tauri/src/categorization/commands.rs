#![allow(clippy::needless_pass_by_value)]

use rusqlite::params;

use crate::ai_commands::{AiCompleteRequest, ai_is_available_impl, complete_ai_impl};
use crate::db::DbState;
use crate::provider::crypto::AppCryptoState;

use super::{
    AiCategorizationCandidate, CATEGORIZE_PROMPT, CategorizationInput, categorize_batch,
    categorize_by_rules, format_ai_categorization_input, parse_ai_categorization_output,
};

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

pub(crate) async fn categorize_threads_with_ai_impl(
    account_id: &str,
    candidates: &[AiCategorizationCandidate],
    db: &DbState,
    crypto: &AppCryptoState,
) -> Result<(), String> {
    if candidates.is_empty() || !ai_is_available_impl(db, crypto).await? {
        return Ok(());
    }

    let request = AiCompleteRequest {
        system_prompt: CATEGORIZE_PROMPT.to_string(),
        user_content: format_ai_categorization_input(candidates),
        max_tokens: Some(512),
    };
    let response = complete_ai_impl(db, crypto, &request).await?;
    let valid_thread_ids = candidates
        .iter()
        .map(|candidate| candidate.id.clone())
        .collect();
    let categories = parse_ai_categorization_output(&response, &valid_thread_ids);

    if categories.is_empty() {
        return Ok(());
    }

    let account_id = account_id.to_string();
    db.with_conn(move |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        for (thread_id, category) in &categories {
            tx.execute(
                "INSERT INTO thread_categories (account_id, thread_id, category, is_manual)
                 VALUES (?1, ?2, ?3, 0)
                 ON CONFLICT(account_id, thread_id) DO UPDATE SET
                   category = ?3
                 WHERE is_manual = 0",
                params![account_id, thread_id, category.as_str()],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}
