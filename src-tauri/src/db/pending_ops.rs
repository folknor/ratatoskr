// tauri::command macro generates code that trips let_underscore_must_use
#![allow(clippy::let_underscore_must_use)]

use tauri::State;

use super::DbState;
pub use ratatoskr_core::db::pending_ops::PendingOperation;

#[tauri::command]
pub async fn db_pending_ops_enqueue(
    state: State<'_, DbState>,
    id: String,
    account_id: String,
    operation_type: String,
    resource_id: String,
    params_json: String,
) -> Result<(), String> {
    ratatoskr_core::db::pending_ops::db_pending_ops_enqueue(
        &state,
        id,
        account_id,
        operation_type,
        resource_id,
        params_json,
    )
    .await
}

#[tauri::command]
pub async fn db_pending_ops_get(
    state: State<'_, DbState>,
    account_id: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<PendingOperation>, String> {
    ratatoskr_core::db::pending_ops::db_pending_ops_get(&state, account_id, limit).await
}

#[tauri::command]
pub async fn db_pending_ops_update_status(
    state: State<'_, DbState>,
    id: String,
    status: String,
    error_message: Option<String>,
) -> Result<(), String> {
    ratatoskr_core::db::pending_ops::db_pending_ops_update_status(&state, id, status, error_message)
        .await
}

#[tauri::command]
pub async fn db_pending_ops_delete(state: State<'_, DbState>, id: String) -> Result<(), String> {
    ratatoskr_core::db::pending_ops::db_pending_ops_delete(&state, id).await
}

#[tauri::command]
pub async fn db_pending_ops_increment_retry(
    state: State<'_, DbState>,
    id: String,
) -> Result<(), String> {
    ratatoskr_core::db::pending_ops::db_pending_ops_increment_retry(&state, id).await
}

#[tauri::command]
pub async fn db_pending_ops_count(
    state: State<'_, DbState>,
    account_id: Option<String>,
) -> Result<i64, String> {
    ratatoskr_core::db::pending_ops::db_pending_ops_count(&state, account_id).await
}

#[tauri::command]
pub async fn db_pending_ops_failed_count(
    state: State<'_, DbState>,
    account_id: Option<String>,
) -> Result<i64, String> {
    ratatoskr_core::db::pending_ops::db_pending_ops_failed_count(&state, account_id).await
}

#[tauri::command]
pub async fn db_pending_ops_for_resource(
    state: State<'_, DbState>,
    account_id: String,
    resource_id: String,
) -> Result<Vec<PendingOperation>, String> {
    ratatoskr_core::db::pending_ops::db_pending_ops_for_resource(&state, account_id, resource_id)
        .await
}

#[tauri::command]
pub async fn db_pending_ops_compact(
    state: State<'_, DbState>,
    account_id: Option<String>,
) -> Result<i64, String> {
    ratatoskr_core::db::pending_ops::db_pending_ops_compact(&state, account_id).await
}

#[tauri::command]
pub async fn db_pending_ops_clear_failed(
    state: State<'_, DbState>,
    account_id: Option<String>,
) -> Result<(), String> {
    ratatoskr_core::db::pending_ops::db_pending_ops_clear_failed(&state, account_id).await
}

#[tauri::command]
pub async fn db_pending_ops_recover_executing(state: State<'_, DbState>) -> Result<i64, String> {
    ratatoskr_core::db::pending_ops::db_pending_ops_recover_executing(&state).await
}

#[tauri::command]
pub async fn db_pending_ops_retry_failed(
    state: State<'_, DbState>,
    account_id: Option<String>,
) -> Result<(), String> {
    ratatoskr_core::db::pending_ops::db_pending_ops_retry_failed(&state, account_id).await
}
