use crate::detect::{decode_to_utf8, detect_delimiter};
use crate::table::{build_table_preview, prepare_table_import};
use crate::types::{
    ColumnMapping, ImportError, ImportFormat, ImportOptions, ImportSource, PreparedImport,
    TablePreview,
};

/// Parse a CSV file into a table preview.
pub fn preview_csv(
    source: &ImportSource,
    options: ImportOptions,
) -> Result<TablePreview, ImportError> {
    let (rows, delimiter) = load_csv_rows(source)?;
    build_table_preview(
        ImportFormat::Csv,
        rows,
        Some(delimiter),
        Vec::new(),
        None,
        options,
    )
}

/// Execute a CSV import with the given column mappings.
pub fn prepare_csv_import(
    source: &ImportSource,
    mappings: &[ColumnMapping],
    options: ImportOptions,
) -> Result<PreparedImport, ImportError> {
    let (rows, _) = load_csv_rows(source)?;
    let has_header = options
        .has_header
        .unwrap_or_else(|| crate::detect::detect_has_header(&rows));
    Ok(prepare_table_import(rows, mappings, has_header))
}

pub(crate) fn load_csv_rows(source: &ImportSource) -> Result<(Vec<Vec<String>>, u8), ImportError> {
    let text = decode_to_utf8(&source.data).map_err(ImportError::EncodingError)?;

    if text.trim().is_empty() {
        return Err(ImportError::EmptyFile);
    }

    let (text, delimiter) = csv_text_and_delimiter(&text);
    let rows = parse_csv_text(text, delimiter)?;

    if rows.is_empty() {
        return Err(ImportError::EmptyFile);
    }

    Ok((rows, delimiter))
}

fn csv_text_and_delimiter(text: &str) -> (&str, u8) {
    let Some(first_line_end) = text.find(['\n', '\r']) else {
        return (text, detect_delimiter(text));
    };
    let first_line = text[..first_line_end].trim();
    let Some(delimiter_text) = first_line.strip_prefix("sep=") else {
        return (text, detect_delimiter(text));
    };
    let mut chars = delimiter_text.chars();
    let Some(delimiter) = chars.next() else {
        return (text, detect_delimiter(text));
    };
    if chars.next().is_some() || !matches!(delimiter, ',' | ';' | '\t') {
        return (text, detect_delimiter(text));
    }

    let body_start = if text[first_line_end..].starts_with("\r\n") {
        first_line_end + 2
    } else {
        first_line_end + 1
    };
    let delimiter = match delimiter {
        ',' => b',',
        ';' => b';',
        '\t' => b'\t',
        _ => return (text, detect_delimiter(text)),
    };
    (&text[body_start..], delimiter)
}

/// Parse raw CSV text into rows of cells.
fn parse_csv_text(text: &str, delimiter: u8) -> Result<Vec<Vec<String>>, ImportError> {
    let delim = char::from(delimiter);
    let mut rows = Vec::new();
    let mut current_row = Vec::new();
    let mut current_cell = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_quotes {
            match ch {
                '"' => {
                    if chars.peek() == Some(&'"') {
                        let _ = chars.next();
                        current_cell.push('"');
                    } else {
                        in_quotes = false;
                    }
                }
                _ => current_cell.push(ch),
            }
            continue;
        }

        if ch == '"' && current_cell.trim().is_empty() {
            current_cell.clear();
            in_quotes = true;
            continue;
        }

        if ch == delim {
            push_cell(&mut current_row, &mut current_cell);
            continue;
        }

        if ch == '\n' || ch == '\r' {
            if ch == '\r' && chars.peek() == Some(&'\n') {
                let _ = chars.next();
            }
            push_cell(&mut current_row, &mut current_cell);
            push_row(&mut rows, &mut current_row);
            continue;
        }

        current_cell.push(ch);
    }

    if in_quotes {
        return Err(ImportError::ParseError(
            "unterminated quoted CSV field".to_string(),
        ));
    }

    if !current_cell.is_empty() || !current_row.is_empty() {
        push_cell(&mut current_row, &mut current_cell);
        push_row(&mut rows, &mut current_row);
    }

    Ok(rows)
}

fn push_cell(row: &mut Vec<String>, cell: &mut String) {
    row.push(cell.trim().to_string());
    cell.clear();
}

fn push_row(rows: &mut Vec<Vec<String>>, row: &mut Vec<String>) {
    if !row.iter().all(String::is_empty) {
        rows.push(std::mem::take(row));
    } else {
        row.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ContactField, ImportSource, MappingConfidence};

    fn make_source(csv_text: &str) -> ImportSource {
        ImportSource::with_format("test.csv", csv_text.as_bytes().to_vec(), ImportFormat::Csv)
    }

    #[test]
    fn parse_basic_csv() {
        let source = make_source(
            "Name,Email,Phone\nAlice,alice@test.com,555-1234\nBob,bob@test.com,555-5678\n",
        );
        let preview = preview_csv(&source, ImportOptions::default()).expect("should parse");
        assert_eq!(preview.headers, vec!["Name", "Email", "Phone"]);
        assert_eq!(preview.rows.len(), 2);
        assert_eq!(preview.total_rows, 2);
        assert!(preview.has_header);
        assert_eq!(preview.stats.importable, 2);
    }

    #[test]
    fn parse_csv_quoted_fields() {
        let source = make_source("Name,Email\n\"Smith, Alice\",alice@test.com\n");
        let preview = preview_csv(&source, ImportOptions::default()).expect("should parse");
        assert_eq!(preview.rows[0].cells[0], "Smith, Alice");
    }

    #[test]
    fn parse_csv_carriage_return_line_endings() {
        let source = make_source("Name,Email\rAlice,alice@test.com\r");
        let preview = preview_csv(&source, ImportOptions::default()).expect("should parse");
        assert_eq!(preview.total_rows, 1);
        assert_eq!(preview.rows[0].cells[0], "Alice");
    }

    #[test]
    fn parse_csv_no_header_detects_email_by_content() {
        let source =
            make_source("alice@test.com,Alice,+1-555-1234\nbob@test.com,Bob,+1-555-5678\n");
        let preview = preview_csv(&source, ImportOptions::default()).expect("should parse");
        assert!(!preview.has_header);
        assert_eq!(preview.total_rows, 2);
        assert_eq!(preview.mappings[0].target_field, ContactField::Email);
        assert_eq!(preview.stats.importable, 2);
    }

    #[test]
    fn parse_excel_sep_directive() {
        let source = make_source("sep=;\nName;Email\nAlice;alice@test.com\n");
        let preview = preview_csv(&source, ImportOptions::default()).expect("should parse");
        assert_eq!(preview.delimiter, Some(b';'));
        assert_eq!(preview.headers, vec!["Name", "Email"]);
        assert_eq!(preview.total_rows, 1);
    }

    #[test]
    fn prepare_csv_import_returns_prepared_batch() {
        let source = make_source("Name,Email\nAlice, Alice@Test.COM \nBad,not-an-email\n");
        let mappings = vec![
            ColumnMapping {
                source_index: 0,
                source_column: "Name".to_string(),
                target_field: ContactField::DisplayName,
                confidence: MappingConfidence::High,
            },
            ColumnMapping {
                source_index: 1,
                source_column: "Email".to_string(),
                target_field: ContactField::Email,
                confidence: MappingConfidence::High,
            },
        ];
        let options = ImportOptions::default().with_header(true);
        let prepared = prepare_csv_import(&source, &mappings, options).expect("should prepare");
        assert_eq!(prepared.contacts.len(), 1);
        assert_eq!(
            prepared.contacts[0].email.as_deref(),
            Some("alice@test.com")
        );
        assert_eq!(prepared.stats.skipped_invalid_email, 1);
    }

    #[test]
    fn unterminated_quote_is_error() {
        let source = make_source("Name,Email\n\"Alice,alice@test.com\n");
        assert!(preview_csv(&source, ImportOptions::default()).is_err());
    }
}
