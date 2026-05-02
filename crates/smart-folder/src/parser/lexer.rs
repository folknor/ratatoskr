// -- Internal types --

pub(super) struct OperatorSpan {
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) operator: String,
    pub(super) value: String,
}

/// Operators recognized by the parser.
pub(super) const OPERATORS: &[&str] = &[
    "from", "to", "has", "is", "before", "after", "label", "account", "folder", "in", "type",
];

// -- Span collection --

/// Walk the input and extract all `operator:value` or `operator:"quoted value"` spans.
pub(super) fn collect_operator_spans(input: &str) -> Vec<OperatorSpan> {
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

    if let Some(after_quote) = trimmed.strip_prefix('"') {
        // Quoted value -- find closing quote.
        if let Some(close) = after_quote.find('"') {
            let value = after_quote[..close].to_owned();
            return (value, start + close + 2);
        }
    }

    // Unquoted -- take until whitespace.
    let token_end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
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

    if let Some(after_quote) = trimmed.strip_prefix('"')
        && let Some(close) = after_quote.find('"')
    {
        let value = after_quote[..close].to_owned();
        return (value, start + close + 2);
    }

    // Take the first non-whitespace token.
    let token_end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
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
