//! Action service — the authoritative write path for email state mutations.
//!
//! Every mutating email action (archive, trash, star, label, etc.) flows
//! through this module. It handles local DB mutation, provider dispatch,
//! and structured outcome reporting. The app crate calls these functions
//! and never constructs providers or dispatches provider calls directly.

mod archive;
mod context;
mod outcome;
mod provider;

pub use archive::archive;
pub use context::ActionContext;
pub use outcome::ActionOutcome;
pub use provider::create_provider;
