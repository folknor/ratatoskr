//! Action service - the authoritative write path for email state mutations.
//!
//! Every mutating email action (archive, trash, star, label, etc.) flows
//! through this module. It handles local DB mutation, provider dispatch,
//! and structured outcome reporting. The app crate calls these functions
//! and never constructs providers or dispatches provider calls directly.

mod archive;
pub mod batch;
pub mod contacts;
mod context;
mod dispatch_target;
mod folder;
mod label;
mod label_group;
mod log;
mod mark_read;
mod mdn_send;
mod move_to_folder;
mod mute;
mod operation;
mod outcome;
pub mod pending;
mod permanent_delete;
mod pin;
// Public so the `core::actions::provider` shim can re-export
// `create_provider` for core callers.
// Once those callers migrate off `crate::actions::provider::*` (Phase
// 2 task 9 / 10), this can drop back to `pub(crate)`.
pub mod provider;
mod send;
mod snooze;
mod spam;
mod star;
#[cfg(test)]
mod tests;
mod trash;
pub(crate) mod wire_conversion;
pub(crate) mod worker;

pub use archive::archive;
pub use batch::batch_execute;
pub use common::types::ProviderFolderMutation;
pub use context::ActionContext;
pub use folder::{create_folder, delete_folder, rename_folder};
pub use log::MutationLog;
pub use mark_read::mark_read;
pub use move_to_folder::move_to_folder;
pub use mute::mute;
pub use operation::MailOperation;
pub use outcome::{ActionError, ActionOutcome, RemoteFailureKind};
pub use permanent_delete::permanent_delete;
pub use pin::pin;
pub use send::{delete_draft, send_email};
pub use service_api::actions::{
    FolderId, LabelGroupId, LabelId, SendAttachment, SendIntent, SendRequest,
};
// create_provider is pub(crate) - only accessible within core, not to downstream crates.
// The app must use action functions or service IPC helpers.
pub use snooze::{snooze, unsnooze};
pub use spam::spam;
pub use star::star;
pub use trash::trash;
