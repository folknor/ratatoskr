//! Action service — the authoritative write path for email state mutations.
//!
//! Every mutating email action (archive, trash, star, label, etc.) flows
//! through this module. It handles local DB mutation, provider dispatch,
//! and structured outcome reporting. The app crate calls these functions
//! and never constructs providers or dispatches provider calls directly.

mod archive;
mod context;
mod mark_read;
mod mute;
mod outcome;
mod pin;
mod provider;
mod star;

pub use archive::archive;
pub use context::ActionContext;
pub use mark_read::mark_read;
pub use mute::mute;
pub use outcome::ActionOutcome;
pub use pin::pin;
pub use provider::create_provider;
pub use star::star;
