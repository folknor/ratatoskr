//! Undo/redo-aware text input widget.
//!
//! Wraps iced's [`TextInput`] and intercepts `Ctrl+Z`, `Ctrl+Y`, and
//! `Ctrl+Shift+Z` keyboard events when the input is focused. On undo/redo
//! the widget emits the caller-provided message instead of forwarding the
//! keystroke to the inner text input.
//!
//! All other behaviour (layout, drawing, selection, copy/paste, etc.)
//! delegates unchanged to the inner [`TextInput`].

use iced::advanced::layout;
use iced::advanced::renderer;
use iced::advanced::widget::Operation;
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget};
use iced::keyboard;
use iced::mouse;
use iced::widget::text_input::{self, TextInput};
use iced::{Element, Event, Length, Rectangle, Size, Theme, Vector};

// ── Public constructor ──────────────────────────────────

/// Create an [`UndoableTextInput`] with the given placeholder and value.
///
/// This is the drop-in replacement for [`iced::widget::text_input()`].
/// Chain `.on_undo(msg)` and `.on_redo(msg)` to wire up history.
pub fn undoable_text_input<'a, Message>(
    placeholder: &str,
    value: &str,
) -> UndoableTextInput<'a, Message>
where
    Message: Clone + 'a,
{
    UndoableTextInput {
        inner: TextInput::new(placeholder, value),
        on_undo: None,
        on_redo: None,
    }
}

// ── Builder ─────────────────────────────────────────────

/// A text input that intercepts undo/redo key bindings.
///
/// Built with [`undoable_text_input()`], then configured with the same
/// builder methods as [`TextInput`] plus `.on_undo()` / `.on_redo()`.
///
/// Call `.into()` to convert to an `Element` for use in a `view()`.
pub struct UndoableTextInput<'a, Message> {
    inner: TextInput<'a, Message>,
    on_undo: Option<Message>,
    on_redo: Option<Message>,
}

impl<'a, Message: Clone + 'a> UndoableTextInput<'a, Message> {
    /// Message to emit when the user presses Ctrl+Z while focused.
    pub fn on_undo(mut self, message: Message) -> Self {
        self.on_undo = Some(message);
        self
    }

    /// Message to emit when the user presses Ctrl+Y or Ctrl+Shift+Z.
    pub fn on_redo(mut self, message: Message) -> Self {
        self.on_redo = Some(message);
        self
    }

    // ── Delegated builder methods ───────────────────────

    /// Sets the [`widget::Id`].
    pub fn id(mut self, id: impl Into<iced::widget::Id>) -> Self {
        self.inner = self.inner.id(id);
        self
    }

    /// Sets the `on_input` callback.
    pub fn on_input(mut self, f: impl Fn(String) -> Message + 'a) -> Self {
        self.inner = self.inner.on_input(f);
        self
    }

    /// Sets the `on_submit` message.
    pub fn on_submit(mut self, message: Message) -> Self {
        self.inner = self.inner.on_submit(message);
        self
    }

    /// Sets the `on_paste` callback.
    pub fn on_paste(mut self, f: impl Fn(String) -> Message + 'a) -> Self {
        self.inner = self.inner.on_paste(f);
        self
    }

    /// Enables secure (password) mode.
    pub fn secure(mut self, is_secure: bool) -> Self {
        self.inner = self.inner.secure(is_secure);
        self
    }

    /// Sets the font.
    pub fn font(mut self, font: iced::Font) -> Self {
        self.inner = self.inner.font(font);
        self
    }

    /// Sets the width.
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.inner = self.inner.width(width);
        self
    }

    /// Sets the padding.
    pub fn padding(mut self, padding: impl Into<iced::Padding>) -> Self {
        self.inner = self.inner.padding(padding);
        self
    }

    /// Sets the text size.
    pub fn size(mut self, size: impl Into<iced::Pixels>) -> Self {
        self.inner = self.inner.size(size);
        self
    }

    /// Sets the line height.
    pub fn line_height(mut self, line_height: impl Into<iced::widget::text::LineHeight>) -> Self {
        self.inner = self.inner.line_height(line_height);
        self
    }

    /// Sets the style.
    pub fn style(
        mut self,
        style: impl Fn(&Theme, text_input::Status) -> text_input::Style + 'a,
    ) -> Self {
        self.inner = self.inner.style(style);
        self
    }
}

// ── Into<Element> (builder → widget conversion) ─────────

impl<'a, Message: Clone + 'a> From<UndoableTextInput<'a, Message>> for Element<'a, Message> {
    fn from(builder: UndoableTextInput<'a, Message>) -> Self {
        let wrapper = UndoableWrapper {
            inner: Element::new(builder.inner),
            on_undo: builder.on_undo,
            on_redo: builder.on_redo,
        };
        Element::new(wrapper)
    }
}

// ── Internal wrapper widget ─────────────────────────────

/// The actual `Widget` implementation. Holds the inner `TextInput`
/// as an `Element` (type-erased) so we can delegate all trait methods
/// without running into lifetime-invariance issues on `TextInput<'a>`.
struct UndoableWrapper<'a, Message> {
    inner: Element<'a, Message>,
    on_undo: Option<Message>,
    on_redo: Option<Message>,
}

/// Tracks whether the inner text input is focused so we only
/// intercept undo/redo keys when appropriate.
#[derive(Debug, Default)]
struct WrapperState {
    is_focused: bool,
}

impl<'a, Message: Clone + 'a> Widget<Message, Theme, iced::Renderer>
    for UndoableWrapper<'a, Message>
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<WrapperState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(WrapperState::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(self.inner.as_widget())]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.inner.as_widget()));
    }

    fn size(&self) -> Size<Length> {
        self.inner.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.inner
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.inner
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.handle_update(
            tree, event, layout, cursor, renderer, clipboard, shell, viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.inner.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.inner.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<iced::advanced::overlay::Element<'b, Message, Theme, iced::Renderer>> {
        self.inner.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<Message: Clone> UndoableWrapper<'_, Message> {
    /// Core update logic, split out to keep `update()` under the line limit.
    fn handle_update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let wrapper = tree.state.downcast_mut::<WrapperState>();

        // Track focus by observing mouse clicks (mirrors TextInput logic).
        if let Event::Mouse(mouse::Event::ButtonPressed {
            button: mouse::Button::Left,
            ..
        }) = event
        {
            wrapper.is_focused = cursor.is_over(layout.bounds());
        }

        // Escape unfocuses.
        if let Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::Escape),
            ..
        }) = event
        {
            wrapper.is_focused = false;
        }

        // Intercept undo/redo BEFORE forwarding to the inner TextInput.
        if wrapper.is_focused {
            if let Some(msg) = self.check_undo_redo(event) {
                shell.publish(msg);
                shell.capture_event();
                return;
            }
        }

        // Forward everything else to the inner TextInput.
        self.inner.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    /// Returns an undo or redo message if the event matches the key binding.
    fn check_undo_redo(&self, event: &Event) -> Option<Message> {
        let Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            modifiers,
            physical_key,
            ..
        }) = event
        else {
            return None;
        };

        if !modifiers.command() {
            return None;
        }

        match key.to_latin(*physical_key) {
            Some('z') if !modifiers.shift() => self.on_undo.clone(),
            Some('z') if modifiers.shift() => self.on_redo.clone(),
            Some('y') => self.on_redo.clone(),
            _ => None,
        }
    }
}

// ── Tests ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    enum Msg {
        Input(String),
        Undo,
        Redo,
    }

    #[test]
    fn builder_compiles() {
        let _: Element<'_, Msg> = undoable_text_input("placeholder", "value")
            .on_input(Msg::Input)
            .on_submit(Msg::Undo)
            .on_undo(Msg::Undo)
            .on_redo(Msg::Redo)
            .width(Length::Fill)
            .size(14.0)
            .padding(4.0)
            .into();
    }

    fn make_key_event(
        key_char: &'static str,
        code: keyboard::key::Code,
        modifiers: keyboard::Modifiers,
    ) -> Event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Character(key_char.into()),
            text: None,
            location: keyboard::Location::Standard,
            modifiers,
            modified_key: keyboard::Key::Character(key_char.into()),
            physical_key: keyboard::key::Physical::Code(code),
            repeat: false,
        })
    }

    fn make_wrapper<'a>() -> UndoableWrapper<'a, Msg> {
        let builder: UndoableTextInput<'a, Msg> = undoable_text_input("", "")
            .on_undo(Msg::Undo)
            .on_redo(Msg::Redo);
        UndoableWrapper {
            inner: Element::new(builder.inner),
            on_undo: builder.on_undo,
            on_redo: builder.on_redo,
        }
    }

    #[test]
    fn ctrl_z_triggers_undo() {
        let w = make_wrapper();
        let event = make_key_event("z", keyboard::key::Code::KeyZ, keyboard::Modifiers::CTRL);
        assert_eq!(w.check_undo_redo(&event), Some(Msg::Undo));
    }

    #[test]
    fn ctrl_shift_z_triggers_redo() {
        let w = make_wrapper();
        let event = make_key_event(
            "z",
            keyboard::key::Code::KeyZ,
            keyboard::Modifiers::CTRL.union(keyboard::Modifiers::SHIFT),
        );
        assert_eq!(w.check_undo_redo(&event), Some(Msg::Redo));
    }

    #[test]
    fn ctrl_y_triggers_redo() {
        let w = make_wrapper();
        let event = make_key_event("y", keyboard::key::Code::KeyY, keyboard::Modifiers::CTRL);
        assert_eq!(w.check_undo_redo(&event), Some(Msg::Redo));
    }

    #[test]
    fn no_callbacks_returns_none() {
        let builder: UndoableTextInput<'_, Msg> = undoable_text_input("", "");
        let w = UndoableWrapper {
            inner: Element::new(builder.inner),
            on_undo: None,
            on_redo: None,
        };
        let event = make_key_event("z", keyboard::key::Code::KeyZ, keyboard::Modifiers::CTRL);
        assert_eq!(w.check_undo_redo(&event), None);
    }

    #[test]
    fn unrelated_key_returns_none() {
        let w = make_wrapper();
        let event = make_key_event("a", keyboard::key::Code::KeyA, keyboard::Modifiers::CTRL);
        assert_eq!(w.check_undo_redo(&event), None);
    }
}
