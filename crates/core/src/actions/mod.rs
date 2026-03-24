//! Action service — the authoritative write path for email state mutations.
//!
//! Every mutating email action (archive, trash, star, label, etc.) flows
//! through this module. It handles local DB mutation, provider dispatch,
//! and structured outcome reporting. The app crate calls these functions
//! and never constructs providers or dispatches provider calls directly.

mod archive;
mod context;
pub mod contacts;
mod folder;
mod label;
mod log;
mod mark_read;
mod move_to_folder;
mod mute;
mod outcome;
mod permanent_delete;
mod pin;
mod provider;
mod send;
mod spam;
mod star;
mod trash;

pub use archive::archive;
pub use context::ActionContext;
pub use folder::{create_folder, delete_folder, rename_folder};
pub use ratatoskr_provider_utils::types::ProviderFolderMutation;
pub use label::{add_label, remove_label};
pub use mark_read::mark_read;
// Re-export send types so callers import from actions, not crate::send directly.
pub use crate::send::{SendAttachment, SendRequest};
pub use send::{delete_draft, send_email};
pub use move_to_folder::move_to_folder;
pub use mute::mute;
pub use log::MutationLog;
pub use outcome::{ActionError, ActionOutcome, RemoteFailureKind};
pub use permanent_delete::permanent_delete;
pub use pin::pin;
pub use provider::create_provider;
pub use spam::spam;
pub use star::star;
pub use trash::trash;
