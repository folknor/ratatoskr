use iced::widget::{Space, button, column, container, row, text};
use iced::{Element, Length};

use crate::ui::layout::*;
use crate::ui::theme;

use super::messages::CalendarMessage;

/// Confirmation dialog before deleting an event.
pub(super) fn delete_confirm_card(title: &str) -> Element<'_, CalendarMessage> {
    let display_title = if title.is_empty() {
        "(Untitled)"
    } else {
        title
    };
    let content = column![
        text("Delete Event")
            .size(TEXT_HEADING)
            .font(crate::font::text_semibold()),
        Space::new().height(SPACE_XS),
        text(format!(
            "Delete \"{display_title}\"? This cannot be undone."
        ))
        .size(TEXT_MD),
        Space::new().height(SPACE_MD),
        row![
            button(text("Delete").size(TEXT_SM))
                .on_press(CalendarMessage::DeleteEvent)
                .padding(PAD_BUTTON)
                .style(theme::ButtonClass::Nav { active: true }.style()),
            button(text("Cancel").size(TEXT_SM))
                .on_press(CalendarMessage::CloseModal)
                .padding(PAD_BUTTON)
                .style(theme::ButtonClass::Ghost.style()),
        ]
        .spacing(SPACE_XS),
    ]
    .spacing(SPACE_XXS);

    container(content)
        .width(Length::Fixed(CALENDAR_OVERLAY_WIDTH))
        .padding(PAD_CARD)
        .style(theme::ContainerClass::Elevated.style())
        .into()
}

/// Confirmation dialog before discarding unsaved editor changes.
pub(super) fn discard_confirm_card(title: &str) -> Element<'_, CalendarMessage> {
    let display_title = if title.is_empty() {
        "Discard unsaved changes?"
    } else {
        title
    };

    let content = column![
        text("Unsaved Changes")
            .size(TEXT_HEADING)
            .font(crate::font::text_semibold()),
        Space::new().height(SPACE_XS),
        text(display_title).size(TEXT_MD),
        Space::new().height(SPACE_MD),
        row![
            button(text("Discard").size(TEXT_SM))
                .on_press(CalendarMessage::DiscardChanges)
                .padding(PAD_BUTTON)
                .style(theme::ButtonClass::Nav { active: true }.style()),
            button(text("Cancel").size(TEXT_SM))
                .on_press(CalendarMessage::CloseModal)
                .padding(PAD_BUTTON)
                .style(theme::ButtonClass::Ghost.style()),
        ]
        .spacing(SPACE_XS),
    ]
    .spacing(SPACE_XXS);

    container(content)
        .width(Length::Fixed(CALENDAR_OVERLAY_WIDTH))
        .padding(PAD_CARD)
        .style(theme::ContainerClass::Elevated.style())
        .into()
}
