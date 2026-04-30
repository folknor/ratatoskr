#![allow(dead_code)]

use iced::widget::{Canvas, button, canvas, container};
use iced::{Color, Element, Rectangle, Renderer, Theme, mouse};

use crate::ui::layout::RADIUS_MD;
use crate::ui::theme;

/// Paints 6 vertical color stripes (bg, text, primary, success, warning, danger).
struct ThemePreviewPainter {
    colors: [Color; 6],
    radius: f32,
}

impl<M> canvas::Program<M> for ThemePreviewPainter {
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
        let stripe_width = bounds.width / 6.0;
        let r = self.radius;

        let bg_rect = canvas::path::Path::new(|b| {
            rounded_rect(b, bounds.width, bounds.height, r);
        });
        frame.fill(&bg_rect, self.colors[0]);

        for i in 1..5 {
            let x = stripe_width * i as f32;
            let rect = canvas::path::Path::rectangle(
                iced::Point::new(x, 0.0),
                iced::Size::new(stripe_width, bounds.height),
            );
            frame.fill(&rect, self.colors[i]);
        }

        let last = canvas::path::Path::new(|b| {
            let x = stripe_width * 5.0;
            b.move_to(iced::Point::new(x, 0.0));
            b.line_to(iced::Point::new(bounds.width - r, 0.0));
            b.arc_to(
                iced::Point::new(bounds.width, 0.0),
                iced::Point::new(bounds.width, r),
                r,
            );
            b.line_to(iced::Point::new(bounds.width, bounds.height - r));
            b.arc_to(
                iced::Point::new(bounds.width, bounds.height),
                iced::Point::new(bounds.width - r, bounds.height),
                r,
            );
            b.line_to(iced::Point::new(x, bounds.height));
            b.close();
        });
        frame.fill(&last, self.colors[5]);

        vec![frame.into_geometry()]
    }
}

fn rounded_rect(builder: &mut canvas::path::Builder, w: f32, h: f32, r: f32) {
    builder.move_to(iced::Point::new(r, 0.0));
    builder.line_to(iced::Point::new(w - r, 0.0));
    builder.arc_to(iced::Point::new(w, 0.0), iced::Point::new(w, r), r);
    builder.line_to(iced::Point::new(w, h - r));
    builder.arc_to(iced::Point::new(w, h), iced::Point::new(w - r, h), r);
    builder.line_to(iced::Point::new(r, h));
    builder.arc_to(iced::Point::new(0.0, h), iced::Point::new(0.0, h - r), r);
    builder.line_to(iced::Point::new(0.0, r));
    builder.arc_to(iced::Point::new(0.0, 0.0), iced::Point::new(r, 0.0), r);
    builder.close();
}

/// Theme preview: 6 vertical stripes in a rounded 16:9 rectangle.
pub fn theme_preview<'a, M: Clone + 'a>(
    palette: &iced::theme::palette::Seed,
    selected: bool,
    on_press: M,
) -> Element<'a, M> {
    let colors = [
        palette.background,
        palette.text,
        palette.primary,
        palette.success,
        palette.warning,
        palette.danger,
    ];

    let preview_width: f32 = 120.0;
    let preview_height: f32 = preview_width * 9.0 / 16.0;

    let preview_canvas = Canvas::new(ThemePreviewPainter {
        colors,
        radius: RADIUS_MD,
    })
    .width(preview_width)
    .height(preview_height);

    let gap = 2.0;
    let border_width = 2.0;
    let total_inset = gap + border_width;

    if selected {
        container(container(preview_canvas).padding(total_inset))
            .style(theme::ContainerClass::ThemeSelectedRing.style())
            .into()
    } else {
        button(container(preview_canvas).padding(total_inset))
            .on_press(on_press)
            .padding(0)
            .style(theme::ButtonClass::BareTransparent.style())
            .into()
    }
}

/// A simple spinning arc indicator.
///
/// Uses the frame's cache invalidation to animate. The spinner re-renders
/// on every frame while visible because the canvas `Program` always
/// returns `canvas::Action::request_redraw()`.
pub fn spinner<'a, M: 'a>(size: f32) -> Element<'a, M> {
    Canvas::new(SpinnerPainter {
        start: std::time::Instant::now(),
    })
    .width(size)
    .height(size)
    .into()
}

struct SpinnerPainter {
    start: std::time::Instant,
}

impl<M> canvas::Program<M> for SpinnerPainter {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry<Renderer>> {
        let palette = theme.palette();
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let center = frame.center();
        let radius = bounds.width.min(bounds.height) / 2.0 - 2.0;

        let elapsed = self.start.elapsed().as_secs_f32();
        let start_angle = elapsed * 4.0;
        let sweep = std::f32::consts::FRAC_PI_2 * 3.0;

        let path = canvas::path::Builder::new();
        let mut builder = canvas::path::Builder::new();
        builder.arc(canvas::path::Arc {
            center,
            radius,
            start_angle: iced::Radians(start_angle),
            end_angle: iced::Radians(start_angle + sweep),
        });
        let arc_path = builder.build();

        frame.stroke(
            &arc_path,
            canvas::Stroke::default()
                .with_color(palette.primary.base.color)
                .with_width(2.5),
        );
        drop(path);

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        mouse::Interaction::default()
    }
}
