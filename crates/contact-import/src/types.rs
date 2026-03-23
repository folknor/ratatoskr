/// The source file for an import operation.
#[derive(Debug, Clone)]
pub struct ImportSource {
    /// Detected or overridden format.
    pub format: ImportFormat,
    /// Raw file bytes.
    pub data: Vec<u8>,
    /// Original filename (for display and format detection).
    pub filename: String,
}

/// Supported import formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportFormat {
    Csv,
    Vcf,
}

/// Target field for a column mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactField {
    DisplayName,
    FirstName,
    LastName,
    Email,
    Email2,
    Phone,
    Company,
    Notes,
    Group,
    /// Column should be ignored during import.
    Ignore,
}

impl ContactField {
    /// All fields available as mapping targets (excluding Ignore).
    pub const ASSIGNABLE: &[ContactField] = &[
        ContactField::DisplayName,
        ContactField::FirstName,
        ContactField::LastName,
        ContactField::Email,
        ContactField::Email2,
        ContactField::Phone,
        ContactField::Company,
        ContactField::Notes,
        ContactField::Group,
    ];

    /// Human-readable label for the field.
    pub fn label(self) -> &'static str {
        match self {
            ContactField::DisplayName => "Name",
            ContactField::FirstName => "First Name",
            ContactField::LastName => "Last Name",
            ContactField::Email => "Email",
            ContactField::Email2 => "Email 2",
            ContactField::Phone => "Phone",
            ContactField::Company => "Company",
            ContactField::Notes => "Notes",
            ContactField::Group => "Group",
            ContactField::Ignore => "Ignore",
        }
    }
}

impl std::fmt::Display for ContactField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// A mapping from a source column to a target contact field.
#[derive(Debug, Clone)]
pub struct ImportPreview {
    /// Column headers (detected or synthetic like "Column 1").
    pub headers: Vec<String>,
    /// First N rows of parsed data.
    pub sample_rows: Vec<Vec<String>>,
    /// Total row count (excluding header).
    pub total_rows: usize,
    /// Whether the first row was detected as a header.
    pub has_header: bool,
    /// Detected delimiter (for CSV).
    pub delimiter: Option<u8>,
}

/// A single imported contact ready for DB insertion.
#[derive(Debug, Clone, Default)]
pub struct ImportedContact {
    pub display_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub email: Option<String>,
    pub email2: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub notes: Option<String>,
    pub groups: Vec<String>,
}

impl ImportedContact {
    /// Compute the effective display name.
    ///
    /// If `display_name` is set, use it. Otherwise, combine first + last.
    pub fn effective_display_name(&self) -> Option<String> {
        if let Some(ref name) = self.display_name
            && !name.is_empty()
        {
            return Some(name.clone());
        }
        match (&self.first_name, &self.last_name) {
            (Some(first), Some(last)) if !first.is_empty() && !last.is_empty() => {
                Some(format!("{first} {last}"))
            }
            (Some(first), _) if !first.is_empty() => Some(first.clone()),
            (_, Some(last)) if !last.is_empty() => Some(last.clone()),
            _ => None,
        }
    }

    /// Normalize the email address: trim whitespace and lowercase.
    pub fn normalized_email(&self) -> Option<String> {
        self.email
            .as_ref()
            .map(|e| e.trim().to_lowercase())
            .filter(|e| !e.is_empty())
    }

    /// Whether this contact has a valid email for import.
    pub fn has_valid_email(&self) -> bool {
        self.normalized_email()
            .is_some_and(|e| e.contains('@'))
    }
}

/// Errors that can occur during import.
#[derive(Debug, Clone)]
pub enum ImportError {
    /// The file could not be decoded (encoding issue).
    EncodingError(String),
    /// The file format is not supported or cannot be parsed.
    ParseError(String),
    /// No data rows found.
    EmptyFile,
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::EncodingError(msg) => write!(f, "Encoding error: {msg}"),
            ImportError::ParseError(msg) => write!(f, "Parse error: {msg}"),
            ImportError::EmptyFile => write!(f, "File contains no data"),
        }
    }
}
