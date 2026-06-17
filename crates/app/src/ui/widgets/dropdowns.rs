#![allow(dead_code)]

use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Color, Element, Length};

use crate::icon;
use crate::ui::layout::{
    AVATAR_DROPDOWN_ITEM, AVATAR_DROPDOWN_TRIGGER, DROPDOWN_ITEM_HEIGHT, DROPDOWN_TRIGGER_HEIGHT,
    ICON_MD, ICON_SM, ICON_XL, PAD_DROPDOWN, PAD_NAV_ITEM, PAD_SELECT_TRIGGER, SELECT_MIN_WIDTH,
    SIDEBAR_MIN_WIDTH, SLOT_DROPDOWN, SPACE_XS, SPACE_XXS, TEXT_MD,
};
use crate::ui::theme;

use super::avatars::{account_avatar_circle, color_dot_sized, radio_circle};

/// Icon type for dropdown items. The dropdown builds the
/// Element internally - callers never pass pre-built UI.
pub enum DropdownIcon<'a> {
    /// Renders an avatar circle from a name string.
    Avatar { name: &'a str, color: Option<Color> },
    /// Renders an icon glyph from a codepoint char.
    Icon(char),
    /// Renders a filled color dot.
    ColorDot(Color),
    /// Renders a radio circle (primary-colored when selected, muted
    /// otherwise). For dropdowns used as single-select pickers.
    Radio { selected: bool },
}

impl DropdownIcon<'_> {
    fn into_element<'a, M: 'a>(self, size: f32) -> Element<'a, M> {
        match self {
            DropdownIcon::Avatar { name, color } => account_avatar_circle(name, color, size),
            DropdownIcon::Icon(codepoint) => icon::to_icon(codepoint)
                .size(ICON_XL)
                .style(text::secondary)
                .into(),
            DropdownIcon::ColorDot(color) => color_dot_sized(color, size),
            DropdownIcon::Radio { selected } => radio_circle(selected),
        }
    }
}

/// One entry in a dropdown menu.
pub struct DropdownEntry<'a, M> {
    pub icon: DropdownIcon<'a>,
    pub label: &'a str,
    pub selected: bool,
    pub on_press: M,
}

/// A complete dropdown: closed trigger + optional open menu.
/// Both trigger and items share the same two-slot layout.
pub fn dropdown<'a, M: Clone + 'a>(
    trigger_icon: DropdownIcon<'a>,
    trigger_label: &'a str,
    open: bool,
    on_toggle: M,
    items: Vec<DropdownEntry<'a, M>>,
) -> Element<'a, M> {
    let trigger = button(
        row![
            container(trigger_icon.into_element(AVATAR_DROPDOWN_TRIGGER))
                .width(SLOT_DROPDOWN)
                .height(SLOT_DROPDOWN)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center),
            container(text(trigger_label).size(TEXT_MD).style(text::base))
                .width(Length::Fill)
                .align_y(Alignment::Center),
            container(
                icon::chevron_down()
                    .size(ICON_SM)
                    .style(theme::TextClass::Tertiary.style())
            )
            .align_y(Alignment::Center),
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .on_press(on_toggle.clone())
    .padding(PAD_DROPDOWN)
    .style(theme::ButtonClass::Action.style())
    .width(Length::Fill)
    .height(DROPDOWN_TRIGGER_HEIGHT);

    if !open {
        return trigger.into();
    }

    let menu_items: Vec<Element<'a, M>> = items
        .into_iter()
        .map(|entry| {
            button(
                row![
                    container(entry.icon.into_element(AVATAR_DROPDOWN_ITEM))
                        .width(SLOT_DROPDOWN)
                        .height(SLOT_DROPDOWN)
                        .align_x(Alignment::Center)
                        .align_y(Alignment::Center),
                    container(text(entry.label).size(TEXT_MD).style(text::base))
                        .width(Length::Fill)
                        .align_y(Alignment::Center),
                ]
                .spacing(SPACE_XS)
                .align_y(Alignment::Center),
            )
            .on_press(entry.on_press)
            .padding(PAD_NAV_ITEM)
            .height(DROPDOWN_ITEM_HEIGHT)
            .style(
                theme::ButtonClass::Dropdown {
                    selected: entry.selected,
                }
                .style(),
            )
            .width(Length::Fill)
            .into()
        })
        .collect();

    let menu = container(column(menu_items).spacing(SPACE_XXS).width(Length::Fill))
        .padding(PAD_DROPDOWN)
        .style(theme::ContainerClass::SelectMenu.style())
        .width(Length::Fill);

    crate::ui::anchored_overlay::anchored_overlay(trigger)
        .popup(menu)
        .popup_width(SIDEBAR_MIN_WIDTH)
        .on_dismiss(on_toggle)
        .into()
}

/// A select widget for choosing from a list of text options.
/// Trigger is transparent with right-aligned label + chevron.
/// Generic over message type.
pub fn select<'a, M: Clone + 'a>(
    options: &[&'a str],
    selected: &'a str,
    open: bool,
    on_toggle: M,
    on_select: impl Fn(String) -> M + 'a,
) -> Element<'a, M> {
    let trigger = button(
        row![
            Space::new().width(Length::Fill),
            container(text(selected).size(TEXT_MD).style(text::base)).align_y(Alignment::Center),
            container(
                icon::chevron_down()
                    .size(ICON_SM)
                    .style(theme::TextClass::Tertiary.style()),
            )
            .align_y(Alignment::Center),
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .on_press(on_toggle.clone())
    .padding(PAD_SELECT_TRIGGER)
    .style(theme::ButtonClass::BareTransparent.style())
    .width(SELECT_MIN_WIDTH);

    if !open {
        return trigger.into();
    }

    let menu_items: Vec<Element<'a, M>> = options
        .iter()
        .map(|&option| {
            let is_selected = option == selected;
            let mut label_row = row![
                container(text(option).size(TEXT_MD).style(text::base)).align_y(Alignment::Center),
            ]
            .spacing(SPACE_XS)
            .align_y(Alignment::Center);

            if is_selected {
                label_row = label_row.push(
                    container(icon::check().size(ICON_MD).style(text::base))
                        .align_y(Alignment::Center),
                );
            }

            button(
                container(label_row)
                    .width(Length::Fill)
                    .align_y(Alignment::Center),
            )
            .on_press(on_select(option.to_string()))
            .padding(PAD_NAV_ITEM)
            .height(DROPDOWN_ITEM_HEIGHT)
            .style(
                theme::ButtonClass::Dropdown {
                    selected: is_selected,
                }
                .style(),
            )
            .width(Length::Fill)
            .into()
        })
        .collect();

    let menu = container(column(menu_items).spacing(SPACE_XXS).width(Length::Fill))
        .padding(PAD_DROPDOWN)
        .style(theme::ContainerClass::SelectMenu.style());

    crate::ui::anchored_overlay::anchored_overlay(trigger)
        .popup(menu)
        .on_dismiss(on_toggle.clone())
        .into()
}

/// Icon kinds supported in `select_with_icons`. Kept narrow on purpose -
/// add variants as call sites demand them.
#[derive(Debug, Clone, Copy)]
pub enum SelectIcon {
    ColorDot(Color),
}

impl SelectIcon {
    fn into_element<'a, M: 'a>(self) -> Element<'a, M> {
        match self {
            SelectIcon::ColorDot(color) => super::avatars::color_dot(color),
        }
    }
}

/// One row in a `select_with_icons` menu.
pub struct SelectOption<'a> {
    /// The value passed back through `on_select`. Trigger picks the matching
    /// option by exact equality against `selected_value`.
    pub value: String,
    pub label: &'a str,
    pub icon: Option<SelectIcon>,
}

/// A select widget with the same trigger/menu shape as `select`, plus optional
/// left-side icons on the trigger and each menu item. When `enabled` is false,
/// the chevron is suppressed, the label dims to `Tertiary`, and the trigger
/// becomes non-interactive (the menu never opens).
pub fn select_with_icons<'a, M: Clone + 'a>(
    options: Vec<SelectOption<'a>>,
    selected_value: Option<&'a str>,
    open: bool,
    enabled: bool,
    placeholder: &'a str,
    on_toggle: M,
    on_select: impl Fn(String) -> M + 'a,
) -> Element<'a, M> {
    let selected_idx = selected_value.and_then(|v| options.iter().position(|o| o.value == v));

    let (trigger_icon, trigger_label) = match selected_idx {
        Some(i) => (options[i].icon, options[i].label),
        None => (None, placeholder),
    };

    let mut trigger_row = row![Space::new().width(Length::Fill)]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center);

    if let Some(kind) = trigger_icon {
        trigger_row =
            trigger_row.push(container(kind.into_element::<M>()).align_y(Alignment::Center));
    }

    let label_style: fn(&iced::Theme) -> iced::widget::text::Style = if enabled {
        text::base
    } else {
        theme::TextClass::Tertiary.style()
    };
    trigger_row = trigger_row.push(
        container(text(trigger_label).size(TEXT_MD).style(label_style)).align_y(Alignment::Center),
    );

    if enabled {
        trigger_row = trigger_row.push(
            container(
                icon::chevron_down()
                    .size(ICON_SM)
                    .style(theme::TextClass::Tertiary.style()),
            )
            .align_y(Alignment::Center),
        );
    }

    let mut trigger = button(trigger_row)
        .padding(PAD_SELECT_TRIGGER)
        .style(theme::ButtonClass::BareTransparent.style())
        .width(SELECT_MIN_WIDTH);
    if enabled {
        trigger = trigger.on_press(on_toggle.clone());
    }

    if !enabled || !open {
        return trigger.into();
    }

    let menu_items: Vec<Element<'a, M>> = options
        .into_iter()
        .map(|opt| {
            let is_selected = selected_value == Some(opt.value.as_str());
            let mut item_row = row![].spacing(SPACE_XS).align_y(Alignment::Center);
            if let Some(kind) = opt.icon {
                item_row =
                    item_row.push(container(kind.into_element::<M>()).align_y(Alignment::Center));
            }
            item_row = item_row.push(
                container(text(opt.label).size(TEXT_MD).style(text::base))
                    .width(Length::Fill)
                    .align_y(Alignment::Center),
            );
            if is_selected {
                item_row = item_row.push(
                    container(icon::check().size(ICON_MD).style(text::base))
                        .align_y(Alignment::Center),
                );
            }
            button(
                container(item_row)
                    .width(Length::Fill)
                    .align_y(Alignment::Center),
            )
            .on_press(on_select(opt.value.clone()))
            .padding(PAD_NAV_ITEM)
            .height(DROPDOWN_ITEM_HEIGHT)
            .style(
                theme::ButtonClass::Dropdown {
                    selected: is_selected,
                }
                .style(),
            )
            .width(Length::Fill)
            .into()
        })
        .collect();

    let menu = container(column(menu_items).spacing(SPACE_XXS).width(Length::Fill))
        .padding(PAD_DROPDOWN)
        .style(theme::ContainerClass::SelectMenu.style());

    crate::ui::anchored_overlay::anchored_overlay(trigger)
        .popup(menu)
        .on_dismiss(on_toggle.clone())
        .into()
}
