// tauri::command macro generates code that trips let_underscore_must_use
#![allow(clippy::let_underscore_must_use)]

use tauri::State;

use crate::db::DbState;

#[tauri::command]
pub async fn backfill_seen_addresses(
    db: State<'_, DbState>,
    account_id: String,
) -> Result<u64, String> {
    ratatoskr_core::seen_addresses::backfill_seen_addresses(&db, account_id).await
}
