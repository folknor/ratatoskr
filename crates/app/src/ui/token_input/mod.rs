//! Token input widget for chip/tag input with inline tokens.
//!
//! Used in compose recipient fields (To/Cc/Bcc), calendar attendee fields,
//! and the contact group editor. The widget is context-agnostic - all
//! compose-specific behavior (autocomplete dropdown, Bcc suggestions,
//! cross-field drag) lives in the parent view, not here.
//!
//! Built as a custom `advanced::Widget` with a wrapping flow layout of
//! token chip backgrounds + text, followed by an inline text input area.
//! The parent manages the token list and text input state via Elm architecture.

mod handlers;
mod layout;
mod render;
mod types;
mod widget;

pub use types::*;
pub use widget::*;

/// Minimal validation - catches obvious typos, not RFC 5321.
pub fn is_plausible_email(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let Some((local, domain)) = trimmed.split_once('@') else {
        return false;
    };
    !local.is_empty() && domain.contains('.')
}
