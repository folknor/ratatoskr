//! Action service — the authoritative write path for email state mutations.
//!
//! Every mutating email action (archive, trash, star, label, etc.) flows
//! through this module. It handles local DB mutation, provider dispatch,
//! and structured outcome reporting. The app crate calls these functions
//! and never constructs providers or dispatches provider calls directly.

mod archive;
mod context;
mod mark_read;
mod move_to_folder;
mod mute;
mod outcome;
mod permanent_delete;
mod pin;
mod provider;
mod spam;
mod star;
mod trash;

pub use archive::archive;
pub use context::ActionContext;
pub use mark_read::mark_read;
pub use move_to_folder::move_to_folder;
pub use mute::mute;
pub use outcome::ActionOutcome;
pub use permanent_delete::permanent_delete;
pub use pin::pin;
pub use provider::create_provider;
pub use spam::spam;
pub use star::star;
pub use trash::trash;
