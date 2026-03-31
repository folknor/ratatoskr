//! Animated toggler widget.
//!
//! Drop-in replacement for `iced::widget::toggler` with smooth
//! EaseOutCubic animation (150ms) on the pill position.
//! Uses iced's built-in `Animation<bool>` + `lilt` for interpolation.

use iced::advanced::layout;
use iced::advanced::renderer::{self, Renderer as _};
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget};
use iced::animation::{self, Easing};
use iced::mouse;
use iced::time::{Duration, Instant};
use iced::touch;
use iced::widget::toggler::{self, Status, Style};
use iced::window;
use iced::{Border, Element, Event, Length, Pixels, Rectangle, Size, Theme, border};

const ANIMATION_DURATION: Duration = Duration::from_millis(150);

/// Style function type for the animated toggler.
type TogglerStyleFn<'a> = Box<dyn Fn(&Theme, Status) -> Style + 'a>;

/// Internal widget state stored in the tree.
struct State {
    anim: animation::Animation<bool>,
    last_status: Option<Status>,
}

/// Animated toggler widget.
pub struct AnimatedToggler<'a, Message> {
    is_toggled: bool,
    on_toggle: Option<Box<dyn Fn(bool) -> Message + 'a>>,
    size: f32,
    style_fn: TogglerStyleFn<'a>,
}

impl<'a, Message> AnimatedToggler<'a, Message> {
    pub fn new(is_toggled: bool) -> Self {
        Self {
            is_toggled,
            on_toggle: None,
            size: 16.0,
            style_fn: Box::new(toggler::default),
        }
    }

    pub fn on_toggle(mut self, on_toggle: impl Fn(bool) -> Message + 'a) -> Self {
        self.on_toggle = Some(Box::new(on_toggle));
        self
    }

    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.size = size.into().0;
        self
    }

    pub fn style(mut self, style: impl Fn(&Theme, Status) -> Style + 'a) -> Self {
        self.style_fn = Box::new(style);
        self
    }
}

impl<Message> Widget<Message, Theme, iced::Renderer> for AnimatedToggler<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State {
            anim: animation::Animation::new(self.is_toggled)
                .easing(Easing::EaseOutCubic)
                .duration(ANIMATION_DURATION),
            last_status: None,
        })
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Shrink,
            height: Length::Shrink,
        }
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        _limits: &layout::Limits,
    ) -> layout::Node {
        // Toggler is 2:1 width:height ratio
        layout::Node::new(Size::new(2.0 * self.size, self.size))
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();

        // Handle click/tap → toggle
        if let Some(on_toggle) = &self.on_toggle {
            match event {
                Event::Mouse(mouse::Event::ButtonPressed {
                    button: mouse::Button::Left,
                    ..
                })
                | Event::Touch(touch::Event::FingerPressed { .. })
                    if cursor.is_over(layout.bounds()) =>
                {
                    shell.publish(on_toggle(!self.is_toggled));
                    shell.capture_event();
                }
                _ => {}
            }
        }

        // Drive animation: when is_toggled changes, start transition
        if state.anim.value() != self.is_toggled
            && let Event::Window(window::Event::RedrawRequested(now)) = event
        {
            state.anim.go_mut(self.is_toggled, *now);
        }

        // Request redraws while animating
        if let Event::Window(window::Event::RedrawRequested(now)) = event {
            if state.anim.is_animating(*now) {
                shell.request_redraw();
            }
            let current_status = self.current_status(cursor, layout);
            state.last_status = Some(current_status);
        } else {
            let current_status = self.current_status(cursor, layout);
            if state.last_status.is_some_and(|s| s != current_status) {
                shell.request_redraw();
            }
        }
    }

    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        if cursor.is_over(layout.bounds()) {
            if self.on_toggle.is_some() {
                mouse::Interaction::Pointer
            } else {
                mouse::Interaction::NotAllowed
            }
        } else {
            mouse::Interaction::default()
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        _defaults: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        let now = Instant::now();

        // Get styles for both states so we can interpolate the background color.
        let base_status = state.last_status.unwrap_or(Status::Disabled {
            is_toggled: self.is_toggled,
        });
        let style_off = (self.style_fn)(theme, with_toggled(base_status, false));
        let style_on = (self.style_fn)(theme, with_toggled(base_status, true));
        // Use the target style for non-interpolated properties.
        let style = (self.style_fn)(theme, base_status);

        let border_radius = style
            .border_radius
            .unwrap_or_else(|| border::Radius::new(bounds.height / 2.0));

        // Animated pill position: interpolate from off (0.0) to on (1.0)
        let t: f32 = state.anim.interpolate(0.0, 1.0, now);

        // Interpolate track background color
        let bg_color = lerp_background(style_off.background, style_on.background, t);

        // Draw track (background)
        renderer.fill_quad(
            renderer::Quad {
                bounds,
                border: Border {
                    radius: border_radius,
                    width: style.background_border_width,
                    color: style.background_border_color,
                },
                ..renderer::Quad::default()
            },
            bg_color,
        );

        let padding = (style.padding_ratio * bounds.height).round();
        let pill_size = bounds.height - (2.0 * padding);
        let travel = bounds.width - bounds.height; // total X distance

        let pill_x = bounds.x + padding + (t * travel);

        let pill_bounds = Rectangle {
            x: pill_x,
            y: bounds.y + padding,
            width: pill_size,
            height: pill_size,
        };

        // Draw pill (foreground)
        renderer.fill_quad(
            renderer::Quad {
                bounds: pill_bounds,
                border: Border {
                    radius: border_radius,
                    width: style.foreground_border_width,
                    color: style.foreground_border_color,
                },
                ..renderer::Quad::default()
            },
            style.foreground,
        );
    }
}

impl<'a, Message> AnimatedToggler<'a, Message> {
    fn current_status(&self, cursor: mouse::Cursor, layout: Layout<'_>) -> Status {
        if self.on_toggle.is_none() {
            Status::Disabled {
                is_toggled: self.is_toggled,
            }
        } else if cursor.is_over(layout.bounds()) {
            Status::Hovered {
                is_toggled: self.is_toggled,
            }
        } else {
            Status::Active {
                is_toggled: self.is_toggled,
            }
        }
    }
}

impl<'a, Message: 'a> From<AnimatedToggler<'a, Message>> for Element<'a, Message> {
    fn from(toggler: AnimatedToggler<'a, Message>) -> Self {
        Element::new(toggler)
    }
}

/// Convenience constructor matching `iced::widget::toggler` API.
pub fn animated_toggler<'a, Message>(is_toggled: bool) -> AnimatedToggler<'a, Message> {
    AnimatedToggler::new(is_toggled)
}

/// Replace the `is_toggled` field in a Status while preserving the variant.
fn with_toggled(status: Status, is_toggled: bool) -> Status {
    match status {
        Status::Active { .. } => Status::Active { is_toggled },
        Status::Hovered { .. } => Status::Hovered { is_toggled },
        Status::Disabled { .. } => Status::Disabled { is_toggled },
    }
}

/// Linearly interpolate between two `Background` values.
/// Only handles solid colors; gradients fall back to `to`.
fn lerp_background(from: iced::Background, to: iced::Background, t: f32) -> iced::Background {
    match (from, to) {
        (iced::Background::Color(a), iced::Background::Color(b)) => {
            iced::Background::Color(iced::Color {
                r: a.r + (b.r - a.r) * t,
                g: a.g + (b.g - a.g) * t,
                b: a.b + (b.b - a.b) * t,
                a: a.a + (b.a - a.a) * t,
            })
        }
        _ => to,
    }
}
