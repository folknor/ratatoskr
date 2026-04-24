//! OIDC Discovery - fetch `.well-known/openid-configuration` and extract
//! OAuth2 endpoints for generic OIDC providers (Keycloak, Authentik, etc.).
//!
//! This enables Ratatoskr to authenticate against any standards-compliant
//! OIDC provider, not just hardcoded Google/Microsoft flows.

use serde::Deserialize;

/// Max response body size for the discovery document (1 MB).
const MAX_BODY_SIZE: usize = 1_024 * 1_024;

/// Subset of the OpenID Connect Discovery 1.0 response that we need.
#[derive(Debug, Deserialize)]
struct OidcDiscoveryDocument {
    /// REQUIRED per spec - must match the issuer URL used to fetch this document.
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    #[serde(default)]
    scopes_supported: Vec<String>,
    #[serde(default)]
    code_challenge_methods_supported: Vec<String>,
    #[serde(default)]
    token_endpoint_auth_methods_supported: Vec<String>,
}

/// Result of a successful OIDC discovery probe.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OidcEndpoints {
    pub issuer_url: String,
    pub auth_url: String,
    pub token_url: String,
    pub scopes: Vec<String>,
    pub supports_pkce_s256: bool,
    /// Whether the provider supports `none` token endpoint auth (public clients).
    pub supports_public_client: bool,
}

/// Scopes we request - `offline_access` is critical for refresh tokens.
const DESIRED_SCOPES: &[&str] = &["openid", "email", "profile", "offline_access"];

/// Intersect the provider's `scopes_supported` with our desired scopes.
/// If `scopes_supported` is absent (empty vec from serde default), return
/// all desired scopes as a best-effort request.
fn negotiate_scopes(scopes_supported: &[String]) -> Vec<String> {
    if scopes_supported.is_empty() {
        return DESIRED_SCOPES.iter().map(|s| (*s).to_string()).collect();
    }
    DESIRED_SCOPES
        .iter()
        .filter(|s| scopes_supported.iter().any(|ss| ss == *s))
        .map(|s| (*s).to_string())
        .collect()
}

/// PKCE S256 is assumed when `code_challenge_methods_supported` is absent
/// (per RFC 8414 / OAuth 2.1 best practice).
fn detect_pkce_s256(methods: &[String]) -> bool {
    methods.is_empty() || methods.iter().any(|m| m == "S256")
}

/// Public client support = `"none"` in `token_endpoint_auth_methods_supported`.
fn detect_public_client(auth_methods: &[String]) -> bool {
    auth_methods.iter().any(|m| m == "none")
}

/// Validate that a URL is non-empty and uses HTTPS.
fn is_valid_https_url(url: &str) -> bool {
    url.starts_with("https://") && url.len() > "https://".len()
}

/// Probe `https://{domain}/.well-known/openid-configuration` for OIDC support.
///
/// Returns `None` if the domain doesn't serve an OIDC discovery document.
/// This is a best-effort probe - failure is not an error, it just means
/// the domain isn't an OIDC issuer.
pub async fn probe(domain: &str) -> Option<OidcEndpoints> {
    probe_issuer(&format!("https://{domain}")).await
}

/// Probe a specific issuer URL for OIDC support.
///
/// The issuer URL may include a path (e.g., `https://auth.example.com/realms/corp`).
/// Appends `/.well-known/openid-configuration` per the OIDC Discovery spec.
pub async fn probe_issuer(issuer_url: &str) -> Option<OidcEndpoints> {
    let normalized_issuer = issuer_url.trim_end_matches('/');
    let url = format!("{normalized_issuer}/.well-known/openid-configuration");

    let client = reqwest::Client::builder()
        .timeout(crate::constants::DISCOVERY_HTTP_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(3))
        .user_agent("Ratatoskr/1.0")
        .build()
        .ok()?;

    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        log::debug!("OIDC discovery: {url} returned {}", resp.status());
        return None;
    }

    // Guard against oversized responses
    if resp
        .content_length()
        .is_some_and(|len| len > MAX_BODY_SIZE as u64)
    {
        log::debug!("OIDC discovery: response too large from {url}");
        return None;
    }

    let bytes = resp.bytes().await.ok()?;
    if bytes.len() > MAX_BODY_SIZE {
        log::debug!("OIDC discovery: response body too large from {url}");
        return None;
    }

    let doc: OidcDiscoveryDocument = serde_json::from_slice(&bytes).ok()?;

    // Spec requirement: issuer in the document must match the request issuer
    if doc.issuer.trim_end_matches('/') != normalized_issuer {
        log::warn!(
            "OIDC discovery: issuer mismatch - expected {normalized_issuer}, got {}",
            doc.issuer
        );
        return None;
    }

    // Validate endpoints are HTTPS
    if !is_valid_https_url(&doc.authorization_endpoint) || !is_valid_https_url(&doc.token_endpoint)
    {
        log::warn!("OIDC discovery: non-HTTPS endpoints from {url}");
        return None;
    }

    let scopes = negotiate_scopes(&doc.scopes_supported);

    // Must have at least `openid` scope to be useful
    if !scopes.iter().any(|s| s == "openid") {
        log::debug!("OIDC discovery: {url} does not support 'openid' scope");
        return None;
    }

    let supports_pkce_s256 = detect_pkce_s256(&doc.code_challenge_methods_supported);
    let supports_public_client = detect_public_client(&doc.token_endpoint_auth_methods_supported);

    log::info!(
        "OIDC discovery: found endpoints at {normalized_issuer} \
         (PKCE: {supports_pkce_s256}, public: {supports_public_client})"
    );

    Some(OidcEndpoints {
        issuer_url: normalized_issuer.to_string(),
        auth_url: doc.authorization_endpoint,
        token_url: doc.token_endpoint,
        scopes,
        supports_pkce_s256,
        supports_public_client,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiate_scopes_intersects_supported() {
        let supported = vec![
            "openid".into(),
            "email".into(),
            "groups".into(),
            "offline_access".into(),
        ];
        let result = negotiate_scopes(&supported);
        assert_eq!(result, vec!["openid", "email", "offline_access"]);
    }

    #[test]
    fn negotiate_scopes_defaults_when_absent() {
        let result = negotiate_scopes(&[]);
        assert_eq!(result, vec!["openid", "email", "profile", "offline_access"]);
    }

    #[test]
    fn negotiate_scopes_rejects_no_openid() {
        let supported = vec!["email".into(), "profile".into()];
        let result = negotiate_scopes(&supported);
        assert!(!result.iter().any(|s| s == "openid"));
        // Caller should check and reject
    }

    #[test]
    fn pkce_assumed_when_absent() {
        assert!(detect_pkce_s256(&[]));
    }

    #[test]
    fn pkce_detected_when_present() {
        assert!(detect_pkce_s256(&["S256".into(), "plain".into()]));
    }

    #[test]
    fn pkce_false_when_only_plain() {
        assert!(!detect_pkce_s256(&["plain".into()]));
    }

    #[test]
    fn public_client_detected() {
        assert!(detect_public_client(&[
            "client_secret_basic".into(),
            "none".into()
        ]));
    }

    #[test]
    fn public_client_absent() {
        assert!(!detect_public_client(&["client_secret_basic".into()]));
    }

    #[test]
    fn https_url_validation() {
        assert!(is_valid_https_url("https://auth.example.com/authorize"));
        assert!(!is_valid_https_url("http://auth.example.com/authorize"));
        assert!(!is_valid_https_url(""));
        assert!(!is_valid_https_url("https://"));
    }
}
