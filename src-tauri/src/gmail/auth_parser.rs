use serde::Serialize;

use super::types::GmailHeader;

/// Individual authentication mechanism result.
#[derive(Debug, Clone, Serialize)]
pub struct AuthVerdict {
    pub result: String,
    pub detail: Option<String>,
}

/// Aggregate authentication result (SPF + DKIM + DMARC).
#[derive(Debug, Clone, Serialize)]
pub struct AuthResult {
    pub spf: AuthVerdict,
    pub dkim: AuthVerdict,
    pub dmarc: AuthVerdict,
    pub aggregate: String,
}

/// Parse email authentication results from message headers.
///
/// Tries these headers in order:
/// 1. `Authentication-Results`
/// 2. `ARC-Authentication-Results`
/// 3. `Received-SPF` (SPF-only fallback)
///
/// Returns `None` if no authentication headers are found.
pub fn parse_authentication_results(headers: &[GmailHeader]) -> Option<AuthResult> {
    let auth_header = find_header(headers, "authentication-results");
    let arc_header = auth_header.or_else(|| find_header(headers, "arc-authentication-results"));
    let received_spf = find_header(headers, "received-spf");

    if arc_header.is_none() && received_spf.is_none() {
        return None;
    }

    let mut spf = unknown_verdict();
    let mut dkim = unknown_verdict();
    let mut dmarc = unknown_verdict();

    if let Some(header_value) = arc_header {
        let normalized = normalize_header(header_value);

        if let Some(v) = parse_verdict(&normalized, "spf") {
            spf = v;
        }

        dkim = parse_dkim_verdicts(&normalized);

        if let Some(v) = parse_verdict(&normalized, "dmarc") {
            dmarc = v;
        }
    } else if let Some(header_value) = received_spf
        && let Some(v) = parse_received_spf(header_value)
    {
        spf = v;
    }

    let aggregate = compute_aggregate(&spf, &dkim, &dmarc);

    Some(AuthResult {
        spf,
        dkim,
        dmarc,
        aggregate,
    })
}

fn find_header<'a>(headers: &'a [GmailHeader], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

fn normalize_header(value: &str) -> String {
    // Collapse folded headers (CRLF + whitespace) into a single space
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' || c == '\n' {
            // Skip whitespace after line breaks
            while chars
                .peek()
                .is_some_and(|&ch| ch == ' ' || ch == '\t' || ch == '\r' || ch == '\n')
            {
                chars.next();
            }
            result.push(' ');
        } else {
            result.push(c);
        }
    }
    result
}

/// Parse a single `mechanism=result (detail)` pattern from the header.
fn parse_verdict(header_value: &str, mechanism: &str) -> Option<AuthVerdict> {
    let lower = header_value.to_lowercase();
    let mech_lower = mechanism.to_lowercase();

    // Find "mechanism=result"
    let pattern = format!("{mech_lower}=");
    let idx = lower.find(&pattern)?;
    let after = &header_value[idx + pattern.len()..];

    // Extract result word
    let result_word: String = after.chars().take_while(|c| c.is_alphanumeric()).collect();
    if result_word.is_empty() {
        return None;
    }

    // Extract optional parenthetical detail
    let after_result = &after[result_word.len()..].trim_start();
    let detail = if after_result.starts_with('(') {
        after_result
            .get(1..)
            .and_then(|s| s.find(')').map(|end| s[..end].trim().to_string()))
    } else {
        None
    };

    Some(AuthVerdict {
        result: result_word.to_lowercase(),
        detail,
    })
}

/// Parse multiple DKIM results — if any passes, use that one.
fn parse_dkim_verdicts(header_value: &str) -> AuthVerdict {
    let lower = header_value.to_lowercase();
    let mut verdicts = Vec::new();
    let mut search_from = 0;

    while let Some(idx) = lower[search_from..].find("dkim=") {
        let abs_idx = search_from + idx;
        let after = &header_value[abs_idx + 5..];
        let result_word: String = after.chars().take_while(|c| c.is_alphanumeric()).collect();

        if !result_word.is_empty() {
            let after_result = &after[result_word.len()..].trim_start();
            let detail = if after_result.starts_with('(') {
                after_result
                    .get(1..)
                    .and_then(|s| s.find(')').map(|end| s[..end].trim().to_string()))
            } else {
                None
            };
            verdicts.push(AuthVerdict {
                result: result_word.to_lowercase(),
                detail,
            });
        }

        search_from = abs_idx + 5;
    }

    if verdicts.is_empty() {
        return unknown_verdict();
    }

    // If any DKIM result passes, use it
    if let Some(pass) = verdicts.iter().find(|v| v.result == "pass") {
        return pass.clone();
    }

    // Otherwise use the first result
    verdicts.into_iter().next().unwrap_or_else(unknown_verdict)
}

/// Parse `Received-SPF` header as fallback (format: `result (detail) ...`).
fn parse_received_spf(header_value: &str) -> Option<AuthVerdict> {
    let normalized = normalize_header(header_value);
    let trimmed = normalized.trim();

    let result_word: String = trimmed
        .chars()
        .take_while(|c| c.is_alphanumeric())
        .collect();
    if result_word.is_empty() {
        return None;
    }

    let after = trimmed[result_word.len()..].trim_start();
    let detail = if after.starts_with('(') {
        after
            .get(1..)
            .and_then(|s| s.find(')').map(|end| s[..end].trim().to_string()))
    } else {
        None
    };

    Some(AuthVerdict {
        result: result_word.to_lowercase(),
        detail,
    })
}

fn unknown_verdict() -> AuthVerdict {
    AuthVerdict {
        result: "unknown".to_string(),
        detail: None,
    }
}

/// Compute the aggregate verdict from SPF, DKIM, and DMARC results.
fn compute_aggregate(spf: &AuthVerdict, dkim: &AuthVerdict, dmarc: &AuthVerdict) -> String {
    // DMARC pass → aggregate pass
    if dmarc.result == "pass" {
        return "pass".to_string();
    }

    // DMARC fail → aggregate fail
    if dmarc.result == "fail" {
        return "fail".to_string();
    }

    // Both SPF and DKIM fail → aggregate fail
    let spf_failed = spf.result == "fail" || spf.result == "hardfail";
    let dkim_failed = dkim.result == "fail" || dkim.result == "hardfail";
    if spf_failed && dkim_failed {
        return "fail".to_string();
    }

    // All unknown → aggregate unknown
    if spf.result == "unknown" && dkim.result == "unknown" && dmarc.result == "unknown" {
        return "unknown".to_string();
    }

    // Both SPF and DKIM pass (DMARC unknown) → aggregate pass
    if spf.result == "pass" && dkim.result == "pass" && dmarc.result == "unknown" {
        return "pass".to_string();
    }

    // Mixed results
    "warning".to_string()
}
