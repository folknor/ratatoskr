//! WebFinger Discovery (RFC 7033) - resolve an email address to its OIDC issuer URL.
//!
//! Per OpenID Connect Discovery 1.0 §2, the email domain advertises an OIDC issuer
//! by serving a WebFinger document. This bridges the gap where the email domain
//! (corp.com) and the IdP (auth.corp.com) are different hosts - the bare-domain
//! probe in `oidc::probe` can't reach the IdP, but the email domain can delegate
//! to it via WebFinger.

use serde::Deserialize;

/// Max response body size for a WebFinger JRD response (1 MB).
const MAX_BODY_SIZE: usize = 1_024 * 1_024;

/// OIDC issuer relation identifier per OpenID Connect Discovery 1.0 §2.
///
/// This is a URI used as an identifier string (the literal "http://..."), not
/// a URL to fetch. The `http://` scheme is part of the standardized identifier.
const OIDC_ISSUER_REL: &str = "http://openid.net/specs/connect/1.0/issuer";

#[derive(Debug, Deserialize)]
struct WebFingerResponse {
    #[serde(default)]
    links: Vec<WebFingerLink>,
}

#[derive(Debug, Deserialize)]
struct WebFingerLink {
    #[serde(default)]
    rel: String,
    #[serde(default)]
    href: Option<String>,
}

/// Build the WebFinger query URL for an OIDC issuer lookup.
///
/// `resource` and `rel` are percent-encoded by the url crate's query serializer.
fn build_query_url(domain: &str, email: &str) -> Option<String> {
    let resource = format!("acct:{email}");
    let url = url::Url::parse_with_params(
        &format!("https://{domain}/.well-known/webfinger"),
        &[("resource", resource.as_str()), ("rel", OIDC_ISSUER_REL)],
    )
    .ok()?;
    Some(url.into())
}

/// Extract the first valid OIDC issuer href from a parsed WebFinger response.
///
/// Returns the first link whose `rel` matches the OIDC issuer relation and
/// whose `href` is a non-empty HTTPS URL. Earlier matching links with bad
/// hrefs (non-HTTPS, empty, missing) are skipped, not treated as hard failures.
fn extract_issuer(resp: &WebFingerResponse) -> Option<String> {
    resp.links.iter().find_map(|l| {
        if l.rel != OIDC_ISSUER_REL {
            return None;
        }
        let href = l.href.as_deref()?.trim();
        if href.is_empty() {
            return None;
        }
        if !super::oidc::is_valid_https_url(href) {
            log::debug!("WebFinger: rejecting non-HTTPS href: {href}");
            return None;
        }
        Some(href.to_string())
    })
}

/// Parse a WebFinger JRD response body and extract the OIDC issuer URL.
fn parse_response(bytes: &[u8]) -> Option<String> {
    let resp: WebFingerResponse = serde_json::from_slice(bytes).ok()?;
    extract_issuer(&resp)
}

/// Probe `https://{domain}/.well-known/webfinger` for an OIDC issuer URL.
///
/// Returns the issuer URL on success, `None` for any failure (network error,
/// 404, parse error, no matching link, non-HTTPS href). WebFinger is
/// best-effort; absence is not an error - it just means the email domain
/// doesn't advertise an OIDC issuer.
pub async fn probe(domain: &str, email: &str) -> Option<String> {
    let url = build_query_url(domain, email)?;
    let url = super::rewrite_for_test_harness(&url);

    // WebFinger is required to be served over HTTPS (RFC 7033 §4.2). The
    // shared client builder enforces `https_only(true)` in production and
    // relaxes it only when `RATATOSKR_TEST_DISCOVERY_BASE` is set; the
    // 3-hop redirect cap bounds redirect-walk attacks in both modes.
    let client = super::discovery_client()?;

    let resp = client
        .get(&url)
        .header(reqwest::header::ACCEPT, "application/jrd+json")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        log::debug!("WebFinger: {url} returned {}", resp.status());
        return None;
    }

    if resp
        .content_length()
        .is_some_and(|len| len > MAX_BODY_SIZE as u64)
    {
        log::debug!("WebFinger: response too large from {url}");
        return None;
    }

    let bytes = resp.bytes().await.ok()?;
    if bytes.len() > MAX_BODY_SIZE {
        log::debug!("WebFinger: response body too large from {url}");
        return None;
    }

    let issuer = parse_response(&bytes)?;
    log::info!("WebFinger: resolved issuer {issuer} for domain {domain}");
    Some(issuer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_query_url_encodes_resource_and_rel() {
        let url = build_query_url("example.com", "user@example.com")
            .expect("valid inputs should produce a URL");
        assert!(url.starts_with("https://example.com/.well-known/webfinger?"));
        assert!(
            url.contains("resource=acct%3Auser%40example.com"),
            "resource not encoded: {url}"
        );
        assert!(
            url.contains("rel=http%3A%2F%2Fopenid.net%2Fspecs%2Fconnect%2F1.0%2Fissuer"),
            "rel not encoded: {url}"
        );
    }

    #[test]
    fn parse_response_extracts_issuer() {
        let body = br#"{
            "subject": "acct:user@example.com",
            "links": [
                {"rel": "http://openid.net/specs/connect/1.0/issuer",
                 "href": "https://auth.example.com"}
            ]
        }"#;
        assert_eq!(
            parse_response(body),
            Some("https://auth.example.com".to_string())
        );
    }

    #[test]
    fn parse_response_picks_matching_rel_among_many() {
        let body = br#"{
            "links": [
                {"rel": "self", "href": "https://example.com/user/self"},
                {"rel": "http://openid.net/specs/connect/1.0/issuer",
                 "href": "https://auth.example.com"},
                {"rel": "avatar", "href": "https://example.com/avatar.png"}
            ]
        }"#;
        assert_eq!(
            parse_response(body),
            Some("https://auth.example.com".to_string())
        );
    }

    #[test]
    fn parse_response_none_when_no_matching_rel() {
        let body = br#"{
            "links": [{"rel": "self", "href": "https://example.com/user"}]
        }"#;
        assert_eq!(parse_response(body), None);
    }

    #[test]
    fn parse_response_none_when_links_missing() {
        let body = br#"{"subject": "acct:user@example.com"}"#;
        assert_eq!(parse_response(body), None);
    }

    #[test]
    fn parse_response_none_when_href_missing() {
        let body = br#"{
            "links": [{"rel": "http://openid.net/specs/connect/1.0/issuer"}]
        }"#;
        assert_eq!(parse_response(body), None);
    }

    #[test]
    fn parse_response_none_when_href_empty() {
        let body = br#"{
            "links": [
                {"rel": "http://openid.net/specs/connect/1.0/issuer",
                 "href": ""}
            ]
        }"#;
        assert_eq!(parse_response(body), None);
    }

    #[test]
    fn parse_response_rejects_non_https_href() {
        let body = br#"{
            "links": [
                {"rel": "http://openid.net/specs/connect/1.0/issuer",
                 "href": "http://auth.example.com"}
            ]
        }"#;
        assert_eq!(parse_response(body), None);
    }

    #[test]
    fn parse_response_rejects_href_with_userinfo() {
        // is_valid_https_url rejects embedded userinfo; ensure the WebFinger
        // path inherits that rejection.
        let body = br#"{
            "links": [
                {"rel": "http://openid.net/specs/connect/1.0/issuer",
                 "href": "https://attacker@victim.example.com"}
            ]
        }"#;
        assert_eq!(parse_response(body), None);
    }

    #[test]
    fn parse_response_skips_invalid_href_picks_next_valid() {
        // Multiple matching rels: an earlier link with a bad href is skipped,
        // a later link with a valid HTTPS href is returned.
        let body = br#"{
            "links": [
                {"rel": "http://openid.net/specs/connect/1.0/issuer",
                 "href": "http://insecure.example.com"},
                {"rel": "http://openid.net/specs/connect/1.0/issuer",
                 "href": "https://auth.example.com"}
            ]
        }"#;
        assert_eq!(
            parse_response(body),
            Some("https://auth.example.com".to_string())
        );
    }

    #[test]
    fn parse_response_none_on_malformed_json() {
        assert_eq!(parse_response(b"not json"), None);
        assert_eq!(parse_response(b""), None);
        assert_eq!(parse_response(b"{"), None);
    }

    #[test]
    fn parse_response_handles_extra_fields() {
        // RFC 7033 allows additional JRD fields - aliases, properties, titles.
        // Make sure they don't break our parse.
        let body = br#"{
            "subject": "acct:user@example.com",
            "aliases": ["mailto:user@example.com"],
            "properties": {"http://example.com/role": "admin"},
            "links": [
                {"rel": "http://openid.net/specs/connect/1.0/issuer",
                 "href": "https://auth.example.com",
                 "type": "application/json",
                 "titles": {"en": "Corp SSO"}}
            ]
        }"#;
        assert_eq!(
            parse_response(body),
            Some("https://auth.example.com".to_string())
        );
    }
}
