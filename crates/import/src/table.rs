use crate::detect::detect_has_header;
use crate::mapping::{auto_detect_mappings, row_status_with_seen};
use crate::types::{
    ColumnMapping, ContactField, ImportError, ImportFormat, ImportOptions, ImportPreviewRow,
    ImportStats, ImportedContact, PreparedImport, SheetInfo, SkippedImportRow, TablePreview,
    clean_text, normalize_email,
};

pub(crate) fn build_table_preview(
    format: ImportFormat,
    rows: Vec<Vec<String>>,
    delimiter: Option<u8>,
    sheets: Vec<SheetInfo>,
    selected_sheet: Option<usize>,
    options: ImportOptions,
) -> Result<TablePreview, ImportError> {
    if rows.is_empty() {
        return Err(ImportError::EmptyFile);
    }

    let has_header = options
        .has_header
        .unwrap_or_else(|| detect_has_header(&rows));
    let (headers, data_rows, first_data_row_number) = split_headers(rows, has_header);
    let sample_cells: Vec<Vec<String>> = data_rows
        .iter()
        .take(options.preview_rows)
        .cloned()
        .collect();
    let mappings = auto_detect_mappings(&headers, &sample_cells, has_header);
    let preview_rows = preview_rows(
        &data_rows,
        &mappings,
        first_data_row_number,
        options.preview_rows,
    );
    let stats = stats_for_rows(&data_rows, &mappings);
    let total_rows = data_rows.len();

    Ok(TablePreview {
        format,
        headers,
        rows: preview_rows,
        total_rows,
        has_header,
        delimiter,
        sheets,
        selected_sheet,
        mappings,
        stats,
    })
}

pub(crate) fn prepare_table_import(
    rows: Vec<Vec<String>>,
    mappings: &[ColumnMapping],
    has_header: bool,
) -> PreparedImport {
    let (_, data_rows, first_data_row_number) = split_headers(rows, has_header);
    let mut prepared = PreparedImport::default();
    let mut seen_emails = std::collections::HashSet::new();

    for (index, row) in data_rows.iter().enumerate() {
        let row_number = first_data_row_number + index;
        let contact = row_to_contact(row, mappings);
        let status = row_status_with_seen(&contact, &mut seen_emails);
        prepared.stats.record(status);
        if status.is_importable() {
            prepared.contacts.push(contact);
        } else {
            prepared
                .skipped_rows
                .push(SkippedImportRow { row_number, status });
        }
    }

    prepared
}

fn split_headers(
    rows: Vec<Vec<String>>,
    has_header: bool,
) -> (Vec<String>, Vec<Vec<String>>, usize) {
    let col_count = rows.iter().map(Vec::len).max().unwrap_or(0);
    if has_header {
        let mut iter = rows.into_iter();
        let header_row = iter.next().unwrap_or_default();
        let headers = normalize_headers(header_row, col_count);
        let data_rows = iter.collect();
        (headers, data_rows, 2)
    } else {
        let headers = synthetic_headers(col_count);
        (headers, rows, 1)
    }
}

fn normalize_headers(mut header_row: Vec<String>, col_count: usize) -> Vec<String> {
    header_row.resize(col_count, String::new());
    header_row
        .into_iter()
        .enumerate()
        .map(|(index, header)| {
            let trimmed = header.trim();
            if trimmed.is_empty() {
                format!("Column {}", index + 1)
            } else {
                trimmed.to_string()
            }
        })
        .collect()
}

fn synthetic_headers(col_count: usize) -> Vec<String> {
    (1..=col_count).map(|i| format!("Column {i}")).collect()
}

fn preview_rows(
    rows: &[Vec<String>],
    mappings: &[ColumnMapping],
    first_data_row_number: usize,
    limit: usize,
) -> Vec<ImportPreviewRow> {
    rows.iter()
        .take(limit)
        .enumerate()
        .scan(
            std::collections::HashSet::new(),
            |seen_emails, (index, row)| {
                let contact = row_to_contact(row, mappings);
                let status = row_status_with_seen(&contact, seen_emails);
                Some(ImportPreviewRow {
                    row_number: first_data_row_number + index,
                    cells: row.clone(),
                    contact,
                    status,
                })
            },
        )
        .collect()
}

fn stats_for_rows(rows: &[Vec<String>], mappings: &[ColumnMapping]) -> ImportStats {
    let mut stats = ImportStats::default();
    let mut seen_emails = std::collections::HashSet::new();
    for row in rows {
        let contact = row_to_contact(row, mappings);
        stats.record(row_status_with_seen(&contact, &mut seen_emails));
    }
    stats
}

pub(crate) fn row_to_contact(row: &[String], mappings: &[ColumnMapping]) -> ImportedContact {
    let mut contact = ImportedContact::default();

    for mapping in mappings {
        let value = row
            .get(mapping.source_index)
            .and_then(|s| clean_text(s))
            .unwrap_or_default();

        if value.is_empty() {
            continue;
        }

        match mapping.target_field {
            ContactField::DisplayName => contact.display_name = Some(value),
            ContactField::FirstName => contact.first_name = Some(value),
            ContactField::LastName => contact.last_name = Some(value),
            ContactField::Email => {
                assign_email_cell(&mut contact.email, &mut contact.display_name, &value);
            }
            ContactField::Email2 => {
                let mut ignored_display_name = None;
                assign_email_cell(&mut contact.email2, &mut ignored_display_name, &value);
            }
            ContactField::Phone => contact.phone = Some(value),
            ContactField::Company => contact.company = Some(value),
            ContactField::Notes => contact.notes = Some(value),
            ContactField::Group => contact.groups.extend(split_groups(&value)),
            ContactField::Ignore => {}
        }
    }

    contact
}

pub(crate) fn table_contacts_from_rows(
    rows: Vec<Vec<String>>,
    mappings: &[ColumnMapping],
    has_header: bool,
) -> Vec<(usize, ImportedContact)> {
    let (_, data_rows, first_data_row_number) = split_headers(rows, has_header);
    data_rows
        .iter()
        .enumerate()
        .map(|(index, row)| (first_data_row_number + index, row_to_contact(row, mappings)))
        .collect()
}

fn assign_email_cell(
    target_email: &mut Option<String>,
    target_display_name: &mut Option<String>,
    value: &str,
) {
    let parsed = crate::recipient_parser::parse_recipient_list(value);
    if let Some(recipient) = parsed.first() {
        *target_email = Some(recipient.email.clone());
        if target_display_name.is_none() {
            *target_display_name = recipient.display_name.clone();
        }
        return;
    }

    *target_email = Some(normalize_email(value));
}

fn split_groups(value: &str) -> Vec<String> {
    value
        .split([';', ',', '|'])
        .filter_map(clean_text)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ImportRowStatus, MappingConfidence};

    #[test]
    fn prepared_import_keeps_only_importable_contacts() {
        let rows = vec![
            vec!["Name".to_string(), "Email".to_string()],
            vec!["Alice".to_string(), "alice@example.com".to_string()],
            vec!["No Email".to_string(), String::new()],
            vec!["Bad".to_string(), "not-an-email".to_string()],
        ];
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

        let prepared = prepare_table_import(rows, &mappings, true);
        assert_eq!(prepared.contacts.len(), 1);
        assert_eq!(prepared.stats.importable, 1);
        assert_eq!(prepared.stats.skipped_no_email, 1);
        assert_eq!(prepared.stats.skipped_invalid_email, 1);
        assert_eq!(prepared.skipped_rows.len(), 2);
    }

    #[test]
    fn prepared_import_skips_duplicate_source_emails() {
        let rows = vec![
            vec!["Name".to_string(), "Email".to_string()],
            vec!["Alice".to_string(), "alice@example.com".to_string()],
            vec!["Alice Again".to_string(), "ALICE@example.com".to_string()],
            vec!["Bob".to_string(), "bob@example.com".to_string()],
        ];
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

        let prepared = prepare_table_import(rows, &mappings, true);
        assert_eq!(prepared.contacts.len(), 2);
        assert_eq!(prepared.stats.importable, 2);
        assert_eq!(prepared.stats.skipped_duplicate, 1);
        assert_eq!(
            prepared.skipped_rows[0].status,
            ImportRowStatus::DuplicateEmail
        );
    }

    #[test]
    fn row_to_contact_extracts_display_address_email_cell() {
        let row = vec!["Alice Smith <ALICE@example.com>".to_string()];
        let mappings = vec![ColumnMapping {
            source_index: 0,
            source_column: "Email".to_string(),
            target_field: ContactField::Email,
            confidence: MappingConfidence::High,
        }];

        let contact = row_to_contact(&row, &mappings);
        assert_eq!(contact.email.as_deref(), Some("alice@example.com"));
        assert_eq!(contact.display_name.as_deref(), Some("Alice Smith"));
    }

    #[test]
    fn row_to_contact_splits_groups() {
        let row = vec![
            "alice@example.com".to_string(),
            "Eng; Project X|VIP".to_string(),
        ];
        let mappings = vec![
            ColumnMapping {
                source_index: 0,
                source_column: "Email".to_string(),
                target_field: ContactField::Email,
                confidence: MappingConfidence::High,
            },
            ColumnMapping {
                source_index: 1,
                source_column: "Group".to_string(),
                target_field: ContactField::Group,
                confidence: MappingConfidence::High,
            },
        ];

        let contact = row_to_contact(&row, &mappings);
        assert_eq!(
            contact.groups,
            vec![
                "Eng".to_string(),
                "Project X".to_string(),
                "VIP".to_string()
            ]
        );
    }
}
