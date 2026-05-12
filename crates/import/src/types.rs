/// The source file for an import operation.
#[derive(Debug, Clone)]
pub struct ImportSource {
    /// Detected or caller-supplied format.
    pub format: ImportFormat,
    /// Raw file bytes.
    pub data: Vec<u8>,
    /// Original filename for display and format detection.
    pub filename: String,
}

impl ImportSource {
    /// Detect the import format from filename and content.
    pub fn detect(filename: impl Into<String>, data: Vec<u8>) -> Result<Self, ImportError> {
        let filename = filename.into();
        let format = crate::detect::detect_format(&filename, &data)?;
        Ok(Self {
            format,
            data,
            filename,
        })
    }

    /// Build a source with an explicit format.
    pub fn with_format(filename: impl Into<String>, data: Vec<u8>, format: ImportFormat) -> Self {
        Self {
            format,
            data,
            filename: filename.into(),
        }
    }
}

/// Supported import formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportFormat {
    Csv,
    Xlsx,
    Vcf,
}

impl ImportFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Csv => "CSV",
            Self::Xlsx => "XLSX",
            Self::Vcf => "vCard",
        }
    }
}

/// Options that affect table parsing and preview generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportOptions {
    /// Maximum preview rows to materialize.
    pub preview_rows: usize,
    /// Header override. `None` lets the parser infer it.
    pub has_header: Option<bool>,
    /// XLSX sheet index. Ignored for CSV and vCard.
    pub sheet_index: Option<usize>,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            preview_rows: 20,
            has_header: None,
            sheet_index: None,
        }
    }
}

impl ImportOptions {
    pub fn with_header(mut self, has_header: bool) -> Self {
        self.has_header = Some(has_header);
        self
    }
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
    /// All fields available as mapping targets, including Ignore.
    pub const ALL: &[ContactField] = &[
        ContactField::Ignore,
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

    /// Assignable fields excluding Ignore.
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

/// Confidence attached to an automatically detected column mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappingConfidence {
    None,
    Low,
    Medium,
    High,
}

/// A mapping from a source column to a target contact field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnMapping {
    /// Index of the source column, zero-based.
    pub source_index: usize,
    /// Original column header text or synthetic column name.
    pub source_column: String,
    /// Target contact field.
    pub target_field: ContactField,
    /// Confidence for auto-detected mappings. User mappings can leave this alone.
    pub confidence: MappingConfidence,
}

impl ColumnMapping {
    pub fn ignored(source_index: usize, source_column: impl Into<String>) -> Self {
        Self {
            source_index,
            source_column: source_column.into(),
            target_field: ContactField::Ignore,
            confidence: MappingConfidence::None,
        }
    }
}

/// Parsed preview for any supported source.
#[derive(Debug, Clone)]
pub enum ImportPreview {
    Table(TablePreview),
    Contacts(ContactPreview),
}

/// Preview for CSV/XLSX table-shaped imports.
#[derive(Debug, Clone)]
pub struct TablePreview {
    pub format: ImportFormat,
    /// Column headers, detected or synthetic like "Column 1".
    pub headers: Vec<String>,
    /// First N parsed data rows with importability status.
    pub rows: Vec<ImportPreviewRow>,
    /// Total data row count excluding the header row.
    pub total_rows: usize,
    /// Whether the first row is treated as a header.
    pub has_header: bool,
    /// Detected delimiter for CSV.
    pub delimiter: Option<u8>,
    /// XLSX sheets, empty for CSV.
    pub sheets: Vec<SheetInfo>,
    /// Selected sheet index for XLSX.
    pub selected_sheet: Option<usize>,
    /// Suggested mappings for the current headers and sample rows.
    pub mappings: Vec<ColumnMapping>,
    /// Importability totals for all parsed data rows.
    pub stats: ImportStats,
}

/// Preview for vCard imports.
#[derive(Debug, Clone)]
pub struct ContactPreview {
    pub rows: Vec<ContactPreviewRow>,
    pub total_rows: usize,
    pub stats: ImportStats,
}

/// A worksheet visible to the importer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetInfo {
    pub index: usize,
    pub name: String,
}

/// A table preview row.
#[derive(Debug, Clone)]
pub struct ImportPreviewRow {
    /// One-based row number in the source file.
    pub row_number: usize,
    /// Raw parsed cells after minimal trimming.
    pub cells: Vec<String>,
    /// Contact produced by current mappings.
    pub contact: ImportedContact,
    /// Whether this row can be imported.
    pub status: ImportRowStatus,
}

/// A structured contact preview row.
#[derive(Debug, Clone)]
pub struct ContactPreviewRow {
    /// One-based contact number in the source file.
    pub row_number: usize,
    pub contact: ImportedContact,
    pub status: ImportRowStatus,
}

/// Why a row will or will not be imported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportRowStatus {
    Ready,
    MissingEmail,
    InvalidEmail,
    DuplicateEmail,
}

impl ImportRowStatus {
    pub fn is_importable(self) -> bool {
        matches!(self, Self::Ready)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "Ready",
            Self::MissingEmail => "No email",
            Self::InvalidEmail => "Invalid email",
            Self::DuplicateEmail => "Duplicate email",
        }
    }
}

/// Importability totals.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportStats {
    pub total_rows: usize,
    pub importable: usize,
    pub skipped_no_email: usize,
    pub skipped_invalid_email: usize,
    pub skipped_duplicate: usize,
}

impl ImportStats {
    pub fn skipped_total(self) -> usize {
        self.skipped_no_email + self.skipped_invalid_email + self.skipped_duplicate
    }

    pub(crate) fn record(&mut self, status: ImportRowStatus) {
        self.total_rows += 1;
        match status {
            ImportRowStatus::Ready => self.importable += 1,
            ImportRowStatus::MissingEmail => self.skipped_no_email += 1,
            ImportRowStatus::InvalidEmail => self.skipped_invalid_email += 1,
            ImportRowStatus::DuplicateEmail => self.skipped_duplicate += 1,
        }
    }
}

/// Contacts ready for DB insertion plus rows skipped by the import crate.
#[derive(Debug, Clone, Default)]
pub struct PreparedImport {
    pub contacts: Vec<ImportedContact>,
    pub skipped_rows: Vec<SkippedImportRow>,
    pub stats: ImportStats,
}

/// A row skipped before the app/database layer sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkippedImportRow {
    pub row_number: usize,
    pub status: ImportRowStatus,
}

/// A single imported contact ready for DB insertion.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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

    /// Normalize the primary email address: strip whitespace and lowercase.
    pub fn normalized_email(&self) -> Option<String> {
        self.email
            .as_deref()
            .map(normalize_email)
            .filter(|e| !e.is_empty())
    }

    /// Whether this contact has an email address that can be imported.
    pub fn has_valid_email(&self) -> bool {
        self.normalized_email()
            .as_deref()
            .is_some_and(is_valid_email)
    }
}

/// Normalize an email address by removing whitespace and lowercasing.
pub fn normalize_email(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Conservative email validity check for import gating.
pub fn is_valid_email(value: &str) -> bool {
    let normalized = normalize_email(value);
    let Some((local, domain)) = normalized.split_once('@') else {
        return false;
    };
    !local.is_empty() && !domain.is_empty() && !domain.contains('@')
}

pub(crate) fn clean_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Errors that can occur during import.
#[derive(Debug, Clone)]
pub enum ImportError {
    /// The file could not be decoded.
    EncodingError(String),
    /// The file format is not supported.
    UnsupportedFormat(String),
    /// The file format is supported but could not be parsed.
    ParseError(String),
    /// No data rows found.
    EmptyFile,
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::EncodingError(msg) => write!(f, "Encoding error: {msg}"),
            ImportError::UnsupportedFormat(msg) => write!(f, "Unsupported format: {msg}"),
            ImportError::ParseError(msg) => write!(f, "Parse error: {msg}"),
            ImportError::EmptyFile => write!(f, "File contains no data"),
        }
    }
}

impl std::error::Error for ImportError {}
