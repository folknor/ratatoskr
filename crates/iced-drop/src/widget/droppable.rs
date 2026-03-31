//! A widget wrapper that makes any iced element draggable with drop zone detection.
//!
//! Vendored from <https://github.com/jhannyj/iced_drop> (v0.2.2).

use iced_core::layout::{Limits, Node};
use iced_core::mouse::Cursor;
use iced_core::renderer::Style;
use iced_core::widget::tree::Tag;
use iced_core::widget::{Id, Operation, Tree};
use iced_core::{
    Element, Event, Layout, Length, Pixels, Point, Rectangle, Size, Vector, Widget, mouse, overlay,
    renderer, touch, window,
};
use std::fmt::Debug;
use std::vec;

/// An element that can be dragged and dropped on a drop zone.
pub struct Droppable<'a, Message, Theme = iced_widget::Theme, Renderer = iced_widget::Renderer>
where
    Message: Clone,
    Renderer: renderer::Renderer,
{
    content: Element<'a, Message, Theme, Renderer>,
    id: Option<Id>,
    drag_threshold: f32,
    on_press: Option<Message>,
    on_click: Option<Message>,
    on_drop: Option<Box<dyn Fn(Point, Rectangle) -> Message + 'a>>,
    on_drag: Option<Box<dyn Fn(Point, Rectangle) -> Message + 'a>>,
    on_cancel: Option<Message>,
    drag_mode: Option<(bool, bool)>,
    drag_overlay: bool,
    drag_hide: bool,
    drag_center: bool,
    drag_size: Option<Size>,
    reset_delay: usize,
    status: Option<Status>,
}

impl<'a, Message, Theme, Renderer> Droppable<'a, Message, Theme, Renderer>
where
    Message: Clone,
    Renderer: renderer::Renderer,
{
    /// Creates a new [`Droppable`].
    pub fn new(content: impl Into<Element<'a, Message, Theme, Renderer>>) -> Self {
        Self {
            content: content.into(),
            id: None,
            drag_threshold: 5.0,
            on_press: None,
            on_click: None,
            on_drop: None,
            on_drag: None,
            on_cancel: None,
            drag_mode: Some((true, true)),
            drag_overlay: true,
            drag_hide: false,
            drag_center: false,
            drag_size: None,
            reset_delay: 0,
            status: None,
        }
    }

    /// Sets the unique identifier of the [`Droppable`].
    pub fn id(mut self, id: Id) -> Self {
        self.id = Some(id);
        self
    }

    /// Sets the drag threshold of the [`Droppable`].
    ///
    /// This controls when [`on_press`] will be triggered (if set) after selecting a droppable and
    /// moving the cursor.
    ///
    /// [`on_press`]: Droppable::on_press
    pub fn drag_threshold(mut self, drag_threshold: impl Into<Pixels>) -> Self {
        self.drag_threshold = drag_threshold.into().0;
        self
    }

    /// Sets the message that will be produced when the [`Droppable`] is clicked.
    ///
    /// This will get triggered even if drag occurs.
    pub fn on_click(mut self, message: Message) -> Self {
        self.on_click = Some(message);
        self
    }

    /// Sets the message that will be produced when the [`Droppable`] is pressed.
    ///
    /// This will get triggered only when not dragging.
    pub fn on_press(mut self, message: Message) -> Self {
        self.on_press = Some(message);
        self
    }

    /// Sets the message that will be produced when the [`Droppable`] is dropped on a drop zone.
    ///
    /// Unless this is set, the [`Droppable`] will be disabled.
    pub fn on_drop<F>(mut self, message: F) -> Self
    where
        F: Fn(Point, Rectangle) -> Message + 'a,
    {
        self.on_drop = Some(Box::new(message));
        self
    }

    /// Sets the message that will be produced when the [`Droppable`] is dragged.
    pub fn on_drag<F>(mut self, message: F) -> Self
    where
        F: Fn(Point, Rectangle) -> Message + 'a,
    {
        self.on_drag = Some(Box::new(message));
        self
    }

    /// Sets the message that will be produced when the user right clicks while dragging.
    pub fn on_cancel(mut self, message: Message) -> Self {
        self.on_cancel = Some(message);
        self
    }

    /// Sets whether the [`Droppable`] should be drawn under the cursor while dragging.
    pub fn drag_overlay(mut self, drag_overlay: bool) -> Self {
        self.drag_overlay = drag_overlay;
        self
    }

    /// Sets whether the [`Droppable`] should be hidden while dragging.
    pub fn drag_hide(mut self, drag_hide: bool) -> Self {
        self.drag_hide = drag_hide;
        self
    }

    /// Sets whether the [`Droppable`] should be centered on the cursor while dragging.
    pub fn drag_center(mut self, drag_center: bool) -> Self {
        self.drag_center = drag_center;
        self
    }

    /// Sets whether the [`Droppable`] can be dragged along individual axes.
    pub fn drag_mode(mut self, drag_x: bool, drag_y: bool) -> Self {
        self.drag_mode = Some((drag_x, drag_y));
        self
    }

    /// Sets whether the [`Droppable`] should be resized to a given size while dragging.
    pub fn drag_size(mut self, hide_size: Size) -> Self {
        self.drag_size = Some(hide_size);
        self
    }

    /// Sets the number of frames/layout calls to wait before resetting the size after dropping.
    ///
    /// This is useful for cases where the [`Droppable`] is being moved to a new location after
    /// some widget operation. The [`Droppable`] will maintain the `drag_size` for the given number
    /// of frames before resetting to its original size, preventing flickering.
    pub fn reset_delay(mut self, reset_delay: usize) -> Self {
        self.reset_delay = reset_delay;
        self
    }
}

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Droppable<'a, Message, Theme, Renderer>
where
    Message: Clone,
    Renderer: renderer::Renderer,
{
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &Limits) -> Node {
        let state: &mut DroppableState = tree.state.downcast_mut::<DroppableState>();
        let content_node =
            self.content
                .as_widget_mut()
                .layout(&mut tree.children[0], renderer, limits);

        // Adjust the size of the original widget if it's being dragged or we're waiting to reset
        if let Some(new_size) = self.drag_size {
            match state.action {
                Action::Drag(_, _) => {
                    return Node::with_children(new_size, content_node.children().to_vec());
                }
                Action::Wait(reveal_index) => {
                    if reveal_index <= 1 {
                        state.action = Action::None;
                    } else {
                        state.action = Action::Wait(reveal_index - 1);
                    }

                    return Node::with_children(new_size, content_node.children().to_vec());
                }
                _ => (),
            }
        }

        content_node
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &Style,
        layout: Layout<'_>,
        cursor: Cursor,
        viewport: &Rectangle,
    ) {
        let state: &DroppableState = tree.state.downcast_ref::<DroppableState>();
        if let Action::Drag(_, _) = state.action
            && self.drag_hide
        {
            return;
        }

        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn tag(&self) -> Tag {
        Tag::of::<DroppableState>()
    }

    fn state(&self) -> iced_core::widget::tree::State {
        iced_core::widget::tree::State::new(DroppableState::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        let state = tree.state.downcast_mut::<DroppableState>();
        operation.custom(self.id.as_ref(), layout.bounds(), state);
        operation.container(self.id.as_ref(), layout.bounds());
        operation.traverse(&mut |operation| {
            self.content.as_widget_mut().operate(
                &mut tree.children[0],
                layout,
                renderer,
                operation,
            );
        });
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn iced_core::Clipboard,
        shell: &mut iced_core::Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<DroppableState>();

        if !matches!(state.action, Action::Drag(_, _)) {
            // Handle the on event of the content first, in case the droppable is nested
            self.content.as_widget_mut().update(
                &mut tree.children[0],
                event,
                layout,
                cursor,
                renderer,
                clipboard,
                shell,
                viewport,
            );
            // This should only be captured if the droppable is nested, or it contains some other
            // widget that captures the event
            if shell.is_event_captured() {
                return;
            }
        }

        if let Some(on_drop) = self.on_drop.as_deref() {
            match event {
                Event::Mouse(mouse::Event::ButtonPressed {
                    button: mouse::Button::Left,
                    ..
                })
                | Event::Touch(touch::Event::FingerPressed { .. }) => {
                    let bounds = layout.bounds();
                    if cursor.is_over(bounds)
                        && let Some(pos) = cursor.position()
                    {
                        // Select the droppable and store the position before dragging
                        state.action = Action::Select(pos);
                        state.widget_pos = bounds.position();
                        state.overlay_bounds.width = bounds.width;
                        state.overlay_bounds.height = bounds.height;

                        if let Some(on_click) = self.on_click.clone() {
                            shell.publish(on_click);
                        }

                        shell.capture_event();
                    }
                }
                Event::Mouse(mouse::Event::ButtonPressed {
                    button: mouse::Button::Right,
                    ..
                })
                | Event::Touch(touch::Event::FingerLost { .. }) => {
                    if let Action::Drag(_, _) = state.action {
                        state.action = Action::None;
                        if let Some(on_cancel) = self.on_cancel.clone() {
                            shell.publish(on_cancel);
                        }

                        shell.invalidate_layout();
                        shell.request_redraw();
                    }
                }
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                | Event::Touch(touch::Event::FingerLifted { .. }) => match state.action {
                    Action::Select(_) => {
                        if let Some(on_press) = self.on_press.clone() {
                            shell.publish(on_press);
                        }

                        state.action = Action::None;
                    }
                    Action::Drag(_, current) => {
                        let message = (on_drop)(current, state.overlay_bounds);
                        shell.publish(message);

                        if self.reset_delay == 0 {
                            state.action = Action::None;
                        } else {
                            state.action = Action::Wait(self.reset_delay);
                        }
                    }
                    _ => (),
                },
                Event::Mouse(mouse::Event::CursorMoved { position })
                | Event::Touch(touch::Event::FingerMoved { position, .. }) => {
                    let position = *position;

                    let should_drag = match state.action {
                        Action::Select(start) => {
                            let distance = ((position.x - start.x).powi(2)
                                + (position.y - start.y).powi(2))
                            .sqrt();

                            if distance >= self.drag_threshold {
                                state.action = Action::Drag(start, position);
                                true
                            } else {
                                false
                            }
                        }
                        Action::Drag(start, _) => {
                            state.action = Action::Drag(start, position);
                            true
                        }
                        _ => false,
                    };

                    if should_drag && let Action::Drag(start, _) = state.action {
                        // Apply drag mode constraints
                        let constrained = if let Some((drag_x, drag_y)) = self.drag_mode {
                            Point {
                                x: if drag_x { position.x } else { start.x },
                                y: if drag_y { position.y } else { start.y },
                            }
                        } else {
                            position
                        };

                        // Update the position of the overlay since the cursor was moved
                        if self.drag_center {
                            state.overlay_bounds.x =
                                constrained.x - state.overlay_bounds.width / 2.0;
                            state.overlay_bounds.y =
                                constrained.y - state.overlay_bounds.height / 2.0;
                        } else {
                            state.overlay_bounds.x = state.widget_pos.x + constrained.x - start.x;
                            state.overlay_bounds.y = state.widget_pos.y + constrained.y - start.y;
                        }

                        // Send on_drag message
                        if let Some(on_drag) = self.on_drag.as_deref() {
                            let message = (on_drag)(constrained, state.overlay_bounds);
                            shell.publish(message);
                        }

                        shell.request_redraw();
                    }
                }
                _ => {}
            }
        }

        let current_status = if self.on_drop.is_none() {
            Status::Disabled
        } else if cursor.is_over(layout.bounds()) {
            if let Action::Drag(_, _) = state.action {
                Status::Dragged
            } else {
                Status::Hovered
            }
        } else {
            Status::Active
        };

        if let Event::Window(window::Event::RedrawRequested(_now)) = event {
            self.status = Some(current_status);
        } else if self.status.is_some_and(|status| status != current_status) {
            shell.request_redraw();
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<DroppableState>();

        if let Action::Drag(_, _) = state.action {
            return mouse::Interaction::Grabbing;
        }

        let child_interact = self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        );

        if child_interact != mouse::Interaction::default() {
            return child_interact;
        }

        if cursor.is_over(layout.bounds()) {
            if self.on_drop.is_some() {
                if self.on_press.is_some() {
                    mouse::Interaction::Pointer
                } else {
                    mouse::Interaction::Grab
                }
            } else {
                mouse::Interaction::NotAllowed
            }
        } else {
            mouse::Interaction::default()
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let state: &mut DroppableState = tree.state.downcast_mut::<DroppableState>();
        if self.drag_overlay
            && let Action::Drag(_, _) = state.action
        {
            return Some(overlay::Element::new(Box::new(DragOverlay {
                content: &mut self.content,
                tree: &mut tree.children[0],
                overlay_bounds: state.overlay_bounds,
            })));
        }
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message, Theme, Renderer> From<Droppable<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a + Clone,
    Theme: 'a,
    Renderer: 'a + renderer::Renderer,
{
    fn from(
        droppable: Droppable<'a, Message, Theme, Renderer>,
    ) -> Element<'a, Message, Theme, Renderer> {
        Element::new(droppable)
    }
}

/// Internal state of a [`Droppable`] widget.
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub struct DroppableState {
    widget_pos: Point,
    overlay_bounds: Rectangle,
    action: Action,
}

/// Visual status of a [`Droppable`].
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub enum Status {
    #[default]
    Active,
    Hovered,
    Dragged,
    Disabled,
}

/// Internal action state tracking the drag lifecycle.
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub enum Action {
    #[default]
    None,
    /// Initial click detected at this point.
    Select(Point),
    /// Active drag: (start position, current position).
    Drag(Point, Point),
    /// Waiting N frames before resetting size after drop.
    Wait(usize),
}

struct DragOverlay<'a, 'b, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    content: &'b mut Element<'a, Message, Theme, Renderer>,
    tree: &'b mut Tree,
    overlay_bounds: Rectangle,
}

impl<'a, 'b, Message, Theme, Renderer> overlay::Overlay<Message, Theme, Renderer>
    for DragOverlay<'a, 'b, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn layout(&mut self, renderer: &Renderer, _bounds: Size) -> Node {
        Widget::<Message, Theme, Renderer>::layout(
            self.content.as_widget_mut(),
            self.tree,
            renderer,
            &Limits::new(Size::ZERO, self.overlay_bounds.size()),
        )
        .move_to(self.overlay_bounds.position())
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        inherited_style: &Style,
        layout: Layout<'_>,
        cursor_position: Cursor,
    ) {
        Widget::<Message, Theme, Renderer>::draw(
            self.content.as_widget(),
            self.tree,
            renderer,
            theme,
            inherited_style,
            layout,
            cursor_position,
            &Rectangle::with_size(Size::INFINITE),
        );
    }
}
