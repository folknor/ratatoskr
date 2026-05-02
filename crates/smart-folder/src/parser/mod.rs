mod apply;
mod dates;
mod lexer;

#[cfg(test)]
mod tests;

use lexer::OPERATORS;

// ── Cursor context analysis ─────────────────────────────

/// Result of analyzing cursor position within a query string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CursorContext {
    /// Cursor is in free text (no operator context).
    FreeText,
    /// Cursor is inside an operator value.
    InsideOperator {
        /// The operator name (e.g., "from", "to", "label").
        operator: String,
        /// The partial value typed so far (e.g., "ali" from "from:ali").
        partial_value: String,
        /// Byte offset where the operator value starts in the query string.
        value_start: usize,
        /// Byte offset where the partial value ends (cursor position).
        value_end: usize,
    },
}

/// Analyze the cursor position in a query string to determine operator context.
///
/// Walks backward from `cursor_pos` to find the nearest `operator:` prefix.
/// If found and no unquoted whitespace sits between the colon and cursor,
/// we are inside that operator's value.
pub fn analyze_cursor_context(query: &str, cursor_pos: usize) -> CursorContext {
    let cursor = cursor_pos.min(query.len());

    // Look backward from cursor to find `operator:` pattern.
    let before = &query[..cursor];

    // Find the last colon before the cursor that might be an operator.
    for (colon_pos, _) in before.rmatch_indices(':') {
        // Extract the word before the colon (the potential operator name).
        let before_colon = &before[..colon_pos];
        let op_start = before_colon.rfind(char::is_whitespace).map_or(0, |p| p + 1);
        let candidate = &before_colon[op_start..];

        if candidate.is_empty() {
            continue;
        }

        // Check if this candidate is a recognized operator (case-insensitive).
        let lower = candidate.to_ascii_lowercase();
        if !OPERATORS.contains(&lower.as_str()) {
            continue;
        }

        // Everything from after the colon to the cursor is the partial value.
        let value_start = colon_pos + 1;
        let partial = &query[value_start..cursor];

        // If the partial value starts with a quote, allow spaces inside it.
        if let Some(after_open) = partial.strip_prefix('"') {
            // Inside a quoted value - check if there's a closing quote before cursor.
            if after_open.contains('"') {
                // Closing quote found before cursor - not inside the operator anymore.
                continue;
            }
            // Still inside an open quote.
            return CursorContext::InsideOperator {
                operator: lower,
                partial_value: after_open.to_owned(),
                value_start,
                value_end: cursor,
            };
        }

        // Unquoted value - if there's whitespace in the partial, the cursor
        // has moved past this operator's value into free text.
        if partial.contains(char::is_whitespace) {
            continue;
        }

        return CursorContext::InsideOperator {
            operator: lower,
            partial_value: partial.to_owned(),
            value_start,
            value_end: cursor,
        };
    }

    CursorContext::FreeText
}

// ── Parsed query types ──────────────────────────────────

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
    let spans = lexer::collect_operator_spans(input);

    // Apply each operator match, tracking which spans were recognized.
    let recognized: Vec<bool> = spans
        .iter()
        .map(|span| apply::apply_operator(&mut result, &span.operator, &span.value))
        .collect();

    // Remove only recognized spans from input to get free text (process in reverse).
    // Unrecognized operator spans (e.g. `is:important`) stay as free text.
    for (span, _) in spans
        .iter()
        .zip(recognized.iter())
        .rev()
        .filter(|(_, r)| **r)
    {
        remaining = format!("{}{}", &remaining[..span.start], &remaining[span.end..]);
    }

    result.free_text = collapse_whitespace(&remaining);
    result
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
