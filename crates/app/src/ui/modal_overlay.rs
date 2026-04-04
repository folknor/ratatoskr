use iced::widget::{container, mouse_area, Space};
use iced::{Element, Length, Padding, mouse};

use super::theme;

/// Which kind of blocking surface to compose.
pub enum ModalSurface {
    /// Dimmed backdrop (`ModalBackdrop`), centered content.
    /// Dismiss via Escape or explicit button — not via backdrop click.
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
/// `blocker_msg` is required because iced's `mouse_area` only captures click
/// events when `on_press` is set. The message is published but should be a
/// no-op in the caller's update loop — `modal_overlay` does not own dismiss
/// behavior.
pub fn modal_overlay<'a, Message: Clone + 'a>(
    base: impl Into<Element<'a, Message>>,
    content: impl Into<Element<'a, Message>>,
    surface: ModalSurface,
    blocker_msg: Message,
) -> Element<'a, Message> {
    match surface {
        ModalSurface::Modal => {
            let blocker = mouse_area(
                container("")
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .style(theme::ContainerClass::ModalBackdrop.style()),
            )
            .on_press(blocker_msg);

            let centered = container(content)
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill);

            iced::widget::stack![base.into(), blocker.into(), centered.into()].into()
        }
        ModalSurface::Sheet { offset } => {
            let blocker = mouse_area(
                container(Space::new().width(Length::Fill).height(Length::Fill))
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(blocker_msg)
            .interaction(mouse::Interaction::default());

            let sheet = container(content)
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(Padding {
                    top: 0.0,
                    right: 0.0,
                    bottom: 0.0,
                    left: offset,
                });

            iced::widget::stack![base.into(), blocker.into(), sheet.into()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        }
    }
}
