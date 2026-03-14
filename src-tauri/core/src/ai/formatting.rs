// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Input data for formatting a message for AI context.
pub struct AiMessageInput {
    pub from_name: Option<String>,
    pub from_email: String,
    pub date: Option<String>,
    pub body_text: Option<String>,
    pub snippet: Option<String>,
}

/// Input data for formatting a search result for AI context.
pub struct AiSearchResult {
    pub message_id: String,
    pub from_email: String,
    pub date: Option<String>,
    pub subject: Option<String>,
    pub preview: Option<String>,
}

// ---------------------------------------------------------------------------
// Message formatting
// ---------------------------------------------------------------------------

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
    let from = match &msg.from_name {
        Some(name) if !name.is_empty() => format!("{name} <{}>", msg.from_email),
        _ => msg.from_email.clone(),
    };

    let date = msg.date.as_deref().unwrap_or("Unknown");

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
    let date = result.date.as_deref().unwrap_or("Unknown");
    let subject = result.subject.as_deref().unwrap_or("(no subject)");
    let preview = result.preview.as_deref().unwrap_or_default();

    format!(
        "[Message ID: {}]\nFrom: {}\nDate: {date}\nSubject: {subject}\nPreview: {preview}",
        result.message_id, result.from_email
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
        from_email: &str,
        date: Option<&str>,
        body: Option<&str>,
        snippet: Option<&str>,
    ) -> AiMessageInput {
        AiMessageInput {
            from_name: from_name.map(String::from),
            from_email: from_email.to_string(),
            date: date.map(String::from),
            body_text: body.map(String::from),
            snippet: snippet.map(String::from),
        }
    }

    #[test]
    fn format_single_message_with_name() {
        let msgs = [make_msg(
            Some("Alice"),
            "alice@example.com",
            Some("Jan 1, 2025"),
            Some("Hello world"),
            None,
        )];
        let result = format_messages_for_ai(&msgs, 10000);
        assert!(result.contains("From: Alice <alice@example.com>"));
        assert!(result.contains("Date: Jan 1, 2025"));
        assert!(result.contains("Hello world"));
        assert!(result.contains("<email_content>"));
        assert!(result.contains("</email_content>"));
    }

    #[test]
    fn format_single_message_without_name() {
        let msgs = [make_msg(
            None,
            "bob@example.com",
            Some("Feb 2, 2025"),
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
            "test@example.com",
            None,
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
            "test@example.com",
            None,
            None,
            Some("Snippet fallback"),
        )];
        let result = format_messages_for_ai(&msgs, 10000);
        assert!(result.contains("Snippet fallback"));
    }

    #[test]
    fn format_multiple_messages_joined() {
        let msgs = [
            make_msg(None, "a@a.com", Some("Jan 1"), Some("First"), None),
            make_msg(None, "b@b.com", Some("Jan 2"), Some("Second"), None),
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
            "alice@example.com",
            Some("Jan 1"),
            Some(&"x".repeat(500)),
            None,
        )];
        let result = format_messages_for_ai(&msgs, 100);
        assert!(result.len() <= 100);
    }

    #[test]
    fn format_truncates_long_body() {
        let long_body = "y".repeat(2000);
        let msgs = [make_msg(None, "a@a.com", None, Some(&long_body), None)];
        let result = format_messages_for_ai(&msgs, 50000);
        // Body should be truncated to MAX_BODY_CHARS
        assert!(!result.contains(&"y".repeat(2000)));
        assert!(result.contains(&"y".repeat(1000)));
    }

    #[test]
    fn format_search_result_basic() {
        let results = [AiSearchResult {
            message_id: "msg-123".to_string(),
            from_email: "sender@example.com".to_string(),
            date: Some("Mar 1, 2025".to_string()),
            subject: Some("Important topic".to_string()),
            preview: Some("Preview of the message".to_string()),
        }];
        let result = format_search_results_for_ai(&results, 10000);
        assert!(result.contains("[Message ID: msg-123]"));
        assert!(result.contains("From: sender@example.com"));
        assert!(result.contains("Date: Mar 1, 2025"));
        assert!(result.contains("Subject: Important topic"));
        assert!(result.contains("Preview: Preview of the message"));
    }

    #[test]
    fn format_search_result_defaults() {
        let results = [AiSearchResult {
            message_id: "msg-456".to_string(),
            from_email: "x@x.com".to_string(),
            date: None,
            subject: None,
            preview: None,
        }];
        let result = format_search_results_for_ai(&results, 10000);
        assert!(result.contains("Date: Unknown"));
        assert!(result.contains("Subject: (no subject)"));
    }

    #[test]
    fn format_search_results_truncated() {
        let results: Vec<AiSearchResult> = (0..50)
            .map(|i| AiSearchResult {
                message_id: format!("msg-{i}"),
                from_email: format!("user{i}@example.com"),
                date: Some("Jan 1".to_string()),
                subject: Some("Subject".to_string()),
                preview: Some("Preview text here".to_string()),
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
                from_email: "a@a.com".to_string(),
                date: None,
                subject: None,
                preview: None,
            },
            AiSearchResult {
                message_id: "b".to_string(),
                from_email: "b@b.com".to_string(),
                date: None,
                subject: None,
                preview: None,
            },
        ];
        let result = format_search_results_for_ai(&results, 10000);
        assert!(result.contains("---"));
    }

    #[test]
    fn format_empty_from_name_uses_email_only() {
        let msgs = [make_msg(
            Some(""),
            "test@test.com",
            None,
            Some("body"),
            None,
        )];
        let result = format_messages_for_ai(&msgs, 10000);
        assert!(result.contains("From: test@test.com"));
        assert!(!result.contains("<>"));
    }
}
