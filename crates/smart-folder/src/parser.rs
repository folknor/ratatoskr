use chrono::TimeZone as _;

/// MIME type expansions for `has:` operator values.
const HAS_EXPANSIONS: &[(&str, &[&str])] = &[
    ("pdf", &["application/pdf"]),
    (
        "image",
        &[
            "image/jpeg",
            "image/png",
            "image/gif",
            "image/webp",
            "image/svg+xml",
        ],
    ),
    (
        "excel",
        &[
            "application/vnd.ms-excel",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "application/vnd.oasis.opendocument.spreadsheet",
            "text/csv",
        ],
    ),
    (
        "word",
        &[
            "application/msword",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "application/vnd.oasis.opendocument.text",
            "application/rtf",
        ],
    ),
    (
        "powerpoint",
        &[
            "application/vnd.ms-powerpoint",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "application/vnd.oasis.opendocument.presentation",
        ],
    ),
    (
        "archive",
        &[
            "application/zip",
            "application/gzip",
            "application/x-tar",
            "application/x-7z-compressed",
            "application/x-rar-compressed",
        ],
    ),
    ("video", &["video/*"]),
    ("audio", &["audio/*"]),
    ("calendar", &["text/calendar", "application/ics"]),
];

/// A parsed smart folder query with structured filter fields.
#[derive(Debug, Default, Clone)]
pub struct ParsedQuery {
    pub free_text: String,

    // Repeated operators = OR (Vec instead of Option)
    pub from: Vec<String>,
    pub to: Vec<String>,
    pub account: Vec<String>,
    pub label: Vec<String>,
    pub folder: Vec<String>,
    pub in_folder: Vec<String>,

    // Attachment filtering
    pub has_attachment: bool,
    pub attachment_types: Vec<String>,
    pub has_contact: bool,

    // Flags
    pub is_unread: Option<bool>,
    pub is_read: Option<bool>,
    pub is_starred: Option<bool>,
    pub is_snoozed: Option<bool>,
    pub is_pinned: Option<bool>,
    pub is_muted: Option<bool>,
    pub is_tagged: Option<bool>,

    // Date
    pub before: Option<i64>,
    pub after: Option<i64>,
}

impl ParsedQuery {
    /// Returns `true` if any field other than `free_text` is set.
    pub fn has_any_operator(&self) -> bool {
        !self.from.is_empty()
            || !self.to.is_empty()
            || !self.account.is_empty()
            || !self.label.is_empty()
            || !self.folder.is_empty()
            || !self.in_folder.is_empty()
            || self.has_attachment
            || !self.attachment_types.is_empty()
            || self.has_contact
            || self.is_unread.is_some()
            || self.is_read.is_some()
            || self.is_starred.is_some()
            || self.is_snoozed.is_some()
            || self.is_pinned.is_some()
            || self.is_muted.is_some()
            || self.is_tagged.is_some()
            || self.before.is_some()
            || self.after.is_some()
    }
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

// -- Internal types --

struct OperatorSpan {
    start: usize,
    end: usize,
    operator: String,
    value: String,
}

/// Operators recognized by the parser.
const OPERATORS: &[&str] = &[
    "from", "to", "has", "is", "before", "after", "label", "account", "folder", "in", "type",
];

// -- Span collection --

/// Walk the input and extract all `operator:value` or `operator:"quoted value"` spans.
fn collect_operator_spans(input: &str) -> Vec<OperatorSpan> {
    let mut spans = Vec::new();
    let len = input.len();
    let mut pos = 0;

    while pos < len {
        if let Some(span) = try_parse_operator_at(input, pos) {
            pos = span.end;
            spans.push(span);
        } else {
            pos += 1;
        }
    }

    spans
}

/// Try to parse an operator at the given position.
fn try_parse_operator_at(input: &str, pos: usize) -> Option<OperatorSpan> {
    // Operator must be at start of string or preceded by whitespace.
    if pos > 0 && !input.as_bytes()[pos - 1].is_ascii_whitespace() {
        return None;
    }

    for &op in OPERATORS {
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
    let is_date_op = op == "before" || op == "after";
    let (value, end) = if is_date_op {
        extract_date_value(input, after_colon)
    } else {
        extract_value(input, after_colon)
    };

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
    let trimmed = rest.trim_start();
    let skip = rest.len() - trimmed.len();
    let start = pos + skip;

    if trimmed.starts_with('"') {
        // Quoted value -- find closing quote.
        if let Some(close) = trimmed[1..].find('"') {
            let value = trimmed[1..close + 1].to_owned();
            return (value, start + close + 2);
        }
    }

    // Unquoted -- take until whitespace.
    let token_end = trimmed
        .find(char::is_whitespace)
        .unwrap_or(trimmed.len());
    let value = trimmed[..token_end].to_owned();
    (value, start + token_end)
}

/// Extract a date value with greedy space-separated consumption.
///
/// After consuming the first token, peeks at following tokens. If they are
/// 1-2 digit numbers, consumes them as month/day components.
fn extract_date_value(input: &str, pos: usize) -> (String, usize) {
    let rest = &input[pos..];
    let trimmed = rest.trim_start();
    let skip = rest.len() - trimmed.len();
    let start = pos + skip;

    if trimmed.starts_with('"') {
        if let Some(close) = trimmed[1..].find('"') {
            let value = trimmed[1..close + 1].to_owned();
            return (value, start + close + 2);
        }
    }

    // Take the first non-whitespace token.
    let token_end = trimmed
        .find(char::is_whitespace)
        .unwrap_or(trimmed.len());
    let first_token = &trimmed[..token_end];

    // Only attempt greedy consumption for pure-digit year tokens (4 digits).
    if !is_four_digit_year(first_token) {
        return (first_token.to_owned(), start + token_end);
    }

    greedy_consume_date_parts(trimmed, first_token, token_end, start)
}

/// Check if a token is exactly a 4-digit year.
fn is_four_digit_year(token: &str) -> bool {
    token.len() == 4 && token.bytes().all(|b| b.is_ascii_digit())
}

/// After consuming a 4-digit year token, greedily consume following
/// 1-2 digit tokens as month and day components.
fn greedy_consume_date_parts(
    trimmed: &str,
    first_token: &str,
    first_end: usize,
    abs_start: usize,
) -> (String, usize) {
    let mut combined = first_token.to_owned();
    let mut cursor = first_end;

    // Try to consume up to 2 more parts (month, day).
    for _ in 0..2 {
        let after = &trimmed[cursor..];
        let next_trimmed = after.trim_start();
        if next_trimmed.is_empty() {
            break;
        }
        let ws_skip = after.len() - next_trimmed.len();

        let next_end = next_trimmed
            .find(char::is_whitespace)
            .unwrap_or(next_trimmed.len());
        let next_token = &next_trimmed[..next_end];

        if is_short_digit_token(next_token) {
            combined.push(' ');
            combined.push_str(next_token);
            cursor += ws_skip + next_end;
        } else {
            break;
        }
    }

    (combined, abs_start + cursor)
}

/// Check if a token is 1-2 digits (month or day component).
fn is_short_digit_token(token: &str) -> bool {
    let len = token.len();
    (1..=2).contains(&len) && token.bytes().all(|b| b.is_ascii_digit())
}

// -- Operator application --

fn apply_operator(result: &mut ParsedQuery, operator: &str, value: &str) {
    match operator {
        "from" => result.from.push(value.to_owned()),
        "to" => result.to.push(value.to_owned()),
        "has" => apply_has_operator(result, value),
        "is" => apply_is_operator(result, value),
        "before" => result.before = parse_date_to_timestamp(value),
        "after" => result.after = parse_date_to_timestamp(value),
        "label" => result.label.push(value.to_owned()),
        "account" => result.account.push(value.to_owned()),
        "folder" => result.folder.push(value.to_owned()),
        "in" => result.in_folder.push(value.to_owned()),
        "type" => result.attachment_types.push(value.to_owned()),
        _ => {}
    }
}

fn apply_has_operator(result: &mut ParsedQuery, value: &str) {
    let lower = value.to_ascii_lowercase();
    match lower.as_str() {
        "attachment" => result.has_attachment = true,
        "contact" => result.has_contact = true,
        _ => expand_has_value(result, &lower),
    }
}

/// Expand a `has:` value into MIME types via the expansion table.
fn expand_has_value(result: &mut ParsedQuery, value: &str) {
    // Handle aliases first.
    match value {
        "spreadsheet" => {
            push_expansion_mimes(result, "excel");
            return;
        }
        "document" => {
            push_expansion_mimes(result, "word");
            push_expansion_mimes(result, "pdf");
            return;
        }
        _ => {}
    }

    push_expansion_mimes(result, value);
}

/// Push MIME types from the expansion table for a given key.
fn push_expansion_mimes(result: &mut ParsedQuery, key: &str) {
    for &(name, mimes) in HAS_EXPANSIONS {
        if name == key {
            for &mime in mimes {
                result.attachment_types.push(mime.to_owned());
            }
            return;
        }
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
        "tagged" => result.is_tagged = Some(true),
        _ => {}
    }
}

// -- Date parsing --

/// Parse a date string into a Unix timestamp (seconds, start of day in local time).
///
/// Supported formats:
/// - Relative offsets: `-7` (7 days ago), `0` (today)
/// - Year only: `2025` -> January 1, 2025
/// - Year+month: `202603` -> March 1, 2026
/// - Full date: `20260311` -> March 11, 2026
/// - Slash-separated: `2026/03/11`
/// - Dash-separated: `2026-03-11`
/// - Space-separated: `2026 03 11` (from greedy consumption)
fn parse_date_to_timestamp(date_str: &str) -> Option<i64> {
    let trimmed = date_str.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Relative offset: starts with `-` or is `0`.
    if trimmed == "0" || trimmed.starts_with('-') {
        return parse_relative_offset(trimmed);
    }

    // Contains separator -> split on it.
    if trimmed.contains('/') || trimmed.contains('-') {
        return parse_separated_date(trimmed);
    }

    // Contains spaces -> split on space (from greedy consumption).
    if trimmed.contains(' ') {
        return parse_space_separated_date(trimmed);
    }

    // Pure digits -> length determines interpretation.
    parse_compact_date(trimmed)
}

/// Parse a relative offset like `-7` or `0` into a timestamp.
fn parse_relative_offset(s: &str) -> Option<i64> {
    let days: i64 = s.parse().ok()?;
    let today = chrono::Local::now().date_naive();
    let target = if days <= 0 {
        today + chrono::Duration::days(days)
    } else {
        // Positive numbers are not valid relative offsets.
        return None;
    };
    naive_date_to_timestamp(target)
}

/// Parse a date with `/` or `-` separators.
fn parse_separated_date(s: &str) -> Option<i64> {
    let sep = if s.contains('/') { '/' } else { '-' };
    let parts: Vec<&str> = s.split(sep).collect();
    match parts.len() {
        3 => {
            let year: i32 = parts[0].parse().ok()?;
            let month: u32 = parts[1].parse().ok()?;
            let day: u32 = parts[2].parse().ok()?;
            let date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
            naive_date_to_timestamp(date)
        }
        2 => {
            let year: i32 = parts[0].parse().ok()?;
            let month: u32 = parts[1].parse().ok()?;
            let date = chrono::NaiveDate::from_ymd_opt(year, month, 1)?;
            naive_date_to_timestamp(date)
        }
        _ => None,
    }
}

/// Parse a space-separated date like `2026 03 11`.
fn parse_space_separated_date(s: &str) -> Option<i64> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    match parts.len() {
        3 => {
            let year: i32 = parts[0].parse().ok()?;
            let month: u32 = parts[1].parse().ok()?;
            let day: u32 = parts[2].parse().ok()?;
            let date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
            naive_date_to_timestamp(date)
        }
        2 => {
            let year: i32 = parts[0].parse().ok()?;
            let month: u32 = parts[1].parse().ok()?;
            let date = chrono::NaiveDate::from_ymd_opt(year, month, 1)?;
            naive_date_to_timestamp(date)
        }
        _ => None,
    }
}

/// Parse a compact digit-only date string.
fn parse_compact_date(s: &str) -> Option<i64> {
    if !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    match s.len() {
        4 => {
            let year: i32 = s.parse().ok()?;
            let date = chrono::NaiveDate::from_ymd_opt(year, 1, 1)?;
            naive_date_to_timestamp(date)
        }
        6 => {
            let year: i32 = s[..4].parse().ok()?;
            let month: u32 = s[4..6].parse().ok()?;
            let date = chrono::NaiveDate::from_ymd_opt(year, month, 1)?;
            naive_date_to_timestamp(date)
        }
        8 => {
            let year: i32 = s[..4].parse().ok()?;
            let month: u32 = s[4..6].parse().ok()?;
            let day: u32 = s[6..8].parse().ok()?;
            let date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
            naive_date_to_timestamp(date)
        }
        _ => None,
    }
}

/// Convert a `NaiveDate` to a Unix timestamp at start of day in local time.
fn naive_date_to_timestamp(date: chrono::NaiveDate) -> Option<i64> {
    let datetime = date.and_hms_opt(0, 0, 0)?;
    let local = chrono::Local.from_local_datetime(&datetime).single()?;
    Some(local.timestamp())
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Basic operator parsing --

    #[test]
    fn parses_simple_unread() {
        let q = parse_query("is:unread");
        assert_eq!(q.is_unread, Some(true));
        assert!(q.free_text.is_empty());
    }

    #[test]
    fn parses_from_with_free_text() {
        let q = parse_query("hello from:alice world");
        assert_eq!(q.from, vec!["alice"]);
        assert_eq!(q.free_text, "hello world");
    }

    #[test]
    fn parses_quoted_value() {
        let q = parse_query("from:\"John Doe\"");
        assert_eq!(q.from, vec!["John Doe"]);
    }

    #[test]
    fn parses_has_attachment() {
        let q = parse_query("has:attachment");
        assert!(q.has_attachment);
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
        assert_eq!(q.label, vec!["Important"]);
    }

    #[test]
    fn parses_multiple_operators() {
        let q = parse_query("is:unread is:starred from:bob has:attachment");
        assert_eq!(q.is_unread, Some(true));
        assert_eq!(q.is_starred, Some(true));
        assert_eq!(q.from, vec!["bob"]);
        assert!(q.has_attachment);
    }

    #[test]
    fn handles_case_insensitive_operators() {
        let q = parse_query("IS:Unread FROM:Alice");
        assert_eq!(q.is_unread, Some(true));
        assert_eq!(q.from, vec!["Alice"]);
    }

    #[test]
    fn parses_extended_is_values() {
        let q = parse_query("is:snoozed");
        assert_eq!(q.is_snoozed, Some(true));

        let q = parse_query("is:pinned");
        assert_eq!(q.is_pinned, Some(true));

        let q = parse_query("is:muted");
        assert_eq!(q.is_muted, Some(true));
    }

    #[test]
    fn date_with_dashes() {
        let q = parse_query("after:2024-06-15");
        assert!(q.after.is_some());
    }

    // -- OR semantics --

    #[test]
    fn or_semantics_from() {
        let q = parse_query("from:alice from:bob");
        assert_eq!(q.from, vec!["alice", "bob"]);
    }

    #[test]
    fn or_semantics_to() {
        let q = parse_query("to:alice to:bob to:carol");
        assert_eq!(q.to, vec!["alice", "bob", "carol"]);
    }

    #[test]
    fn or_semantics_label() {
        let q = parse_query("label:Work label:Personal");
        assert_eq!(q.label, vec!["Work", "Personal"]);
    }

    // -- New operators --

    #[test]
    fn parses_account_operator() {
        let q = parse_query("account:work");
        assert_eq!(q.account, vec!["work"]);
    }

    #[test]
    fn parses_folder_operator() {
        let q = parse_query("folder:Inbox");
        assert_eq!(q.folder, vec!["Inbox"]);
    }

    #[test]
    fn parses_in_operator() {
        let q = parse_query("in:inbox");
        assert_eq!(q.in_folder, vec!["inbox"]);
    }

    #[test]
    fn parses_is_tagged() {
        let q = parse_query("is:tagged");
        assert_eq!(q.is_tagged, Some(true));
    }

    #[test]
    fn parses_has_contact() {
        let q = parse_query("has:contact");
        assert!(q.has_contact);
    }

    #[test]
    fn parses_type_operator() {
        let q = parse_query("type:application/pdf");
        assert_eq!(q.attachment_types, vec!["application/pdf"]);
    }

    // -- has: expansion --

    #[test]
    fn has_pdf_expansion() {
        let q = parse_query("has:pdf");
        assert_eq!(q.attachment_types, vec!["application/pdf"]);
    }

    #[test]
    fn has_image_expansion() {
        let q = parse_query("has:image");
        assert_eq!(q.attachment_types.len(), 5);
        assert!(q.attachment_types.contains(&"image/jpeg".to_owned()));
        assert!(q.attachment_types.contains(&"image/png".to_owned()));
    }

    #[test]
    fn has_excel_expansion() {
        let q = parse_query("has:excel");
        assert_eq!(q.attachment_types.len(), 4);
        assert!(q.attachment_types.contains(&"text/csv".to_owned()));
    }

    #[test]
    fn has_spreadsheet_alias() {
        let q_excel = parse_query("has:excel");
        let q_spreadsheet = parse_query("has:spreadsheet");
        assert_eq!(q_excel.attachment_types, q_spreadsheet.attachment_types);
    }

    #[test]
    fn has_document_union() {
        let q = parse_query("has:document");
        // Should contain word types + pdf.
        assert!(q.attachment_types.contains(&"application/msword".to_owned()));
        assert!(q.attachment_types.contains(&"application/pdf".to_owned()));
        assert!(q.attachment_types.contains(&"application/rtf".to_owned()));
    }

    #[test]
    fn has_archive_expansion() {
        let q = parse_query("has:archive");
        assert!(q.attachment_types.contains(&"application/zip".to_owned()));
        assert!(q.attachment_types.contains(&"application/gzip".to_owned()));
    }

    #[test]
    fn has_video_expansion() {
        let q = parse_query("has:video");
        assert_eq!(q.attachment_types, vec!["video/*"]);
    }

    #[test]
    fn has_audio_expansion() {
        let q = parse_query("has:audio");
        assert_eq!(q.attachment_types, vec!["audio/*"]);
    }

    #[test]
    fn has_calendar_expansion() {
        let q = parse_query("has:calendar");
        assert!(q.attachment_types.contains(&"text/calendar".to_owned()));
        assert!(q.attachment_types.contains(&"application/ics".to_owned()));
    }

    #[test]
    fn has_powerpoint_expansion() {
        let q = parse_query("has:powerpoint");
        assert_eq!(q.attachment_types.len(), 3);
    }

    // -- Date parsing --

    #[test]
    fn date_relative_offset_negative() {
        let q = parse_query("after:-7");
        assert!(q.after.is_some());
        // Should be 7 days ago at start of day.
        let today = chrono::Local::now().date_naive();
        let expected = today - chrono::Duration::days(7);
        let expected_ts = naive_date_to_timestamp(expected);
        assert_eq!(q.after, expected_ts);
    }

    #[test]
    fn date_relative_offset_zero() {
        let q = parse_query("after:0");
        assert!(q.after.is_some());
        let today = chrono::Local::now().date_naive();
        let expected_ts = naive_date_to_timestamp(today);
        assert_eq!(q.after, expected_ts);
    }

    #[test]
    fn date_year_only() {
        let q = parse_query("after:2025");
        let expected = chrono::NaiveDate::from_ymd_opt(2025, 1, 1)
            .and_then(|d| naive_date_to_timestamp(d));
        assert_eq!(q.after, expected);
    }

    #[test]
    fn date_year_month_compact() {
        let q = parse_query("after:202603");
        let expected = chrono::NaiveDate::from_ymd_opt(2026, 3, 1)
            .and_then(|d| naive_date_to_timestamp(d));
        assert_eq!(q.after, expected);
    }

    #[test]
    fn date_full_compact() {
        let q = parse_query("after:20260311");
        let expected = chrono::NaiveDate::from_ymd_opt(2026, 3, 11)
            .and_then(|d| naive_date_to_timestamp(d));
        assert_eq!(q.after, expected);
    }

    #[test]
    fn date_slash_separated() {
        let q = parse_query("before:2026/03/11");
        let expected = chrono::NaiveDate::from_ymd_opt(2026, 3, 11)
            .and_then(|d| naive_date_to_timestamp(d));
        assert_eq!(q.before, expected);
    }

    #[test]
    fn date_dash_separated() {
        let q = parse_query("before:2026-03-11");
        let expected = chrono::NaiveDate::from_ymd_opt(2026, 3, 11)
            .and_then(|d| naive_date_to_timestamp(d));
        assert_eq!(q.before, expected);
    }

    #[test]
    fn date_space_separated_greedy() {
        let q = parse_query("after:2026 03 11");
        let expected = chrono::NaiveDate::from_ymd_opt(2026, 3, 11)
            .and_then(|d| naive_date_to_timestamp(d));
        assert_eq!(q.after, expected);
    }

    #[test]
    fn date_space_separated_year_month_only() {
        let q = parse_query("after:2026 03 hello");
        let expected = chrono::NaiveDate::from_ymd_opt(2026, 3, 1)
            .and_then(|d| naive_date_to_timestamp(d));
        assert_eq!(q.after, expected);
        assert_eq!(q.free_text, "hello");
    }

    #[test]
    fn date_space_greedy_does_not_consume_non_digits() {
        let q = parse_query("after:2026 hello");
        let expected = chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
            .and_then(|d| naive_date_to_timestamp(d));
        assert_eq!(q.after, expected);
        assert_eq!(q.free_text, "hello");
    }

    // -- has_any_operator helper --

    #[test]
    fn has_any_operator_empty() {
        let q = ParsedQuery::default();
        assert!(!q.has_any_operator());
    }

    #[test]
    fn has_any_operator_with_from() {
        let q = parse_query("from:alice");
        assert!(q.has_any_operator());
    }

    #[test]
    fn has_any_operator_free_text_only() {
        let q = parse_query("hello world");
        assert!(!q.has_any_operator());
    }

    #[test]
    fn has_any_operator_with_flags() {
        let q = parse_query("is:unread");
        assert!(q.has_any_operator());
    }

    #[test]
    fn has_any_operator_with_date() {
        let q = parse_query("after:2024/01/01");
        assert!(q.has_any_operator());
    }

    #[test]
    fn has_any_operator_with_attachment() {
        let q = parse_query("has:attachment");
        assert!(q.has_any_operator());
    }

    // -- Free text extraction with new operators --

    #[test]
    fn free_text_with_new_operators() {
        let q = parse_query("hello account:work folder:Inbox in:sent world");
        assert_eq!(q.free_text, "hello world");
        assert_eq!(q.account, vec!["work"]);
        assert_eq!(q.folder, vec!["Inbox"]);
        assert_eq!(q.in_folder, vec!["sent"]);
    }

    #[test]
    fn complex_query_with_many_operators() {
        let q = parse_query(
            "meeting notes from:alice from:bob label:Work is:unread has:pdf account:personal",
        );
        assert_eq!(q.free_text, "meeting notes");
        assert_eq!(q.from, vec!["alice", "bob"]);
        assert_eq!(q.label, vec!["Work"]);
        assert_eq!(q.is_unread, Some(true));
        assert_eq!(q.attachment_types, vec!["application/pdf"]);
        assert_eq!(q.account, vec!["personal"]);
    }

    // -- Removed operators should not parse --

    #[test]
    fn subject_not_parsed_as_operator() {
        let q = parse_query("subject:meeting");
        // "subject:" is not an operator, so it becomes free text.
        assert_eq!(q.free_text, "subject:meeting");
    }

    #[test]
    fn is_important_not_parsed() {
        let q = parse_query("is:important");
        // "important" is not a recognized is: value, so the operator span is
        // still consumed but no flag is set.
        assert!(q.free_text.is_empty());
        // Verify no flags were set.
        assert!(!q.has_any_operator());
    }
}
