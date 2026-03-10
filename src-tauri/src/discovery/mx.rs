use crate::discovery::autoconfig;
use crate::discovery::registry;
use crate::discovery::types::{DiscoverySource, ProtocolOption};
use hickory_resolver::TokioResolver;
use std::time::Duration;

/// Stage timeout for the entire MX lookup stage.
const MX_TIMEOUT: Duration = Duration::from_secs(5);

/// Stage 3: DNS MX lookup → extract base domain → re-check registry + autoconfig.
pub async fn lookup(domain: &str, email: &str) -> (Vec<ProtocolOption>, Option<String>) {
    match tokio::time::timeout(MX_TIMEOUT, lookup_inner(domain, email)).await {
        Ok(result) => result,
        Err(_) => {
            log::debug!("MX lookup for {domain} timed out after 5s");
            (Vec::new(), None)
        }
    }
}

async fn lookup_inner(domain: &str, email: &str) -> (Vec<ProtocolOption>, Option<String>) {
    let resolver = match TokioResolver::builder_tokio() {
        Ok(builder) => builder.build(),
        Err(e) => {
            log::debug!("MX resolver creation failed: {e}");
            return (Vec::new(), None);
        }
    };

    let mx_records = match resolver.mx_lookup(domain).await {
        Ok(records) => records,
        Err(e) => {
            log::debug!("MX lookup for {domain} failed: {e}");
            return (Vec::new(), None);
        }
    };

    // Sort by preference (lower = higher priority)
    let mut exchanges: Vec<String> = mx_records
        .iter()
        .map(|mx| mx.exchange().to_utf8().trim_end_matches('.').to_string())
        .collect();
    exchanges.sort();
    exchanges.dedup();

    for mx_host in &exchanges {
        // Progressive suffix stripping: try each suffix against the registry
        if let Some(result) = try_registry_match(mx_host) {
            return result;
        }

        // Try autoconfig on the MX base domain (eTLD+1 approximation)
        if let Some(base) = extract_base_domain(mx_host)
            && base != domain
        {
            let results = autoconfig::fetch(&base, email).await;
            if !results.is_empty() {
                let retagged = retag_results(results, mx_host);
                return (retagged, Some(base));
            }
        }
    }

    (Vec::new(), None)
}

fn try_registry_match(mx_host: &str) -> Option<(Vec<ProtocolOption>, Option<String>)> {
    let segments: Vec<&str> = mx_host.split('.').collect();
    for start in 0..segments.len().saturating_sub(1) {
        let candidate = segments[start..].join(".");
        let results = registry::lookup(&candidate);
        if !results.is_empty() {
            let retagged = retag_results(results, mx_host);
            return Some((retagged, Some(candidate)));
        }
    }
    None
}

fn retag_results(results: Vec<ProtocolOption>, mx_host: &str) -> Vec<ProtocolOption> {
    results
        .into_iter()
        .map(|mut opt| {
            opt.source = DiscoverySource::MxLookup {
                mx_domain: mx_host.to_string(),
            };
            opt
        })
        .collect()
}

/// Simple base domain extraction: take last 2 segments, with exceptions
/// for known multi-part TLDs.
fn extract_base_domain(hostname: &str) -> Option<String> {
    let segments: Vec<&str> = hostname.split('.').collect();
    if segments.len() < 2 {
        return None;
    }

    // Known multi-part TLDs where we need 3 segments
    let multi_part_tlds = [
        "co.uk", "co.jp", "com.au", "com.br", "co.in", "co.nz", "co.za",
        "com.mx", "com.ar", "com.sg",
    ];

    let last_two = format!(
        "{}.{}",
        segments[segments.len() - 2],
        segments[segments.len() - 1]
    );

    if multi_part_tlds.contains(&last_two.as_str()) && segments.len() >= 3 {
        Some(format!(
            "{}.{last_two}",
            segments[segments.len() - 3],
        ))
    } else {
        Some(last_two)
    }
}
