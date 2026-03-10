use crate::discovery::types::DiscoveredConfig;

#[tauri::command]
pub async fn discover_email_config(email: String) -> Result<DiscoveredConfig, String> {
    crate::discovery::discover(&email).await
}
