use iced::widget::{Space, container, mouse_area};
use iced::{Element, Length, Padding, mouse};

use super::theme;

/// Which kind of blocking surface to compose.
pub enum ModalSurface {
    /// Dimmed backdrop (`ModalBackdrop`), centered content.
    /// Dismiss via Escape or explicit button - not via backdrop click.
    Modal,

    /// Opaque sheet sliding from the right edge. Unstyled event blocker
    /// underneath (the sheet covers everything visually).
    ///
    /// `offset` is raw left padding in pixels on a full-viewport-width
    /// container. `0.0` means the sheet fills the entire area (fully visible).
    /// The settings animation uses `((1.0 - t) * 2000.0)` where 2000 exceeds
    /// any realistic viewport width, pushing the sheet offscreen to the right
    /// when closed.
    Sheet { offset: f32 },
}

/// Compose a blocking overlay surface: `base` underneath, event-blocking
/// layer in the middle, `content` on top.
///
/// `blocker_msg` is required because iced's `mouse_area` only captures
/// events for which a handler is wired. The blocker wires every
/// capturing handler (`on_press`, `on_double_click`, `on_right_press`,
/// `on_middle_press`, `on_scroll`) to `blocker_msg` so left clicks,
/// right clicks, middle clicks, double clicks, and scroll events on the
/// blocker area are all swallowed and don't reach the widgets behind
/// it. The message itself should be a no-op in the caller's update
/// loop - `modal_overlay` does not own dismiss behavior.
///
/// Release events (`on_release`, `on_right_release`,
/// `on_middle_release`) are intentionally NOT wired: iced's
/// `mouse_area` doesn't capture release events, and base widgets only
/// fire on press anyway, so an unblocked release does no harm.
#[allow(clippy::needless_pass_by_value)]
pub fn modal_overlay<'a, Message: Clone + 'a>(
    base: impl Into<Element<'a, Message>>,
    content: impl Into<Element<'a, Message>>,
    surface: ModalSurface,
    blocker_msg: Message,
) -> Element<'a, Message> {
    match surface {
        ModalSurface::Modal => {
            let blocker = blocking_mouse_area(
                container("")
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .style(theme::ContainerClass::ModalBackdrop.style())
                    .into(),
                blocker_msg,
            );

            let centered: Element<'a, Message> = container(content)
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .into();

            iced::widget::stack![base.into(), blocker, centered].into()
        }
        ModalSurface::Sheet { offset } => {
            let blocker = blocking_mouse_area(
                container(Space::new().width(Length::Fill).height(Length::Fill))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                blocker_msg,
            );

            let sheet: Element<'a, Message> = container(content)
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(Padding {
                    top: 0.0,
                    right: 0.0,
                    bottom: 0.0,
                    left: offset,
                })
                .into();

            iced::widget::stack![base.into(), blocker, sheet]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        }
    }
}

/// Wrap a content element in a `mouse_area` configured to absorb every
/// interactive mouse event so the layer below cannot receive them.
fn blocking_mouse_area<'a, Message: Clone + 'a>(
    content: Element<'a, Message>,
    blocker_msg: Message,
) -> Element<'a, Message> {
    let scroll_msg = blocker_msg.clone();
    mouse_area(content)
        .on_press(blocker_msg.clone())
        .on_double_click(blocker_msg.clone())
        .on_right_press(blocker_msg.clone())
        .on_middle_press(blocker_msg.clone())
        .on_scroll(move |_delta| scroll_msg.clone())
        // `Interaction::Idle` (not `default()`, which is `None`) makes the
        // blocker actively claim the regular cursor. iced's `stack`
        // mouse_interaction walks children top-to-bottom and skips any
        // child returning `None`, so a `default()` blocker would let the
        // base layer's pointer / text cursors bleed through to the
        // dimmed area above it.
        .interaction(mouse::Interaction::Idle)
        .into()
}
