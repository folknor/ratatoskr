use iced::widget::{canvas, container, text, Canvas};
use iced::{mouse, Element, Length, Rectangle, Renderer, Theme};

use crate::ui::theme as colors;
use crate::Message;

/// Colored circle with initial letter, used for avatars.
pub fn avatar_circle<'a>(name: &str, size: f32) -> Element<'a, Message> {
    let color = colors::avatar_color(name);
    let letter = colors::initial(name);

    let circle = Canvas::new(CirclePainter { color, size })
        .width(size)
        .height(size);

    // Overlay the letter on the circle
    iced::widget::stack![
        circle,
        container(
            text(letter)
                .size(size * 0.45)
                .color(iced::Color::WHITE),
        )
        .center(Length::Fill),
    ]
    .width(size)
    .height(size)
    .into()
}

struct CirclePainter {
    color: iced::Color,
    size: f32,
}

impl canvas::Program<Message> for CirclePainter {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry<Renderer>> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let radius = self.size / 2.0;
        let circle = canvas::path::Path::circle(
            iced::Point::new(radius, radius),
            radius,
        );
        frame.fill(&circle, self.color);
        vec![frame.into_geometry()]
    }
}

/// Small colored dot for labels in sidebar.
pub fn color_dot<'a>(color: iced::Color) -> Element<'a, Message> {
    let dot = Canvas::new(DotPainter { color })
        .width(8)
        .height(8);
    container(dot)
        .center_y(Length::Shrink)
        .into()
}

struct DotPainter {
    color: iced::Color,
}

impl canvas::Program<Message> for DotPainter {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry<Renderer>> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let circle = canvas::path::Path::circle(
            iced::Point::new(4.0, 4.0),
            4.0,
        );
        frame.fill(&circle, self.color);
        vec![frame.into_geometry()]
    }
}
