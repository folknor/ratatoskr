use super::contacts::ImportContactField;

/// The current step of the import wizard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportStep {
    /// Waiting for file selection.
    FileSelect,
    /// Column mapping + preview (CSV only).
    Mapping,
    /// Preview for vCard imports (no mapping needed).
    VcfPreview,
    /// Import is running.
    Importing,
    /// Import complete - show summary.
    Summary,
}

/// Result of an import operation.
#[derive(Debug, Clone)]
pub struct ImportResult {
    pub imported: usize,
    pub skipped_no_email: usize,
    pub skipped_invalid_email: usize,
    pub skipped_duplicate: usize,
    pub updated: usize,
    pub groups_created: usize,
}

/// State for the contact import wizard.
#[derive(Debug, Clone)]
pub struct ImportWizardState {
    pub step: ImportStep,
    /// Selected file path (display only).
    pub file_path: Option<String>,
    /// Parsed import source.
    pub source: Option<import::ImportSource>,
    /// Preview data for the selected source.
    pub preview: Option<import::ImportPreview>,
    /// Column mappings (one per header column).
    pub mappings: Vec<ImportContactField>,
    /// Whether the first row is treated as a header.
    pub has_header: bool,
    /// Target account for import.
    pub account_id: Option<String>,
    /// Whether to update existing contacts on duplicate email.
    pub update_existing: bool,
    /// Import result after completion.
    pub result: Option<ImportResult>,
}

impl ImportWizardState {
    pub fn new() -> Self {
        Self {
            step: ImportStep::FileSelect,
            file_path: None,
            source: None,
            preview: None,
            mappings: Vec::new(),
            has_header: true,
            account_id: None,
            update_existing: false,
            result: None,
        }
    }
}
