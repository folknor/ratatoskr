pub mod message_view;

pub use message_view::{MessageViewMessage, MessageViewState};

/// Identifies what a pop-out window is showing.
#[derive(Debug, Clone)]
pub enum PopOutWindow {
    MessageView(MessageViewState),
    // Future variants:
    // Compose(ComposeWindowState),
    // Calendar(CalendarWindowState),
}

/// Messages internal to pop-out windows.
#[derive(Debug, Clone)]
pub enum PopOutMessage {
    MessageView(MessageViewMessage),
    // Future: Compose(ComposeMessage), Calendar(CalendarMessage)
}
