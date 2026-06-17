use crate::types::{
    ColumnMapping, ContactField, ImportRowStatus, MappingConfidence, is_valid_email,
};

/// Auto-detect column mappings using headers first, then cell content.
///
/// When a file has no header row, synthetic headers like "Column 1" are
/// intentionally ignored and content sniffing does the useful work.
pub fn auto_detect_mappings(
    headers: &[String],
    sample_rows: &[Vec<String>],
    has_header: bool,
) -> Vec<ColumnMapping> {
    let mut mappings = Vec::with_capacity(headers.len());
    let mut used_fields: Vec<ContactField> = Vec::new();

    for (index, header) in headers.iter().enumerate() {
        let detected = if has_header {
            detect_from_header(header, &used_fields)
        } else {
            None
        }
        .or_else(|| detect_from_content(index, sample_rows, &used_fields));

        let (target_field, confidence) =
            detected.unwrap_or((ContactField::Ignore, MappingConfidence::None));
        if target_field != ContactField::Ignore {
            used_fields.push(target_field);
        }

        mappings.push(ColumnMapping {
            source_index: index,
            source_column: header.clone(),
            target_field,
            confidence,
        });
    }

    mappings
}

fn detect_from_header(
    header: &str,
    used: &[ContactField],
) -> Option<(ContactField, MappingConfidence)> {
    let normalized = normalize_header(header);

    // FirstName/LastName must precede DisplayName: NAME_PATTERNS contains
    // generic substrings like "name" and "navn".
    let candidates = [
        (ContactField::Email, EMAIL_PATTERNS),
        (ContactField::FirstName, FIRST_NAME_PATTERNS),
        (ContactField::LastName, LAST_NAME_PATTERNS),
        (ContactField::DisplayName, NAME_PATTERNS),
        (ContactField::Phone, PHONE_PATTERNS),
        (ContactField::Company, COMPANY_PATTERNS),
        (ContactField::Email2, EMAIL2_PATTERNS),
        (ContactField::Notes, NOTES_PATTERNS),
        (ContactField::Group, GROUP_PATTERNS),
    ];

    for (field, patterns) in candidates {
        if used.contains(&field) {
            if field == ContactField::Email
                && !used.contains(&ContactField::Email2)
                && patterns.iter().any(|p| matches_pattern(&normalized, p))
            {
                return Some((ContactField::Email2, MappingConfidence::High));
            }
            continue;
        }
        if patterns.iter().any(|p| matches_pattern(&normalized, p)) {
            return Some((field, MappingConfidence::High));
        }
    }

    None
}

fn detect_from_content(
    index: usize,
    rows: &[Vec<String>],
    used: &[ContactField],
) -> Option<(ContactField, MappingConfidence)> {
    let mut non_empty = 0usize;
    let mut emailish = 0usize;
    let mut phoneish = 0usize;
    let mut groupish = 0usize;

    for row in rows {
        let Some(value) = row.get(index).map(|s| s.trim()).filter(|s| !s.is_empty()) else {
            continue;
        };
        non_empty += 1;
        if is_valid_email(value) || !crate::recipient_parser::parse_recipient_list(value).is_empty()
        {
            emailish += 1;
        }
        if looks_like_phone(value) {
            phoneish += 1;
        }
        if looks_like_group_list(value) {
            groupish += 1;
        }
    }

    if non_empty == 0 {
        return None;
    }

    if emailish * 2 >= non_empty && !used.contains(&ContactField::Email) {
        return Some((ContactField::Email, MappingConfidence::Medium));
    }
    if emailish * 2 >= non_empty && !used.contains(&ContactField::Email2) {
        return Some((ContactField::Email2, MappingConfidence::Medium));
    }
    if phoneish * 2 >= non_empty && !used.contains(&ContactField::Phone) {
        return Some((ContactField::Phone, MappingConfidence::Low));
    }
    if groupish * 2 >= non_empty && !used.contains(&ContactField::Group) {
        return Some((ContactField::Group, MappingConfidence::Low));
    }

    None
}

fn normalize_header(header: &str) -> String {
    header
        .trim()
        .to_lowercase()
        .replace(['_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn matches_pattern(normalized: &str, pattern: &str) -> bool {
    if normalized == pattern {
        return true;
    }

    let normalized_tokens: Vec<&str> = normalized.split_whitespace().collect();
    let pattern_tokens: Vec<&str> = pattern.split_whitespace().collect();

    if pattern_tokens.len() == 1 {
        return normalized_tokens.contains(&pattern);
    }

    normalized_tokens
        .windows(pattern_tokens.len())
        .any(|window| window == pattern_tokens.as_slice())
}

fn looks_like_phone(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.len() < 5 {
        return false;
    }
    let digit_count = trimmed.chars().filter(char::is_ascii_digit).count();
    if digit_count < 5 {
        return false;
    }
    let allowed_count = trimmed
        .chars()
        .filter(|ch| {
            ch.is_ascii_digit()
                || matches!(ch, '+' | '-' | '(' | ')' | ' ' | '.' | '\u{00A0}' | '/')
        })
        .count();
    allowed_count == trimmed.chars().count()
}

fn looks_like_group_list(value: &str) -> bool {
    value.contains(';') || value.contains('|')
}

pub(crate) fn row_status(contact: &crate::types::ImportedContact) -> ImportRowStatus {
    let Some(email) = contact.normalized_email() else {
        return ImportRowStatus::MissingEmail;
    };
    if is_valid_email(&email) {
        ImportRowStatus::Ready
    } else {
        ImportRowStatus::InvalidEmail
    }
}

pub(crate) fn row_status_with_seen(
    contact: &crate::types::ImportedContact,
    seen_emails: &mut std::collections::HashSet<String>,
) -> ImportRowStatus {
    let status = row_status(contact);
    if status != ImportRowStatus::Ready {
        return status;
    }

    let Some(email) = contact.normalized_email() else {
        return ImportRowStatus::MissingEmail;
    };
    if seen_emails.insert(email) {
        ImportRowStatus::Ready
    } else {
        ImportRowStatus::DuplicateEmail
    }
}

const EMAIL_PATTERNS: &[&str] = &[
    "email",
    "e mail",
    "email address",
    "e mail address",
    "mail",
    "e post",
    "epost",
    "correo",
];

const EMAIL2_PATTERNS: &[&str] = &[
    "email 2",
    "email2",
    "e mail 2",
    "secondary email",
    "other email",
    "home email",
    "work email",
];

const NAME_PATTERNS: &[&str] = &[
    "name",
    "full name",
    "display name",
    "displayname",
    "contact",
    "contact name",
    "navn",
    "nombre",
    "nom",
];

const FIRST_NAME_PATTERNS: &[&str] = &[
    "first name",
    "firstname",
    "first",
    "given name",
    "given",
    "fornavn",
    "prenom",
    "vorname",
];

const LAST_NAME_PATTERNS: &[&str] = &[
    "last name",
    "lastname",
    "last",
    "surname",
    "family name",
    "family",
    "etternavn",
    "nom de famille",
    "nachname",
];

const PHONE_PATTERNS: &[&str] = &[
    "phone",
    "telephone",
    "tel",
    "mobile",
    "cell",
    "telefon",
    "telefono",
    "phone number",
];

const COMPANY_PATTERNS: &[&str] = &[
    "company",
    "organization",
    "organisation",
    "org",
    "firma",
    "empresa",
    "societe",
    "work",
    "employer",
];

const NOTES_PATTERNS: &[&str] = &[
    "notes",
    "note",
    "comments",
    "comment",
    "description",
    "memo",
    "notat",
];

const GROUP_PATTERNS: &[&str] = &[
    "group",
    "groups",
    "category",
    "categories",
    "list",
    "tag",
    "tags",
    "gruppe",
    "label",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_detect_common_headers() {
        let headers = vec![
            "Name".to_string(),
            "Email".to_string(),
            "Phone".to_string(),
            "Company".to_string(),
        ];
        let mappings = auto_detect_mappings(&headers, &[], true);
        assert_eq!(mappings[0].target_field, ContactField::DisplayName);
        assert_eq!(mappings[1].target_field, ContactField::Email);
        assert_eq!(mappings[2].target_field, ContactField::Phone);
        assert_eq!(mappings[3].target_field, ContactField::Company);
    }

    #[test]
    fn auto_detect_first_last_name() {
        let headers = vec![
            "First Name".to_string(),
            "Last Name".to_string(),
            "E-Mail".to_string(),
        ];
        let mappings = auto_detect_mappings(&headers, &[], true);
        assert_eq!(mappings[0].target_field, ContactField::FirstName);
        assert_eq!(mappings[1].target_field, ContactField::LastName);
        assert_eq!(mappings[2].target_field, ContactField::Email);
    }

    #[test]
    fn content_detects_no_header_email_and_phone() {
        let headers = vec![
            "Column 1".to_string(),
            "Column 2".to_string(),
            "Column 3".to_string(),
        ];
        let rows = vec![
            vec![
                "alice@example.com".to_string(),
                "Alice".to_string(),
                "+47 123 45 678".to_string(),
            ],
            vec![
                "bob@example.com".to_string(),
                "Bob".to_string(),
                "+47 555 12 345".to_string(),
            ],
        ];
        let mappings = auto_detect_mappings(&headers, &rows, false);
        assert_eq!(mappings[0].target_field, ContactField::Email);
        assert_eq!(mappings[1].target_field, ContactField::Ignore);
        assert_eq!(mappings[2].target_field, ContactField::Phone);
    }

    #[test]
    fn content_detects_email_inside_display_address() {
        let headers = vec!["Column 1".to_string(), "Column 2".to_string()];
        let rows = vec![
            vec![
                "Alice Smith <alice@example.com>".to_string(),
                "Engineering".to_string(),
            ],
            vec![
                "Bob Jones <bob@example.com>".to_string(),
                "Sales".to_string(),
            ],
        ];

        let mappings = auto_detect_mappings(&headers, &rows, false);
        assert_eq!(mappings[0].target_field, ContactField::Email);
    }

    #[test]
    fn header_matching_avoids_substring_false_positives() {
        let headers = vec!["Mailing Address".to_string(), "Email".to_string()];

        let mappings = auto_detect_mappings(&headers, &[], true);
        assert_eq!(mappings[0].target_field, ContactField::Ignore);
        assert_eq!(mappings[1].target_field, ContactField::Email);
    }

    #[test]
    fn norwegian_headers() {
        let headers = vec![
            "Fornavn".to_string(),
            "Etternavn".to_string(),
            "E-post".to_string(),
            "Telefon".to_string(),
            "Firma".to_string(),
        ];
        let mappings = auto_detect_mappings(&headers, &[], true);
        assert_eq!(mappings[0].target_field, ContactField::FirstName);
        assert_eq!(mappings[1].target_field, ContactField::LastName);
        assert_eq!(mappings[2].target_field, ContactField::Email);
        assert_eq!(mappings[3].target_field, ContactField::Phone);
        assert_eq!(mappings[4].target_field, ContactField::Company);
    }
}
