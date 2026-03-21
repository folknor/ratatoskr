mod accounts;
mod calendar;
mod commands;
mod contacts;
mod keyboard;
mod palette;
mod pop_out;
mod search;
pub mod signatures;

use crate::ui::settings::SignatureEntry;

/// Result variants for signature operations, used as the output type of
/// `Task::perform` so the caller can map them to `Message` variants.
#[derive(Debug, Clone)]
pub enum SignatureResult {
    Saved(Result<(), String>),
    Deleted(Result<(), String>),
    Loaded(Result<Vec<SignatureEntry>, String>),
}
