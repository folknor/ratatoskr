use tauri::State;

use ratatoskr_core::ai::inference;
use ratatoskr_core::ai::types::AiCompletionRequest;

use crate::db::DbState;
use crate::provider::crypto::AppCryptoState;

/// Re-export so internal callers (`categorization::commands`, `smart_labels::commands`)
/// can keep using the same import path without changes.
pub use ratatoskr_core::ai::types::AiCompletionRequest as AiCompleteRequest;

// ---------------------------------------------------------------------------
// pub(crate) helpers consumed by categorization + smart_labels commands
// ---------------------------------------------------------------------------

pub(crate) async fn ai_is_available_impl(
    db: &DbState,
    crypto: &AppCryptoState,
) -> Result<bool, String> {
    inference::ai_is_available(db, crypto)
        .await
        .map_err(|e| e.to_string())
}

pub(crate) async fn complete_ai_impl(
    db: &DbState,
    crypto: &AppCryptoState,
    request: &AiCompleteRequest,
) -> Result<String, String> {
    inference::complete(db, crypto, request)
        .await
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn ai_get_provider_name(
    db: State<'_, DbState>,
) -> Result<String, String> {
    inference::get_provider_name(&db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn ai_is_available(
    db: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
) -> Result<bool, String> {
    ai_is_available_impl(&db, &crypto).await
}

#[tauri::command]
pub async fn ai_test_connection(
    db: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
) -> Result<bool, String> {
    let config = inference::load_ai_config(&db, crypto.encryption_key())
        .await
        .map_err(|e| e.to_string())?;
    let request = AiCompletionRequest {
        system_prompt: "You are a helpful assistant.".to_string(),
        user_content: "Say hi".to_string(),
        max_tokens: Some(16),
    };
    inference::complete_with_config(&config, &request)
        .await
        .map(|_| true)
        .or(Ok(false))
}

#[tauri::command]
pub async fn ai_complete(
    db: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
    request: AiCompleteRequest,
) -> Result<String, String> {
    complete_ai_impl(&db, &crypto, &request).await
}
