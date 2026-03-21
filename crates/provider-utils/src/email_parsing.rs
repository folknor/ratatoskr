/// Returns `true` if the MIME type represents AMP email content (`text/x-amp-html`).
///
/// AMP emails contain tracking-heavy interactive content that should be blocked.
/// Prefer `text/html` (or `text/plain` as fallback) over AMP parts.
pub fn is_amp_content_type(mime_type: &str) -> bool {
    mime_type.eq_ignore_ascii_case("text/x-amp-html")
}

pub fn parse_single_address_header(raw: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(raw) = raw else {
        return (None, None);
    };

    log::debug!("Parsing address header: {:?}", raw);

    if let Some(angle_start) = raw.rfind('<')
        && let Some(angle_end) = raw[angle_start..].find('>')
    {
        let address = raw[angle_start + 1..angle_start + angle_end].trim();
        let name_part = raw[..angle_start].trim().trim_matches('"').trim();
        let name = if name_part.is_empty() || name_part == address {
            None
        } else {
            Some(name_part.to_string())
        };
        return (name, Some(address.to_string()));
    }

    (None, Some(raw.trim().to_string()))
}

pub fn format_name_addr(name: Option<&str>, email: &str) -> String {
    match name {
        Some(name) if !name.is_empty() => format!("{name} <{email}>"),
        _ => email.to_string(),
    }
}

pub fn format_address_list<I>(entries: I) -> Option<String>
where
    I: IntoIterator<Item = (Option<String>, String)>,
{
    let parts: Vec<String> = entries
        .into_iter()
        .map(|(name, email)| format_name_addr(name.as_deref(), &email))
        .collect();

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        format_address_list, format_name_addr, is_amp_content_type, parse_single_address_header,
    };

    #[test]
    fn parses_name_addr_header() {
        let (name, email) = parse_single_address_header(Some("\"Test User\" <test@example.com>"));
        assert_eq!(name.as_deref(), Some("Test User"));
        assert_eq!(email.as_deref(), Some("test@example.com"));
    }

    #[test]
    fn parses_bare_email_header() {
        let (name, email) = parse_single_address_header(Some("test@example.com"));
        assert_eq!(name, None);
        assert_eq!(email.as_deref(), Some("test@example.com"));
    }

    #[test]
    fn formats_name_addr() {
        assert_eq!(
            format_name_addr(Some("Test User"), "test@example.com"),
            "Test User <test@example.com>"
        );
        assert_eq!(
            format_name_addr(None, "test@example.com"),
            "test@example.com"
        );
    }

    #[test]
    fn detects_amp_content_type() {
        assert!(is_amp_content_type("text/x-amp-html"));
        assert!(is_amp_content_type("Text/X-Amp-Html"));
        assert!(is_amp_content_type("TEXT/X-AMP-HTML"));
        assert!(!is_amp_content_type("text/html"));
        assert!(!is_amp_content_type("text/plain"));
        assert!(!is_amp_content_type("application/x-amp-html"));
    }

    #[test]
    fn formats_address_list() {
        let value = format_address_list(vec![
            (Some("One".to_string()), "one@example.com".to_string()),
            (None, "two@example.com".to_string()),
        ]);
        assert_eq!(
            value.as_deref(),
            Some("One <one@example.com>, two@example.com")
        );
    }
}
