//! Extracted handler modules for the app crate.
//!
//! Each module encapsulates the DB logic for a specific feature domain,
//! replacing raw SQL that was previously inlined in `main.rs`.

pub mod signatures;

use crate::ui::settings::SignatureEntry;

/// Result variants for signature operations, used as the output type of
/// `Task::perform` so the caller can map them to `Message` variants.
#[derive(Debug)]
pub enum SignatureResult {
    Saved(Result<(), String>),
    Deleted(Result<(), String>),
    Loaded(Result<Vec<SignatureEntry>, String>),
}
