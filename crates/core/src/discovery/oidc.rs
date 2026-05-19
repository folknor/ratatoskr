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
    /// RFC 7591 dynamic client registration endpoint. Optional - many
    /// production IdPs (especially commercial ones) don't expose it.
    #[serde(default)]
    registration_endpoint: Option<String>,
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
    /// RFC 7591 dynamic client registration endpoint, if the discovery
    /// document advertised one. `None` for providers that require
    /// pre-registered clients.
    pub registration_endpoint: Option<String>,
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

/// Validate that an OIDC endpoint URL is suitable to send credentials to.
///
/// Per OIDC Discovery (RFC 8414) the endpoints in the discovery document
/// MUST use HTTPS. The previous `starts_with("https://")` check accepted
/// any string that began with the literal prefix - including
/// `https://attacker.example/?@victim.example` (URL-confusion via embedded
/// userinfo) and `https:///path` (no host). Parse the URL properly and
/// require: HTTPS scheme, a host present, no embedded userinfo (so a
/// crafted issuer can't bypass our notion of which host the user authorized),
/// no fragment.
pub(super) fn is_valid_https_url(url: &str) -> bool {
    is_valid_url_with_test_base(
        url,
        std::env::var(super::DISCOVERY_TEST_BASE_ENV).ok().as_deref(),
    )
}

/// Pure form of `is_valid_https_url` with the test base injected.
/// Production callers go through `is_valid_https_url` which reads the
/// env var; tests pass an explicit base (or `None`).
fn is_valid_url_with_test_base(url: &str, test_base: Option<&str>) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    if !parsed.has_host() {
        return false;
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return false;
    }
    if parsed.fragment().is_some() {
        return false;
    }
    match parsed.scheme() {
        "https" => true,
        // Test-mode escape: an `http://` URL passes only when it points at
        // the configured discovery test base (saehrimnir on localhost).
        // Production builds never set the env var, so this branch is
        // unreachable outside the harness.
        "http" => test_base
            .and_then(|base| url::Url::parse(base).ok())
            .is_some_and(|base_parsed| {
                parsed.scheme() == base_parsed.scheme()
                    && parsed.host_str() == base_parsed.host_str()
                    && parsed.port_or_known_default()
                        == base_parsed.port_or_known_default()
            }),
        _ => false,
    }
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
    let url = super::rewrite_for_test_harness(&url);

    // OIDC discovery is required to be HTTPS end-to-end. The shared client
    // builder enforces `https_only(true)` in production and relaxes it only
    // when `RATATOSKR_TEST_DISCOVERY_BASE` is set; the 3-hop redirect cap
    // bounds redirect-walk attacks in both modes.
    let client = super::discovery_client()?;

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

    // Only carry forward a registration endpoint we'd actually use - if
    // the IdP advertises a plaintext or otherwise malformed URL here, we
    // do not want a later `dyn_registration::register` call to surface
    // that bug; drop it at discovery time.
    let registration_endpoint = doc
        .registration_endpoint
        .as_deref()
        .filter(|url| is_valid_https_url(url))
        .map(str::to_string);

    log::info!(
        "OIDC discovery: found endpoints at {normalized_issuer} \
         (PKCE: {supports_pkce_s256}, public: {supports_public_client}, \
          dyn-reg: {})",
        registration_endpoint.is_some()
    );

    Some(OidcEndpoints {
        issuer_url: normalized_issuer.to_string(),
        auth_url: doc.authorization_endpoint,
        token_url: doc.token_endpoint,
        scopes,
        supports_pkce_s256,
        supports_public_client,
        registration_endpoint,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_valid_url_rejects_userinfo_and_fragment() {
        assert!(!is_valid_url_with_test_base("https://attacker@victim.com", None));
        assert!(!is_valid_url_with_test_base("https://victim.com/path#frag", None));
    }

    #[test]
    fn is_valid_url_https_passes_regardless_of_test_base() {
        assert!(is_valid_url_with_test_base("https://idp.example.com", None));
        assert!(is_valid_url_with_test_base(
            "https://idp.example.com",
            Some("http://127.0.0.1:12345"),
        ));
    }

    #[test]
    fn is_valid_url_http_only_passes_against_matching_test_base() {
        // Same host + port as the test base: allowed.
        assert!(is_valid_url_with_test_base(
            "http://127.0.0.1:12345/idp/realms/corp",
            Some("http://127.0.0.1:12345"),
        ));
        // Different host: rejected.
        assert!(!is_valid_url_with_test_base(
            "http://attacker.example.com/",
            Some("http://127.0.0.1:12345"),
        ));
        // Same host, different port: rejected.
        assert!(!is_valid_url_with_test_base(
            "http://127.0.0.1:9999/",
            Some("http://127.0.0.1:12345"),
        ));
        // No test base at all: rejected.
        assert!(!is_valid_url_with_test_base(
            "http://127.0.0.1:12345/",
            None,
        ));
    }

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

    #[test]
    fn https_url_rejects_userinfo_and_other_schemes() {
        // Embedded credentials in the issuer's endpoint URL would let the
        // server bypass our notion of "which host the user authorized."
        assert!(!is_valid_https_url("https://attacker@victim.example/path"));
        assert!(!is_valid_https_url(
            "https://user:pass@victim.example/path"
        ));
        // Non-HTTPS schemes are rejected (RFC 8414 mandates HTTPS).
        assert!(!is_valid_https_url("javascript:alert(1)"));
        assert!(!is_valid_https_url("data:text/plain,foo"));
        assert!(!is_valid_https_url("file:///etc/passwd"));
        // Fragment identifiers don't make sense on an OIDC endpoint and
        // their presence is a red flag.
        assert!(!is_valid_https_url("https://example.com/auth#token=stolen"));
    }
}
