use std::ops::Range;

use crate::types::{clean_text, is_valid_email, normalize_email};

/// A recipient parsed from pasted To/Cc/Bcc text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRecipient {
    pub email: String,
    pub display_name: Option<String>,
}

/// All recipient-paste representations the app could read from the clipboard.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecipientPastePayload {
    pub plain_text: Option<String>,
    pub html: Option<String>,
    pub rtf: Option<String>,
}

impl RecipientPastePayload {
    pub fn from_plain_text(text: impl Into<String>) -> Self {
        Self {
            plain_text: Some(text.into()),
            html: None,
            rtf: None,
        }
    }
}

/// Which clipboard representation produced a recipient paste result.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RecipientPasteSourceFormat {
    Html,
    Rtf,
    PlainText,
    #[default]
    Empty,
}

/// Why a candidate recipient was skipped during paste parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecipientSkipReason {
    DuplicateEmail,
}

/// A skipped recipient candidate from a paste operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedRecipient {
    pub source_index: usize,
    pub email: String,
    pub display_name: Option<String>,
    pub reason: RecipientSkipReason,
}

/// Parsed recipient paste output plus source and skip diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecipientPasteResult {
    pub recipients: Vec<ParsedRecipient>,
    pub skipped: Vec<SkippedRecipient>,
    pub source_format: RecipientPasteSourceFormat,
}

/// Parse a rich recipient paste payload into normalized recipients.
///
/// Preference order is HTML table/fragment, RTF, then plain text. Callers that
/// only have text should use `RecipientPastePayload::from_plain_text`.
pub fn parse_recipient_paste(payload: &RecipientPastePayload) -> RecipientPasteResult {
    if let Some(html) = payload.html.as_deref().filter(|s| !s.trim().is_empty()) {
        let recipients = parse_html_recipients(html);
        if !recipients.is_empty() {
            return finish_paste_result(recipients, RecipientPasteSourceFormat::Html);
        }
    }

    if let Some(rtf) = payload.rtf.as_deref().filter(|s| !s.trim().is_empty()) {
        let recipients = parse_rtf_recipients(rtf);
        if !recipients.is_empty() {
            return finish_paste_result(recipients, RecipientPasteSourceFormat::Rtf);
        }
    }

    if let Some(text) = payload.plain_text.as_deref().filter(|s| !s.trim().is_empty()) {
        return finish_paste_result(
            parse_recipient_list(text),
            RecipientPasteSourceFormat::PlainText,
        );
    }

    RecipientPasteResult::default()
}

/// Parse a pasted recipient list into normalized recipients.
///
/// This is intentionally more forgiving than RFC address parsing. It is
/// built for corporate paste buffers from Word/Excel: mixed delimiters,
/// missing quotes, missing angle brackets, and name/email runs glued
/// together by bad copy/paste operations.
pub fn parse_recipient_list(input: &str) -> Vec<ParsedRecipient> {
    let spans = find_email_spans(input);
    let mut recipients = Vec::with_capacity(spans.len());
    let mut previous_email_end = 0usize;

    for span in spans {
        let raw_email = &input[span.clone()];
        let email = normalize_email(raw_email);
        if !is_valid_email(&email) {
            previous_email_end = span.end;
            continue;
        }

        let display_name = extract_display_name(input, previous_email_end, span.start);
        recipients.push(ParsedRecipient {
            email,
            display_name,
        });
        previous_email_end = span.end;
    }

    recipients
}

/// Deduplicate parsed recipients by email, keeping the first occurrence.
pub fn dedup_recipients(recipients: &mut Vec<ParsedRecipient>) {
    let mut seen = std::collections::HashSet::new();
    recipients.retain(|recipient| seen.insert(recipient.email.clone()));
}

fn finish_paste_result(
    recipients: Vec<ParsedRecipient>,
    source_format: RecipientPasteSourceFormat,
) -> RecipientPasteResult {
    let mut accepted = Vec::with_capacity(recipients.len());
    let mut skipped = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for (source_index, recipient) in recipients.into_iter().enumerate() {
        if seen.insert(recipient.email.clone()) {
            accepted.push(recipient);
        } else {
            skipped.push(SkippedRecipient {
                source_index,
                email: recipient.email,
                display_name: recipient.display_name,
                reason: RecipientSkipReason::DuplicateEmail,
            });
        }
    }

    RecipientPasteResult {
        recipients: accepted,
        skipped,
        source_format,
    }
}

fn parse_html_recipients(html: &str) -> Vec<ParsedRecipient> {
    let fragment = extract_cf_html_fragment(html).unwrap_or(html);

    for table in extract_html_tables(fragment) {
        let recipients = recipients_from_table_rows(table);
        if !recipients.is_empty() {
            return recipients;
        }
    }

    parse_recipient_list(&html_to_text(fragment))
}

fn recipients_from_table_rows(rows: Vec<Vec<String>>) -> Vec<ParsedRecipient> {
    let Ok(preview) = crate::table::build_table_preview(
        crate::types::ImportFormat::Csv,
        rows.clone(),
        None,
        Vec::new(),
        None,
        crate::types::ImportOptions::default(),
    ) else {
        return Vec::new();
    };

    crate::table::table_contacts_from_rows(rows, &preview.mappings, preview.has_header)
        .into_iter()
        .filter_map(|(_, contact)| {
            let email = contact.normalized_email()?;
            is_valid_email(&email).then(|| ParsedRecipient {
                email,
                display_name: contact.effective_display_name(),
            })
        })
        .collect()
}

fn extract_cf_html_fragment(html: &str) -> Option<&str> {
    let start = cf_html_offset(html, "StartFragment")?;
    let end = cf_html_offset(html, "EndFragment")?;
    if start >= end || end > html.len() {
        return None;
    }
    html.get(start..end)
}

fn cf_html_offset(html: &str, key: &str) -> Option<usize> {
    for line in html.lines().take(16) {
        let Some(rest) = line.strip_prefix(key) else {
            continue;
        };
        let value = rest.strip_prefix(':')?.trim();
        let offset = value.parse::<isize>().ok()?;
        let offset = usize::try_from(offset).ok()?;
        return Some(offset);
    }
    None
}

fn extract_html_tables(html: &str) -> Vec<Vec<Vec<String>>> {
    let mut tables = Vec::new();
    let mut cursor = 0usize;

    while let Some((table_start, table_content_start)) = find_open_tag(html, "table", cursor) {
        let table_end = find_close_tag(html, "table", table_content_start).unwrap_or(html.len());
        let rows = extract_html_rows(&html[table_content_start..table_end]);
        if !rows.is_empty() {
            tables.push(rows);
        }
        cursor = table_end.max(table_start + 1);
    }

    tables
}

fn extract_html_rows(table_html: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut cursor = 0usize;

    while let Some((row_start, row_content_start)) = find_open_tag(table_html, "tr", cursor) {
        let row_end = find_close_tag(table_html, "tr", row_content_start).unwrap_or(table_html.len());
        let cells = extract_html_cells(&table_html[row_content_start..row_end]);
        if !cells.is_empty() {
            rows.push(cells);
        }
        cursor = row_end.max(row_start + 1);
    }

    rows
}

fn extract_html_cells(row_html: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut cursor = 0usize;

    while let Some((tag, cell_start, cell_content_start)) = find_next_cell_open_tag(row_html, cursor) {
        let cell_end = find_close_tag(row_html, tag, cell_content_start).unwrap_or(row_html.len());
        let text = html_to_text(&row_html[cell_content_start..cell_end]);
        cells.push(clean_text(&text).unwrap_or_default());
        cursor = cell_end.max(cell_start + 1);
    }

    cells
}

fn find_next_cell_open_tag(html: &str, from: usize) -> Option<(&'static str, usize, usize)> {
    let td = find_open_tag(html, "td", from).map(|(start, end)| ("td", start, end));
    let th = find_open_tag(html, "th", from).map(|(start, end)| ("th", start, end));

    match (td, th) {
        (Some(td), Some(th)) if td.1 <= th.1 => Some(td),
        (Some(_), Some(th)) => Some(th),
        (Some(td), None) => Some(td),
        (None, Some(th)) => Some(th),
        (None, None) => None,
    }
}

fn find_open_tag(html: &str, tag: &str, from: usize) -> Option<(usize, usize)> {
    let mut cursor = from;
    loop {
        let needle = format!("<{tag}");
        let start = find_ascii_case_insensitive(html, &needle, cursor)?;
        let after_name = start + needle.len();
        if is_tag_boundary(html, after_name) {
            let tag_end = html[after_name..].find('>')? + after_name + 1;
            return Some((start, tag_end));
        }
        cursor = after_name;
    }
}

fn find_close_tag(html: &str, tag: &str, from: usize) -> Option<usize> {
    find_ascii_case_insensitive(html, &format!("</{tag}"), from)
}

fn find_ascii_case_insensitive(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    let haystack_bytes = haystack.as_bytes();
    let needle_bytes = needle.as_bytes();
    if needle_bytes.is_empty() || from >= haystack_bytes.len() {
        return None;
    }
    haystack_bytes[from..]
        .windows(needle_bytes.len())
        .position(|window| window.eq_ignore_ascii_case(needle_bytes))
        .map(|offset| from + offset)
}

fn is_tag_boundary(html: &str, index: usize) -> bool {
    html.as_bytes()
        .get(index)
        .is_some_and(|byte| byte.is_ascii_whitespace() || matches!(*byte, b'>' | b'/'))
}

fn html_to_text(html: &str) -> String {
    let mut out = String::new();
    let mut cursor = 0usize;

    while let Some(rel_start) = html[cursor..].find('<') {
        let start = cursor + rel_start;
        out.push_str(&decode_html_entities(&html[cursor..start]));

        let Some(rel_end) = html[start..].find('>') else {
            cursor = start;
            break;
        };
        let end = start + rel_end + 1;
        append_html_tag_separator(&mut out, &html[start + 1..end - 1]);
        cursor = end;
    }

    if cursor < html.len() {
        out.push_str(&decode_html_entities(&html[cursor..]));
    }

    out
}

fn append_html_tag_separator(out: &mut String, tag_body: &str) {
    let trimmed = tag_body.trim_start_matches('/').trim_start();
    let name: String = trimmed
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect();

    match name.as_str() {
        "br" | "p" | "div" | "tr" => out.push('\n'),
        "td" | "th" => out.push('\t'),
        _ => {}
    }
}

fn decode_html_entities(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0usize;

    while let Some(rel_amp) = input[cursor..].find('&') {
        let amp = cursor + rel_amp;
        out.push_str(&input[cursor..amp]);

        let Some(rel_semicolon) = input[amp..].find(';') else {
            cursor = amp;
            break;
        };
        let semicolon = amp + rel_semicolon;
        let entity = &input[amp + 1..semicolon];
        if let Some(decoded) = decode_html_entity(entity) {
            out.push_str(&decoded);
            cursor = semicolon + 1;
        } else {
            out.push('&');
            cursor = amp + 1;
        }
    }

    if cursor < input.len() {
        out.push_str(&input[cursor..]);
    }

    out
}

fn decode_html_entity(entity: &str) -> Option<String> {
    match entity {
        "amp" => Some("&".to_string()),
        "lt" => Some("<".to_string()),
        "gt" => Some(">".to_string()),
        "quot" => Some("\"".to_string()),
        "apos" | "#39" => Some("'".to_string()),
        "nbsp" => Some(" ".to_string()),
        _ if entity.starts_with("#x") || entity.starts_with("#X") => u32::from_str_radix(&entity[2..], 16)
            .ok()
            .and_then(char::from_u32)
            .map(|ch| ch.to_string()),
        _ if entity.starts_with('#') => entity[1..]
            .parse::<u32>()
            .ok()
            .and_then(char::from_u32)
            .map(|ch| ch.to_string()),
        _ => None,
    }
}

fn parse_rtf_recipients(rtf: &str) -> Vec<ParsedRecipient> {
    parse_recipient_list(&rtf_to_text(rtf))
}

fn rtf_to_text(rtf: &str) -> String {
    let chars: Vec<char> = rtf.chars().collect();
    let mut out = String::new();
    let mut index = 0usize;

    while index < chars.len() {
        match chars[index] {
            '{' | '}' => index += 1,
            '\\' => consume_rtf_control(&chars, &mut index, &mut out),
            ch => {
                out.push(ch);
                index += 1;
            }
        }
    }

    out
}

fn consume_rtf_control(chars: &[char], index: &mut usize, out: &mut String) {
    *index += 1;
    let Some(&next) = chars.get(*index) else {
        return;
    };

    if matches!(next, '\\' | '{' | '}') {
        out.push(next);
        *index += 1;
        return;
    }

    if next == '\'' {
        if let Some(ch) = decode_rtf_hex(chars, *index + 1) {
            out.push(ch);
        }
        *index = (*index + 3).min(chars.len());
        return;
    }

    if !next.is_ascii_alphabetic() {
        *index += 1;
        return;
    }

    let word_start = *index;
    while chars.get(*index).is_some_and(char::is_ascii_alphabetic) {
        *index += 1;
    }
    let word: String = chars[word_start..*index].iter().collect();

    let sign = if chars.get(*index) == Some(&'-') {
        *index += 1;
        -1i32
    } else {
        1i32
    };
    let number_start = *index;
    while chars.get(*index).is_some_and(char::is_ascii_digit) {
        *index += 1;
    }
    let number = if number_start < *index {
        chars[number_start..*index]
            .iter()
            .collect::<String>()
            .parse::<i32>()
            .ok()
            .map(|n| n * sign)
    } else {
        None
    };

    if chars.get(*index) == Some(&' ') {
        *index += 1;
    }

    match word.as_str() {
        "cell" | "tab" => out.push('\t'),
        "row" | "par" | "line" => out.push('\n'),
        "u" => {
            if let Some(value) = number.and_then(|n| u32::try_from(n).ok()).and_then(char::from_u32) {
                out.push(value);
            }
        }
        _ => {}
    }
}

fn decode_rtf_hex(chars: &[char], index: usize) -> Option<char> {
    let high = chars.get(index)?.to_digit(16)?;
    let low = chars.get(index + 1)?.to_digit(16)?;
    char::from_u32(high * 16 + low)
}

fn find_email_spans(input: &str) -> Vec<Range<usize>> {
    let mut spans = Vec::new();
    let bytes = input.as_bytes();

    for (at_index, byte) in bytes.iter().enumerate() {
        if *byte != b'@' {
            continue;
        }
        let Some(start) = scan_local_start(bytes, at_index) else {
            continue;
        };
        let Some(end) = scan_domain_end(input, at_index + 1) else {
            continue;
        };
        if start >= at_index || end <= at_index + 1 {
            continue;
        }
        let candidate = trim_email_span(input, start..end);
        if candidate.start < candidate.end && is_valid_email(&input[candidate.clone()]) {
            if spans
                .last()
                .is_some_and(|last: &Range<usize>| candidate.start < last.end)
            {
                continue;
            }
            spans.push(candidate);
        }
    }

    spans
}

fn scan_local_start(bytes: &[u8], at_index: usize) -> Option<usize> {
    if at_index == 0 {
        return None;
    }
    let mut start = at_index;
    while start > 0 && is_local_byte(bytes[start - 1]) {
        start -= 1;
    }
    (start < at_index).then_some(start)
}

fn scan_domain_end(input: &str, start: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut end = start;
    while end < bytes.len() && is_domain_byte(bytes[end]) {
        end += 1;
    }
    if end == start {
        return None;
    }

    Some(trim_glued_name_suffix(input, start, end))
}

fn trim_glued_name_suffix(input: &str, domain_start: usize, domain_end: usize) -> usize {
    let domain = &input[domain_start..domain_end];
    let Some(last_dot) = domain.rfind('.') else {
        return domain_end;
    };
    let suffix_start = domain_start + last_dot + 1;
    let suffix = &input[suffix_start..domain_end];
    if COMMON_TLDS.iter().any(|tld| suffix.eq_ignore_ascii_case(tld)) {
        return domain_end;
    }
    for tld in COMMON_TLDS {
        if suffix.len() > tld.len()
            && suffix[..tld.len()].eq_ignore_ascii_case(tld)
            && suffix.as_bytes()[tld.len()].is_ascii_uppercase()
        {
            return suffix_start + tld.len();
        }
    }
    domain_end
}

fn trim_email_span(input: &str, span: Range<usize>) -> Range<usize> {
    let mut end = span.end;
    while end > span.start {
        let Some(ch) = input[..end].chars().next_back() else {
            break;
        };
        if matches!(ch, '.' | ',' | ';' | ':' | ')' | ']' | '}' | '>' | '"' | '\'') {
            end -= ch.len_utf8();
        } else {
            break;
        }
    }
    span.start..end
}

fn extract_display_name(input: &str, previous_email_end: usize, email_start: usize) -> Option<String> {
    if previous_email_end >= email_start {
        return None;
    }

    let mut name = &input[previous_email_end..email_start];
    if let Some(line_break) = name.rfind(['\n', '\r']) {
        name = &name[line_break + 1..];
    }
    if let Some(angle_index) = name.rfind('<') {
        name = &name[..angle_index];
    }
    if let Some(close_index) = name.rfind('>') {
        name = &name[close_index + 1..];
    }
    let cleaned = clean_display_name(name)?;
    if is_valid_email(&cleaned) {
        None
    } else {
        Some(cleaned)
    }
}

fn clean_display_name(name: &str) -> Option<String> {
    let mut trimmed = name
        .trim_matches(|ch: char| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    '"' | '\'' | '<' | '>' | ',' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}'
                )
        })
        .trim();

    for prefix in ["To", "Cc", "Bcc"] {
        if trimmed.len() > prefix.len()
            && trimmed[..prefix.len()].eq_ignore_ascii_case(prefix)
            && trimmed[prefix.len()..].trim_start().starts_with(':')
        {
            trimmed = trimmed[prefix.len() + 1..].trim();
        }
    }

    let without_wrapping_quote =
        if trimmed.starts_with('"') || trimmed.starts_with('\'') {
            &trimmed[1..]
        } else {
            trimmed
        };
    let without_wrapping_quote =
        if without_wrapping_quote.ends_with('"') || without_wrapping_quote.ends_with('\'') {
            &without_wrapping_quote[..without_wrapping_quote.len() - 1]
        } else {
            without_wrapping_quote
        };

    let squashed = without_wrapping_quote
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    clean_text(&squashed)
}

fn is_local_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'%' | b'+' | b'-' | b'\'')
}

fn is_domain_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-')
}

const COMMON_TLDS: &[&str] = &[
    "com", "org", "net", "edu", "gov", "mil", "int", "io", "ai", "co", "uk", "us", "no", "se",
    "dk", "de", "fr", "nl", "es", "it", "pl", "ca", "au", "jp", "cn", "in", "br", "mx",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_addresses() {
        let parsed = parse_recipient_list(
            "Alice Smith <alice@example.com>, \"Bob Jones\" <bob@example.com>; carol@example.com",
        );
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].display_name.as_deref(), Some("Alice Smith"));
        assert_eq!(parsed[1].display_name.as_deref(), Some("Bob Jones"));
        assert_eq!(parsed[2].display_name, None);
    }

    #[test]
    fn parses_missing_quotes_and_angles() {
        let parsed = parse_recipient_list(
            "\"Alice Smith <alice@example.com Bob Jones bob@example.com",
        );
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].email, "alice@example.com");
        assert_eq!(parsed[0].display_name.as_deref(), Some("Alice Smith"));
        assert_eq!(parsed[1].email, "bob@example.com");
        assert_eq!(parsed[1].display_name.as_deref(), Some("Bob Jones"));
    }

    #[test]
    fn parses_excel_cells_and_newlines() {
        let parsed = parse_recipient_list(
            "Name\tEmail\nAlice Smith\talice@example.com\nBob Jones\tbob@example.com",
        );
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].display_name.as_deref(), Some("Alice Smith"));
        assert_eq!(parsed[1].display_name.as_deref(), Some("Bob Jones"));
    }

    #[test]
    fn trims_glued_capitalized_name_after_common_tld() {
        let parsed = parse_recipient_list(
            "Alice alice@example.comBob Jones <bob@example.com>",
        );
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].email, "alice@example.com");
        assert_eq!(parsed[1].email, "bob@example.com");
        assert_eq!(parsed[1].display_name.as_deref(), Some("Bob Jones"));
    }

    #[test]
    fn does_not_trim_uppercase_tld_as_shorter_tld_plus_name() {
        let parsed = parse_recipient_list("Alice@Test.COM");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].email, "alice@test.com");
    }

    #[test]
    fn deduplicates_by_normalized_email() {
        let mut parsed = parse_recipient_list("Alice <ALICE@example.com>, alice@example.com");
        dedup_recipients(&mut parsed);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].email, "alice@example.com");
    }

    #[test]
    fn rich_paste_prefers_html_table() {
        let payload = RecipientPastePayload {
            plain_text: Some("this fallback should not win".to_string()),
            html: Some(
                "<table><tr><th>Name</th><th>Email</th></tr>\
                 <tr><td>Alice Smith</td><td>alice@example.com</td></tr>\
                 <tr><td>Bob Jones</td><td>bob@example.com</td></tr></table>"
                    .to_string(),
            ),
            rtf: None,
        };

        let result = parse_recipient_paste(&payload);
        assert_eq!(result.source_format, RecipientPasteSourceFormat::Html);
        assert_eq!(result.recipients.len(), 2);
        assert_eq!(result.recipients[0].display_name.as_deref(), Some("Alice Smith"));
        assert_eq!(result.recipients[1].email, "bob@example.com");
    }

    #[test]
    fn rich_paste_reads_cf_html_fragment_offsets() {
        let fragment = "<table><tr><td>Alice</td><td>alice@example.com</td></tr></table>";
        let mut html = "Version:1.0\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\n<html><body><!--StartFragment-->".to_string();
        let start = html.len();
        html.push_str(fragment);
        let end = html.len();
        html.push_str("<!--EndFragment--></body></html>");
        html = html
            .replace("StartFragment:0000000000", &format!("StartFragment:{start:010}"))
            .replace("EndFragment:0000000000", &format!("EndFragment:{end:010}"));

        let result = parse_recipient_paste(&RecipientPastePayload {
            plain_text: None,
            html: Some(html),
            rtf: None,
        });

        assert_eq!(result.source_format, RecipientPasteSourceFormat::Html);
        assert_eq!(result.recipients[0].email, "alice@example.com");
    }

    #[test]
    fn rich_paste_falls_back_to_rtf_table_text() {
        let payload = RecipientPastePayload {
            plain_text: None,
            html: None,
            rtf: Some(
                r"{\rtf1 Name\cell Email\row Alice Smith\cell alice@example.com\row Bob\cell bob@example.com\row}"
                    .to_string(),
            ),
        };

        let result = parse_recipient_paste(&payload);
        assert_eq!(result.source_format, RecipientPasteSourceFormat::Rtf);
        assert_eq!(result.recipients.len(), 2);
        assert_eq!(result.recipients[0].display_name.as_deref(), Some("Alice Smith"));
    }

    #[test]
    fn rich_paste_reports_duplicate_candidates() {
        let result = parse_recipient_paste(&RecipientPastePayload::from_plain_text(
            "Alice <alice@example.com>, ALICE@example.com",
        ));

        assert_eq!(result.recipients.len(), 1);
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].reason, RecipientSkipReason::DuplicateEmail);
    }
}
