use calcard::vcard::{VCard, VCardProperty, VCardValue};

use crate::mapping::row_status_with_seen;
use crate::types::{
    ContactPreview, ContactPreviewRow, ImportError, ImportStats, ImportedContact, PreparedImport,
    SkippedImportRow, clean_text, normalize_email,
};

/// Build a vCard preview. vCard files skip column mapping because the
/// format is structured.
pub fn preview_vcf(data: &[u8], preview_rows: usize) -> Result<ContactPreview, ImportError> {
    let contacts = parse_vcf_contacts(data)?;
    let total_rows = contacts.len();
    let mut stats = ImportStats::default();
    let mut rows = Vec::new();
    let mut seen_emails = std::collections::HashSet::new();
    for (index, contact) in contacts.into_iter().enumerate() {
        let status = row_status_with_seen(&contact, &mut seen_emails);
        stats.record(status);
        if rows.len() < preview_rows {
            rows.push(ContactPreviewRow {
                row_number: index + 1,
                contact,
                status,
            });
        }
    }

    Ok(ContactPreview {
        rows,
        total_rows,
        stats,
    })
}

/// Prepare vCard contacts for import.
pub fn prepare_vcf_import(data: &[u8]) -> Result<PreparedImport, ImportError> {
    let contacts = parse_vcf_contacts(data)?;
    let mut prepared = PreparedImport::default();
    let mut seen_emails = std::collections::HashSet::new();

    for (index, contact) in contacts.into_iter().enumerate() {
        let row_number = index + 1;
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

    Ok(prepared)
}

/// Parse all contacts from a .vcf file, including contacts without email.
pub fn parse_vcf_contacts(data: &[u8]) -> Result<Vec<ImportedContact>, ImportError> {
    let text = crate::detect::decode_to_utf8(data).map_err(ImportError::EncodingError)?;

    if text.trim().is_empty() {
        return Err(ImportError::EmptyFile);
    }

    let vcard_blocks = split_vcards(&text);

    if vcard_blocks.is_empty() {
        return Err(ImportError::ParseError("No vCard entries found".to_string()));
    }

    let mut contacts = Vec::with_capacity(vcard_blocks.len());
    let mut failures = 0usize;

    for block in &vcard_blocks {
        match parse_single_vcard(block) {
            Ok(contact) => contacts.push(contact),
            Err(_) => failures += 1,
        }
    }

    if contacts.is_empty() {
        return Err(ImportError::ParseError(format!(
            "No valid vCards found ({failures} failed)"
        )));
    }

    Ok(contacts)
}

fn split_vcards(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current_block = String::new();
    let mut in_vcard = false;

    for line in text.lines() {
        let trimmed = line.trim();

        if trimmed.eq_ignore_ascii_case("BEGIN:VCARD") {
            in_vcard = true;
            current_block.clear();
            current_block.push_str(line);
            current_block.push_str("\r\n");
            continue;
        }

        if trimmed.eq_ignore_ascii_case("END:VCARD") {
            current_block.push_str(line);
            current_block.push_str("\r\n");
            blocks.push(current_block.clone());
            current_block.clear();
            in_vcard = false;
            continue;
        }

        if in_vcard {
            current_block.push_str(line);
            current_block.push_str("\r\n");
        }
    }

    blocks
}

fn parse_single_vcard(vcard_text: &str) -> Result<ImportedContact, String> {
    let vcard = VCard::parse(vcard_text).map_err(|e| format!("vCard parse error: {e:?}"))?;

    let display_name = first_text(&vcard, &VCardProperty::Fn).and_then(|s| clean_text(&s));
    let email = first_text(&vcard, &VCardProperty::Email)
        .and_then(|s| clean_text(&s))
        .map(|s| normalize_email(&s));
    let email2 = vcard
        .properties(&VCardProperty::Email)
        .nth(1)
        .and_then(|entry| entry.values.first())
        .and_then(extract_text_value)
        .and_then(|s| clean_text(&s))
        .map(|s| normalize_email(&s));
    let phone = first_text(&vcard, &VCardProperty::Tel).and_then(|s| clean_text(&s));
    let company = first_text(&vcard, &VCardProperty::Org).and_then(|s| clean_text(&s));
    let notes = first_text(&vcard, &VCardProperty::Note).and_then(|s| clean_text(&s));
    let groups = extract_categories(&vcard);
    let (first_name, last_name) = extract_structured_name(&vcard);

    Ok(ImportedContact {
        display_name,
        first_name,
        last_name,
        email,
        email2,
        phone,
        company,
        notes,
        groups,
    })
}

fn first_text(vcard: &VCard, property: &VCardProperty) -> Option<String> {
    vcard
        .property(property)
        .and_then(|entry| entry.values.first())
        .and_then(extract_text_value)
}

fn extract_structured_name(vcard: &VCard) -> (Option<String>, Option<String>) {
    let Some(entry) = vcard.property(&VCardProperty::N) else {
        return (None, None);
    };

    let values = &entry.values;

    let last_name = values
        .first()
        .and_then(extract_text_value)
        .and_then(|s| clean_text(&s));

    let first_name = values
        .get(1)
        .and_then(extract_text_value)
        .and_then(|s| clean_text(&s));

    (first_name, last_name)
}

fn extract_categories(vcard: &VCard) -> Vec<String> {
    let mut groups = Vec::new();
    for entry in vcard.properties(&VCardProperty::Categories) {
        for value in &entry.values {
            if let Some(text) = extract_text_value(value) {
                groups.extend(
                    text.split([';', ',', '|'])
                        .filter_map(clean_text),
                );
            }
        }
    }
    groups
}

fn extract_text_value(value: &VCardValue) -> Option<String> {
    match value {
        VCardValue::Text(s) => clean_text(s),
        VCardValue::Component(parts) => clean_text(&parts.join(";")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_vcard_contact() {
        let data = b"BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Alice Smith\r\nEMAIL:Alice@Test.COM\r\nTEL:+1-555-0100\r\nORG:Acme Corp\r\nNOTE:A note\r\nCATEGORIES:Engineering,Project X\r\nEND:VCARD\r\n";
        let contacts = parse_vcf_contacts(data).expect("should parse");
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].display_name.as_deref(), Some("Alice Smith"));
        assert_eq!(contacts[0].email.as_deref(), Some("alice@test.com"));
        assert_eq!(contacts[0].phone.as_deref(), Some("+1-555-0100"));
        assert_eq!(contacts[0].company.as_deref(), Some("Acme Corp"));
        assert_eq!(contacts[0].notes.as_deref(), Some("A note"));
        assert_eq!(
            contacts[0].groups,
            vec!["Engineering".to_string(), "Project X".to_string()]
        );
    }

    #[test]
    fn preview_counts_invalid_rows() {
        let data = b"BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Alice\r\nEMAIL:alice@test.com\r\nEND:VCARD\r\nBEGIN:VCARD\r\nVERSION:3.0\r\nFN:No Email\r\nEND:VCARD\r\n";
        let preview = preview_vcf(data, 10).expect("should preview");
        assert_eq!(preview.total_rows, 2);
        assert_eq!(preview.stats.importable, 1);
        assert_eq!(preview.stats.skipped_no_email, 1);
    }

    #[test]
    fn prepare_vcf_returns_only_importable_contacts() {
        let data = b"BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Alice\r\nEMAIL:alice@test.com\r\nEND:VCARD\r\nBEGIN:VCARD\r\nVERSION:3.0\r\nFN:Bad\r\nEMAIL:not-an-email\r\nEND:VCARD\r\n";
        let prepared = prepare_vcf_import(data).expect("should prepare");
        assert_eq!(prepared.contacts.len(), 1);
        assert_eq!(prepared.stats.skipped_invalid_email, 1);
    }

    #[test]
    fn prepare_vcf_skips_duplicate_source_emails() {
        let data = b"BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Alice\r\nEMAIL:alice@test.com\r\nEND:VCARD\r\nBEGIN:VCARD\r\nVERSION:3.0\r\nFN:Alice Again\r\nEMAIL:ALICE@test.com\r\nEND:VCARD\r\n";
        let prepared = prepare_vcf_import(data).expect("should prepare");
        assert_eq!(prepared.contacts.len(), 1);
        assert_eq!(prepared.stats.skipped_duplicate, 1);
        assert_eq!(prepared.skipped_rows[0].status, crate::types::ImportRowStatus::DuplicateEmail);
    }
}
