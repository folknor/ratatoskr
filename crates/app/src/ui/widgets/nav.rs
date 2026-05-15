#![allow(dead_code)]

use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Color, Element, Length, Theme};

use crate::ui::layout::{
    ICON_MD, ICON_XL, PAD_ICON_BTN, PAD_NAV_ITEM, PAD_SETTINGS_ROW, SPACE_XS, SPACE_XXS, TEXT_LG,
    TEXT_MD,
};
use crate::ui::theme;

use super::avatars::color_dot;
use super::layout::count_badge;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavSize {
    /// Sidebar folder list - compact padding
    Compact,
    /// Settings tabs - more spacious padding
    Regular,
}

/// Generic navigation button used in both the sidebar and settings.
/// Accepts data only - builds its own two-slot (icon + label) structure.
/// Generic over message type so settings can use it with SettingsMessage.
pub fn nav_button<'a, M: Clone + 'a>(
    ico: Option<iced::widget::Text<'a>>,
    label: &'a str,
    active: bool,
    size: NavSize,
    badge: Option<i64>,
    on_press: M,
) -> Element<'a, M> {
    let label_style: fn(&Theme) -> text::Style = if active {
        text::primary
    } else {
        theme::TextClass::Muted.style()
    };
    let icon_style: fn(&Theme) -> text::Style = if active {
        text::primary
    } else {
        theme::TextClass::Muted.style()
    };
    let pad = match size {
        NavSize::Compact => PAD_NAV_ITEM,
        NavSize::Regular => PAD_SETTINGS_ROW,
    };
    let icon_size = match size {
        NavSize::Compact => ICON_MD,
        NavSize::Regular => ICON_XL,
    };
    let text_size = match size {
        NavSize::Compact => TEXT_MD,
        NavSize::Regular => TEXT_LG,
    };

    let mut content = row![].spacing(SPACE_XS).align_y(Alignment::Center);

    if let Some(ico) = ico {
        content = content
            .push(container(ico.size(icon_size).style(icon_style)).align_y(Alignment::Center));
    }

    content = content.push(
        container(text(label).size(text_size).style(label_style))
            .align_y(Alignment::Center),
    );

    if let Some(count) = badge
        && count > 0
    {
        content = content
            .push(Space::new().width(Length::Fill))
            .push(count_badge(count));
    }

    button(content)
        .on_press(on_press)
        .padding(pad)
        .style(theme::ButtonClass::Nav { active }.style())
        .width(Length::Fill)
        .into()
}

pub struct NavItem<'a> {
    pub label: &'a str,
    pub id: &'a str,
    pub unread: i64,
}

pub fn nav_group<'a, M: Clone + 'a>(
    items: &[NavItem<'a>],
    selection: &'a types::SidebarSelection,
    on_select: impl Fn(types::SidebarSelection) -> M,
) -> Element<'a, M> {
    let mut col = column![].spacing(SPACE_XXS);
    for item in items {
        let item_sel = crate::ui::sidebar::universal_folder_selection(item.id);
        let is_active = *selection == item_sel;
        let on_press = on_select(item_sel);
        col = col.push(nav_button(
            None,
            item.label,
            is_active,
            NavSize::Compact,
            Some(item.unread),
            on_press,
        ));
    }
    col.into()
}

pub fn label_nav_item<'a, M: Clone + 'a>(
    name: &'a str,
    _id: &'a str,
    color: Color,
    active: bool,
    on_press: M,
) -> Element<'a, M> {
    let lbl_style: fn(&Theme) -> text::Style = if active {
        text::primary
    } else {
        text::secondary
    };

    button(
        row![
            color_dot(color),
            container(text(name).size(TEXT_MD).style(lbl_style)).align_y(Alignment::Center),
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .on_press(on_press)
    .padding(PAD_ICON_BTN)
    .style(theme::ButtonClass::Nav { active }.style())
    .width(Length::Fill)
    .into()
}
