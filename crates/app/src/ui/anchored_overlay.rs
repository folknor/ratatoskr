use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, overlay, renderer, widget};
use iced::{Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector, mouse};

/// A widget that displays a base element and optionally floats a popup
/// overlay relative to it, without affecting layout. Clicks outside the
/// popup dismiss it via `on_dismiss`.
pub struct AnchoredOverlay<'a, Message> {
    base: iced::Element<'a, Message>,
    popup: Option<iced::Element<'a, Message>>,
    on_dismiss: Option<Message>,
    popup_width: Option<f32>,
    position: AnchorPosition,
    anchor_point: Option<Point>,
}

/// Where the popup appears relative to the base widget.
#[derive(Debug, Clone, Copy, Default)]
pub enum AnchorPosition {
    /// Below the base, left-aligned.
    #[default]
    Below,
    /// Below the base, right edge aligned with the base's right edge.
    BelowRight,
}

pub fn anchored_overlay<'a, Message: 'a>(
    base: impl Into<iced::Element<'a, Message>>,
) -> AnchoredOverlay<'a, Message> {
    AnchoredOverlay {
        base: base.into(),
        popup: None,
        on_dismiss: None,
        popup_width: None,
        position: AnchorPosition::default(),
        anchor_point: None,
    }
}

impl<'a, Message: Clone + 'a> AnchoredOverlay<'a, Message> {
    pub fn popup(mut self, popup: impl Into<iced::Element<'a, Message>>) -> Self {
        self.popup = Some(popup.into());
        self
    }

    pub fn on_dismiss(mut self, message: Message) -> Self {
        self.on_dismiss = Some(message);
        self
    }

    /// Set a fixed width for the popup. If unset, the popup uses the base's width.
    pub fn popup_width(mut self, width: f32) -> Self {
        self.popup_width = Some(width);
        self
    }

    pub fn position(mut self, position: AnchorPosition) -> Self {
        self.position = position;
        self
    }

    pub fn anchor_point(mut self, point: Point) -> Self {
        self.anchor_point = Some(point);
        self
    }
}

impl<Message: Clone> Widget<Message, Theme, Renderer> for AnchoredOverlay<'_, Message> {
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

        let base_position = layout.position() + translation;
        let overlay = overlay::Element::new(Box::new(AnchoredOverlayLayer {
            content: popup,
            tree: &mut second[0],
            base_bounds: layout.bounds(),
            base_position,
            viewport: *viewport,
            on_dismiss: self.on_dismiss.clone(),
            popup_width: self.popup_width,
            position: self.position,
            anchor_point: self.anchor_point,
        }));

        Some(
            overlay::Group::with_children(base.into_iter().chain(Some(overlay)).collect())
                .overlay(),
        )
    }
}

impl<'a, Message: Clone + 'a> From<AnchoredOverlay<'a, Message>> for iced::Element<'a, Message> {
    fn from(overlay: AnchoredOverlay<'a, Message>) -> Self {
        iced::Element::new(overlay)
    }
}

struct AnchoredOverlayLayer<'a, 'b, Message> {
    content: &'b mut iced::Element<'a, Message>,
    tree: &'b mut widget::Tree,
    base_bounds: Rectangle,
    base_position: Point,
    viewport: Rectangle,
    on_dismiss: Option<Message>,
    popup_width: Option<f32>,
    position: AnchorPosition,
    anchor_point: Option<Point>,
}

impl<Message: Clone> overlay::Overlay<Message, Theme, Renderer>
    for AnchoredOverlayLayer<'_, '_, Message>
{
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        let anchor_position = self.anchor_point.unwrap_or(self.base_position);
        let anchor_width = if self.anchor_point.is_some() {
            0.0
        } else {
            self.base_bounds.width
        };
        let anchor_height = if self.anchor_point.is_some() {
            0.0
        } else {
            self.base_bounds.height
        };

        let popup_width = self.popup_width.unwrap_or(anchor_width);
        let below_y = anchor_position.y + anchor_height;
        let available_height = (bounds.height - below_y).max(0.0);

        let limits = layout::Limits::new(
            Size::ZERO,
            Size {
                width: popup_width,
                height: available_height,
            },
        )
        .width(Length::Fill);

        let node = self
            .content
            .as_widget_mut()
            .layout(self.tree, renderer, &limits);

        // Calculate X based on position mode
        let x = match self.position {
            AnchorPosition::Below => anchor_position.x,
            AnchorPosition::BelowRight => {
                let right_edge = anchor_position.x + anchor_width;
                (right_edge - node.size().width).max(0.0)
            }
        };

        // Clamp so popup stays within viewport
        let x = x.clamp(0.0, (bounds.width - node.size().width).max(0.0));

        node.move_to(Point::new(x, below_y))
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

        // Clicks inside the overlay are captured so they don't propagate.
        // Clicks outside the overlay dismiss it.
        if let Event::Mouse(mouse::Event::ButtonPressed {
            button: mouse::Button::Left,
            ..
        })
        | Event::Touch(iced::touch::Event::FingerPressed { .. }) = event
        {
            if cursor.is_over(layout.bounds()) {
                shell.capture_event();
            } else if let Some(on_dismiss) = &self.on_dismiss {
                shell.publish(on_dismiss.clone());
            }
        } else if matches!(event, Event::Mouse(_) | Event::Touch(_))
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
        let interaction = self.content.as_widget().mouse_interaction(
            self.tree,
            layout,
            cursor,
            &layout.bounds(),
            renderer,
        );

        // If cursor is over the overlay but between child widgets (e.g. in
        // spacing gaps), the content returns Interaction::None.  iced treats
        // None as "not interactive" and passes the cursor through to base
        // widgets underneath, causing hover states to bleed through.  Return
        // Idle instead so iced blocks the cursor from the base layer.
        if interaction == mouse::Interaction::None && cursor.is_over(layout.bounds()) {
            mouse::Interaction::Idle
        } else {
            interaction
        }
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
