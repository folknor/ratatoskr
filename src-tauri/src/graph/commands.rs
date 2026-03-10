#![allow(clippy::let_underscore_must_use)]

use serde::Serialize;
use tauri::State;

use crate::db::DbState;

use super::client::{GraphClient, GraphState};
use super::types::GraphProfile;

// ── Lifecycle commands ──────────────────────────────────────

#[tauri::command]
pub async fn graph_init_client(
    account_id: String,
    db: State<'_, DbState>,
    graph: State<'_, GraphState>,
) -> Result<(), String> {
    let client = GraphClient::from_account(&db, &account_id, *graph.encryption_key()).await?;
    graph.insert(account_id, client).await;
    Ok(())
}

#[tauri::command]
pub async fn graph_remove_client(
    account_id: String,
    graph: State<'_, GraphState>,
) -> Result<(), String> {
    graph.remove(&account_id).await;
    Ok(())
}

#[tauri::command]
pub async fn graph_test_connection(
    account_id: String,
    db: State<'_, DbState>,
    graph: State<'_, GraphState>,
) -> Result<GraphTestResult, String> {
    let client = graph.get(&account_id).await?;
    let profile: GraphProfile = client
        .get_json("/me?$select=displayName,mail,userPrincipalName", &db)
        .await?;

    let display = profile
        .mail
        .or(profile.user_principal_name)
        .unwrap_or_else(|| "Unknown".to_string());

    Ok(GraphTestResult {
        success: true,
        message: format!("Connected as {display}"),
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphTestResult {
    pub success: bool,
    pub message: String,
}

#[tauri::command]
pub async fn graph_get_profile(
    account_id: String,
    db: State<'_, DbState>,
    graph: State<'_, GraphState>,
) -> Result<GraphProfile, String> {
    let client = graph.get(&account_id).await?;
    client
        .get_json("/me?$select=displayName,mail,userPrincipalName", &db)
        .await
}
