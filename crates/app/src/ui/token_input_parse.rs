//! RFC 5322 address parsing for pasted text.
//!
//! Handles common formats:
//! - Bare email: `alice@corp.com`
//! - Name + angle-bracket: `Alice Smith <alice@corp.com>`
//! - Quoted name + angle-bracket: `"Alice Smith" <alice@corp.com>`
//! - Multiple addresses separated by commas, semicolons, or newlines

use super::token_input::is_plausible_email;

/// A parsed address from pasted text.
#[derive(Debug, Clone)]
pub struct ParsedAddress {
    pub email: String,
    pub display_name: Option<String>,
}

/// Parse pasted text into (display_name, email) pairs.
///
/// Returns all valid addresses found. Invalid fragments are silently dropped.
/// Handles common RFC 5322 mailbox formats:
/// - `alice@corp.com`
/// - `Alice Smith <alice@corp.com>`
/// - `"Alice Smith" <alice@corp.com>`
/// - Mixed formats separated by commas, semicolons, or newlines
pub fn parse_pasted_addresses(input: &str) -> Vec<ParsedAddress> {
    let mut results = Vec::new();

    for fragment in split_addresses(input) {
        let trimmed = fragment.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(parsed) = parse_single_address(trimmed) {
            results.push(parsed);
        }
    }

    results
}

/// Split input on commas, semicolons, and newlines, respecting quoted strings.
fn split_addresses(input: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let mut in_angle = false;

    for (i, ch) in input.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            '<' if !in_quotes => in_angle = true,
            '>' if !in_quotes => in_angle = false,
            ',' | ';' | '\n' if !in_quotes && !in_angle => {
                parts.push(&input[start..i]);
                start = i + ch.len_utf8();
            }
            _ => {}
        }
    }
    // Push the remaining part
    if start <= input.len() {
        parts.push(&input[start..]);
    }
    parts
}

/// Parse a single address fragment into a `ParsedAddress`.
///
/// Handles:
/// - `alice@corp.com` (bare email)
/// - `Alice Smith <alice@corp.com>` (name + angle-bracket)
/// - `"Alice Smith" <alice@corp.com>` (quoted name + angle-bracket)
/// - `alice@corp.com <alice@corp.com>` (email as name, extract from angle)
fn parse_single_address(input: &str) -> Option<ParsedAddress> {
    let trimmed = input.trim();

    // Try angle-bracket format: `Name <email>` or `"Name" <email>`
    if let Some(angle_start) = trimmed.rfind('<') {
        if let Some(angle_end) = trimmed.rfind('>') {
            if angle_end > angle_start {
                let email = trimmed[angle_start + 1..angle_end].trim();
                if !is_plausible_email(email) {
                    return None;
                }

                let name_part = trimmed[..angle_start].trim();
                let display_name = extract_display_name(name_part);

                return Some(ParsedAddress {
                    email: email.to_lowercase(),
                    display_name,
                });
            }
        }
    }

    // Try bare email
    if is_plausible_email(trimmed) {
        return Some(ParsedAddress {
            email: trimmed.to_lowercase(),
            display_name: None,
        });
    }

    None
}

/// Extract display name from the name part of an address.
///
/// Strips surrounding quotes if present: `"Alice Smith"` -> `Alice Smith`.
/// Returns `None` if the name is empty or looks like an email itself.
fn extract_display_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Strip surrounding quotes
    let unquoted = if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
        trimmed[1..trimmed.len() - 1].trim()
    } else {
        trimmed
    };

    if unquoted.is_empty() {
        return None;
    }

    // If the "name" is just the email address repeated, skip it
    if is_plausible_email(unquoted) {
        return None;
    }

    Some(unquoted.to_string())
}

/// Deduplicate parsed addresses by email (case-insensitive), keeping first occurrence.
pub fn dedup_parsed(addresses: &mut Vec<ParsedAddress>) {
    let mut seen = std::collections::HashSet::new();
    addresses.retain(|addr| seen.insert(addr.email.to_lowercase()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_email() {
        let results = parse_pasted_addresses("alice@corp.com");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].email, "alice@corp.com");
        assert!(results[0].display_name.is_none());
    }

    #[test]
    fn name_angle_bracket() {
        let results = parse_pasted_addresses("Alice Smith <alice@corp.com>");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].email, "alice@corp.com");
        assert_eq!(results[0].display_name.as_deref(), Some("Alice Smith"));
    }

    #[test]
    fn quoted_name() {
        let results = parse_pasted_addresses("\"Alice Smith\" <alice@corp.com>");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].email, "alice@corp.com");
        assert_eq!(results[0].display_name.as_deref(), Some("Alice Smith"));
    }

    #[test]
    fn multiple_mixed() {
        let input = "Alice <alice@corp.com>, bob@example.com; \"Charlie D\" <charlie@d.com>";
        let results = parse_pasted_addresses(input);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].email, "alice@corp.com");
        assert_eq!(results[1].email, "bob@example.com");
        assert_eq!(results[2].email, "charlie@d.com");
    }

    #[test]
    fn newline_separated() {
        let input = "alice@corp.com\nbob@corp.com\ncharlie@corp.com";
        let results = parse_pasted_addresses(input);
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn invalid_dropped() {
        let input = "alice@corp.com, not-an-email, bob@corp.com";
        let results = parse_pasted_addresses(input);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn dedup_within_paste() {
        let mut addresses = parse_pasted_addresses("alice@corp.com, Alice <ALICE@corp.com>");
        dedup_parsed(&mut addresses);
        assert_eq!(addresses.len(), 1);
    }
}
