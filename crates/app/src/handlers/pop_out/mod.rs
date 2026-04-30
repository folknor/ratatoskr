//! Pop-out window handler methods for `App`.
//!
//! All pop-out window logic lives here. `main.rs` dispatches to these methods
//! via one-line match arms.

use std::time::Duration;

pub const DRAFT_AUTO_SAVE_INTERVAL: Duration = Duration::from_secs(30);

mod compose_clipboard;
mod compose_draft;
mod compose_send;
mod compose_signature;
mod dispatcher;
mod message_view;
mod save_as;
mod session;
mod window_lifecycle;
