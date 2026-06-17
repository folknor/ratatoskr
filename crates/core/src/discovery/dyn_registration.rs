//! OAuth 2.0 Dynamic Client Registration (RFC 7591).
//!
//! Lets a client register itself with an OIDC provider at runtime, obtaining
//! a `client_id` (and optionally `client_secret`) without pre-registration by
//! an IT admin. Useful for on-prem deployments (Keycloak, Authentik) that
//! expose `registration_endpoint` in their discovery document.
//!
//! For desktop / native-app deployments we register as a public client per
//! RFC 8252: `token_endpoint_auth_method = "none"`, PKCE only, no client
//! secret. Confidential-client registration (with secret) is out of scope
//! here - it'd require a secret-handling story that the wider app doesn't
//! yet have.

use serde::{Deserialize, Serialize};

/// Max response body size for an RFC 7591 registration response (1 MB).
const MAX_BODY_SIZE: usize = 1_024 * 1_024;

/// Request body for RFC 7591 dynamic client registration.
///
/// All fields are owned `Vec<String>` / `String` to keep `Send` and avoid
/// lifetime entanglements with the caller's data. The caller assembles the
/// arrays once at registration time; this is not a hot path.
#[derive(Debug, Clone, Serialize)]
pub struct RegistrationRequest {
    pub redirect_uris: Vec<String>,
    pub client_name: String,
    pub grant_types: Vec<String>,
    pub response_types: Vec<String>,
    pub token_endpoint_auth_method: String,
    /// Space-separated scope string per RFC 6749 §3.3.
    pub scope: String,
}

/// Subset of the RFC 7591 registration response we actually consume.
///
/// The server may return many more fields (`client_id_issued_at`,
/// `client_secret_expires_at`, `registration_access_token`, …) - we ignore
/// them. Future RFC 7592 (client management) work can extend the parser.
#[derive(Debug, Clone, Deserialize)]
pub struct RegisteredClient {
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
}

/// Default registration request shape for a public Ratatoskr desktop client.
///
/// `redirect_uris` and `scope` are caller-supplied; everything else is the
/// RFC 8252 "OAuth 2.0 for Native Apps" recommended set.
pub fn public_client_request(redirect_uri: &str, scope: &str) -> RegistrationRequest {
    RegistrationRequest {
        redirect_uris: vec![redirect_uri.to_string()],
        client_name: "Ratatoskr".to_string(),
        grant_types: vec![
            "authorization_code".to_string(),
            "refresh_token".to_string(),
        ],
        response_types: vec!["code".to_string()],
        token_endpoint_auth_method: "none".to_string(),
        scope: scope.to_string(),
    }
}

/// Register a client with the given RFC 7591 endpoint.
///
/// Returns `Some(RegisteredClient)` on success, `None` for any failure
/// (network error, non-HTTPS endpoint, non-2xx status, parse error, missing
/// `client_id` in the response). Best-effort: the caller decides whether to
/// surface a failure to the user or fall back to a manual flow.
pub async fn register(endpoint: &str, request: &RegistrationRequest) -> Option<RegisteredClient> {
    if !super::oidc::is_valid_https_url(endpoint) {
        log::debug!("dyn_registration: rejecting non-HTTPS endpoint {endpoint}");
        return None;
    }
    let endpoint_url = super::rewrite_for_test_harness(endpoint);

    let client = super::discovery_client()?;

    let resp = client
        .post(&endpoint_url)
        .header(reqwest::header::ACCEPT, "application/json")
        .json(request)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        log::debug!("dyn_registration: {endpoint} returned {}", resp.status());
        return None;
    }

    if resp
        .content_length()
        .is_some_and(|len| len > MAX_BODY_SIZE as u64)
    {
        log::debug!("dyn_registration: response too large from {endpoint}");
        return None;
    }

    let bytes = resp.bytes().await.ok()?;
    if bytes.len() > MAX_BODY_SIZE {
        log::debug!("dyn_registration: response body too large from {endpoint}");
        return None;
    }

    parse_response(&bytes)
}

fn parse_response(bytes: &[u8]) -> Option<RegisteredClient> {
    let parsed: RegisteredClient = serde_json::from_slice(bytes).ok()?;
    if parsed.client_id.trim().is_empty() {
        return None;
    }
    // Treat an empty-string secret the same as missing - some IdPs serialize
    // it as `""` rather than omitting the field for public clients.
    let client_secret = parsed.client_secret.filter(|s| !s.trim().is_empty());
    Some(RegisteredClient {
        client_id: parsed.client_id,
        client_secret,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_client_request_shape() {
        let req = public_client_request("http://127.0.0.1:17248/callback", "openid email");
        assert_eq!(req.redirect_uris, vec!["http://127.0.0.1:17248/callback"]);
        assert_eq!(req.client_name, "Ratatoskr");
        assert_eq!(req.grant_types, vec!["authorization_code", "refresh_token"]);
        assert_eq!(req.response_types, vec!["code"]);
        assert_eq!(req.token_endpoint_auth_method, "none");
        assert_eq!(req.scope, "openid email");
    }

    #[test]
    fn registration_request_serializes_as_rfc7591_json() {
        let req = public_client_request("http://localhost/callback", "openid");
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["client_name"], "Ratatoskr");
        assert_eq!(json["token_endpoint_auth_method"], "none");
        assert!(json["redirect_uris"].is_array());
        assert!(json["grant_types"].is_array());
        assert!(json["response_types"].is_array());
        assert_eq!(json["scope"], "openid");
    }

    #[test]
    fn parse_response_public_client_no_secret() {
        let body = br#"{"client_id": "abc123"}"#;
        let parsed = parse_response(body).expect("should parse");
        assert_eq!(parsed.client_id, "abc123");
        assert_eq!(parsed.client_secret, None);
    }

    #[test]
    fn parse_response_confidential_client_with_secret() {
        let body = br#"{"client_id": "abc123", "client_secret": "shh"}"#;
        let parsed = parse_response(body).expect("should parse");
        assert_eq!(parsed.client_id, "abc123");
        assert_eq!(parsed.client_secret, Some("shh".to_string()));
    }

    #[test]
    fn parse_response_empty_secret_treated_as_none() {
        let body = br#"{"client_id": "abc123", "client_secret": ""}"#;
        let parsed = parse_response(body).expect("should parse");
        assert_eq!(parsed.client_secret, None);
    }

    #[test]
    fn parse_response_ignores_extra_fields() {
        let body = br#"{
            "client_id": "abc123",
            "client_id_issued_at": 1700000000,
            "client_secret_expires_at": 0,
            "registration_access_token": "ratoken",
            "registration_client_uri": "https://idp.example.com/clients/abc123"
        }"#;
        let parsed = parse_response(body).expect("should parse");
        assert_eq!(parsed.client_id, "abc123");
        assert_eq!(parsed.client_secret, None);
    }

    #[test]
    fn parse_response_rejects_missing_client_id() {
        let body = br#"{"client_secret": "shh"}"#;
        assert!(parse_response(body).is_none());
    }

    #[test]
    fn parse_response_rejects_empty_client_id() {
        let body = br#"{"client_id": ""}"#;
        assert!(parse_response(body).is_none());
        let body = br#"{"client_id": "   "}"#;
        assert!(parse_response(body).is_none());
    }

    #[test]
    fn parse_response_rejects_malformed_json() {
        assert!(parse_response(b"not json").is_none());
        assert!(parse_response(b"").is_none());
        assert!(parse_response(b"{").is_none());
    }
}
