use crate::discovery::types::DiscoveredConfig;

#[tauri::command]
pub async fn discover_email_config(email: String) -> Result<DiscoveredConfig, String> {
    crate::discovery::discover(&email).await
}

/// Quick check whether a domain belongs to a known JMAP provider.
/// Checks only the hardcoded registry — no network calls.
#[tauri::command]
pub fn is_known_jmap_provider(domain: String) -> bool {
    crate::discovery::registry::is_known_jmap_provider(&domain)
}
