pub mod compose;
pub mod message_view;

pub use compose::{ComposeMessage, ComposeState};
pub use message_view::{MessageViewMessage, MessageViewState};

/// Identifies what a pop-out window is showing.
pub enum PopOutWindow {
    MessageView(MessageViewState),
    Compose(ComposeState),
}

/// Messages internal to pop-out windows.
#[derive(Debug, Clone)]
pub enum PopOutMessage {
    MessageView(MessageViewMessage),
    Compose(ComposeMessage),
}
