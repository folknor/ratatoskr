use iced::advanced::{
    layout, overlay, renderer, widget, Clipboard, Layout, Shell, Widget,
};
use iced::{Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector, mouse};

/// A widget that displays a base element and optionally floats a popup
/// overlay below it, without affecting layout.
pub struct Popover<'a, Message> {
    base: iced::Element<'a, Message>,
    popup: Option<iced::Element<'a, Message>>,
}

pub fn popover<'a, Message: 'a>(
    base: impl Into<iced::Element<'a, Message>>,
) -> Popover<'a, Message> {
    Popover {
        base: base.into(),
        popup: None,
    }
}

impl<'a, Message: 'a> Popover<'a, Message> {
    pub fn popup(mut self, popup: impl Into<iced::Element<'a, Message>>) -> Self {
        self.popup = Some(popup.into());
        self
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Popover<'_, Message> {
    fn size(&self) -> Size<Length> {
        self.base.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.base.as_widget().size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut widget::Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.base
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.base.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn children(&self) -> Vec<widget::Tree> {
        match &self.popup {
            Some(popup) => vec![widget::Tree::new(&self.base), widget::Tree::new(popup)],
            None => vec![widget::Tree::new(&self.base)],
        }
    }

    fn diff(&self, tree: &mut widget::Tree) {
        match &self.popup {
            Some(popup) => tree.diff_children(&[&self.base, popup]),
            None => tree.diff_children(&[&self.base]),
        }
    }

    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation<()>,
    ) {
        self.base
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.base.as_widget_mut().update(
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

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.base.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut widget::Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let popup = self.popup.as_mut()?;

        let (first, second) = tree.children.split_at_mut(1);

        let base = self.base.as_widget_mut().overlay(
            &mut first[0],
            layout,
            renderer,
            viewport,
            translation,
        );

        let overlay = overlay::Element::new(Box::new(PopoverOverlay {
            content: popup,
            tree: &mut second[0],
            base_bounds: layout.bounds(),
            position: layout.position(),
            viewport: *viewport,
        }));

        Some(
            overlay::Group::with_children(
                base.into_iter().chain(Some(overlay)).collect(),
            )
            .overlay(),
        )
    }
}

impl<'a, Message: 'a> From<Popover<'a, Message>> for iced::Element<'a, Message> {
    fn from(popover: Popover<'a, Message>) -> Self {
        iced::Element::new(popover)
    }
}

struct PopoverOverlay<'a, 'b, Message> {
    content: &'b mut iced::Element<'a, Message>,
    tree: &'b mut widget::Tree,
    base_bounds: Rectangle,
    position: Point,
    viewport: Rectangle,
}

impl<Message> overlay::Overlay<Message, Theme, Renderer>
    for PopoverOverlay<'_, '_, Message>
{
    fn layout(&mut self, renderer: &Renderer, _bounds: Size) -> layout::Node {
        let limits = layout::Limits::new(
            Size::ZERO,
            Size {
                width: self.base_bounds.width,
                height: self.viewport.height - self.position.y - self.base_bounds.height,
            },
        )
        .width(Length::Fill);

        let node = self
            .content
            .as_widget_mut()
            .layout(self.tree, renderer, &limits);

        // Position directly below the base widget
        node.move_to(self.position + Vector::new(0.0, self.base_bounds.height))
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        self.content.as_widget().draw(
            self.tree,
            renderer,
            theme,
            style,
            layout,
            cursor,
            &layout.bounds(),
        );
    }

    fn operate(
        &mut self,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation<()>,
    ) {
        self.content
            .as_widget_mut()
            .operate(self.tree, layout, renderer, operation);
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        self.content.as_widget_mut().update(
            self.tree,
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            &layout.bounds(),
        );

        // Capture mouse/touch events over the overlay so they don't
        // propagate to widgets underneath.
        if matches!(event, Event::Mouse(_) | Event::Touch(_))
            && cursor.is_over(layout.bounds())
        {
            shell.capture_event();
        }
    }

    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            self.tree,
            layout,
            cursor,
            &layout.bounds(),
            renderer,
        )
    }

    fn overlay<'c>(
        &'c mut self,
        layout: Layout<'c>,
        renderer: &Renderer,
    ) -> Option<overlay::Element<'c, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            self.tree,
            layout,
            renderer,
            &self.viewport,
            Vector::default(),
        )
    }
}
