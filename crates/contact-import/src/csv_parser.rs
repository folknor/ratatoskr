use crate::detect::{decode_to_utf8, detect_delimiter, detect_has_header};
use crate::mapping::ColumnMapping;
use crate::types::{ImportError, ImportPreview, ImportSource, ImportedContact};

/// Parse a CSV file into a preview with auto-detected settings.
///
/// Detects encoding, delimiter, and header presence. Returns a preview
/// with the first `preview_rows` rows and suggested column mappings.
pub fn parse_csv(source: &ImportSource, preview_rows: usize) -> Result<ImportPreview, ImportError> {
    let text = decode_to_utf8(&source.data).map_err(ImportError::EncodingError)?;

    if text.trim().is_empty() {
        return Err(ImportError::EmptyFile);
    }

    let delimiter = detect_delimiter(&text);
    let all_rows = parse_csv_text(&text, delimiter)?;

    if all_rows.is_empty() {
        return Err(ImportError::EmptyFile);
    }

    let has_header = detect_has_header(&all_rows);

    let (headers, data_rows) = if has_header {
        let headers = all_rows[0].clone();
        let data = all_rows[1..].to_vec();
        (headers, data)
    } else {
        // Generate synthetic headers
        let col_count = all_rows.iter().map(Vec::len).max().unwrap_or(0);
        let headers: Vec<String> = (1..=col_count).map(|i| format!("Column {i}")).collect();
        (headers, all_rows)
    };

    let total_rows = data_rows.len();
    let sample_rows: Vec<Vec<String>> = data_rows
        .into_iter()
        .take(preview_rows)
        .collect();

    Ok(ImportPreview {
        headers,
        sample_rows,
        total_rows,
        has_header,
        delimiter: Some(delimiter),
    })
}

/// Parse a CSV file with an explicit header override.
///
/// Like `parse_csv`, but the caller specifies whether the first row
/// is a header instead of auto-detecting.
pub fn parse_csv_with_header(
    source: &ImportSource,
    preview_rows: usize,
    has_header: bool,
) -> Result<ImportPreview, ImportError> {
    let text = decode_to_utf8(&source.data).map_err(ImportError::EncodingError)?;

    if text.trim().is_empty() {
        return Err(ImportError::EmptyFile);
    }

    let delimiter = detect_delimiter(&text);
    let all_rows = parse_csv_text(&text, delimiter)?;

    if all_rows.is_empty() {
        return Err(ImportError::EmptyFile);
    }

    let (headers, data_rows) = if has_header {
        let headers = all_rows[0].clone();
        let data = all_rows[1..].to_vec();
        (headers, data)
    } else {
        let col_count = all_rows.iter().map(Vec::len).max().unwrap_or(0);
        let headers: Vec<String> = (1..=col_count).map(|i| format!("Column {i}")).collect();
        (headers, all_rows)
    };

    let total_rows = data_rows.len();
    let sample_rows: Vec<Vec<String>> = data_rows
        .into_iter()
        .take(preview_rows)
        .collect();

    Ok(ImportPreview {
        headers,
        sample_rows,
        total_rows,
        has_header,
        delimiter: Some(delimiter),
    })
}

/// Execute a CSV import with the given column mappings.
///
/// Parses the full file and applies mappings to produce contacts.
pub fn execute_csv_import(
    source: &ImportSource,
    mappings: &[ColumnMapping],
    has_header: bool,
) -> Result<Vec<ImportedContact>, ImportError> {
    let text = decode_to_utf8(&source.data).map_err(ImportError::EncodingError)?;
    let delimiter = detect_delimiter(&text);
    let all_rows = parse_csv_text(&text, delimiter)?;

    let data_rows = if has_header && !all_rows.is_empty() {
        &all_rows[1..]
    } else {
        &all_rows
    };

    let contacts: Vec<ImportedContact> = data_rows
        .iter()
        .map(|row| row_to_contact(row, mappings))
        .collect();

    Ok(contacts)
}

/// Parse raw CSV text into rows of cells.
fn parse_csv_text(text: &str, delimiter: u8) -> Result<Vec<Vec<String>>, ImportError> {
    let delim = delimiter as char;
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut current_row: Vec<String> = Vec::new();
    let mut current_cell = String::new();
    let mut in_quotes = false;
    let mut prev_was_quote = false;

    for ch in text.chars() {
        if prev_was_quote {
            prev_was_quote = false;
            if ch == '"' {
                // Escaped quote inside quoted field
                current_cell.push('"');
                continue;
            }
            // End of quoted field
            in_quotes = false;
            // Fall through to handle the current char normally
        }

        if ch == '"' && !in_quotes && current_cell.is_empty() {
            in_quotes = true;
            continue;
        }

        if ch == '"' && in_quotes {
            prev_was_quote = true;
            continue;
        }

        if ch == delim && !in_quotes {
            current_row.push(current_cell.trim().to_string());
            current_cell = String::new();
            continue;
        }

        if (ch == '\n' || ch == '\r') && !in_quotes {
            if ch == '\r' {
                // Skip \r, the \n will end the row (or if \r alone, end here)
                continue;
            }
            current_row.push(current_cell.trim().to_string());
            current_cell = String::new();
            if !current_row.iter().all(String::is_empty) {
                rows.push(current_row);
            }
            current_row = Vec::new();
            continue;
        }

        current_cell.push(ch);
    }

    // Handle final row
    if prev_was_quote {
        in_quotes = false;
    }
    let _ = in_quotes; // consumed
    if !current_cell.is_empty() || !current_row.is_empty() {
        current_row.push(current_cell.trim().to_string());
        if !current_row.iter().all(String::is_empty) {
            rows.push(current_row);
        }
    }

    Ok(rows)
}

/// Convert a single CSV row into an `ImportedContact` using the mappings.
fn row_to_contact(row: &[String], mappings: &[ColumnMapping]) -> ImportedContact {
    let mut contact = ImportedContact::default();

    for mapping in mappings {
        let value = row
            .get(mapping.source_index)
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        if value.is_empty() {
            continue;
        }

        match mapping.target_field {
            crate::types::ContactField::DisplayName => contact.display_name = Some(value),
            crate::types::ContactField::FirstName => contact.first_name = Some(value),
            crate::types::ContactField::LastName => contact.last_name = Some(value),
            crate::types::ContactField::Email => contact.email = Some(value.to_lowercase()),
            crate::types::ContactField::Email2 => contact.email2 = Some(value.to_lowercase()),
            crate::types::ContactField::Phone => contact.phone = Some(value),
            crate::types::ContactField::Company => contact.company = Some(value),
            crate::types::ContactField::Notes => contact.notes = Some(value),
            crate::types::ContactField::Group => {
                // Split on common group delimiters
                let groups: Vec<String> = value
                    .split([';', ',', '|'])
                    .map(|g| g.trim().to_string())
                    .filter(|g| !g.is_empty())
                    .collect();
                contact.groups.extend(groups);
            }
            crate::types::ContactField::Ignore => {}
        }
    }

    contact
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ImportFormat;

    fn make_source(csv_text: &str) -> ImportSource {
        ImportSource {
            format: ImportFormat::Csv,
            data: csv_text.as_bytes().to_vec(),
            filename: "test.csv".into(),
        }
    }

    #[test]
    fn parse_basic_csv() {
        let source = make_source("Name,Email,Phone\nAlice,alice@test.com,555-1234\nBob,bob@test.com,555-5678\n");
        let preview = parse_csv(&source, 10).expect("should parse");
        assert_eq!(preview.headers, vec!["Name", "Email", "Phone"]);
        assert_eq!(preview.sample_rows.len(), 2);
        assert_eq!(preview.total_rows, 2);
        assert!(preview.has_header);
    }

    #[test]
    fn parse_csv_quoted_fields() {
        let source = make_source("Name,Email\n\"Smith, Alice\",alice@test.com\n");
        let preview = parse_csv(&source, 10).expect("should parse");
        assert_eq!(preview.sample_rows[0][0], "Smith, Alice");
    }

    #[test]
    fn parse_csv_semicolon_delimiter() {
        let source = make_source("Name;Email;Phone\nAlice;alice@test.com;555\n");
        let preview = parse_csv(&source, 10).expect("should parse");
        assert_eq!(preview.headers, vec!["Name", "Email", "Phone"]);
        assert_eq!(preview.delimiter, Some(b';'));
    }

    #[test]
    fn parse_csv_no_header() {
        let source = make_source("alice@test.com,Alice,+1-555-1234\nbob@test.com,Bob,+1-555-5678\n");
        let preview = parse_csv(&source, 10).expect("should parse");
        assert!(!preview.has_header);
        assert_eq!(preview.total_rows, 2);
    }

    #[test]
    fn execute_csv_with_mappings() {
        let source = make_source("Name,Email,Phone\nAlice,alice@test.com,555-1234\n");
        let mappings = vec![
            ColumnMapping { source_index: 0, source_column: "Name".into(), target_field: crate::types::ContactField::DisplayName },
            ColumnMapping { source_index: 1, source_column: "Email".into(), target_field: crate::types::ContactField::Email },
            ColumnMapping { source_index: 2, source_column: "Phone".into(), target_field: crate::types::ContactField::Phone },
        ];
        let contacts = execute_csv_import(&source, &mappings, true).expect("should import");
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].display_name.as_deref(), Some("Alice"));
        assert_eq!(contacts[0].email.as_deref(), Some("alice@test.com"));
        assert_eq!(contacts[0].phone.as_deref(), Some("555-1234"));
    }

    #[test]
    fn empty_csv_returns_error() {
        let source = make_source("");
        assert!(parse_csv(&source, 10).is_err());
    }
}
