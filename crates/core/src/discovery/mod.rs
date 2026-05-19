pub mod autoconfig;
pub mod dyn_registration;
pub mod jmap_wellknown;
pub mod merge;
pub mod mx;
pub mod oidc;
pub mod probe;
pub mod registry;
pub mod types;
pub mod webfinger;

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Notify;
use types::{DiscoveredConfig, ProtocolOption, StageDiagnostic, StageOutcome};

/// Overall timeout for the entire discovery operation.
const OVERALL_TIMEOUT: Duration = Duration::from_secs(15);

/// Run the full discovery cascade for an email address.
pub async fn discover(email: &str) -> Result<DiscoveredConfig, String> {
    let email = email.trim().to_lowercase();
    log::info!("Starting email provider discovery for {email}");
    let domain = extract_domain(&email)?;

    match tokio::time::timeout(OVERALL_TIMEOUT, run_cascade(&email, &domain)).await {
        Ok(Ok(config)) => {
            log::info!(
                "Discovery completed for {email}: {} protocol options found",
                config.options.len()
            );
            Ok(config)
        }
        Ok(Err(e)) => {
            log::error!("Discovery failed for {email}: {e}");
            Err(e)
        }
        Err(_) => {
            log::error!("Discovery timed out for {email} after 15 seconds");
            Err("Discovery timed out after 15 seconds".to_string())
        }
    }
}

async fn run_cascade(email: &str, domain: &str) -> Result<DiscoveredConfig, String> {
    let imap_found = Arc::new(Notify::new());

    // Run WebFinger + OIDC probe in parallel with stages 1-4. WebFinger lets
    // the email domain delegate to an IdP at a different host (corp.com →
    // auth.corp.com); if it returns an issuer we probe that. Otherwise fall
    // back to the bare-domain `.well-known/openid-configuration` probe.
    let (d_oidc, e_oidc) = (domain.to_string(), email.to_string());
    let oidc_handle = tokio::spawn(async move {
        let wf_start = Instant::now();
        let wf_issuer = webfinger::probe(&d_oidc, &e_oidc).await;
        let wf_diag = StageDiagnostic {
            stage: "webfinger",
            duration_ms: elapsed_ms(wf_start),
            outcome: if wf_issuer.is_some() {
                StageOutcome::Found { count: 1 }
            } else {
                StageOutcome::NotFound
            },
        };

        let oidc_start = Instant::now();
        let endpoints = match &wf_issuer {
            Some(issuer) => oidc::probe_issuer(issuer).await,
            None => oidc::probe(&d_oidc).await,
        };
        let oidc_diag = StageDiagnostic {
            stage: "oidc_discovery",
            duration_ms: elapsed_ms(oidc_start),
            outcome: if endpoints.is_some() {
                StageOutcome::Found { count: 1 }
            } else {
                StageOutcome::NotFound
            },
        };
        (endpoints, wf_diag, oidc_diag)
    });

    let (stages_1_4, resolved_domain) = run_stages(domain, email, &imap_found).await;
    let stage_5 = run_probe_stage(domain, &imap_found, &stages_1_4).await;

    let (oidc_endpoints, wf_diag, oidc_diag) = oidc_handle.await.unwrap_or_else(|_| {
        (
            None,
            err_diag("webfinger"),
            err_diag("oidc_discovery"),
        )
    });

    let (mut all_results, mut diagnostics) = unpack_stages(stages_1_4);
    all_results.push(stage_5.0);
    diagnostics.push(stage_5.1);
    diagnostics.push(wf_diag);
    diagnostics.push(oidc_diag);

    let mut options = merge::merge_and_rank(all_results);

    // Post-process: upgrade OAuth2Unsupported results using OIDC endpoints
    if let Some(ref endpoints) = oidc_endpoints
        && upgrade_oauth2_unsupported(&mut options, endpoints)
    {
        // Re-sort: upgraded options now have OidcWellKnown source
        // (confidence 1) which may change their ranking.
        merge::re_rank(&mut options);
    }

    Ok(DiscoveredConfig {
        email: email.to_string(),
        domain: domain.to_string(),
        options,
        resolved_domain,
        oidc_endpoints,
        diagnostics,
    })
}

/// Upgrade `AuthMethod::OAuth2Unsupported` to `AuthMethod::OAuth2` using
/// the OIDC discovery endpoints. This bridges the gap where autoconfig or
/// MX lookup found IMAP/SMTP servers with OAuth2 authentication but couldn't
/// resolve the specific OAuth2 endpoints.
///
/// Only upgrades when the `provider_domain` from the OAuth2Unsupported result
/// matches the OIDC issuer domain - prevents assigning wrong OIDC endpoints
/// to third-party IMAP servers discovered via autoconfig.
fn upgrade_oauth2_unsupported(
    options: &mut [ProtocolOption],
    endpoints: &oidc::OidcEndpoints,
) -> bool {
    let mut upgraded = false;
    let issuer_domain = endpoints
        .issuer_url
        .strip_prefix("https://")
        .unwrap_or(&endpoints.issuer_url)
        .split('/')
        .next()
        .unwrap_or("");

    for opt in options.iter_mut() {
        if let types::AuthMethod::OAuth2Unsupported {
            ref provider_domain,
        } = opt.auth.method
        {
            if !domains_related(provider_domain, issuer_domain) {
                log::debug!(
                    "OIDC upgrade skipped: provider_domain={provider_domain} \
                     does not match issuer={issuer_domain}"
                );
                continue;
            }
            log::info!(
                "Upgrading OAuth2Unsupported to OAuth2 via OIDC discovery for {:?}",
                opt.protocol
            );
            opt.auth.method = types::AuthMethod::OAuth2 {
                provider_id: format!("oidc:{}", endpoints.issuer_url),
                auth_url: endpoints.auth_url.clone(),
                token_url: endpoints.token_url.clone(),
                scopes: endpoints.scopes.clone(),
                use_pkce: endpoints.supports_pkce_s256,
            };
            opt.source = types::DiscoverySource::OidcWellKnown;
            upgraded = true;
        }
    }
    upgraded
}

/// Check if two domains are related (same domain or one is a subdomain of the other).
/// e.g., "auth.corp.example.com" and "corp.example.com" are related.
fn domains_related(a: &str, b: &str) -> bool {
    let a = a.to_lowercase();
    let b = b.to_lowercase();
    a == b || a.ends_with(&format!(".{b}")) || b.ends_with(&format!(".{a}"))
}

/// Extract the domain from an email address.
///
/// The returned string is fed into HTTPS probes (`autoconfig.{domain}`,
/// `https://{domain}/.well-known/...`, etc.). Without validation, an address
/// like `evil@10.0.0.1` or `evil@host:8080/path` turns the discovery cascade
/// into a probe-flavor SSRF oracle for hosts on the user's network. Only
/// admit shapes that are unambiguously a hostname pointing at a public host.
fn extract_domain(email: &str) -> Result<String, String> {
    let domain = email
        .split('@')
        .nth(1)
        .ok_or_else(|| "Invalid email address: missing @".to_string())?;
    if domain.is_empty() {
        return Err("Invalid email address: empty domain".to_string());
    }
    let bad_byte = |b: u8| {
        matches!(
            b,
            b':' | b'/' | b'\\' | b'?' | b'#' | b'@' | b'[' | b']' | b' ' | b'\t'
        ) || b.is_ascii_control()
    };
    if domain.bytes().any(bad_byte) {
        return Err("Invalid email address: bad domain characters".to_string());
    }
    if let Ok(v4) = domain.parse::<Ipv4Addr>()
        && !is_public_v4(&v4)
    {
        return Err("Invalid email address: non-routable IP".to_string());
    }
    // `Ipv4Addr::from_str` is strict dotted-quad, but the system resolver
    // happily expands shorthand forms (`127.1` -> `127.0.0.1`,
    // `0x7f.1` -> loopback, etc.). Reject any all-numeric host so a
    // hand-crafted address can't sneak past via the resolver.
    if domain.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return Err("Invalid email address: numeric host disallowed".to_string());
    }
    if !domain.contains('.') {
        return Err("Invalid email address: bad domain".to_string());
    }
    Ok(domain.to_string())
}

/// Is this IPv4 address routable on the public internet?
///
/// Rejects loopback, RFC 1918 private space, link-local, multicast, broadcast,
/// the unspecified address, RFC 5737 documentation ranges, and RFC 6598
/// carrier-grade NAT (`100.64.0.0/10`). Bracketed / bare IPv6 literals are
/// already rejected upstream by the bad-character filter, so v4 is the only
/// shape that can reach this check.
fn is_public_v4(v4: &Ipv4Addr) -> bool {
    if v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.is_broadcast()
        || v4.is_multicast()
        || v4.is_documentation()
    {
        return false;
    }
    let octets = v4.octets();
    !(octets[0] == 100 && (octets[1] & 0xC0) == 64)
}

fn elapsed_ms(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Env var that, when set, redirects discovery probes to a local test
/// harness over plain HTTP. Read by `discovery_client` and
/// `rewrite_for_test_harness`; never set in production builds, so
/// production paths run with the same hardening as before.
///
/// Saehrimnir mounts the discovery routes (WebFinger, OIDC, autoconfig)
/// on its JMAP HTTP listener, so we reuse the existing
/// `RATATOSKR_TEST_JMAP_ENDPOINT` plumbing rather than adding a new
/// brokkr-side env-var slot. Functionally identical.
const DISCOVERY_TEST_BASE_ENV: &str = "RATATOSKR_TEST_JMAP_ENDPOINT";

/// Build the shared reqwest client for discovery probes
/// (`webfinger::probe`, `oidc::probe_issuer`, `dyn_registration::register`).
///
/// `https_only` is relaxed when `RATATOSKR_TEST_DISCOVERY_BASE` is set so
/// the probes can reach saehrimnir on plain HTTP localhost. The env var is
/// the only gate; production builds where it is unset get the same
/// `https_only(true)` posture they had before this helper existed.
pub(super) fn discovery_client() -> Option<reqwest::Client> {
    let test_mode = std::env::var(DISCOVERY_TEST_BASE_ENV).is_ok();
    reqwest::Client::builder()
        .https_only(!test_mode)
        .timeout(crate::constants::DISCOVERY_HTTP_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(3))
        .user_agent("Ratatoskr/1.0")
        .build()
        .ok()
}

/// If `RATATOSKR_TEST_DISCOVERY_BASE` is set, rewrite an `https://{host}/...`
/// URL so the request lands on saehrimnir at `${BASE}/{host}/...`. Query and
/// fragment are preserved. Plain HTTP URLs (which only appear in test mode
/// for chained-issuer documents whose absolute href already points at the
/// test base) pass through unchanged. Production: env var unset, function
/// is a no-op.
pub(super) fn rewrite_for_test_harness(url: &str) -> String {
    match std::env::var(DISCOVERY_TEST_BASE_ENV) {
        Ok(base) => rewrite_with_base(url, &base),
        Err(_) => url.to_string(),
    }
}

/// Pure rewrite given an explicit base URL - separated so tests don't have
/// to touch process-global env state.
fn rewrite_with_base(url: &str, base: &str) -> String {
    let Ok(parsed) = url::Url::parse(url) else {
        return url.to_string();
    };
    if parsed.scheme() != "https" {
        return url.to_string();
    }
    let host = parsed.host_str().unwrap_or("");
    let mut rewritten = String::with_capacity(url.len() + base.len());
    rewritten.push_str(base.trim_end_matches('/'));
    rewritten.push('/');
    rewritten.push_str(host);
    rewritten.push_str(parsed.path());
    if let Some(query) = parsed.query() {
        rewritten.push('?');
        rewritten.push_str(query);
    }
    if let Some(fragment) = parsed.fragment() {
        rewritten.push('#');
        rewritten.push_str(fragment);
    }
    rewritten
}

#[cfg(test)]
mod test_harness_tests {
    use super::rewrite_with_base;

    #[test]
    fn rewrite_https_url_preserves_host_path_query() {
        assert_eq!(
            rewrite_with_base(
                "https://corp.test/.well-known/webfinger?resource=acct%3Auser%40corp.test&rel=http%3A%2F%2Fopenid.net%2Fspecs%2Fconnect%2F1.0%2Fissuer",
                "http://127.0.0.1:12345",
            ),
            "http://127.0.0.1:12345/corp.test/.well-known/webfinger?resource=acct%3Auser%40corp.test&rel=http%3A%2F%2Fopenid.net%2Fspecs%2Fconnect%2F1.0%2Fissuer",
        );
    }

    #[test]
    fn rewrite_https_url_trailing_slash_in_base_is_stripped() {
        assert_eq!(
            rewrite_with_base(
                "https://corp.test/.well-known/openid-configuration",
                "http://127.0.0.1:12345/",
            ),
            "http://127.0.0.1:12345/corp.test/.well-known/openid-configuration",
        );
    }

    #[test]
    fn rewrite_passes_through_non_https() {
        // Plain HTTP at the test base (a chained-issuer URL saehrimnir
        // already emitted as absolute) must not be rewritten - the request
        // already points where it needs to go.
        let original = "http://127.0.0.1:12345/idp/realms/corp/.well-known/openid-configuration";
        assert_eq!(rewrite_with_base(original, "http://127.0.0.1:12345"), original);
    }

    #[test]
    fn rewrite_passes_through_unparseable_url() {
        assert_eq!(rewrite_with_base("not a url", "http://127.0.0.1:12345"), "not a url");
    }

    #[test]
    fn rewrite_preserves_fragment() {
        assert_eq!(
            rewrite_with_base(
                "https://corp.test/.well-known/openid-configuration#section",
                "http://127.0.0.1:12345",
            ),
            "http://127.0.0.1:12345/corp.test/.well-known/openid-configuration#section",
        );
    }
}

fn make_diag(stage: &'static str, start: Instant, results: &[ProtocolOption]) -> StageDiagnostic {
    log::debug!(
        "Discovery stage '{stage}' completed: {} results in {}ms",
        results.len(),
        elapsed_ms(start)
    );
    let outcome = if results.is_empty() {
        StageOutcome::NotFound
    } else {
        StageOutcome::Found {
            count: results.len(),
        }
    };
    StageDiagnostic {
        stage,
        duration_ms: elapsed_ms(start),
        outcome,
    }
}

type StageResult = (Vec<ProtocolOption>, StageDiagnostic);
type MxResult = (Vec<ProtocolOption>, Option<String>, StageDiagnostic);

async fn run_stages(
    domain: &str,
    email: &str,
    imap_found: &Arc<Notify>,
) -> (
    (StageResult, StageResult, MxResult, StageResult),
    Option<String>,
) {
    let d1 = domain.to_string();
    let s1 = tokio::spawn(async move {
        let start = Instant::now();
        let results = registry::lookup(&d1);
        let diag = make_diag("registry", start, &results);
        (results, diag)
    });

    let (d2, e2) = (domain.to_string(), email.to_string());
    let s2 = tokio::spawn(async move {
        let start = Instant::now();
        let results = autoconfig::fetch(&d2, &e2).await;
        let diag = make_diag("autoconfig", start, &results);
        (results, diag)
    });

    let (d3, e3) = (domain.to_string(), email.to_string());
    let s3 = tokio::spawn(async move {
        let start = Instant::now();
        let (results, resolved) = mx::lookup(&d3, &e3).await;
        let diag = make_diag("mx_lookup", start, &results);
        (results, resolved, diag)
    });

    let d4 = domain.to_string();
    let s4 = tokio::spawn(async move {
        let start = Instant::now();
        let results: Vec<ProtocolOption> = jmap_wellknown::probe(&d4).await.into_iter().collect();
        let diag = make_diag("jmap_wellknown", start, &results);
        (results, diag)
    });

    let (r1, r2, r3, r4) = tokio::join!(s1, s2, s3, s4);

    let r1 = r1.unwrap_or_else(|_| (Vec::new(), err_diag("registry")));
    let r2 = r2.unwrap_or_else(|_| (Vec::new(), err_diag("autoconfig")));
    let r3 = r3.unwrap_or_else(|_| (Vec::new(), None, err_diag("mx_lookup")));
    let r4 = r4.unwrap_or_else(|_| (Vec::new(), err_diag("jmap_wellknown")));

    let resolved_domain = r3.1.clone();

    // Signal port probe if IMAP found
    let has_imap =
        r1.0.iter()
            .chain(r2.0.iter())
            .chain(r3.0.iter())
            .any(|opt| matches!(opt.protocol, types::Protocol::Imap { .. }));
    if has_imap {
        imap_found.notify_one();
    }

    ((r1, r2, r3, r4), resolved_domain)
}

async fn run_probe_stage(
    domain: &str,
    imap_found: &Arc<Notify>,
    _stages: &(StageResult, StageResult, MxResult, StageResult),
) -> StageResult {
    let d = domain.to_string();
    let notify = Arc::clone(imap_found);
    let handle = tokio::spawn(async move {
        let start = Instant::now();
        let cancelled = tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(3)) => false,
            () = notify.notified() => true,
        };
        let results = probe::probe_ports(&d, cancelled).await;
        let outcome = if cancelled {
            StageOutcome::Skipped
        } else if results.is_empty() {
            StageOutcome::NotFound
        } else {
            StageOutcome::Found {
                count: results.len(),
            }
        };
        (
            results,
            StageDiagnostic {
                stage: "port_probe",
                duration_ms: elapsed_ms(start),
                outcome,
            },
        )
    });
    handle
        .await
        .unwrap_or_else(|_| (Vec::new(), err_diag("port_probe")))
}

fn unpack_stages(
    stages: (StageResult, StageResult, MxResult, StageResult),
) -> (Vec<Vec<ProtocolOption>>, Vec<StageDiagnostic>) {
    let (r1, r2, r3, r4) = stages;
    let results = vec![r1.0, r2.0, r3.0, r4.0];
    let diagnostics = vec![r1.1, r2.1, r3.2, r4.1];
    (results, diagnostics)
}

fn err_diag(stage: &'static str) -> StageDiagnostic {
    StageDiagnostic {
        stage,
        duration_ms: 0,
        outcome: StageOutcome::Error {
            message: "task panicked".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_domain_accepts_normal_email() {
        assert_eq!(
            extract_domain("user@example.com").as_deref(),
            Ok("example.com")
        );
        assert_eq!(
            extract_domain("first.last@mail.corp.example.com").as_deref(),
            Ok("mail.corp.example.com")
        );
    }

    #[test]
    fn extract_domain_requires_at_sign_and_dot() {
        assert!(extract_domain("noatsign").is_err());
        assert!(extract_domain("user@localhost").is_err());
        assert!(extract_domain("user@").is_err());
    }

    #[test]
    fn extract_domain_rejects_loopback_and_private_ipv4() {
        for bad in [
            "user@127.0.0.1",
            "user@10.0.0.5",
            "user@192.168.1.1",
            "user@172.16.0.1",
            "user@169.254.0.1",  // link-local
            "user@224.0.0.1",    // multicast
            "user@255.255.255.255", // broadcast
            "user@0.0.0.0",      // unspecified
            "user@192.0.2.1",    // documentation
            "user@100.64.0.1",   // CGNAT
        ] {
            assert!(
                extract_domain(bad).is_err(),
                "expected {bad} to be rejected"
            );
        }
    }

    #[test]
    fn extract_domain_rejects_numeric_shorthand_hosts() {
        // `Ipv4Addr::from_str` is strict, but `getaddrinfo` will expand
        // `127.1` into `127.0.0.1`. Reject anything that's all digits + dots
        // so the shorthand can't sneak the IP past the dotted-quad check.
        // (Rejecting public IPv4 literals like 8.8.8.8 too is a fine
        // collateral - email-by-IP is a vanishing use case.)
        for bad in ["user@127.1", "user@8.8.8.8", "user@192.168.1"] {
            assert!(
                extract_domain(bad).is_err(),
                "expected {bad} to be rejected"
            );
        }
    }

    #[test]
    fn extract_domain_rejects_url_syntax() {
        for bad in [
            "user@example.com:8080",       // port
            "user@example.com/path",       // path
            "user@example.com?query=1",    // query
            "user@example.com#frag",       // fragment
            "user@evil@example.com",       // userinfo (extra @)
            "user@[::1]",                  // bracketed IPv6
            "user@[2001:db8::1]",          // bracketed public-looking IPv6
            "user@example .com",           // whitespace
            "user@example\tcom",           // tab
            "user@example.com\n",          // trailing newline / control
        ] {
            assert!(
                extract_domain(bad).is_err(),
                "expected {bad} to be rejected"
            );
        }
    }

    #[test]
    fn extract_domain_rejects_bare_ipv6() {
        // Bare IPv6 contains `:` so it falls out at the bad-character filter
        // even before reaching the IP check.
        assert!(extract_domain("user@::1").is_err());
        assert!(extract_domain("user@2001:db8::1").is_err());
    }
}
