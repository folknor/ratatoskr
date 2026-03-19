// ---------------------------------------------------------------------------
// Message formatting
// ---------------------------------------------------------------------------

use chrono::{TimeZone, Utc};

use crate::types::{AiMessageInput, AiSearchResult};

/// Maximum body text length per message before truncation.
const MAX_BODY_CHARS: usize = 1000;

/// Format a list of messages for AI context.
///
/// Each message is wrapped in `<email_content>` tags with From, Date, and body.
/// Messages are joined with `---` separators. The total output is truncated
/// to `max_chars`.
pub fn format_messages_for_ai(messages: &[AiMessageInput], max_chars: usize) -> String {
    let formatted: Vec<String> = messages.iter().map(format_single_message).collect();
    let joined = formatted.join("\n---\n");
    truncate_to_char_boundary(&joined, max_chars)
}

fn format_single_message(msg: &AiMessageInput) -> String {
    let from = match (&msg.from_name, &msg.from_address) {
        (Some(name), Some(addr)) if !name.is_empty() => format!("{name} <{addr}>"),
        (_, Some(addr)) => addr.clone(),
        (Some(name), None) if !name.is_empty() => name.clone(),
        _ => "Unknown".to_string(),
    };

    let date = Utc
        .timestamp_opt(msg.date, 0)
        .single()
        .map(|dt| dt.format("%b %d, %Y").to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    let body = msg
        .body_text
        .as_deref()
        .filter(|b| !b.trim().is_empty())
        .or(msg.snippet.as_deref())
        .unwrap_or_default()
        .trim();

    let body_truncated = truncate_to_char_boundary(body, MAX_BODY_CHARS);

    format!("<email_content>\nFrom: {from}\nDate: {date}\n{body_truncated}\n</email_content>")
}

// ---------------------------------------------------------------------------
// Search result formatting
// ---------------------------------------------------------------------------

/// Format search results for ask-inbox AI context.
///
/// Each result includes Message ID, From, Date, Subject, and Preview.
/// Results are joined with `---` separators. The total output is truncated
/// to `max_chars`.
pub fn format_search_results_for_ai(results: &[AiSearchResult], max_chars: usize) -> String {
    let formatted: Vec<String> = results.iter().map(format_single_search_result).collect();
    let joined = formatted.join("\n---\n");
    truncate_to_char_boundary(&joined, max_chars)
}

fn format_single_search_result(result: &AiSearchResult) -> String {
    let date = Utc
        .timestamp_opt(result.date, 0)
        .single()
        .map(|dt| dt.format("%b %d, %Y").to_string())
        .unwrap_or_else(|| "Unknown".to_string());
    let subject = result.subject.as_deref().unwrap_or("(no subject)");
    let from = match (&result.from_name, &result.from_address) {
        (Some(name), Some(addr)) if !name.is_empty() => format!("{name} <{addr}>"),
        (_, Some(addr)) => addr.clone(),
        (Some(name), None) if !name.is_empty() => name.clone(),
        _ => "Unknown".to_string(),
    };
    let snippet = result.snippet.as_deref().unwrap_or_default();

    format!(
        "[Message ID: {}]\nFrom: {from}\nDate: {date}\nSubject: {subject}\nPreview: {snippet}",
        result.message_id
    )
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Truncate a string to at most `max_chars`, respecting char boundaries.
fn truncate_to_char_boundary(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        return s.to_string();
    }
    let mut end = max_chars;
    while !s.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    s[..end].to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_msg(
        from_name: Option<&str>,
        from_address: Option<&str>,
        date: i64,
        body: Option<&str>,
        snippet: Option<&str>,
    ) -> AiMessageInput {
        AiMessageInput {
            from_name: from_name.map(String::from),
            from_address: from_address.map(String::from),
            date,
            body_text: body.map(String::from),
            snippet: snippet.map(String::from),
            subject: None,
        }
    }

    #[test]
    fn format_single_message_with_name() {
        let msgs = [make_msg(
            Some("Alice"),
            Some("alice@example.com"),
            1735689600, // Jan 1, 2025
            Some("Hello world"),
            None,
        )];
        let result = format_messages_for_ai(&msgs, 10000);
        assert!(result.contains("From: Alice <alice@example.com>"));
        assert!(result.contains("Hello world"));
        assert!(result.contains("<email_content>"));
        assert!(result.contains("</email_content>"));
    }

    #[test]
    fn format_single_message_without_name() {
        let msgs = [make_msg(
            None,
            Some("bob@example.com"),
            1738454400, // Feb 2, 2025
            Some("Body text"),
            None,
        )];
        let result = format_messages_for_ai(&msgs, 10000);
        assert!(result.contains("From: bob@example.com"));
        // No angle brackets around the email (no name to wrap)
        assert!(!result.contains("From: bob@example.com>"));
    }

    #[test]
    fn format_uses_snippet_when_body_empty() {
        let msgs = [make_msg(
            None,
            Some("test@example.com"),
            0,
            Some("   "),
            Some("Snippet text here"),
        )];
        let result = format_messages_for_ai(&msgs, 10000);
        assert!(result.contains("Snippet text here"));
    }

    #[test]
    fn format_uses_snippet_when_body_none() {
        let msgs = [make_msg(
            None,
            Some("test@example.com"),
            0,
            None,
            Some("Snippet fallback"),
        )];
        let result = format_messages_for_ai(&msgs, 10000);
        assert!(result.contains("Snippet fallback"));
    }

    #[test]
    fn format_multiple_messages_joined() {
        let msgs = [
            make_msg(None, Some("a@a.com"), 1735689600, Some("First"), None),
            make_msg(None, Some("b@b.com"), 1735776000, Some("Second"), None),
        ];
        let result = format_messages_for_ai(&msgs, 10000);
        assert!(result.contains("---"));
        assert!(result.contains("First"));
        assert!(result.contains("Second"));
    }

    #[test]
    fn format_truncates_total_output() {
        let msgs = [make_msg(
            Some("Alice"),
            Some("alice@example.com"),
            1735689600,
            Some(&"x".repeat(500)),
            None,
        )];
        let result = format_messages_for_ai(&msgs, 100);
        assert!(result.len() <= 100);
    }

    #[test]
    fn format_truncates_long_body() {
        let long_body = "y".repeat(2000);
        let msgs = [make_msg(None, Some("a@a.com"), 0, Some(&long_body), None)];
        let result = format_messages_for_ai(&msgs, 50000);
        // Body should be truncated to MAX_BODY_CHARS
        assert!(!result.contains(&"y".repeat(2000)));
        assert!(result.contains(&"y".repeat(1000)));
    }

    #[test]
    fn format_search_result_basic() {
        let results = [AiSearchResult {
            message_id: "msg-123".to_string(),
            from_name: Some("Sender".to_string()),
            from_address: Some("sender@example.com".to_string()),
            date: 1740787200, // Mar 1, 2025
            subject: Some("Important topic".to_string()),
            snippet: Some("Preview of the message".to_string()),
        }];
        let result = format_search_results_for_ai(&results, 10000);
        assert!(result.contains("[Message ID: msg-123]"));
        assert!(result.contains("From: Sender <sender@example.com>"));
        assert!(result.contains("Subject: Important topic"));
        assert!(result.contains("Preview: Preview of the message"));
    }

    #[test]
    fn format_search_result_defaults() {
        let results = [AiSearchResult {
            message_id: "msg-456".to_string(),
            from_name: None,
            from_address: Some("x@x.com".to_string()),
            date: 0,
            subject: None,
            snippet: None,
        }];
        let result = format_search_results_for_ai(&results, 10000);
        assert!(result.contains("Subject: (no subject)"));
    }

    #[test]
    fn format_search_results_truncated() {
        let results: Vec<AiSearchResult> = (0..50)
            .map(|i| AiSearchResult {
                message_id: format!("msg-{i}"),
                from_name: None,
                from_address: Some(format!("user{i}@example.com")),
                date: 1735689600,
                subject: Some("Subject".to_string()),
                snippet: Some("Preview text here".to_string()),
            })
            .collect();
        let result = format_search_results_for_ai(&results, 500);
        assert!(result.len() <= 500);
    }

    #[test]
    fn format_search_results_multiple_joined() {
        let results = [
            AiSearchResult {
                message_id: "a".to_string(),
                from_name: None,
                from_address: Some("a@a.com".to_string()),
                date: 0,
                subject: None,
                snippet: None,
            },
            AiSearchResult {
                message_id: "b".to_string(),
                from_name: None,
                from_address: Some("b@b.com".to_string()),
                date: 0,
                subject: None,
                snippet: None,
            },
        ];
        let result = format_search_results_for_ai(&results, 10000);
        assert!(result.contains("---"));
    }

    #[test]
    fn format_empty_from_name_uses_email_only() {
        let msgs = [make_msg(
            Some(""),
            Some("test@test.com"),
            0,
            Some("body"),
            None,
        )];
        let result = format_messages_for_ai(&msgs, 10000);
        assert!(result.contains("From: test@test.com"));
        assert!(!result.contains("<>"));
    }
}
