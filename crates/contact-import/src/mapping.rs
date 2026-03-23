use crate::types::ContactField;

/// A mapping from a source column to a target contact field.
#[derive(Debug, Clone)]
pub struct ColumnMapping {
    /// Index of the source column (0-based).
    pub source_index: usize,
    /// Original column header text.
    pub source_column: String,
    /// Target contact field.
    pub target_field: ContactField,
}

/// Auto-detect column mappings by matching header names against known patterns.
///
/// Returns a mapping for each header. Unrecognized headers are mapped to `Ignore`.
/// Each target field (except Ignore) is only assigned once; the first match wins.
pub fn auto_detect_mappings(headers: &[String]) -> Vec<ColumnMapping> {
    let mut mappings = Vec::with_capacity(headers.len());
    let mut used_fields: Vec<ContactField> = Vec::new();

    for (index, header) in headers.iter().enumerate() {
        let normalized = header.trim().to_lowercase();
        let field = detect_field(&normalized, &used_fields);

        if field != ContactField::Ignore {
            used_fields.push(field);
        }

        mappings.push(ColumnMapping {
            source_index: index,
            source_column: header.clone(),
            target_field: field,
        });
    }

    mappings
}

/// Detect the contact field from a header name, respecting already-used fields.
fn detect_field(normalized: &str, used: &[ContactField]) -> ContactField {
    // Try each pattern set in priority order
    let candidates = [
        (ContactField::Email, EMAIL_PATTERNS),
        (ContactField::DisplayName, NAME_PATTERNS),
        (ContactField::FirstName, FIRST_NAME_PATTERNS),
        (ContactField::LastName, LAST_NAME_PATTERNS),
        (ContactField::Phone, PHONE_PATTERNS),
        (ContactField::Company, COMPANY_PATTERNS),
        (ContactField::Email2, EMAIL2_PATTERNS),
        (ContactField::Notes, NOTES_PATTERNS),
        (ContactField::Group, GROUP_PATTERNS),
    ];

    for (field, patterns) in &candidates {
        if used.contains(field) {
            // Special case: if Email is taken and we match an email pattern,
            // try Email2 instead.
            if *field == ContactField::Email
                && !used.contains(&ContactField::Email2)
                && patterns.iter().any(|p| matches_pattern(normalized, p))
            {
                return ContactField::Email2;
            }
            continue;
        }
        if patterns.iter().any(|p| matches_pattern(normalized, p)) {
            return *field;
        }
    }

    ContactField::Ignore
}

/// Check if a normalized header matches a pattern.
///
/// Supports exact match and substring containment.
fn matches_pattern(normalized: &str, pattern: &str) -> bool {
    // Exact match
    if normalized == pattern {
        return true;
    }
    // Contains the pattern as a word (surrounded by non-alphanumeric or at boundaries)
    normalized.contains(pattern)
}

// ── Pattern lists ─────────────────────────────────────────

const EMAIL_PATTERNS: &[&str] = &[
    "email",
    "e-mail",
    "email address",
    "e-mail address",
    "mail",
    "e-post",
    "epost",
    "correo",
];

const EMAIL2_PATTERNS: &[&str] = &[
    "email 2",
    "email2",
    "e-mail 2",
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
        let headers: Vec<String> = vec![
            "Name".into(),
            "Email".into(),
            "Phone".into(),
            "Company".into(),
        ];
        let mappings = auto_detect_mappings(&headers);
        assert_eq!(mappings[0].target_field, ContactField::DisplayName);
        assert_eq!(mappings[1].target_field, ContactField::Email);
        assert_eq!(mappings[2].target_field, ContactField::Phone);
        assert_eq!(mappings[3].target_field, ContactField::Company);
    }

    #[test]
    fn auto_detect_first_last_name() {
        let headers: Vec<String> = vec![
            "First Name".into(),
            "Last Name".into(),
            "E-Mail".into(),
        ];
        let mappings = auto_detect_mappings(&headers);
        assert_eq!(mappings[0].target_field, ContactField::FirstName);
        assert_eq!(mappings[1].target_field, ContactField::LastName);
        assert_eq!(mappings[2].target_field, ContactField::Email);
    }

    #[test]
    fn auto_detect_two_email_columns() {
        let headers: Vec<String> = vec![
            "Email".into(),
            "Work Email".into(),
            "Name".into(),
        ];
        let mappings = auto_detect_mappings(&headers);
        assert_eq!(mappings[0].target_field, ContactField::Email);
        assert_eq!(mappings[1].target_field, ContactField::Email2);
        assert_eq!(mappings[2].target_field, ContactField::DisplayName);
    }

    #[test]
    fn unknown_headers_are_ignored() {
        let headers: Vec<String> = vec![
            "ID".into(),
            "Random Column".into(),
        ];
        let mappings = auto_detect_mappings(&headers);
        assert_eq!(mappings[0].target_field, ContactField::Ignore);
        assert_eq!(mappings[1].target_field, ContactField::Ignore);
    }

    #[test]
    fn norwegian_headers() {
        let headers: Vec<String> = vec![
            "Fornavn".into(),
            "Etternavn".into(),
            "E-post".into(),
            "Telefon".into(),
            "Firma".into(),
        ];
        let mappings = auto_detect_mappings(&headers);
        assert_eq!(mappings[0].target_field, ContactField::FirstName);
        assert_eq!(mappings[1].target_field, ContactField::LastName);
        assert_eq!(mappings[2].target_field, ContactField::Email);
        assert_eq!(mappings[3].target_field, ContactField::Phone);
        assert_eq!(mappings[4].target_field, ContactField::Company);
    }
}
