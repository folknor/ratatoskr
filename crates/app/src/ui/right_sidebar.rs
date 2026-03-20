use iced::widget::{column, container, scrollable, text, Space};
use iced::{Element, Length};

use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets;

pub fn view<'a, M: 'a>(open: bool) -> Element<'a, M> {
    if !open {
        return Space::new().width(0).height(0).into();
    }

    let content = column![
        calendar_section(),
        widgets::divider(),
        pinned_section(),
    ]
    .spacing(0)
    .width(Length::Fill);

    container(scrollable(content).spacing(SCROLLBAR_SPACING).height(Length::Fill))
        .width(RIGHT_SIDEBAR_WIDTH)
        .height(Length::Fill)
        .style(theme::ContainerClass::Sidebar.style())
        .into()
}

fn calendar_section<'a, M: 'a>() -> Element<'a, M> {
    container(
        column![
            widgets::section_header("CALENDAR"),
            container(
                text("Calendar placeholder")
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style()),
            )
            .padding(PAD_ICON_BTN),
        ]
        .spacing(SPACE_XXS),
    )
    .padding(PAD_RIGHT_SIDEBAR)
    .into()
}

fn pinned_section<'a, M: 'a>() -> Element<'a, M> {
    container(
        column![
            widgets::section_header("PINNED ITEMS"),
            container(
                text("No pinned items")
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style()),
            )
            .padding(PAD_ICON_BTN),
        ]
        .spacing(SPACE_XXS),
    )
    .padding(PAD_RIGHT_SIDEBAR)
    .into()
}
