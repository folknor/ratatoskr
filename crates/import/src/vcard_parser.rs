use calcard::vcard::{VCard, VCardProperty, VCardValue};

use crate::types::{ImportError, ImportedContact};

/// Parse a .vcf file containing one or more vCards.
///
/// A single .vcf file can contain multiple contacts separated by
/// `BEGIN:VCARD` / `END:VCARD` boundaries. This function splits
/// the file and parses each vCard individually.
pub fn parse_vcf(data: &[u8]) -> Result<Vec<ImportedContact>, ImportError> {
    let text = crate::detect::decode_to_utf8(data).map_err(ImportError::EncodingError)?;

    if text.trim().is_empty() {
        return Err(ImportError::EmptyFile);
    }

    let vcard_blocks = split_vcards(&text);

    if vcard_blocks.is_empty() {
        return Err(ImportError::ParseError("No vCard entries found".into()));
    }

    let mut contacts = Vec::with_capacity(vcard_blocks.len());

    for block in &vcard_blocks {
        match parse_single_vcard(block) {
            Ok(contact) => contacts.push(contact),
            Err(e) => {
                // Log but continue — skip unparseable entries
                eprintln!("Skipping unparseable vCard: {e}");
            }
        }
    }

    if contacts.is_empty() {
        return Err(ImportError::ParseError("No valid vCards found".into()));
    }

    Ok(contacts)
}

/// Split a VCF file into individual vCard blocks.
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

/// Parse a single vCard block into an `ImportedContact`.
fn parse_single_vcard(vcard_text: &str) -> Result<ImportedContact, String> {
    let vcard = VCard::parse(vcard_text).map_err(|e| format!("vCard parse error: {e:?}"))?;

    let display_name = vcard
        .property(&VCardProperty::Fn)
        .and_then(|entry| entry.values.first())
        .and_then(|v| v.as_text())
        .filter(|s| !s.is_empty())
        .map(|s| s.trim().to_string());

    let email = vcard
        .property(&VCardProperty::Email)
        .and_then(|entry| entry.values.first())
        .and_then(|v| v.as_text())
        .filter(|s| !s.is_empty())
        .map(|s| s.trim().to_lowercase());

    // Try to get a second email from additional EMAIL properties
    let email2 = vcard
        .properties(&VCardProperty::Email)
        .nth(1)
        .and_then(|entry| entry.values.first())
        .and_then(|v| v.as_text())
        .filter(|s| !s.is_empty())
        .map(|s| s.trim().to_lowercase());

    let phone = vcard
        .property(&VCardProperty::Tel)
        .and_then(|entry| entry.values.first())
        .and_then(|v| v.as_text())
        .filter(|s| !s.is_empty())
        .map(|s| s.trim().to_string());

    let company = vcard
        .property(&VCardProperty::Org)
        .and_then(|entry| entry.values.first())
        .and_then(extract_text_value)
        .filter(|s| !s.is_empty());

    let notes = vcard
        .property(&VCardProperty::Note)
        .and_then(|entry| entry.values.first())
        .and_then(|v| v.as_text())
        .filter(|s| !s.is_empty())
        .map(|s| s.trim().to_string());

    // Extract N (structured name) for first/last if FN is missing
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
        groups: Vec::new(),
    })
}

/// Extract first and last name from the N property.
fn extract_structured_name(vcard: &VCard) -> (Option<String>, Option<String>) {
    let Some(entry) = vcard.property(&VCardProperty::N) else {
        return (None, None);
    };

    // N property has components: family;given;additional;prefix;suffix
    let values = &entry.values;

    let last_name = values
        .first()
        .and_then(extract_text_value)
        .filter(|s| !s.is_empty());

    let first_name = values
        .get(1)
        .and_then(extract_text_value)
        .filter(|s| !s.is_empty());

    (first_name, last_name)
}

/// Extract text from a `VCardValue`, handling both `Text` and `Component` variants.
fn extract_text_value(value: &VCardValue) -> Option<String> {
    match value {
        VCardValue::Text(s) => {
            let trimmed = s.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        }
        VCardValue::Component(parts) => {
            let joined = parts.join(";");
            let trimmed = joined.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_vcard_contact() {
        let data = b"BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Alice Smith\r\nEMAIL:Alice@Test.COM\r\nTEL:+1-555-0100\r\nORG:Acme Corp\r\nNOTE:A note\r\nEND:VCARD\r\n";
        let contacts = parse_vcf(data).expect("should parse");
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].display_name.as_deref(), Some("Alice Smith"));
        assert_eq!(contacts[0].email.as_deref(), Some("alice@test.com"));
        assert_eq!(contacts[0].phone.as_deref(), Some("+1-555-0100"));
        assert_eq!(contacts[0].company.as_deref(), Some("Acme Corp"));
        assert_eq!(contacts[0].notes.as_deref(), Some("A note"));
    }

    #[test]
    fn parse_multiple_vcards() {
        let data = b"BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Alice\r\nEMAIL:alice@test.com\r\nEND:VCARD\r\nBEGIN:VCARD\r\nVERSION:3.0\r\nFN:Bob\r\nEMAIL:bob@test.com\r\nEND:VCARD\r\n";
        let contacts = parse_vcf(data).expect("should parse");
        assert_eq!(contacts.len(), 2);
        assert_eq!(contacts[0].display_name.as_deref(), Some("Alice"));
        assert_eq!(contacts[1].display_name.as_deref(), Some("Bob"));
    }

    #[test]
    fn parse_vcard_no_email() {
        let data = b"BEGIN:VCARD\r\nVERSION:3.0\r\nFN:No Email\r\nTEL:555-1234\r\nEND:VCARD\r\n";
        let contacts = parse_vcf(data).expect("should parse");
        assert_eq!(contacts.len(), 1);
        assert!(contacts[0].email.is_none());
        assert!(!contacts[0].has_valid_email());
    }

    #[test]
    fn empty_vcf_returns_error() {
        assert!(parse_vcf(b"").is_err());
    }

    #[test]
    fn vcard_effective_display_name() {
        let contact = ImportedContact {
            first_name: Some("Alice".into()),
            last_name: Some("Smith".into()),
            ..Default::default()
        };
        assert_eq!(
            contact.effective_display_name().as_deref(),
            Some("Alice Smith")
        );
    }
}
