pub mod compose;
pub mod message_view;
pub mod session;

pub use compose::{ComposeMessage, ComposeState};
pub use message_view::{MessageViewMessage, MessageViewState, RenderingMode};

/// Identifies what a pop-out window is showing.
pub enum PopOutWindow {
    MessageView(Box<MessageViewState>),
    Compose(Box<ComposeState>),
    /// Calendar pop-out - full calendar UI in a separate window. The
    /// calendar's view and date live on `App.calendar`, so the variant
    /// only carries window geometry for session restore.
    Calendar(CalendarPopOutGeometry),
}

/// Window geometry for the calendar pop-out. Tracked per-window from
/// resize/move events; persisted to `session.json` on close.
#[derive(Debug, Clone, Copy)]
pub struct CalendarPopOutGeometry {
    pub width: f32,
    pub height: f32,
    pub x: Option<f32>,
    pub y: Option<f32>,
}

/// Messages internal to pop-out windows.
#[derive(Debug, Clone)]
pub enum PopOutMessage {
    MessageView(MessageViewMessage),
    Compose(ComposeMessage),
    /// Calendar pop-out messages are routed through CalendarMessage.
    Calendar(Box<crate::ui::calendar::CalendarMessage>),
}
