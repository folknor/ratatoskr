use serde::Serialize;

const KNOWN_PROVIDERS: &[(&str, &str)] = &[
    ("fastmail.com", "https://api.fastmail.com/jmap/session"),
    (
        "messagingengine.com",
        "https://api.fastmail.com/jmap/session",
    ),
];

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JmapDiscoveryResult {
    pub session_url: String,
    pub source: String,
}

/// Discover the JMAP session URL for a given email address.
///
/// Checks known providers first, then tries `.well-known/jmap` discovery.
pub async fn discover_jmap_url(email: &str) -> Option<JmapDiscoveryResult> {
    let domain = email.split('@').nth(1)?.to_lowercase();

    // Check known providers
    if let Some(&(_, url)) = KNOWN_PROVIDERS.iter().find(|&&(d, _)| d == domain) {
        return Some(JmapDiscoveryResult {
            session_url: url.to_string(),
            source: "known-provider".into(),
        });
    }

    // Try .well-known/jmap
    let well_known = format!("https://{domain}/.well-known/jmap");
    if let Ok(resp) = reqwest::get(&well_known).await {
        if resp.status().is_success() {
            return Some(JmapDiscoveryResult {
                session_url: well_known,
                source: "well-known".into(),
            });
        }
    }

    None
}
