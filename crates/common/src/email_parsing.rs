/// Returns `true` if the MIME type represents AMP email content (`text/x-amp-html`).
///
/// AMP emails contain tracking-heavy interactive content that should be blocked.
/// Prefer `text/html` (or `text/plain` as fallback) over AMP parts.
pub fn is_amp_content_type(mime_type: &str) -> bool {
    mime_type.eq_ignore_ascii_case("text/x-amp-html")
}

/// Returns `true` if the MIME type represents an iMIP / iCalendar attachment.
///
/// Recognizes both `text/calendar` (RFC 5545 / RFC 6047) and the legacy
/// `application/ics` form some clients still emit. Used at message-insert
/// time to populate `messages.has_meeting_invite`.
pub fn is_calendar_content_type(mime_type: &str) -> bool {
    let lower = mime_type.trim().to_ascii_lowercase();
    lower.starts_with("text/calendar") || lower.starts_with("application/ics")
}

/// Result of inspecting a list of attachment MIME types for an iMIP payload.
#[derive(Debug, Clone, Copy, Default)]
pub struct CalendarAttachmentInfo {
    pub has_invite: bool,
    /// `Some(...)` only when the MIME type carries a `method=` parameter.
    pub method_index: Option<usize>,
}

/// Find the index of the first iMIP attachment in a slice of MIME-type
/// strings. The MIME type may include parameters (`text/calendar; method=REQUEST`).
/// Used by provider sync paths that already collect attachment metadata.
pub fn find_calendar_attachment(mime_types: &[&str]) -> Option<usize> {
    mime_types
        .iter()
        .position(|mt| is_calendar_content_type(mt))
}

/// Extract the `method=` parameter from a `text/calendar` MIME type string.
/// Returns the canonical uppercased token (e.g. `REQUEST`, `REPLY`, `CANCEL`)
/// or `None` if the parameter isn't present.
pub fn extract_imip_method(content_type: &str) -> Option<String> {
    let lower = content_type.to_ascii_lowercase();
    let idx = lower.find("method=")?;
    let after = &content_type[idx + "method=".len()..];
    let token = after
        .split(|c: char| c == ';' || c.is_whitespace())
        .next()?
        .trim_matches('"')
        .trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_ascii_uppercase())
    }
}

pub fn parse_single_address_header(raw: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(raw) = raw else {
        return (None, None);
    };

    log::debug!("Parsing address header: {raw:?}");

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
    fn detects_calendar_content_type() {
        use super::is_calendar_content_type;
        assert!(is_calendar_content_type("text/calendar"));
        assert!(is_calendar_content_type("Text/Calendar; charset=utf-8"));
        assert!(is_calendar_content_type("text/calendar; method=REQUEST"));
        assert!(is_calendar_content_type("application/ics"));
        assert!(!is_calendar_content_type("text/plain"));
        assert!(!is_calendar_content_type("application/octet-stream"));
    }

    #[test]
    fn extracts_imip_method() {
        use super::extract_imip_method;
        assert_eq!(
            extract_imip_method("text/calendar; method=REQUEST"),
            Some("REQUEST".to_string())
        );
        assert_eq!(
            extract_imip_method("text/calendar; charset=utf-8; method=reply"),
            Some("REPLY".to_string())
        );
        assert_eq!(
            extract_imip_method("text/calendar; method=\"CANCEL\""),
            Some("CANCEL".to_string())
        );
        assert_eq!(extract_imip_method("text/calendar"), None);
        assert_eq!(extract_imip_method("application/octet-stream"), None);
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
