#![allow(dead_code)]

use iced::widget::{Space, button, column, container, row, rule, text};
use iced::{Alignment, Element, Length};

use crate::icon;
use crate::ui::layout::{
    ICON_XS, PAD_BADGE, PAD_COLLAPSIBLE_HEADER, PAD_SECTION_HEADER, PAD_STAT_ROW, SPACE_XXS,
    TEXT_MD, TEXT_SM, TEXT_TITLE, TEXT_XS,
};
use crate::ui::theme;

pub fn leading_slot<'a, M: 'a>(content: impl Into<Element<'a, M>>, size: f32) -> Element<'a, M> {
    container(content)
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .center(Length::Shrink)
        .into()
}

pub fn count_badge<'a, M: 'a>(count: i64) -> Element<'a, M> {
    if count == 0 {
        return Space::new().width(0).height(0).into();
    }
    let label = if count > 999 {
        "999+".to_string()
    } else {
        count.to_string()
    };
    container(text(label).size(TEXT_XS).style(text::secondary))
        .padding(PAD_BADGE)
        .style(theme::ContainerClass::Badge.style())
        .into()
}

pub fn divider<'a, M: 'a>() -> Element<'a, M> {
    rule::horizontal(1)
        .style(theme::RuleClass::Divider.style())
        .into()
}

pub fn section_break<'a, M: 'a>() -> Element<'a, M> {
    column![
        Space::new().height(SPACE_XXS),
        divider(),
        Space::new().height(SPACE_XXS),
    ]
    .into()
}

pub fn collapsible_section<'a, M: Clone + 'a>(
    title: &'a str,
    expanded: bool,
    on_toggle: M,
    children: Vec<Element<'a, M>>,
) -> Element<'a, M> {
    let chevron = if expanded {
        icon::chevron_down()
    } else {
        icon::chevron_right()
    };

    let header = button(
        row![
            container(
                text(title)
                    .size(TEXT_XS)
                    .style(theme::TextClass::Tertiary.style())
            )
            .align_y(Alignment::Center),
            Space::new().width(Length::Fill),
            container(
                chevron
                    .size(ICON_XS)
                    .style(theme::TextClass::Tertiary.style())
            )
            .align_y(Alignment::Center),
        ]
        .align_y(Alignment::Center),
    )
    .on_press(on_toggle)
    .padding(PAD_COLLAPSIBLE_HEADER)
    .style(theme::ButtonClass::Action.style())
    .width(Length::Fill);

    let mut col = column![header].spacing(SPACE_XXS);

    if expanded {
        for child in children {
            col = col.push(child);
        }
    }

    col.into()
}

pub fn empty_placeholder<'a, M: 'a>(title: &'a str, subtitle: &'a str) -> Element<'a, M> {
    container(
        column![
            text(title)
                .size(TEXT_TITLE)
                .style(theme::TextClass::Tertiary.style()),
            text(subtitle)
                .size(TEXT_MD)
                .style(theme::TextClass::Tertiary.style()),
        ]
        .spacing(SPACE_XXS)
        .align_x(Alignment::Center),
    )
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

pub fn section_header<'a, M: 'a>(display_text: &'a str) -> Element<'a, M> {
    container(
        text(display_text)
            .size(TEXT_XS)
            .style(theme::TextClass::Tertiary.style()),
    )
    .padding(PAD_SECTION_HEADER)
    .width(Length::Fill)
    .into()
}

pub fn stat_row<'a, M: 'a>(display_text: &'a str, value: &'a str) -> Element<'a, M> {
    container(
        row![
            text(display_text)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
            Space::new().width(Length::Fill),
            text(value).size(TEXT_SM).style(text::secondary),
        ]
        .align_y(Alignment::Center),
    )
    .padding(PAD_STAT_ROW)
    .width(Length::Fill)
    .into()
}
