/// A parsed smart folder query with structured filter fields.
#[derive(Debug, Default, Clone)]
pub struct ParsedQuery {
    pub free_text: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub subject: Option<String>,
    pub has_attachment: Option<bool>,
    pub is_unread: Option<bool>,
    pub is_read: Option<bool>,
    pub is_starred: Option<bool>,
    pub is_snoozed: Option<bool>,
    pub is_pinned: Option<bool>,
    pub is_muted: Option<bool>,
    pub is_important: Option<bool>,
    pub before: Option<i64>,
    pub after: Option<i64>,
    pub label: Option<String>,
}

/// Parse a query string like `is:unread from:alice has:attachment` into a
/// structured `ParsedQuery`.
///
/// Supports quoted values: `from:"John Doe"`.
pub fn parse_query(input: &str) -> ParsedQuery {
    let mut result = ParsedQuery::default();
    let mut remaining = input.to_owned();

    // Collect all operator matches and their spans (in reverse order for removal).
    let spans = collect_operator_spans(input);

    // Apply each operator match.
    for span in &spans {
        apply_operator(&mut result, &span.operator, &span.value);
    }

    // Remove matched spans from input to get free text (process in reverse).
    for span in spans.iter().rev() {
        remaining = format!("{}{}", &remaining[..span.start], &remaining[span.end..]);
    }

    result.free_text = collapse_whitespace(&remaining);
    result
}

// ── Internal types ──────────────────────────────────────────

struct OperatorSpan {
    start: usize,
    end: usize,
    operator: String,
    value: String,
}

// ── Span collection ─────────────────────────────────────────

/// Walk the input and extract all `operator:value` or `operator:"quoted value"` spans.
fn collect_operator_spans(input: &str) -> Vec<OperatorSpan> {
    let operators = [
        "from", "to", "subject", "has", "is", "before", "after", "label",
    ];
    let mut spans = Vec::new();
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut pos = 0;

    while pos < len {
        if let Some(span) = try_parse_operator_at(input, pos, &operators) {
            pos = span.end;
            spans.push(span);
        } else {
            pos += 1;
        }
    }

    spans
}

/// Try to parse an operator at the given position.
/// Returns `None` if no operator starts here.
fn try_parse_operator_at(
    input: &str,
    pos: usize,
    operators: &[&str],
) -> Option<OperatorSpan> {
    // Operator must be at start of string or preceded by whitespace.
    if pos > 0 && !input.as_bytes()[pos - 1].is_ascii_whitespace() {
        return None;
    }

    for &op in operators {
        if let Some(span) = match_single_operator(input, pos, op) {
            return Some(span);
        }
    }
    None
}

/// Check if `op:value` starts at `pos` and extract the span.
fn match_single_operator(input: &str, pos: usize, op: &str) -> Option<OperatorSpan> {
    let rest = &input[pos..];
    let prefix = format!("{op}:");

    if !rest.to_ascii_lowercase().starts_with(&prefix) {
        return None;
    }

    let after_colon = pos + prefix.len();
    let (value, end) = extract_value(input, after_colon);

    if value.is_empty() {
        return None;
    }

    Some(OperatorSpan {
        start: pos,
        end,
        operator: op.to_owned(),
        value,
    })
}

/// Extract a value starting at `pos`: either a `"quoted string"` or a
/// contiguous non-whitespace token.
fn extract_value(input: &str, pos: usize) -> (String, usize) {
    let rest = &input[pos..];
    // Skip optional whitespace between colon and value.
    let trimmed = rest.trim_start();
    let skip = rest.len() - trimmed.len();
    let start = pos + skip;

    if trimmed.starts_with('"') {
        // Quoted value — find closing quote.
        if let Some(close) = trimmed[1..].find('"') {
            let value = trimmed[1..close + 1].to_owned();
            return (value, start + close + 2);
        }
    }

    // Unquoted — take until whitespace.
    let token_end = trimmed
        .find(char::is_whitespace)
        .unwrap_or(trimmed.len());
    let value = trimmed[..token_end].to_owned();
    (value, start + token_end)
}

// ── Operator application ────────────────────────────────────

fn apply_operator(result: &mut ParsedQuery, operator: &str, value: &str) {
    match operator {
        "from" => result.from = Some(value.to_owned()),
        "to" => result.to = Some(value.to_owned()),
        "subject" => result.subject = Some(value.to_owned()),
        "has" => apply_has_operator(result, value),
        "is" => apply_is_operator(result, value),
        "before" => result.before = parse_date_to_timestamp(value),
        "after" => result.after = parse_date_to_timestamp(value),
        "label" => result.label = Some(value.to_owned()),
        _ => {}
    }
}

fn apply_has_operator(result: &mut ParsedQuery, value: &str) {
    if value.eq_ignore_ascii_case("attachment") {
        result.has_attachment = Some(true);
    }
}

fn apply_is_operator(result: &mut ParsedQuery, value: &str) {
    match value.to_ascii_lowercase().as_str() {
        "unread" => result.is_unread = Some(true),
        "read" => result.is_read = Some(true),
        "starred" => result.is_starred = Some(true),
        "snoozed" => result.is_snoozed = Some(true),
        "pinned" => result.is_pinned = Some(true),
        "muted" => result.is_muted = Some(true),
        "important" => result.is_important = Some(true),
        _ => {}
    }
}

// ── Date parsing ────────────────────────────────────────────

/// Parse a date string like `2024/01/15` or `2024-01-15` to a Unix timestamp (seconds).
fn parse_date_to_timestamp(date_str: &str) -> Option<i64> {
    let normalized = date_str.replace('-', "/");
    let parts: Vec<&str> = normalized.split('/').collect();
    if parts.len() != 3 {
        return None;
    }

    let year: i32 = parts[0].parse().ok()?;
    let month: u32 = parts[1].parse().ok()?;
    let day: u32 = parts[2].parse().ok()?;

    let date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
    let datetime = date.and_hms_opt(0, 0, 0)?;
    let local = chrono::Local
        .from_local_datetime(&datetime)
        .single()?;
    Some(local.timestamp())
}

fn collapse_whitespace(s: &str) -> String {
    let trimmed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    trimmed
}

// ── Re-export for use in chrono ─────────────────────────────

use chrono::TimeZone as _;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_unread() {
        let q = parse_query("is:unread");
        assert_eq!(q.is_unread, Some(true));
        assert!(q.free_text.is_empty());
    }

    #[test]
    fn parses_from_with_free_text() {
        let q = parse_query("hello from:alice world");
        assert_eq!(q.from.as_deref(), Some("alice"));
        assert_eq!(q.free_text, "hello world");
    }

    #[test]
    fn parses_quoted_value() {
        let q = parse_query("from:\"John Doe\"");
        assert_eq!(q.from.as_deref(), Some("John Doe"));
    }

    #[test]
    fn parses_has_attachment() {
        let q = parse_query("has:attachment");
        assert_eq!(q.has_attachment, Some(true));
    }

    #[test]
    fn parses_date_filters() {
        let q = parse_query("after:2024/01/01 before:2024/12/31");
        assert!(q.after.is_some());
        assert!(q.before.is_some());
    }

    #[test]
    fn parses_label() {
        let q = parse_query("label:Important");
        assert_eq!(q.label.as_deref(), Some("Important"));
    }

    #[test]
    fn parses_multiple_operators() {
        let q = parse_query("is:unread is:starred from:bob has:attachment");
        assert_eq!(q.is_unread, Some(true));
        assert_eq!(q.is_starred, Some(true));
        assert_eq!(q.from.as_deref(), Some("bob"));
        assert_eq!(q.has_attachment, Some(true));
    }

    #[test]
    fn handles_case_insensitive_operators() {
        let q = parse_query("IS:Unread FROM:Alice");
        assert_eq!(q.is_unread, Some(true));
        assert_eq!(q.from.as_deref(), Some("Alice"));
    }

    #[test]
    fn parses_extended_is_values() {
        let q = parse_query("is:snoozed");
        assert_eq!(q.is_snoozed, Some(true));

        let q = parse_query("is:pinned");
        assert_eq!(q.is_pinned, Some(true));

        let q = parse_query("is:muted");
        assert_eq!(q.is_muted, Some(true));

        let q = parse_query("is:important");
        assert_eq!(q.is_important, Some(true));
    }

    #[test]
    fn date_with_dashes() {
        let q = parse_query("after:2024-06-15");
        assert!(q.after.is_some());
    }
}
