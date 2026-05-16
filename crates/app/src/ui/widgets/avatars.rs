#![allow(dead_code)]

use std::path::Path;

use iced::widget::{Canvas, Space, canvas, container, image, text};
use iced::{Alignment, Color, Element, Length, Rectangle, Renderer, Theme, mouse};

use crate::font;
use crate::ui::label_paint::LabelPaint;
use crate::ui::layout::{DOT_SIZE, LABEL_DOT_SIZE, RADIO_CIRCLE_SIZE};
use crate::ui::theme;

fn avatar_circle_with_color<'a, M: 'a>(name: &str, size: f32, color: Color) -> Element<'a, M> {
    let letter = theme::initial(name);

    let circle = Canvas::new(CirclePainter { color, size })
        .width(size)
        .height(size);

    iced::widget::stack![
        circle,
        container(
            text(letter)
                .size(size * 0.45)
                .color(theme::ON_AVATAR)
                .font(iced::Font {
                    weight: iced::font::Weight::Bold,
                    ..font::text()
                }),
        )
        .center(Length::Fill),
    ]
    .width(size)
    .height(size)
    .into()
}

pub fn avatar_circle<'a, M: 'a>(name: &str, size: f32) -> Element<'a, M> {
    let color = theme::avatar_color(name);
    avatar_circle_with_color(name, size, color)
}

pub fn account_avatar_circle<'a, M: 'a>(
    name: &str,
    account_color: Option<Color>,
    size: f32,
) -> Element<'a, M> {
    let color = account_color.unwrap_or_else(|| theme::avatar_color(name));
    avatar_circle_with_color(name, size, color)
}

/// Render a sender avatar: BIMI logo if available, initials circle otherwise.
pub fn sender_avatar<'a, M: 'a>(name: &str, bimi_logo: Option<&Path>, size: f32) -> Element<'a, M> {
    match bimi_logo {
        Some(path) => {
            let handle = image::Handle::from_path(path);
            container(
                image(handle)
                    .width(size)
                    .height(size)
                    .content_fit(iced::ContentFit::Cover),
            )
            .width(size)
            .height(size)
            .clip(true)
            .style(move |_theme: &Theme| container::Style {
                border: iced::Border {
                    radius: (size / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
        }
        None => avatar_circle(name, size),
    }
}

pub fn color_dot<'a, M: 'a>(color: Color) -> Element<'a, M> {
    let dot = Canvas::new(DotPainter { color })
        .width(DOT_SIZE)
        .height(DOT_SIZE);
    container(dot).center_y(Length::Shrink).into()
}

/// A color dot at a custom size.
pub fn color_dot_sized<'a, M: 'a>(color: Color, size: f32) -> Element<'a, M> {
    let dot = Canvas::new(DotPainter { color }).width(size).height(size);
    container(dot).center_y(Length::Shrink).into()
}

// Label dots render only `paint.bg()` - a single-color disc has no
// foreground channel to paint - but the signature takes the complete
// `LabelPaint` so every label-shaped surface goes through the same
// sealed-pair boundary as pills and rows. Don't "simplify" these to take
// a bare `Color`: that would let raw label hex bypass `LabelStyleHex`.
pub fn label_color_dot<'a, M: 'a>(paint: LabelPaint) -> Element<'a, M> {
    color_dot(paint.bg())
}

pub fn label_dot<'a, M: 'a>(paint: LabelPaint) -> Element<'a, M> {
    let dot = Canvas::new(DotPainter { color: paint.bg() })
        .width(LABEL_DOT_SIZE)
        .height(LABEL_DOT_SIZE);
    container(dot).center_y(Length::Shrink).into()
}

/// Decorative radio circle: outer ring (primary color when selected, muted
/// otherwise) with a smaller filled disk centered inside when selected.
/// Has no click handler - the parent widget owns interaction.
pub fn radio_circle<'a, M: 'a>(selected: bool) -> Element<'a, M> {
    let inner: Element<'a, M> = if selected {
        let inner_size = RADIO_CIRCLE_SIZE * 0.3;
        container(Space::new())
            .width(Length::Fixed(inner_size))
            .height(Length::Fixed(inner_size))
            .style(theme::ContainerClass::RadioCircleInner.style())
            .into()
    } else {
        Space::new().into()
    };

    let outer_class = if selected {
        theme::ContainerClass::RadioCircleSelected
    } else {
        theme::ContainerClass::RadioCircleUnselected
    };

    container(inner)
        .width(Length::Fixed(RADIO_CIRCLE_SIZE))
        .height(Length::Fixed(RADIO_CIRCLE_SIZE))
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(outer_class.style())
        .into()
}

struct CirclePainter {
    color: Color,
    size: f32,
}

impl<M> canvas::Program<M> for CirclePainter {
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
        let circle = canvas::path::Path::circle(iced::Point::new(radius, radius), radius);
        frame.fill(&circle, self.color);
        vec![frame.into_geometry()]
    }
}

struct DotPainter {
    color: Color,
}

impl<M> canvas::Program<M> for DotPainter {
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
        let radius = DOT_SIZE / 2.0;
        let circle = canvas::path::Path::circle(iced::Point::new(radius, radius), radius);
        frame.fill(&circle, self.color);
        vec![frame.into_geometry()]
    }
}
