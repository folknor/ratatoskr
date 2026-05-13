// Feature-scoped handler modules. Each file contains `impl App` blocks
// with methods that `main.rs::update()` dispatches to. Add new handler
// methods in the appropriate file - do NOT put handler logic in main.rs.
//
// All `App` fields are accessible from these modules (they are
// descendant modules of the crate root where `App` is defined).
// Import types with `use crate::` paths.

mod accounts;
pub(crate) mod attachments;
mod calendar;
mod chat;
pub(crate) mod commands;
mod contacts;
mod core;
mod keyboard;
mod navigation;
mod palette;
pub(crate) mod pop_out;
pub(crate) mod provider;
pub(crate) mod search;
pub mod signatures;

use crate::ui::settings::SignatureEntry;

/// Result variants for signature operations, used as the output type
/// of `Task::perform` so the caller can map them to `Message`
/// variants.
///
/// Phase 6a: each IPC method gets its own ack variant, per the
/// per-surface checklist. The Task 6 fixup caught the conflation
/// pattern (one variant serving an IPC ack and an unrelated UI
/// workflow with divergent side effects) - here every variant maps
/// 1:1 to a Service IPC method (create / update / delete / reorder)
/// or to the UI-only read path (`Loaded`). `Loaded` is the only
/// variant that does not correspond to an IPC.
#[derive(Debug, Clone)]
pub enum SignatureResult {
    /// `signature.create` ack. Carries the new id on success so a
    /// future caller can reference it without re-listing first; the
    /// `handle_signature_op` arm currently just triggers a re-list
    /// because the settings UI is the only consumer.
    CreatedAck(Result<String, String>),
    /// `signature.update` ack.
    UpdatedAck(Result<(), String>),
    /// `signature.delete` ack.
    DeletedAck(Result<(), String>),
    /// `signature.reorder` ack.
    ReorderedAck(Result<(), String>),
    /// Local-read result from `db_get_all_signatures` - not an IPC
    /// surface, but routed through this enum because it shares the
    /// same `Message::SignatureOp` dispatch path.
    Loaded(Result<Vec<SignatureEntry>, String>),
}
