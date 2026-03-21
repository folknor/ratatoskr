//! Contact import library for CSV and vCard formats.
//!
//! This crate handles file format detection, encoding conversion,
//! delimiter detection, column header heuristic matching, and
//! structured contact output. The UI lives in the app crate.

pub mod csv_parser;
mod detect;
mod mapping;
mod types;
mod vcard_parser;

pub use csv_parser::parse_csv;
pub use mapping::{auto_detect_mappings, ColumnMapping};
pub use types::{ContactField, ImportError, ImportFormat, ImportPreview, ImportSource, ImportedContact};
pub use vcard_parser::parse_vcf;
