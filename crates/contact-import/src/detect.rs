/// Detect the encoding of raw bytes and convert to a UTF-8 string.
///
/// Checks for BOM markers (UTF-8, UTF-16 LE/BE), then falls back to
/// heuristic detection between UTF-8 and Windows-1252/Latin-1.
pub fn decode_to_utf8(data: &[u8]) -> Result<String, String> {
    // UTF-8 BOM
    if data.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Ok(String::from_utf8_lossy(&data[3..]).into_owned());
    }

    // UTF-16 LE BOM
    if data.starts_with(&[0xFF, 0xFE]) {
        let (decoded, _, had_errors) = encoding_rs::UTF_16LE.decode(data);
        if had_errors {
            return Err("UTF-16 LE decoding error".into());
        }
        return Ok(decoded.into_owned());
    }

    // UTF-16 BE BOM
    if data.starts_with(&[0xFE, 0xFF]) {
        let (decoded, _, had_errors) = encoding_rs::UTF_16BE.decode(data);
        if had_errors {
            return Err("UTF-16 BE decoding error".into());
        }
        return Ok(decoded.into_owned());
    }

    // Try UTF-8 first
    if let Ok(s) = std::str::from_utf8(data) {
        return Ok(s.to_string());
    }

    // Fall back to Windows-1252 (superset of Latin-1)
    let (decoded, _, _) = encoding_rs::WINDOWS_1252.decode(data);
    Ok(decoded.into_owned())
}

/// Detect the CSV delimiter by counting occurrences in the first few lines.
///
/// Checks comma, semicolon, and tab. Returns the delimiter that appears
/// most consistently across lines.
pub fn detect_delimiter(text: &str) -> u8 {
    let candidates: &[u8] = b",;\t";
    let lines: Vec<&str> = text.lines().take(10).collect();

    if lines.is_empty() {
        return b',';
    }

    let mut best_delim = b',';
    let mut best_score: f64 = 0.0;

    for &delim in candidates {
        let counts: Vec<usize> = lines
            .iter()
            .map(|line| count_unquoted_delimiters(line, delim))
            .collect();

        // Skip if no occurrences
        if counts.iter().all(|&c| c == 0) {
            continue;
        }

        // Score: consistency (low variance) + occurrence count
        let avg = counts.iter().sum::<usize>() as f64 / counts.len() as f64;
        if avg < 0.5 {
            continue;
        }

        let variance = counts.iter().map(|&c| {
            let diff = c as f64 - avg;
            diff * diff
        }).sum::<f64>() / counts.len() as f64;

        // Lower variance is better; higher average count is better
        let score = avg / (1.0 + variance);
        if score > best_score {
            best_score = score;
            best_delim = delim;
        }
    }

    best_delim
}

/// Count occurrences of a delimiter that are not inside quoted fields.
fn count_unquoted_delimiters(line: &str, delim: u8) -> usize {
    let delim_char = delim as char;
    let mut count = 0;
    let mut in_quotes = false;

    for ch in line.chars() {
        if ch == '"' {
            in_quotes = !in_quotes;
        } else if ch == delim_char && !in_quotes {
            count += 1;
        }
    }

    count
}

/// Detect whether the first row is likely a header row.
///
/// Heuristic: if the first row contains values that look like data
/// (email addresses, phone numbers, mostly digits), it's probably
/// not a header.
pub fn detect_has_header(rows: &[Vec<String>]) -> bool {
    let Some(first_row) = rows.first() else {
        return false;
    };

    // If there's only one row, assume it's data
    if rows.len() < 2 {
        return false;
    }

    let data_like_count = first_row
        .iter()
        .filter(|cell| looks_like_data(cell))
        .count();

    // If more than half the cells look like data, it's not a header
    let threshold = first_row.len().div_ceil(2);
    data_like_count < threshold
}

/// Check if a cell value looks like data rather than a header label.
fn looks_like_data(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }

    // Contains @: likely an email
    if trimmed.contains('@') {
        return true;
    }

    // Starts with + and has digits: likely a phone number
    if trimmed.starts_with('+') && trimmed.chars().skip(1).any(|c| c.is_ascii_digit()) {
        return true;
    }

    // All digits (or digits with common phone separators): likely data
    let digit_ratio = trimmed
        .chars()
        .filter(char::is_ascii_digit)
        .count() as f64
        / trimmed.len() as f64;
    if digit_ratio > 0.7 && trimmed.len() > 3 {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_comma_delimiter() {
        let text = "Name,Email,Phone\nAlice,alice@test.com,555-1234\n";
        assert_eq!(detect_delimiter(text), b',');
    }

    #[test]
    fn detect_semicolon_delimiter() {
        let text = "Name;Email;Phone\nAlice;alice@test.com;555-1234\n";
        assert_eq!(detect_delimiter(text), b';');
    }

    #[test]
    fn detect_tab_delimiter() {
        let text = "Name\tEmail\tPhone\nAlice\talice@test.com\t555-1234\n";
        assert_eq!(detect_delimiter(text), b'\t');
    }

    #[test]
    fn detect_header_row() {
        let rows = vec![
            vec!["Name".into(), "Email".into(), "Phone".into()],
            vec!["Alice".into(), "alice@test.com".into(), "+1-555".into()],
        ];
        assert!(detect_has_header(&rows));
    }

    #[test]
    fn detect_no_header_row() {
        let rows = vec![
            vec!["Alice".into(), "alice@test.com".into(), "+1-555-1234".into()],
            vec!["Bob".into(), "bob@test.com".into(), "+1-555-5678".into()],
        ];
        assert!(!detect_has_header(&rows));
    }

    #[test]
    fn decode_utf8() {
        let data = b"hello world";
        assert_eq!(decode_to_utf8(data).ok().as_deref(), Some("hello world"));
    }

    #[test]
    fn decode_utf8_bom() {
        let mut data = vec![0xEF, 0xBB, 0xBF];
        data.extend_from_slice(b"hello");
        assert_eq!(decode_to_utf8(&data).ok().as_deref(), Some("hello"));
    }

    #[test]
    fn decode_latin1() {
        // 0xE9 = e-acute in Latin-1/Windows-1252
        let data = vec![0x52, 0xE9, 0x73, 0x75, 0x6D, 0xE9]; // Resume with accents
        let result = decode_to_utf8(&data).expect("should decode");
        assert!(result.contains('\u{00E9}')); // e-acute in Unicode
    }
}
