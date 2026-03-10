use super::{categorize_batch, categorize_by_rules, CategorizationInput};

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
