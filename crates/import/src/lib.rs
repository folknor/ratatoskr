//! Contact import library for CSV, XLSX, and vCard formats.
//!
//! This crate owns file format detection, encoding conversion, delimiter
//! detection, column mapping heuristics, row-level preview status, and
//! prepared contact output. The app crate owns UI and persistence.

pub mod csv_parser;
mod detect;
mod mapping;
pub mod recipient_parser;
mod table;
mod types;
pub mod vcard_parser;
pub mod xlsx_parser;

pub use mapping::auto_detect_mappings;
pub use recipient_parser::{
    ParsedRecipient, RecipientPastePayload, RecipientPasteResult, RecipientPasteSourceFormat,
    RecipientSkipReason, SkippedRecipient, dedup_recipients, parse_recipient_list,
    parse_recipient_paste,
};
pub use types::{
    ColumnMapping, ContactField, ContactPreview, ContactPreviewRow, ImportError, ImportFormat,
    ImportOptions, ImportPreview, ImportPreviewRow, ImportRowStatus, ImportSource, ImportStats,
    ImportedContact, MappingConfidence, PreparedImport, SheetInfo, SkippedImportRow, TablePreview,
    is_valid_email, normalize_email,
};

/// Build a UI-ready preview for any supported import source.
pub fn preview_source(
    source: &ImportSource,
    options: ImportOptions,
) -> Result<ImportPreview, ImportError> {
    match source.format {
        ImportFormat::Csv => csv_parser::preview_csv(source, options).map(ImportPreview::Table),
        ImportFormat::Xlsx => xlsx_parser::preview_xlsx(source, options).map(ImportPreview::Table),
        ImportFormat::Vcf => vcard_parser::preview_vcf(&source.data, options.preview_rows)
            .map(ImportPreview::Contacts),
    }
}

/// Prepare importable contacts for any supported import source.
pub fn prepare_import(
    source: &ImportSource,
    mappings: &[ColumnMapping],
    options: ImportOptions,
) -> Result<PreparedImport, ImportError> {
    match source.format {
        ImportFormat::Csv => csv_parser::prepare_csv_import(source, mappings, options),
        ImportFormat::Xlsx => xlsx_parser::prepare_xlsx_import(source, mappings, options),
        ImportFormat::Vcf => vcard_parser::prepare_vcf_import(&source.data),
    }
}
