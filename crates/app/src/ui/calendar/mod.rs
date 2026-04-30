//! Calendar view skeleton: layout, state, and messages.
//!
//! When the app is in Calendar mode, this module renders the two-panel
//! calendar layout: a sidebar (mini-month, view switcher, calendar list)
//! and a main content area dispatched by the active view.
//! Includes event detail popover and creation/editing overlays.

mod dialogs;
mod event_detail;
mod event_editor;
mod event_full_modal;
mod format;
mod layout;
mod messages;
mod sidebar;
mod types;

pub use layout::calendar_layout;
pub use messages::*;
pub use types::*;
