use crate::types::{ImportError, ImportFormat};

/// Detect import format from filename and file signature.
pub fn detect_format(filename: &str, data: &[u8]) -> Result<ImportFormat, ImportError> {
    let lower = filename.to_lowercase();
    if lower.ends_with(".vcf") || lower.ends_with(".vcard") {
        return Ok(ImportFormat::Vcf);
    }
    if lower.ends_with(".xlsx") || lower.ends_with(".xlsm") {
        return Ok(ImportFormat::Xlsx);
    }
    if lower.ends_with(".csv") || lower.ends_with(".txt") {
        return Ok(ImportFormat::Csv);
    }

    if data.starts_with(b"PK\x03\x04") {
        return Ok(ImportFormat::Xlsx);
    }

    let prefix = String::from_utf8_lossy(&data[..data.len().min(512)]);
    if prefix.trim_start().starts_with("BEGIN:VCARD") {
        return Ok(ImportFormat::Vcf);
    }

    if data.is_empty() {
        return Err(ImportError::EmptyFile);
    }

    Ok(ImportFormat::Csv)
}

/// Detect the encoding of raw bytes and convert to a UTF-8 string.
///
/// Checks BOM markers, then UTF-16 without BOM using NUL-byte layout,
/// then falls back to UTF-8 and Windows-1252.
pub fn decode_to_utf8(data: &[u8]) -> Result<String, String> {
    if data.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Ok(String::from_utf8_lossy(&data[3..]).into_owned());
    }

    if data.starts_with(&[0xFF, 0xFE]) {
        return decode_utf16(&data[2..], encoding_rs::UTF_16LE, "UTF-16 LE");
    }

    if data.starts_with(&[0xFE, 0xFF]) {
        return decode_utf16(&data[2..], encoding_rs::UTF_16BE, "UTF-16 BE");
    }

    if looks_like_utf16_le(data) {
        return decode_utf16(data, encoding_rs::UTF_16LE, "UTF-16 LE");
    }

    if looks_like_utf16_be(data) {
        return decode_utf16(data, encoding_rs::UTF_16BE, "UTF-16 BE");
    }

    if let Ok(s) = std::str::from_utf8(data) {
        return Ok(s.to_string());
    }

    let (decoded, _, _) = encoding_rs::WINDOWS_1252.decode(data);
    Ok(decoded.into_owned())
}

fn decode_utf16(
    data: &[u8],
    encoding: &'static encoding_rs::Encoding,
    label: &str,
) -> Result<String, String> {
    let (decoded, _, had_errors) = encoding.decode(data);
    if had_errors {
        Err(format!("{label} decoding error"))
    } else {
        Ok(decoded.into_owned())
    }
}

fn looks_like_utf16_le(data: &[u8]) -> bool {
    looks_like_utf16(data, 1)
}

fn looks_like_utf16_be(data: &[u8]) -> bool {
    looks_like_utf16(data, 0)
}

fn looks_like_utf16(data: &[u8], zero_offset: usize) -> bool {
    let sample_len = data.len().min(256);
    if sample_len < 8 {
        return false;
    }
    let mut zeroes = 0usize;
    let mut checked = 0usize;
    for (index, byte) in data[..sample_len].iter().enumerate() {
        if index % 2 == zero_offset {
            checked += 1;
            if *byte == 0 {
                zeroes += 1;
            }
        }
    }
    checked > 0 && zeroes * 2 >= checked
}

/// Detect the CSV delimiter by counting occurrences in the first few lines.
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

        if counts.iter().all(|&c| c == 0) {
            continue;
        }

        let avg = counts.iter().sum::<usize>() as f64 / counts.len() as f64;
        if avg < 0.5 {
            continue;
        }

        let variance = counts
            .iter()
            .map(|&c| {
                let diff = c as f64 - avg;
                diff * diff
            })
            .sum::<f64>()
            / counts.len() as f64;

        let score = avg / (1.0 + variance);
        if score > best_score {
            best_score = score;
            best_delim = delim;
        }
    }

    best_delim
}

fn count_unquoted_delimiters(line: &str, delim: u8) -> usize {
    let delim_char = char::from(delim);
    let mut count = 0;
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '"' {
            if in_quotes && chars.peek() == Some(&'"') {
                let _ = chars.next();
            } else {
                in_quotes = !in_quotes;
            }
        } else if ch == delim_char && !in_quotes {
            count += 1;
        }
    }

    count
}

/// Detect whether the first row is likely a header row.
pub fn detect_has_header(rows: &[Vec<String>]) -> bool {
    let Some(first_row) = rows.first() else {
        return false;
    };

    if rows.len() < 2 {
        return false;
    }

    let data_like_count = first_row
        .iter()
        .filter(|cell| looks_like_data(cell))
        .count();

    let threshold = first_row.len().div_ceil(2);
    data_like_count < threshold
}

fn looks_like_data(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }

    if crate::types::is_valid_email(trimmed) {
        return true;
    }

    if trimmed.starts_with('+') && trimmed.chars().skip(1).any(|c| c.is_ascii_digit()) {
        return true;
    }

    let digit_ratio =
        trimmed.chars().filter(char::is_ascii_digit).count() as f64 / trimmed.len() as f64;
    digit_ratio > 0.7 && trimmed.len() > 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_formats() {
        assert_eq!(
            detect_format("contacts.csv", b"Name,Email").expect("detect csv"),
            ImportFormat::Csv
        );
        assert_eq!(
            detect_format("contacts.vcf", b"BEGIN:VCARD").expect("detect vcf"),
            ImportFormat::Vcf
        );
        assert_eq!(
            detect_format("contacts.xlsx", b"PK\x03\x04").expect("detect xlsx"),
            ImportFormat::Xlsx
        );
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
            vec!["Name".to_string(), "Email".to_string(), "Phone".to_string()],
            vec![
                "Alice".to_string(),
                "alice@test.com".to_string(),
                "+1-555".to_string(),
            ],
        ];
        assert!(detect_has_header(&rows));
    }

    #[test]
    fn detect_no_header_row() {
        let rows = vec![
            vec![
                "Alice".to_string(),
                "alice@test.com".to_string(),
                "+1-555-1234".to_string(),
            ],
            vec![
                "Bob".to_string(),
                "bob@test.com".to_string(),
                "+1-555-5678".to_string(),
            ],
        ];
        assert!(!detect_has_header(&rows));
    }

    #[test]
    fn decode_utf16_le_without_bom() {
        let data = [
            0x4E, 0x00, 0x61, 0x00, 0x6D, 0x00, 0x65, 0x00, 0x0A, 0x00,
        ];
        assert_eq!(decode_to_utf8(&data).ok().as_deref(), Some("Name\n"));
    }

    #[test]
    fn decode_utf16_le_with_bom() {
        let data = [
            0xFF, 0xFE, 0x4E, 0x00, 0x61, 0x00, 0x6D, 0x00, 0x65, 0x00,
        ];
        assert_eq!(decode_to_utf8(&data).ok().as_deref(), Some("Name"));
    }

    #[test]
    fn decode_latin1() {
        let data = vec![0x52, 0xE9, 0x73, 0x75, 0x6D, 0xE9];
        let result = decode_to_utf8(&data).expect("should decode");
        assert!(result.contains('\u{00E9}'));
    }
}
