// Feature-scoped handler modules. Each file contains `impl App` blocks
// with methods that `main.rs::update()` dispatches to. Add new handler
// methods in the appropriate file — do NOT put handler logic in main.rs.
//
// All `App` fields are accessible from these modules (they are
// descendant modules of the crate root where `App` is defined).
// Import types with `use crate::` paths.

mod accounts;
mod calendar;
mod commands;
mod contacts;
mod keyboard;
mod palette;
pub(crate) mod pop_out;
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
