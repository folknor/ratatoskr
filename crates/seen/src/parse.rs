use mail_parser::MessageParser;

use super::types::{AddressObservation, Direction, ObservationParams};

/// Parse a formatted address list string ("Name <email>, Name2 <email2>")
/// into individual (display_name, email) pairs using mail-parser's RFC 5322
/// compliant parser.
pub fn parse_address_list(raw: &str) -> Vec<(Option<String>, String)> {
    if raw.trim().is_empty() {
        return Vec::new();
    }

    // Wrap in a synthetic RFC 5322 message so mail-parser can parse the addresses.
    let synthetic = format!("To: {raw}\r\n\r\n");
    let parser = MessageParser::default();
    let Some(message) = parser.parse(synthetic.as_bytes()) else {
        return fallback_parse(raw);
    };

    let Some(to) = message.to() else {
        log::debug!("mail-parser could not extract addresses, using fallback parser");
        return fallback_parse(raw);
    };

    let mut results = Vec::new();
    for addr in to.iter() {
        if let Some(email) = addr.address.as_ref()
            && email.contains('@')
        {
            let name = addr.name.as_ref().map(ToString::to_string);
            results.push((name, email.to_string()));
        }
    }

    if results.is_empty() {
        return fallback_parse(raw);
    }

    results
}

/// Simple fallback: try to extract "Name <email>" or bare "email" from each
/// comma-separated segment. Used when mail-parser can't parse the input.
fn fallback_parse(raw: &str) -> Vec<(Option<String>, String)> {
    let mut results = Vec::new();
    for part in raw.split(',') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(angle_start) = trimmed.rfind('<') {
            if let Some(angle_end) = trimmed[angle_start..].find('>') {
                let email = trimmed[angle_start + 1..angle_start + angle_end].trim();
                if email.contains('@') {
                    let name_part = trimmed[..angle_start].trim().trim_matches('"').trim();
                    let name = if name_part.is_empty() || name_part == email {
                        None
                    } else {
                        Some(name_part.to_string())
                    };
                    results.push((name, email.to_string()));
                }
            }
        } else if trimmed.contains('@') {
            results.push((None, trimmed.to_string()));
        }
    }
    results
}

/// Extract address observations from a single message's headers.
///
/// Direction detection: if `from_address` matches any of `self_emails`,
/// the message is outbound (SentTo/SentCc). Otherwise it's inbound
/// (ReceivedFrom/ReceivedCc). Self-references are filtered out.
pub fn extract_observations(params: &ObservationParams<'_>) -> Vec<AddressObservation> {
    let mut observations = Vec::new();

    let from_email_lower = params
        .from_address
        .map(str::to_lowercase)
        .unwrap_or_default();

    let is_sent = params
        .self_emails
        .iter()
        .any(|s| s.eq_ignore_ascii_case(&from_email_lower));

    if is_sent {
        // Outbound: collect To as SentTo, Cc/Bcc as SentCc
        collect_from_field(
            params.to_addresses,
            Direction::SentTo,
            params.date_ms,
            params.self_emails,
            &mut observations,
        );
        collect_from_field(
            params.cc_addresses,
            Direction::SentCc,
            params.date_ms,
            params.self_emails,
            &mut observations,
        );
        collect_from_field(
            params.bcc_addresses,
            Direction::SentCc,
            params.date_ms,
            params.self_emails,
            &mut observations,
        );
    } else {
        // Inbound: From is ReceivedFrom, Cc is ReceivedCc
        if !from_email_lower.is_empty() && from_email_lower.contains('@') {
            observations.push(AddressObservation {
                email: from_email_lower,
                display_name: params.from_name.map(ToString::to_string),
                direction: Direction::ReceivedFrom,
                date_ms: params.date_ms,
            });
        }
        collect_from_field(
            params.cc_addresses,
            Direction::ReceivedCc,
            params.date_ms,
            params.self_emails,
            &mut observations,
        );
        // Also collect To addresses on received messages as ReceivedCc
        // (other recipients we were alongside)
        collect_from_field(
            params.to_addresses,
            Direction::ReceivedCc,
            params.date_ms,
            params.self_emails,
            &mut observations,
        );
    }

    observations
}

fn collect_from_field(
    field: Option<&str>,
    direction: Direction,
    date_ms: i64,
    self_emails: &[String],
    out: &mut Vec<AddressObservation>,
) {
    let Some(raw) = field else { return };
    for (name, email) in parse_address_list(raw) {
        let lower = email.to_lowercase();
        // Skip self-references
        if self_emails.iter().any(|s| s.eq_ignore_ascii_case(&lower)) {
            continue;
        }
        if !lower.contains('@') {
            continue;
        }
        out.push(AddressObservation {
            email: lower,
            display_name: name,
            direction,
            date_ms,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_address_list() {
        let result = parse_address_list("Alice <alice@example.com>, bob@example.com");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0.as_deref(), Some("Alice"));
        assert_eq!(result[0].1, "alice@example.com");
        assert_eq!(result[1].0, None);
        assert_eq!(result[1].1, "bob@example.com");
    }

    #[test]
    fn parse_quoted_name_with_comma() {
        let result = parse_address_list("\"Doe, Jane\" <jane@example.com>");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, "jane@example.com");
    }

    #[test]
    fn parse_empty_string() {
        assert!(parse_address_list("").is_empty());
        assert!(parse_address_list("   ").is_empty());
    }

    #[test]
    fn extract_sent_message_observations() {
        let self_emails = vec!["me@example.com".to_string()];
        let params = ObservationParams {
            self_emails: &self_emails,
            from_address: Some("me@example.com"),
            from_name: Some("Me"),
            to_addresses: Some("alice@example.com, bob@example.com"),
            cc_addresses: Some("carol@example.com"),
            bcc_addresses: None,
            date_ms: 1_700_000_000_000,
        };
        let obs = extract_observations(&params);
        assert_eq!(obs.len(), 3);
        assert_eq!(obs[0].direction, Direction::SentTo);
        assert_eq!(obs[0].email, "alice@example.com");
        assert_eq!(obs[1].direction, Direction::SentTo);
        assert_eq!(obs[2].direction, Direction::SentCc);
    }

    #[test]
    fn extract_received_message_observations() {
        let self_emails = vec!["me@example.com".to_string()];
        let params = ObservationParams {
            self_emails: &self_emails,
            from_address: Some("alice@example.com"),
            from_name: Some("Alice"),
            to_addresses: Some("me@example.com"),
            cc_addresses: Some("bob@example.com"),
            bcc_addresses: None,
            date_ms: 1_700_000_000_000,
        };
        let obs = extract_observations(&params);
        // alice (ReceivedFrom) + bob (ReceivedCc), self filtered out from To
        assert_eq!(obs.len(), 2);
        assert_eq!(obs[0].direction, Direction::ReceivedFrom);
        assert_eq!(obs[0].email, "alice@example.com");
        assert_eq!(obs[1].direction, Direction::ReceivedCc);
    }

    #[test]
    fn alias_detected_as_sent() {
        let self_emails = vec![
            "me@example.com".to_string(),
            "alias@example.com".to_string(),
        ];
        let params = ObservationParams {
            self_emails: &self_emails,
            from_address: Some("alias@example.com"),
            from_name: None,
            to_addresses: Some("recipient@example.com"),
            cc_addresses: None,
            bcc_addresses: None,
            date_ms: 1_700_000_000_000,
        };
        let obs = extract_observations(&params);
        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].direction, Direction::SentTo);
    }
}
