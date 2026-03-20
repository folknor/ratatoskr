use iced::{Element, Task};

/// A self-contained UI component with its own message and event types.
///
/// - `Message`: internal messages the component handles (button clicks, toggles, etc.)
/// - `Event`: outward signals the parent should react to (navigation changes, actions)
///
/// The parent dispatches incoming messages via `update()`, maps the returned
/// `Task` with its own wrapper, and handles any emitted `Event`.
pub trait Component {
    type Message;
    type Event;

    fn update(&mut self, message: Self::Message) -> (Task<Self::Message>, Option<Self::Event>);
    fn view(&self) -> Element<'_, Self::Message>;
}
