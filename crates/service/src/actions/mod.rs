//! Action service - the authoritative write path for email state mutations.
//!
//! Every mutating email action (archive, trash, star, label, etc.) flows
//! through this module. It handles local DB mutation, resident-engine
//! dispatch, and structured outcome reporting. The app crate calls these
//! functions and never constructs providers or dispatches provider calls
//! directly.

mod archive;
pub mod batch;
pub mod contacts;
mod context;
pub(crate) mod dispatch_target;
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
// Public for retained provider-backed send, draft, folder, attachment, MDN,
// and prefetch paths. Email action mutations no longer call `create_provider`.
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

pub use batch::batch_execute;
pub use common::types::ProviderFolderMutation;
pub use context::ActionContext;
pub use folder::{create_folder, delete_folder, rename_folder};
pub use log::MutationLog;
pub use mute::mute;
pub use operation::MailOperation;
pub use outcome::{ActionError, ActionOutcome, RemoteFailureKind};
pub use pin::pin;
pub use send::{cancel_scheduled_send, delete_draft, reschedule_send, send_email, send_scheduled};
pub use service_api::actions::{
    FolderId, LabelGroupId, LabelId, SendAttachment, SendIntent, SendRequest,
};
pub use snooze::{snooze, unsnooze};
