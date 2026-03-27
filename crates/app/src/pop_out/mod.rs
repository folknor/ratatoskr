pub mod compose;
pub mod message_view;
pub mod session;

pub use compose::{ComposeMessage, ComposeState};
pub use message_view::{MessageViewMessage, MessageViewState, RenderingMode};

/// Identifies what a pop-out window is showing.
pub enum PopOutWindow {
    MessageView(Box<MessageViewState>),
    Compose(Box<ComposeState>),
    /// Calendar pop-out — full calendar UI in a separate window.
    Calendar,
}

/// Messages internal to pop-out windows.
#[derive(Debug, Clone)]
pub enum PopOutMessage {
    MessageView(MessageViewMessage),
    Compose(ComposeMessage),
    /// Calendar pop-out messages are routed through CalendarMessage.
    Calendar(Box<crate::ui::calendar::CalendarMessage>),
}
